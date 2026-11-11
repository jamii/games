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
//!
//! Every action (players' and the bot's, refused ones too), undo, new game,
//! seat change and player's note goes in a journal on disk, one JSON object a
//! line, in `LOG_DIR/journal-YYYY-MM-DD.jsonl` (default `logs/`, UTC dates),
//! kept for `LOG_DAYS` days (default 7; 0 keeps none). `tools/notes.py` lists
//! the notes, and `examples/replay.rs` replays the game a note was made in.

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use game_001::data::data;
use game_001::net::Net;
use game_001::session::{BotConfig, Outbox, Session};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
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

/// The journal on disk: a file a day, the oldest deleted after `days` days.
struct Journal {
    dir: PathBuf,
    days: i64,
    /// The day of the open file, and the file.
    open: Option<(String, std::fs::File)>,
}

impl Journal {
    fn from_env() -> Option<Journal> {
        let days = std::env::var("LOG_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
        let dir = std::env::var("LOG_DIR").unwrap_or_else(|_| "logs".into());
        (days > 0).then(|| Journal {
            dir: dir.into(),
            days,
            open: None,
        })
    }

    /// Appends the records, each with the time, to today's file.
    fn write(&mut self, records: Vec<Value>) -> Result<(), String> {
        if records.is_empty() {
            return Ok(());
        }
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let today = date(secs.div_euclid(86_400));
        let s = secs.rem_euclid(86_400);
        let time = format!("{today}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60);
        if self.open.as_ref().is_none_or(|(day, _)| *day != today) {
            std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
            let path = self.dir.join(format!("journal-{today}.jsonl"));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            self.open = Some((today.clone(), file));
            self.prune(secs.div_euclid(86_400));
        }
        let (_, file) = self.open.as_mut().expect("opened above");
        let mut text = String::new();
        for mut record in records {
            record["time"] = json!(time);
            text.push_str(&record.to_string());
            text.push('\n');
        }
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())
    }

    /// Deletes the files more than `days` days old.
    fn prune(&self, today: i64) {
        let oldest = date(today - self.days);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(day) = name.strip_prefix("journal-").and_then(|n| n.strip_suffix(".jsonl"))
                && day < oldest.as_str()
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// YYYY-MM-DD for a count of days since 1970-01-01 (proleptic Gregorian).
fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The session, and where to send each connection's messages.
struct Server {
    session: Session,
    senders: HashMap<u64, mpsc::UnboundedSender<String>>,
    journal: Option<Journal>,
}

impl Server {
    fn deliver(&self, out: Outbox) {
        for (id, text) in out {
            if let Some(tx) = self.senders.get(&id) {
                let _ = tx.send(text);
            }
        }
    }

    /// Writes what the session journaled. If a note couldn't be saved, tells
    /// whoever wrote it (`from`).
    fn save_journal(&mut self, from: Option<u64>) {
        let records = self.session.take_journal();
        let Some(journal) = &mut self.journal else {
            return;
        };
        let note = records.iter().any(|r| r["kind"] == "comment");
        if let Err(e) = journal.write(records) {
            eprintln!("Couldn't write the journal: {e}");
            if let (true, Some(id)) = (note, from) {
                let msg = json!({ "type": "error", "message": format!("The note wasn't saved: {e}") });
                self.deliver(vec![(id, msg.to_string())]);
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
            s.save_journal(None);
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
                        s.save_journal(Some(id));
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
    let mut session = Session::new(bot_config_from_env(), rand::random());
    println!("Bot: {}.", session.bot.describe());
    let journal = Journal::from_env();
    match &journal {
        Some(j) => {
            session.keep_journal();
            println!("Journal: {}/journal-*.jsonl, kept {} days.", j.dir.display(), j.days);
        }
        None => println!("Journal: off (LOG_DAYS=0)."),
    }
    let shared: Shared = Arc::new(Mutex::new(Server {
        session,
        senders: HashMap::new(),
        journal,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(59), "1970-03-01");
        assert_eq!(date(11_016), "2000-02-29");
        assert_eq!(date(20_768), "2026-11-11");
        assert_eq!(date(-1), "1969-12-31");
    }

    #[test]
    fn the_journal_appends_and_prunes() {
        let dir = std::env::temp_dir().join(format!("001-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("journal-2000-01-01.jsonl");
        std::fs::write(&old, "{}\n").unwrap();
        let keep = dir.join("notes.txt");
        std::fs::write(&keep, "mine").unwrap();
        let mut j = Journal { dir: dir.clone(), days: 7, open: None };
        j.write(vec![json!({ "kind": "action" })]).unwrap();
        j.write(vec![json!({ "kind": "comment", "text": "x" }), json!({ "kind": "undo" })]).unwrap();
        assert!(!old.exists(), "old files are deleted");
        assert!(keep.exists(), "other files are left alone");
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        let today = files.iter().find(|p| p.extension().is_some_and(|e| e == "jsonl")).unwrap();
        let lines: Vec<Value> = std::fs::read_to_string(today)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["text"], "x");
        assert!(lines[0]["time"].as_str().unwrap().ends_with('Z'));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
