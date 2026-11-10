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
const CHAT_LIMIT: usize = 500;

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
    Chat {
        text: String,
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
    /// Snapshots for undo, cleared whenever the turn passes or hidden cards are revealed.
    history: Vec<(Game, String)>,
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

    /// A new connection: returns its id, and the board and state to send it.
    pub fn connect(&mut self) -> (u64, Outbox) {
        let id = self.next_conn;
        self.next_conn += 1;
        self.conns.insert(id, None);
        let out = vec![(id, init_message()), (id, self.state_message(None))];
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
                match self.apply_action(seat, &action) {
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
                let Some((game, what)) = self.history.pop() else {
                    return self.error(id, "Nothing to undo.", None);
                };
                self.game = game;
                self.version += 1;
                self.push_log(
                    Some(seat),
                    vec![format!("{} undid: {what}", SEAT_NAMES[seat])],
                );
                self.broadcast()
            }

            ClientMsg::Chat { text } => {
                let text: String = text.trim().chars().take(CHAT_LIMIT).collect();
                if text.is_empty() {
                    return Vec::new();
                }
                let seat = self.seat_of(id);
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
                self.broadcast()
            }
        }
    }

    /// Carries out a player's (or the bot's) action, keeping the undo history.
    fn apply_action(&mut self, seat: Seat, action: &Action) -> Result<(), ActionError> {
        // While a prompt is waiting, the game is still the state before
        // the action, plus the prompt.
        let mut before = self.game.clone();
        before.pending = None;
        let lines = self.game.apply(seat, action)?;
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
            self.history.push((before, what));
            if self.history.len() > UNDO_LIMIT {
                self.history.remove(0);
            }
        }
        self.push_log(Some(seat), lines);
        Ok(())
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
            && let Err(e) = self.apply_action(job.seat, action)
        {
            self.push_log(None, vec![format!("The bot got stuck: {}", e.message)]);
            self.bots[job.seat] = false;
        }
        self.broadcast()
    }
}

/// The static data the client needs: the board, the cards and the rules.
pub fn init_message() -> String {
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
}
