// Runs the game and the bot in the browser: the Rust engine compiled to
// WebAssembly (game.wasm), driven the way the web server drives a session.
// local-socket.js passes the page's messages here and ours back, so app.js
// works exactly as it does against the server.

const SIMULATIONS = 400, SAMPLES = 4; // the bot's search, as in the web server
const DELAY_MS = 1000;                // each bot action takes at least this long, so you can follow it

let wasm = null;
let seated = false, botRunning = false;
const waiting = []; // messages from the page that arrived before the engine was ready
const encoder = new TextEncoder(), decoder = new TextDecoder();

// Calls an export that answers text: with a string argument (passed in as
// UTF-8 through the module's memory), or with numbers.
function call(name, ...args) {
  if (typeof args[0] === 'string') {
    const bytes = encoder.encode(args[0]);
    const ptr = wasm.alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    args = [ptr, bytes.length];
  }
  const len = wasm[name](...args);
  return decoder.decode(new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), len));
}

function deliver(answer) {
  for (const text of JSON.parse(answer)) postMessage({ type: 'server', text });
}

function failed(error) {
  let why = '';
  try { why = wasm ? call('panic_message') : ''; } catch (e) { /* the module is gone */ }
  postMessage({ type: 'failed', message: why || String(error) });
  wasm = null;
}

const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

async function boot() {
  try {
    const [code, net] = await Promise.all([
      fetch('game.wasm').then(r => {
        if (!r.ok) throw new Error(`could not load game.wasm (${r.status})`);
        return r.arrayBuffer();
      }),
      fetch('net.json').then(r => (r.ok ? r.text() : '')).catch(() => ''),
    ]);
    wasm = (await WebAssembly.instantiate(code, {})).instance.exports;
    if (net) {
      const problem = call('load_net', net);
      if (problem) postMessage({ type: 'note', message: `Playing without the trained network: ${problem}` });
    }
    const seed = crypto.getRandomValues(new Uint32Array(2));
    deliver(call('start', seed[0], seed[1], SIMULATIONS, SAMPLES));
    for (const text of waiting.splice(0)) receive(text);
  } catch (e) {
    failed(e);
  }
}

function receive(text) {
  if (!wasm) {
    waiting.push(text);
    return;
  }
  try {
    const out = call('handle', text);
    deliver(out);
    // Notes (lines typed in the chat) are saved by the page, in the artifact's database.
    for (const note of JSON.parse(call('notes'))) postMessage({ type: 'save-note', note });
    // One player: sit them down and give the other seat to the bot.
    if (!seated && JSON.parse(text).type === 'hello') {
      seated = true;
      deliver(call('handle', JSON.stringify({ type: 'claim', seat: 0 })));
      deliver(call('handle', JSON.stringify({ type: 'add_bot', seat: 1 })));
    }
  } catch (e) {
    failed(e);
    return;
  }
  runBot();
}

// While it's the bot's move: show it thinking, think, wait out the rest of
// the delay (the page stays responsive: messages are handled meanwhile, and a
// move made stale by an undo or a new game is dropped), then play.
async function runBot() {
  if (botRunning) return;
  botRunning = true;
  try {
    while (wasm) {
      const shown = call('bot_start');
      if (!wasm.bot_waiting()) break;
      deliver(shown);
      await pause(20); // let the page draw "thinking" before the busy part
      if (!wasm) break;
      const started = performance.now();
      wasm.bot_think();
      await pause(Math.max(0, DELAY_MS - (performance.now() - started)));
      if (!wasm) break;
      deliver(call('bot_finish'));
    }
  } catch (e) {
    failed(e);
  } finally {
    botRunning = false;
  }
}

self.onmessage = ({ data }) => receive(data);
boot();
