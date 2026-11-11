// The single-player page has no server: the game and the bot run in a Web
// Worker (worker.js). This stands in for the WebSocket that app.js opens, so
// app.js runs unchanged (apart from its help text). Load it before app.js.
window.SINGLE_PLAYER = true;
(() => {
  const worker = new Worker('worker.js');
  let socket = null;
  const early = []; // messages that arrive before app.js is listening

  function stopped(message) {
    const text = `The game stopped: ${message}. Reload the page to start a new game.`;
    if (typeof window.showToast === 'function') window.showToast(text);
    const banner = document.getElementById('engine-stopped');
    if (banner) {
      banner.textContent = text;
      banner.hidden = false;
    }
  }

  worker.onmessage = ({ data }) => {
    if (data.type === 'server') {
      if (socket && socket.readyState === 1 && socket.onmessage) socket.onmessage({ data: data.text });
      else early.push(data.text);
    } else if (data.type === 'failed') {
      stopped(data.message);
    } else if (data.type === 'note') {
      console.warn(data.message);
    } else if (data.type === 'save-note') {
      saveNote(data.note);
    }
  };

  // Notes (whatever the player types in the chat) go in this page's database,
  // where only its owner and editors can read them (and Claude, for them).
  // Elsewhere (a plain web server, a viewer who can't edit) the chat only goes
  // in the log.
  const notesDb = (async () => {
    if (!window.claude || typeof window.claude.use !== 'function') return null;
    let db = null;
    try {
      db = await window.claude.use('db');
      const user = db && await window.claude.use('user');
      if (user && !(await user.canEdit())) db = null;
    } catch (e) {
      db = null; // no notes here
    }
    if (db) {
      window.NOTES_HERE = true;
      setTimeout(() => { if (typeof render === 'function') render(); }, 0); // says so in the chat box
    }
    return db;
  })();

  function tell(msg) {
    if (socket && socket.onmessage) socket.onmessage({ data: JSON.stringify(msg) });
  }

  async function saveNote(note) {
    const db = await notesDb;
    if (!db) return;
    // Stored as objects ({seat, action}): some stores refuse arrays in arrays.
    const body = { ...note, actions: (note.actions || []).map(([seat, action]) => ({ seat, action })),
      time: new Date().toISOString(), status: 'open' };
    try {
      await db.collection('notes').add(body);
      tell({ type: 'notice', message: 'Saved as a note with this page.' });
    } catch (e) {
      tell({ type: 'error', message: `The note wasn't saved: ${(e && (e.message || e.code)) || e}` });
    }
  }
  worker.onerror = e => stopped(e.message || 'the game engine could not start');

  class LocalSocket {
    constructor() {
      socket = this;
      this.readyState = LocalSocket.CONNECTING;
      setTimeout(() => {
        this.readyState = LocalSocket.OPEN;
        if (this.onopen) this.onopen({});
        for (const text of early.splice(0)) if (this.onmessage) this.onmessage({ data: text });
      }, 0);
    }
    send(text) {
      worker.postMessage(text);
    }
    close() {}
  }
  LocalSocket.CONNECTING = 0;
  LocalSocket.OPEN = 1;
  LocalSocket.CLOSING = 2;
  LocalSocket.CLOSED = 3;
  window.WebSocket = LocalSocket;
})();
