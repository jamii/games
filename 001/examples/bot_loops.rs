//! Plays games between two copies of the web app's bot (the network-guided
//! search, with a fresh agent for every action, as `session::BotJob` does) and
//! counts the actions it wastes in its turns:
//!
//! - repeats: actions that bring the turn back to a position it was already
//!   in (eg adding the free *Walk* to a unit and discarding it again);
//! - take-backs: discarding (or replacing) a power that was put on a unit
//!   this turn, or putting back one that was taken off it, for the free
//!   powers and for cards (counted more strictly than `bot::bot_actions`
//!   judges them: even if *Vacuum* caught the power, or the unit used a power
//!   in between);
//! - capped turns: turns that ran out of actions (`bot::DEFAULT_ACTION_CAP`).
//!
//!   cargo run --release --example bot_loops -- [GAMES] [FIRST_SEED] [MAX_TURNS] [SIMULATIONS] [SAMPLES]
//!
//! Defaults: 8 games from seed 0, 30 turns, 400 simulations in 4 samples (the
//! web app's bot). Set VERBOSE=1 to print every turn with a repeat or a
//! take-back, or VERBOSE=all to print every turn.

use game_001::bot::{Agent, DEFAULT_ACTION_CAP, Evaluator, MctsAgent, MctsConfig, to_move};
use game_001::data::data;
use game_001::game::{Action, Game, Options, Phase};
use game_001::net::Net;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The position, ignoring how far through the turn it is: the action count
/// and the record of changes to powers (`Game::power_changes`) are left out.
fn key(game: &Game) -> String {
    let mut g = game.clone();
    g.actions_this_turn = 0;
    let text = format!("{g:?}");
    match text.find("power_changes: [") {
        Some(start) => {
            let end = start + text[start..].find(']').unwrap();
            format!("{}{}", &text[..start], &text[end..])
        }
        None => text,
    }
}

fn power_on(game: &Game, unit: u32, index: usize) -> Option<(usize, bool)> {
    let u = game.units.iter().find(|u| u.id == unit)?;
    u.powers.get(index).map(|p| (p.power, p.card))
}

fn describe(game: &Game, action: &Action) -> String {
    let name = |power: usize| data().powers[power].name.clone();
    match *action {
        Action::ApplyFree { power, unit, .. } => format!("add free {} to #{unit}", name(power)),
        Action::ApplyHand { card, unit, .. } => format!("add {} to #{unit}", name(card)),
        Action::RemovePower { unit, index, .. } => match power_on(game, unit, index) {
            Some((power, _)) => format!("discard {} from #{unit}", name(power)),
            None => format!("{action:?}"),
        },
        Action::Activate {
            unit,
            facing,
            power,
            ..
        } => match power.and_then(|i| power_on(game, unit, i)) {
            Some((power, _)) => format!("activate #{unit} facing {facing} ({})", name(power)),
            None => format!("turn #{unit} to face {facing}"),
        },
        Action::EndTurn { .. } => "end turn".into(),
        _ => format!("{action:?}"),
    }
}

#[derive(Default, Clone, Copy)]
struct Counts {
    games: u32,
    finished: u32,
    turns: u32,
    actions: u32,
    repeats: u32,
    repeat_turns: u32,
    free_changes: u32,
    take_backs: u32,
    card_take_backs: u32,
    capped_turns: u32,
    decisions: u32,
    thinking: f64,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.games += o.games;
        self.finished += o.finished;
        self.turns += o.turns;
        self.actions += o.actions;
        self.repeats += o.repeats;
        self.repeat_turns += o.repeat_turns;
        self.free_changes += o.free_changes;
        self.take_backs += o.take_backs;
        self.card_take_backs += o.card_take_backs;
        self.capped_turns += o.capped_turns;
        self.decisions += o.decisions;
        self.thinking += o.thinking;
    }
}

fn all_turns() -> bool {
    std::env::var("VERBOSE").is_ok_and(|v| v == "all")
}

/// One game; returns its counts and the text of the turns worth printing.
fn play(seed: u64, max_turns: u32, config: &MctsConfig) -> (Counts, Vec<String>) {
    let (mut game, _) = Game::new(Options {
        seed: Some(seed),
        ..Options::default()
    })
    .unwrap();
    let mut counts = Counts {
        games: 1,
        ..Counts::default()
    };
    let mut shown = Vec::new();
    // This turn: the positions seen, the changes to powers, and what happened.
    let mut seen: Vec<String> = Vec::new();
    let mut changes: Vec<(u32, usize, bool)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let (mut repeats, mut take_backs, mut card_take_backs, mut main_actions) = (0, 0, 0, 0);
    let mut version = seed << 20;
    let mut decisions = 0;
    while !matches!(game.phase, Phase::Over { .. }) && game.turn <= max_turns && decisions < 5000 {
        decisions += 1;
        let seat = to_move(&game);
        let (turn, current) = (game.turn, game.current);
        // A fresh agent for every action, seeded like the web app's.
        let started = std::time::Instant::now();
        let action = MctsAgent::new(config.clone(), version).choose(&game, seat);
        counts.thinking += started.elapsed().as_secs_f64();
        counts.decisions += 1;
        version += 1;
        let main = game.phase == Phase::Main && game.pending.is_none();
        if main {
            if seen.is_empty() {
                seen.push(key(&game));
            }
            main_actions += 1;
            lines.push(describe(&game, &action));
            // (unit, power, put on, is a card) for each power put on or taken off.
            let mut change: Vec<(u32, usize, bool, bool)> = Vec::new();
            let taken_off = |unit: u32, index: Option<usize>| {
                index
                    .and_then(|i| power_on(&game, unit, i))
                    .map(|(power, card)| (unit, power, false, card))
            };
            match action {
                Action::ApplyFree {
                    power,
                    unit,
                    replace,
                    ..
                } => change.extend(
                    [Some((unit, power, true, false)), taken_off(unit, replace)]
                        .into_iter()
                        .flatten(),
                ),
                Action::ApplyHand {
                    card,
                    unit,
                    replace,
                    ..
                } => change.extend(
                    [Some((unit, card, true, true)), taken_off(unit, replace)]
                        .into_iter()
                        .flatten(),
                ),
                Action::RemovePower { unit, index, .. } => {
                    change.extend(taken_off(unit, Some(index)))
                }
                _ => {}
            }
            for (unit, power, added, card) in change {
                counts.free_changes += !card as u32;
                if changes.contains(&(unit, power, !added)) {
                    *if card {
                        &mut card_take_backs
                    } else {
                        &mut take_backs
                    } += 1;
                    lines.push("    ^ takes back an earlier change".into());
                }
                changes.push((unit, power, added));
            }
        }
        game.apply(seat, &action).unwrap();
        let same_turn = (game.turn, game.current) == (turn, current);
        if same_turn && game.phase == Phase::Main && game.pending.is_none() && !seen.is_empty() {
            let k = key(&game);
            if seen.contains(&k) {
                repeats += 1;
                lines.push("    ^ back to an earlier position".into());
            }
            seen.push(k);
        }
        if !same_turn || matches!(game.phase, Phase::Over { .. }) {
            if main_actions > 0 {
                counts.turns += 1;
                counts.actions += main_actions;
                counts.repeats += repeats;
                counts.take_backs += take_backs;
                counts.card_take_backs += card_take_backs;
                counts.repeat_turns += (repeats > 0) as u32;
                // The last action of a capped turn is the forced end of turn.
                counts.capped_turns += (main_actions > DEFAULT_ACTION_CAP) as u32;
                if repeats + take_backs + card_take_backs > 0 || all_turns() {
                    shown.push(format!(
                        "seed {seed}, turn {turn}, seat {current}:\n  {}",
                        lines.join("\n  ")
                    ));
                }
            }
            seen.clear();
            changes.clear();
            lines.clear();
            (repeats, take_backs, card_take_backs, main_actions) = (0, 0, 0, 0);
        }
    }
    counts.finished = matches!(game.phase, Phase::Over { .. }) as u32;
    (counts, shown)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize, default: u64| args.get(i).map_or(default, |s| s.parse().unwrap());
    let (games, first, max_turns) = (arg(1, 8), arg(2, 0), arg(3, 30) as u32);
    let config = MctsConfig {
        simulations: arg(4, 400) as u32,
        samples: arg(5, 4) as u32,
        eval: Evaluator::Net(Arc::new(Net::load("bots/net.json").unwrap())),
        ..MctsConfig::default()
    };
    let verbose = std::env::var_os("VERBOSE").is_some();
    let next = Arc::new(AtomicU64::new(first));
    let total = Arc::new(Mutex::new(Counts::default()));
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (next, total, config) = (next.clone(), total.clone(), config.clone());
            std::thread::spawn(move || loop {
                let seed = next.fetch_add(1, Ordering::Relaxed);
                if seed >= first + games {
                    break;
                }
                let (c, shown) = play(seed, max_turns, &config);
                if verbose {
                    for turn in shown {
                        eprintln!("{turn}");
                    }
                }
                println!(
                    "seed {seed}: {} turns, {} actions, {} repeats, {} take-backs, {} capped turns",
                    c.turns, c.actions, c.repeats, c.take_backs + c.card_take_backs, c.capped_turns
                );
                total.lock().unwrap().add(&c);
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let t = *total.lock().unwrap();
    println!(
        "{} games ({} finished), {} turns, {} actions in them ({:.1} a turn)\n\
         repeats: {} ({} turns)\n\
         free-power changes: {}, take-backs: {} (and {} of cards)\n\
         turns that hit the {}-action cap: {}\n\
         thinking: {:.1} ms a decision",
        t.games,
        t.finished,
        t.turns,
        t.actions,
        t.actions as f64 / t.turns.max(1) as f64,
        t.repeats,
        t.repeat_turns,
        t.free_changes,
        t.take_backs,
        t.card_take_backs,
        DEFAULT_ACTION_CAP,
        t.capped_turns,
        t.thinking * 1000.0 / t.decisions.max(1) as f64,
    );
}
