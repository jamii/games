#!/usr/bin/env python3
"""Train the network for stage 2 (see BOT.md) from self-play samples.

    python3 tools/train.py samples.jsonl [more.jsonl ...] --out net.json [--init net.json]

The samples come from `selfplay --samples FILE`: every searched decision, from
the deciding player's point of view, with the search's visit counts (the policy
target) and who won (the value target). The network is the one in src/net.rs:
an embedding bag per token, a few transformer layers, a value head on the game
token and a policy head that scores each legal action. This script and
src/net.rs must stay in step; the weights file carries a few positions with
this script's outputs, and src/net.rs checks it computes the same when loading.

Runs on a GPU if there is one (--device cuda), otherwise on the CPU.
Needs PyTorch and numpy.
"""

import argparse
import collections
import json
import math
import os
import random
import sys
import time

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F


# --------------------------------------------------------------------------
# Data


class Sample:
    """One decision, as flat arrays: each kind of token's ids and values
    concatenated, with the number of features in each token."""
    __slots__ = ("g_i", "g_v", "u_i", "u_v", "u_n", "a_i", "a_v", "a_n", "au", "at", "pi", "z", "q", "seed")


def flatten(tokens):
    ids = np.array([i for t in tokens for i in t["i"]], dtype=np.int32)
    vals = np.array([v for t in tokens for v in t["v"]], dtype=np.float32)
    lengths = np.array([len(t["i"]) for t in tokens], dtype=np.int32)
    return ids, vals, lengths


def load(paths, max_samples=None):
    # With a limit, keep the newest (the files are read in order).
    samples, layout = collections.deque(maxlen=max_samples), None
    for path in paths:
        with open(path) as f:
            for line in f:
                if not line.strip():
                    continue
                r = json.loads(line)
                if layout is None:
                    layout = r["layout"]
                elif r["layout"] != layout:
                    sys.exit(f"{path}: samples from different encodings ({r['layout']} vs {layout})")
                visits = np.array(r["visits"], dtype=np.float32)
                if visits.sum() <= 0 or len(visits) < 2:
                    continue
                s = Sample()
                obs = r["obs"]
                s.g_i = np.array(obs["g"]["i"], dtype=np.int32)
                s.g_v = np.array(obs["g"]["v"], dtype=np.float32)
                s.u_i, s.u_v, s.u_n = flatten(obs["u"])
                s.a_i, s.a_v, s.a_n = flatten([a["t"] for a in r["actions"]])
                s.au = np.array([a["u"] for a in r["actions"]], dtype=np.int64)
                s.at = np.array([a["g"] for a in r["actions"]], dtype=np.int64)
                s.pi = visits / visits.sum()
                s.z = float(r["z"])
                s.q = float(r.get("q", 0.0))
                s.seed = r["seed"]
                samples.append(s)
    return list(samples), layout


def offsets_of(lengths):
    out = np.zeros(len(lengths), dtype=np.int64)
    if len(lengths):
        out[1:] = np.cumsum(lengths)[:-1]
    return out


def collate(batch, device):
    """Pads a list of samples into tensors."""
    b = len(batch)
    n_units = max(1, max(len(s.u_n) for s in batch))
    n_actions = max(len(s.a_n) for s in batch)
    au = np.full((b, n_actions), -1, dtype=np.int64)
    at = np.full((b, n_actions), -1, dtype=np.int64)
    pi = np.zeros((b, n_actions), dtype=np.float32)
    amask = np.zeros((b, n_actions), dtype=bool)
    umask = np.zeros((b, n_units), dtype=bool)
    unit_slot, action_slot = [], []
    for k, s in enumerate(batch):
        nu, na = len(s.u_n), len(s.a_n)
        unit_slot.append(k * n_units + np.arange(nu))
        action_slot.append(k * n_actions + np.arange(na))
        umask[k, :nu] = True
        au[k, :na] = s.au
        at[k, :na] = s.at
        pi[k, :na] = s.pi
        amask[k, :na] = True

    def t(x, dtype=None):
        x = torch.from_numpy(np.ascontiguousarray(x))
        return (x.to(dtype) if dtype else x).to(device)

    def bag(ids, vals, lengths):
        return [t(np.concatenate(ids), torch.int64), t(offsets_of(np.concatenate(lengths))),
                t(np.concatenate(vals))]

    return {
        "g": bag([s.g_i for s in batch], [s.g_v for s in batch], [[len(s.g_i)] for s in batch]),
        "u": bag([s.u_i for s in batch], [s.u_v for s in batch], [s.u_n for s in batch]),
        "u_slot": t(np.concatenate(unit_slot)),
        "umask": t(umask),
        "a": bag([s.a_i for s in batch], [s.a_v for s in batch], [s.a_n for s in batch]),
        "a_slot": t(np.concatenate(action_slot)),
        "au": t(au),
        "at": t(at),
        "amask": t(amask),
        "pi": t(pi),
        "z": torch.tensor([s.z for s in batch], dtype=torch.float32, device=device),
        "q": torch.tensor([s.q for s in batch], dtype=torch.float32, device=device),
    }


# --------------------------------------------------------------------------
# Model (mirrored in src/net.rs)


class Block(nn.Module):
    def __init__(self, d, heads, ff):
        super().__init__()
        self.heads = heads
        self.norm1 = nn.LayerNorm(d)
        self.q, self.k, self.v, self.o = (nn.Linear(d, d) for _ in range(4))
        self.norm2 = nn.LayerNorm(d)
        self.ff1 = nn.Linear(d, ff)
        self.ff2 = nn.Linear(ff, d)

    def forward(self, x, mask):
        b, n, d = x.shape
        h, dh = self.heads, d // self.heads
        y = self.norm1(x)
        q = self.q(y).view(b, n, h, dh).transpose(1, 2)
        k = self.k(y).view(b, n, h, dh).transpose(1, 2)
        v = self.v(y).view(b, n, h, dh).transpose(1, 2)
        att = (q @ k.transpose(-1, -2)) / math.sqrt(dh)
        att = att.masked_fill(~mask[:, None, None, :], float("-inf")).softmax(-1)
        x = x + self.o((att @ v).transpose(1, 2).reshape(b, n, d))
        return x + self.ff2(F.relu(self.ff1(self.norm2(x))))


class Net(nn.Module):
    def __init__(self, layout, d=64, heads=4, layers=2, ff=128):
        super().__init__()
        self.config = {"d": d, "heads": heads, "layers": layers, "ff": ff}
        self.layout = layout
        self.emb_global = nn.EmbeddingBag(layout["global"], d, mode="sum")
        self.emb_unit = nn.EmbeddingBag(layout["unit"], d, mode="sum")
        self.emb_action = nn.EmbeddingBag(layout["action"], d, mode="sum")
        for e in (self.emb_global, self.emb_unit, self.emb_action):
            nn.init.normal_(e.weight, std=0.05)
        self.blocks = nn.ModuleList(Block(d, heads, ff) for _ in range(layers))
        self.norm = nn.LayerNorm(d)
        self.value1 = nn.Linear(d, d)
        self.value2 = nn.Linear(d, 1)
        self.from_unit = nn.Linear(d, d, bias=False)
        self.from_target = nn.Linear(d, d, bias=False)
        self.from_global = nn.Linear(d, d, bias=False)
        self.policy1 = nn.Linear(d, d)
        self.policy2 = nn.Linear(d, 1)

    def forward(self, x):
        b, n_units = x["umask"].shape
        d = self.config["d"]
        g = self.emb_global(x["g"][0], x["g"][1], per_sample_weights=x["g"][2])
        u = torch.zeros(b * n_units, d, device=g.device)
        if len(x["u_slot"]):
            u[x["u_slot"]] = self.emb_unit(x["u"][0], x["u"][1], per_sample_weights=x["u"][2])
        tokens = torch.cat([g[:, None, :], u.view(b, n_units, d)], dim=1)
        mask = torch.cat([torch.ones(b, 1, dtype=torch.bool, device=g.device), x["umask"]], dim=1)
        for block in self.blocks:
            tokens = block(tokens, mask)
        tokens = self.norm(tokens)
        hg, hu = tokens[:, 0], tokens[:, 1:]
        value = torch.tanh(self.value2(F.relu(self.value1(hg)))).squeeze(-1)

        n_actions = x["amask"].shape[1]
        e = torch.zeros(b * n_actions, d, device=g.device)
        e[x["a_slot"]] = self.emb_action(x["a"][0], x["a"][1], per_sample_weights=x["a"][2])
        e = e.view(b, n_actions, d) + self.from_global(hg)[:, None, :]

        def pointed(proj, ptr):
            idx = ptr.clamp(min=0)
            picked = torch.gather(proj(hu), 1, idx[..., None].expand(-1, -1, d))
            return picked * (ptr >= 0)[..., None]

        e = e + pointed(self.from_unit, x["au"]) + pointed(self.from_target, x["at"])
        logits = self.policy2(F.relu(self.policy1(F.relu(e)))).squeeze(-1)
        logits = logits.masked_fill(~x["amask"], float("-inf"))
        return value, logits


def export(model, path, checks):
    tensors = {}
    for name, p in model.state_dict().items():
        tensors[name] = {"shape": list(p.shape), "data": [round(float(v), 7) for v in p.flatten().cpu()]}
    with open(path, "w") as f:
        json.dump({"layout": model.layout, "config": model.config, "tensors": tensors, "checks": checks}, f)


def make_checks(model, samples, device):
    """A few positions with this script's outputs, for src/net.rs to verify."""
    def tokens(ids, vals, lengths):
        out, at = [], 0
        for n in lengths:
            out.append({"i": ids[at:at + n].tolist(), "v": vals[at:at + n].tolist()})
            at += n
        return out

    checks = []
    model.eval()
    with torch.no_grad():
        for s in samples[:3]:
            v, logits = model(collate([s], device))
            checks.append({
                "obs": {"g": {"i": s.g_i.tolist(), "v": s.g_v.tolist()}, "u": tokens(s.u_i, s.u_v, s.u_n)},
                "actions": [{"t": t, "u": int(u), "g": int(g)}
                            for t, u, g in zip(tokens(s.a_i, s.a_v, s.a_n), s.au, s.at)],
                "value": float(v[0]),
                "logits": [float(l) for l in logits[0]],
            })
    return checks


# --------------------------------------------------------------------------
# Training


def run_epoch(model, data, batch_size, device, opt=None, q_mix=0.0, value_weight=1.0):
    training = opt is not None
    model.train(training)
    order = list(range(len(data)))
    if training:
        random.shuffle(order)
    totals = {"policy": 0.0, "value": 0.0, "outcome": 0.0, "top1": 0.0, "entropy": 0.0, "n": 0}
    for start in range(0, len(order), batch_size):
        batch = [data[i] for i in order[start:start + batch_size]]
        x = collate(batch, device)
        with torch.set_grad_enabled(training):
            value, logits = model(x)
            logp = logits.log_softmax(-1).masked_fill(~x["amask"], 0.0)
            policy_loss = -(x["pi"] * logp).sum(-1).mean()
            target = (1 - q_mix) * x["z"] + q_mix * x["q"]
            value_loss = F.mse_loss(value, target)
            loss = policy_loss + value_weight * value_loss
            if training:
                opt.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(model.parameters(), 1.0)
                opt.step()
        n = len(batch)
        totals["policy"] += policy_loss.item() * n
        totals["value"] += value_loss.item() * n
        # How well the value predicts who won, whatever the target (guessing 0 scores 1).
        totals["outcome"] += F.mse_loss(value, x["z"], reduction="sum").item()
        pi = x["pi"]
        totals["top1"] += (logits.argmax(-1) == pi.argmax(-1)).float().sum().item()
        ent = -(pi * torch.log(pi.clamp(min=1e-9))).sum(-1)
        totals["entropy"] += ent.sum().item()
        totals["n"] += n
    n = max(totals.pop("n"), 1)
    return {k: v / n for k, v in totals.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("files", nargs="+")
    parser.add_argument("--out", required=True, help="where to write the weights (JSON, for selfplay --net)")
    parser.add_argument("--init", help="start from these weights (eg the previous generation)")
    parser.add_argument("--epochs", type=int, default=10)
    parser.add_argument("--batch", type=int, default=256)
    parser.add_argument("--lr", type=float, default=1e-3)
    parser.add_argument("--weight-decay", type=float, default=1e-4)
    parser.add_argument("--d", type=int, default=64)
    parser.add_argument("--heads", type=int, default=4)
    parser.add_argument("--layers", type=int, default=2)
    parser.add_argument("--ff", type=int, default=128)
    parser.add_argument("--q-mix", type=float, default=0.5,
                        help="value target: (1-m)·outcome + m·search value (default 0.5). Every position in "
                             "a game has the same outcome, so with few games the network can memorise "
                             "which deals were won; the search's value varies within a game")
    parser.add_argument("--value-weight", type=float, default=0.5)
    parser.add_argument("--max-samples", type=int, default=None, help="only read this many (newest last)")
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--threads", type=int, default=None,
                        help="CPU threads for PyTorch (default: half the cores). Keep it below the number "
                             "of idle cores: this network's operations are small, and when a thread "
                             "waits for a core, training can slow down twentyfold")
    args = parser.parse_args()
    if args.threads:
        torch.set_num_threads(args.threads)
    elif args.device == "cpu":
        torch.set_num_threads(max(1, (os.cpu_count() or 2) // 2))
    random.seed(0)
    torch.manual_seed(0)

    t0 = time.time()
    samples, layout = load(args.files, args.max_samples)
    if not samples:
        sys.exit("no samples")
    # Hold out every tenth game, so the held-out positions are from unseen games.
    held = [s for s in samples if s.seed % 10 == 0]
    train = [s for s in samples if s.seed % 10 != 0]
    print(f"{len(samples)} samples ({len(train)} for training, {len(held)} held out), "
          f"read in {time.time() - t0:.0f}s; layout {layout}")

    if args.init:
        with open(args.init) as f:
            init = json.load(f)
        c = init["config"]
        model = Net(layout, c["d"], c["heads"], c["layers"], c["ff"])
        state = {k: torch.tensor(v["data"], dtype=torch.float32).view(v["shape"]) for k, v in init["tensors"].items()}
        model.load_state_dict(state)
    else:
        model = Net(layout, args.d, args.heads, args.layers, args.ff)
    model.to(args.device)
    params = sum(p.numel() for p in model.parameters())
    print(f"model {model.config}, {params} parameters, on {args.device}")
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=args.weight_decay)
    steps = args.epochs * math.ceil(len(train) / args.batch)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=args.lr, total_steps=max(steps, 1), pct_start=0.1)

    class Stepper:
        def zero_grad(self):
            opt.zero_grad()

        def step(self):
            opt.step()
            sched.step()

    # Keep the epoch that does best on the held-out games.
    best = (float("inf"), None, 0)
    for epoch in range(args.epochs):
        t = time.time()
        tr = run_epoch(model, train, args.batch, args.device, Stepper(), args.q_mix, args.value_weight)
        ho = run_epoch(model, held, args.batch, args.device, None, args.q_mix) if held else None
        line = (f"epoch {epoch + 1}: train policy {tr['policy']:.3f} (target entropy {tr['entropy']:.3f}) "
                f"top-1 {100 * tr['top1']:.1f}% value {tr['value']:.3f} outcome {tr['outcome']:.3f}")
        if ho:
            line += (f" | held out policy {ho['policy']:.3f} (entropy {ho['entropy']:.3f}) "
                     f"top-1 {100 * ho['top1']:.1f}% value {ho['value']:.3f} outcome {ho['outcome']:.3f}")
            score = ho["policy"] + args.value_weight * ho["value"]
            if score < best[0]:
                best = (score, {k: v.detach().clone() for k, v in model.state_dict().items()}, epoch + 1)
        print(line + f" | {time.time() - t:.0f}s", flush=True)
    if best[1] is not None:
        model.load_state_dict(best[1])
        print(f"keeping epoch {best[2]}")

    model.cpu()
    export(model, args.out, make_checks(model, held or train, "cpu"))
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
