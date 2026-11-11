# Bots

Bots that play 001 with automatic rules, and tools for using them to find
balance problems. The plan has two stages:

1. **Search with a hand-made (or learned) evaluation** (done). It adapts
   instantly to rule changes, because it searches the real rules, so it's the
   fast balance check.
2. **AlphaZero-style self-play with a neural network** (built, and trained for
   a first night on a 4-core VM). This is the route to playing better than
   people; see [Stage 2](#stage-2-neural-network) for the design and costs.

## Key findings

- **Balance breaks at the flag.** Every card was measured by giving it to a
  player on the same deals. The cards that move the flag towards their owner
  (*Swap*, *Pull*, *Heavy*, *Lift*) or take the *Flag* power (*Trade*) are
  each worth +18 to +31 percentage points of win rate; so are the ones that
  change the count next to it (*Strong*, *Weak*). Five more are worth +10 to
  +15, and the other ~45 are within about ±8 of zero. With the stronger stage 2
  bot on both sides the effects shrink but the ranking holds (Trade +30, Heavy
  +19, Strong +19, Pull +14, Swap +12, Weak +10, Lift +9). With the flag fixed
  in place and the *Flag* power not tradeable, the four flag-movers we
  re-measured drop back in line. See [Cards](#cards) and RULES_DECISIONS.md.
- **Climb needs a rules decision, but it's less urgent**: it's worth +20
  against the stage 1 bot because we read "buried units don't count at the
  flag", which the rules text doesn't settle (if they do count, it drops to +9).
  Against the stronger bot it's only +3: good players avoid being buried. See
  RULES_DECISIONS.md, "Not decided yet".
- **If you fix the flag, give the second player 2 points.** Without the
  flag-moving cards the first player's edge grows; with the stronger bot, 2
  starting points bring it back to 50%. See [Cards](#cards).
- **The first player's advantage is small.** With the stronger bot, the current
  rule (second player starts with 1 point) gives 50% (44–57%); with no
  compensation it's 55%. See [First player advantage](#first-player-advantage).
- **Win-rate correlations mislead about cards.** Measuring a card by giving it
  to a player on the same deals found Forbid and Bully strong and Trail and
  Explosive no help, the opposite of what the correlations said. Use
  `tools/card_value.sh`.
- **The trained bot beats the hand-made search** 98–2 (net10 against
  `mcts:400x4`) and is about 570 Elo above its first generation. You can play
  it in the web app (**Add a bot**), or with no server at all in the
  single-player page ([001 Solo](https://claude.ai/artifact/G66juB744M28dXyHugJiRU),
  built by `tools/build_web.sh`). Progress levelled off around generation
  10 (of 1,000 games each), even with twice the search; more games per
  generation, on more cores, is the next step
  (about $15–40 of rented CPU; a GPU isn't needed yet). See
  [Where to run it](#where-to-run-it-and-what-it-costs).
- Self-play found an engine bug (offering a move that couldn't be finished)
  and a rules loophole (unlimited *Split* actions), both described below.

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
percentage points to these bots, and with the current 1 point the first player
still has a small edge.

**With the stronger stage 2 bot** (net2, `mctsN:200x4`, 200 games each on the
first 200 of the same deals, `results/stage2-sweep/`):

| second player starts with | first player wins (95% interval) | turns |
|---|---|---|
| 0 points | 55% (48–62%) | 11.8 |
| 1 point (the current rules) | 50% (44–57%) | 10.6 |
| 2 points | 52% (46–59%) | 9.4 |

Better play shrinks the first player's advantage: even with no compensation
it's about 55%, and **the current 1 point looks fair**. The stage 1 bots
overstated the advantage. (200 games can't separate 50% from 52%; the
intervals are about ±7 points.)

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
difference in the second player's win rate is what having the card in hand
is worth. It costs 400 games per card (about 30 minutes on one core).

**Every card has been measured** (all 62 except the *Flag* and *Walk*;
`cards/`, 200 paired games each with `mcts:400x4`, ± is a 95% interval, bold
where it excludes 0; rebuilt by `tools/card_table.py`). They're being
measured again with the best trained bot (net10, `cards-net10/`, in progress,
strongest cards first); some were measured with an earlier one (net5,
`cards-net/`) and under the fixed-flag variant (`cards-fixed-flag/`). In
summary:

- **Dominant (+18 to +31 points):** the cards that move the flag (Swap, Pull,
  Heavy, Lift, and Trade in a strong player's hands) and the ones that change
  the count at the flag (Strong, Weak, and Climb by burying enemies).
- **Strong (+10 to +15):** Push, Bridge, Spotlight, Remote, Chain.
- **Everything else** (about 45 cards) is within about ±8 points of zero:
  200 games can't tell them apart, and none is measurably bad.
- **With the stage 2 bot** (top cards only) the effects shrink but stay
  significant: Trade +29.5, Heavy +19.0, Strong +18.5, Pull +14.0, Swap
  +11.5, Weak +10.0, Lift +9.0; Climb drops to +3.0.

So the game's balance problems are concentrated in one place, the flag: what
moves it, and what changes who counts next to it. Fix the flag first, then
re-run the survey (ideally with the stage 2 bot and 800 games per card, to
rank the middle).

**Side effects of fixing the flag** (`--fixed-flag`, the same 600 deals,
`results/fixed-flag/`): the flag-moving cards were partly a comeback
mechanism, so without them the first player's edge grows, and the starting
points matter more.

| rules (stage 1 bot) | first player wins | turns | unfinished |
|---|---|---|---|
| current | 54% (50–58%) | 12.6 | 4 |
| flag fixed | 60% (56–64%) | 13.5 | 12 |
| flag fixed, second player starts with 2 | 45% (41–49%) | 12.6 | 14 |
| flag fixed, 2 points, target 6 | 44% (40–48%) | 15.2 | 20 |

So with the flag fixed, the fair compensation is somewhere between 1 and 2
points for these bots (a longer race doesn't dilute it). **With the stronger
stage 2 bot** (net10, 200 games each), fixing the flag moves the first player
from 50% to 55% (48–61%) at 1 point, and 2 points brings it back to **50%
(43–57%)**, with no stalemates and games of normal length. So for strong
play, "fix the flag and give the second player 2 points" looks fair; weaker
players may find it slightly favours the second player.

<!-- cards -->
| card | worth in hand (points) | with the trained bot (net10) | with net5 | with the flag fixed | relative lift (points) | learned value (log-odds) |
|---|---|---|---|---|---|---|
| Heavy | **+29.6 ± 8.7** | **+31.5 ± 8.5** | **+19.0 ± 9.4** | +0.0 ± 7.2 | +15 | +0.51 |
| Swap | **+31.3 ± 8.1** | **+9.5 ± 8.9** | **+11.5 ± 8.8** | +6.8 ± 9.1 | +3 | +0.49 |
| Pull | **+30.2 ± 8.5** | **+12.0 ± 8.4** | **+14.0 ± 8.5** | -3.1 ± 8.0 | +0 | +0.42 |
| Trade | **+10.0 ± 8.3** | **+26.6 ± 8.8** | **+29.5 ± 8.8** | +1.5 ± 8.0 (stage 2 bot) | -1 | +0.02 |
| Strong | **+22.1 ± 8.5** | **+11.0 ± 8.1** | **+18.5 ± 8.6** |  | +24 | +0.20 |
| Climb | **+20.3 ± 8.0** | +6.5 ± 8.3 | +3.0 ± 8.8 |  | -2 | +0.24 |
| Weak | **+19.7 ± 8.0** | +5.0 ± 7.8 | **+10.0 ± 8.6** |  | +12 | +0.23 |
| Lift | **+17.6 ± 8.0** | **+10.0 ± 7.7** | **+9.0 ± 8.0** |  | +0 | -0.06 |
| Push | **+14.6 ± 7.9** | +0.0 ± 8.2 |  |  | -1 | +0.14 |
| Split | -2.5 ± 8.2 | **+14.0 ± 8.5** |  |  | -8 | +0.25 |
| Colorblind | **+7.6 ± 7.4** | **+13.6 ± 8.2** |  |  | -0 | -0.14 |
| Bridge | **+12.6 ± 6.8** | -5.0 ± 8.6 |  |  | +18 | +0.57 |
| Spotlight | **+12.6 ± 7.5** | +4.5 ± 8.4 |  |  | +5 | +0.35 |
| Remote | **+11.7 ± 6.8** | **+10.0 ± 8.2** |  |  | +19 | +0.65 |
| Chain | **+10.1 ± 7.4** | +6.5 ± 8.5 |  |  | -0 | -0.44 |
| Bully | **+9.1 ± 7.7** | -2.5 ± 7.8 |  |  | -1 | -0.64 |
| Jump | **+9.1 ± 8.2** | +0.0 ± 8.2 |  |  | -7 | +0.15 |
| Run | **+9.1 ± 8.8** | -5.0 ± 8.3 |  |  | -8 | +0.31 |
| Spin | +4.5 ± 6.7 | **+9.0 ± 7.5** |  |  | -5 | -0.45 |
| Beam | **+8.5 ± 7.1** | +2.5 ± 8.0 |  |  | -3 | +0.14 |
| Fast | +3.0 ± 6.7 | +8.5 ± 8.7 |  |  | +1 | +0.10 |
| Telepath | **+8.1 ± 7.3** | +1.5 ± 8.0 |  |  | +10 | -0.13 |
| Forbid | **+8.0 ± 7.4** | +3.0 ± 7.8 |  |  | -8 | -0.23 |
| Halo | +7.5 ± 7.6 | +4.0 ± 8.0 |  |  | +0 | -0.44 |
| Six-shooter | **+7.5 ± 6.7** | +2.5 ± 8.4 |  |  | +9 | +0.11 |
| Reach | +7.0 ± 8.0 | -0.5 ± 7.8 |  |  | +1 | +0.41 |
| Vengeance | **+7.0 ± 6.3** | -3.0 ± 8.3 |  |  | +3 | -0.35 |
| Phoenix | +6.1 ± 6.2 | +6.0 ± 8.4 |  |  | +15 | -0.60 |
| Click Click | -1.5 ± 7.6 | +5.5 ± 7.4 |  |  | -5 | -0.20 |
| Trigger | +5.5 ± 7.8 | +3.5 ± 8.1 |  |  | -3 | +0.09 |
| Dogpile | +1.0 ± 7.8 | +5.5 ± 9.0 |  |  | -7 | -0.28 |
| Leader | -1.0 ± 7.0 | +5.5 ± 8.6 |  |  | -2 | +0.21 |
| Grip | +5.1 ± 5.9 | -6.5 ± 8.7 |  |  | +12 | -0.66 |
| Stubborn | -1.5 ± 6.4 | +5.0 ± 8.2 |  |  | +16 | -0.38 |
| Jammer | +4.6 ± 5.9 | -4.5 ± 7.6 |  |  | +17 | -0.03 |
| Catapult | +4.5 ± 7.4 | +1.5 ± 8.3 |  |  | -5 | +0.42 |
| Twitchy | +3.0 ± 6.1 | +4.5 ± 7.5 |  |  | +15 | -0.06 |
| Stalker | +4.0 ± 7.0 | +1.5 ± 8.5 |  |  | +4 | -0.28 |
| Banish | +2.5 ± 7.8 | +4.0 ± 8.1 |  |  | -6 | +0.04 |
| Friendly | +3.6 ± 6.2 | +1.5 ± 8.6 |  |  | +12 | +0.04 |
| Lifeguard | +3.6 ± 6.0 | -3.0 ± 8.1 |  |  | +12 | +0.11 |
| Circle | +3.5 ± 7.9 | -4.0 ± 8.3 |  |  | -7 | +0.06 |
| Rush | +3.5 ± 7.3 | -7.5 ± 7.7 |  |  | -12 | -0.13 |
| Liability | +3.0 ± 7.2 | +3.5 ± 8.1 |  |  | +1 | -0.60 |
| Trail | -0.5 ± 7.2 | +3.5 ± 8.1 |  |  | +17 | +0.65 |
| Possess | +3.1 ± 7.6 | +0.5 ± 7.5 |  |  | -8 | -0.20 |
| Mine | +3.0 ± 7.7 | +1.0 ± 8.5 |  |  | -9 | -0.25 |
| Spiky | -2.5 ± 6.9 | +2.5 ± 8.2 |  |  | +11 | +0.46 |
| Neutralize | -2.5 ± 7.8 | +2.0 ± 8.2 |  |  | -9 | -0.24 |
| Convert | +0.5 ± 7.2 | +1.5 ± 8.4 |  |  | -5 | +0.13 |
| Recall | +1.0 ± 7.4 | -3.5 ± 8.2 |  |  | -8 | +0.06 |
| Pacifist | -2.0 ± 7.0 | +1.0 ± 7.9 |  |  | +15 | +0.38 |
| Stinky | -1.0 ± 7.6 | +1.0 ± 7.9 |  |  | -4 | -0.22 |
| Vacuum | +0.5 ± 7.1 | -2.0 ± 8.1 |  |  | +12 | -0.55 |
| Backstab | +0.0 ± 7.8 | -3.0 ± 7.8 |  |  | -5 | -0.52 |
| Nuke | -1.0 ± 7.5 | -6.5 ± 8.0 |  |  | -8 | -0.09 |
| Recycle | -1.5 ± 6.5 | **-9.5 ± 7.9** |  |  | +13 | -1.00 |
| Burrow | -2.5 ± 8.0 | -5.0 ± 8.3 | -3.0 ± 8.6 |  | -11 | -0.04 |
| Bungee | -3.0 ± 7.6 |  |  |  | -13 | -0.21 |
| Explosive | -4.0 ± 6.7 |  |  |  | +11 | +0.72 |
| Mystery | -4.0 ± 7.4 |  |  |  | -5 | +0.34 |
| Stun | -6.5 ± 8.0 |  |  |  | -7 | -0.15 |
<!-- /cards -->

**Swap and Heavy are balance problems, and the cause is moving the flag.**
In hand each is worth about +30 points against the stage 1 bot (the second
player's win rate goes from about 40% to 72%), three times any other card
measured. Stronger play blunts them but doesn't fix them: with the stage 2 bot
on both sides, Heavy is still worth +19 and Swap +12, and they're its top two
draft picks (taken about 80% of the times they're offered). `examples/flag_moves.rs` shows why: the *Flag* card only limits the
flag to one space per turn, so both are ways to drag the flag towards your
own units. 68% of the bots' swaps are with the flag (gaining 1.4 in flag lead
each), and Heavy pulls the flag at the end of the turn, gaining 2.2 in flag
lead each time, right before scoring. Pull does the same, less often.
Possible fixes: the flag can't be moved by powers at all, or only by its own
holder. **Testing the first** (`selfplay --fixed-flag`, `cards-fixed-flag/`):
with the flag fixed in place, Swap in hand is worth +6.8 ± 9.1 points instead
of +31.3, and Heavy +0.0 ± 7.2 instead of +29.6: both back in line with the
other cards. **Making the flag immovable fixes both.**

**Trade is the third way to take the flag.** With the stage 2 bot it's worth
+29.5 ± 8.8 in hand (but only +10.0 ± 8.3 with the stage 1 bot, whose
hand-made evaluation doesn't see the trick: a reminder that the stage 1
numbers are lower bounds for cards that need a clever use), and in 20 games the bots used it 22 times to trade for
the *Flag* power itself: their unit becomes the flag, which they can then
walk towards their own units. So `--fixed-flag` also stops Trade taking the
*Flag* power, and with that, Trade drops to +1.5 ± 8.0 with the same bot.
**Pull is the fourth**: worth +30.2 ± 8.5 in hand, by pulling the flag into
your unit's space, and −3.1 ± 8.0 with the flag fixed. **Lift is the fifth**
(+17.6 ± 8.0): 64% of its uses moved the flag, gaining 2.1 in flag lead each.

**Climb (+20.3 ± 8.0) is strong for a different reason, and it depends on a
rules decision of ours.** Every Climb in 20 games buried an enemy unit
(gaining 1.2 in flag lead each): a buried unit doesn't count at the flag, so
climbing onto an enemy next to the flag both removes it and puts your unit
there. The rules text only says buried units can't be targeted or activated;
"buried units don't score" is our reading (RULES_DECISIONS.md, "Units, stacks
and home spaces"). Measured with buried units still counting at the flag
(`selfplay --buried-count`, `cards-buried-count/`), Climb is worth +8.6 ± 8.7:
in line with the other good cards. So that reading fixes it. And with the
stage 2 bot on both sides it's only +3.0 ± 8.8 even under our reading: a
strong opponent avoids getting buried next to the flag.

Strong (+22.1 ± 8.5) and Weak (+19.7 ± 8.0) are direct changes to the count
at the flag (a unit with Strong counts twice; Weak's target counts one less),
which is often the difference in a close count. Strong is the stage 2 bot's
fourth favourite draft pick. If they're meant to be ordinary cards, a cap
(eg not at the flag, or only when outnumbering) would tone them down.

Push (+14.6 ± 7.9) is strong in the ordinary way: only 8% of its uses moved
the flag; the rest pushed enemy or other units for about +0.9 in flag lead
each. Worth watching, not obviously broken. **The fix for all of them: the flag can't be moved by
powers, and the *Flag* power can't be traded.**

(The measure includes the value of simply having one more card; since many
cards come out near 0, an extra card is worth little by itself, and the large
values come from what those cards do.)

**The correlational measures are unreliable**: they rated Forbid and Bully
weak (both are among the strongest cards in hand), and Trail and Explosive
strong (neither measurably helps). Treat them as pointers at most and use the
causal test for decisions.

**The trained bot's draft choices are a better cheap signal.**
`tools/draft_values.py` fits how strongly the bot prefers each card when it
drafts (`results/stage2/draft-gen*.txt`); it comes free with self-play. By
generation 5 its top three picks were Heavy, Swap and Trade, the three
flag-takers, and its last was Burrow; measured causally with the same bot,
those are worth +19, +12, +30 and −3. By generation 11 it takes each of the
three 85–90% of the times they're offered, with Strong and Pull next. Use it
to choose which cards to test. Running it for all
64 cards is about 25,000 games: a couple of hours on a rented 32-thread
machine, a few dollars. Doing it with the stage 2 bot
(`tools/card_value.sh CARD 200 mctsN:200x4@bots/net.json`) would measure the
cards as a stronger player uses them.

### Rule problems found by self-play

- **Unlimited actions with *Split*** (in RULES_DECISIONS.md under "Not decided
  yet"): one game had over 500 splits in a single turn. Bots cap themselves at
  16 actions per turn, which hides it in experiments, but *Split* is still the
  most used active power (1.8 times per game, after *Walk*).
- Self-play also found engine bugs that the scenario tests missed (a crash when
  an end-of-turn effect referred to a dead unit, actions with prompts not
  counting towards the per-turn cap, and, during stage 2 training, *Lift* with
  *Chain* offering a move that couldn't be finished, because the search for a
  way to finish gave up optimistically), now fixed and covered by tests. To
  debug one, rerun the game with `GAME_DEBUG=1` (selfplay saves its actions)
  and `cargo run --release --example replay -- bug-seed-N.json`.



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

<!-- generations -->
| network | against `mcts:400x4` | against the previous network | Elo vs net0 (chained) |
|---|---|---|---|
| net0 | 74–26 |  | 0 |
| net1 | 75–25 | 61–39 | +78 |
| net2 | 81–19 | 59–41 | +141 |
| net3 | 83–17 | 63–37 | +233 |
| net4 | 93–7 | 63–37 | +326 |
| net5 | 96–4 | 56–44 | +368 |
| net6 | 92–8 | 58–42 | +424 |
| net7 | 91–9 | 55–45 | +459 |
| net8 | 92–8 | 50–50 | +459 |
| net9 | 96–4 | 62–38 | +544 |
| net10 | 98–2 | 54–46 | +572 |
| net11 | 90–10 | 44–56 | +530 |
| net12 | 98–2 | 51–49 | +537 |
| net13 | 94–6 | 51–49 | +544 |
| net14 | 97–3 | 53–46 | +568 |
| net15 | 95–5 | 46–54 | +540 |
<!-- /generations -->

The yardstick saturates (the game has enough luck that even a much stronger
bot loses some games), so the head-to-head column is the better measure of
progress. (Each match is 100 games, so each Elo step is uncertain by about
±70.) As a direct check, **net6 beat net0 88–12** (about +350 Elo), so the
chain isn't just accumulated noise.

Progress slowed around net7–net8 (net8 drew net7, though net9 then beat net8
62–38). To see whether the network was too small, a network 4 times the size
(width 128, 3 layers, 640k parameters) was trained on the same recent
samples: it lost to net8 41–59, and its value head overfitted more. So the
limit here isn't network size but the number of independent games: a
training window held about 4,000 game results, and both networks learn to
recognise games rather than judge positions. From generation 10 the window
is 8 generations (capped at the newest 900,000 samples). That didn't move
the plateau: net12 and net10 drew 52–48 head to head. From generation 14,
self-play searches twice as much (`400x4`) to see if better search targets
restart progress (the head-to-head matches stay at `200x4`). They didn't:
net14 beat net13 53–46, then net15 lost to net14 46–54. At this scale (one
4-core machine, 1,000 games a generation) the bot has levelled off at about
550 Elo above net0, so the run stopped after generation 15 and the machine
went to measuring every card instead. The next levers are
more games per generation (4,000+) and more search per move, which is what a
many-core rented machine buys.

The web app's bot (**Add a bot**) and the single-player page use
`bots/net.json`, currently net10. In the browser the bot takes about 150 ms a
move (400x4 search), a few times slower than natively.

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
# Continue from the committed network (the samples aren't in the repository):
INIT_NET=bots/net.json PYTHON=~/venv/bin/python tools/alphazero.sh training 50 4000 200x4
```

Watch `training/progress.txt` (against `mcts:400x4`) and play the newest
network against the previous one (`selfplay --agents
mctsN:200x4@training/netN.json,mctsN:200x4@training/netM.json`). Keep
`THREADS` and the training threads (`TRAIN_ARGS="--threads N"`) within the
cores you have: oversubscribing slows training twentyfold.

## Next steps

1. **Keep training** until the head-to-head gains stop, on a rented many-core
   machine (above). Then grow the network (`TRAIN_ARGS="--d 128 --layers 4"`)
   or the search (`400x4`) and see if gains resume.
2. **Measure every card causally with the strongest bot**:
   `tools/card_value.sh CARD 200 mctsN:200x4@bots/net.json` for each of the
   64 cards (about 25,000 games). Cards worth more than about ±8 points in hand
   are the balance problems; the stage 1 measures aren't reliable enough.
3. **Settle the *Split* rule** (RULES_DECISIONS.md); the bots work around it
   with a 16-action cap, but people won't.
4. **Play the bot** (Add a bot in the web app) to see whether it plays like a
   strong human yet, and where it doesn't. Losses to people are the best guide
   to what the training is missing.
5. Re-run the first-player sweep with more games once the bot is stronger: 200
   games per variant can't separate 50% from 52%.

