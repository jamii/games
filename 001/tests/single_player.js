// Plays the single-player page (web/dist, built by tools/build_web.sh), where
// the game and the bot run in the browser: checks that you're seated against
// the bot, plays random legal moves until the game ends, and checks that undo
// and a new game work and that nothing errors.
//
//   tools/build_web.sh && python3 -m http.server 3125 -d web/dist &
//   node tests/single_player.js
//
// Then, on a phone: everything fits on the screen without scrolling, and the
// bot's moves are animated. And possessing the flag (Possess + Stalker): the
// board keeps redrawing after a press released outside it, and picking a card
// then clicking the full flag explains why instead of an empty dialog. And with a stand-in for the artifact's database,
// a line typed in the chat goes in the log and is saved there as a note that
// replays the game (without one, it only goes in the log).
//
// Options (environment variables): URL (default http://localhost:3125/), SEED.
const { chromium, devices } = require('playwright');
const path = require('path');

const URL = process.env.URL || 'http://localhost:3125/';
let seed = +(process.env.SEED || 4);
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
  page.on('console', m => { if (m.type() === 'error' || m.type() === 'warning') errors.push(m.text()); });
  await page.goto(URL);

  // Seated as Letters, with the bot as Numbers, without doing anything.
  await page.waitForFunction(() => typeof view !== 'undefined' && view && view.you === 0 && view.seats[1].bot, null, { timeout: 30000 });
  const help = await page.textContent('#inspector');
  if (!help.includes('against the bot')) throw new Error('the help text is the two-player one');
  if (await page.isVisible('button[data-act=toggle-mode]')) throw new Error('the rules switch is showing');
  // No database here, so the chat only goes in the log, and doesn't say it keeps notes.
  if (await page.$('[data-act=note], [data-modal=note]')) throw new Error('there is a Note button');
  const plain = await page.getAttribute('#chat-input', 'placeholder');
  if (plain !== 'Say something…') throw new Error(`the chat box says "${plain}" without a database`);
  await page.fill('#chat-input', 'Just chatting.');
  await page.press('#chat-input', 'Enter');
  await page.waitForFunction(() => view.log.some(e => e.chat && e.text === 'Just chatting.'), null, { timeout: 5000 });
  await page.waitForTimeout(300);
  if (await page.isVisible('#toast')) throw new Error(`a toast after chatting without a database: ${await page.textContent('#toast')}`);

  // Undo: make an undoable move on my first main turn, undo it.
  let undone = false;
  // "Can't use: why?" on a power: tapping it shows the rules' reason.
  let why = null;
  let moves = 0, botTurns = 0, lastTurn = -1;
  const started = Date.now();
  for (;;) {
    const s = await page.evaluate(() => ({
      phase: view.game.phase.kind, current: view.game.current, turn: view.game.turn,
      legal: view.game.legal || [], thinking: view.seats[1].thinking, canUndo: view.can_undo,
      prompt: !!view.game.prompt, mine: view.game.units.filter(u => u.owner === 0).map(u => u.id),
    }));
    if (s.phase === 'over' || s.turn > 60) break;
    if (Date.now() - started > 300000) throw new Error('the game took too long');
    if (s.current === 1 && s.turn !== lastTurn) { botTurns++; lastTurn = s.turn; }
    if (why === null && s.current === 0 && s.phase === 'main' && !s.prompt && s.legal.length && !s.thinking) {
      for (const id of s.mine) {
        await page.click(`.unit[data-unit="${id}"] .body`).catch(() => {});
        const button = await page.$('#inspector button[data-act=why]');
        if (!button) continue;
        await button.click();
        await page.waitForFunction(() => !document.getElementById('toast').hidden, null, { timeout: 5000 });
        why = (await page.textContent('#toast')).trim();
        break;
      }
      await page.keyboard.press('Escape');
    }
    if (s.canUndo && !undone) {
      const before = await page.evaluate(() => view.log.length);
      await page.click('button[data-act=undo]');
      await page.waitForFunction(n => view.log.length > n && view.log[view.log.length - 1].text.includes('undid'), before);
      undone = true;
      continue;
    }
    if (s.legal.length > 0 && !s.thinking) {
      const end = s.legal.find(a => a.type === 'end_turn');
      const action = end && rand() < 0.25 ? end : s.legal[Math.floor(rand() * s.legal.length)];
      const sig = await page.evaluate(() => view.log.length + ':' + JSON.stringify(view.game.units));
      await page.evaluate(a => send({ type: 'action', action: a }), action);
      await page.waitForFunction(sg => view.log.length + ':' + JSON.stringify(view.game.units) !== sg
        || view.game.phase.kind === 'over', sig, { timeout: 30000 }).catch(() => {});
      moves++;
    } else {
      await page.waitForTimeout(50);
    }
  }
  const final = await page.evaluate(() => ({ phase: view.game.phase, scores: view.game.scores }));
  await page.screenshot({ path: path.join(__dirname, 'screenshots', 'single-player.png') });
  console.log(`game ${final.phase.kind}: scores ${final.scores}, my moves ${moves}, bot turns ${botTurns}, undo tested: ${undone}`);
  console.log(`"why?" on an unusable power: ${why === null ? 'never came up' : JSON.stringify(why)}`);

  // A new game starts with the bot still playing.
  await page.evaluate(() => send({ type: 'new_game', options: { automatic: true, first_player: 1 } }));
  await page.waitForFunction(() => view.game.phase.kind === 'draft' && view.game.hand_counts[1] > 0, null, { timeout: 30000 });

  await phoneCheck(browser, errors);
  await notesCheck(browser, errors);
  await possessFlagCheck(browser, errors);

  if (botTurns < 2) throw new Error('the bot did not take its turns');
  if (final.phase.kind !== 'over') throw new Error('the game did not finish');
  if (!undone) throw new Error('never got to try undo');
  if (errors.length) throw new Error('page errors: ' + errors.join('; '));
  console.log('single player test passed');
  await browser.close();
})().catch(e => { console.error(e); process.exit(1); });

// A phone: draft by tapping, watch the bot's first turn, check the layout.
async function phoneCheck(browser, errors) {
  const page = await (await browser.newContext(devices['Pixel 5'])).newPage();
  page.on('pageerror', e => errors.push(`phone: ${e.message}`));
  // The artifact viewer adds the viewport meta to the page; a plain server doesn't.
  await page.route(URL, async route => {
    const r = await route.fetch();
    route.fulfill({ response: r, body: `<meta name="viewport" content="width=device-width, initial-scale=1">${await r.text()}` });
  });
  await page.goto(URL);
  await page.waitForFunction(() => typeof view !== 'undefined' && view && view.you === 0 && view.seats[1].bot, null, { timeout: 30000 });
  // Count the cards that fly and the units that move.
  await page.evaluate(() => {
    window.__flights = 0;
    new MutationObserver(ms => ms.forEach(m => m.addedNodes.forEach(n => { if (n.classList && n.classList.contains('flight')) window.__flights++; })))
      .observe(document.getElementById('fx'), { childList: true });
  });
  for (let i = 0; i < 200; i++) {
    const s = await page.evaluate(() => ({ phase: view.game.phase.kind, current: view.game.current, turn: view.game.turn }));
    if (s.phase === 'main' && s.current === 0 && s.turn > 2) break;
    if (s.current === 0 && (s.phase === 'draft' || s.phase === 'draw')) await page.tap('#market .card[data-act=faceup]').catch(() => {});
    else if (s.current === 0 && s.phase === 'main') await page.tap('button[data-act=end-turn]').then(() => page.tap('.modal-box button[data-modal=end]')).catch(() => {});
    await page.waitForTimeout(250);
  }
  const layout = await page.evaluate(() => {
    const r = id => document.getElementById(id).getBoundingClientRect();
    const inside = b => b.top >= 0 && b.left >= 0 && b.bottom <= innerHeight + 0.5 && b.right <= innerWidth + 0.5 && b.height > 0;
    const cut = [...document.querySelectorAll('#hand .card .cname')].filter(n => n.scrollWidth > n.clientWidth + 1).map(n => n.textContent);
    return {
      scroll: [document.documentElement.scrollHeight - innerHeight, document.documentElement.scrollWidth - innerWidth],
      board: inside(r('board-wrap')) ? r('board-wrap').height / innerHeight : 0,
      hand: inside(r('hand')), me: inside(r('seat-bottom')), opp: inside(r('seat-top')),
      cut, flights: window.__flights, turn: view.game.turn,
    };
  });
  await page.screenshot({ path: path.join(__dirname, 'screenshots', 'single-player-phone.png') });
  console.log('phone:', JSON.stringify(layout));
  if (layout.scroll[0] > 0 || layout.scroll[1] > 0) throw new Error('the phone page scrolls');
  if (layout.board < 0.4) throw new Error('the board is too small on a phone');
  if (!layout.hand || !layout.me || !layout.opp) throw new Error('the hand or a player bar is off the phone screen');
  if (layout.flights === 0) throw new Error('no cards flew');
}

// The artifact viewer gives the page a database (window.claude.use('db')); a
// stand-in records what the page saves.
async function notesCheck(browser, errors) {
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 720 } })).newPage();
  page.on('pageerror', e => errors.push(`notes: ${e.message}`));
  await page.addInitScript(() => {
    window.__notes = [];
    const db = { collection: name => ({ add: async body => { window.__notes.push({ name, body: JSON.parse(JSON.stringify(body)) }); return {}; } }) };
    const user = { canEdit: async () => true };
    window.claude = { use: async name => (name === 'db' ? db : name === 'user' ? user : null) };
  });
  await page.goto(URL);
  await page.waitForFunction(() => typeof view !== 'undefined' && view && view.you === 0 && view.seats[1].bot, null, { timeout: 30000 });
  // Play into the game a little, then note.
  for (let i = 0; i < 60; i++) {
    const s = await page.evaluate(() => ({ cur: view.game.current, turn: view.game.turn, legal: view.game.legal || [] }));
    if (s.turn >= 3 && s.cur === 0) break;
    if (s.cur === 0 && s.legal.length) await page.evaluate(() => { const l = view.game.legal; send({ type: 'action', action: l.find(a => a.type === 'take_faceup') || l.find(a => a.type === 'end_turn') || l[0] }); });
    await page.waitForTimeout(250);
  }
  const placeholder = await page.getAttribute('#chat-input', 'placeholder');
  if (!/note a bug/.test(placeholder)) throw new Error(`the chat box doesn't say it takes notes: "${placeholder}"`);
  // Select a unit (the flag is always there), so the note says what was selected.
  const unit = await page.evaluate(() => (view.game.units.find(u => u.owner === 0) || view.game.units[0]).id);
  await page.click(`.unit[data-unit="${unit}"] .body`);
  await page.waitForFunction(u => ui.sel && ui.sel.unit === u, unit, { timeout: 5000 });
  const text = 'Test note from the single-player page.';
  await page.fill('#chat-input', text);
  await page.press('#chat-input', 'Enter');
  await page.waitForFunction(() => window.__notes.length > 0 && !document.getElementById('toast').hidden, null, { timeout: 10000 });
  const toast = await page.textContent('#toast span');
  const saved = await page.evaluate(() => window.__notes[0]);
  const inLog = await page.evaluate(t => view.log.some(e => e.chat && e.seat === 0 && e.text === t), text);
  const fs = require('fs');
  fs.writeFileSync(path.join(__dirname, 'screenshots', 'single-player-note.json'), JSON.stringify(saved.body));
  const b = saved.body;
  console.log(`note: saved to "${saved.name}" with ${b.actions.length} actions, turn ${b.turn}, showing "${b.context.showing}", selected ${JSON.stringify(b.context.selected)}; toast "${toast}"`);
  if (!inLog) throw new Error('the note is not in the log');
  if (saved.name !== 'notes' || b.text !== text || b.kind !== 'comment' || b.status !== 'open' || !b.time) throw new Error('the note was not saved');
  if (!b.actions.length || b.actions[0].seat === undefined || !b.options || typeof b.seed !== 'string') throw new Error('the note has no replay');
  if (!b.context.showing || !b.context.screen || !Array.isArray(b.log) || !b.log.length) throw new Error('the note does not say what the screen showed');
  if (!b.context.selected || b.context.selected.unit !== unit) throw new Error('the note does not say which unit was selected');
  if (!/Saved/.test(toast)) throw new Error(`unexpected toast: ${toast}`);
}

// Jamie's report: "I used possess and stalker to possess the flag and the game
// crashed." Possess the flag through the UI, then check the two ways the page
// could look stuck: a press on the board released outside it used to stop the
// board redrawing, and a card applied to the full flag opened a "Replace a
// power" dialog with nothing to choose.
async function possessFlagCheck(browser, errors) {
  const page = await (await browser.newContext({ viewport: { width: 1728, height: 1075 } })).newPage();
  page.on('pageerror', e => errors.push(`possess: ${e.message}`));
  await page.goto(URL);
  await page.waitForFunction(() => typeof view !== 'undefined' && view && view.you === 0 && view.seats[1].bot, null, { timeout: 30000 });
  const [POSSESS, STALKER] = await page.evaluate(() => ['Possess', 'Stalker'].map(n => init.powers.findIndex(p => p.name === n)));
  await page.evaluate(o => send({ type: 'new_game', options: o }),
    { automatic: true, first_player: 0, seed: 3, given_first: [POSSESS, STALKER] });
  await page.waitForFunction(() => view.game.options.seed === 3 && view.game.phase.kind === 'draft');
  // Take faceup cards until it's my main phase.
  const toMain = async turn => {
    for (let i = 0; i < 400; i++) {
      const s = await page.evaluate(() => ({ phase: view.game.phase.kind, current: view.game.current, turn: view.game.turn, thinking: view.seats[1].thinking }));
      if (s.phase === 'main' && s.current === 0 && s.turn >= turn) return;
      if (s.current === 0 && !s.thinking && s.phase !== 'main') await page.click('#market .card.takeable[data-act=faceup]', { timeout: 2000 }).catch(() => {});
      await page.waitForTimeout(200);
    }
    throw new Error('possess: never got to my turn');
  };
  const spawn = async () => {
    await page.click('#board .cell.spawnable[data-cell="0,1"] polygon');
    await page.click('button[data-act=spawn]');
    await page.waitForFunction(() => view.game.units.some(u => u.owner === 0 && u.cell.x === 0 && u.cell.y === 1));
    return page.evaluate(() => view.game.units.find(u => u.owner === 0 && u.cell.x === 0 && u.cell.y === 1).id);
  };
  await toMain(1);
  const me = await spawn();
  for (const card of [POSSESS, STALKER]) {
    await page.click(`#hand .card[data-act=hand][data-card="${card}"]`);
    await page.click(`#board .unit[data-unit="${me}"] .body`);
    await page.waitForFunction(([id, c]) => view.game.units.find(u => u.id === id).powers.some(p => p.power === c), [me, card]);
  }
  await page.click(`#board .unit[data-unit="${me}"] .body`);
  await page.click(`#inspector button[data-act=use][data-index="0"]`);
  await page.waitForFunction(id => !view.game.units.some(u => u.id === id), me);
  const flag = await page.evaluate(() => view.game.units.find(u => u.powers.some(p => p.power === init.flag)));
  if (flag.owner !== null || flag.powers.length !== 3) throw new Error(`possess: the flag after possessing it: ${JSON.stringify(flag)}`);
  await page.waitForTimeout(1000);

  // Press on the flag, let go over the side panel, end the turn: the bot's
  // turn has to show.
  const b = await page.locator(`#board .unit[data-unit="${flag.id}"] .body`).boundingBox();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
  await page.mouse.down();
  const side = await page.locator('#inspector').boundingBox();
  await page.mouse.move(side.x + side.width / 2, side.y + 40, { steps: 8 });
  await page.mouse.up();
  await page.click('button[data-act=end-turn]');
  await page.click('.modal-box button[data-modal=end]');
  await page.waitForFunction(() => view.game.current === 0 && view.game.turn >= 3, null, { timeout: 120000 });
  await page.waitForTimeout(1000);
  const shown = await page.evaluate(() => ({
    drawn: document.querySelectorAll('#board .unit[data-unit]').length,
    phase: document.querySelector('#topbar .phase').textContent,
  }));
  if (/turn 1\b/.test(shown.phase) || shown.drawn === 0) throw new Error(`possess: the board stopped redrawing after a press released outside it: ${JSON.stringify(shown)}`);

  // Pick a card (a unit at home makes it usable) and click the full flag.
  await toMain(3);
  await spawn();
  const card = await page.evaluate(() => view.game.hand[0]);
  await page.click(`#hand .card[data-act=hand][data-card="${card}"]`);
  const now = await page.evaluate(() => view.game.units.find(u => u.powers.some(p => p.power === init.flag)));
  if (now.owner === null && now.powers.length === 3) {
    await page.click(`#board .unit[data-unit="${now.id}"] .body`);
    await page.waitForFunction(() => !document.getElementById('toast').hidden, null, { timeout: 5000 });
    const toast = (await page.textContent('#toast span')).trim();
    if (await page.$('.modal-box')) throw new Error('possess: a "Replace a power" dialog opened for the flag');
    console.log(`possess: the flag is still full, and applying a card to it says "${toast}"`);
  } else {
    console.log(`possess: the bot changed the flag (${JSON.stringify(now.powers.map(p => p.power))}), so the full-flag click wasn't checked`);
  }
  console.log(`possess: the board followed the bot's turn after a press released outside it (${shown.phase})`);
}
