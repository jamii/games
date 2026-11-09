#!/usr/bin/env bash
# What is a card worth? Plays the same deals twice: once with the card left
# out of the deck, once with it left out but given to the second player when
# the draft ends. The difference in the second player's win rate, game by
# game, is the value of having that card in hand (on top of a normal draft).
#
#   tools/card_value.sh CARD [GAMES] [AGENT] [OUT_DIR]
#
# Defaults: 200 games, mcts:400x4 for both players, results in cards/.
set -euo pipefail
cd "$(dirname "$0")/.."
card=${1:?usage: tools/card_value.sh CARD [GAMES] [AGENT] [OUT_DIR]}
games=${2:-200}
agent=${3:-mcts:400x4}
out=${4:-cards}
mkdir -p "$out"
cargo build --release --bin selfplay 2> /dev/null
for mode in remove give; do
    file="$out/$card-$mode.jsonl"
    if [ ! -s "$file" ]; then
        if [ "$mode" = remove ]; then flag=(--remove "$card"); else flag=(--give "second:$card"); fi
        ./target/release/selfplay --games "$games" --first-seed 50000 --agents "$agent,$agent" --max-turns 60 \
            --variant "$card-$mode" "${flag[@]}" --out "$file.part" 2> "$out/$card-$mode.log"
        mv "$file.part" "$file"
    fi
done
python3 - "$card" "$out/$card-remove.jsonl" "$out/$card-give.jsonl" <<'EOF'
import json, math, sys
card, without, with_ = sys.argv[1:]
def second_wins(path):
    out = {}
    for line in open(path):
        g = json.loads(line)
        if g["winner"] is not None:
            out[g["seed"]] = int(g["winner"] != g["first_player"])
    return out
a, b = second_wins(without), second_wins(with_)
seeds = sorted(set(a) & set(b))
diffs = [b[s] - a[s] for s in seeds]
n = len(diffs)
mean = sum(diffs) / n
sd = math.sqrt(sum((d - mean) ** 2 for d in diffs) / max(n - 1, 1))
print(f"{card}: second player wins {100 * sum(a[s] for s in seeds) / n:.0f}% without it, "
      f"{100 * sum(b[s] for s in seeds) / n:.0f}% with it in hand: "
      f"{100 * mean:+.1f} points ± {196 * sd / math.sqrt(n):.1f} (95%, {n} paired games)")
EOF
