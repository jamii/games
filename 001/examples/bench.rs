//! Measures how fast the engine plays random games: time spent listing legal
//! actions versus applying them.
//!
//!   cargo run --release --example bench [games]
use game_001::game::{Game, Options, Phase};
use std::time::{Duration, Instant};

fn main() {
    let games: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let (mut legal_time, mut apply_time) = (Duration::ZERO, Duration::ZERO);
    let (mut decisions, mut legal_total) = (0u64, 0u64);
    let mut rng = 12345u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for seed in 0..games {
        let (mut g, _) = Game::new(Options {
            seed: Some(seed),
            ..Options::default()
        })
        .unwrap();
        let mut per_turn = 0;
        let mut turn = g.turn;
        while !matches!(g.phase, Phase::Over { .. }) && g.turn <= 40 {
            let seat = g.pending.as_ref().map(|p| p.seat).unwrap_or(g.current);
            let t = Instant::now();
            let legal = g.legal_actions(seat);
            legal_time += t.elapsed();
            legal_total += legal.len() as u64;
            if g.turn != turn {
                turn = g.turn;
                per_turn = 0;
            }
            per_turn += 1;
            let end = legal
                .iter()
                .position(|a| matches!(a, game_001::game::Action::EndTurn { .. }));
            let pick = match end {
                Some(i) if per_turn > 20 || next() % 6 == 0 => i,
                _ => (next() % legal.len() as u64) as usize,
            };
            let t = Instant::now();
            g.apply(seat, &legal[pick]).unwrap();
            apply_time += t.elapsed();
            decisions += 1;
        }
    }
    let total = legal_time + apply_time;
    println!(
        "{decisions} decisions in {:.2}s: {:.0} decisions/s, legal_actions {:.1}µs avg ({:.0} actions avg), apply {:.1}µs avg",
        total.as_secs_f64(),
        decisions as f64 / total.as_secs_f64(),
        legal_time.as_micros() as f64 / decisions as f64,
        legal_total as f64 / decisions as f64,
        apply_time.as_micros() as f64 / decisions as f64,
    );
}
