# Bots

Bots that play 001 with automatic rules, and tools for using them to find
balance problems. The plan has two stages:

1. **Search with a hand-made (or learned) evaluation** (done). It adapts
   instantly to rule changes, because it searches the real rules, so it's the
   fast balance check.
2. **AlphaZero-style self-play with a neural network** (not started). This is
   the route to playing better than people; see [Stage 2](#stage-2-neural-network)
   for the design and costs.

## The bots

All of them are in `src/bot.rs` and implement `bot::Agent`.

- `random`: picks a legal action at random.
- `greedy`: picks the action whose result the evaluation likes best.
- `mcts:SIMSxSAMPLES`: Monte Carlo tree search over single actions. Every new
  position in the tree gets move priors by evaluating each legal action one
  step ahead, and leaves are scored by the evaluation (no random playouts). It
  never looks at hidden cards. Each decision searches `SAMPLES` copies of the
  game in which the opponent's unseen cards and the deck order are resampled
  (`Game::determinize`), and adds up the results.
- `greedyL`, `mctsL:SIMSxSAMPLES`: the same, with an evaluation learned from
  self-play (`--weights`).

Every bot stops after 16 actions in a turn. Without a cap, search can wander
into pointless loops (applying and discarding free powers forever).

**Evaluation** (`bot::features`, `bot::evaluate`). Everything is from one
player's point of view, using only what they can see:
- points still needed to win (counting the point the current player is on
  course to score this turn)
- lead at the flag
- units near the flag, weighted by distance
- units, distinct powers on units (cards worth more than free powers),
  units that can move, and cards in hand (capped)

The learned evaluation (`tools/fit_eval.py`) also adds each card's count on my
units minus theirs, and the cards in my hand.

## Running games

```bash
cargo build --release
# 200 games between two search bots, results in games.jsonl
./target/release/selfplay --games 200 --agents mcts:400x4,mcts:400x4 --out games.jsonl
# Summarise: first-player advantage, and per-card statistics
python3 tools/analyze.py games.jsonl
```

`selfplay --help` isn't implemented. The options are at the top of
`src/bin/selfplay.rs`. The ones for balance experiments are:
- `--second-player-points K` and `--target-score K` change the rules
- `--remove CARD,CARD` leaves cards out of the deck
- `--first-seed S` picks the deals. The same seeds give the same deals, so
  variants can be compared game by game.

To learn an evaluation:

```bash
./target/release/selfplay --games 1000 --agents mcts:400x4,mcts:400x4 --out games.jsonl --positions positions.jsonl
python3 tools/fit_eval.py positions.jsonl --out weights.json
./target/release/selfplay --games 100 --agents mctsL:400x4,mcts:400x4 --weights weights.json --out vs.jsonl
```

## Results

RESULTS

## Stage 2: neural network

DESIGN
