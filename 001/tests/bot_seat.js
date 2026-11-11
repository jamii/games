// Plays a game against the server's bot: a browser takes the Blue seat, clicks
// "Add a bot" on the other seat, then plays random legal moves until the game
// ends. Checks that the bot takes its turns (including the draft and answering
// prompts), that removing and re-adding it works, and that nothing errors.
//
//   BOT_DELAY_MS=50 PORT=3124 cargo run --release &
//   node tests/bot_seat.js
//
// Midway it leaves a note with the Note button, and checks the server saved it.
//
// Options (environment variables): URL (default http://localhost:3124/), SEED.
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
    // Midway, leave a note: the server confirms it's in its log.
    if (!noted && state.turn >= 4 && state.current === 0 && !state.thinking) {
      await page.click('#topbar button[data-act=note]');
      await page.fill('#note-text', 'Test note: this position looks odd.');
      await page.click('.modal-box button[data-modal=save]');
      await page.waitForFunction(() => !document.getElementById('toast').hidden
        && /Noted/.test(document.getElementById('toast').textContent), null, { timeout: 5000 });
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
