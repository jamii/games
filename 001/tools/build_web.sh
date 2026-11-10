#!/usr/bin/env bash
# Builds the single-player page into web/dist/: the game and the bot (with the
# network in bots/net.json) compiled to WebAssembly, running in the browser.
#
#   tools/build_web.sh
#
# web/dist/ is a static site: publish it as a Claude artifact, or serve it with
# any static file server (eg `python3 -m http.server -d web/dist`). Needs the
# WebAssembly target once: `rustup target add wasm32-unknown-unknown`.
set -euo pipefail
cd "$(dirname "$0")/.."
(cd web && cargo build --release --target wasm32-unknown-unknown)
out=web/dist
rm -rf "$out"
mkdir -p "$out"
cp web/target/wasm32-unknown-unknown/release/game_001_web.wasm "$out/game.wasm"
cp static/app.js static/style.css web/worker.js web/local-socket.js "$out/"
cp bots/net.json "$out/net.json"
# The page: the web app's markup, with the stand-in socket loaded before app.js.
# No <html>, <head> or <body>: the artifact viewer adds them.
python3 - static/index.html "$out/index.html" <<'EOF'
import re, sys
src, dst = sys.argv[1:]
html = open(src).read()
body = re.search(r"<body>(.*)</body>", html, re.S).group(1)
body = body.replace('<script src="/app.js"></script>',
                    '<script src="local-socket.js"></script>\n  <script src="app.js"></script>')
assert "local-socket.js" in body, "static/index.html no longer loads /app.js the way this script expects"
head = """<title>001 Solo</title>
<link rel="stylesheet" href="style.css">
<style>
  /* Single player: the bot only plays with automatic rules, so no rules switch. */
  [data-act="toggle-mode"] { display: none; }
  #engine-stopped { margin: 8px max(16px, env(safe-area-inset-left, 0px)); padding: 10px 14px;
    border: 1px solid var(--danger); border-radius: 6px; background: var(--panel); color: var(--danger); }
</style>
<div id="engine-stopped" role="alert" hidden></div>"""
# Entities for anything outside ASCII, so the page reads right whatever
# encoding a server declares.
open(dst, "w").write((head + body).encode("ascii", "xmlcharrefreplace").decode())
EOF
ls -la "$out"
