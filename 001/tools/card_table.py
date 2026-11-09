#!/usr/bin/env python3
"""Rebuilds the card value table in BOT.md from cards/ (tools/card_value.sh),
next to the two correlational measures from the stage 1 results: relative
lift (results/p2-*.jsonl) and the learned value (results/weights.json)."""
import collections
import glob
import json
import math
import os
import re

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")


def load(path):
    return [json.loads(l) for l in open(path) if l.strip()]


def card_names():
    names = []
    for line in open(os.path.join(root, "powers.md")):
        m = re.match(r"\*([^*]+)\*\.", line.strip())
        if m:
            names.append(m.group(1).strip())
    return names


# Relative lift, as in tools/analyze.py.
games = [g for p in glob.glob(os.path.join(root, "results", "p2-*.jsonl")) for g in load(p)]
by_variant = collections.defaultdict(list)
for g in games:
    by_variant[g["variant"]].append(g)
first_rate = {}
for v, gs in by_variant.items():
    done = [g for g in gs if g["winner"] is not None]
    first_rate[v] = sum(g["winner"] == g["first_player"] for g in done) / len(done)
lift, count = collections.defaultdict(float), collections.Counter()
for g in games:
    if g["winner"] is None:
        continue
    for seat in (0, 1):
        p = first_rate[g["variant"]]
        expected = p if seat == g["first_player"] else 1 - p
        for card in set(g["applied"][seat]):
            lift[card] += (g["winner"] == seat) - expected
            count[card] += 1
mean = sum(lift.values()) / sum(count.values())

names = card_names()
weights = json.load(open(os.path.join(root, "results", "weights.json")))["weights"]
learned = dict(zip(names, weights[9:9 + len(names)]))

rows = []
for remove in sorted(glob.glob(os.path.join(root, "cards", "*-remove.jsonl"))):
    card = os.path.basename(remove)[:-len("-remove.jsonl")]
    give = remove.replace("-remove.jsonl", "-give.jsonl")
    if not os.path.exists(give):
        continue
    def second(path):
        return {g["seed"]: int(g["winner"] != g["first_player"]) for g in load(path) if g["winner"] is not None}
    a, b = second(remove), second(give)
    seeds = sorted(set(a) & set(b))
    d = [b[s] - a[s] for s in seeds]
    m = sum(d) / len(d)
    sd = math.sqrt(sum((x - m) ** 2 for x in d) / (len(d) - 1))
    half = 1.96 * sd / math.sqrt(len(d))
    worth = f"{100 * m:+.1f} ± {100 * half:.1f}"
    if abs(m) > half:
        worth = f"**{worth}**"
    rel = 100 * (lift[card] / count[card] - mean) if count[card] else float("nan")
    rows.append((m, f"| {card} | {worth} | {rel:+.0f} | {learned.get(card, float('nan')):+.2f} |"))
rows.sort(key=lambda r: -r[0])
table = ("| card | worth in hand (points) | relative lift (points) | learned value (log-odds) |\n"
         "|---|---|---|---|\n" + "\n".join(r for _, r in rows))
path = os.path.join(root, "BOT.md")
text = open(path).read()
text = re.sub(r"<!-- cards -->.*<!-- /cards -->", lambda _: f"<!-- cards -->\n{table}\n<!-- /cards -->", text,
              flags=re.S)
open(path, "w").write(text)
print(table)
