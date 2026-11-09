'use strict';

// ---------------------------------------------------------------------------
// Constants and state
// ---------------------------------------------------------------------------

const SQ3 = Math.sqrt(3);
// Neighbour offsets in doubled-width coordinates, clockwise from east (same as the server).
const DIRS = [[2, 0], [1, 1], [-1, 1], [-2, 0], [-1, -1], [1, -1]];
const SEAT_KEYS = ['letters', 'numbers'];
const DRAFT_CARDS = 3;

let ws = null;
let connected = false;
let init = null; // static data: board, powers, rules, ...
let view = null; // the latest game state, as seen by this browser
let cellIndex = new Map();
let center = { x: 0, y: 0 };

const ui = {
  sel: null, // {unit: id} or {cell: {x, y}}
  mode: null, // {kind: 'apply-hand', card} | {kind: 'apply-free', power} | {kind: 'move-power', unit, index}
  facing: null, // facing chosen in the activation picker
  modal: null, // {render, handlers}
  retry: null, // forced version of the last rejected action
  refTab: 'rules',
};

let drag = null;
let renderPending = false;
let toastTimer = null;
let logLength = 0;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const $ = id => document.getElementById(id);
const esc = s => String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const md = s => esc(s).replace(/\*([^*]+)\*/g, '<em>$1</em>');
const possessive = name => (name.endsWith('s') ? `${name}'` : `${name}'s`);
const plural = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`;

const G = () => view.game;
const me = () => (view ? view.you : null);
const seatName = s => init.seat_names[s];
const P = i => init.powers[i];
const key = c => `${c.x},${c.y}`;
const parseKey = k => { const [x, y] = k.split(',').map(Number); return { x, y }; };
const sameCell = (a, b) => a.x === b.x && a.y === b.y;
const unitById = id => G().units.find(u => u.id === id);
const unitsAt = c => G().units.filter(u => sameCell(u.cell, c)); // bottom of the stack first
const topAt = c => { const us = unitsAt(c); return us.length ? us[us.length - 1] : null; };
const cellInfo = c => cellIndex.get(key(c));
const isFlagUnit = u => u.powers.some(p => p.power === init.flag);
const ownerKey = u => (u.owner !== null ? SEAT_KEYS[u.owner] : isFlagUnit(u) ? 'flag' : 'neutral');
const unitLabel = u => (u.owner !== null ? u.label : isFlagUnit(u) ? '⚑' : '');
const unitName = u => (u.owner !== null ? `Unit ${u.label}` : isFlagUnit(u) ? 'The flag' : 'Neutral unit');
const inHomeOf = (u, seat) => { const c = cellInfo(u.cell); return !!c && c.kind === 'home' && c.seat === seat; };

function myTurn() {
  return !!view && me() !== null && G().current === me() && G().phase.kind !== 'over';
}
function canMain() {
  return myTurn() && G().phase.kind === 'main';
}
function canTake() {
  return myTurn() && (G().phase.kind === 'draft' || G().phase.kind === 'draw');
}

function send(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}
function act(action) {
  ui.facing = null;
  send({ type: 'action', action });
}

function token() {
  // Per tab, so two tabs in one browser can be the two players.
  let t = null;
  try { t = sessionStorage.getItem('token-001'); } catch (e) { /* storage blocked */ }
  if (!t) {
    t = Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
    try { sessionStorage.setItem('token-001', t); } catch (e) { /* storage blocked */ }
  }
  return t;
}
const TOKEN = token();

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

function connect() {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  ws = new WebSocket(`${proto}://${location.host}/ws`);
  ws.onopen = () => {
    connected = true;
    send({ type: 'hello', token: TOKEN });
  };
  ws.onmessage = ev => {
    const msg = JSON.parse(ev.data);
    if (msg.type === 'init') {
      setInit(msg);
    } else if (msg.type === 'state') {
      view = msg;
      cleanUpSelection();
      render();
    } else if (msg.type === 'error') {
      showToast(msg.message, msg.retry);
    }
  };
  ws.onclose = () => {
    connected = false;
    if (view) render();
    setTimeout(connect, 1000);
  };
}

function setInit(msg) {
  init = msg;
  cellIndex = new Map(init.board.map(c => [key(c), c]));
  const xs = init.board.map(c => c.x), ys = init.board.map(c => c.y);
  center = { x: (Math.min(...xs) + Math.max(...xs)) / 2, y: (Math.min(...ys) + Math.max(...ys)) / 2 };
}

function cleanUpSelection() {
  if (ui.sel && ui.sel.unit !== undefined && !unitById(ui.sel.unit)) ui.sel = null;
  if (ui.mode) {
    const m = ui.mode;
    const gone = (m.kind === 'apply-hand' && !(G().hand || []).includes(m.card))
      || (m.kind === 'apply-free' && !G().free_powers.includes(m.power))
      || (m.kind === 'move-power' && !unitById(m.unit))
      || !canMain();
    if (gone) ui.mode = null;
  }
}

// ---------------------------------------------------------------------------
// Geometry. The board is drawn with pointy-top hexes of circumradius 1. Each
// player sees their own home at the bottom, so Letters (top home) sees the
// board rotated by 180 degrees.
// ---------------------------------------------------------------------------

const flipped = () => me() === 0;

function toScreen(c) {
  const px = (c.x - center.x) * SQ3 / 2, py = (c.y - center.y) * 1.5;
  return flipped() ? [-px, -py] : [px, py];
}

function dirVec(f) {
  const [dx, dy] = DIRS[((f % 6) + 6) % 6];
  const vx = dx / 2, vy = dy * 1.5 / SQ3;
  return flipped() ? [-vx, -vy] : [vx, vy];
}

function hexPoints(px, py, r) {
  const pts = [];
  for (let i = 0; i < 6; i++) {
    const a = Math.PI / 180 * (60 * i - 90);
    pts.push(`${(px + r * Math.cos(a)).toFixed(3)},${(py + r * Math.sin(a)).toFixed(3)}`);
  }
  return pts.join(' ');
}

const pt = ([x, y]) => `${x.toFixed(3)},${y.toFixed(3)}`;

function arrowPoints(cx, cy, vx, vy, tipDist, baseDist, halfWidth) {
  const tip = [cx + vx * tipDist, cy + vy * tipDist];
  const b1 = [cx + vx * baseDist - vy * halfWidth, cy + vy * baseDist + vx * halfWidth];
  const b2 = [cx + vx * baseDist + vy * halfWidth, cy + vy * baseDist - vx * halfWidth];
  return `${pt(tip)} ${pt(b1)} ${pt(b2)}`;
}

// A line with an arrowhead, unambiguous in every direction (unlike a lone triangle).
function arrowPath(cx, cy, vx, vy) {
  const tip = [cx + vx * 0.22, cy + vy * 0.22], tail = [cx - vx * 0.22, cy - vy * 0.22];
  const back = [cx + vx * 0.04, cy + vy * 0.04], px = -vy * 0.16, py = vx * 0.16;
  return `M ${pt(tail)} L ${pt(tip)} M ${pt([back[0] + px, back[1] + py])} L ${pt(tip)} L ${pt([back[0] - px, back[1] - py])}`;
}

function svgPoint(e) {
  const svg = $('board');
  const p = svg.createSVGPoint();
  p.x = e.clientX;
  p.y = e.clientY;
  return p.matrixTransform(svg.getScreenCTM().inverse());
}

function cellAt(p) {
  let best = null, bestDist = 0.95;
  for (const c of init.board) {
    const [px, py] = toScreen(c);
    const d = Math.hypot(px - p.x, py - p.y);
    if (d < bestDist) { best = c; bestDist = d; }
  }
  return best ? { x: best.x, y: best.y } : null;
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function render() {
  if (!init || !view) return;
  if (drag && drag.moved) { renderPending = true; return; }
  renderPending = false;
  renderTop();
  renderBoard();
  renderRosters();
  renderHand();
  renderMarket();
  renderInspector();
  renderLog();
}

function phaseText() {
  const g = G(), cur = seatName(g.current), mine = myTurn();
  if (ui.mode) {
    const m = ui.mode;
    if (m.kind === 'apply-hand' || m.kind === 'apply-free') {
      const name = P(m.kind === 'apply-hand' ? m.card : m.power).name;
      return { text: `Applying ${name}: click one of your units in a home space (Esc to cancel)`, cls: 'mode' };
    }
    if (m.kind === 'move-power') {
      const u = unitById(m.unit);
      return { text: `Moving ${P(u.powers[m.index].power).name}: click the unit to move it to (Esc to cancel)`, cls: 'mode' };
    }
  }
  switch (g.phase.kind) {
    case 'draft':
      return mine
        ? { text: `Draft: take a card from the faceup row or the deck (${(g.hand || []).length}/${DRAFT_CARDS})`, cls: 'mine' }
        : { text: `Draft: waiting for ${cur} to take a card`, cls: '' };
    case 'draw':
      return mine
        ? { text: `Draw ${plural(g.phase.remaining, 'card')}: take from the faceup row or the deck`, cls: 'mine' }
        : { text: `${cur} is drawing`, cls: '' };
    case 'main':
      return mine
        ? { text: `Your turn (turn ${g.turn})`, cls: 'mine' }
        : { text: `${possessive(cur)} turn (turn ${g.turn})`, cls: '' };
    case 'over':
      return { text: `${seatName(g.phase.winner)} wins!`, cls: g.phase.winner === me() ? 'mine' : '' };
  }
  return { text: '', cls: '' };
}

function renderTop() {
  const phase = phaseText();
  let buttons = '';
  if (view.can_undo) buttons += '<button data-act="undo" title="Undo your last action this turn">Undo</button>';
  if (canMain()) buttons += '<button class="primary" data-act="end-turn">End turn</button>';
  if (me() !== null) buttons += '<button data-act="new-game">New game</button>';
  buttons += '<button data-act="reference">Rules &amp; cards</button>';
  $('topbar').innerHTML = `
    <div class="title">001</div>
    <div class="phase ${phase.cls}">${esc(phase.text)}</div>
    <div class="buttons">${buttons}</div>
    <div class="conn ${connected ? 'on' : ''}" title="${connected ? 'Connected' : 'Reconnecting…'}"></div>`;
}

function isTarget(u) {
  const m = ui.mode;
  if (!m) return false;
  if (m.kind === 'apply-hand' || m.kind === 'apply-free') return u.owner === me() && inHomeOf(u, me());
  if (m.kind === 'move-power') return u.id !== m.unit;
  return false;
}

function renderBoard() {
  const svg = $('board');
  const pts = init.board.map(toScreen);
  const xs = pts.map(p => p[0]), ys = pts.map(p => p[1]);
  const minX = Math.min(...xs) - 1, maxX = Math.max(...xs) + 1;
  const minY = Math.min(...ys) - 1.05, maxY = Math.max(...ys) + 1.05;
  svg.setAttribute('viewBox', `${minX} ${minY} ${maxX - minX} ${maxY - minY}`);

  const spawnable = canMain() && !G().spawned_this_turn && !ui.mode;
  let html = '';
  for (const c of init.board) {
    const [px, py] = toScreen(c);
    const cls = ['cell', c.kind];
    if (c.kind === 'home') cls.push(`home-${SEAT_KEYS[c.seat]}`);
    if (ui.sel && ui.sel.cell && sameCell(ui.sel.cell, c)) cls.push('selected');
    if (spawnable && c.kind === 'home' && c.seat === me() && !topAt(c)) cls.push('spawnable');
    html += `<g class="${cls.join(' ')}" data-cell="${key(c)}"><polygon points="${hexPoints(px, py, 0.97)}"/>`;
    if (c.kind === 'flag') html += `<text class="cell-mark" x="${px.toFixed(3)}" y="${(py + 0.8).toFixed(3)}">F</text>`;
    html += '</g>';
  }

  const stacks = new Map();
  for (const u of G().units) {
    const k = key(u.cell);
    if (!stacks.has(k)) stacks.set(k, []);
    stacks.get(k).push(u);
  }
  for (const us of stacks.values()) us.forEach((u, i) => { html += unitSvg(u, i, us.length); });
  svg.innerHTML = html;

  // Neutral units with powers other than the Flag are easy to miss, so list them.
  const neutrals = G().units.filter(u => u.owner === null && u.powers.some(p => p.power !== init.flag));
  $('neutral').innerHTML = neutrals.map(u =>
    `<div><b>${esc(unitName(u))}:</b> ${u.powers.map(powerChip).join('')}</div>`).join('');
}

function unitSvg(u, i, n) {
  let [px, py] = toScreen(u.cell);
  const off = (n - 1 - i) * 0.16; // lower units in a stack peek out up and to the left
  px -= off;
  py -= off;
  const [vx, vy] = dirVec(u.facing);
  const cls = ['unit', ownerKey(u)];
  if (ui.sel && ui.sel.unit === u.id) cls.push('selected');
  if (u.activations >= u.max_activations) cls.push('spent');
  if (isTarget(u)) cls.push('target');
  if (i < n - 1) cls.push('buried');
  const label = unitLabel(u);
  let s = `<g class="${cls.join(' ')}" data-unit="${u.id}" transform="translate(${px.toFixed(3)},${py.toFixed(3)})">`;
  s += `<title>${esc(unitTooltip(u))}</title>`;
  s += `<polygon class="pointer" points="${arrowPoints(0, 0, vx, vy, 0.88, 0.45, 0.26)}"/>`;
  s += '<circle class="body" r="0.6"/>';
  s += `<text class="label" y="-0.06">${esc(label)}</text>`;
  const m = u.powers.length;
  u.powers.forEach((p, j) => {
    const x = (j - (m - 1) / 2) * 0.2;
    s += `<circle class="pip ${P(p.power).kind}${p.card ? '' : ' free'}" cx="${x.toFixed(3)}" cy="0.34" r="0.07"/>`;
  });
  if (u.tokens > 0) s += `<g class="badge"><circle cx="0.47" cy="-0.43" r="0.17"/><text x="0.47" y="-0.43">${u.tokens}</text></g>`;
  if (n > 1 && i === n - 1) s += `<g class="badge stack"><circle cx="-0.47" cy="-0.43" r="0.17"/><text x="-0.47" y="-0.43">${n}</text></g>`;
  return s + '</g>';
}

function unitTooltip(u) {
  const powers = u.powers.map(p => P(p.power).name).join(', ') || 'no powers';
  let t = `${unitName(u)}: ${powers}`;
  if (u.tokens) t += ` · ${plural(u.tokens, 'token')}`;
  return t;
}

function powerChip(p) {
  const pw = P(p.power);
  return `<span class="pchip ${pw.kind}${p.card ? '' : ' free'}" title="${esc(`${pw.name} (${pw.kind}${p.card ? '' : ', free'}): ${pw.text}`)}">${esc(pw.name)}</span>`;
}

function renderRosters() {
  const bottom = me() === null ? 1 : me();
  $('roster-top').innerHTML = rosterHtml(1 - bottom);
  $('roster-bottom').innerHTML = rosterHtml(bottom);
}

function rosterHtml(seat) {
  const g = G(), sv = view.seats[seat];
  const turn = g.current === seat && g.phase.kind !== 'over';
  let status = '';
  if (me() === seat) status = '<span class="tag you">you</span><button class="mini" data-act="leave">Leave seat</button>';
  else if (sv.claimed && sv.connected) status = '<span class="tag online">online</span>';
  else if (sv.claimed) status = '<span class="tag offline">offline</span>';
  else status = '<span class="tag">empty seat</span>';
  if (me() === null && !sv.connected) {
    status += ` <button class="mini primary" data-act="claim" data-seat="${seat}">Play as ${seatName(seat)}</button>`;
  }
  const adjust = canMain()
    ? ` <button class="mini" data-act="score" data-seat="${seat}" data-delta="-1" title="Manual score adjustment">−</button><button class="mini" data-act="score" data-seat="${seat}" data-delta="1" title="Manual score adjustment">+</button>`
    : '';
  const first = g.first_player === seat ? '<span class="tag" title="Goes first">1st</span>' : '';
  let html = `<div class="roster-head ${SEAT_KEYS[seat]}${turn ? ' turn' : ''}">
    <span class="seat-name">${seatName(seat)}</span>${first}${status}
    <span class="spacer"></span>
    <span class="muted">hand: ${g.hand_counts[seat]}</span>
    <span class="muted">flag: ${g.flag_counts[seat]}</span>
    <span class="score">${g.scores[seat]} / ${g.target_score} pts</span>${adjust}
  </div><div class="slots">`;
  for (const label of init.labels[seat]) {
    const u = g.units.find(u => u.owner === seat && u.label === label);
    if (!u) {
      html += `<div class="slot empty"><span class="slot-label">${esc(label)}</span></div>`;
      continue;
    }
    const sel = ui.sel && ui.sel.unit === u.id ? ' selected' : '';
    const meta = [];
    if (u.tokens) meta.push(plural(u.tokens, 'token'));
    if (u.activations >= u.max_activations) meta.push('activated');
    html += `<div class="slot filled ${SEAT_KEYS[seat]}${sel}" data-act="select-unit" data-unit="${u.id}">
      <span class="slot-label">${esc(label)}</span>${u.powers.map(powerChip).join('') || '<span class="muted">no powers</span>'}
      ${meta.length ? `<div class="slot-meta">${meta.join(' · ')}</div>` : ''}
    </div>`;
  }
  return html + '</div>';
}

function cardHtml(p, opts = {}) {
  const pw = P(p);
  const attrs = Object.entries(opts.data || {}).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
  return `<div class="card ${pw.kind} ${opts.cls || ''}"${attrs}>
    ${opts.badge ? `<span class="next">${esc(opts.badge)}</span>` : ''}
    <div class="cname">${esc(pw.name)}</div>
    <div class="ckind">${esc(pw.kind)}${opts.free ? ' · free' : ''}</div>
    <div class="ctext">${md(pw.text)}</div>
    ${opts.buttons ? `<div class="card-btns">${opts.buttons}</div>` : ''}
  </div>`;
}

function renderHand() {
  if (me() === null) {
    $('hand').innerHTML = '<div class="muted">You are watching. Take a seat above to play.</div>';
    return;
  }
  const cards = G().hand || [];
  const main = canMain();
  const items = cards.map(c => {
    const selected = ui.mode && ui.mode.kind === 'apply-hand' && ui.mode.card === c;
    return cardHtml(c, {
      cls: (main ? 'clickable' : '') + (selected ? ' selected' : ''),
      data: { act: 'hand', card: c },
      buttons: selected ? `<button class="mini" data-act="discard-card" data-card="${c}">Discard</button>` : '',
    });
  }).join('');
  $('hand').innerHTML = `<div class="strip-title">Your hand (${cards.length})</div>
    <div class="cards">${items || '<span class="muted">No cards.</span>'}</div>`;
}

function renderMarket() {
  const g = G();
  const take = canTake(), main = canMain();
  const faceup = g.faceup.map((c, i) => cardHtml(c, {
    cls: take ? 'clickable takeable' : main ? 'clickable' : '',
    data: { act: 'faceup', slot: i },
    badge: i === 0 ? 'discarded next turn' : '',
  })).join('');
  const deck = `<div class="pile ${take ? 'takeable' : ''}" data-act="deck" title="Draw the top card"><div class="pile-n">${g.deck_count}</div>Deck</div>`;
  const discard = `<div class="pile" data-act="discard-pile" title="Look through the discard pile"><div class="pile-n">${g.discard.length}</div>Discard</div>`;
  const free = g.free_powers.map(p => {
    const selected = ui.mode && ui.mode.kind === 'apply-free' && ui.mode.power === p;
    return cardHtml(p, { cls: `free${main ? ' clickable' : ''}${selected ? ' selected' : ''}`, data: { act: 'free', power: p }, free: true });
  }).join('');
  $('market').innerHTML = `<div class="strip-row">
    <div><div class="strip-title">Faceup</div><div class="cards">${faceup || '<span class="muted">Empty.</span>'}</div></div>
    <div><div class="strip-title">Piles</div><div class="cards">${deck}${discard}</div></div>
    <div><div class="strip-title">Free powers</div><div class="cards">${free}</div></div>
  </div>`;
}

function renderInspector() {
  const el = $('inspector');
  const s = ui.sel;
  if (s && s.unit !== undefined) {
    const u = unitById(s.unit);
    if (u) { el.innerHTML = unitPanel(u); return; }
  }
  if (s && s.cell) { el.innerHTML = cellPanel(s.cell); return; }
  el.innerHTML = helpPanel();
}

function helpPanel() {
  return `<h3>How to play</h3>
  <ul>
    <li>Each player opens this page in their own browser and clicks <b>Play as…</b>.</li>
    <li>Draft and draw by clicking a faceup card or the deck.</li>
    <li>Click one of your empty home spaces to spawn a unit.</li>
    <li>Click a card in your hand (or a free power), then one of your units in a home space, to apply it.</li>
    <li>Click a unit to activate it: pick a facing, then use one of its active powers (or just rotate).</li>
    <li>Power effects are carried out by hand: <b>drag units</b> to move them, and use the manual tools in this panel to kill units, move powers, add tokens and so on.</li>
    <li>When a rule check blocks something a power allows, choose <b>Do it anyway</b>. <b>Undo</b> works until you draw or end your turn.</li>
    <li><b>End turn</b> counts units next to the flag and offers to score.</li>
  </ul>`;
}

function unitPanel(u) {
  const main = canMain();
  const mine = u.owner === me();
  const canActivate = main && (mine || (u.owner === null && isFlagUnit(u)));
  const where = cellInfo(u.cell);
  const whereText = where.kind === 'home' ? `${seatName(where.seat)} home` : where.kind === 'flag' ? 'flag space' : 'on the board';
  let h = `<div class="insp-head">
    <div class="token-chip ${ownerKey(u)}">${esc(unitLabel(u) || '·')}</div>
    <div>
      <div class="insp-title">${esc(unitName(u))}${u.owner !== null ? ` <span class="muted">(${seatName(u.owner)})</span>` : ''}</div>
      <div class="muted small">${whereText} · activated ${u.activations}/${u.max_activations} · ${plural(u.tokens, 'token')}</div>
    </div>
  </div>`;

  const stack = unitsAt(u.cell);
  if (stack.length > 1) {
    h += `<div class="stack-nav small">Stack, bottom to top: ${stack.map(s =>
      `<button class="mini${s.id === u.id ? ' on' : ''}" data-act="select-unit" data-unit="${s.id}">${esc(unitLabel(s) || '·')}</button>`).join(' ')}</div>`;
  }

  h += '<h4>Powers</h4>';
  if (!u.powers.length) h += '<div class="muted">No powers.</div>';
  u.powers.forEach((p, i) => {
    const pw = P(p.power);
    h += `<div class="power ${pw.kind}${p.card ? '' : ' free'}">
      <div class="power-head"><b>${esc(pw.name)}</b> <span class="kind">${esc(pw.kind)}${p.card ? '' : ' · free'}</span></div>
      <div class="power-text">${md(pw.text)}</div>`;
    if (main) {
      h += '<div class="power-btns">';
      if (canActivate && pw.kind === 'active') h += `<button class="primary mini" data-act="use" data-index="${i}">Activate &amp; use</button>`;
      h += `<button class="mini" data-act="remove-power" data-index="${i}">Discard</button>`;
      if (p.card) h += `<button class="mini" data-act="power-to-hand" data-index="${i}" title="Return this card to your hand">To my hand</button>`;
      h += `<button class="mini" data-act="move-power" data-index="${i}" title="Move or swap this power onto another unit">Move / swap…</button>`;
      h += '</div>';
    }
    h += '</div>';
  });

  if (canActivate) {
    h += `<div class="section"><h4>Activate</h4><div class="activate-row">${facingPicker(u)}
      <div><div class="hint">Choose a facing, then press <b>Activate &amp; use</b> on an active power above, or:</div>
      <div class="btn-row"><button data-act="activate">Activate (rotate only)</button></div></div></div></div>`;
  }

  if (main) {
    const owners = [me(), null, 1 - me()].filter(o => o !== u.owner);
    const ownerLabel = o => (o === null ? 'neutral' : o === me() ? 'mine' : possessive(seatName(o)));
    h += `<div class="section"><h4>Manual adjustments</h4>
      <div class="btn-row">
        <button class="mini" data-act="rotate" data-delta="-1" title="Rotate without activating">⟲ Rotate</button>
        <button class="mini" data-act="rotate" data-delta="1" title="Rotate without activating">Rotate ⟳</button>
        <button class="mini" data-act="tokens" data-delta="-1">− Token</button>
        <button class="mini" data-act="tokens" data-delta="1">+ Token</button>
        <button class="mini danger" data-act="kill">Kill</button>
      </div>
      <div class="btn-row">${owners.map(o =>
        `<button class="mini" data-act="owner" data-owner="${o === null ? 'null' : o}">Make ${ownerLabel(o)}</button>`).join('')}</div>
      <div class="hint">Drag the unit on the board to move it.</div>
    </div>`;
  }
  return h;
}

function facingPicker(u) {
  const chosen = ui.facing ?? u.facing;
  let s = '<svg class="facing" viewBox="-1.65 -1.65 3.3 3.3">';
  s += `<circle class="fp-body ${ownerKey(u)}" r="0.55"/><text class="fp-label">${esc(unitLabel(u))}</text>`;
  for (let f = 0; f < 6; f++) {
    const [vx, vy] = dirVec(f);
    const cx = vx * 1.18, cy = vy * 1.18;
    const cls = f === chosen ? 'chosen' : f === u.facing ? 'current' : '';
    s += `<g class="fp-dir ${cls}" data-act="facing" data-facing="${f}">
      <title>${f === u.facing ? 'Current facing' : 'Face this way'}</title>
      <circle cx="${cx.toFixed(3)}" cy="${cy.toFixed(3)}" r="0.4"/>
      <path d="${arrowPath(cx, cy, vx, vy)}"/></g>`;
  }
  return s + '</svg>';
}

function cellPanel(c) {
  const info = cellInfo(c);
  const title = info.kind === 'home' ? `${seatName(info.seat)} home space` : info.kind === 'flag' ? 'Flag space (F)' : 'Empty space';
  let h = `<h3>${title}</h3>`;
  if (!canMain()) return h + '<div class="muted">Nothing here.</div>';
  if (info.kind === 'home' && info.seat === me()) {
    h += `<div class="btn-row"><button class="primary" data-act="spawn">Spawn a unit here</button></div>`;
    if (G().spawned_this_turn) h += '<div class="hint">You have already spawned a unit this turn.</div>';
  }
  h += `<div class="section"><h4>Manual</h4>
    <div class="btn-row">
      <button class="mini" data-act="create" data-owner="${me()}">Create my unit</button>
      <button class="mini" data-act="create" data-owner="null">Create neutral unit</button>
    </div>
    <div class="hint">For powers like <em>Split</em> and <em>Trail</em>.</div></div>`;
  return h;
}

function renderLog() {
  const el = $('log');
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  el.innerHTML = view.log.map(e => {
    const seat = e.seat !== null ? SEAT_KEYS[e.seat] : '';
    const who = e.chat ? `<b>${e.seat !== null ? seatName(e.seat) : 'Spectator'}:</b> ` : '';
    return `<div class="entry ${seat}${e.chat ? ' chat' : ''}">${who}${md(e.text)}</div>`;
  }).join('');
  if (atBottom || view.log.length !== logLength) el.scrollTop = el.scrollHeight;
  logLength = view.log.length;
}

// ---------------------------------------------------------------------------
// Modals and toasts
// ---------------------------------------------------------------------------

function openModal(html, handlers = {}, wide = false) {
  ui.modal = { handlers };
  const el = $('modal');
  el.innerHTML = `<div class="modal-box${wide ? ' wide' : ''}" role="dialog">${html}</div>`;
  el.hidden = false;
}

function closeModal() {
  ui.modal = null;
  $('modal').hidden = true;
  $('modal').innerHTML = '';
}

function showToast(message, retry) {
  ui.retry = retry || null;
  const el = $('toast');
  el.innerHTML = `<span>${esc(message)}</span>${retry ? '<button class="mini" data-act="retry">Do it anyway</button>' : ''}<button class="mini" data-act="dismiss">✕</button>`;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(hideToast, retry ? 10000 : 5000);
}

function hideToast() {
  $('toast').hidden = true;
  ui.retry = null;
}

function choosePower(u, title, intro, onChoose) {
  const buttons = u.powers.map((p, i) =>
    `<button data-modal="choose" data-index="${i}"><b>${esc(P(p.power).name)}</b> — <span class="muted">${md(P(p.power).text)}</span></button>`).join('');
  openModal(`<h2>${esc(title)}</h2><p>${intro}</p><div class="choice-list">${buttons}</div>
    <div class="modal-actions"><button data-modal="close">Cancel</button></div>`,
  { choose: d => { closeModal(); onChoose(+d.index); } });
}

function applyToUnit(m, u) {
  const build = replace => (m.kind === 'apply-hand'
    ? { type: 'apply_hand', card: m.card, unit: u.id, replace }
    : { type: 'apply_free', power: m.power, unit: u.id, replace });
  const name = P(m.kind === 'apply-hand' ? m.card : m.power).name;
  if (u.powers.length >= init.max_powers) {
    choosePower(u, `Replace a power on ${unitName(u)}`,
      `${esc(unitName(u))} already has ${init.max_powers} powers. Which one should ${esc(name)} replace? (Replaced cards are discarded.)`,
      i => act(build(i)));
  } else {
    act(build(null));
  }
}

function openMovePower(fromId, index, toId) {
  const from = unitById(fromId), to = unitById(toId);
  const p = from.powers[index];
  const name = P(p.power).name;
  let body = '<div class="choice-list">';
  if (to.powers.length < init.max_powers) body += `<button class="primary" data-modal="add">Move ${esc(name)} to ${esc(unitName(to))}</button>`;
  to.powers.forEach((q, i) => {
    body += `<button data-modal="replace" data-index="${i}">Replace ${esc(unitName(to))}'s ${esc(P(q.power).name)} (discarded)</button>`;
    body += `<button data-modal="swap" data-index="${i}">Swap with ${esc(unitName(to))}'s ${esc(P(q.power).name)}</button>`;
  });
  body += '</div>';
  const go = (replace, swap) => { closeModal(); act({ type: 'move_power', unit: fromId, index, to_unit: toId, replace, swap }); };
  openModal(`<h2>Move ${esc(name)}</h2><p>From ${esc(unitName(from))} to ${esc(unitName(to))}.</p>${body}
    <div class="modal-actions"><button data-modal="close">Cancel</button></div>`,
  { add: () => go(null, false), replace: d => go(+d.index, false), swap: d => go(+d.index, true) });
}

function openEndTurn() {
  const g = G(), mine = g.flag_counts[me()], theirs = g.flag_counts[1 - me()];
  const scores = mine > theirs;
  const reminders = g.units
    .filter(u => u.owner === me())
    .flatMap(u => u.powers.filter(p => /end of your turn/i.test(P(p.power).text)).map(p => `${P(p.power).name} (${u.label})`));
  openModal(`<h2>End your turn</h2>
    ${reminders.length ? `<p>Remember to resolve your end-of-turn powers first: <b>${esc(reminders.join(', '))}</b>.</p>` : ''}
    <p>Units next to the flag (counting <em>Strong</em>): <b>you ${mine}</b>, <b>${seatName(1 - me())} ${theirs}</b>.
    ${scores ? 'You have more, so you score a point.' : 'You don\'t have more, so you don\'t score.'}</p>
    <p class="muted small">If other powers change the count (<em>Weak</em>, <em>Friendly</em>, <em>Bridge</em>, …), pick the other option.</p>
    <div class="modal-actions">
      <button data-modal="close">Cancel</button>
      <button class="${scores ? '' : 'primary'}" data-modal="end" data-score="0">End turn without scoring</button>
      <button class="${scores ? 'primary' : ''}" data-modal="end" data-score="1">Score 1 point &amp; end turn</button>
    </div>`,
  { end: d => { closeModal(); act({ type: 'end_turn', score: d.score === '1' }); } });
}

function openNewGame() {
  const opts = G().options;
  const powerOptions = init.powers
    .map((p, i) => ({ p, i }))
    .filter(({ i }) => i !== init.flag)
    .sort((a, b) => a.p.name.localeCompare(b.p.name))
    .map(({ p, i }) => `<option value="${i}"${i === (opts.free_power ?? init.walk) ? ' selected' : ''}>${esc(p.name)}</option>`)
    .join('');
  openModal(`<h2>New game</h2>
    <p class="muted">This ends the current game for both players.</p>
    <div class="form-row"><label for="ng-first">First player</label>
      <select id="ng-first"><option value="">Random</option><option value="0">${seatName(0)}</option><option value="1">${seatName(1)}</option></select></div>
    <div class="form-row"><label for="ng-free">Free power (X)</label><select id="ng-free">${powerOptions}</select></div>
    <div class="form-row"><label><input type="checkbox" id="ng-y"${opts.free_power_y ? ' checked' : ''}> Second free power (Y) drawn from the deck</label></div>
    <div class="form-row"><label><input type="checkbox" id="ng-flag"${opts.flag_power ? ' checked' : ''}> Add a power from the deck to the flag</label></div>
    <div class="form-row"><label><input type="checkbox" id="ng-skip"${opts.skip_first_draw ? ' checked' : ''}> First player doesn't draw on their first turn</label></div>
    <div class="form-row"><label for="ng-target">Points to win</label><input type="number" id="ng-target" min="1" max="50" value="${opts.target_score}"></div>
    <div class="form-row"><label for="ng-bonus">Second player's starting points</label><input type="number" id="ng-bonus" min="0" max="50" value="${opts.second_player_points}"></div>
    <div class="modal-actions"><button data-modal="close">Cancel</button><button class="primary" data-modal="start">Start new game</button></div>`,
  {
    start: () => {
      const first = $('ng-first').value;
      const options = {
        first_player: first === '' ? null : +first,
        free_power: +$('ng-free').value,
        free_power_y: $('ng-y').checked,
        flag_power: $('ng-flag').checked,
        skip_first_draw: $('ng-skip').checked,
        target_score: Math.max(1, +$('ng-target').value || 5),
        second_player_points: Math.max(0, +$('ng-bonus').value || 0),
      };
      closeModal();
      ui.sel = null;
      ui.mode = null;
      send({ type: 'new_game', options });
    },
  });
}

function openReference(tab) {
  ui.refTab = tab || ui.refTab;
  const tabs = `<div class="tabs">
    <button class="${ui.refTab === 'rules' ? 'on' : ''}" data-modal="tab" data-tab="rules">Rules</button>
    <button class="${ui.refTab === 'powers' ? 'on' : ''}" data-modal="tab" data-tab="powers">All powers (${init.powers.length})</button>
  </div>`;
  const body = ui.refTab === 'rules'
    ? `<div class="rules-text">${md(init.rules)}</div>`
    : `<div class="power-list"><div class="cards">${init.powers.map((_, i) => cardHtml(i)).join('')}</div></div>`;
  openModal(`${tabs}${body}<div class="modal-actions"><button data-modal="close">Close</button></div>`,
    { tab: d => openReference(d.tab) }, true);
}

function openDiscardPile() {
  const cards = G().discard.slice().reverse();
  openModal(`<h2>Discard pile (${cards.length})</h2><p class="muted small">Most recent first.</p>
    <div class="power-list"><div class="cards">${cards.map(c => cardHtml(c)).join('') || '<span class="muted">Empty.</span>'}</div></div>
    <div class="modal-actions"><button data-modal="close">Close</button></div>`, {}, true);
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

function select(sel) {
  ui.sel = sel;
  ui.facing = null;
  render();
}

function selectedUnit() {
  return ui.sel && ui.sel.unit !== undefined ? unitById(ui.sel.unit) : null;
}

function boardClick(target) {
  const ug = target.closest('[data-unit]');
  const cg = target.closest('[data-cell]');
  let unit = ug ? unitById(+ug.dataset.unit) : null;
  const cell = unit ? unit.cell : cg ? parseKey(cg.dataset.cell) : null;
  if (!unit && cell) unit = topAt(cell);
  if (ui.mode) {
    if (unit) targetUnit(unit);
    else { ui.mode = null; render(); }
    return;
  }
  if (unit) select({ unit: unit.id });
  else if (cell) select({ cell });
  else select(null);
}

function targetUnit(u) {
  const m = ui.mode;
  ui.mode = null;
  if (m.kind === 'apply-hand' || m.kind === 'apply-free') {
    applyToUnit(m, u);
    ui.sel = { unit: u.id };
  } else if (m.kind === 'move-power') {
    if (u.id !== m.unit) openMovePower(m.unit, m.index, u.id);
  }
  render();
}

function setupBoard() {
  const svg = $('board');
  svg.addEventListener('pointerdown', e => {
    if (e.button !== 0 || !view) return;
    const ug = e.target.closest('[data-unit]');
    drag = { target: e.target, id: ug ? +ug.dataset.unit : null, g: ug, x0: e.clientX, y0: e.clientY, moved: false, pointer: e.pointerId };
  });
  svg.addEventListener('pointermove', e => {
    if (!drag || drag.id === null || !canMain() || ui.mode) return;
    if (!drag.moved) {
      if (Math.hypot(e.clientX - drag.x0, e.clientY - drag.y0) < 6) return;
      drag.moved = true;
      svg.setPointerCapture(drag.pointer);
      drag.g.classList.add('dragging');
      svg.appendChild(drag.g);
    }
    const p = svgPoint(e);
    drag.g.setAttribute('transform', `translate(${p.x.toFixed(3)},${p.y.toFixed(3)})`);
    const c = cellAt(p);
    svg.querySelectorAll('.cell.drop').forEach(n => n.classList.remove('drop'));
    if (c) { const n = svg.querySelector(`[data-cell="${key(c)}"]`); if (n) n.classList.add('drop'); }
  });
  svg.addEventListener('pointerup', e => {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (d.moved) {
      const c = cellAt(svgPoint(e));
      const u = unitById(d.id);
      if (c && u && !sameCell(c, u.cell)) {
        act({ type: 'move', unit: d.id, cell: c });
        ui.sel = { unit: d.id };
      }
      render();
    } else {
      boardClick(d.target);
    }
    if (renderPending) render();
  });
  svg.addEventListener('pointercancel', () => {
    drag = null;
    render();
  });
}

function handleAct(a, d) {
  const u = selectedUnit();
  switch (a) {
    case 'claim': send({ type: 'claim', seat: +d.seat }); break;
    case 'leave': send({ type: 'leave' }); break;
    case 'undo': send({ type: 'undo' }); break;
    case 'end-turn': openEndTurn(); break;
    case 'new-game': openNewGame(); break;
    case 'reference': openReference(); break;
    case 'retry': if (ui.retry) send({ type: 'action', action: ui.retry }); hideToast(); break;
    case 'dismiss': hideToast(); break;
    case 'score': act({ type: 'adjust_score', seat: +d.seat, delta: +d.delta }); break;
    case 'select-unit': select({ unit: +d.unit }); break;

    case 'hand':
      if (!canMain()) break;
      ui.mode = ui.mode && ui.mode.kind === 'apply-hand' && ui.mode.card === +d.card ? null : { kind: 'apply-hand', card: +d.card };
      render();
      break;
    case 'free':
      if (!canMain()) break;
      ui.mode = ui.mode && ui.mode.kind === 'apply-free' && ui.mode.power === +d.power ? null : { kind: 'apply-free', power: +d.power };
      render();
      break;
    case 'discard-card':
      ui.mode = null;
      act({ type: 'discard_card', card: +d.card });
      break;
    case 'faceup':
      if (canTake() || (canMain() && confirm(`Take ${P(G().faceup[+d.slot]).name} from the faceup row as a manual draw (eg for Mine)? This can't be undone.`))) {
        act({ type: 'take_faceup', slot: +d.slot });
      }
      break;
    case 'deck':
      if (canTake() || (canMain() && confirm('Draw the top card of the deck as a manual draw (eg for Mine)? This can\'t be undone.'))) {
        act({ type: 'take_deck' });
      }
      break;
    case 'discard-pile': openDiscardPile(); break;

    case 'facing': ui.facing = +d.facing; render(); break;
    case 'use': if (u) act({ type: 'activate', unit: u.id, facing: ui.facing ?? u.facing, power: +d.index }); break;
    case 'activate': if (u) act({ type: 'activate', unit: u.id, facing: ui.facing ?? u.facing, power: null }); break;
    case 'remove-power': if (u) act({ type: 'remove_power', unit: u.id, index: +d.index, to_hand: false }); break;
    case 'power-to-hand': if (u) act({ type: 'remove_power', unit: u.id, index: +d.index, to_hand: true }); break;
    case 'move-power': if (u) { ui.mode = { kind: 'move-power', unit: u.id, index: +d.index }; render(); } break;
    case 'rotate': if (u) act({ type: 'rotate', unit: u.id, facing: (u.facing + +d.delta + 6) % 6 }); break;
    case 'tokens': if (u) act({ type: 'tokens', unit: u.id, delta: +d.delta }); break;
    case 'kill': if (u) act({ type: 'kill', unit: u.id }); break;
    case 'owner': if (u) act({ type: 'set_owner', unit: u.id, owner: d.owner === 'null' ? null : +d.owner }); break;
    case 'spawn': if (ui.sel && ui.sel.cell) act({ type: 'spawn', cell: ui.sel.cell }); break;
    case 'create': if (ui.sel && ui.sel.cell) act({ type: 'create_unit', cell: ui.sel.cell, owner: d.owner === 'null' ? null : +d.owner }); break;
  }
}

document.addEventListener('click', e => {
  const m = e.target.closest('[data-modal]');
  if (m && ui.modal) {
    if (m.dataset.modal === 'close') closeModal();
    else if (ui.modal.handlers[m.dataset.modal]) ui.modal.handlers[m.dataset.modal](m.dataset);
    return;
  }
  if (e.target === $('modal')) { closeModal(); return; }
  const t = e.target.closest('[data-act]');
  if (!t || t.closest('#board') || !view) return;
  handleAct(t.dataset.act, t.dataset);
});

document.addEventListener('keydown', e => {
  if (e.key !== 'Escape') return;
  if (ui.modal) closeModal();
  else if (ui.mode) { ui.mode = null; render(); }
  else if (ui.sel) select(null);
});

$('chat').addEventListener('submit', e => {
  e.preventDefault();
  const input = $('chat-input');
  if (input.value.trim()) send({ type: 'chat', text: input.value });
  input.value = '';
});

setupBoard();
connect();
