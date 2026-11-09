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
- `src/main.rs` is the axum server: one websocket per browser, and each one gets its own
  view of the game.
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

```bash
PORT=3123 cargo run --release &
npm install playwright && npx playwright install chromium   # once
node tests/auto_playthrough.js
node tests/playthrough.js
```
