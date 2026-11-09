#!/usr/bin/env bash
# Train the stage 2 network by expert iteration (AlphaZero-style). See BOT.md.
#
#   tools/alphazero.sh DIR [GENERATIONS] [GAMES] [SEARCH]
#
# Generation 0 imitates the hand-made search (mcts:400x4). Each later
# generation plays GAMES games of network-guided search (mctsN:SEARCH, with
# exploration noise) using the previous network, then trains the next network
# on the newest samples, starting from the previous weights. After each
# generation, the new network plays 100 games against mcts:400x4 as a fixed
# yardstick. Everything goes in DIR; rerunning continues where it stopped.
#
# Defaults: 10 generations, 1000 games each, 200x4 search. Settings via the
# environment: WINDOW (generations of samples to train on, default 4), EPOCHS
# (default 4; 12 for generation 0), EVAL_GAMES (default 100), TRAIN_ARGS
# (extra arguments for tools/train.py, eg "--device cuda --d 128 --layers 4"),
# THREADS (self-play threads, default all cores), PYTHON (with PyTorch),
# INIT_NET (start from this network instead of imitating mcts:400x4, eg
# bots/net.json to continue a run on another machine; generation 0 then plays
# with it).
set -euo pipefail
cd "$(dirname "$0")/.."
dir=${1:?usage: tools/alphazero.sh DIR [GENERATIONS] [GAMES] [SEARCH]}
generations=${2:-10}
games=${3:-1000}
search=${4:-200x4}
window=${WINDOW:-4}
epochs=${EPOCHS:-4}
eval_games=${EVAL_GAMES:-100}
python=${PYTHON:-python3}
mkdir -p "$dir"
cargo build --release --bin selfplay
selfplay="./target/release/selfplay ${THREADS:+--threads $THREADS}"
# A generation's samples are done once written (they may since have been
# compressed to save space).
have() { [ -s "$1" ] || [ -s "$1.gz" ]; }

# Seeds: generation g uses g*100000 onwards, the yardstick 900000+g*1000.
if [ -n "${INIT_NET:-}" ] && [ ! -s "$dir/net0.json" ]; then
    cp "$INIT_NET" "$dir/net0.json"
fi
if ! have "$dir/gen0.jsonl" && [ -s "$dir/net0.json" ]; then
    echo "== generation 0: $games games of mctsN:$search with net0"
    $selfplay --games "$games" --first-seed 0 --net "$dir/net0.json" --agents "mctsN:$search,mctsN:$search" \
        --noise 0.25 --explore-turns 2 --max-turns 60 --variant gen0 --out "$dir/gen0-games.jsonl" \
        --samples "$dir/gen0.jsonl.part" 2> "$dir/gen0.log"
    mv "$dir/gen0.jsonl.part" "$dir/gen0.jsonl"
fi
if ! have "$dir/gen0.jsonl"; then
    echo "== generation 0: $games games of mcts:400x4"
    $selfplay --games "$games" --first-seed 0 --agents mcts:400x4,mcts:400x4 --max-turns 60 \
        --variant gen0 --out "$dir/gen0-games.jsonl" --samples "$dir/gen0.jsonl.part" 2> "$dir/gen0.log"
    mv "$dir/gen0.jsonl.part" "$dir/gen0.jsonl"
fi
if [ ! -s "$dir/net0.json" ]; then
    echo "== training net0"
    $python tools/train.py "$dir/gen0.jsonl" --out "$dir/net0.json" --epochs $((epochs * 3)) ${TRAIN_ARGS:-} \
        | tee "$dir/train0.log"
fi

yardstick() {
    local g=$1
    if [ ! -s "$dir/eval$g.jsonl" ]; then
        echo "== net$g vs mcts:400x4 ($eval_games games)"
        $selfplay --games "$eval_games" --first-seed $((900000 + g * 1000)) --net "$dir/net$g.json" \
            --agents "mctsN:$search,mcts:400x4" --max-turns 60 --variant "eval$g" \
            --out "$dir/eval$g.jsonl.part" 2> "$dir/eval$g.log"
        mv "$dir/eval$g.jsonl.part" "$dir/eval$g.jsonl"
    fi
    python3 -c "
import json, sys
games = [json.loads(l) for l in open('$dir/eval$g.jsonl')]
wins = sum(1 for x in games if x['winner'] is not None and x['agents'][x['winner']].startswith('mctsN'))
done = sum(1 for x in games if x['winner'] is not None)
print(f'net$g: won {wins} of {done} finished games against mcts:400x4 ({len(games) - done} unfinished)')
" | tee -a "$dir/progress.txt"
}

yardstick 0
for g in $(seq 1 "$generations"); do
    prev=$((g - 1))
    if ! have "$dir/gen$g.jsonl"; then
        echo "== generation $g: $games games of mctsN:$search with net$prev"
        $selfplay --games "$games" --first-seed $((g * 100000)) --net "$dir/net$prev.json" \
            --agents "mctsN:$search,mctsN:$search" --noise 0.25 --explore-turns 2 --max-turns 60 \
            --variant "gen$g" --out "$dir/gen$g-games.jsonl" --samples "$dir/gen$g.jsonl.part" 2> "$dir/gen$g.log"
        mv "$dir/gen$g.jsonl.part" "$dir/gen$g.jsonl"
    fi
    if [ ! -s "$dir/net$g.json" ]; then
        files=()
        for k in $(seq $((g - window + 1)) "$g"); do
            if [ "$k" -lt 0 ]; then continue; fi
            if [ -s "$dir/gen$k.jsonl" ]; then files+=("$dir/gen$k.jsonl");
            elif [ -s "$dir/gen$k.jsonl.gz" ]; then files+=("$dir/gen$k.jsonl.gz"); fi
        done
        echo "== training net$g on ${files[*]}"
        $python tools/train.py "${files[@]}" --init "$dir/net$prev.json" --out "$dir/net$g.json" \
            --epochs "$epochs" --lr 5e-4 ${TRAIN_ARGS:-} | tee "$dir/train$g.log"
    fi
    yardstick "$g"
done
