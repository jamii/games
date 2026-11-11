//! The game and the bot for a single-player web page, compiled to
//! WebAssembly. `worker.js` drives it from a Web Worker, the way the web server
//! drives a `Session`: messages in, messages out.
//!
//! Strings cross as UTF-8 bytes. To pass one in, JavaScript calls `alloc(len)`,
//! writes the bytes there and passes `(ptr, len)`; the call takes ownership.
//! Every call that answers leaves its answer (JSON text) in a buffer and returns
//! its length; `out_ptr()` says where it is.

use game_001::game::Action;
use game_001::net::Net;
use game_001::session::{BotConfig, BotJob, Session};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    session: Option<Session>,
    /// The page's connection to the session.
    conn: u64,
    net: Option<Arc<Net>>,
    job: Option<BotJob>,
    answer: Option<Action>,
    out: Vec<u8>,
    /// Notes: the lines the player typed in the chat (journal records of kind
    /// "comment"), for the page to save; the rest of the journal is dropped.
    notes: Vec<serde_json::Value>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// The last panic's message, for the page to show (the module stops after one).
static PANIC: Mutex<String> = Mutex::new(String::new());

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// Leaves `text` as the answer and returns its length.
fn answer(text: String) -> usize {
    with(|s| {
        s.out = text.into_bytes();
        s.out.len()
    })
}

/// Keeps the notes from the session's journal, and drops the rest.
fn keep_notes(s: &mut State) {
    if let Some(session) = &mut s.session {
        let records = session.take_journal();
        s.notes.extend(records.into_iter().filter(|r| r["kind"] == "comment"));
    }
}

/// The messages for the page, as a JSON array of message texts.
fn messages(out: game_001::session::Outbox) -> usize {
    let texts: Vec<String> = out.into_iter().map(|(_, text)| text).collect();
    answer(serde_json::to_string(&texts).expect("strings serialize"))
}

/// Takes back a string passed in from JavaScript.
fn input(ptr: *mut u8, len: usize) -> String {
    // SAFETY: `ptr` came from `alloc(len)` and is passed in exactly once.
    let bytes = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
    String::from_utf8(bytes.into_vec()).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn out_ptr() -> *const u8 {
    with(|s| s.out.as_ptr())
}

/// Loads the bot's network (the JSON written by tools/train.py). Answers ""
/// or why it couldn't.
#[unsafe(no_mangle)]
pub extern "C" fn load_net(ptr: *mut u8, len: usize) -> usize {
    let text = input(ptr, len);
    let result = serde_json::from_str(&text)
        .map_err(|e| e.to_string())
        .and_then(|json| Net::from_json(&json));
    match result {
        Ok(net) => {
            with(|s| s.net = Some(Arc::new(net)));
            answer(String::new())
        }
        Err(e) => answer(e),
    }
}

/// Starts a session (with the network, if one was loaded) and connects the
/// page. `seed` drives the shuffles; there's no other randomness here. Answers
/// the first messages for the page.
#[unsafe(no_mangle)]
pub extern "C" fn start(seed_low: u32, seed_high: u32, simulations: u32, samples: u32) -> usize {
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut message) = PANIC.lock() {
            *message = info.to_string();
        }
    }));
    let out = with(|s| {
        let bot = BotConfig {
            net: s.net.clone(),
            simulations,
            samples,
            delay: std::time::Duration::ZERO,
        };
        let mut session = Session::new(bot, (seed_high as u64) << 32 | seed_low as u64);
        session.keep_journal();
        let (conn, out) = session.connect();
        s.session = Some(session);
        s.conn = conn;
        out
    });
    messages(out)
}

/// A message from the page (the same JSON the web server takes).
#[unsafe(no_mangle)]
pub extern "C" fn handle(ptr: *mut u8, len: usize) -> usize {
    let text = input(ptr, len);
    let out = with(|s| {
        let conn = s.conn;
        let out = s
            .session
            .as_mut()
            .map(|session| session.handle(conn, &text))
            .unwrap_or_default();
        keep_notes(s);
        out
    });
    messages(out)
}

/// The notes written since the last call (see the session's journal: each
/// has the text, what the screen showed, and the game's options and
/// actions), as a JSON array.
/// Seeds (64-bit) are written as text: JavaScript numbers would round them,
/// and a replay with a rounded seed deals a different game.
#[unsafe(no_mangle)]
pub extern "C" fn notes() -> usize {
    let mut notes = with(|s| std::mem::take(&mut s.notes));
    for note in &mut notes {
        for field in ["seed", "game"] {
            if let Some(n) = note[field].as_u64() {
                note[field] = n.to_string().into();
            }
        }
        if let Some(n) = note["options"]["seed"].as_u64() {
            note["options"]["seed"] = n.to_string().into();
        }
    }
    answer(serde_json::to_string(&notes).expect("notes serialize"))
}

/// If it's the bot's move, sets up its thinking and answers the messages that
/// show it thinking; otherwise answers no messages. `bot_waiting` says which.
#[unsafe(no_mangle)]
pub extern "C" fn bot_start() -> usize {
    let out = with(
        |s| match s.session.as_mut().and_then(|session| session.bot_due()) {
            Some((job, out)) => {
                s.job = Some(job);
                s.answer = None;
                out
            }
            None => Vec::new(),
        },
    );
    messages(out)
}

#[unsafe(no_mangle)]
pub extern "C" fn bot_waiting() -> u32 {
    with(|s| s.job.is_some() as u32)
}

/// The bot thinks (this takes a while). The move is played by `bot_finish`.
#[unsafe(no_mangle)]
pub extern "C" fn bot_think() {
    // Think outside the borrow: nothing else runs meanwhile, but keep it simple.
    let job = with(|s| s.job.take());
    if let Some(job) = job {
        let action = job.think();
        with(|s| {
            s.answer = Some(action);
            s.job = Some(job);
        });
    }
}

/// Plays the bot's move, unless the game changed while it thought, and
/// answers the messages for the page.
#[unsafe(no_mangle)]
pub extern "C" fn bot_finish() -> usize {
    let out = with(|s| {
        let (Some(job), Some(action)) = (s.job.take(), s.answer.take()) else {
            return Vec::new();
        };
        let out = s
            .session
            .as_mut()
            .map(|session| session.bot_done(&job, &action))
            .unwrap_or_default();
        keep_notes(s);
        out
    });
    messages(out)
}

/// Why the module stopped, if it panicked.
#[unsafe(no_mangle)]
pub extern "C" fn panic_message() -> usize {
    let message = PANIC.lock().map(|m| m.clone()).unwrap_or_default();
    answer(message)
}
