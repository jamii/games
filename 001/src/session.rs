//! A game session: one game, the two seats, the log, undo, and the bot.
//!
//! The web server (src/main.rs) and the browser build (web/) both run one.
//! It doesn't send anything itself: every call returns the messages to deliver,
//! as (connection, JSON text) pairs, and the caller delivers them over a
//! websocket or to a page. The bot's thinking is handed out as a [`BotJob`],
//! so the caller can run it on another thread (or in a web worker) and report
//! back with [`Session::bot_done`].

use crate::bot::{Agent, Evaluator, MctsAgent, MctsConfig, to_move};
use crate::data::{self, CellKind, Seat, data};
use crate::game::{
    Action, ActionError, Game, LABELS, MAX_POWERS, Options, Phase, SEAT_NAMES, next_random,
};
use crate::net::Net;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

/// How many log entries a session keeps, and how many it sends to clients.
const LOG_LIMIT: usize = 1000;
const LOG_SENT: usize = 300;
const UNDO_LIMIT: usize = 200;
/// Chat lines are also bug notes, so they can be long.
const CHAT_LIMIT: usize = 2000;
/// How many of the log's last lines a note keeps.
const NOTE_LOG_LINES: usize = 40;

/// Messages to deliver: (connection, JSON text).
pub type Outbox = Vec<(u64, String)>;

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
    /// A line in the log's chat. With a journal it's also a note for whoever
    /// looks after the game (eg a bug), kept with what the browser was showing
    /// and everything needed to replay the game.
    Chat {
        text: String,
        #[serde(default)]
        context: Value,
    },
    AddBot {
        seat: Seat,
    },
    RemoveBot {
        seat: Seat,
    },
}

/// How the bot plays.
#[derive(Clone)]
pub struct BotConfig {
    /// The network to search with; without one, the hand-made evaluation.
    pub net: Option<Arc<Net>>,
    pub simulations: u32,
    pub samples: u32,
    /// How long each of the bot's actions should take at least, so people can
    /// follow them. The caller waits; the session doesn't.
    pub delay: std::time::Duration,
}

impl Default for BotConfig {
    fn default() -> BotConfig {
        BotConfig {
            net: None,
            simulations: 400,
            samples: 4,
            delay: std::time::Duration::from_millis(700),
        }
    }
}

impl BotConfig {
    pub fn describe(&self) -> String {
        format!(
            "{} search, {}x{}",
            if self.net.is_some() {
                "network-guided"
            } else {
                "hand-made"
            },
            self.simulations,
            self.samples
        )
    }

    fn agent(&self, seed: u64) -> MctsAgent {
        let mut config = MctsConfig {
            simulations: self.simulations,
            samples: self.samples,
            ..MctsConfig::default()
        };
        if let Some(net) = &self.net {
            config.eval = Evaluator::Net(net.clone());
        }
        MctsAgent::new(config, seed)
    }
}

/// A move for the bot to think about. Run [`BotJob::think`] anywhere, then
/// give the answer to [`Session::bot_done`].
pub struct BotJob {
    pub game: Game,
    pub seat: Seat,
    /// The session's version when the job was handed out: if the game has
    /// changed since (an undo, a new game), the answer is thrown away.
    pub version: u64,
    pub config: Arc<BotConfig>,
}

impl BotJob {
    pub fn think(&self) -> Action {
        self.config
            .agent(self.version)
            .choose(&self.game, self.seat)
    }
}

#[derive(Serialize)]
struct LogEntry {
    seat: Option<Seat>,
    text: String,
    chat: bool,
}

pub struct Session {
    pub game: Game,
    log: Vec<LogEntry>,
    /// How many log entries there have ever been, so a page can tell which are new.
    logged: u64,
    /// Snapshots for undo, cleared whenever the turn passes or hidden cards are
    /// revealed, with how many of `actions` there were then.
    history: Vec<(Game, String, usize)>,
    /// The actions played in this game so far (undone ones removed), which
    /// replay it exactly from its options.
    actions: Vec<(Seat, Action)>,
    /// How many actions there were when the current prompt's action began.
    prompt_from: usize,
    /// When on (the server and the browser build turn it on), a record of
    /// every action, seat change and chat line, for the caller to store: see
    /// `take_journal`.
    journal: Option<Vec<Value>>,
    /// The token of the browser in each seat.
    seats: [Option<String>; 2],
    /// Each connection and the token it said hello with.
    conns: BTreeMap<u64, Option<String>>,
    next_conn: u64,
    /// Seats played by the bot.
    bots: [bool; 2],
    /// A bot is choosing an action.
    thinking: bool,
    /// Counts changes to the game, so a bot's late answer can be thrown away.
    version: u64,
    pub bot: Arc<BotConfig>,
    /// Seeds for new games that don't name one.
    rng: u64,
}

impl Session {
    /// A session with a fresh game. `seed` drives the shuffles of this and
    /// later games (the browser build has no other source of randomness).
    pub fn new(bot: BotConfig, seed: u64) -> Session {
        let mut rng = seed;
        let (game, lines) = Game::new(Options {
            seed: Some(next_random(&mut rng)),
            ..Options::default()
        })
        .expect("the default options are valid");
        let mut session = Session {
            game,
            log: Vec::new(),
            logged: 0,
            history: Vec::new(),
            actions: Vec::new(),
            prompt_from: 0,
            journal: None,
            seats: [None, None],
            conns: BTreeMap::new(),
            next_conn: 1,
            bots: [false, false],
            thinking: false,
            version: 0,
            bot: Arc::new(bot),
            rng,
        };
        session.push_log(None, lines);
        session
    }

    fn push_log(&mut self, seat: Option<Seat>, lines: Vec<String>) {
        self.logged += lines.len() as u64;
        self.log.extend(lines.into_iter().map(|text| LogEntry {
            seat,
            text,
            chat: false,
        }));
        if self.log.len() > LOG_LIMIT {
            self.log.drain(..self.log.len() - LOG_LIMIT);
        }
    }

    /// Starts keeping a journal (see `take_journal`).
    pub fn keep_journal(&mut self) {
        self.journal.get_or_insert_with(Vec::new);
    }

    /// The journal records since the last call: one JSON object per action
    /// (players' and the bot's, refused ones too), undo, new game, seat change
    /// and chat line (kind "comment": a player's note). Each names the game by
    /// its seed; a comment also carries what the player's screen showed, the
    /// log's last lines and the game's options and actions (the format
    /// `examples/replay.rs` reads).
    pub fn take_journal(&mut self) -> Vec<Value> {
        self.journal.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn record(&mut self, mut entry: Value) {
        if let Some(journal) = &mut self.journal {
            entry["game"] = json!(self.game.options.seed);
            entry["turn"] = json!(self.game.turn);
            journal.push(entry);
        }
    }

    /// The log lines added since the log had `since` entries in all.
    fn lines_since(&self, since: u64) -> Vec<String> {
        let n = ((self.logged - since) as usize).min(self.log.len());
        self.log[self.log.len() - n..].iter().map(|e| e.text.clone()).collect()
    }

    /// A new connection: returns its id, and the board and state to send it.
    pub fn connect(&mut self) -> (u64, Outbox) {
        let id = self.next_conn;
        self.next_conn += 1;
        self.conns.insert(id, None);
        let out = vec![
            (id, init_message(self.journal.is_some())),
            (id, self.state_message(None)),
        ];
        (id, out)
    }

    pub fn disconnect(&mut self, id: u64) -> Outbox {
        self.conns.remove(&id);
        self.broadcast()
    }

    fn seat_of(&self, id: u64) -> Option<Seat> {
        let token = self.conns.get(&id)?.as_ref()?;
        self.seats
            .iter()
            .position(|seat| seat.as_ref() == Some(token))
    }

    fn seat_connected(&self, seat: Seat) -> bool {
        self.bots[seat]
            || self.seats[seat]
                .as_ref()
                .is_some_and(|token| self.conns.values().any(|t| t.as_ref() == Some(token)))
    }

    fn state_message(&self, seat: Option<Seat>) -> String {
        let seats: Vec<Value> = (0..2)
            .map(|s| {
                json!({
                    "claimed": self.seats[s].is_some() || self.bots[s],
                    "connected": self.seat_connected(s),
                    "bot": self.bots[s],
                    "thinking": self.bots[s] && self.thinking,
                })
            })
            .collect();
        let log = &self.log[self.log.len().saturating_sub(LOG_SENT)..];
        json!({
            "type": "state",
            "you": seat,
            "seats": seats,
            "game": self.game.view(seat),
            "log": log,
            "log_total": self.logged,
            "can_undo": seat == Some(self.game.current)
                && self.game.pending.is_none()
                && !self.history.is_empty(),
        })
        .to_string()
    }

    /// Everyone's view of the game.
    pub fn broadcast(&self) -> Outbox {
        self.conns
            .keys()
            .map(|&id| (id, self.state_message(self.seat_of(id))))
            .collect()
    }

    fn error(&self, id: u64, message: impl Into<String>, retry: Option<Value>) -> Outbox {
        let msg = json!({ "type": "error", "message": message.into(), "retry": retry });
        vec![(id, msg.to_string())]
    }

    /// A message from a connection.
    pub fn handle(&mut self, id: u64, text: &str) -> Outbox {
        let msg: ClientMsg = match serde_json::from_str(text) {
            Ok(msg) => msg,
            Err(e) => return self.error(id, format!("Bad message: {e}"), None),
        };
        match msg {
            ClientMsg::Hello { token } => {
                if let Some(conn) = self.conns.get_mut(&id) {
                    *conn = Some(token);
                }
                self.broadcast()
            }

            ClientMsg::Claim { seat } => {
                if seat > 1 {
                    return self.error(id, "Unknown seat.", None);
                }
                let Some(token) = self.conns.get(&id).cloned().flatten() else {
                    return self.error(id, "Still connecting, try again.", None);
                };
                if self.seats[seat].as_ref() == Some(&token) {
                    return Vec::new();
                }
                if self.bots[seat] {
                    return self.error(id, "A bot is playing there: remove it first.", None);
                }
                if self.seat_connected(seat) {
                    return self.error(id, "Someone else is sitting there.", None);
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
                self.record(json!({ "kind": "seat", "event": "claim", "seat": seat }));
                self.broadcast()
            }

            ClientMsg::Leave => {
                let Some(seat) = self.seat_of(id) else {
                    return Vec::new();
                };
                self.seats[seat] = None;
                self.push_log(
                    None,
                    vec![format!("The {} player left their seat.", SEAT_NAMES[seat])],
                );
                self.record(json!({ "kind": "seat", "event": "leave", "seat": seat }));
                self.broadcast()
            }

            ClientMsg::NewGame { mut options } => {
                let Some(seat) = self.seat_of(id) else {
                    return self.error(id, "Take a seat to start a new game.", None);
                };
                if options.seed.is_none() {
                    options.seed = Some(next_random(&mut self.rng));
                }
                match Game::new(options) {
                    Ok((game, lines)) => {
                        self.game = game;
                        self.version += 1;
                        self.history.clear();
                        self.actions.clear();
                        self.record(json!({
                            "kind": "new_game",
                            "seat": seat,
                            "options": self.game.options,
                        }));
                        self.push_log(
                            Some(seat),
                            vec![format!("{} started a new game.", SEAT_NAMES[seat])],
                        );
                        self.push_log(None, lines);
                        self.broadcast()
                    }
                    Err(message) => self.error(id, message, None),
                }
            }

            ClientMsg::Action { action } => {
                let Some(seat) = self.seat_of(id) else {
                    return self.error(id, "Take a seat to play.", None);
                };
                match self.apply_action(seat, &action, "player") {
                    Ok(()) => self.broadcast(),
                    Err(e) => {
                        let retry = e.forceable.then(|| {
                            let mut value =
                                serde_json::to_value(&action).expect("actions serialize");
                            value["force"] = json!(true);
                            value
                        });
                        self.error(id, e.message, retry)
                    }
                }
            }

            ClientMsg::Undo => {
                let Some(seat) = self.seat_of(id) else {
                    return self.error(id, "Take a seat to play.", None);
                };
                if seat != self.game.current {
                    return self.error(id, "You can only undo during your own turn.", None);
                }
                if self.game.pending.is_some() {
                    return self.error(id, "Cancel the current action first.", None);
                }
                let Some((game, what, played)) = self.history.pop() else {
                    return self.error(id, "Nothing to undo.", None);
                };
                self.game = game;
                self.version += 1;
                self.actions.truncate(played);
                self.push_log(
                    Some(seat),
                    vec![format!("{} undid: {what}", SEAT_NAMES[seat])],
                );
                self.record(json!({ "kind": "undo", "seat": seat, "undid": what }));
                self.broadcast()
            }

            ClientMsg::Chat { text, context } => {
                let text: String = text.trim().chars().take(CHAT_LIMIT).collect();
                if text.is_empty() {
                    return Vec::new();
                }
                let seat = self.seat_of(id);
                if self.journal.is_some() {
                    let recent = self.log.len().saturating_sub(NOTE_LOG_LINES);
                    let log: Vec<&str> =
                        self.log[recent..].iter().map(|e| e.text.as_str()).collect();
                    let entry = json!({
                        "kind": "comment",
                        "seat": seat,
                        "text": text,
                        "context": context,
                        "log": log,
                        "options": self.game.options,
                        "seed": self.game.options.seed,
                        "actions": self.actions,
                    });
                    self.record(entry);
                }
                self.log.push(LogEntry {
                    seat,
                    text,
                    chat: true,
                });
                self.logged += 1;
                self.push_log(None, Vec::new());
                self.broadcast()
            }

            ClientMsg::AddBot { seat } => {
                if seat > 1 {
                    return self.error(id, "Unknown seat.", None);
                }
                if self.bots[seat] {
                    return Vec::new();
                }
                if self.seat_connected(seat) {
                    return self.error(id, "Someone is sitting there.", None);
                }
                self.seats[seat] = None;
                self.bots[seat] = true;
                self.version += 1;
                let mut lines = vec![format!(
                    "A bot took the {} seat ({}).",
                    SEAT_NAMES[seat],
                    self.bot.describe()
                )];
                if !self.game.automatic {
                    lines.push("The bot only plays with automatic rules.".into());
                }
                self.push_log(None, lines);
                self.record(json!({ "kind": "seat", "event": "add_bot", "seat": seat }));
                self.broadcast()
            }

            ClientMsg::RemoveBot { seat } => {
                if seat > 1 || !self.bots[seat] {
                    return Vec::new();
                }
                self.bots[seat] = false;
                self.version += 1;
                self.push_log(
                    None,
                    vec![format!("The bot left the {} seat.", SEAT_NAMES[seat])],
                );
                self.record(json!({ "kind": "seat", "event": "remove_bot", "seat": seat }));
                self.broadcast()
            }
        }
    }

    /// Carries out a player's (or the bot's) action, keeping the undo history
    /// and the journal.
    fn apply_action(&mut self, seat: Seat, action: &Action, by: &str) -> Result<(), ActionError> {
        let since = self.logged;
        let result = self.apply_and_keep(seat, action);
        if self.journal.is_some() {
            let mut entry = json!({ "kind": "action", "seat": seat, "by": by, "action": action });
            match &result {
                Ok(()) => entry["log"] = json!(self.lines_since(since)),
                Err(e) => entry["error"] = json!(e.message),
            }
            self.record(entry);
        }
        result
    }

    fn apply_and_keep(&mut self, seat: Seat, action: &Action) -> Result<(), ActionError> {
        // While a prompt is waiting, the game is still the state before
        // the action, plus the prompt.
        let mut before = self.game.clone();
        before.pending = None;
        // Undo goes back to before the action that started a prompt.
        let prompted = self.game.pending.is_some();
        let played = if prompted { self.prompt_from } else { self.actions.len() };
        let lines = self.game.apply(seat, action)?;
        self.actions.push((seat, action.clone()));
        if !prompted && self.game.pending.is_some() {
            self.prompt_from = played;
        }
        self.version += 1;
        let revealed = self.game.draws != before.draws;
        let turn_over = before.turn != self.game.turn || before.current != self.game.current;
        if revealed || turn_over || !action.undoable() {
            self.history.clear();
        } else if self.game.pending.is_none()
            && !matches!(action, Action::Cancel)
            && !lines.is_empty()
        {
            let what = lines.first().cloned().unwrap_or_default();
            self.history.push((before, what, played));
            if self.history.len() > UNDO_LIMIT {
                self.history.remove(0);
            }
        }
        self.push_log(Some(seat), lines);
        Ok(())
    }

    /// What it takes to pick this game up again (eg after the single-player
    /// page reloads): its options and every action so far, in the format
    /// `examples/replay.rs` reads. None once the game is over, so a reload
    /// then starts a new one.
    pub fn saved_game(&self) -> Option<Value> {
        if matches!(self.game.phase, Phase::Over { .. }) {
            return None;
        }
        Some(json!({ "options": self.game.options, "actions": self.actions }))
    }

    /// Picks up a game from `saved_game` by replaying its actions on a new
    /// game with the same options, so the log and undo come back too. Seats
    /// and bots stay as they are. If the save doesn't replay (it's damaged, or
    /// the rules changed since), nothing changes and the answer says why.
    pub fn resume(&mut self, saved: &Value) -> Result<Outbox, String> {
        let options: Options = serde_json::from_value(saved["options"].clone())
            .map_err(|e| format!("its options don't read: {e}"))?;
        let actions: Vec<(Seat, Action)> = serde_json::from_value(saved["actions"].clone())
            .map_err(|e| format!("its moves don't read: {e}"))?;
        if options.seed.is_none() {
            return Err("it has no seed".into());
        }
        // Players' own actions only ever come from seats 0 and 1, so the
        // engine doesn't check other seats everywhere.
        if actions.iter().any(|&(seat, _)| seat > 1) {
            return Err("it has a move by a seat that doesn't exist".into());
        }
        // Check the whole replay first, so a bad save leaves this game alone.
        let (mut check, _) = Game::new(options.clone())?;
        for (i, (seat, action)) in actions.iter().enumerate() {
            check
                .apply(*seat, action)
                .map_err(|e| format!("move {} no longer works: {}", i + 1, e.message))?;
        }
        let (game, lines) = Game::new(options)?;
        self.game = game;
        self.version += 1;
        self.thinking = false;
        self.history.clear();
        self.actions.clear();
        self.prompt_from = 0;
        self.push_log(None, lines);
        for (seat, action) in &actions {
            self.apply_and_keep(*seat, action)
                .map_err(|e| format!("the replay went wrong: {}", e.message))?;
        }
        self.push_log(
            None,
            vec!["Picked up the game where it left off.".to_string()],
        );
        Ok(self.broadcast())
    }

    /// If it's a bot's move (and it isn't already thinking), the job to run,
    /// and the messages that show it thinking.
    pub fn bot_due(&mut self) -> Option<(BotJob, Outbox)> {
        if self.thinking || !self.game.automatic || matches!(self.game.phase, Phase::Over { .. }) {
            return None;
        }
        let seat = to_move(&self.game);
        if !self.bots[seat] {
            return None;
        }
        self.thinking = true;
        let job = BotJob {
            game: self.game.clone(),
            seat,
            version: self.version,
            config: self.bot.clone(),
        };
        Some((job, self.broadcast()))
    }

    /// The bot's answer to a job: played, unless the game changed meanwhile.
    pub fn bot_done(&mut self, job: &BotJob, action: &Action) -> Outbox {
        self.thinking = false;
        // Shouldn't fail: the bot only picks legal actions.
        if self.version == job.version
            && self.bots[job.seat]
            && let Err(e) = self.apply_action(job.seat, action, "bot")
        {
            self.push_log(None, vec![format!("The bot got stuck: {}", e.message)]);
            self.bots[job.seat] = false;
        }
        self.broadcast()
    }
}

/// The static data the client needs: the board, the cards and the rules, and
/// whether lines typed in the chat are kept as notes (with a journal).
pub fn init_message(notes: bool) -> String {
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
        "notes": notes,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(out: &Outbox, to: u64) -> Vec<Value> {
        out.iter()
            .filter(|(id, _)| *id == to)
            .map(|(_, text)| serde_json::from_str(text).unwrap())
            .collect()
    }

    #[test]
    fn a_player_and_the_bot_play_a_game() {
        let bot = BotConfig {
            simulations: 8,
            samples: 1,
            ..BotConfig::default()
        };
        let mut s = Session::new(bot, 5);
        let (me, out) = s.connect();
        assert_eq!(messages(&out, me)[0]["type"], "init");
        s.handle(me, r#"{"type":"hello","token":"t"}"#);
        s.handle(me, r#"{"type":"claim","seat":0}"#);
        let out = s.handle(me, r#"{"type":"add_bot","seat":1}"#);
        assert_eq!(messages(&out, me)[0]["seats"][1]["bot"], true);
        let mut rng = crate::bot::Rng::new(1);
        for _ in 0..3000 {
            if matches!(s.game.phase, Phase::Over { .. }) || s.game.turn > 30 {
                break;
            }
            if let Some((job, _)) = s.bot_due() {
                let action = job.think();
                s.bot_done(&job, &action);
                continue;
            }
            // My move: something legal, ending the turn now and then.
            let legal = s.game.legal_actions(0);
            let end = legal.iter().find(|a| matches!(a, Action::EndTurn { .. }));
            let action = match end {
                Some(end) if rng.below(4) == 0 => end.clone(),
                _ => legal[rng.below(legal.len())].clone(),
            };
            let text = json!({ "type": "action", "action": action }).to_string();
            let out = s.handle(me, &text);
            assert_eq!(
                messages(&out, me)[0]["type"],
                "state",
                "{action:?} was refused"
            );
        }
        assert!(s.game.turn > 3, "the game moved on");
    }

    #[test]
    fn a_chat_line_is_a_note_that_replays_the_game_exactly() {
        let bot = BotConfig {
            simulations: 8,
            samples: 1,
            ..BotConfig::default()
        };
        let mut s = Session::new(bot, 9);
        s.keep_journal();
        let (me, _) = s.connect();
        s.handle(me, r#"{"type":"hello","token":"t"}"#);
        s.handle(me, r#"{"type":"claim","seat":0}"#);
        s.handle(me, r#"{"type":"add_bot","seat":1}"#);
        let mut rng = crate::bot::Rng::new(4);
        let mut undid = 0;
        for _ in 0..600 {
            if matches!(s.game.phase, Phase::Over { .. }) || s.game.turn > 8 {
                break;
            }
            if let Some((job, _)) = s.bot_due() {
                let action = job.think();
                s.bot_done(&job, &action);
                continue;
            }
            let legal = s.game.legal_actions(0);
            let end = legal.iter().find(|a| matches!(a, Action::EndTurn { .. }));
            let action = match end {
                Some(end) if rng.below(4) == 0 => end.clone(),
                _ => legal[rng.below(legal.len())].clone(),
            };
            s.handle(me, &json!({ "type": "action", "action": action }).to_string());
            // Undo now and then (it has to drop undone actions from the replay).
            if !s.history.is_empty() && rng.below(3) == 0 {
                let out = s.handle(me, r#"{"type":"undo"}"#);
                assert_eq!(messages(&out, me)[0]["type"], "state");
                undid += 1;
            }
        }
        assert!(undid > 0, "undo was tried");
        // A refused action is journaled too.
        let out = s.handle(me, r#"{"type":"action","action":{"type":"spawn","cell":{"x":99,"y":99}}}"#);
        assert_eq!(messages(&out, me)[0]["type"], "error");
        // A line in the chat goes in the log, and in the journal as a note.
        let out = s.handle(me, r#"{"type":"chat","text":" that looks wrong ","context":{"screen":"393x727"}}"#);
        let state = &messages(&out, me)[0];
        assert_eq!(state["type"], "state");
        let line = state["log"].as_array().unwrap().last().unwrap();
        assert_eq!((&line["text"], &line["chat"], &line["seat"]), (&json!("that looks wrong"), &json!(true), &json!(0)));

        let journal = s.take_journal();
        assert!(s.take_journal().is_empty(), "taken once");
        let kinds = |k: &str| journal.iter().filter(|e| e["kind"] == k).count();
        assert!(kinds("action") > 10 && kinds("undo") == undid && kinds("seat") == 2);
        assert!(journal.iter().any(|e| e["by"] == "bot"));
        assert!(journal.iter().any(|e| e["kind"] == "action" && e["error"].is_string()));
        let comment = journal.last().unwrap();
        assert_eq!(comment["kind"], "comment");
        assert_eq!(comment["text"], "that looks wrong");
        assert_eq!(comment["context"]["screen"], "393x727");
        assert_eq!(comment["seat"], 0);
        assert_eq!(comment["game"], comment["seed"]);
        // The log's last lines, from before the note.
        let log = comment["log"].as_array().unwrap();
        assert!(!log.is_empty() && log.len() <= NOTE_LOG_LINES);
        assert_ne!(log.last().unwrap(), "that looks wrong");

        // Replaying the comment's actions from its options gives the same game.
        let options: Options = serde_json::from_value(comment["options"].clone()).unwrap();
        let actions: Vec<(Seat, Action)> = serde_json::from_value(comment["actions"].clone()).unwrap();
        let (mut game, _) = Game::new(options).unwrap();
        for (seat, action) in &actions {
            game.apply(*seat, action).unwrap();
        }
        for seat in [Some(0), Some(1), None] {
            assert_eq!(game.view(seat), s.game.view(seat));
        }
    }

    #[test]
    fn without_a_journal_nothing_is_kept() {
        let mut s = Session::new(BotConfig::default(), 2);
        let (me, _) = s.connect();
        s.handle(me, r#"{"type":"hello","token":"t"}"#);
        s.handle(me, r#"{"type":"claim","seat":0}"#);
        // Chat still works, with or without what the screen showed.
        let out = s.handle(me, r#"{"type":"chat","text":"hi"}"#);
        assert_eq!(messages(&out, me)[0]["log"].as_array().unwrap().last().unwrap()["text"], "hi");
        s.handle(me, r#"{"type":"chat","text":"again","context":{"screen":"1x1"}}"#);
        assert!(s.take_journal().is_empty());
    }

    #[test]
    fn the_bot_answer_is_dropped_after_an_undo_or_new_game() {
        let mut s = Session::new(BotConfig::default(), 1);
        let (me, _) = s.connect();
        s.handle(me, r#"{"type":"hello","token":"t"}"#);
        s.handle(me, r#"{"type":"claim","seat":0}"#);
        s.handle(me, r#"{"type":"add_bot","seat":1}"#);
        // Whoever moves first, get to a position where the bot is due.
        let mut job = None;
        for _ in 0..10 {
            if let Some((j, _)) = s.bot_due() {
                job = Some(j);
                break;
            }
            let legal = s.game.legal_actions(0);
            let text = json!({ "type": "action", "action": legal[0] }).to_string();
            s.handle(me, &text);
        }
        let job = job.expect("the bot gets a move");
        let action = job.think();
        s.handle(me, r#"{"type":"new_game","options":{}}"#);
        let before = s.game.view(None);
        s.bot_done(&job, &action);
        assert_eq!(s.game.view(None), before, "the old answer wasn't played");
    }
    #[test]
    fn a_saved_game_picks_up_where_it_left_off() {
        let bot = || BotConfig {
            simulations: 8,
            samples: 1,
            ..BotConfig::default()
        };
        let mut s = Session::new(bot(), 3);
        let (me, _) = s.connect();
        s.handle(me, r#"{"type":"hello","token":"t"}"#);
        s.handle(me, r#"{"type":"claim","seat":0}"#);
        s.handle(me, r#"{"type":"add_bot","seat":1}"#);
        // Split in my hand, for a prompt.
        let split = data()
            .powers
            .iter()
            .position(|p| p.name == "Split")
            .unwrap();
        let options = json!({ "seed": 11, "first_player": 0, "given_first": [split] });
        s.handle(
            me,
            &json!({ "type": "new_game", "options": options }).to_string(),
        );
        // Play until a prompt is waiting for me in the main phase, so the
        // save has prompts and draws in it, and stops in the middle of one.
        let mut rng = crate::bot::Rng::new(4);
        let mut prompted = false;
        for _ in 0..3000 {
            if s.game.pending.as_ref().is_some_and(|p| p.seat == 0) && s.game.turn > 2 {
                prompted = true;
                break;
            }
            if matches!(s.game.phase, Phase::Over { .. }) {
                break;
            }
            if let Some((job, _)) = s.bot_due() {
                let action = job.think();
                s.bot_done(&job, &action);
                continue;
            }
            let legal = s.game.legal_actions(0);
            // Later on, take an action that asks something, if there is one.
            let asks = legal.iter().find(|a| {
                let mut g = s.game.clone();
                s.game.turn > 2 && g.apply(0, a).is_ok() && g.pending.is_some()
            });
            let action = asks.unwrap_or(&legal[rng.below(legal.len())]).clone();
            s.handle(
                me,
                &json!({ "type": "action", "action": action }).to_string(),
            );
        }
        assert!(prompted, "found a prompt to stop at");
        let saved = s.saved_game().expect("the game isn't over");
        // Through text, the way the page keeps it.
        let saved: Value = serde_json::from_str(&saved.to_string()).unwrap();

        // A reload: a new session with another game, then the kept one.
        let mut t = Session::new(bot(), 77);
        let (me, _) = t.connect();
        t.handle(me, r#"{"type":"hello","token":"t"}"#);
        t.handle(me, r#"{"type":"claim","seat":0}"#);
        t.handle(me, r#"{"type":"add_bot","seat":1}"#);
        let out = t.resume(&saved).unwrap();
        let shown = |s: &Session| {
            let mut v = s.game.view(Some(0));
            v["legal"] = json!(s.game.legal_actions(0));
            v
        };
        assert_eq!(shown(&t), shown(&s), "the same position, prompt and all");
        assert_eq!(messages(&out, me)[0]["game"], s.game.view(Some(0)));
        assert_eq!(t.actions, s.actions);
        assert!(t.log.last().unwrap().text.contains("Picked up"));
        assert_eq!(t.seats, s.seats, "seats stay as they were");
        assert!(t.bots[1]);

        // It plays on: answer the prompt in both, and they stay the same.
        let answer =
            json!({ "type": "action", "action": {"type": "choose", "index": 0} }).to_string();
        s.handle(me, &answer);
        t.handle(me, &answer);
        assert_eq!(shown(&t), shown(&s));

        // A save that doesn't replay changes nothing.
        let before = shown(&t);
        let mut bad = saved.clone();
        bad["actions"] = json!([[1, {"type": "end_turn", "score": false}]]);
        assert!(t.resume(&bad).is_err());
        assert!(t.resume(&json!({"options": 3})).is_err());
        let manual = json!({
            "options": {"seed": 1, "automatic": false},
            "actions": [[2, {"type": "take_deck"}]],
        });
        assert!(t.resume(&manual).is_err(), "no seat 2");
        assert_eq!(shown(&t), before);

        // A finished game isn't kept, so a reload starts a new one.
        t.game.phase = Phase::Over { winner: 0 };
        assert!(t.saved_game().is_none());
    }
}
