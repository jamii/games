#!/usr/bin/env python3
"""Rebuilds the generation table in BOT.md from results/stage2 (see
tools/snapshot_training.sh): each network's games against mcts:400x4 and
against the previous network, and a running Elo estimate."""
import json
import math
import os
import re

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
stage2 = os.path.join(root, "results", "stage2")


def wins(path, prefix):
    games = [json.loads(l) for l in open(path)]
    done = [g for g in games if g["winner"] is not None]
    won = sum(1 for g in done if g["agents"][g["winner"]].startswith(prefix))
    return won, len(done)


rows, elo = [], 0.0
for g in range(0, 100):
    ev = os.path.join(stage2, f"eval{g}.jsonl")
    if not os.path.exists(ev):
        break
    w, n = wins(ev, "mctsN")
    h2h = os.path.join(stage2, f"h2h-{g}-{g - 1}.jsonl")
    vs_prev, elo_text = "", "0" if g == 0 else ""
    if g > 0 and os.path.exists(h2h):
        hw, hn = wins(h2h, f"mctsN:200x4@training/net{g}.json")
        p = min(max(hw / hn, 0.01), 0.99)
        elo += 400 * math.log10(p / (1 - p))
        vs_prev = f"{hw}–{hn - hw}"
        elo_text = f"{elo:+.0f}"
    rows.append(f"| net{g} | {w}–{n - w} | {vs_prev} | {elo_text} |")

table = ("| network | against `mcts:400x4` | against the previous network | Elo vs net0 (chained) |\n"
         "|---|---|---|---|\n" + "\n".join(rows))
path = os.path.join(root, "BOT.md")
text = open(path).read()
text = re.sub(r"<!-- generations -->.*<!-- /generations -->",
              lambda _: f"<!-- generations -->\n{table}\n<!-- /generations -->", text, flags=re.S)
open(path, "w").write(text)
print(table)
