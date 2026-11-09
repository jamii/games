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

All with `mcts:400x4` against itself, 60-turn limit, unless noted. Raw games
are in `results/` (`results/summary.txt` has the full card table).

### Bot strength and speed

- Strength rises with search: `greedy` beat `random` 20–0, `mcts:800x4` beat
  `greedy` 14–2 and beat `mcts:200x4` 23–12. (Small samples; the point is that
  more search still helps, so the evaluation isn't the only limit.)
- The rules engine plays about 16,000 random decisions a second on one core
  (listing legal actions takes about 60 µs, applying one about 3 µs).
  `mcts:400x4` takes about 10 ms per decision, and a game 5–10 s on one core,
  so 600 games take about 15 minutes on 4 cores.

### First player advantage

| second player starts with | first player wins (95% interval) | turns |
|---|---|---|
| 0 points | 59% (55–63%) | 12.6 |
| 1 point (the current rules) | 54% (50–58%) | 12.6 |
| 2 points | 49% (45–53%) | 10.8 |

600 games each, on the same 600 deals. Each starting point is worth about 5
percentage points. With the current 1 point the first player still has a
small edge (the interval only just reaches 50%); 2 points looks even for bots
of this strength, and shortens games. Stronger players may value tempo
differently, so this is worth re-running with the stage 2 bot.

### Cards

Two correlational measures, both noisy at this sample size:
- **Lift**: how much more often players who applied the card won than seat
  order predicts. Highest: Phoenix, Strong, Remote, Lifeguard, Trail, Bridge
  (+18 to +22 points relative to the average card). Lowest: Bungee, Rush,
  Neutralize, Forbid, Burrow, Possess (−9 to −14).
- **Learned value** (`tools/fit_eval.py`, logistic regression on 13,800
  positions, controlling for score and board): highest Explosive, Remote,
  Trail, Bridge, Heavy, Swap; lowest Bully, Liability, Phoenix, Vacuum,
  Backstab, Spin.

They agree on Remote, Trail and Bridge looking strong, and on Bungee, Rush,
Forbid, Possess and Neutralize looking weak, or weak in these bots' hands.
They disagree on Phoenix and Stubborn, which are rare enough (about 200
positions) that neither measure is reliable. A striking pattern in lift is
that passive cards do better than active ones; part of that is likely the
bots: the hand-made evaluation rewards having powers on units, and the bots
use some active cards badly or never (*Recall* was applied in 280 games and
activated in none, because the evaluation always prefers a unit on the board
to cards in hand).

The causal test is `tools/card_value.sh CARD`: the same deals with the card
removed, and with it given to the second player after the draft. The
difference in win rate is what the card is worth. It costs about 400 games
per card.

### Rule problems found by self-play

- **Unlimited actions with *Split*** (in RULES_DECISIONS.md under "Not decided
  yet"): one game had over 500 splits in a single turn. Bots cap themselves at
  16 actions per turn, which hides it in experiments, but *Split* is still the
  most used active power (1.8 times per game, after *Walk*).
- Self-play also found engine bugs that the scenario tests missed (a crash when
  an end-of-turn effect referred to a dead unit, and actions with prompts not
  counting towards the per-turn cap), now fixed and covered by tests.



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

### First results

Generation 0 imitated `mcts:400x4` from 1,000 of its self-play games (127,000
decisions), trained for 10 epochs on 4 CPU threads (about 25 s an epoch):

- The network alone (`policy`, no search) picks the search's favourite action
  56% of the time on held-out games. It beats `greedy` 65–35 but loses to
  `mcts:400x4` 14–85: it's a good intuition, not a player.
- **With search (`mctsN:200x4`), it beat `mcts:400x4` 74–26** over 100 games
  (seats alternating), with half the simulations and similar time per
  decision (40 ms against 30 ms). One round of imitation already gives a
  stronger bot than stage 1, because the learned priors focus the search
  far better than one-step lookahead does.
- The value head overfits easily: every position in a game shares one result,
  so with 1,000 games it learned which deals were won (held-out error worse
  than guessing 0). Mixing the search's own value into the target
  (`--q-mix 0.5`) and keeping the best held-out epoch fixed it. A target of
  only the search's value did worse (63–37 against the same yardstick).

Expert iteration from there (`tools/alphazero.sh`, 1,000 games per
generation, `mctsN:200x4`, about 45 minutes a generation on this VM). Each
network plays 100 games against the fixed yardstick `mcts:400x4`, and 100
against the previous network (`results/stage2/`):

| network | against `mcts:400x4` | against the previous network |
|---|---|---|
| net0 | 74–26 | |
| net1 | 75–25 | 61–39 |
| net2 | 81–19 | 59–41 |

The yardstick saturates (the game has enough luck that even a much stronger
bot loses some games), so the head-to-head column is the better measure of
progress: each generation so far is about 70 Elo stronger than the last.

The web app's bot (**Add a bot**) uses `bots/net.json`, currently net2.

### Where to run it, and what it costs

Everything so far ran on this 4-core cloud VM, without a GPU. Measured:

| step | cost |
|---|---|
| self-play, `mctsN:200x4` | about 7 s per game per core |
| training, default network | about 25 s per epoch per 115,000 samples on 4 cores |
| one generation (1,000 games, 4 epochs on 4 generations of samples, 100-game yardstick) | about 40 min on 4 cores |

So with this network size **the bottleneck is CPU cores for self-play, not a
GPU**. Rent by core count:

- **Many-core CPU machines** are the best value: a dedicated server (eg Hetzner
  AX102, 16 cores/32 threads, roughly €260/month) or hourly cloud machines with
  32–64 vCPUs (Hetzner, or the CPU instances on Vast.ai/RunPod). Hetzner
  raised cloud prices sharply in 2026, so compare current prices; around
  $0.01–0.03 per vCPU-hour is typical.
- **A GPU only matters once the network grows** (width 128+, 4+ layers), for
  training and then for batched inference during self-play (which would need
  a batching inference server; the search currently calls the network one
  position at a time). A single RTX 4090 or L4 on Vast.ai or RunPod (about
  $0.35–0.90/hour) is plenty; A100/H100s and the big clouds' GPU prices aren't
  worth it at this scale.

Rough budget: AlphaZero-style training on games of this size typically needs
a few hundred thousand self-play games. 300,000 games at 7 core-seconds is
about 600 core-hours: a day on a 32-thread machine, about **$15–40** of rented
CPU. If a bigger network turns out to be needed, add GPU time and the
engineering for batched inference: perhaps **$100–300** for the project
including experiments. Start with the small network on CPUs and grow it only
if the yardstick results stop improving.

To run it on a rented Linux machine:

```bash
git clone <this repo> && cd <repo>/001
curl https://sh.rustup.rs -sSf | sh -s -- -y && . ~/.cargo/env
python3 -m venv ~/venv && ~/venv/bin/pip install torch numpy
PYTHON=~/venv/bin/python tools/alphazero.sh training 50 4000 200x4
```
