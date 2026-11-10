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
    }
  };
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
