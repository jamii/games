#!/usr/bin/env python3
"""Summarise self-play results (the JSON lines written by the selfplay binary).

    python3 tools/analyze.py games.jsonl [more.jsonl ...] [--min-games 20] [--markdown]

For each variant (a set of rule options) it reports the first player's win
rate and game length. For each card it reports how often it was drafted and
how early, how often it was applied, and its *lift*: how much more often the
players who applied it won than their seat order alone would predict. Lift is
correlational: players who are ahead get more turns in which to apply cards,
so most cards have positive lift. Compare cards by *relative lift* (lift minus
the average over all cards), and treat it as a pointer for experiments like
removing the card (`selfplay --remove`), not as a verdict.
"""

import argparse
import collections
import json
import math


def wilson(k, n, z=1.96):
    """95% confidence interval for a proportion."""
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (centre - half, centre + half)


def pct(x):
    return f"{100 * x:.0f}%"


def table(rows, headers, markdown):
    rows = [[str(c) for c in r] for r in rows]
    widths = [max(len(h), *(len(r[i]) for r in rows)) if rows else len(h) for i, h in enumerate(headers)]
    if markdown:
        out = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
        out += ["| " + " | ".join(r) + " |" for r in rows]
    else:
        fmt = "  ".join("{:<%d}" % w for w in widths)
        out = [fmt.format(*headers), fmt.format(*("-" * w for w in widths))]
        out += [fmt.format(*r) for r in rows]
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("files", nargs="+")
    parser.add_argument("--min-games", type=int, default=20, help="only list cards applied in at least this many games")
    parser.add_argument("--markdown", action="store_true", help="print markdown tables")
    args = parser.parse_args()

    games = []
    for path in args.files:
        with open(path) as f:
            games += [json.loads(line) for line in f if line.strip()]
    by_variant = collections.defaultdict(list)
    for g in games:
        by_variant[g.get("variant", "?")].append(g)

    # --- Variants ---------------------------------------------------------
    rows = []
    first_rate = {}
    for variant, gs in sorted(by_variant.items()):
        finished = [g for g in gs if g["winner"] is not None]
        k = sum(g["winner"] == g["first_player"] for g in finished)
        n = len(finished)
        first_rate[variant] = k / n if n else 0.5
        lo, hi = wilson(k, n)
        turns = sum(g["turns"] for g in finished) / max(n, 1)
        secs = sum(g["seconds"] for g in gs) / len(gs)
        agents = collections.Counter()
        for g in finished:
            agents[g["agents"][g["winner"]]] += 1
        rows.append([variant, len(gs), len(gs) - n, f"{pct(k / max(n, 1))} ({pct(lo)}–{pct(hi)})", f"{turns:.1f}",
                     f"{secs:.0f}s", ", ".join(f"{a} {w}" for a, w in agents.most_common())])
    print("Variants\n")
    print(table(rows, ["variant", "games", "unfinished", "first player wins (95% CI)", "turns", "time/game", "wins"],
                args.markdown))

    # Paired comparison: the same seeds under different variants.
    if len(by_variant) > 1:
        seeds = collections.defaultdict(dict)
        for variant, gs in by_variant.items():
            for g in gs:
                seeds[g["seed"]][variant] = g
        print("\nPaired seeds (same deals) per variant: "
              + ", ".join(f"{v}: {sum(1 for s in seeds.values() if v in s)}" for v in sorted(by_variant)))

    # --- Cards ------------------------------------------------------------
    stats = collections.defaultdict(lambda: collections.Counter())
    pick_positions = collections.defaultdict(list)
    lift_sum = collections.defaultdict(float)
    lift_var = collections.defaultdict(float)
    used = collections.Counter()
    finished = [g for g in games if g["winner"] is not None]
    for g in finished:
        p_first = first_rate[g.get("variant", "?")]
        for seat in (0, 1):
            won = g["winner"] == seat
            expected = p_first if seat == g["first_player"] else 1 - p_first
            for i, card in enumerate(g["drafted"][seat]):
                stats[card]["drafted"] += 1
                pick_positions[card].append(i + 1)
            for card in g["taken_faceup"][seat]:
                stats[card]["taken"] += 1
            for card in set(g["applied"][seat]):
                stats[card]["applied"] += 1
                stats[card]["wins"] += won
                lift_sum[card] += won - expected
                lift_var[card] += expected * (1 - expected)
            for power, n in g["used"][seat].items():
                used[power] += n
    total_n = sum(s["applied"] for s in stats.values())
    mean_lift = sum(lift_sum.values()) / max(total_n, 1)
    rows = []
    for card, s in stats.items():
        n = s["applied"]
        if n < args.min_games:
            continue
        lift = lift_sum[card] / n
        se = math.sqrt(lift_var[card]) / n
        picks = pick_positions[card]
        rows.append([card, s["drafted"], f"{sum(picks) / len(picks):.2f}" if picks else "-", s["taken"], n,
                     pct(s["wins"] / n), f"{100 * lift:+.0f}", f"{100 * (lift - mean_lift):+.0f} ± {196 * se:.0f}",
                     used.get(card, 0)])
    rows.sort(key=lambda r: -float(r[7].split()[0]))
    print(f"\nCards applied in at least {args.min_games} games ({len(finished)} finished games). Lift is the "
          "win rate of players who applied the card above what seat order predicts, in percentage points; "
          f"relative lift subtracts the average over all cards ({100 * mean_lift:+.0f}). ± is a 95% interval.\n")
    print(table(rows, ["card", "drafted", "avg pick", "taken faceup", "applied", "win rate", "lift",
                       "relative lift", "times used"], args.markdown))

    print("\nMost used powers (activations per finished game)\n")
    rows = [[p, f"{n / max(len(finished), 1):.2f}"] for p, n in used.most_common(15)]
    print(table(rows, ["power", "per game"], args.markdown))


if __name__ == "__main__":
    main()
