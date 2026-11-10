//! A web app for playing one game of 001 between two browsers, or against a
//! bot.
//!
//! The server holds a single game in memory (a `session::Session`). Each
//! browser connects over a websocket, takes one of the two seats, and gets its
//! own view of the game (hands are hidden from the other player) every time
//! anything changes.
//!
//! A bot can take an empty seat (with automatic rules). It searches with the
//! network in `BOT_NET` (default `bots/net.json`, if present) or else the
//! hand-made evaluation; `BOT_SEARCH` sets the search size (default 400x4)
//! and `BOT_DELAY_MS` the pause before each of its actions (default 1000).

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use game_001::data::data;
use game_001::net::Net;
use game_001::session::{BotConfig, Outbox, Session};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const STYLE_CSS: &str = include_str!("../static/style.css");

fn bot_config_from_env() -> BotConfig {
    let path = std::env::var("BOT_NET").unwrap_or_else(|_| "bots/net.json".into());
    let net = if std::path::Path::new(&path).exists() {
        match Net::load(&path) {
            Ok(net) => Some(Arc::new(net)),
            Err(e) => {
                eprintln!("Not using the bot network: {e}");
                None
            }
        }
    } else {
        None
    };
    let search = std::env::var("BOT_SEARCH").unwrap_or_else(|_| "400x4".into());
    let (sims, samples) = search.split_once('x').unwrap_or((&search, "4"));
    BotConfig {
        net,
        simulations: sims.parse().unwrap_or(400),
        samples: samples.parse().unwrap_or(4),
        delay: std::time::Duration::from_millis(
            std::env::var("BOT_DELAY_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
        ),
    }
}

/// The session, and where to send each connection's messages.
struct Server {
    session: Session,
    senders: HashMap<u64, mpsc::UnboundedSender<String>>,
}

impl Server {
    fn deliver(&self, out: Outbox) {
        for (id, text) in out {
            if let Some(tx) = self.senders.get(&id) {
                let _ = tx.send(text);
            }
        }
    }
}

type Shared = Arc<Mutex<Server>>;

/// If it's a bot's move, think on another thread and then play it (unless the
/// game changed in the meantime), and repeat.
fn run_bot(shared: Shared) {
    let job = {
        let mut s = shared.lock().unwrap();
        let Some((job, out)) = s.session.bot_due() else {
            return;
        };
        s.deliver(out);
        job
    };
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let action = job.think();
        if let Some(rest) = job.config.delay.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
        {
            let mut s = shared.lock().unwrap();
            let out = s.session.bot_done(&job, &action);
            s.deliver(out);
        }
        run_bot(shared);
    });
}

async fn ws_handler(ws: WebSocketUpgrade, State(shared): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, shared))
}

async fn handle_socket(mut socket: WebSocket, shared: Shared) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let id = {
        let mut s = shared.lock().unwrap();
        let (id, out) = s.session.connect();
        s.senders.insert(id, tx);
        s.deliver(out);
        id
    };
    loop {
        tokio::select! {
            outgoing = rx.recv() => match outgoing {
                Some(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    {
                        let mut s = shared.lock().unwrap();
                        let out = s.session.handle(id, text.as_str());
                        s.deliver(out);
                    }
                    run_bot(shared.clone());
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    let mut s = shared.lock().unwrap();
    s.senders.remove(&id);
    let out = s.session.disconnect(id);
    s.deliver(out);
}

#[tokio::main]
async fn main() {
    // Parse the rules and cards up front so that mistakes in them show up immediately.
    let d = data();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let session = Session::new(bot_config_from_env(), rand::random());
    println!("Bot: {}.", session.bot.describe());
    let shared: Shared = Arc::new(Mutex::new(Server {
        session,
        senders: HashMap::new(),
    }));
    let app = Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    APP_JS,
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    STYLE_CSS,
                )
            }),
        )
        .route("/ws", get(ws_handler))
        .with_state(shared);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| panic!("could not listen on port {port}: {e}"));
    println!(
        "Loaded {} powers and a {}-space board. Open http://localhost:{port} in two browsers.",
        d.powers.len(),
        d.board.cells.len()
    );
    axum::serve(listener, app).await.expect("server error");
}
