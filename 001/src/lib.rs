//! 001: the game's data, state and rules, used by the web server and usable by bots.
//!
//! With automatic rules, `Game::legal_actions` lists every legal action for a
//! player and `Game::apply` carries one out. Shuffles are deterministic given
//! `Options::seed`.

pub mod bot;
pub mod data;
pub mod game;
pub mod rules;
