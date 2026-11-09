#!/usr/bin/env bash
# Run the standard balance experiments: the default rules, and the second
# player's starting points at 0 and 2, on the same deals (seeds), so the
# variants can be compared game by game.
#
#   tools/balance_suite.sh [OUT_DIR] [GAMES] [AGENT]
#
# Defaults: results/, 400 games per variant, mcts:400x4 for both players.
# Results go to OUT_DIR/*.jsonl (one line per game) and the default-rules
# positions to OUT_DIR/positions.jsonl (for tools/fit_eval.py). Summarise with
#   python3 tools/analyze.py OUT_DIR/p2-*.jsonl
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-results}
games=${2:-400}
agent=${3:-mcts:400x4}
mkdir -p "$out"
cargo build --release --bin selfplay
run() {
    local label=$1
    shift
    echo "== $label: $games games, $agent vs $agent"
    ./target/release/selfplay --games "$games" --agents "$agent,$agent" --max-turns 60 \
        --variant "$label" --out "$out/$label.jsonl" "$@" 2>> "$out/$label.log"
}
run p2-1 --positions "$out/positions.jsonl"
run p2-0 --second-player-points 0
run p2-2 --second-player-points 2
python3 tools/analyze.py "$out"/p2-*.jsonl --min-games 30 > "$out/summary.txt"
echo "== summary in $out/summary.txt"
