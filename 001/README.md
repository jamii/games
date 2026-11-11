# 001 web app

A small Rust web server for playing one game of 001 between two browsers, plus a rules
engine that bots can use directly.

```bash
cd 001
cargo run --release
```

Open `http://<host>:3000` in two browsers (set `PORT` to change the port) and click
**Play as Letters** in one and **Play as Numbers** in the other. Anyone else who opens
the page can watch. Seats are remembered per browser tab, so reloading keeps your seat.
Two tabs in the same browser can also be the two players. If a player's tab is closed,
whoever opens the page next can take over their seat. It works on phones too (tap and
drag with a finger).

To play against the computer, take a seat and click **Add a bot** on the other one. The
bot uses the trained network in `bots/net.json` if there is one (see [BOT.md](BOT.md)),
and otherwise the hand-made search. Environment variables: `BOT_NET` (another network),
`BOT_SEARCH` (search size, default `400x4`; bigger is stronger and slower) and
`BOT_DELAY_MS` (pause before each of its actions, default 1000, so you can follow them).

## Notes and the server log

The server keeps a journal of every game action for 7 days: each player's and the bot's
actions (refused ones too, with the reason), undos, new games and seat changes, one JSON
object a line in `logs/journal-YYYY-MM-DD.jsonl` (UTC dates). `LOG_DIR` and `LOG_DAYS`
change where and for how long (`LOG_DAYS=0` turns it off).

**Note** (top right; in **☰** on a phone) saves a note in the journal: say what looks
wrong, and the note keeps the game's options and every action so far, what your screen
was showing (the phase, the selected unit, the card you'd picked, the screen size) and
the last lines of the log. The other player doesn't see it. Later:

```bash
python3 tools/notes.py                     # list the notes, oldest first
python3 tools/notes.py --save 3 note.json  # save note 3 ...
cargo run --release --example replay -- note.json   # ... and replay to that position
```

## The table

The page fits one screen and never scrolls, laid out like a digital deck-building game:
the board in the middle between the two players' bars (the opponent above, you below,
with **End turn** at the right of your bar), your hand along the bottom, the market
(faceup row, deck and discard pile) on the left, and the details of whatever you tapped
and the log on the right. On a phone it's one column: the market as a strip of names
above the board, your hand as names below it (tap a card to read it in full), the
details sliding in between the board and your bar, and the log in a drawer (**Log**);
**☰** holds the rest.

Every change is animated, so you can follow what happened, above all when the bot plays:
units slide, turn, pop in when spawned and flash away when killed; a unit that activates
sends out a ring and names the power it used; cards fly between the market, the piles,
the hands and the units (a card the other player puts on a unit is first held up over
the board so you can read it); points and counts pop; a banner announces each turn; and
the line above the board says what just happened (tap it for the log). With
*reduce motion* set in the system, only that line changes.

## Single player in the browser

`web/` is a version that needs no server: the rules engine and the bot (with the network
in `bots/net.json`) are compiled to WebAssembly and run in the page, in a Web Worker.
You play Letters against the bot. It's published as a private Claude artifact,
[001 Solo](https://claude.ai/artifact/G66juB744M28dXyHugJiRU); share it from its Share menu.

```bash
rustup target add wasm32-unknown-unknown   # once
tools/build_web.sh                          # builds web/dist/, a static site
python3 -m http.server 3125 -d web/dist     # to try it locally
```

After changing the rules, the cards or the bot, rebuild and republish `web/dist/` (a new
card set also needs a newly trained network). `web/worker.js` sets the bot's search and the
pause before each of its actions.

**Note** works there too, without a server: the note (with the game's options and moves,
and what the screen showed) is saved in the artifact's own database, in a `notes`
collection that only the artifact's owner and editors can read or write. Claude reads it
with its artifact data tool; a note saved to a file replays with
`cargo run --release --example replay -- note.json` like a server note. (Seeds are stored
as text there, since JavaScript numbers would round them.) Anywhere else, eg served by
`python3 -m http.server`, there's no database and no Note button.

The board diagram is read from `rules.md` and the cards from `powers.md` (both in this
folder) at compile time, so after editing either one, rebuild to pick up the changes.

## Automatic and manual rules

Each game uses one of two rule sets. You pick one in **New game**, and the current
player can switch with the rules button at the top at any time.

**Automatic rules** (the default) carry out every power and only allow legal moves:

- Spaces you can spawn in are outlined. Units that can act have a dashed ring.
- Clicking a card highlights the units you can apply it to.
- Arrows in the activation panel get a green ring if the unit can use a power facing
  that way. Only usable powers have an **Activate & use** button.
- When a power needs a decision (where a lifted unit goes, which power to stun, ...), the
  options are highlighted on the board and listed in the side panel. **Cancel** abandons
  the action.
- Scoring and end-of-turn powers happen automatically.

The rules leave some details open. The decisions taken are listed in
[RULES_DECISIONS.md](RULES_DECISIONS.md), to be worked through later.

**Manual rules** are for playtesting rule changes. The app handles the bookkeeping
(setup, the draft, drawing, the faceup row, hidden hands, spawning, applying powers,
activation limits, the flag) and leaves power effects to the players:

- drag units on the board to move them (dropping onto an occupied space stacks them)
- rotate, kill, add or remove tokens, or change a unit's owner
- move or swap powers between units, discard them, or return them to your hand
- create units (for *Split*, *Trail*, ...), draw extra cards (for *Mine*), adjust scores

When a rule check blocks something that a power allows, the error has a **Do it anyway**
button. **End turn** counts the units next to the flag and offers to score.

With either rules, **Undo** reverts your actions this turn until you draw a card or end
your turn. There's also a chat box in the log.

The game lives in memory, so restarting the server ends it.

## Using the rules engine from code

The crate is also a library (`game_001`). With automatic rules:

```rust
use game_001::game::{Game, Options};

let (mut game, _log) = Game::new(Options { seed: Some(1), ..Options::default() }).unwrap();
let seat = game.current;
let actions = game.legal_actions(seat); // every legal action, including prompt answers
game.apply(seat, &actions[0]).unwrap();
```

When a power needs a decision, `apply` returns with `game.pending` set, and the current
player's legal actions are `Action::Choose { index }`, one for each option of
`game.prompt()`. Shuffles are deterministic given `Options::seed`, and `Game` is cheap
to clone. `examples/legal_actions.rs` plays a few moves
(`cargo run --example legal_actions`).

## Code

- `src/data.rs` parses the board and the cards, and maps each card to its effect.
- `src/game.rs` holds the game state and the manual rules.
- `src/rules.rs` holds the automatic rules: targeting, movement, triggers, every power,
  legal actions and prompts.
- `src/session.rs` is a game session: the game, the seats, the log, undo and the bot. It
  returns the messages to send, so the server and the browser build share it.
- `src/main.rs` is the axum server: one websocket per browser, and each one gets its own
  view of the game.
- `web/` is the single-player browser build (see above).
- `src/bot.rs`, `src/encode.rs`, `src/net.rs`, `src/bin/selfplay.rs` and `tools/` are the
  bots and the tools for training them and running balance experiments: see
  [BOT.md](BOT.md).
- `static/` holds the front end (plain HTML, CSS and JS, embedded in the binary).

## Tests

`cargo test` checks the manual rules, has a scenario for every power, and plays random
games choosing among the legal actions, checking after every action that no card was lost
or duplicated, the unit and power limits hold, and there's exactly one flag. To fuzz
harder:

```bash
SELF_PLAY_GAMES=1000 cargo test --release random_self_play -- --nocapture
```

The browser tests play complete games through the UI, with two bot players in separate
browsers, using only clicks, taps and drags. After every turn they check that both
browsers agree and hands stay hidden.

- `tests/auto_playthrough.js` uses automatic rules. It also checks that the UI highlights
  exactly the legal moves and that illegal moves are refused.
- `tests/playthrough.js` uses manual rules, and includes a game where one player is on a
  phone.
- `tests/bot_seat.js` plays a game against the server's bot (start the server with
  `BOT_DELAY_MS=50 PORT=3124`), and leaves a note.
- `tests/single_player.js` plays the single-player page (serve `web/dist` on port 3125),
  then checks on a phone that the page fits the screen and the bot's moves are animated.

```bash
PORT=3123 cargo run --release &
npm install playwright && npx playwright install chromium   # once
node tests/auto_playthrough.js
node tests/playthrough.js
```
