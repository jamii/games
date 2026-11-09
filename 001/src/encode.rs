//! Encodings of positions and actions for a neural network (stage 2 of
//! BOT.md).
//!
//! A position is encoded from one player's point of view, using only what that
//! player can see, as a set of tokens: one for the game as a whole and one per
//! unit. Each legal action is encoded as another token, plus pointers to the
//! units it involves. Tokens are sparse: a list of feature ids, each with a
//! value (mostly 1). The network turns each token into a vector by adding up
//! one learned vector per feature (an embedding bag), so categorical facts
//! (which card, which space) and numbers (scores) are treated the same way.
//!
//! The board is turned around for the second player, so both players see
//! their own home at the top and the network only has to learn one side.
//!
//! Feature ids are laid out by [`Layout`], which is also written next to
//! trained weights so a mismatch (eg after adding cards) is caught when they
//! are loaded.

use crate::data::{Cell, CellKind, Seat, data};
use crate::game::{Action, Game, Phase, Unit};
use serde::{Deserialize, Serialize};

/// A sparse token: feature ids and their values. (The short names keep
/// training files small.)
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Token {
    #[serde(rename = "i")]
    pub ids: Vec<u32>,
    #[serde(rename = "v")]
    pub vals: Vec<f32>,
}

impl Token {
    fn one(&mut self, id: usize) {
        self.ids.push(id as u32);
        self.vals.push(1.0);
    }
    fn val(&mut self, id: usize, v: f32) {
        if v != 0.0 {
            self.ids.push(id as u32);
            self.vals.push(v);
        }
    }
    /// A one-hot over `n` buckets starting at `base`, clamping `i`.
    fn bucket(&mut self, base: usize, n: usize, i: usize) {
        self.one(base + i.min(n - 1));
    }
}

/// A position from one player's point of view.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Observation {
    #[serde(rename = "g")]
    pub global: Token,
    #[serde(rename = "u")]
    pub units: Vec<Token>,
    /// Unit ids, in the order of `units` (for action pointers).
    #[serde(skip)]
    pub unit_ids: Vec<u32>,
}

/// An action: its own features, and the units it involves (indexes into
/// `Observation::units`, or -1).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ActionToken {
    #[serde(rename = "t")]
    pub token: Token,
    #[serde(rename = "u")]
    pub unit: i32,
    #[serde(rename = "g")]
    pub target: i32,
}

/// Sizes of the feature vocabularies. Grows when cards or the board change.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Layout {
    pub cards: usize,
    pub cells: usize,
    pub global: usize,
    pub unit: usize,
    pub action: usize,
}

const MAX_HAND: usize = 10;
const MAX_ACTIONS_IN_TURN: usize = 17;
const TEXT_BUCKETS: usize = 32;
const MAX_CHOICE: usize = 10;
const ACTION_KINDS: usize = 11;

/// Offsets of the feature groups. Computed once from the number of cards
/// and spaces.
struct Offsets {
    cards: usize,
    cells: usize,
    // Global.
    g_phase: usize,
    g_flags: usize,
    g_need_me: usize,
    g_need_them: usize,
    g_scalars: usize,
    g_actions: usize,
    g_draws: usize,
    g_hand_me: usize,
    g_hand_them: usize,
    g_units_me: usize,
    g_units_them: usize,
    g_my_hand: usize,
    g_their_known: usize,
    g_faceup: usize,
    g_discard: usize,
    g_free: usize,
    g_forbid_me: usize,
    g_forbid_them: usize,
    g_removed: usize,
    g_pending_kind: usize,
    g_pending_power: usize,
    g_prompt_text: usize,
    g_answers: usize,
    g_end: usize,
    // Unit.
    u_owner: usize,
    u_flags: usize,
    u_cell: usize,
    u_dist: usize,
    u_facing: usize,
    u_front: usize,
    u_depth: usize,
    u_cards: usize,
    u_free: usize,
    u_tokens: usize,
    u_used: usize,
    u_moved: usize,
    u_enemies: usize,
    u_friends: usize,
    u_end: usize,
    // Action.
    a_kind: usize,
    a_power: usize,
    a_replace: usize,
    a_cell: usize,
    a_facing: usize,
    a_turn: usize,
    a_slot: usize,
    a_choice: usize,
    a_text: usize,
    a_mentions: usize,
    a_flags: usize,
    a_end: usize,
}

fn offsets() -> &'static Offsets {
    static OFFSETS: std::sync::OnceLock<Offsets> = std::sync::OnceLock::new();
    OFFSETS.get_or_init(|| {
        let d = data();
        let cards = d.powers.len();
        let cells = d.board.cells.len();
        let mut next = 0;
        let mut take = |n: usize| {
            let at = next;
            next += n;
            at
        };
        // Global (id 0 is always on, as a bias).
        take(1);
        let g_phase = take(4);
        // my turn, I went first, spawned this turn, prompt waiting
        let g_flags = take(4);
        let g_need_me = take(8);
        let g_need_them = take(8);
        // the hand-made evaluation's features, turn, deck and discard sizes,
        // unseen cards in their hand, second player's starting points
        let g_scalars = take(crate::bot::BASE_FEATURES.len() + 5);
        let g_actions = take(MAX_ACTIONS_IN_TURN);
        let g_draws = take(4);
        let g_hand_me = take(MAX_HAND + 1);
        let g_hand_them = take(MAX_HAND + 1);
        let g_units_me = take(7);
        let g_units_them = take(7);
        let g_my_hand = take(cards);
        let g_their_known = take(cards);
        let g_faceup = take(cards);
        let g_discard = take(cards);
        let g_free = take(cards);
        let g_forbid_me = take(cards);
        let g_forbid_them = take(cards);
        let g_removed = take(cards);
        let g_pending_kind = take(ACTION_KINDS);
        let g_pending_power = take(cards);
        let g_prompt_text = take(TEXT_BUCKETS);
        let g_answers = take(6);
        let g_end = next;

        let mut next = 0;
        let mut take = |n: usize| {
            let at = next;
            next += n;
            at
        };
        take(1);
        let u_owner = take(3);
        // flag, new sixth unit, on top, can still activate, my home, their home
        let u_flags = take(6);
        let u_cell = take(cells);
        let u_dist = take(5);
        let u_facing = take(6);
        let u_front = take(5);
        let u_depth = take(4);
        let u_cards = take(cards);
        let u_free = take(cards);
        let u_tokens = take(6);
        let u_used = take(4);
        let u_moved = take(4);
        let u_enemies = take(7);
        let u_friends = take(7);
        let u_end = next;

        let mut next = 0;
        let mut take = |n: usize| {
            let at = next;
            next += n;
            at
        };
        take(1);
        let a_kind = take(ACTION_KINDS);
        let a_power = take(cards);
        let a_replace = take(cards);
        let a_cell = take(cells);
        let a_facing = take(6);
        let a_turn = take(6);
        let a_slot = take(8);
        let a_choice = take(MAX_CHOICE);
        let a_text = take(TEXT_BUCKETS);
        let a_mentions = take(cards);
        // uses another unit's space (Remote), option names a unit, a space, a power
        let a_flags = take(4);
        let a_end = next;
        Offsets {
            cards,
            cells,
            g_phase,
            g_flags,
            g_need_me,
            g_need_them,
            g_scalars,
            g_actions,
            g_draws,
            g_hand_me,
            g_hand_them,
            g_units_me,
            g_units_them,
            g_my_hand,
            g_their_known,
            g_faceup,
            g_discard,
            g_free,
            g_forbid_me,
            g_forbid_them,
            g_removed,
            g_pending_kind,
            g_pending_power,
            g_prompt_text,
            g_answers,
            g_end,
            u_owner,
            u_flags,
            u_cell,
            u_dist,
            u_facing,
            u_front,
            u_depth,
            u_cards,
            u_free,
            u_tokens,
            u_used,
            u_moved,
            u_enemies,
            u_friends,
            u_end,
            a_kind,
            a_power,
            a_replace,
            a_cell,
            a_facing,
            a_turn,
            a_slot,
            a_choice,
            a_text,
            a_mentions,
            a_flags,
            a_end,
        }
    })
}

pub fn layout() -> Layout {
    let o = offsets();
    Layout {
        cards: o.cards,
        cells: o.cells,
        global: o.g_end,
        unit: o.u_end,
        action: o.a_end,
    }
}

/// Turns the board around for seat 1, so both seats see their home at the top.
fn canonical(cell: Cell, seat: Seat) -> Cell {
    if seat == 0 {
        cell
    } else {
        let c = data().board.flag;
        Cell {
            x: 2 * c.x - cell.x,
            y: 2 * c.y - cell.y,
        }
    }
}

fn canonical_facing(facing: usize, seat: Seat) -> usize {
    if seat == 0 {
        facing % 6
    } else {
        (facing + 3) % 6
    }
}

fn cell_index(cell: Cell, seat: Seat) -> Option<usize> {
    let c = canonical(cell, seat);
    data().board.cells.iter().position(|&(x, _)| x == c)
}

fn hash_text(text: &str) -> usize {
    // FNV-1a.
    let mut h: u32 = 0x811c_9dc5;
    for b in text.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h as usize % TEXT_BUCKETS
}

/// The fixed part of a prompt or option text: unit names, space names and
/// power names vary, so keep only the words before a colon, or else the
/// lowercase words.
fn text_key(text: &str) -> String {
    if let Some((head, _)) = text.split_once(':') {
        return head.to_string();
    }
    text.split_whitespace()
        .filter(|w| w.chars().next().is_some_and(|c| c.is_lowercase()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn action_kind(action: &Action) -> usize {
    match action {
        Action::TakeFaceup { .. } => 0,
        Action::TakeDeck => 1,
        Action::Spawn { .. } => 2,
        Action::ApplyHand { .. } => 3,
        Action::ApplyFree { .. } => 4,
        Action::Activate { .. } => 5,
        Action::RemovePower { .. } => 6,
        Action::EndTurn { .. } => 7,
        Action::Choose { .. } => 8,
        Action::Cancel => 9,
        _ => 10,
    }
}

/// The board as `seat` should judge it: while a prompt waits, the game as it
/// is at the moment of the question.
fn shown(game: &Game) -> &Game {
    game.pending.as_ref().map(|p| &*p.preview).unwrap_or(game)
}

fn power_of(game: &Game, unit: u32, index: usize) -> Option<usize> {
    game.units
        .iter()
        .find(|u| u.id == unit)
        .and_then(|u| u.powers.get(index))
        .map(|p| p.power)
}

pub fn observe(game: &Game, seat: Seat) -> Observation {
    let o = offsets();
    let d = data();
    let g = shown(game);
    let them = 1 - seat;
    let mut t = Token::default();
    t.one(0);
    t.one(
        o.g_phase
            + match g.phase {
                Phase::Draft => 0,
                Phase::Draw { .. } => 1,
                Phase::Main => 2,
                Phase::Over { .. } => 3,
            },
    );
    if g.current == seat {
        t.one(o.g_flags);
    }
    if g.first_player == seat {
        t.one(o.g_flags + 1);
    }
    if g.spawned_this_turn {
        t.one(o.g_flags + 2);
    }
    if game.pending.is_some() {
        t.one(o.g_flags + 3);
    }
    let target = g.options.target_score;
    t.bucket(
        o.g_need_me,
        8,
        target.saturating_sub(g.scores[seat]) as usize,
    );
    t.bucket(
        o.g_need_them,
        8,
        target.saturating_sub(g.scores[them]) as usize,
    );
    let base = crate::bot::BASE_FEATURES.len();
    if let Ok(f) = crate::bot::features(game, seat) {
        for (i, v) in f[..base].iter().enumerate() {
            t.val(o.g_scalars + i, *v / 3.0);
        }
    }
    t.val(o.g_scalars + base, g.turn as f32 / 30.0);
    t.val(o.g_scalars + base + 1, g.deck.len() as f32 / 30.0);
    t.val(o.g_scalars + base + 2, g.discard.len() as f32 / 30.0);
    let known = g.known_hand(them);
    t.val(
        o.g_scalars + base + 3,
        g.hands[them].len().saturating_sub(known.len()) as f32 / 5.0,
    );
    let second_points = g.options.second_player_points as f32;
    t.val(o.g_scalars + base + 4, second_points);
    t.bucket(
        o.g_actions,
        MAX_ACTIONS_IN_TURN,
        g.actions_this_turn as usize,
    );
    if let Phase::Draw { remaining } = g.phase {
        t.bucket(o.g_draws, 4, remaining as usize);
    }
    t.bucket(o.g_hand_me, MAX_HAND + 1, g.hands[seat].len());
    t.bucket(o.g_hand_them, MAX_HAND + 1, g.hands[them].len());
    let mut counts = |base: usize, cards: &[usize]| {
        let mut c = vec![0u32; o.cards];
        for &card in cards {
            c[card] += 1;
        }
        for (card, &n) in c.iter().enumerate() {
            t.val(base + card, n as f32);
        }
    };
    counts(o.g_my_hand, &g.hands[seat]);
    counts(o.g_their_known, &known);
    counts(o.g_faceup, &g.faceup);
    counts(o.g_discard, &g.discard);
    counts(o.g_free, &g.free_powers);
    counts(o.g_forbid_me, &g.forbidden[seat]);
    counts(o.g_forbid_them, &g.forbidden[them]);
    counts(o.g_removed, &g.options.removed);
    if let Some(p) = &game.pending {
        let action = p.action();
        t.one(o.g_pending_kind + action_kind(action));
        let power = match action {
            Action::ApplyHand { card, .. } => Some(*card),
            Action::ApplyFree { power, .. } => Some(*power),
            Action::Activate {
                unit,
                power: Some(i),
                ..
            } => power_of(&p.base, *unit, *i),
            Action::TakeFaceup { slot } => p.base.faceup.get(*slot).copied(),
            _ => None,
        };
        if let Some(power) = power {
            t.one(o.g_pending_power + power);
        }
        t.one(o.g_prompt_text + hash_text(&text_key(&p.prompt.text)));
        t.bucket(o.g_answers, 6, p.answers().len());
    }

    // Units, in a fixed order: by owner (mine, theirs, neutral), then id.
    let mut order: Vec<&Unit> = g.units.iter().collect();
    let rank = |u: &Unit| match u.owner {
        Some(s) if s == seat => 0,
        Some(_) => 1,
        None => 2,
    };
    order.sort_by_key(|u| (rank(u), u.id));
    let flag_cell = g.flag_cell();
    let mut units = Vec::with_capacity(order.len());
    let mut unit_ids = Vec::with_capacity(order.len());
    let mut mine = 0;
    let mut theirs = 0;
    for u in &order {
        let mut ut = Token::default();
        ut.one(0);
        let r = rank(u);
        ut.one(o.u_owner + r);
        match r {
            0 => mine += 1,
            1 => theirs += 1,
            _ => {}
        }
        if u.is_flag() {
            ut.one(o.u_flags);
        }
        if u.label == "+" {
            ut.one(o.u_flags + 1);
        }
        // Depth in the stack: 0 is on top.
        let depth = g
            .units
            .iter()
            .filter(|v| v.cell == u.cell && v.z > u.z)
            .count();
        if depth == 0 {
            ut.one(o.u_flags + 2);
        }
        ut.bucket(o.u_depth, 4, depth);
        if u.activations < u.max_activations() {
            ut.one(o.u_flags + 3);
        }
        match d.board.kind(u.cell) {
            Some(CellKind::Home(s)) if s == seat => ut.one(o.u_flags + 4),
            Some(CellKind::Home(_)) => ut.one(o.u_flags + 5),
            _ => {}
        }
        if let Some(i) = cell_index(u.cell, seat) {
            ut.one(o.u_cell + i);
        }
        if let Some(f) = flag_cell {
            ut.bucket(o.u_dist, 5, crate::rules::hex_dist(u.cell, f) as usize);
        }
        ut.one(o.u_facing + canonical_facing(u.facing, seat));
        let front = u.cell.step(u.facing);
        let top_in_front = g
            .units
            .iter()
            .filter(|v| v.cell == front)
            .max_by_key(|v| v.z);
        ut.one(
            o.u_front
                + match (d.board.kind(front), top_in_front) {
                    (None, _) => 0,
                    (Some(_), None) => 1,
                    (Some(_), Some(v)) if v.owner.is_none() => 4,
                    (Some(_), Some(v)) if v.owner == u.owner => 2,
                    (Some(_), Some(_)) => 3,
                },
        );
        for p in &u.powers {
            let base = if p.card { o.u_cards } else { o.u_free };
            // Repeated powers add up.
            let id = (base + p.power) as u32;
            match ut.ids.iter().position(|&x| x == id) {
                Some(i) => ut.vals[i] += 1.0,
                None => ut.one(id as usize),
            }
        }
        ut.bucket(o.u_tokens, 6, u.tokens as usize);
        ut.bucket(o.u_used, 4, u.activations as usize);
        ut.bucket(o.u_moved, 4, u.moved as usize);
        let (mut enemies, mut friends) = (0, 0);
        for n in u.cell.neighbors() {
            if let Some(v) = g.units.iter().filter(|v| v.cell == n).max_by_key(|v| v.z)
                && let (Some(a), Some(b)) = (v.owner, u.owner)
            {
                if a == b {
                    friends += 1;
                } else {
                    enemies += 1;
                }
            }
        }
        ut.bucket(o.u_enemies, 7, enemies);
        ut.bucket(o.u_friends, 7, friends);
        units.push(ut);
        unit_ids.push(u.id);
    }
    t.bucket(o.g_units_me, 7, mine);
    t.bucket(o.g_units_them, 7, theirs);
    Observation {
        global: t,
        units,
        unit_ids,
    }
}

/// Encodes the actions `seat` could take in `game` (eg from
/// `bot::bot_actions`), with pointers into `obs`.
pub fn encode_actions(
    game: &Game,
    seat: Seat,
    obs: &Observation,
    actions: &[Action],
) -> Vec<ActionToken> {
    let o = offsets();
    let d = data();
    let g = shown(game);
    let index_of = |id: u32| -> i32 {
        obs.unit_ids
            .iter()
            .position(|&x| x == id)
            .map_or(-1, |i| i as i32)
    };
    let unit = |id: u32| g.units.iter().find(|u| u.id == id);
    actions
        .iter()
        .map(|action| {
            let mut t = Token::default();
            t.one(0);
            t.one(o.a_kind + action_kind(action));
            let mut ptr = -1;
            let mut target = -1;
            match action {
                Action::TakeFaceup { slot } => {
                    t.bucket(o.a_slot, 8, *slot);
                    if let Some(&card) = g.faceup.get(*slot) {
                        t.one(o.a_power + card);
                    }
                }
                Action::Spawn { cell, .. } => {
                    if let Some(i) = cell_index(*cell, seat) {
                        t.one(o.a_cell + i);
                    }
                }
                Action::ApplyHand {
                    card,
                    unit: id,
                    replace,
                    ..
                }
                | Action::ApplyFree {
                    power: card,
                    unit: id,
                    replace,
                    ..
                } => {
                    t.one(o.a_power + card);
                    ptr = index_of(*id);
                    if let Some(r) = replace.and_then(|r| power_of(g, *id, r)) {
                        t.one(o.a_replace + r);
                    }
                }
                Action::Activate {
                    unit: id,
                    facing,
                    power,
                    origin,
                    ..
                } => {
                    ptr = index_of(*id);
                    t.one(o.a_facing + canonical_facing(*facing, seat));
                    if let Some(u) = unit(*id) {
                        t.one(o.a_turn + (*facing + 6 - u.facing) % 6);
                    }
                    if let Some(p) = power.and_then(|i| power_of(g, *id, i)) {
                        t.one(o.a_power + p);
                    }
                    if let Some(origin) = origin {
                        t.one(o.a_flags);
                        target = index_of(*origin);
                    }
                }
                Action::RemovePower {
                    unit: id, index, ..
                } => {
                    ptr = index_of(*id);
                    if let Some(p) = power_of(g, *id, *index) {
                        t.one(o.a_power + p);
                    }
                }
                Action::Choose { index } => {
                    t.bucket(o.a_choice, MAX_CHOICE, *index);
                    if let Some(opt) = game.prompt().and_then(|p| p.options.get(*index)) {
                        if let Some(id) = opt.unit {
                            t.one(o.a_flags + 1);
                            target = index_of(id);
                        }
                        if let Some(c) = opt.cell {
                            t.one(o.a_flags + 2);
                            if let Some(i) = cell_index(c, seat) {
                                t.one(o.a_cell + i);
                            }
                        }
                        if let Some(p) = opt.power {
                            t.one(o.a_flags + 3);
                            t.one(o.a_power + p);
                        }
                        if opt.unit.is_none() && opt.cell.is_none() && opt.power.is_none() {
                            t.one(o.a_text + hash_text(&text_key(&opt.label)));
                            for (i, p) in d.powers.iter().enumerate() {
                                if opt.label.contains(p.name.as_str()) {
                                    t.one(o.a_mentions + i);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            ActionToken {
                token: t,
                unit: ptr,
                target,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Options;

    #[test]
    fn encodings_stay_in_range_during_random_games() {
        let l = layout();
        let mut rng = crate::bot::Rng::new(7);
        for seed in 0..20 {
            let (mut game, _) = Game::new(Options {
                seed: Some(seed),
                ..Options::default()
            })
            .unwrap();
            for _ in 0..400 {
                if matches!(game.phase, Phase::Over { .. }) {
                    break;
                }
                let seat = crate::bot::to_move(&game);
                for s in [0, 1] {
                    let obs = observe(&game, s);
                    assert!(obs.global.ids.iter().all(|&i| (i as usize) < l.global));
                    for u in &obs.units {
                        assert!(u.ids.iter().all(|&i| (i as usize) < l.unit));
                    }
                }
                let obs = observe(&game, seat);
                let actions = crate::bot::bot_actions(&game, seat, 8);
                let encoded = encode_actions(&game, seat, &obs, &actions);
                for a in &encoded {
                    assert!(a.token.ids.iter().all(|&i| (i as usize) < l.action));
                    assert!(a.unit < obs.units.len() as i32);
                    assert!(a.target < obs.units.len() as i32);
                }
                let action = &actions[rng.below(actions.len())];
                game.apply(seat, action).unwrap();
            }
        }
    }

    #[test]
    fn each_seat_sees_its_own_home_at_the_top() {
        let d = data();
        for &(cell, kind) in &d.board.cells {
            assert_eq!(canonical(canonical(cell, 1), 1), cell);
            if let CellKind::Home(seat) = kind {
                // Seen by its owner, a home space is where seat 0's homes are.
                let seen = canonical(cell, seat);
                assert_eq!(d.board.kind(seen), Some(CellKind::Home(0)));
            }
        }
    }

    #[test]
    fn canonical_cells_are_on_the_board() {
        for &(cell, _) in &data().board.cells {
            assert!(cell_index(cell, 0).is_some());
            assert!(cell_index(cell, 1).is_some());
        }
    }
}
