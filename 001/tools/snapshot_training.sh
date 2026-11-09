#!/usr/bin/env bash
# Copies the small results of a tools/alphazero.sh run into the repository
# (results/stage2/): progress, training logs, evaluation and head-to-head
# games, and the self-play game records (compressed). Not the samples or
# the networks, which are big.
#
#   tools/snapshot_training.sh [TRAINING_DIR]
set -euo pipefail
cd "$(dirname "$0")/.."
src=${1:-training}
dst=results/stage2
mkdir -p "$dst"
cp "$src"/progress.txt "$dst"/ 2>/dev/null || true
cp "$src"/h2h-progress.txt "$dst"/ 2>/dev/null || true
for f in "$src"/train*.log "$src"/eval*.jsonl "$src"/h2h-*.jsonl; do
    [ -e "$f" ] && cp "$f" "$dst"/
done
for f in "$src"/gen*-games.jsonl; do
    [ -e "$f" ] || continue
    out="$dst/$(basename "$f").gz"
    # Only finished generations (the file is complete once its samples are).
    g=$(basename "$f" -games.jsonl)
    if [ -e "$src/$g.jsonl" ] || [ -e "$src/$g.jsonl.gz" ]; then
        [ -e "$out" ] || gzip -c "$f" > "$out"
    fi
done
ls "$dst" | wc -l
