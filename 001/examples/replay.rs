//! Replays a game saved by `GAME_DEBUG=1 selfplay` after an engine bug, and
//! shows the position before the last action: the prompt, its options, and
//! whether each option can be applied.
//!
//!   cargo run --release --example replay -- bug-seed-N.json

use game_001::data::data;
use game_001::game::{Action, Game, Options};

fn main() {
    let path = std::env::args().nth(1).expect("usage: replay FILE");
    let dump: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let options: Options = serde_json::from_value(dump["options"].clone()).unwrap();
    let seed = dump["seed"].as_u64().unwrap();
    let actions: Vec<(usize, Action)> = serde_json::from_value(dump["actions"].clone()).unwrap();
    let (mut game, _) = Game::new(Options {
        seed: Some(seed),
        ..options
    })
    .unwrap();
    let (last, rest) = actions.split_last().unwrap();
    for (seat, action) in rest {
        let log = game.apply(*seat, action).unwrap();
        for line in log {
            println!("  {line}");
        }
    }
    let d = data();
    println!("\nunits:");
    for u in &game.units {
        let powers: Vec<&str> = u
            .powers
            .iter()
            .map(|p| d.powers[p.power].name.as_str())
            .collect();
        println!(
            "  {} {:?} owner {:?} at ({}, {}) facing {} z {} {:?}",
            u.id, u.label, u.owner, u.cell.x, u.cell.y, u.facing, u.z, powers
        );
    }
    if let Some(p) = &game.pending {
        println!("\npending {:?}, answers {:?}", p.action(), p.answers());
        println!("prompt: {}", p.prompt.text);
        for (i, o) in p.prompt.options.iter().enumerate() {
            let ok = game.clone().apply(last.0, &Action::Choose { index: i });
            println!(
                "  {i}: {} {:?} {:?} {:?} -> {:?}",
                o.label,
                o.cell,
                o.unit,
                o.power,
                ok.map(|_| "ok").map_err(|e| e.message)
            );
        }
    }
    println!(
        "\nlast action {:?} by {}: {:?}",
        last.1,
        last.0,
        game.clone().apply(last.0, &last.1).map_err(|e| e.message)
    );
}
