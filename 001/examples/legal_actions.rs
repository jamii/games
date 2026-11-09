//! The example from the README: play a few random legal actions.
use game_001::game::{Game, Options};

fn main() {
    let (mut game, _log) = Game::new(Options {
        seed: Some(1),
        ..Options::default()
    })
    .unwrap();
    for _ in 0..20 {
        let seat = game
            .pending
            .as_ref()
            .map(|p| p.seat)
            .unwrap_or(game.current);
        let actions = game.legal_actions(seat);
        game.apply(seat, &actions[0]).unwrap();
    }
    println!("turn {}, phase {:?}", game.turn, game.phase);
}
