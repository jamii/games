# Bots

Bots that play 001 with automatic rules, and tools for using them to find
balance problems. The plan has two stages:

1. **Search with a hand-made (or learned) evaluation** (done). It adapts
   instantly to rule changes, because it searches the real rules, so it's the
   fast balance check.
2. **AlphaZero-style self-play with a neural network** (the pieces are built
   and tested; first training runs below). This is the route to playing better
   than people; see [Stage 2](#stage-2-neural-network) for the design and costs.

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
- `policy`, `mctsN:SIMSxSAMPLES`: the stage 2 network (`--net`), playing its
  favourite move without search, or guiding the search: one network call per
  new position gives both the move priors and the leaf value.

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

### What's built

- `src/encode.rs` turns a position into tokens from one player's point of view
  (only what they can see): one token for the game as a whole (scores, phase,
  hands as card counts, the faceup row, discards, what's forbidden, the prompt
  being answered) and one per unit (owner, space, facing, what's in front, its
  powers, tokens, activations, neighbours). The board is turned around for the
  second player, so both see their home at the top. Each legal action becomes a
  token too (kind, card or power, space, facing, prompt option), with pointers
  to the units it involves. Tokens are sparse lists of feature ids, so adding
  a card adds features rather than changing the shape of anything.
- `src/net.rs` runs the network on the CPU, in Rust: an embedding bag per token,
  a few transformer layers so units can attend to each other, a value head on
  the game token, and a policy head that scores each legal action from its own
  token, the units it points to, and the game token. Default size: width 64, 2
  layers, 4 heads, about 170k parameters; one call takes about 0.1 ms.
- `tools/train.py` trains it with PyTorch (CPU or GPU) from `selfplay --samples`
  files: every searched decision with the search's visit counts (the policy
  target) and the eventual winner (the value target). The weights file holds a
  few positions with PyTorch's outputs, and `net.rs` refuses to load the file
  if it doesn't compute the same numbers, so the two implementations can't
  drift apart silently.
- `tools/alphazero.sh` runs expert iteration: generation 0 imitates the
  hand-made search; each later generation plays with the previous network
  (with exploration noise at the root and random-ish choices in the first two
  turns), trains the next network on the newest samples, and plays it against
  `mcts:400x4` as a fixed yardstick.

```bash
# Everything in training/, 10 generations of 1000 games, 200x4 search:
PYTHON=~/venv/bin/python tools/alphazero.sh training 10 1000 200x4
# On a GPU machine, with a bigger network:
TRAIN_ARGS="--device cuda --d 128 --layers 4" tools/alphazero.sh training 20 4000 400x4
```

### Choices and their reasons

- **Hidden information.** The search samples the hidden cards (determinization)
  and the network sees only the deciding player's view. This is the simplest
  thing that works; its known weakness is that it can't plan to hide or
  reveal information (it assumes it'll know everything later). For a game where
  hidden hands matter less than the board, that should cost little. The
  heavier alternatives (ReBeL, Student of Games) are much more work.
- **Actions are scored one by one** rather than as a fixed-size output, because
  the action space depends on the units, cards and prompts present. Prompts
  (choosing a target, a card, a space) are just more actions.
- **Small network, CPU inference.** Self-play is dominated by the rules engine
  and search, not the network, so the network stays small enough to run inside
  the search on CPU threads. This also means **training doesn't need a big
  GPU**: the bottleneck is CPU cores for self-play.

STAGE2_RESULTS

### Where to run it, and what it costs

COSTS
