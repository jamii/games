// Plays complete games of 001 with manual rules through the UI, with two bot players in
// separate browsers. (tests/auto_playthrough.js does the same with automatic rules.)
//
// The bots only use the page the way a person would (clicking, tapping, dragging), and read
// the page's state to decide what to do and to check that every turn left the game consistent
// in both browsers. It plays three games: default options, every advanced option, and one
// where Letters switches to a phone and plays with touch.
//
//   PORT=3123 cargo run --release &
//   npm install playwright && npx playwright install chromium   # once
//   node tests/playthrough.js
//
// Options (environment variables): URL (default http://localhost:3123/), SEED (bot choices),
// PHONE_ONLY=1 (just the phone game), OUT (screenshot folder, default tests/screenshots).
const { chromium, devices } = require('playwright');
const fs = require('fs');
const path = require('path');

const URL = process.env.URL || 'http://localhost:3123/';
const OUT = process.env.OUT || path.join(__dirname, 'screenshots');
const MAX_TURNS = 150;
fs.mkdirSync(OUT, { recursive: true });

// Deterministic bot choices (the server's shuffles are still random).
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
const step = (c, d) => ({ x: c.x + DIRS[d][0], y: c.y + DIRS[d][1] });
const same = (a, b) => a.x === b.x && a.y === b.y;
const dist = (a, b) => { const dx = Math.abs(a.x - b.x), dy = Math.abs(a.y - b.y); return dy + Math.max(0, (dx - dy) / 2); };

const errors = [];
const counts = {};
const tally = k => { counts[k] = (counts[k] || 0) + 1; };
function check(cond, msg) { if (!cond) throw new Error('CHECK FAILED: ' + msg); }
let INIT = null;
const ALL = [];

// ---------------------------------------------------------------------------
// Browser helpers
// ---------------------------------------------------------------------------

async function openPlayer(browser, name, phone = false) {
  const ctx = await browser.newContext(phone ? { ...devices['Pixel 5'] } : { viewport: { width: 1400, height: 1000 } });
  const page = await ctx.newPage();
  page.on('console', m => { if (m.type() === 'error') errors.push(`${name}: ${m.text()}`); });
  page.on('pageerror', e => errors.push(`${name}: ${e.message}`));
  page.on('dialog', d => d.accept());
  await page.addInitScript(() => {
    window.__toasts = [];
    window.__events = [];
    for (const type of ['pointerdown', 'pointerup', 'pointercancel', 'touchstart', 'touchend', 'mousedown', 'mouseup', 'click']) {
      document.addEventListener(type, e => {
        const t = e.target;
        const desc = t && t.closest ? (t.closest('[data-act],[data-modal],[data-unit],[data-cell]') || t).outerHTML.slice(0, 70) : String(t);
        window.__events.push(`${type} ${e.pointerType || ''} ${desc}`);
        if (window.__events.length > 30) window.__events.shift();
      }, true);
    }
    document.addEventListener('DOMContentLoaded', () => {
      const t = document.getElementById('toast');
      new MutationObserver(() => {
        if (!t.hidden) { const s = t.querySelector('span'); window.__toasts.push(s ? s.textContent : t.textContent); }
      }).observe(t, { attributes: true, childList: true });
    });
  });
  await page.goto(URL);
  await page.waitForFunction(() => typeof view !== 'undefined' && view && init);
  const pl = { name, page, ctx, phone };
  if (phone) pl.cdp = await ctx.newCDPSession(page);
  if (!INIT) INIT = await page.evaluate(() => init);
  ALL.push(pl);
  return pl;
}

const V = pl => pl.page.evaluate(() => view);
const waitFor = (pl, fn, arg) => pl.page.waitForFunction(fn, arg, { timeout: 8000 });

async function press(pl, selector) {
  const loc = pl.page.locator(selector).first();
  if (pl.phone) await loc.tap({ timeout: 8000 });
  else await loc.click({ timeout: 8000 });
}

async function takeToasts(pl) { return pl.page.evaluate(() => window.__toasts.splice(0)); }
async function noToasts(pl, when) {
  const t = await takeToasts(pl);
  check(t.length === 0, `${pl.name}: unexpected error toast ${JSON.stringify(t)} (${when})`);
}
async function expectToast(pl, re) {
  await pl.page.waitForSelector('#toast:not([hidden])', { timeout: 8000 });
  const t = await takeToasts(pl);
  check(t.some(x => re.test(x)), `${pl.name}: expected a toast matching ${re}, got ${JSON.stringify(t)}`);
}

// Drag a unit to a space with the mouse, or with a finger on the phone.
async function drag(pl, unitId, dest) {
  const page = pl.page;
  await page.evaluate(() => document.getElementById('board').scrollIntoView({ block: 'center' }));
  const from = await page.locator(`.unit[data-unit="${unitId}"] .body`).boundingBox();
  const to = await page.locator(`.cell[data-cell="${dest.x},${dest.y}"] polygon`).boundingBox();
  const fx = from.x + from.width / 2, fy = from.y + from.height / 2;
  const tx = to.x + to.width / 2, ty = to.y + to.height / 2;
  const steps = 8;
  if (pl.phone) {
    const touch = (type, pts) => pl.cdp.send('Input.dispatchTouchEvent', { type, touchPoints: pts });
    await touch('touchStart', [{ x: fx, y: fy }]);
    for (let i = 1; i <= steps; i++) await touch('touchMove', [{ x: fx + (tx - fx) * i / steps, y: fy + (ty - fy) * i / steps }]);
    await touch('touchEnd', []);
    // Chrome ignores a tap that comes within ~150ms of the end of a drag; people are slower than that.
    await page.waitForTimeout(400);
  } else {
    await page.mouse.move(fx, fy);
    await page.mouse.down();
    for (let i = 1; i <= steps; i++) await page.mouse.move(fx + (tx - fx) * i / steps, fy + (ty - fy) * i / steps);
    await page.mouse.up();
  }
}

// ---------------------------------------------------------------------------
// Reading the state
// ---------------------------------------------------------------------------

function analyze(v) {
  const g = v.game, me = v.you;
  const cells = new Map(INIT.board.map(c => [`${c.x},${c.y}`, c]));
  const at = c => g.units.filter(u => same(u.cell, c));
  const top = u => { const s = at(u.cell); return s[s.length - 1].id === u.id; };
  const isFlag = u => u.powers.some(p => p.power === INIT.flag);
  const homeOf = (c, s) => { const i = cells.get(`${c.x},${c.y}`); return !!i && i.kind === 'home' && i.seat === s; };
  return {
    g, me, at, top, isFlag, homeOf,
    onBoard: c => cells.has(`${c.x},${c.y}`),
    flag: g.units.find(isFlag),
    mine: g.units.filter(u => u.owner === me),
    homeCells: s => INIT.board.filter(c => c.kind === 'home' && c.seat === s),
  };
}

const unitIn = (v, id) => v.game.units.find(u => u.id === id);

async function synced(players) {
  const key = v => JSON.stringify([v.game.units, v.game.scores, v.game.current, v.game.phase, v.game.turn,
    v.game.faceup, v.game.discard, v.game.deck_count, v.game.hand_counts]);
  for (let i = 0; i < 80; i++) {
    const vs = await Promise.all(players.map(V));
    if (vs.every(v => key(v) === key(vs[0]))) return vs;
    await players[0].page.waitForTimeout(50);
  }
  throw new Error('CHECK FAILED: the two browsers disagree about the game state');
}

async function checkInvariants(players) {
  const vs = await synced(players);
  const g = vs[0].game;
  for (const v of vs) {
    check(v.game.hand.length === v.game.hand_counts[v.you], `${v.you}: own hand matches the count`);
  }
  const [h0, h1] = vs.map(v => v.game.hand);
  check(!h0.some(c => h1.includes(c)), 'hands are disjoint');
  // Every card is in exactly one place.
  const onUnits = g.units.flatMap(u => u.powers.filter(p => p.card).map(p => p.power));
  const visible = [...g.faceup, ...g.discard, ...h0, ...h1, ...onUnits];
  check(new Set(visible).size === visible.length, 'no card is in two places');
  const total = INIT.powers.length - 1 - g.free_powers.length;
  check(visible.length + g.deck_count === total, `card count ${visible.length + g.deck_count} === ${total}`);
  for (const s of [0, 1]) {
    const us = g.units.filter(u => u.owner === s);
    check(us.length <= 5, 'at most 5 units');
    check(new Set(us.map(u => u.label)).size === us.length, 'unit labels are unique');
  }
  check(g.units.every(u => u.powers.length <= 3), 'at most 3 powers per unit');
  check(g.units.filter(u => u.powers.some(p => p.power === INIT.flag)).length === 1, 'exactly one flag');
  return vs;
}

// ---------------------------------------------------------------------------
// The bot
// ---------------------------------------------------------------------------

async function selectUnit(pl, id) {
  await press(pl, `.unit[data-unit="${id}"] .body`);
  await waitFor(pl, id => document.querySelector(`.unit[data-unit="${id}"].selected`) !== null, id);
}

async function activate(pl, u, facing, powerIndex) {
  await selectUnit(pl, u.id);
  await press(pl, `.fp-dir[data-facing="${facing}"]`);
  if (powerIndex === null) await press(pl, 'button[data-act=activate]');
  else await press(pl, `button[data-act=use][data-index="${powerIndex}"]`);
  await waitFor(pl, ([id, n, f]) => { const x = view.game.units.find(x => x.id === id); return x.activations === n + 1 && x.facing === f; },
    [u.id, u.activations, facing]);
}

async function draw(pl) {
  const v = await V(pl);
  const n = v.game.hand.length;
  if (v.game.faceup.length && chance(0.5)) {
    await press(pl, `.card[data-act=faceup][data-slot="${Math.floor(rand() * v.game.faceup.length)}"]`);
    tally(v.game.phase.kind === 'draft' ? 'draft: faceup' : 'draw: faceup');
  } else {
    await press(pl, '.pile[data-act=deck]');
    tally(v.game.phase.kind === 'draft' ? 'draft: deck' : 'draw: deck');
  }
  await waitFor(pl, n => view.game.hand.length === n + 1, n);
}

async function playTurn(pl, opp) {
  let v = await V(pl);
  const me = v.you;
  check(v.game.current === me, `${pl.name} plays on its own turn`);

  // The other player can't do anything.
  if (chance(0.15)) {
    const before = JSON.stringify((await V(opp)).game.hand_counts);
    await press(opp, '.pile[data-act=deck]');
    await opp.page.waitForTimeout(150);
    check(JSON.stringify((await V(opp)).game.hand_counts) === before, 'the other player cannot draw out of turn');
    check(await opp.page.locator('button[data-act=end-turn]').count() === 0, 'the other player has no end turn button');
    tally('out-of-turn click ignored');
  }

  while ((await V(pl)).game.phase.kind === 'draw') await draw(pl);
  v = await V(pl);
  check(v.game.phase.kind === 'main', 'main phase after drawing');
  let a = analyze(v);
  const mover = v.game.free_powers[0];

  // Spawn on a highlighted home space.
  if (a.mine.length < 5 && chance(0.85)) {
    const spawnable = await pl.page.$$eval('.cell.spawnable', els => els.map(e => e.dataset.cell));
    if (spawnable.length) {
      await press(pl, `.cell[data-cell="${pick(spawnable)}"] polygon`);
      await press(pl, 'button[data-act=spawn]');
      await waitFor(pl, ([me, n]) => view.game.units.filter(u => u.owner === me).length === n + 1, [me, a.mine.length]);
      tally('spawn');
      // Sometimes try a second spawn: it's blocked, then override it.
      v = await V(pl); a = analyze(v);
      const free = a.homeCells(me).filter(c => a.at(c).length === 0);
      if (a.mine.length < 5 && free.length && chance(0.12)) {
        await press(pl, `.cell[data-cell="${free[0].x},${free[0].y}"] polygon`);
        await press(pl, 'button[data-act=spawn]');
        await expectToast(pl, /one unit per turn/);
        await press(pl, 'button[data-act=retry]');
        await waitFor(pl, ([me, n]) => view.game.units.filter(u => u.owner === me).length === n + 1, [me, a.mine.length]);
        tally('second spawn (overridden)');
      }
    }
  }

  // Give the free movement power to units at home that lack it.
  v = await V(pl); a = analyze(v);
  for (const u of a.mine.filter(u => a.homeOf(u.cell, me) && a.top(u) && u.powers.length < 3 && !u.powers.some(p => p.power === mover))) {
    await press(pl, `.card[data-act=free][data-power="${mover}"]`);
    await press(pl, `.unit[data-unit="${u.id}"] .body`);
    await waitFor(pl, ([id, n]) => view.game.units.find(x => x.id === id).powers.length === n + 1, [u.id, u.powers.length]);
    tally('apply free power');
  }

  // Apply a card from hand to a unit at home, replacing a power if it's full.
  v = await V(pl); a = analyze(v);
  const atHome = a.mine.filter(u => a.homeOf(u.cell, me) && a.top(u));
  if (v.game.hand.length && atHome.length && chance(0.7)) {
    const u = pick(atHome), card = pick(v.game.hand);
    await press(pl, `.card[data-act=hand][data-card="${card}"]`);
    await press(pl, `.unit[data-unit="${u.id}"] .body`);
    if (u.powers.length >= 3) {
      const i = Math.max(0, u.powers.findIndex(p => p.power !== mover));
      await press(pl, `.modal-box button[data-modal=choose][data-index="${i}"]`);
      tally('apply card (replacing a power)');
    } else {
      tally('apply card');
    }
    await waitFor(pl, ([c, id]) => !view.game.hand.includes(c) && view.game.units.find(x => x.id === id).powers.some(p => p.power === c), [card, u.id]);
  }

  // Keep the hand from growing forever.
  v = await V(pl);
  while (v.game.hand.length > 6) {
    const card = pick(v.game.hand);
    await press(pl, `.card[data-act=hand][data-card="${card}"]`);
    await press(pl, `button[data-act=discard-card][data-card="${card}"]`);
    await waitFor(pl, c => !view.game.hand.includes(c) && view.game.discard.includes(c), card);
    tally('discard from hand');
    v = await V(pl);
  }

  // Activate each unit: attack an adjacent enemy, or walk towards the flag.
  for (const id of analyze(await V(pl)).mine.map(u => u.id)) {
    v = await V(pl); a = analyze(v);
    const u = unitIn(v, id);
    if (!u || !a.top(u) || u.activations >= u.max_activations) continue;
    const enemies = [0, 1, 2, 3, 4, 5]
      .map(d => ({ d, e: a.at(step(u.cell, d)).slice(-1)[0] }))
      .filter(({ e }) => e && e.owner === 1 - me);
    if (enemies.length && chance(0.5)) {
      const { d, e } = pick(enemies);
      const attack = u.powers.findIndex(p => INIT.powers[p.power].kind === 'active' && p.power !== mover);
      await activate(pl, u, d, attack >= 0 ? attack : null);
      await selectUnit(pl, e.id);
      await press(pl, 'button[data-act=kill]');
      await waitFor(pl, id => !view.game.units.some(x => x.id === id), e.id);
      tally('activate + kill an adjacent enemy');
      continue;
    }
    const moverIndex = u.powers.findIndex(p => p.power === mover);
    const here = dist(u.cell, a.flag.cell);
    if (moverIndex >= 0 && here > 1) {
      const options = [0, 1, 2, 3, 4, 5].map(d => ({ d, c: step(u.cell, d) }))
        .filter(({ c }) => a.onBoard(c) && a.at(c).length === 0 && !a.homeOf(c, 1 - me));
      const closer = options.filter(o => dist(o.c, a.flag.cell) < here);
      const choice = closer.length ? pick(closer) : options.length && chance(0.3) ? pick(options) : null;
      if (choice) {
        await activate(pl, u, choice.d, moverIndex);
        await drag(pl, u.id, choice.c);
        await waitFor(pl, ([id, c]) => { const x = view.game.units.find(x => x.id === id); return x.cell.x === c.x && x.cell.y === c.y; }, [u.id, choice.c]);
        tally('activate + drag forward');
        continue;
      }
    }
    if (chance(0.25)) {
      await activate(pl, u, Math.floor(rand() * 6), null);
      tally('activate (rotate only)');
    }
  }

  // Exercise the manual tools now and then.
  v = await V(pl); a = analyze(v);
  const tops = a.mine.filter(u => a.top(u));
  if (tops.length && chance(0.15)) {
    const u = pick(tops);
    await selectUnit(pl, u.id);
    await press(pl, 'button[data-act=tokens][data-delta="1"]');
    await waitFor(pl, ([id, n]) => view.game.units.find(x => x.id === id).tokens === n + 1, [u.id, u.tokens]);
    await press(pl, 'button[data-act=undo]');
    await waitFor(pl, ([id, n]) => view.game.units.find(x => x.id === id).tokens === n, [u.id, u.tokens]);
    tally('add token, then undo');
  }
  if (tops.length && chance(0.12)) {
    const u = pick(tops);
    await selectUnit(pl, u.id);
    await press(pl, 'button[data-act=rotate][data-delta="1"]');
    await waitFor(pl, ([id, f]) => view.game.units.find(x => x.id === id).facing === f, [u.id, (u.facing + 1) % 6]);
    tally('manual rotate');
  }
  const givers = tops.filter(u => u.powers.some(p => p.card));
  if (givers.length && tops.length > 1 && chance(0.15)) {
    const u = pick(givers), t = pick(tops.filter(x => x.id !== u.id));
    const i = u.powers.findIndex(p => p.card), card = u.powers[i].power;
    await selectUnit(pl, u.id);
    await press(pl, `button[data-act=move-power][data-index="${i}"]`);
    await press(pl, `.unit[data-unit="${t.id}"] .body`);
    if (t.powers.length < 3) { await press(pl, '.modal-box button[data-modal=add]'); tally('move a power to another unit'); }
    else { await press(pl, '.modal-box button[data-modal=swap][data-index="0"]'); tally('swap powers between units'); }
    await waitFor(pl, ([id, c]) => view.game.units.find(x => x.id === id).powers.some(p => p.power === c), [t.id, card]);
  }
  if (chance(0.08)) {
    // Stack a unit on a neighbour: blocked, then overridden.
    v = await V(pl); a = analyze(v);
    const pairs = a.mine.filter(u => a.top(u)).flatMap(u => [0, 1, 2, 3, 4, 5].map(d => ({ u, c: step(u.cell, d) })))
      .filter(({ u, c }) => a.at(c).length > 0 && !a.homeOf(c, 1 - me) && !(a.homeOf(c, me) && a.at(c).some(x => x.owner !== me)));
    if (pairs.length) {
      const { u, c } = pick(pairs);
      await drag(pl, u.id, c);
      await expectToast(pl, /occupied/);
      await press(pl, 'button[data-act=retry]');
      await waitFor(pl, ([id, c]) => { const x = view.game.units.find(x => x.id === id); return x.cell.x === c.x && x.cell.y === c.y; }, [u.id, c]);
      await pl.page.waitForSelector('.badge.stack');
      tally('stack units (overridden)');
    }
  }
  if (chance(0.1)) {
    await opp.page.fill('#chat-input', `gg ${counts.chat || 0}`);
    await opp.page.press('#chat-input', 'Enter');
    await waitFor(pl, t => view.log.some(e => e.chat && e.text === t), `gg ${counts.chat || 0}`);
    tally('chat');
  }

  // End the turn, scoring if the dialog says so.
  v = await V(pl);
  const flag = v.game.flag_counts;
  const shouldScore = flag[me] > flag[1 - me];
  const before = v.game.scores[me];
  await press(pl, 'button[data-act=end-turn]');
  await pl.page.waitForSelector('.modal-box button.primary[data-modal=end]');
  const recommended = await pl.page.getAttribute('.modal-box button.primary[data-modal=end]', 'data-score');
  check(recommended === (shouldScore ? '1' : '0'), `end-turn dialog recommends ${shouldScore ? '' : 'not '}scoring (flag ${flag})`);
  await press(pl, '.modal-box button.primary[data-modal=end]');
  await waitFor(pl, me => view.game.current !== me || view.game.phase.kind === 'over', me);
  v = await V(pl);
  check(v.game.scores[me] === before + (shouldScore ? 1 : 0), 'score updated');
  tally(shouldScore ? 'end turn + score' : 'end turn');
  await noToasts(pl, 'end of turn');
  await noToasts(opp, 'end of turn');
}

// ---------------------------------------------------------------------------
// Games
// ---------------------------------------------------------------------------

async function claim(pl, seat) {
  await press(pl, `button[data-act=claim][data-seat="${seat}"]`);
  await waitFor(pl, s => view.you === s, seat);
  pl.seat = seat;
}

async function newGame(pl, o) {
  const page = pl.page;
  await press(pl, 'button[data-act=new-game]');
  await page.waitForSelector('#ng-first');
  await page.selectOption('#ng-rules', 'manual');
  await page.selectOption('#ng-first', o.first === null ? '' : String(o.first));
  await page.selectOption('#ng-free', { label: o.free || 'Walk' });
  await page.setChecked('#ng-y', !!o.y);
  await page.setChecked('#ng-flag', !!o.flag);
  await page.setChecked('#ng-skip', !!o.skip);
  await page.fill('#ng-target', String(o.target || 5));
  await page.fill('#ng-bonus', String(o.bonus ?? 1));
  await press(pl, '.modal-box button[data-modal=start]');
  await waitFor(pl, () => view.game.phase.kind === 'draft' && view.game.hand.length === 0
    && view.log.some(e => /started a new game/.test(e.text)) && view.log[view.log.length - 1].text.startsWith('Draft'));
}

async function playGame(players, starter, options, label) {
  await newGame(starter, options);
  let [v] = await synced(players);
  const g0 = v.game;
  const first = g0.current;
  check(options.first === null || first === options.first, 'first player as chosen');
  check(g0.scores[1 - first] === (options.bonus ?? 1) && g0.scores[first] === 0, 'second player starts with bonus points');
  check(g0.free_powers.length === (options.y ? 2 : 1), 'free powers');
  check(INIT.powers[g0.free_powers[0]].name === (options.free || 'Walk'), 'free power X');
  const flagUnit = g0.units.find(u => u.powers.some(p => p.power === INIT.flag));
  check(flagUnit.powers.length === (options.flag ? 2 : 1), 'extra flag power');

  // Draft.
  while (true) {
    const vs = await synced(players);
    if (vs[0].game.phase.kind !== 'draft') break;
    await draw(players.find(p => p.seat === vs[0].game.current));
  }
  [v] = await synced(players);
  check(v.game.hand_counts.every(n => n === 3), 'everyone drafted 3 cards');
  check(v.game.turn === 1 && v.game.current === first, 'first player starts turn 1');
  check(v.game.phase.kind === (options.skip ? 'main' : 'draw'), 'first draw skipped only when asked');

  // Turns.
  while (true) {
    const vs = await checkInvariants(players);
    const g = vs[0].game;
    if (g.phase.kind === 'over') break;
    check(g.turn <= MAX_TURNS, `game finished within ${MAX_TURNS} turns`);
    const cur = players.find(p => p.seat === g.current), opp = players.find(p => p.seat !== g.current);
    await playTurn(cur, opp);
  }
  [v] = await synced(players);
  const winner = v.game.phase.winner;
  check(v.game.scores[winner] >= v.game.target_score, 'winner reached the target score');
  for (const pl of players) {
    const banner = await pl.page.textContent('#topbar .phase');
    check(banner.includes(`${INIT.seat_names[winner]} wins!`), `${pl.name} shows the winner`);
    check(await pl.page.locator('button[data-act=end-turn]').count() === 0, 'no end turn button after the game');
    await pl.page.evaluate(() => window.scrollTo(0, 0));
    await pl.page.screenshot({ path: path.join(OUT, `${label}-${pl.name}.png`) });
  }
  const units = v.game.units.filter(u => u.owner !== null).length;
  console.log(`${label}: ${INIT.seat_names[winner]} won ${v.game.scores.join('-')} after ${v.game.turn} turns ` +
    `(${units} units on the board, ${v.log.length} log entries sent)`);
}

(async () => {
  const browser = await chromium.launch();
  const a = await openPlayer(browser, 'letters-desktop');
  const b = await openPlayer(browser, 'numbers-desktop');
  await claim(a, 0);
  await claim(b, 1);

  if (!process.env.PHONE_ONLY) await playGame([a, b], a, { first: null }, 'game1-default');
  if (!process.env.PHONE_ONLY) await playGame([a, b], b, { first: 1, free: 'Run', y: true, flag: true, skip: true, target: 4, bonus: 2 }, 'game2-advanced');

  // Letters switches to a phone: close the desktop tab and take over the seat.
  await a.ctx.close();
  const phone = await openPlayer(browser, 'letters-phone', true);
  await waitFor(phone, () => view.seats[0].claimed && !view.seats[0].connected);
  await claim(phone, 0);
  const overflow = await phone.page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  check(overflow <= 0, `no sideways scrolling on the phone (${overflow}px)`);
  await playGame([phone, b], phone, { first: 0 }, 'game3-phone');

  console.log('\nUI actions performed:');
  for (const [k, n] of Object.entries(counts).sort((x, y) => y[1] - x[1])) console.log(`  ${String(n).padStart(4)}  ${k}`);
  check(errors.length === 0, 'no console errors: ' + errors.join(' | '));
  console.log('\nAll games finished with no errors.');
  await browser.close();
})().catch(async e => {
  console.error(e);
  for (const pl of ALL) {
    try {
      await pl.page.screenshot({ path: path.join(OUT, `FAIL-${pl.name}.png`) });
      console.error(pl.name, 'state:', JSON.stringify(await pl.page.evaluate(() => ({ phase: view.game.phase, current: view.game.current, you: view.you, modal: document.getElementById('modal').hidden, toasts: window.__toasts, events: window.__events, top: document.getElementById('topbar').innerText }))));
    } catch (err) { /* page closed */ }
  }
  console.error('console errors:', errors);
  process.exit(1);
});
