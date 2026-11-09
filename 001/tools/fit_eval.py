#!/usr/bin/env python3
"""Learn an evaluation function from self-play positions.

    python3 tools/fit_eval.py positions.jsonl [more.jsonl ...] --out weights.json

The positions come from `selfplay --positions FILE`: the start of every turn,
from both players' points of view, labelled with whether that player won. This
fits a logistic regression (L2-regularised) predicting the winner from the
features in `bot::features`. Bots load the result with `selfplay --weights`
(agents greedyL and mctsL).

It also prints the learned value of each card: how much having it on one of
your units (rather than on one of the opponent's) shifts the predicted chance
of winning. That's another measure of card strength, less biased than raw win
rates because it controls for the score and board position.
"""

import argparse
import json
import sys

import numpy as np

sys.path.insert(0, __file__.rsplit("/", 1)[0])


def load(paths):
    xs, ys, games = [], [], []
    for path in paths:
        with open(path) as f:
            for line in f:
                if line.strip():
                    r = json.loads(line)
                    xs.append(r["x"])
                    ys.append(r["y"])
                    games.append(r.get("game", len(games)))
    return np.array(xs, dtype=np.float64), np.array(ys, dtype=np.float64)


def fit(x, y, l2, iterations=30):
    """Newton's method for L2-regularised logistic regression (bias not regularised)."""
    n, d = x.shape
    a = np.hstack([x, np.ones((n, 1))])
    w = np.zeros(d + 1)
    reg = np.full(d + 1, l2)
    reg[-1] = 0.0
    for _ in range(iterations):
        p = 1 / (1 + np.exp(-(a @ w)))
        grad = a.T @ (p - y) + reg * w
        hess = (a * (p * (1 - p))[:, None]).T @ a + np.diag(reg) + 1e-9 * np.eye(d + 1)
        step = np.linalg.solve(hess, grad)
        w -= step
        if np.abs(step).max() < 1e-8:
            break
    return w[:-1], w[-1]


def evaluate(x, y, w, b):
    p = 1 / (1 + np.exp(-(x @ w + b)))
    p = np.clip(p, 1e-6, 1 - 1e-6)
    loss = -np.mean(y * np.log(p) + (1 - y) * np.log(1 - p))
    accuracy = np.mean((p > 0.5) == (y > 0.5))
    return loss, accuracy


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("files", nargs="+")
    parser.add_argument("--out", required=True, help="where to write the weights (JSON)")
    parser.add_argument("--l2", type=float, default=2.0, help="regularisation strength")
    parser.add_argument("--powers", default=None,
                        help="powers.md, for card names (default: the one next to this script)")
    args = parser.parse_args()

    x, y = load(args.files)
    n, d = x.shape
    names = card_names(args.powers)
    base = d - 2 * len(names)
    if base < 0:
        sys.exit(f"{d} features but {len(names)} cards: was powers.md changed since the positions were recorded?")

    # Hold out the last fifth (positions are grouped by game, so this is roughly by game).
    split = int(n * 0.8)
    w, b = fit(x[:split], y[:split], args.l2)
    train = evaluate(x[:split], y[:split], w, b)
    test = evaluate(x[split:], y[split:], w, b)
    baseline = -np.mean(y[split:]) * np.log(max(y.mean(), 1e-6)) - (1 - np.mean(y[split:])) * np.log(max(1 - y.mean(), 1e-6))
    print(f"{n} positions, {d} features")
    print(f"train: log-loss {train[0]:.3f}, accuracy {100 * train[1]:.1f}%")
    print(f"held out: log-loss {test[0]:.3f} (always guessing the base rate: {baseline:.3f}), "
          f"accuracy {100 * test[1]:.1f}%")

    # Refit on everything for the weights we keep.
    w, b = fit(x, y, args.l2)
    with open(args.out, "w") as f:
        json.dump({"weights": w.tolist(), "bias": float(b), "positions": n}, f)
    print(f"wrote {args.out}")

    base_names = ["points ahead", "scoring now", "flag margin", "presence", "units", "powers", "mobility", "hand",
                  "my turn"][:base]
    print("\nGeneral features (log-odds per unit):")
    for name, weight in zip(base_names, w[:base]):
        print(f"  {name:<14} {weight:+.3f}")

    on_units = w[base:base + len(names)]
    in_hand = w[base + len(names):]
    counts = (x[:, base:base + len(names)] != 0).sum(axis=0)
    order = np.argsort(-on_units)
    print("\nCards, by learned value on a unit (log-odds of winning; positions where the card was on a unit)")
    print(f"  {'card':<14} {'on a unit':>9} {'in hand':>8} {'positions':>10}")
    for i in order:
        if counts[i] == 0:
            continue
        print(f"  {names[i]:<14} {on_units[i]:+9.3f} {in_hand[i]:+8.3f} {int(counts[i]):>10}")


def card_names(path):
    import os
    import re
    path = path or os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "powers.md")
    names = []
    with open(path) as f:
        for line in f:
            m = re.match(r"\*([^*]+)\*\.", line.strip())
            if m:
                names.append(m.group(1).strip())
    return names


if __name__ == "__main__":
    main()
