//! Tests for the automatic rules. Board coordinates: rows y = 0..6 with
//! 4, 5, 6, 7, 6, 5, 4 spaces; x steps by 2 within a row and rows are centred.
//! The flag starts on (0, 3). Letters' home is (-1, 0), (1, 0), (0, 1);
//! Numbers' home is (0, 5), (-1, 6), (1, 6). Facing: 0 E, 1 SE, 2 SW, 3 W,
//! 4 NW, 5 NE.

use super::*;
use crate::game::{Options, next_random};

const E: usize = 0;
const SE: usize = 1;
const W: usize = 3;
const NE: usize = 5;

fn power(name: &str) -> usize {
    data()
        .powers
        .iter()
        .position(|p| p.name == name)
        .unwrap_or_else(|| panic!("no power {name}"))
}

fn c(x: i32, y: i32) -> Cell {
    Cell { x, y }
}

/// Letters' turn, main phase, empty hands, just the flag on the board.
fn game() -> Game {
    let options = Options {
        first_player: Some(0),
        seed: Some(7),
        ..Options::default()
    };
    let (mut g, _) = Game::new(options).unwrap();
    g.phase = Phase::Main;
    g.turn = 1;
    g.current = 0;
    g.hands = [Vec::new(), Vec::new()];
    g
}

fn put(g: &mut Game, owner: Option<Seat>, at: (i32, i32), facing: usize, powers: &[&str]) -> u32 {
    let powers = powers
        .iter()
        .map(|n| PowerRef {
            power: power(n),
            card: true,
        })
        .collect();
    let index = g.add_unit(owner, c(at.0, at.1), facing, powers);
    let id = g.units[index].id;
    g.sort_units();
    id
}

fn flag(g: &Game) -> u32 {
    g.flag_holder().unwrap()
}

fn at(g: &Game, id: u32) -> (i32, i32) {
    let cell = g.unit(id).cell;
    (cell.x, cell.y)
}

fn names(g: &Game, id: u32) -> Vec<&'static str> {
    g.unit(id)
        .powers
        .iter()
        .map(|p| power_name(p.power))
        .collect()
}

fn activate(g: &mut Game, id: u32, facing: usize, slot: usize) -> Result<Vec<String>, ActionError> {
    let seat = g.current;
    g.apply(
        seat,
        &Action::Activate {
            unit: id,
            facing,
            power: Some(slot),
            origin: None,
            force: false,
        },
    )
}

fn end_turn(g: &mut Game) {
    let seat = g.current;
    g.apply(seat, &Action::EndTurn { score: false }).unwrap();
}

/// Skip the other player's turn (and the draw at the start of ours).
fn pass_turns(g: &mut Game) {
    end_turn(g);
    while matches!(g.phase, Phase::Draw { .. }) {
        let seat = g.current;
        g.apply(seat, &Action::TakeDeck).unwrap();
    }
    end_turn(g);
    while matches!(g.phase, Phase::Draw { .. }) {
        let seat = g.current;
        g.apply(seat, &Action::TakeDeck).unwrap();
    }
}

fn prompt(g: &Game) -> &Prompt {
    g.prompt().expect("a prompt is waiting")
}

fn choose_where(g: &mut Game, pick: impl Fn(&Opt) -> bool) {
    let index = prompt(g)
        .options
        .iter()
        .position(pick)
        .expect("the option is offered");
    let seat = g.pending.as_ref().unwrap().seat;
    g.apply(seat, &Action::Choose { index }).unwrap();
}

fn choose_cell(g: &mut Game, x: i32, y: i32) {
    choose_where(g, |o| o.cell == Some(c(x, y)));
}

fn choose_unit(g: &mut Game, id: u32) {
    choose_where(g, |o| o.unit == Some(id));
}

fn choose_label(g: &mut Game, text: &str) {
    choose_where(g, |o| o.label.contains(text));
}

fn targets(g: &Game, id: u32) -> Vec<u32> {
    let u = g.unit(id);
    let mut t = g.targets(id, u.cell, u.facing, Some(&g.effects().mods));
    t.sort();
    t
}

#[test]
fn geometry() {
    assert_eq!(hex_dist(c(-5, 2), c(-1, 2)), 2);
    assert_eq!(hex_dist(c(0, 3), c(-3, 2)), 2);
    assert_eq!(line_dir(c(-5, 2), c(-1, 2)), Some((E, 2)));
    assert_eq!(line_dir(c(0, 3), c(-2, 5)), Some((2, 2)));
    assert_eq!(line_dir(c(0, 3), c(-3, 2)), None);
}

#[test]
fn walk_run_and_rush() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Walk"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(at(&g, a), (-3, 2));
    assert!(
        activate(&mut g, a, E, 0).is_err(),
        "only one activation per turn"
    );

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Walk"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    assert!(activate(&mut g, a, E, 0).is_err(), "blocked");
    assert_eq!(g.unit(a).activations, 0, "a failed action changes nothing");

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Run"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(at(&g, a), (-1, 2), "Run only needs the landing space empty");

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Rush"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(at(&g, a), (-2, 3), "stops before the flag");
}

#[test]
fn units_cant_enter_enemy_homes() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-1, 4), SE, &["Walk"]);
    assert!(
        activate(&mut g, a, SE, 0).is_err(),
        "(0, 5) is Numbers' home"
    );
    let a = put(&mut g, Some(0), (-1, 2), NE, &["Walk"]);
    activate(&mut g, a, 4, 0).unwrap();
    assert_eq!(at(&g, a), (-2, 1));
}

#[test]
fn push() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!((at(&g, a), at(&g, b)), ((-3, 2), (-1, 2)));

    // Blocked behind.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    put(&mut g, Some(1), (-1, 2), E, &[]);
    assert!(activate(&mut g, a, E, 0).is_err());

    // With Beam, every pushed unit needs a space behind it.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push", "Beam"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    put(&mut g, Some(1), (-1, 2), E, &[]);
    assert!(activate(&mut g, a, E, 0).is_err(), "contiguous line");
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push", "Beam"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &[]);
    let d = put(&mut g, Some(1), (1, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(
        (at(&g, a), at(&g, b), at(&g, d)),
        ((-3, 2), (-1, 2), (3, 2))
    );

    // Stubborn units can't be pushed; Spiky ones kill the pusher.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    put(&mut g, Some(1), (-3, 2), E, &["Stubborn"]);
    assert!(activate(&mut g, a, E, 0).is_err());
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &["Spiky"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(!g.alive(a));
    assert_eq!(at(&g, b), (-1, 2));
}

#[test]
fn pull_swap_jump_climb() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-1, 2), W, &["Pull"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, W, 0).unwrap();
    assert_eq!((at(&g, a), at(&g, b)), ((1, 2), (-1, 2)));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Swap"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!((at(&g, a), at(&g, b)), ((-3, 2), (-5, 2)));

    // An enemy can't be swapped into Letters' home.
    let mut g = game();
    let a = put(&mut g, Some(0), (0, 1), SE, &["Swap"]);
    put(&mut g, Some(1), (1, 2), E, &[]);
    assert!(activate(&mut g, a, SE, 0).is_err());

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Jump"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(at(&g, a), (-1, 2));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Climb"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(at(&g, a), (-3, 2));
    assert!(g.on_top(a) && !g.on_top(b));
}

#[test]
fn lift_circle_catapult_bungee_burrow() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Lift"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(g.pending.is_some(), "asks where to");
    assert_eq!(
        g.unit(a).activations,
        0,
        "nothing happens until the action is complete"
    );
    choose_cell(&mut g, -2, 1);
    assert_eq!(at(&g, b), (-2, 1));
    assert_eq!(g.unit(a).activations, 1);

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Circle"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    choose_cell(&mut g, -1, 2);
    assert_eq!(at(&g, a), (-1, 2));

    let mut g = game();
    let a = put(&mut g, Some(0), (-1, 0), E, &["Catapult"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(
        !prompt(&g)
            .options
            .iter()
            .any(|o| o.cell.is_some_and(|c| home_of(c) == Some(1)))
    );
    choose_cell(&mut g, 4, 3);
    assert_eq!(at(&g, a), (4, 3));
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Catapult"]);
    assert!(activate(&mut g, a, E, 0).is_err(), "not at home");

    let mut g = game();
    let a = put(&mut g, Some(0), (4, 3), E, &["Bungee"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(prompt(&g).options.len(), 3);
    choose_cell(&mut g, 1, 0);
    assert_eq!(at(&g, a), (1, 0));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Burrow"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(
        !prompt(&g).options.iter().any(|o| o.cell == Some(c(2, 3))),
        "next to the flag"
    );
    choose_cell(&mut g, 5, 4);
    assert_eq!(at(&g, a), (5, 4));
}

#[test]
fn actions_with_prompts_count_towards_the_turn() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Lift", "Fast"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(g.actions_this_turn, 0, "not counted until it's done");
    choose_cell(&mut g, -2, 1);
    assert_eq!(g.actions_this_turn, 1);
    g.apply(
        0,
        &Action::Activate {
            unit: a,
            facing: SE,
            power: None,
            origin: None,
            force: false,
        },
    )
    .unwrap();
    assert_eq!(g.actions_this_turn, 2);
    end_turn(&mut g);
    assert_eq!(g.actions_this_turn, 0);
}

#[test]
fn cancelling_a_prompt_changes_nothing() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Lift"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    let before = format!("{:?}", g.units);
    activate(&mut g, a, NE, 0).unwrap_err();
    activate(&mut g, a, E, 0).unwrap();
    assert!(
        g.apply(1, &Action::Choose { index: 0 }).is_err(),
        "not their choice"
    );
    assert!(
        g.apply(0, &Action::EndTurn { score: false }).is_err(),
        "answer first"
    );
    g.apply(0, &Action::Cancel).unwrap();
    assert!(g.pending.is_none());
    assert_eq!(before, format!("{:?}", g.units));
}

#[test]
fn leader_and_trail() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Walk", "Leader"]);
    let f = put(&mut g, Some(0), (-4, 3), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!((at(&g, a), at(&g, f)), ((-3, 2), (-5, 2)));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Walk", "Trail"]);
    activate(&mut g, a, E, 0).unwrap();
    let left = g.top_at(c(-5, 2)).expect("a neutral unit was left behind");
    assert!(g.unit(left).owner.is_none() && g.unit(left).powers.is_empty());

    // No trail in home spaces.
    let mut g = game();
    let a = put(&mut g, Some(0), (0, 1), SE, &["Walk", "Trail"]);
    activate(&mut g, a, SE, 0).unwrap();
    assert!(g.top_at(c(0, 1)).is_none());
}

#[test]
fn grip_jammer_and_the_flag_limit() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), SE, &["Walk"]);
    put(&mut g, Some(1), (-3, 2), W, &["Grip"]);
    assert!(
        activate(&mut g, a, SE, 0).is_err(),
        "gripped units can't move themselves"
    );

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), SE, &["Walk"]);
    put(&mut g, Some(1), (-3, 2), W, &["Jammer"]);
    assert!(
        activate(&mut g, a, SE, 0).is_err(),
        "jammed units can't use powers"
    );
    g.apply(
        0,
        &Action::Activate {
            unit: a,
            facing: E,
            power: None,
            origin: None,
            force: false,
        },
    )
    .unwrap();

    let mut g = game();
    let a = put(&mut g, Some(0), (-2, 3), E, &["Push", "Fast"]);
    let f = flag(&g);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!((at(&g, a), at(&g, f)), ((0, 3), (2, 3)));
    assert!(
        activate(&mut g, a, E, 0).is_err(),
        "the flag can only move one space per turn"
    );
    pass_turns(&mut g);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(at(&g, f), (4, 3));
}

#[test]
fn heavy_and_stinky() {
    let mut g = game();
    put(&mut g, Some(0), (-5, 4), E, &["Heavy"]);
    let e = put(&mut g, Some(1), (-1, 4), E, &[]);
    end_turn(&mut g);
    assert_eq!(at(&g, e), (-3, 4));

    let mut g = game();
    put(&mut g, Some(0), (3, 4), E, &["Stinky"]);
    let e = put(&mut g, Some(1), (2, 5), E, &[]);
    end_turn(&mut g);
    assert_eq!(at(&g, e), (1, 6), "pushed into its own home");

    // The enemy's end-of-turn powers don't trigger on our turn.
    let mut g = game();
    put(&mut g, Some(1), (3, 4), E, &["Stinky"]);
    let mine = put(&mut g, Some(0), (2, 5), E, &[]);
    end_turn(&mut g);
    assert_eq!(at(&g, mine), (2, 5));

    // Two end-of-turn powers: the player picks the order.
    let mut g = game();
    put(&mut g, Some(0), (-5, 4), E, &["Heavy"]);
    put(&mut g, Some(0), (3, 4), E, &["Stinky"]);
    put(&mut g, Some(1), (-1, 4), E, &[]);
    put(&mut g, Some(1), (2, 5), E, &[]);
    end_turn(&mut g);
    assert!(prompt(&g).text.contains("end-of-turn"));
    choose_label(&mut g, "Stinky");
    assert_eq!(g.current, 1);
}

#[test]
fn end_of_turn_log_and_flag_units() {
    // The log says which end-of-turn power moved units (the page animates it),
    // and only when it moved something.
    let mut g = game();
    put(&mut g, Some(0), (-5, 4), E, &["Heavy"]);
    put(&mut g, Some(1), (-1, 4), E, &[]);
    let log = g.apply(0, &Action::EndTurn { score: false }).unwrap();
    let at = |line: &str| log.iter().position(|l| l == line);
    assert!(
        at("A is heavy.").unwrap() < at("1 was moved.").unwrap(),
        "{log:?}"
    );

    let mut g = game();
    put(&mut g, Some(0), (5, 4), E, &["Stinky"]);
    let log = g.apply(0, &Action::EndTurn { score: false }).unwrap();
    assert!(!log.iter().any(|l| l.contains("stinky")), "{log:?}");

    // The units counted at the flag add up to the flag counts.
    let mut g = game();
    let f = g.unit(flag(&g)).cell;
    let a = put(&mut g, Some(0), (f.x - 2, f.y), E, &["Strong"]);
    let b = put(&mut g, Some(1), (f.x + 2, f.y), E, &[]);
    put(&mut g, Some(1), (f.x + 4, f.y), E, &[]);
    assert_eq!(g.auto_flag_units(), vec![(a, 0, 2), (b, 1, 1)]);
    assert_eq!(g.auto_flag_counts(), [2, 1]);
}

#[test]
fn trade_stun_trigger_mystery() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Trade", "Walk"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &["Stun"]);
    activate(&mut g, a, E, 0).unwrap();
    choose_label(&mut g, "Give Walk");
    assert_eq!(
        (names(&g, a), names(&g, b)),
        (vec!["Trade", "Stun"], vec!["Walk"])
    );

    // Can't give them a power they already have.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Trade"]);
    put(&mut g, Some(1), (-3, 2), E, &["Trade"]);
    assert!(activate(&mut g, a, E, 0).is_err());

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Stun"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &["Walk", "Push"]);
    activate(&mut g, a, E, 0).unwrap();
    choose_label(&mut g, "Push");
    assert_eq!(names(&g, b), vec!["Walk"]);
    assert!(g.discard.contains(&power("Push")));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Trigger"]);
    let b = put(&mut g, Some(1), (-3, 2), E, &["Lift"]);
    activate(&mut g, a, E, 0).unwrap();
    choose_cell(&mut g, -1, 2);
    assert_eq!(at(&g, b), (-1, 2), "a used b's Lift on b");

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Mystery"]);
    assert!(activate(&mut g, a, E, 0).is_err(), "empty hand");
    g.hands[0].push(power("Run"));
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(names(&g, a), vec!["Run"]);
    assert!(g.hands[0].is_empty() && g.discard.contains(&power("Mystery")));
}

#[test]
fn targeting_powers() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Reach"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    let far = put(&mut g, Some(1), (-1, 2), E, &[]);
    assert_eq!(targets(&g, a), vec![far]);

    let mut g = game();
    let a = put(&mut g, Some(0), (-2, 3), E, &["Spin"]);
    let n1 = put(&mut g, Some(1), (-4, 3), E, &[]);
    put(&mut g, Some(1), (-6, 3), E, &[]);
    assert_eq!(targets(&g, a), vec![flag(&g), n1]);

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Beam"]);
    let b = put(&mut g, Some(1), (-2, 3), E, &[]);
    let mut expect = vec![b, flag(&g)];
    expect.sort();
    assert_eq!(targets(&g, a), expect);

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Chain"]);
    let b = put(&mut g, Some(1), (-4, 3), E, &[]);
    let d = put(&mut g, Some(1), (-2, 3), E, &[]);
    put(&mut g, Some(1), (4, 3), E, &[]);
    assert_eq!(
        targets(&g, a),
        vec![flag(&g), b, d],
        "connected through the flag, not beyond"
    );

    let mut g = game();
    let a = put(&mut g, Some(0), (-4, 3), E, &["Halo"]);
    assert_eq!(targets(&g, a), vec![flag(&g)]);

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Colorblind"]);
    let n = put(&mut g, None, (5, 4), E, &[]);
    put(&mut g, Some(1), (3, 4), E, &[]);
    assert_eq!(targets(&g, a), vec![flag(&g), n]);

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Liability"]);
    let f = put(&mut g, Some(0), (5, 4), E, &[]);
    put(&mut g, Some(1), (3, 4), E, &[]);
    assert_eq!(targets(&g, a), vec![f]);

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Stalker"]);
    let lonely = put(&mut g, Some(1), (5, 4), E, &[]);
    put(&mut g, Some(1), (-2, 3), E, &[]);
    assert_eq!(
        targets(&g, a),
        vec![lonely],
        "units next to the flag aren't lonely"
    );

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Spotlight"]);
    let b = put(&mut g, Some(1), (-2, 3), E, &[]);
    let d = put(&mut g, Some(0), (1, 2), E, &[]);
    assert_eq!(targets(&g, a), vec![b, d]);

    let mut g = game();
    let a = put(&mut g, Some(0), (0, 1), E, &["Six-shooter"]);
    let b = put(&mut g, Some(1), (4, 1), E, &[]);
    put(&mut g, Some(1), (2, 1), W, &[]);
    let near = g.top_at(c(2, 1)).unwrap();
    let down = put(&mut g, Some(1), (2, 3), E, &[]);
    let t = targets(&g, a);
    assert!(t.contains(&near) && !t.contains(&b) && t.contains(&down));

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Vengeance"]);
    let killer = put(&mut g, Some(1), (5, 4), E, &[]);
    g.kills_last_turn.push((0, killer));
    assert_eq!(targets(&g, a), vec![killer]);

    // Bully: outnumbered units.
    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Bully"]);
    let victim = put(&mut g, Some(1), (3, 4), E, &[]);
    put(&mut g, Some(0), (5, 4), E, &[]);
    put(&mut g, Some(0), (4, 5), E, &[]);
    assert_eq!(targets(&g, a), vec![victim]);
}

#[test]
fn bridge_telepath_strong_weak_friendly_scoring() {
    let mut g = game();
    put(&mut g, Some(0), (-2, 3), E, &["Bridge"]);
    put(&mut g, Some(0), (-4, 3), E, &[]);
    put(&mut g, Some(1), (2, 3), E, &[]);
    assert_eq!(
        g.auto_flag_counts(),
        [2, 1],
        "the unit next to the bridge counts"
    );

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Telepath"]);
    let b = put(&mut g, Some(0), (5, 4), E, &[]);
    assert!(g.adjacent(a, b));

    let mut g = game();
    put(&mut g, Some(0), (-2, 3), E, &["Strong"]);
    put(&mut g, Some(1), (2, 3), E, &[]);
    put(&mut g, Some(1), (1, 4), E, &[]);
    assert_eq!(g.auto_flag_counts(), [2, 2]);
    put(&mut g, Some(1), (-4, 3), E, &["Weak"]);
    assert_eq!(
        g.auto_flag_counts(),
        [1, 2],
        "the weak unit is in front of the weakener"
    );

    let mut g = game();
    put(&mut g, Some(0), (-4, 1), SE, &["Friendly"]);
    put(&mut g, None, (-3, 2), E, &[]);
    put(&mut g, None, (-1, 2), E, &[]);
    assert_eq!(
        g.auto_flag_counts(),
        [0, 0],
        "(-3, 2) isn't next to the flag"
    );
    let mut g = game();
    put(&mut g, Some(0), (-3, 2), E, &["Friendly"]);
    put(&mut g, None, (-1, 2), E, &[]);
    assert_eq!(g.auto_flag_counts(), [1, 0]);

    // Scoring happens at the end of the turn.
    let mut g = game();
    put(&mut g, Some(0), (-2, 3), E, &[]);
    end_turn(&mut g);
    assert_eq!(g.scores, [1, 1]);
}

#[test]
fn killing_powers() {
    // Dogpile needs the target outnumbered.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Dogpile"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &[]);
    assert!(activate(&mut g, a, E, 0).is_err());
    put(&mut g, Some(0), (-4, 1), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(!g.alive(e));
    assert_eq!(g.kills_this_turn, vec![(1, a)]);

    // Backstab needs a friend on the far side.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Backstab"]);
    let e = put(&mut g, Some(1), (-3, 2), W, &["Explosive"]);
    assert!(activate(&mut g, a, E, 0).is_err());
    put(&mut g, Some(0), (-1, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(
        !g.alive(e) && !g.alive(a),
        "the explosion killed the unit in front of it"
    );

    // Click Click.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Click Click"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(g.unit(a).tokens, 1);
    pass_turns(&mut g);
    activate(&mut g, a, E, 0).unwrap();
    choose_label(&mut g, "kill");
    assert!(!g.alive(e) && g.unit(a).tokens == 0);

    // Neutralize.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Neutralize"]);
    let p = put(&mut g, Some(1), (3, 4), E, &[]);
    let n = put(&mut g, None, (5, 4), E, &[]);
    let keep = put(&mut g, Some(1), (4, 3), E, &["Walk"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(!g.alive(p) && !g.alive(n) && g.alive(keep) && g.alive(a));

    // Lifeguard.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Backstab"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &[]);
    put(&mut g, Some(0), (-1, 2), E, &[]);
    put(&mut g, Some(1), (-4, 1), SE, &["Lifeguard"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(g.alive(e));

    // Phoenix.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Backstab"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &["Phoenix"]);
    put(&mut g, Some(0), (-1, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    choose_cell(&mut g, 0, 5);
    assert_eq!(at(&g, e), (0, 5));
}

#[test]
fn killing_the_flag_brings_it_back() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-4, 3), E, &["Click Click"]);
    g.unit_mut(a).tokens = 1;
    let f = flag(&g);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(
        g.unit(a).tokens,
        2,
        "nothing to kill, so it just adds a token"
    );
    assert!(g.alive(f));
    pass_turns(&mut g);
    // Reach the flag with Beam: the space in front is empty.
    g.unit_mut(a).powers.push(PowerRef {
        power: power("Beam"),
        card: true,
    });
    activate(&mut g, a, E, 0).unwrap();
    choose_label(&mut g, "kill");
    assert!(!g.alive(f));
    let new = flag(&g);
    assert_eq!(at(&g, new), (0, 3));
}

#[test]
fn banish_split_trail_convert_and_the_unit_cap() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Banish"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    choose_cell(&mut g, 1, 6);
    assert_eq!(at(&g, e), (1, 6));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Split", "Walk"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(prompt(&g).options.len(), 4);
    choose_label(&mut g, "Give it Walk");
    let new = g.top_at(c(-3, 2)).unwrap();
    assert_eq!(
        (names(&g, a), names(&g, new)),
        (vec!["Split"], vec!["Walk"])
    );
    assert_eq!(g.unit(new).owner, Some(0));

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Convert"]);
    let n = put(&mut g, None, (-3, 2), E, &["Walk"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(g.unit(n).owner, Some(0));
    assert_eq!(g.unit(n).label, "B");

    // A sixth unit means killing one.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Convert"]);
    for x in [-5, -3, -1, 1] {
        put(&mut g, Some(0), (x, 4), E, &[]);
    }
    let n = put(&mut g, None, (-3, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(prompt(&g).text.contains("six units"));
    choose_unit(&mut g, a);
    assert!(!g.alive(a));
    assert_eq!(g.unit(n).label, "A", "the new unit takes the free label");
}

#[test]
fn flag_powers_pacifist_recycle_mine_vacuum_nuke() {
    // Pacifist: killed units go home instead.
    let mut g = game();
    put(&mut g, Some(0), (-2, 3), E, &["Pacifist"]);
    let a = put(&mut g, Some(0), (-5, 2), E, &["Backstab"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &[]);
    put(&mut g, Some(0), (-1, 2), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    choose_cell(&mut g, -1, 6);
    assert_eq!(at(&g, e), (-1, 6));

    // Recycle: my discarded powers come back to my hand.
    let mut g = game();
    put(&mut g, Some(0), (-2, 3), E, &["Recycle"]);
    let a = put(&mut g, Some(0), (-1, 0), E, &["Push"]);
    g.apply(
        0,
        &Action::RemovePower {
            unit: a,
            index: 0,
            to_hand: false,
            force: false,
        },
    )
    .unwrap();
    assert_eq!(g.hands[0], vec![power("Push")]);

    // Mine.
    let mut g = game();
    let m = put(&mut g, Some(0), (-2, 3), E, &["Mine"]);
    activate(&mut g, m, E, 0).unwrap();
    choose_label(&mut g, "deck");
    assert_eq!(g.hands[0].len(), 1);
    let faceup = g.faceup.clone();
    let next = *g.deck.last().unwrap();
    g.unit_mut(m).activations = 0;
    activate(&mut g, m, E, 0).unwrap();
    choose_label(&mut g, &format!("Take {}", power_name(faceup[1])));
    assert_eq!(g.hands[0][1], faceup[1]);
    let expected = [faceup[0], faceup[2], faceup[3], faceup[4], faceup[5], next];
    assert_eq!(g.faceup, expected, "the faceup row slides left");
    let far = put(&mut g, Some(0), (-6, 3), E, &["Mine"]);
    assert!(activate(&mut g, far, E, 0).is_err(), "not next to the flag");

    // Vacuum catches removed powers.
    let mut g = game();
    let v = put(&mut g, Some(0), (-2, 3), E, &["Vacuum"]);
    let a = put(&mut g, Some(0), (-5, 2), E, &["Stun"]);
    put(&mut g, Some(1), (-3, 2), E, &["Grip"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(names(&g, v), vec!["Vacuum", "Grip"]);

    // Nuke.
    let mut g = game();
    let n = put(&mut g, Some(0), (-2, 3), E, &["Nuke"]);
    put(&mut g, Some(1), (3, 4), E, &[]);
    for _ in 0..3 {
        activate(&mut g, n, E, 0).unwrap();
        pass_turns(&mut g);
    }
    assert_eq!(g.unit(n).tokens, 3);
    activate(&mut g, n, E, 0).unwrap();
    choose_label(&mut g, "kill every unit");
    assert_eq!(g.units.len(), 1, "only the new flag is left");
    assert!(g.units[0].is_flag());
}

#[test]
fn forbid_recall_possess_twitchy_remote() {
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Forbid"]);
    let e = put(&mut g, Some(1), (3, 4), E, &["Push"]);
    put(&mut g, Some(0), (5, 4), E, &[]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(
        !prompt(&g).options.iter().any(|o| o.label == "Walk"),
        "free powers can't be named"
    );
    choose_label(&mut g, "Push");
    end_turn(&mut g);
    while matches!(g.phase, Phase::Draw { .. }) {
        g.apply(1, &Action::TakeDeck).unwrap();
    }
    assert!(activate(&mut g, e, E, 0).is_err(), "forbidden this turn");
    end_turn(&mut g);
    assert!(g.forbidden[1].is_empty());

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Recall"]);
    let f = put(&mut g, Some(0), (-3, 2), E, &["Push", "Walk"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(!g.alive(f));
    assert_eq!(g.hands[0], vec![power("Push"), power("Walk")]);
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Recall"]);
    put(&mut g, Some(1), (-3, 2), E, &[]);
    assert!(activate(&mut g, a, E, 0).is_err(), "only friendly units");

    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Possess", "Walk"]);
    let e = put(&mut g, Some(1), (-3, 2), E, &["Stun", "Grip"]);
    activate(&mut g, a, E, 0).unwrap();
    assert!(prompt(&g).text.contains("full"));
    choose_label(&mut g, "Grip");
    assert!(!g.alive(a));
    assert_eq!(names(&g, e), vec!["Stun", "Walk", "Possess"]);

    let mut g = game();
    let t = put(&mut g, Some(1), (-3, 4), W, &["Twitchy"]);
    let a = put(&mut g, Some(0), (-5, 2), SE, &["Walk"]);
    activate(&mut g, a, SE, 0).unwrap();
    assert_eq!(at(&g, a), (-4, 3));
    assert_eq!(g.unit(t).facing, 4, "turned to face the unit that arrived");

    let mut g = game();
    let a = put(&mut g, Some(0), (-6, 3), E, &["Stun", "Remote"]);
    let friend = put(&mut g, Some(0), (3, 4), W, &[]);
    let e = put(&mut g, Some(1), (5, 4), E, &["Walk"]);
    assert!(
        activate(&mut g, a, E, 0).is_err(),
        "nothing in front of the unit itself"
    );
    g.apply(
        0,
        &Action::Activate {
            unit: a,
            facing: E,
            power: Some(0),
            origin: Some(friend),
            force: false,
        },
    )
    .unwrap();
    assert!(g.unit(e).powers.is_empty());
}

#[test]
fn spawning_applying_and_discarding() {
    let mut g = game();
    g.hands[0] = vec![power("Push")];
    g.apply(
        0,
        &Action::Spawn {
            cell: c(-1, 0),
            force: false,
        },
    )
    .unwrap();
    assert!(
        g.apply(
            0,
            &Action::Spawn {
                cell: c(1, 0),
                force: false
            }
        )
        .is_err(),
        "once per turn"
    );
    let a = g.top_at(c(-1, 0)).unwrap();
    let walk = data().walk;
    for _ in 0..3 {
        g.apply(
            0,
            &Action::ApplyFree {
                power: walk,
                unit: a,
                replace: None,
                force: false,
            },
        )
        .unwrap();
    }
    let full = Action::ApplyHand {
        card: power("Push"),
        unit: a,
        replace: None,
        force: false,
    };
    assert!(g.apply(0, &full).is_err());
    let same = Action::ApplyFree {
        power: walk,
        unit: a,
        replace: Some(0),
        force: false,
    };
    assert!(g.apply(0, &same).is_err(), "no-op");
    g.apply(
        0,
        &Action::ApplyHand {
            card: power("Push"),
            unit: a,
            replace: Some(1),
            force: false,
        },
    )
    .unwrap();
    assert_eq!(names(&g, a), vec!["Walk", "Push", "Walk"]);
    g.apply(
        0,
        &Action::RemovePower {
            unit: a,
            index: 1,
            to_hand: false,
            force: false,
        },
    )
    .unwrap();
    assert!(g.discard.contains(&power("Push")));
    let away = put(&mut g, Some(0), (4, 3), E, &["Walk"]);
    assert!(
        g.apply(
            0,
            &Action::RemovePower {
                unit: away,
                index: 0,
                to_hand: false,
                force: false
            }
        )
        .is_err()
    );
    assert!(
        g.apply(0, &Action::Kill { unit: away }).is_err(),
        "manual actions aren't allowed"
    );
}

#[test]
fn the_flag_with_an_active_power_can_be_activated_by_either_player() {
    let mut g = game();
    let f = flag(&g);
    g.unit_mut(f).powers.push(PowerRef {
        power: power("Walk"),
        card: true,
    });
    activate(&mut g, f, E, 1).unwrap();
    assert_eq!(at(&g, f), (2, 3));
    pass_turns(&mut g);
    assert!(
        g.legal_actions(0)
            .iter()
            .any(|a| matches!(a, Action::Activate { unit, .. } if *unit == f))
    );
}

#[test]
fn legal_actions_are_all_legal() {
    let mut g = game();
    g.hands[0] = vec![power("Push"), power("Lift")];
    put(&mut g, Some(0), (-1, 0), SE, &["Walk", "Lift", "Fast"]);
    put(&mut g, Some(0), (0, 1), E, &[]);
    let legal = g.legal_actions(0);
    assert!(legal.iter().any(|a| matches!(a, Action::EndTurn { .. })));
    for action in &legal {
        let mut copy = g.clone();
        copy.apply(0, action)
            .unwrap_or_else(|e| panic!("{action:?}: {}", e.message));
    }
    assert!(g.legal_actions(1).is_empty(), "not their turn");
}

// ---------------------------------------------------------------------------
// Random self-play
// ---------------------------------------------------------------------------

fn check_invariants(g: &Game) {
    let d = data();
    let mut cards: Vec<usize> = Vec::new();
    cards.extend(&g.deck);
    cards.extend(&g.discard);
    cards.extend(&g.faceup);
    cards.extend(&g.hands[0]);
    cards.extend(&g.hands[1]);
    for u in &g.units {
        cards.extend(u.powers.iter().filter(|p| p.card).map(|p| p.power));
    }
    let expected = d.powers.len() - 1 - g.free_powers.len();
    let mut unique = cards.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), cards.len(), "a card is in two places");
    assert_eq!(cards.len(), expected, "cards went missing or appeared");
    assert_eq!(
        g.units.iter().filter(|u| u.is_flag()).count(),
        1,
        "exactly one flag"
    );
    for seat in 0..2 {
        let mine: Vec<&Unit> = g.units.iter().filter(|u| u.owner == Some(seat)).collect();
        assert!(mine.len() <= 5, "too many units");
        let mut labels: Vec<&str> = mine.iter().map(|u| u.label).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), mine.len(), "duplicate labels");
        assert!(mine.iter().all(|u| u.label != "+"));
    }
    for u in &g.units {
        assert!(u.powers.len() <= MAX_POWERS, "too many powers");
        assert!(on_board(u.cell));
        if let Some(home) = home_of(u.cell) {
            assert_eq!(u.owner, Some(home), "only friendly units in a home space");
        }
    }
}

/// Play random games, choosing among the legal actions (preferring to use
/// powers), and check every legal action can be applied and nothing breaks.
fn self_play(games: u64, max_turns: u32, options: Options) -> (u32, u32) {
    let mut finished = 0;
    let mut actions = 0;
    let mut used: HashMap<&'static str, u32> = HashMap::new();
    let first = std::env::var("SELF_PLAY_FIRST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    for seed in first..first + games {
        let (mut g, _) = Game::new(Options {
            seed: Some(seed),
            ..options.clone()
        })
        .unwrap();
        let mut rng = seed.wrapping_mul(7919) + 1;
        let mut per_turn = 0;
        let mut turn = g.turn;
        while !matches!(g.phase, Phase::Over { .. }) && g.turn <= max_turns {
            let seat = g.pending.as_ref().map(|p| p.seat).unwrap_or(g.current);
            let legal = g.legal_actions(seat);
            assert!(
                !legal.is_empty(),
                "no legal actions (seed {seed}, turn {})",
                g.turn
            );
            if g.turn != turn {
                turn = g.turn;
                per_turn = 0;
            }
            per_turn += 1;
            let end = legal
                .iter()
                .position(|a| matches!(a, Action::EndTurn { .. }));
            let pick = match end {
                Some(i) if per_turn > 25 || next_random(&mut rng).is_multiple_of(6) => i,
                _ => {
                    let powers: Vec<usize> = (0..legal.len())
                        .filter(|&i| {
                            matches!(
                                legal[i],
                                Action::Activate { power: Some(_), .. } | Action::Spawn { .. }
                            )
                        })
                        .collect();
                    if !powers.is_empty() && next_random(&mut rng).is_multiple_of(2) {
                        powers[(next_random(&mut rng) % powers.len() as u64) as usize]
                    } else {
                        (next_random(&mut rng) % legal.len() as u64) as usize
                    }
                }
            };
            let action = legal[pick].clone();
            if let Action::Activate {
                unit,
                power: Some(slot),
                ..
            } = action
            {
                *used
                    .entry(power_name(g.unit(unit).powers[slot].power))
                    .or_default() += 1;
            }
            if let Err(e) = g.apply(seat, &action) {
                panic!("seed {seed}: legal action {action:?} failed: {}", e.message);
            }
            actions += 1;
            if g.pending.is_none() {
                check_invariants(&g);
            }
        }
        if matches!(g.phase, Phase::Over { .. }) {
            finished += 1;
        }
    }
    let mut used: Vec<_> = used.into_iter().collect();
    used.sort();
    println!("powers used: {used:?}");
    (finished, actions)
}

#[test]
fn random_self_play() {
    // SELF_PLAY_GAMES=500 cargo test --release random_self_play -- --nocapture
    let games = std::env::var("SELF_PLAY_GAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);
    let (finished, actions) = self_play(games, 40, Options::default());
    assert!(actions > 1000, "only {actions} actions");
    println!("{finished} of {games} games finished, {actions} actions");
}

#[test]
fn random_self_play_with_every_power_on_the_flag() {
    let options = Options {
        free_power_y: true,
        flag_power: true,
        skip_first_draw: true,
        ..Options::default()
    };
    let games = std::env::var("SELF_PLAY_GAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    self_play(games, 30, options);
}

// ---------------------------------------------------------------------------
// Hidden information
// ---------------------------------------------------------------------------

/// Play random legal actions until `turns` turns have passed.
fn random_game(seed: u64, turns: u32) -> Game {
    let (mut g, _) = Game::new(Options {
        seed: Some(seed),
        ..Options::default()
    })
    .unwrap();
    let mut rng = seed + 99;
    while g.turn < turns && !matches!(g.phase, Phase::Over { .. }) {
        let seat = g.pending.as_ref().map(|p| p.seat).unwrap_or(g.current);
        let legal = g.legal_actions(seat);
        let end = legal
            .iter()
            .position(|a| matches!(a, Action::EndTurn { .. }));
        let pick = match end {
            Some(i) if next_random(&mut rng).is_multiple_of(4) => i,
            _ => (next_random(&mut rng) % legal.len() as u64) as usize,
        };
        g.apply(seat, &legal[pick]).unwrap();
    }
    g
}

#[test]
fn faceup_takes_are_public_and_draws_are_not() {
    let options = Options {
        first_player: Some(0),
        seed: Some(3),
        ..Options::default()
    };
    let (mut g, _) = Game::new(options).unwrap();
    let card = g.faceup[1];
    g.apply(0, &Action::TakeFaceup { slot: 1 }).unwrap();
    g.apply(1, &Action::TakeDeck).unwrap();
    assert_eq!(g.known_hand(0), vec![card]);
    assert!(g.known_hand(1).is_empty());
}

#[test]
fn determinize_only_changes_what_is_hidden() {
    for seed in 0..20 {
        let g = random_game(seed, 6);
        for seat in 0..2 {
            let other = 1 - seat;
            let d = g.determinize(seat, seed * 7 + 1);
            assert_eq!(format!("{:?}", d.units), format!("{:?}", g.units));
            assert_eq!(
                (&d.faceup, &d.discard, d.scores, d.turn),
                (&g.faceup, &g.discard, g.scores, g.turn)
            );
            assert_eq!(d.hands[seat], g.hands[seat], "your own hand is known");
            assert_eq!(d.hands[other].len(), g.hands[other].len());
            assert_eq!(d.deck.len(), g.deck.len());
            for c in g.known_hand(other) {
                assert!(
                    d.hands[other].contains(&c),
                    "cards you saw them take stay in their hand"
                );
            }
            let mut before: Vec<usize> = g.deck.iter().chain(&g.hands[other]).copied().collect();
            let mut after: Vec<usize> = d.deck.iter().chain(&d.hands[other]).copied().collect();
            before.sort();
            after.sort();
            assert_eq!(before, after, "hidden cards are only shuffled around");
            assert_eq!(
                d.legal_actions(seat).len(),
                g.legal_actions(seat).len(),
                "your options don't change"
            );
        }
    }
    let g = random_game(5, 6);
    let samples: HashSet<Vec<usize>> = (0..10).map(|s| g.determinize(0, s).deck).collect();
    assert!(samples.len() > 1, "different seeds give different samples");
}

#[test]
fn given_cards_arrive_after_the_draft_and_match_removed_deals() {
    let card = power("Phoenix");
    let removed = Options {
        seed: Some(5),
        removed: vec![card],
        ..Options::default()
    };
    let given = Options {
        seed: Some(5),
        given_second: vec![card],
        ..Options::default()
    };
    let (mut a, _) = Game::new(removed).unwrap();
    let (mut b, _) = Game::new(given).unwrap();
    assert_eq!(a.deck, b.deck, "the same deal either way");
    assert_eq!(a.faceup, b.faceup);
    while a.phase == Phase::Draft {
        let seat = a.current;
        a.apply(seat, &Action::TakeFaceup { slot: 0 }).unwrap();
        b.apply(seat, &Action::TakeFaceup { slot: 0 }).unwrap();
    }
    let second = 1 - b.first_player;
    assert_eq!(b.hands[second].len(), a.hands[second].len() + 1);
    assert!(b.hands[second].contains(&card));
    assert!(
        b.known_hand(second).contains(&card),
        "both players know about it"
    );
    assert_eq!(a.hands[1 - second], b.hands[1 - second]);
}

#[test]
fn with_a_fixed_flag_no_power_moves_it() {
    for fixed in [false, true] {
        let mut g = game();
        g.options.fixed_flag = fixed;
        let flag = g.units.iter().find(|u| u.is_flag()).unwrap().id;
        let flag_cell = at(&g, flag);
        // A unit facing the flag from the west, with Swap.
        let a = put(
            &mut g,
            Some(0),
            (flag_cell.0 - 2, flag_cell.1),
            E,
            &["Swap"],
        );
        let swapped = activate(&mut g, a, E, 0).is_ok();
        assert_eq!(swapped, !fixed, "fixed flag: {fixed}");
        assert_eq!(at(&g, flag) != flag_cell, !fixed);
    }
}

/// Replays a saved game (from `GAME_DEBUG=1 selfplay`).
fn replay(json: &str) -> Game {
    let dump: serde_json::Value = serde_json::from_str(json).unwrap();
    let options: Options = serde_json::from_value(dump["options"].clone()).unwrap();
    let seed = dump["seed"].as_u64().unwrap();
    let actions: Vec<(Seat, Action)> = serde_json::from_value(dump["actions"].clone()).unwrap();
    let (mut g, _) = Game::new(Options {
        seed: Some(seed),
        ..options
    })
    .unwrap();
    for (seat, action) in actions {
        g.apply(seat, &action).unwrap();
    }
    g
}

/// Every legal action can be carried out, answering each prompt with any of
/// the options offered (here: the first and the last, all the way down).
fn check_offers_can_finish(g: &Game, seat: Seat, depth: u32) {
    for action in g.legal_actions(seat) {
        let mut next = g.clone();
        next.apply(seat, &action)
            .unwrap_or_else(|e| panic!("{action:?} was offered but failed: {}", e.message));
        if next.pending.is_some() && depth < 12 {
            let n = next.prompt().unwrap().options.len();
            for index in [0, n - 1] {
                let mut chosen = next.clone();
                chosen
                    .apply(seat, &Action::Choose { index })
                    .unwrap_or_else(|e| {
                        panic!("option {index} was offered but failed: {}", e.message)
                    });
                if chosen.pending.is_some() {
                    check_offers_can_finish(&chosen, seat, depth + 1);
                }
            }
        }
    }
}

#[test]
fn offered_prompt_options_can_always_be_finished() {
    // Lift with Chain asks where each connected unit goes: too many
    // combinations to search fully. The search used to give up by offering
    // options anyway, and some couldn't be finished.
    let g = replay(include_str!("fixtures/chain-lift.json"));
    let lift = Action::Activate {
        unit: 2,
        facing: 2,
        power: Some(1),
        origin: None,
        force: false,
    };
    let mut budget = 1_000_000;
    let possible = viable(&g, &lift, 0, Vec::new(), &mut budget);
    assert!(!possible, "a full search finds no way to finish it");
    assert!(budget > 0);
    assert!(!g.is_legal(0, &lift), "so it isn't offered");
    check_offers_can_finish(&g, 0, 0);
}

#[test]
fn with_a_fixed_flag_trade_cant_take_the_flag_power() {
    for fixed in [false, true] {
        let mut g = game();
        g.options.fixed_flag = fixed;
        let flag = g.units.iter().find(|u| u.is_flag()).unwrap().id;
        let flag_cell = at(&g, flag);
        let a = put(
            &mut g,
            Some(0),
            (flag_cell.0 - 2, flag_cell.1),
            E,
            &["Trade"],
        );
        let traded = activate(&mut g, a, E, 0).is_ok();
        assert_eq!(traded, !fixed, "fixed flag: {fixed}");
        assert_eq!(g.unit(a).is_flag(), !fixed);
    }
}

#[test]
fn buried_units_count_at_the_flag_only_in_the_variant() {
    for variant in [false, true] {
        let mut g = game();
        g.options.buried_count = variant;
        let flag = g.units.iter().find(|u| u.is_flag()).unwrap().id;
        let (fx, fy) = at(&g, flag);
        // An enemy next to the flag, with a unit of ours on top of it.
        put(&mut g, Some(1), (fx - 2, fy), E, &[]);
        put(&mut g, Some(0), (fx - 2, fy), E, &[]);
        let counts = g.auto_flag_counts();
        assert_eq!(counts[0], 1);
        assert_eq!(counts[1], if variant { 1 } else { 0 }, "variant: {variant}");
    }
}

#[test]
fn pull_with_colorblind_pulls_the_flag_one_space() {
    let mut g = game();
    let flag = g.units.iter().find(|u| u.is_flag()).unwrap().id;
    let (fx, fy) = at(&g, flag);
    // Next to the flag: the user steps back and the flag (and every other
    // neutral unit) comes into its space.
    let a = put(&mut g, Some(0), (fx - 2, fy), NE, &["Pull", "Colorblind"]);
    let n = put(&mut g, None, (fx + 4, fy + 2), E, &["Walk"]);
    activate(&mut g, a, NE, 0).unwrap();
    assert_eq!(at(&g, a), (fx - 4, fy));
    assert_eq!(at(&g, flag), (fx - 2, fy));
    assert_eq!(at(&g, n), (fx - 2, fy), "stacked on the flag");
    // Two spaces away: the flag can't move that far in one turn, and the
    // refusal says so.
    let mut g = game();
    let a = put(&mut g, Some(0), (fx - 4, fy), E, &["Pull", "Colorblind"]);
    let refusal = activate(&mut g, a, E, 0).unwrap_err().message;
    assert!(refusal.contains("one space"), "{refusal}");
}

#[test]
fn a_gripped_unit_cant_push_even_the_unit_gripping_it() {
    // Pushing the gripper away would end the grip before the pusher follows.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    let gripper = put(&mut g, Some(1), (-3, 2), W, &["Grip"]);
    let e = activate(&mut g, a, E, 0).unwrap_err();
    assert!(e.message.contains("gripped"), "{}", e.message);
    assert_eq!(g.unit(gripper).cell, c(-3, 2));
    assert!(
        !g.legal_actions(0)
            .iter()
            .any(|act| matches!(act, Action::Activate { unit, power: Some(_), .. } if *unit == a))
    );

    // Gripped from behind: the same.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    put(&mut g, Some(1), (-7, 2), E, &["Grip"]);
    put(&mut g, None, (-3, 2), E, &[]);
    assert!(activate(&mut g, a, E, 0).is_err());

    // Not gripped: it pushes and follows.
    let mut g = game();
    let a = put(&mut g, Some(0), (-5, 2), E, &["Push"]);
    put(&mut g, Some(1), (-3, 2), E, &["Grip"]);
    activate(&mut g, a, E, 0).unwrap();
    assert_eq!(g.unit(a).cell, c(-3, 2));
}
