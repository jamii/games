//! A web app for playing one game of 001 between two browsers.
//!
//! The server holds a single game in memory. Each browser connects over a
//! websocket, takes one of the two seats, and gets its own view of the game
//! (hands are hidden from the other player) every time anything changes.

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use game_001::data::{self, CellKind, Seat, data};
use game_001::game::{Action, Game, LABELS, MAX_POWERS, Options, SEAT_NAMES};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const STYLE_CSS: &str = include_str!("../static/style.css");

/// How many log entries the server keeps, and how many it sends to clients.
const LOG_LIMIT: usize = 1000;
const LOG_SENT: usize = 300;
const UNDO_LIMIT: usize = 200;
const CHAT_LIMIT: usize = 500;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    /// Identifies the browser tab, so that it gets its seat back after a reconnect.
    Hello {
        token: String,
    },
    Claim {
        seat: Seat,
    },
    Leave,
    NewGame {
        options: Options,
    },
    Action {
        action: Action,
    },
    Undo,
    Chat {
        text: String,
    },
}

#[derive(Serialize)]
struct LogEntry {
    seat: Option<Seat>,
    text: String,
    chat: bool,
}

struct Conn {
    token: Option<String>,
    tx: mpsc::UnboundedSender<String>,
}

struct Server {
    game: Game,
    log: Vec<LogEntry>,
    /// Snapshots for undo, cleared whenever the turn passes or hidden cards are revealed.
    history: Vec<(Game, String)>,
    /// The token of the browser in each seat.
    seats: [Option<String>; 2],
    conns: HashMap<u64, Conn>,
    next_conn: u64,
}

type Shared = Arc<Mutex<Server>>;

impl Server {
    fn new() -> Server {
        let (game, lines) = Game::new(Options::default()).expect("the default options are valid");
        let mut server = Server {
            game,
            log: Vec::new(),
            history: Vec::new(),
            seats: [None, None],
            conns: HashMap::new(),
            next_conn: 1,
        };
        server.push_log(None, lines);
        server
    }

    fn push_log(&mut self, seat: Option<Seat>, lines: Vec<String>) {
        self.log.extend(lines.into_iter().map(|text| LogEntry {
            seat,
            text,
            chat: false,
        }));
        if self.log.len() > LOG_LIMIT {
            self.log.drain(..self.log.len() - LOG_LIMIT);
        }
    }

    fn connect(&mut self, tx: mpsc::UnboundedSender<String>) -> u64 {
        let id = self.next_conn;
        self.next_conn += 1;
        let _ = tx.send(init_message());
        let _ = tx.send(self.state_message(None));
        self.conns.insert(id, Conn { token: None, tx });
        id
    }

    fn disconnect(&mut self, id: u64) {
        self.conns.remove(&id);
        self.broadcast();
    }

    fn seat_of(&self, id: u64) -> Option<Seat> {
        let token = self.conns.get(&id)?.token.as_ref()?;
        self.seats
            .iter()
            .position(|seat| seat.as_ref() == Some(token))
    }

    fn seat_connected(&self, seat: Seat) -> bool {
        self.seats[seat]
            .as_ref()
            .is_some_and(|token| self.conns.values().any(|c| c.token.as_ref() == Some(token)))
    }

    fn state_message(&self, seat: Option<Seat>) -> String {
        let seats: Vec<Value> = (0..2)
            .map(|s| json!({ "claimed": self.seats[s].is_some(), "connected": self.seat_connected(s) }))
            .collect();
        let log = &self.log[self.log.len().saturating_sub(LOG_SENT)..];
        json!({
            "type": "state",
            "you": seat,
            "seats": seats,
            "game": self.game.view(seat),
            "log": log,
            "can_undo": seat == Some(self.game.current)
                && self.game.pending.is_none()
                && !self.history.is_empty(),
        })
        .to_string()
    }

    fn broadcast(&self) {
        for (&id, conn) in &self.conns {
            let _ = conn.tx.send(self.state_message(self.seat_of(id)));
        }
    }

    fn send_error(&self, id: u64, message: impl Into<String>, retry: Option<Value>) {
        if let Some(conn) = self.conns.get(&id) {
            let msg = json!({ "type": "error", "message": message.into(), "retry": retry });
            let _ = conn.tx.send(msg.to_string());
        }
    }

    fn handle(&mut self, id: u64, text: &str) {
        let msg: ClientMsg = match serde_json::from_str(text) {
            Ok(msg) => msg,
            Err(e) => return self.send_error(id, format!("Bad message: {e}"), None),
        };
        match msg {
            ClientMsg::Hello { token } => {
                if let Some(conn) = self.conns.get_mut(&id) {
                    conn.token = Some(token);
                }
                self.broadcast();
            }

            ClientMsg::Claim { seat } => {
                if seat > 1 {
                    return self.send_error(id, "Unknown seat.", None);
                }
                let Some(token) = self.conns.get(&id).and_then(|c| c.token.clone()) else {
                    return self.send_error(id, "Still connecting, try again.", None);
                };
                if self.seats[seat].as_ref() == Some(&token) {
                    return;
                }
                if self.seat_connected(seat) {
                    return self.send_error(id, "Someone else is sitting there.", None);
                }
                for held in &mut self.seats {
                    if held.as_ref() == Some(&token) {
                        *held = None;
                    }
                }
                self.seats[seat] = Some(token);
                self.push_log(
                    None,
                    vec![format!("A player took the {} seat.", SEAT_NAMES[seat])],
                );
                self.broadcast();
            }

            ClientMsg::Leave => {
                if let Some(seat) = self.seat_of(id) {
                    self.seats[seat] = None;
                    self.push_log(
                        None,
                        vec![format!("The {} player left their seat.", SEAT_NAMES[seat])],
                    );
                    self.broadcast();
                }
            }

            ClientMsg::NewGame { options } => {
                let Some(seat) = self.seat_of(id) else {
                    return self.send_error(id, "Take a seat to start a new game.", None);
                };
                match Game::new(options) {
                    Ok((game, lines)) => {
                        self.game = game;
                        self.history.clear();
                        self.push_log(
                            Some(seat),
                            vec![format!("{} started a new game.", SEAT_NAMES[seat])],
                        );
                        self.push_log(None, lines);
                        self.broadcast();
                    }
                    Err(message) => self.send_error(id, message, None),
                }
            }

            ClientMsg::Action { action } => {
                let Some(seat) = self.seat_of(id) else {
                    return self.send_error(id, "Take a seat to play.", None);
                };
                // While a prompt is waiting, the game is still the state before
                // the action, plus the prompt.
                let mut before = self.game.clone();
                before.pending = None;
                match self.game.apply(seat, &action) {
                    Ok(lines) => {
                        let revealed = self.game.draws != before.draws;
                        let turn_over =
                            before.turn != self.game.turn || before.current != self.game.current;
                        if revealed || turn_over || !action.undoable() {
                            self.history.clear();
                        } else if self.game.pending.is_none()
                            && !matches!(action, Action::Cancel)
                            && !lines.is_empty()
                        {
                            let what = lines.first().cloned().unwrap_or_default();
                            self.history.push((before, what));
                            if self.history.len() > UNDO_LIMIT {
                                self.history.remove(0);
                            }
                        }
                        self.push_log(Some(seat), lines);
                        self.broadcast();
                    }
                    Err(e) => {
                        let retry = e.forceable.then(|| {
                            let mut value =
                                serde_json::to_value(&action).expect("actions serialize");
                            value["force"] = json!(true);
                            value
                        });
                        self.send_error(id, e.message, retry);
                    }
                }
            }

            ClientMsg::Undo => {
                let Some(seat) = self.seat_of(id) else {
                    return self.send_error(id, "Take a seat to play.", None);
                };
                if seat != self.game.current {
                    return self.send_error(id, "You can only undo during your own turn.", None);
                }
                if self.game.pending.is_some() {
                    return self.send_error(id, "Cancel the current action first.", None);
                }
                let Some((game, what)) = self.history.pop() else {
                    return self.send_error(id, "Nothing to undo.", None);
                };
                self.game = game;
                self.push_log(
                    Some(seat),
                    vec![format!("{} undid: {what}", SEAT_NAMES[seat])],
                );
                self.broadcast();
            }

            ClientMsg::Chat { text } => {
                let text: String = text.trim().chars().take(CHAT_LIMIT).collect();
                if text.is_empty() {
                    return;
                }
                let seat = self.seat_of(id);
                self.log.push(LogEntry {
                    seat,
                    text,
                    chat: true,
                });
                self.push_log(None, Vec::new());
                self.broadcast();
            }
        }
    }
}

/// The static data the client needs: the board, the cards and the rules.
fn init_message() -> String {
    let d = data();
    let board: Vec<Value> = d
        .board
        .cells
        .iter()
        .map(|(cell, kind)| {
            let (kind, seat) = match kind {
                CellKind::Empty => ("empty", None),
                CellKind::Home(seat) => ("home", Some(*seat)),
                CellKind::Flag => ("flag", None),
            };
            json!({ "x": cell.x, "y": cell.y, "kind": kind, "seat": seat })
        })
        .collect();
    json!({
        "type": "init",
        "board": board,
        "powers": d.powers,
        "flag": d.flag,
        "walk": d.walk,
        "rules": data::RULES_MD,
        "seat_names": SEAT_NAMES,
        "labels": LABELS,
        "max_powers": MAX_POWERS,
    })
    .to_string()
}

async fn ws_handler(ws: WebSocketUpgrade, State(shared): State<Shared>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, shared))
}

async fn handle_socket(mut socket: WebSocket, shared: Shared) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let id = shared.lock().unwrap().connect(tx);
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
                Some(Ok(Message::Text(text))) => shared.lock().unwrap().handle(id, text.as_str()),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    shared.lock().unwrap().disconnect(id);
}

#[tokio::main]
async fn main() {
    // Parse the rules and cards up front so that mistakes in them show up immediately.
    let d = data();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let shared: Shared = Arc::new(Mutex::new(Server::new()));
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
