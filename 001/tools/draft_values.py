#!/usr/bin/env python3
"""How much does the bot want each card? Fitted from its draft decisions.

    python3 tools/draft_values.py training/gen5.jsonl [more.jsonl ...] [--phase draft|draw|both]

Reads `selfplay --samples` files. At every decision where the player could take
a faceup card, the search spread its visits over the cards on offer (and the
deck). This fits a choice model to those visits: each card gets a score, and
the chance of picking a card is proportional to exp(score) among what was on
offer. Scores are relative to drawing blind from the deck (0), so a positive
score means the bot prefers the card to an unknown one. Because it compares
cards only against what else was available, it isn't skewed by which cards
happened to come up.

Only meaningful for bots that judge cards in hand, ie the network bots
(mctsN). The hand-made evaluation counts cards in hand without caring which,
so its drafting is close to random.
"""

import argparse
import gzip
import json
import math
import os
import sys

import numpy as np

# Feature ids from src/encode.rs: action id 0 is the bias, then 11 action
# kinds (TakeFaceup is the first, TakeDeck the second), then one per card.
A_KIND = 1
TAKE_FACEUP, TAKE_DECK = A_KIND + 0, A_KIND + 1
A_POWER = A_KIND + 11
# Global id 0 is the bias, then the phase: draft, draw, main, over.
G_DRAFT, G_DRAW = 1, 2


def card_names():
    import re
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "powers.md")
    names = []
    with open(path) as f:
        for line in f:
            m = re.match(r"\*([^*]+)\*\.", line.strip())
            if m:
                names.append(m.group(1).strip())
    return names


def read(paths, phase):
    """Each decision as (offered option ids, visit shares). Option id = card, or -1 for the deck."""
    decisions = []
    for path in paths:
        opener = gzip.open if path.endswith(".gz") else open
        with opener(path, "rt") as f:
            for line in f:
                r = json.loads(line)
                g = set(r["obs"]["g"]["i"])
                in_draft, in_draw = G_DRAFT in g, G_DRAW in g
                if not ((phase in ("draft", "both") and in_draft) or (phase in ("draw", "both") and in_draw)):
                    continue
                options, visits = [], []
                for a, v in zip(r["actions"], r["visits"]):
                    ids = a["t"]["i"]
                    if TAKE_FACEUP in ids:
                        cards = [i - A_POWER for i in ids if i >= A_POWER and i < A_POWER + r["layout"]["cards"]]
                        if not cards:
                            continue
                        options.append(cards[0])
                    elif TAKE_DECK in ids:
                        options.append(-1)
                    else:
                        continue
                    visits.append(v)
                total = sum(visits)
                if len(options) >= 2 and total > 0:
                    decisions.append((options, [v / total for v in visits]))
    return decisions


def fit(decisions, cards, l2=0.05, iterations=3000, lr=2.0):
    """Conditional logit with soft targets, by gradient ascent; the deck's score is fixed at 0."""
    width = max(len(o) for o, _ in decisions)
    deck, pad = cards, cards + 1
    options = np.full((len(decisions), width), pad)
    targets = np.zeros((len(decisions), width))
    for k, (o, t) in enumerate(decisions):
        options[k, :len(o)] = [deck if x < 0 else x for x in o]
        targets[k, :len(t)] = t
    score = np.zeros(cards + 2)
    for _ in range(iterations):
        score[deck], score[pad] = 0.0, -np.inf
        s = score[options]
        p = np.exp(s - s.max(axis=1, keepdims=True))
        p /= p.sum(axis=1, keepdims=True)
        grad = np.zeros(cards + 2)
        np.add.at(grad, options, targets - p)
        grad = grad[:cards] / len(decisions) - l2 * score[:cards] / len(decisions)
        score[:cards] += lr * grad * cards
    return score[:cards]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("files", nargs="+")
    parser.add_argument("--phase", default="both", choices=["draft", "draw", "both"])
    args = parser.parse_args()
    names = card_names()
    decisions = read(args.files, args.phase)
    if not decisions:
        sys.exit("no decisions with faceup cards to choose from")
    score = fit(decisions, len(names))
    offered = np.zeros(len(names))
    taken = np.zeros(len(names))
    for options, shares in decisions:
        best = options[int(np.argmax(shares))]
        for o in options:
            if o >= 0:
                offered[o] += 1
        if best >= 0:
            taken[best] += 1
    deck_best = sum(1 for o, s in decisions if o[int(np.argmax(s))] == -1)
    print(f"{len(decisions)} decisions ({args.phase}); the deck was the favourite in {deck_best}\n")
    print(f"  {'card':<14} {'score':>6} {'offered':>8} {'favourite':>10}")
    for i in np.argsort(-score):
        if offered[i] == 0:
            continue
        print(f"  {names[i]:<14} {score[i]:+6.2f} {int(offered[i]):>8} {100 * taken[i] / offered[i]:>9.0f}%")


if __name__ == "__main__":
    main()
