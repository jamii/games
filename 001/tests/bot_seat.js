// Plays a game against the server's bot: a browser takes the Blue seat, clicks
// "Add a bot" on the other seat, then plays random legal moves until the game
// ends. Checks that the bot takes its turns (including the draft and answering
// prompts), that removing and re-adding it works, and that nothing errors.
//
//   BOT_DELAY_MS=50 PORT=3124 cargo run --release &
//   node tests/bot_seat.js
//
// Midway it notes something in the chat box, and checks that the line is in the
// log and the server saved it in its journal as a note (with the game's moves).
//
// Options (environment variables): URL (default http://localhost:3124/), SEED,
// LOG_DIR (the server's journal: default logs/ in this folder, where the server
// writes it when started here; otherwise give both the same absolute path).
const { chromium } = require('playwright');
const path = require('path');

const URL = process.env.URL || 'http://localhost:3124/';
let seed = +(process.env.SEED || 3);
function rand() {
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

(async () => {
  const browser = await chromium.launch();
  const page = await (await browser.newContext({ viewport: { width: 1400, height: 1000 } })).newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('dialog', d => d.accept());
  await page.goto(URL);
  await page.waitForFunction(() => view && view.game);

  // Start a fresh automatic game, then sit down and invite the bot.
  await page.click('button[data-act=claim][data-seat="0"]');
  await page.waitForFunction(() => view.you === 0);
  await page.evaluate(() => send({ type: 'new_game', options: { automatic: true, seed: 11, first_player: 1 } }));
  await page.waitForFunction(() => view.game.phase.kind === 'draft' && view.game.turn === 0);
  await page.click('button[data-act=add-bot][data-seat="1"]');
  await page.waitForFunction(() => view.seats[1].bot);
  // The bot goes first, so it should draft without being asked.
  await page.waitForFunction(() => view.game.hand_counts[1] > 0, null, { timeout: 30000 });

  // Removing the bot stops it; adding it again picks up where it was.
  await page.waitForFunction(() => view.game.current === 0 && view.game.legal && view.game.legal.length > 0);
  await page.click('button[data-act=remove-bot][data-seat="1"]');
  await page.waitForFunction(() => !view.seats[1].bot);
  await page.click('button[data-act=add-bot][data-seat="1"]');
  await page.waitForFunction(() => view.seats[1].bot);

  let moves = 0, botTurns = 0, lastTurn = -1, prompts = 0, noted = false;
  const started = Date.now();
  while (true) {
    const state = await page.evaluate(() => ({
      phase: view.game.phase.kind, current: view.game.current, turn: view.game.turn,
      legal: view.game.legal || [], prompt: !!view.game.prompt, thinking: view.seats[1].thinking,
    }));
    if (state.phase === 'over' || state.turn > 60) break;
    // Midway, note something in the chat: it's in the log and the server's journal.
    if (!noted && state.turn >= 4 && state.current === 0 && !state.thinking) {
      await checkNote(page, state.turn);
      noted = true;
    }
    if (Date.now() - started > 240000) throw new Error('game took too long');
    if (state.current === 1 && state.turn !== lastTurn) { botTurns++; lastTurn = state.turn; }
    if (state.legal.length > 0 && !state.thinking) {
      // My move: something random, but end the turn now and then so games finish.
      const legal = state.legal;
      if (state.prompt) prompts++;
      const end = legal.find(a => a.type === 'end_turn');
      const action = end && rand() < 0.25 ? end : legal[Math.floor(rand() * legal.length)];
      const sig = await page.evaluate(() => view.log.length + ':' + JSON.stringify(view.game.units));
      await page.evaluate(a => send({ type: 'action', action: a }), action);
      await page.waitForFunction(s => view.log.length + ':' + JSON.stringify(view.game.units) !== s
        || view.game.phase.kind === 'over', sig, { timeout: 30000 }).catch(() => {});
      moves++;
    } else {
      await page.waitForTimeout(50);
    }
  }
  const final = await page.evaluate(() => ({ phase: view.game.phase, scores: view.game.scores, log: view.log.map(l => l.text) }));
  await page.screenshot({ path: path.join(__dirname, 'screenshots', 'bot-game.png') });
  console.log(`game ${final.phase.kind}: scores ${final.scores}, my moves ${moves}, bot turns ${botTurns}, my prompts ${prompts}`);
  if (botTurns < 2) throw new Error('the bot did not take its turns');
  if (!noted) throw new Error('never got to leave a note');
  if (errors.length) throw new Error('page errors: ' + errors.join('; '));
  if (final.phase.kind !== 'over') throw new Error('the game did not finish');
  console.log('bot seat test passed');
  await browser.close();
})().catch(e => { console.error(e); process.exit(1); });

// Types a note in the chat box and finds it in the log and in the journal.
async function checkNote(page, turn) {
  const fs = require('fs');
  const placeholder = await page.getAttribute('#chat-input', 'placeholder');
  if (!/note a bug/.test(placeholder)) throw new Error(`the chat box doesn't say it takes notes: "${placeholder}"`);
  const text = `Test note ${Date.now()}: this position looks odd.`;
  await page.fill('#chat-input', text);
  await page.press('#chat-input', 'Enter');
  await page.waitForFunction(t => view.log.some(e => e.chat && e.text === t), text, { timeout: 5000 });
  const dir = path.resolve(__dirname, '..', process.env.LOG_DIR || 'logs');
  let note = null;
  for (let i = 0; i < 20 && !note; i++) {
    const files = fs.existsSync(dir) ? fs.readdirSync(dir).filter(f => /^journal-.*\.jsonl$/.test(f)).sort() : [];
    const lines = files.length ? fs.readFileSync(path.join(dir, files[files.length - 1]), 'utf8').trim().split('\n') : [];
    note = lines.map(l => { try { return JSON.parse(l); } catch (e) { return {}; } }).find(r => r.kind === 'comment' && r.text === text);
    if (!note) await page.waitForTimeout(100);
  }
  if (!note) throw new Error(`the note isn't in the server's journal in ${dir}`);
  if (note.seat !== 0 || note.turn !== turn || !note.actions.length || !note.options || !note.context.showing) {
    throw new Error(`the note is missing details: ${JSON.stringify(note).slice(0, 300)}`);
  }
  console.log(`note: in the journal with ${note.actions.length} actions, turn ${note.turn}, showing "${note.context.showing}"`);
}
