# 001 web app

A small Rust web server for playing one game of 001 between two browsers.

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

The board diagram is read from `../rules.md` and the cards from `../powers.md` at
compile time, so after editing either one, rebuild to pick up the changes.

## What the app does for you

- Setup: free power (*Walk* by default), 6 faceup cards, the flag on F, the draft until
  each player has 3 cards, and 1 point for the second player.
- Each turn: discards the leftmost faceup card, refills the row, and asks the current
  player to draw 2 (from the faceup row or the deck).
- Hands are hidden from the other player and from spectators.
- Checks spawning (own empty home space, once per turn, at most 5 units), applying and
  discarding powers (own units in home spaces, at most 3 powers, choose one to replace),
  and activation (own units or the flag, top of the stack, once per turn, twice with
  *Fast*, active powers only).
- If the *Flag* power ever leaves the board, a new flag appears on F.
- **End turn** counts the units next to the flag (including *Strong*) and offers to score.
  The first player to reach 5 points wins.

## What you do by hand

The power rules are still being playtested, so the app doesn't carry out power effects.
Activating a unit records the facing and which power was used (with its text) in the log,
then the current player carries out the effect:

- drag units on the board to move them (dropping onto an occupied space stacks them)
- rotate, kill, add or remove tokens, or change a unit's owner
- move or swap powers between units, discard them, or return them to your hand
- create units (for *Split*, *Trail*, ...), draw extra cards (for *Mine*), adjust scores

When a rule check blocks something that a power allows, the error has a **Do it anyway**
button. **Undo** reverts your actions this turn, until you draw a card or end your turn.
There's also a chat box in the log.

## Interpretations

Some details aren't spelled out in the rules. This is what the app currently does:

- "Draw 2 cards" works like drawing during setup: take a faceup card (and refill its slot)
  or the top card of the deck.
- When the deck runs out, the discard pile is shuffled to make a new deck.
- Free powers and the *Flag* power disappear when removed instead of going to the discard
  pile.
- New units face the opposite side of the board.
- **New game** has options for the advanced setup (another free power, an extra power on
  the flag), the "first player doesn't draw on their first turn" variant, the target
  score and the second player's starting points.

The game lives in memory, so restarting the server ends it.

## Code

- `src/data.rs` parses the board and the cards.
- `src/game.rs` holds the game state and rules (`cargo test` runs the rule tests).
- `src/main.rs` is the axum server: one websocket per browser, and each one gets its own
  view of the game.
- `static/` holds the front end (plain HTML, CSS and JS, embedded in the binary).

## Tests

`cargo test` checks the rules. `tests/playthrough.js` plays three complete games in real
browsers, using only clicks, taps and drags: default options, all the advanced options,
and a game where one player is on a phone. After every turn it checks that both browsers
agree, hands stay hidden, no card is lost or duplicated, and nothing shows an error.

```bash
PORT=3123 cargo run --release &
npm install playwright && npx playwright install chromium   # once
node tests/playthrough.js
```
