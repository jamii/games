// Plays complete games with automatic rules through the UI, with two bot
// players in separate browsers.
//
// Each step, the bot reads the legal actions the server sent to the page,
// picks one, and carries it out the way a person would: clicking cards,
// spaces, units, facing arrows, buttons and prompt options (sometimes the
// button, sometimes the highlighted space or unit on the board). It checks
// that the UI highlights exactly the legal moves, that illegal moves are
// refused, and that both browsers agree after every turn.
//
//   PORT=3123 cargo run --release &
//   node tests/auto_playthrough.js
//
// Options (environment variables): URL (default http://localhost:3123/), SEED
// (bot choices), GAMES (default 2), OUT (screenshot folder, default tests/screenshots).
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const URL = process.env.URL || 'http://localhost:3123/';
const OUT = process.env.OUT || path.join(__dirname, 'screenshots');
const MAX_TURNS = 120;
fs.mkdirSync(OUT, { recursive: true });

let seed = +(process.env.SEED || 1);
function rand() {
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}
const pick = xs => xs[Math.floor(rand() * xs.length)];
const chance = p => rand() < p;

const DIRS = [[2, 0], [1, 1], [-1, 1], [-2, 0], [-1, -1], [1, -1]];
const dist = (a, b) => { const dx = Math.abs(a.x - b.x), dy = Math.abs(a.y - b.y); return dy + Math.max(0, (dx - dy) / 2); };

const errors = [];
const counts = {};
const tally = k => { counts[k] = (counts[k] || 0) + 1; };
function check(cond, msg) { if (!cond) throw new Error('CHECK FAILED: ' + msg); }
let INIT = null;
const ALL = [];
const shots = new Set();

async function openPlayer(browser, name) {
  const ctx = await browser.newContext({ viewport: { width: 1400, height: 1000 } });
  const page = await ctx.newPage();
  page.on('console', m => { if (m.type() === 'error') errors.push(`${name}: ${m.text()}`); });
  page.on('pageerror', e => errors.push(`${name}: ${e.message}`));
  page.on('dialog', d => d.accept());
  await page.addInitScript(() => {
    // Something that changes whenever an action does anything.
    window.__sig = () => JSON.stringify([view.game.prompt, view.game.units, view.game.hand, view.game.phase,
      view.game.current, view.game.scores, view.game.faceup, view.log.length, view.log[view.log.length - 1]]);
    window.__toasts = [];
    document.addEventListener('DOMContentLoaded', () => {
      const t = document.getElementById('toast');
      new MutationObserver(() => {
        if (!t.hidden) { const s = t.querySelector('span'); window.__toasts.push(s ? s.textContent : t.textContent); }
      }).observe(t, { attributes: true, childList: true });
    });
  });
  await page.goto(URL);
  await page.waitForFunction(() => typeof view !== 'undefined' && view && init);
  if (!INIT) INIT = await page.evaluate(() => init);
  const pl = { name, page };
  ALL.push(pl);
  return pl;
}

const V = pl => pl.page.evaluate(() => view);
const waitFor = (pl, fn, arg) => pl.page.waitForFunction(fn, arg, { timeout: 10000 });
const press = (pl, selector) => pl.page.locator(selector).first().click({ timeout: 10000 });
const takeToasts = pl => pl.page.evaluate(() => window.__toasts.splice(0));

async function selectUnit(pl, id) {
  await press(pl, `.unit[data-unit="${id}"] .body`);
  await waitFor(pl, id => document.querySelector(`.unit[data-unit="${id}"].selected`) !== null, id);
}

const isTop = (v, id) => {
  const u = v.game.units.find(x => x.id === id);
  const stack = v.game.units.filter(x => x.cell.x === u.cell.x && x.cell.y === u.cell.y);
  return stack[stack.length - 1].id === id;
};

// Carry out a legal action through the UI.
async function perform(pl, action, v) {
  switch (action.type) {
    case 'take_faceup': await press(pl, `.card[data-act=faceup][data-slot="${action.slot}"]`); break;
    case 'take_deck': await press(pl, '.pile[data-act=deck]'); break;
    case 'spawn':
      await press(pl, `.cell[data-cell="${action.cell.x},${action.cell.y}"] polygon`);
      await press(pl, 'button[data-act=spawn]');
      break;
    case 'apply_hand':
    case 'apply_free':
      await press(pl, action.type === 'apply_hand'
        ? `.card[data-act=hand][data-card="${action.card}"]`
        : `.card[data-act=free][data-power="${action.power}"]`);
      await press(pl, `.unit[data-unit="${action.unit}"] .body`);
      if (action.replace !== null) await press(pl, `.modal-box button[data-modal=choose][data-index="${action.replace}"]`);
      break;
    case 'remove_power':
      await selectUnit(pl, action.unit);
      await press(pl, `button[data-act=remove-power][data-index="${action.index}"]`);
      break;
    case 'activate':
      await selectUnit(pl, action.unit);
      await press(pl, `.fp-dir[data-facing="${action.facing}"]`);
      if (action.origin !== null) await press(pl, `button[data-act=origin][data-origin="${action.origin}"]`);
      await press(pl, action.power === null ? 'button[data-act=activate]' : `button[data-act=use][data-index="${action.power}"]`);
      break;
    case 'choose': {
      const o = v.game.prompt.options[action.index];
      const emptyCell = o.cell && !v.game.units.some(u => u.cell.x === o.cell.x && u.cell.y === o.cell.y);
      if (o.unit !== undefined && isTop(v, o.unit) && chance(0.5)) {
        await press(pl, `.unit[data-unit="${o.unit}"] .body`);
        tally('choose by clicking a unit');
      } else if (emptyCell && chance(0.5)) {
        await press(pl, `.cell[data-cell="${o.cell.x},${o.cell.y}"] polygon`);
        tally('choose by clicking a space');
      } else {
        await press(pl, `button[data-act=choose][data-index="${action.index}"]`);
        tally('choose with a button');
      }
      break;
    }
    case 'end_turn':
      await press(pl, 'button[data-act=end-turn]');
      await press(pl, '.modal-box button[data-modal=end]');
      break;
    default: throw new Error('unexpected action ' + JSON.stringify(action));
  }
}

// Prefer useful actions: spawning, moving towards the flag, using powers.
function choose(v, perTurn) {
  const legal = v.game.legal;
  if (legal[0].type === 'choose' || v.game.phase.kind !== 'main') return pick(legal);
  const end = legal.find(a => a.type === 'end_turn');
  if (perTurn > 12) return end;
  const flag = v.game.units.find(u => u.powers.some(p => p.power === INIT.flag));
  const forward = legal.filter(a => {
    if (a.type !== 'activate' || a.power === null) return false;
    const u = v.game.units.find(x => x.id === a.unit);
    const p = u.powers[a.power].power;
    if (!['Walk', 'Run', 'Rush'].includes(INIT.powers[p].name)) return false;
    const next = { x: u.cell.x + DIRS[a.facing][0], y: u.cell.y + DIRS[a.facing][1] };
    return dist(next, flag.cell) < dist(u.cell, flag.cell);
  });
  const powers = legal.filter(a => a.type === 'activate' && a.power !== null);
  const spawns = legal.filter(a => a.type === 'spawn');
  const applies = legal.filter(a => a.type === 'apply_hand' || a.type === 'apply_free');
  const others = legal.filter(a => a.type === 'remove_power' || (a.type === 'activate' && a.power === null));
  const r = rand();
  if (spawns.length && r < 0.25) return pick(spawns);
  if (forward.length && r < 0.55) return pick(forward);
  if (powers.length && r < 0.75) return pick(powers);
  if (applies.length && r < 0.88) return pick(applies);
  if (others.length && r < 0.92) return pick(others);
  return end || pick(legal);
}

// The UI must highlight exactly the legal moves.
async function checkHighlights(pl) {
  const r = await pl.page.evaluate(() => {
    const legal = view.game.legal;
    const spawns = legal.filter(a => a.type === 'spawn').length;
    const ready = new Set(legal.filter(a => a.type === 'activate' || a.type === 'remove_power').map(a => a.unit)).size;
    return {
      spawns, ready,
      shownSpawns: document.querySelectorAll('#board .cell.spawnable').length,
      shownReady: document.querySelectorAll('#board .unit .ready').length,
    };
  });
  check(r.spawns === r.shownSpawns, `spawnable spaces shown (${r.shownSpawns}) match legal spawns (${r.spawns})`);
  check(r.ready === r.shownReady, `units marked ready (${r.shownReady}) match units with legal actions (${r.ready})`);
}

async function checkPromptHighlights(pl) {
  const r = await pl.page.evaluate(() => {
    const o = view.game.prompt.options;
    return {
      cells: new Set(o.filter(x => x.cell).map(x => `${x.cell.x},${x.cell.y}`)).size,
      units: new Set(o.filter(x => x.unit !== undefined).map(x => x.unit)).size,
      shownCells: document.querySelectorAll('#board .cell.choice').length,
      shownUnits: document.querySelectorAll('#board .unit.choice').length,
      buttons: document.querySelectorAll('#inspector button[data-act=choose]').length,
      options: o.length,
    };
  });
  check(r.cells === r.shownCells && r.units === r.shownUnits, `prompt highlights match its options ${JSON.stringify(r)}`);
  check(r.buttons === r.options, 'a button for every option');
}

// Clicking a hand card highlights exactly the units it can legally be applied to.
async function checkApplyTargets(pl) {
  const v = await V(pl);
  const cards = [...new Set(v.game.legal.filter(a => a.type === 'apply_hand').map(a => a.card))];
  if (!cards.length) return;
  const card = pick(cards);
  await press(pl, `.card[data-act=hand][data-card="${card}"]`);
  const r = await pl.page.evaluate(card => ({
    legal: new Set(view.game.legal.filter(a => a.type === 'apply_hand' && a.card === card).map(a => a.unit)).size,
    shown: document.querySelectorAll('#board .unit.target').length,
  }), card);
  check(r.legal === r.shown, `apply targets shown (${r.shown}) match legal ones (${r.legal})`);
  await press(pl, `.card[data-act=hand][data-card="${card}"]`); // deselect
  tally('checked apply targets');
}

// Illegal actions are refused by the server, and unhighlighted things do nothing.
async function checkIllegal(pl) {
  const v = await V(pl);
  const unit = v.game.units[0];
  const before = await pl.page.evaluate(() => __sig());
  await pl.page.evaluate(id => send({ type: 'action', action: { type: 'kill', unit: id } }), unit.id);
  await pl.page.waitForSelector('#toast:not([hidden])', { timeout: 10000 });
  const toasts = await takeToasts(pl);
  check(toasts.some(t => /manual rules/.test(t)), 'manual actions are refused: ' + JSON.stringify(toasts));
  await pl.page.evaluate(() => send({ type: 'action', action: { type: 'take_deck' } }));
  await pl.page.waitForTimeout(300);
  check(await pl.page.evaluate(() => __sig()) === before, 'an illegal draw changed nothing');
  await takeToasts(pl);
  tally('illegal actions refused');
}

// Switch to manual rules and back in the middle of a turn.
async function checkToggle(pl, unit) {
  await press(pl, 'button[data-act=toggle-mode]');
  await waitFor(pl, () => !view.game.automatic && view.game.legal.length === 0);
  await selectUnit(pl, unit);
  check(await pl.page.locator('#inspector button[data-act=kill]').count() === 1, 'manual tools appear with manual rules');
  await press(pl, 'button[data-act=toggle-mode]');
  await waitFor(pl, () => view.game.automatic && view.game.legal.length > 0);
  check(await pl.page.locator('#inspector button[data-act=kill]').count() === 0, 'and disappear with automatic rules');
  await press(pl, `.unit[data-unit="${unit}"] .body`);
  tally('switched rules and back');
}

async function synced(players) {
  const key = v => JSON.stringify([v.game.units, v.game.scores, v.game.current, v.game.phase, v.game.turn,
    v.game.faceup, v.game.discard, v.game.deck_count, v.game.hand_counts, v.game.prompt && v.game.prompt.text]);
  for (let i = 0; i < 100; i++) {
    const vs = await Promise.all(players.map(V));
    if (vs.every(v => key(v) === key(vs[0]))) return vs;
    await players[0].page.waitForTimeout(50);
  }
  throw new Error('CHECK FAILED: the two browsers disagree about the game state');
}

async function checkInvariants(players) {
  const vs = await synced(players);
  const g = vs[0].game;
  for (const v of vs) check(v.game.hand.length === v.game.hand_counts[v.you], 'own hand matches the count');
  const [h0, h1] = vs.map(v => v.game.hand);
  check(!h0.some(c => h1.includes(c)), 'each player sees only their own hand');
  const onUnits = g.units.flatMap(u => u.powers.filter(p => p.card).map(p => p.power));
  const visible = [...g.faceup, ...g.discard, ...h0, ...h1, ...onUnits];
  check(new Set(visible).size === visible.length, 'no card is in two places');
  check(visible.length + g.deck_count === INIT.powers.length - 1 - g.free_powers.length, 'no card lost');
  check(g.units.filter(u => u.powers.some(p => p.power === INIT.flag)).length === 1, 'exactly one flag');
  for (const s of [0, 1]) check(g.units.filter(u => u.owner === s).length <= 5, 'at most 5 units');
  check(g.units.every(u => u.powers.length <= 3), 'at most 3 powers per unit');
  return vs;
}

async function claim(pl, seat) {
  await press(pl, `button[data-act=claim][data-seat="${seat}"]`);
  await waitFor(pl, s => view.you === s, seat);
  pl.seat = seat;
}

async function newGame(pl, o) {
  const page = pl.page;
  await press(pl, 'button[data-act=new-game]');
  await page.waitForSelector('#ng-first');
  await page.selectOption('#ng-rules', 'auto');
  await page.selectOption('#ng-first', o.first === null ? '' : String(o.first));
  await page.selectOption('#ng-free', { label: o.free || 'Walk' });
  await page.setChecked('#ng-y', !!o.y);
  await page.setChecked('#ng-flag', !!o.flag);
  await page.setChecked('#ng-skip', !!o.skip);
  await page.fill('#ng-target', String(o.target || 5));
  await page.fill('#ng-bonus', String(o.bonus ?? 1));
  await press(pl, '.modal-box button[data-modal=start]');
  await waitFor(pl, () => view.game.automatic && view.game.phase.kind === 'draft' && view.game.hand.length === 0
    && view.log[view.log.length - 1].text.startsWith('Draft'));
}

async function playGame(players, starter, options, label) {
  await newGame(starter, options);
  let checkedIllegal = false;
  let toggled = false;
  let turn = -1, perTurn = 0, steps = 0;
  while (true) {
    const vs = await synced(players);
    const g = vs[0].game;
    if (g.phase.kind === 'over') break;
    check(g.turn <= MAX_TURNS, `game finished within ${MAX_TURNS} turns`);
    if (g.turn !== turn) {
      await checkInvariants(players);
      turn = g.turn;
      perTurn = 0;
    }
    const seat = g.prompt ? g.prompt.seat : g.current;
    const pl = players.find(p => p.seat === seat);
    const other = players.find(p => p.seat !== seat);
    const v = await V(pl);
    check(v.game.legal.length > 0, 'the player has legal actions');
    check((await V(other)).game.legal.length === 0, 'the other player has none');
    if (v.game.prompt) {
      await checkPromptHighlights(pl);
      if (!shots.has(v.game.prompt.text.split(':')[0])) {
        shots.add(v.game.prompt.text.split(':')[0]);
        await pl.page.screenshot({ path: path.join(OUT, `prompt-${shots.size}.png`) });
      }
    } else if (v.game.phase.kind === 'main') {
      await checkHighlights(pl);
      if (chance(0.1)) await checkApplyTargets(pl);
      if (!checkedIllegal) { await checkIllegal(pl); checkedIllegal = true; }
      if (!toggled && g.turn >= 3 && v.game.units.some(u => u.owner === seat)) {
        await checkToggle(pl, v.game.units.find(u => u.owner === seat).id);
        toggled = true;
      }
    }
    const action = choose(v, perTurn);
    perTurn += 1;
    steps += 1;
    const before = await pl.page.evaluate(() => __sig());
    await perform(pl, action, v);
    tally(action.type === 'activate' && action.power !== null ? 'activate + use a power' : action.type);
    await pl.page.waitForFunction(b => __sig() !== b, before, { timeout: 10000 });
    const toasts = await takeToasts(pl);
    check(toasts.length === 0, `${pl.name}: unexpected error ${JSON.stringify(toasts)} after ${JSON.stringify(action)}`);
  }
  const [v] = await synced(players);
  const winner = v.game.phase.winner;
  for (const pl of players) {
    check((await pl.page.textContent('#topbar .phase')).includes(`${INIT.seat_names[winner]} wins!`), 'winner shown');
    await pl.page.evaluate(() => window.scrollTo(0, 0));
    await pl.page.screenshot({ path: path.join(OUT, `${label}-${pl.name}.png`) });
  }
  console.log(`${label}: ${INIT.seat_names[winner]} won ${v.game.scores.join('-')} after ${v.game.turn} turns and ${steps} UI actions`);
}

(async () => {
  const browser = await chromium.launch();
  const a = await openPlayer(browser, 'letters');
  const b = await openPlayer(browser, 'numbers');
  await claim(a, 0);
  await claim(b, 1);
  const games = +(process.env.GAMES || 2);
  const setups = [
    [a, { first: null }, 'auto-default'],
    [b, { first: 1, free: 'Run', y: true, flag: true, skip: true, target: 4, bonus: 2 }, 'auto-advanced'],
  ];
  for (let i = 0; i < games; i++) {
    const [starter, options, label] = setups[i % setups.length];
    await playGame([a, b], starter, options, `${label}-${i + 1}`);
  }
  console.log('\nUI actions performed:');
  for (const [k, n] of Object.entries(counts).sort((x, y) => y[1] - x[1])) console.log(`  ${String(n).padStart(5)}  ${k}`);
  check(errors.length === 0, 'no console errors: ' + errors.join(' | '));
  console.log('\nAll games finished with no errors.');
  await browser.close();
})().catch(async e => {
  console.error(e);
  for (const pl of ALL) {
    try {
      await pl.page.screenshot({ path: path.join(OUT, `FAIL-${pl.name}.png`) });
      console.error(pl.name, JSON.stringify(await pl.page.evaluate(() => ({
        prompt: view.game.prompt, phase: view.game.phase, current: view.game.current, legal: view.game.legal.length,
        toasts: window.__toasts, last: view.log.slice(-6).map(e => e.text),
      }))));
    } catch (err) { /* page closed */ }
  }
  console.error('console errors:', errors);
  process.exit(1);
});
