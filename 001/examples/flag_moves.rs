//! What moves the flag? Plays games with CARD given to the first player and
//! records every action that moves the flag: which power did it (or the end of
//! the turn, for passive powers like *Heavy*), and how it changed the mover's
//! lead at the flag. Also reports how CARD itself was used (if it's an active
//! power): whether it moved the flag, buried or moved an enemy unit, and how
//! it changed the user's lead at the flag.
//!
//!   cargo run --release --example flag_moves -- [CARD] [GAMES] [AGENT]
//!
//! Defaults: Swap, 20 games, mcts:400x4. AGENT can also be
//! mctsN:SIMSxSAMPLES@NET.

use game_001::bot::{Agent, Evaluator, MctsAgent, MctsConfig, to_move};
use game_001::data::data;
use game_001::game::{Action, Game, Options, Phase};
use game_001::net::Net;
use std::collections::BTreeMap;
use std::sync::Arc;

fn agent(spec: &str, seed: u64) -> MctsAgent {
    let (spec, net) = match spec.split_once('@') {
        Some((s, path)) => (s, Some(Arc::new(Net::load(path).expect("net")))),
        None => (spec, None),
    };
    let numbers = spec.split_once(':').map_or("400x4", |(_, n)| n);
    let (sims, samples) = numbers.split_once('x').unwrap_or((numbers, "4"));
    let mut config = MctsConfig {
        simulations: sims.parse().unwrap(),
        samples: samples.parse().unwrap(),
        ..MctsConfig::default()
    };
    if let Some(net) = net {
        config.eval = Evaluator::Net(net);
    }
    MctsAgent::new(config, seed)
}

#[derive(Default)]
struct Tally {
    moves: u32,
    gain: i64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let d = data();
    let name = args.first().cloned().unwrap_or_else(|| "Swap".into());
    let card = d
        .powers
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(&name))
        .unwrap_or_else(|| panic!("no card called {name}"));
    let games: u64 = args.get(1).map_or(20, |s| s.parse().unwrap());
    let spec = args.get(2).cloned().unwrap_or_else(|| "mcts:400x4".into());
    let mut causes: BTreeMap<String, Tally> = BTreeMap::new();
    let mut uses: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut turns = 0;
    for seed in 0..games {
        let (mut game, _) = Game::new(Options {
            seed: Some(70000 + seed),
            given_first: vec![card],
            ..Options::default()
        })
        .unwrap();
        let mut agents = [agent(&spec, seed * 2 + 1), agent(&spec, seed * 2 + 2)];
        let mut decisions = 0;
        while !matches!(game.phase, Phase::Over { .. }) && game.turn <= 60 && decisions < 3000 {
            let seat = to_move(&game);
            let action = agents[seat].choose(&game, seat);
            decisions += 1;
            // The power being used: this action's, or (for an answer to a
            // prompt) the power of the action waiting for it.
            let activation = match (&action, &game.pending) {
                (Action::Choose { .. }, Some(p)) => p.action().clone(),
                _ => action.clone(),
            };
            let power = match &activation {
                Action::Activate {
                    unit,
                    power: Some(i),
                    ..
                } => game
                    .pending
                    .as_ref()
                    .map_or(&game, |p| &*p.preview)
                    .units
                    .iter()
                    .find(|u| u.id == *unit)
                    .and_then(|u| u.powers.get(*i))
                    .map(|p| p.power),
                _ => None,
            };
            let before = game.clone();
            game.apply(seat, &action).unwrap();
            if game.pending.is_some() {
                continue;
            }
            // Count once per finished action, from the state before it began.
            let start = before.pending.as_ref().map_or(&before, |_| &before);
            let margin = |g: &Game| {
                let c = g.auto_flag_counts();
                c[seat] as i64 - c[1 - seat] as i64
            };
            let flag_moved = game.flag_cell() != start.flag_cell();
            if power == Some(card) {
                let tops = |g: &Game| g.top_units().iter().map(|u| u.id).collect::<Vec<_>>();
                let (top_before, top_after) = (tops(start), tops(&game));
                let enemy = |id: &u32| {
                    start
                        .units
                        .iter()
                        .find(|u| u.id == *id)
                        .is_some_and(|u| u.owner.is_some_and(|o| o != seat))
                };
                let buried_enemy = top_before.iter().any(|id| {
                    enemy(id) && !top_after.contains(id) && game.units.iter().any(|u| u.id == *id)
                });
                let moved_enemy = start.units.iter().any(|u| {
                    enemy(&u.id)
                        && game
                            .units
                            .iter()
                            .find(|v| v.id == u.id)
                            .is_some_and(|v| v.cell != u.cell)
                });
                let kind = if flag_moved {
                    "moved the flag"
                } else if buried_enemy {
                    "buried an enemy unit"
                } else if moved_enemy {
                    "moved an enemy unit"
                } else {
                    "other"
                };
                let t = uses.entry(kind).or_default();
                t.moves += 1;
                t.gain += margin(&game) - margin(start);
            }
            if matches!(action, Action::EndTurn { .. }) {
                turns += 1;
            }
            if flag_moved {
                let cause = match (&action, power) {
                    (Action::EndTurn { .. }, _) => "end of turn (passive powers)".to_string(),
                    (_, Some(p)) => d.powers[p].name.clone(),
                    _ => format!("{action:?}")
                        .split([' ', '{'])
                        .next()
                        .unwrap()
                        .to_string(),
                };
                // The end of the turn is scored after the margin changes, so
                // judge it by the player who just ended their turn.
                let t = causes.entry(cause).or_default();
                t.moves += 1;
                t.gain += margin(&game) - margin(start);
            }
        }
        eprintln!("game {seed} done");
    }
    println!(
        "{games} games ({spec}), {} given to the first player, {turns} turns:",
        d.powers[card].name
    );
    println!("  what moved the flag            times  avg change in the mover's flag lead");
    for (cause, t) in &causes {
        println!(
            "  {cause:<30} {:>5}  {:+.2}",
            t.moves,
            t.gain as f64 / t.moves.max(1) as f64
        );
    }
    let total: u32 = uses.values().map(|t| t.moves).sum();
    if total > 0 {
        println!("  {} was used {total} times:", d.powers[card].name);
        for (kind, t) in &uses {
            println!(
                "    {kind:<22} {:>4} ({:.0}%), avg change in the user's flag lead {:+.2}",
                t.moves,
                100.0 * t.moves as f64 / total as f64,
                t.gain as f64 / t.moves.max(1) as f64
            );
        }
    }
}
