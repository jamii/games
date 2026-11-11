# 001: board game rules engine, web app, bots and balance

## Working on it
- Repo `jamii/games`, folder `001/`. Develop on `main` and push there unless asked for a branch.
- Never edit a script while it's running (write a new file, then `mv` it into place).
- Long jobs (training, card surveys) run with `nohup`; after a container restart, check they're still running and resume them (`training/cards-net10.sh` skips cards already measured).
- After changing the engine or the front end, rebuild the server (`static/` is embedded in the binary), run the tests, rebuild the browser build and republish the artifact.

## What's where
- **Rules:** `src/game.rs` (state, manual rules), `src/rules.rs` (automatic rules: targeting, every power, legal actions, prompts), `src/rules/tests.rs`. Rules interpretations are in `RULES_DECISIONS.md`.
- **Web app:** `src/session.rs` (seats, log, undo, bot, journal), `src/main.rs` (axum server), `static/` (front end: one-screen table, animations). See `README.md`.
- **Single player in the browser:** `web/` (WebAssembly build, worker, socket shim), built by `tools/build_web.sh` into `web/dist/`. Published as the artifact "001 Solo": https://claude.ai/artifact/G66juB744M28dXyHugJiRU (republish from `001/web/dist/index.html`, with `app.js`, `style.css`, `worker.js`, `local-socket.js`, `net.json` and `game.wasm` as `application/wasm`).
- **Bots:** `src/bot.rs` (search), `src/net.rs` + `tools/train.py` + `tools/alphazero.sh` (trained network). The best network is net10, in `bots/net.json`. See `BOT.md`.
- **Balance:** `tools/card_value.sh CARD` measures a card by giving it to a player on the same deals; `tools/card_table.py` rebuilds the card table in `BOT.md`. Main finding: balance breaks at the flag (Heavy and Trade dominate with the trained bot); recommendation: fix the flag in place and give the second player 2 points.

## Bug notes from play
- The server logs every action for 7 days in `logs/journal-YYYY-MM-DD.jsonl`; the **Note** button adds the player's note with the game's actions and what their screen showed.
- Notes from the 001 Solo artifact are in its database, collection `notes` (one document each, `status: "open"` until fixed): read them with the ArtifactData tool (`list`, collection `notes`), save one's body to a file and replay it the same way.
- To work through them: `python3 tools/notes.py` lists the server's notes; `python3 tools/notes.py --save N note.json` and `cargo run --release --example replay -- note.json` rebuild the exact position. Fix each, add a regression test, mark artifact notes `status: "fixed"`, and say which notes were fixed.

## Tests
- `cargo test --release` (rules, session, journal).
- Browser tests in `tests/` (Playwright): `auto_playthrough.js` and `playthrough.js` on a server at port 3123, `bot_seat.js` on port 3124 with `BOT_DELAY_MS=50`, `single_player.js` on `web/dist` served at port 3125.
