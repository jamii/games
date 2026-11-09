//! How do bots use *Swap*? Plays games with Swap given to both players and
//! records, for every Swap, what it swapped with and how the flag count changed.
//!
//!   cargo run --release --example swap_usage -- [GAMES] [AGENT]
//!
//! AGENT is mcts:SIMSxSAMPLES (default mcts:400x4) or mctsN:SIMSxSAMPLES@NET.

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let games: u64 = args.first().map_or(20, |s| s.parse().unwrap());
    let spec = args.get(1).cloned().unwrap_or_else(|| "mcts:400x4".into());
    let d = data();
    let swap = d.powers.iter().position(|p| p.name == "Swap").unwrap();
    let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
    let mut flag_gain = 0i64;
    let mut swaps = 0;
    for seed in 0..games {
        let (mut game, _) = Game::new(Options {
            seed: Some(70000 + seed),
            given_first: vec![swap],
            ..Options::default()
        })
        .unwrap();
        let mut agents = [agent(&spec, seed * 2 + 1), agent(&spec, seed * 2 + 2)];
        let mut decisions = 0;
        while !matches!(game.phase, Phase::Over { .. }) && game.turn <= 60 && decisions < 3000 {
            let seat = to_move(&game);
            let action = agents[seat].choose(&game, seat);
            decisions += 1;
            let swapping = match &action {
                Action::Activate {
                    unit,
                    power: Some(i),
                    ..
                } => game
                    .units
                    .iter()
                    .find(|u| u.id == *unit)
                    .is_some_and(|u| u.powers[*i].power == swap),
                _ => false,
            };
            let before = game.clone();
            game.apply(seat, &action).unwrap();
            if swapping && game.pending.is_none() {
                swaps += 1;
                let flag_before = before.flag_cell();
                let moved: Vec<_> = before
                    .units
                    .iter()
                    .filter(|u| {
                        game.units
                            .iter()
                            .find(|v| v.id == u.id)
                            .is_some_and(|v| v.cell != u.cell)
                    })
                    .collect();
                let kind = if game.flag_cell() != flag_before {
                    "the flag"
                } else if moved.iter().any(|u| u.owner.is_some_and(|o| o != seat)) {
                    "an enemy unit"
                } else if moved.iter().filter(|u| u.owner == Some(seat)).count() > 1 {
                    "a friendly unit"
                } else {
                    "other"
                };
                *kinds.entry(kind).or_default() += 1;
                let margin = |g: &Game| {
                    let c = g.auto_flag_counts();
                    c[seat] as i64 - c[1 - seat] as i64
                };
                flag_gain += margin(&game) - margin(&before);
            }
        }
        eprintln!("game {seed}: {swaps} swaps so far");
    }
    println!("{swaps} swaps in {games} games ({spec}), with Swap given to the first player:");
    for (kind, n) in &kinds {
        println!(
            "  with {kind}: {n} ({:.0}%)",
            100.0 * *n as f64 / swaps.max(1) as f64
        );
    }
    println!(
        "  average change in the swapper's lead at the flag: {:+.2}",
        flag_gain as f64 / swaps.max(1) as f64
    );
}
