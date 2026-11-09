//! Game state, and the manual rules.
//!
//! A game is played with automatic or manual rules (players can switch between
//! them). The automatic rules live in `rules.rs`: they carry out every power and
//! only allow legal actions.
//!
//! With manual rules, the server automates the parts of the rules that are pure
//! bookkeeping (the draft, drawing, the faceup row, spawning, applying powers,
//! activation limits, the flag, scoring) and checks the structural rules. Power
//! effects are carried out by the current player with the "manual" actions
//! (move, kill, move powers, ...). Structural checks that powers can
//! legitimately bend are "soft": the client can resend the action with
//! `force: true` to override them.

use crate::data::{Cell, CellKind, Kind, Seat, data};
use crate::rules::Pending;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const SEAT_NAMES: [&str; 2] = ["Letters", "Numbers"];
pub const LABELS: [[&str; 5]; 2] = [["A", "B", "C", "D", "E"], ["1", "2", "3", "4", "5"]];
pub const MAX_POWERS: usize = 3;
pub const FACEUP_SLOTS: usize = 6;
pub const DRAFT_CARDS: usize = 3;
pub const DRAWS_PER_TURN: u32 = 2;

pub(crate) fn power_name(power: usize) -> &'static str {
    &data().powers[power].name
}

pub(crate) fn cap(text: String) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

/// "Letters'" rather than "Letters's".
pub(crate) fn possessive(name: &str) -> String {
    if name.ends_with('s') {
        format!("{name}'")
    } else {
        format!("{name}'s")
    }
}

fn overridden(force: bool) -> &'static str {
    if force { " (rules overridden)" } else { "" }
}

pub(crate) fn plural(n: u32, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Units start facing the opposite side of the board.
pub(crate) fn default_facing(seat: Seat) -> usize {
    if seat == 0 { 2 } else { 5 }
}

pub(crate) fn turn_text(from: usize, to: usize) -> Option<&'static str> {
    match (to + 6 - from) % 6 {
        1 => Some("60° clockwise"),
        2 => Some("120° clockwise"),
        3 => Some("around"),
        4 => Some("120° counter-clockwise"),
        5 => Some("60° counter-clockwise"),
        _ => None,
    }
}

pub type Powers = smallvec::SmallVec<[PowerRef; 4]>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerRef {
    pub power: usize,
    /// True for real cards, which go to the discard pile when removed. False
    /// for free powers and the Flag power, which just disappear.
    pub card: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Unit {
    pub id: u32,
    pub owner: Option<Seat>,
    /// "A"–"E" or "1"–"5"; "" for neutral units; "+" for a sixth unit until its
    /// owner decides which unit dies.
    pub label: &'static str,
    pub cell: Cell,
    /// Index into `DIRS`.
    pub facing: usize,
    /// At most `MAX_POWERS`, stored inline so copying a game allocates less.
    pub powers: Powers,
    pub tokens: u32,
    pub activations: u32,
    /// Stacking order: the unit with the highest `z` in a space is on top.
    pub z: u32,
    /// How many spaces the unit has moved this turn (for the *Flag* power).
    pub moved: u32,
}

impl Unit {
    pub(crate) fn count(&self, power: Option<usize>) -> u32 {
        power.map_or(0, |power| {
            self.powers.iter().filter(|p| p.power == power).count() as u32
        })
    }

    pub fn is_flag(&self) -> bool {
        self.count(Some(data().flag)) > 0
    }

    /// *Fast*: can activate this unit twice per turn (stacks).
    pub fn max_activations(&self) -> u32 {
        1 + self.count(data().fast)
    }

    /// *Strong*: counts as 1 extra unit for scoring (stacks).
    pub fn weight(&self) -> u32 {
        1 + self.count(data().strong)
    }

    pub fn name(&self) -> String {
        match self.owner {
            // A sixth unit, until its owner decides which unit dies.
            Some(_) if self.label == "+" => "the new unit".into(),
            Some(_) => self.label.to_string(),
            None if self.is_flag() => "the flag".into(),
            None => "a neutral unit".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    /// None picks at random.
    #[serde(default)]
    pub first_player: Option<Seat>,
    /// The power with the *Free Power X* token. None means *Walk*.
    #[serde(default)]
    pub free_power: Option<usize>,
    /// Advanced: draw a power from the deck and make it free too.
    #[serde(default)]
    pub free_power_y: bool,
    /// Advanced: draw a power from the deck and add it to the flag.
    #[serde(default)]
    pub flag_power: bool,
    /// Playtesting variant: the first player doesn't draw on their first turn.
    #[serde(default)]
    pub skip_first_draw: bool,
    #[serde(default = "default_target_score")]
    pub target_score: u32,
    #[serde(default = "default_second_player_points")]
    pub second_player_points: u32,
    /// Automatic rules (every power is carried out and only legal actions are
    /// allowed) or manual rules (players carry out powers themselves).
    #[serde(default = "default_automatic")]
    pub automatic: bool,
    /// Seed for shuffling. None picks one at random.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Cards left out of the deck (for balance experiments).
    #[serde(default)]
    pub removed: Vec<usize>,
}

fn default_automatic() -> bool {
    true
}

fn default_target_score() -> u32 {
    5
}

fn default_second_player_points() -> u32 {
    1
}

impl Default for Options {
    fn default() -> Options {
        Options {
            first_player: None,
            free_power: None,
            free_power_y: false,
            flag_power: false,
            skip_first_draw: false,
            target_score: default_target_score(),
            second_player_points: default_second_player_points(),
            automatic: default_automatic(),
            seed: None,
            removed: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Phase {
    /// Players take turns taking a card until each has `DRAFT_CARDS`.
    Draft,
    /// Start of a turn: the current player must draw.
    Draw {
        remaining: u32,
    },
    /// The rest of the turn.
    Main,
    Over {
        winner: Seat,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    // Drawing. Allowed in the draft and draw phases, and as a manual action
    // during the main phase (eg for *Mine*).
    TakeFaceup {
        slot: usize,
    },
    TakeDeck,

    // The structured turn actions.
    Spawn {
        cell: Cell,
        #[serde(default)]
        force: bool,
    },
    ApplyHand {
        card: usize,
        unit: u32,
        replace: Option<usize>,
        #[serde(default)]
        force: bool,
    },
    ApplyFree {
        power: usize,
        unit: u32,
        replace: Option<usize>,
        #[serde(default)]
        force: bool,
    },
    Activate {
        unit: u32,
        facing: usize,
        power: Option<usize>,
        /// With *Remote*: use the power from the space of this friendly unit.
        #[serde(default)]
        origin: Option<u32>,
        #[serde(default)]
        force: bool,
    },
    /// Discard a power from a unit (`to_hand: false`, a structured action), or
    /// return it to the current player's hand (manual, eg for *Recall*).
    RemovePower {
        unit: u32,
        index: usize,
        #[serde(default)]
        to_hand: bool,
        #[serde(default)]
        force: bool,
    },
    /// With manual rules, `score` says whether the player scores. With
    /// automatic rules, scoring is worked out and `score` is ignored.
    EndTurn {
        #[serde(default)]
        score: bool,
    },

    // Manual actions for resolving power effects.
    Move {
        unit: u32,
        cell: Cell,
        #[serde(default)]
        force: bool,
    },
    Rotate {
        unit: u32,
        facing: usize,
    },
    Kill {
        unit: u32,
    },
    CreateUnit {
        cell: Cell,
        owner: Option<Seat>,
        #[serde(default)]
        force: bool,
    },
    SetOwner {
        unit: u32,
        owner: Option<Seat>,
    },
    MovePower {
        unit: u32,
        index: usize,
        to_unit: u32,
        replace: Option<usize>,
        #[serde(default)]
        swap: bool,
    },
    DiscardCard {
        card: usize,
    },
    Tokens {
        unit: u32,
        delta: i32,
    },
    AdjustScore {
        seat: Seat,
        delta: i32,
    },

    // Automatic rules: answer the current prompt, or abandon the action it belongs to.
    Choose {
        index: usize,
    },
    Cancel,
    /// Switch between automatic and manual rules.
    SetMode {
        automatic: bool,
    },
}

impl Action {
    /// Drawing reveals hidden cards and ending the turn hands over control, so
    /// neither can be undone.
    pub fn undoable(&self) -> bool {
        !matches!(
            self,
            Action::TakeFaceup { .. } | Action::TakeDeck | Action::EndTurn { .. }
        )
    }
}

#[derive(Debug)]
pub struct ActionError {
    pub message: String,
    /// The action broke a soft rule and can be retried with `force: true`.
    pub forceable: bool,
}

type Result<T> = std::result::Result<T, ActionError>;

pub(crate) fn hard(message: impl Into<String>) -> ActionError {
    ActionError {
        message: message.into(),
        forceable: false,
    }
}

pub(crate) fn check(ok: bool, force: bool, message: &str) -> Result<()> {
    if ok || force {
        Ok(())
    } else {
        Err(ActionError {
            message: message.into(),
            forceable: true,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Game {
    pub options: Options,
    pub phase: Phase,
    pub current: Seat,
    pub first_player: Seat,
    pub turn: u32,
    pub scores: [u32; 2],
    /// Sorted by `z`.
    pub units: Vec<Unit>,
    pub hands: [Vec<usize>; 2],
    /// The top of the deck is the end of the vec.
    pub deck: Vec<usize>,
    pub discard: Vec<usize>,
    pub faceup: Vec<usize>,
    pub free_powers: Vec<usize>,
    pub spawned_this_turn: bool,
    /// Actions the current player has taken this turn (bots cap this).
    pub actions_this_turn: u32,
    pub(crate) next_id: u32,
    pub(crate) next_z: u32,
    /// Automatic or manual rules.
    pub automatic: bool,
    /// With automatic rules: an action waiting for the current player to answer a prompt.
    pub pending: Option<Box<Pending>>,
    /// Shuffling is deterministic given the seed, so actions can be replayed exactly.
    pub(crate) rng: u64,
    /// How many cards have been drawn from the deck (drawing can't be undone).
    pub draws: u64,
    /// Powers each player can't apply or activate this turn (*Forbid*).
    pub forbidden: [Vec<usize>; 2],
    /// (victim's owner, killer) for each kill this turn and last turn (*Vengeance*).
    pub(crate) kills_this_turn: Vec<(Seat, u32)>,
    pub(crate) kills_last_turn: Vec<(Seat, u32)>,
    /// Cards each player's opponent has seen go into their hand (taken from the
    /// faceup row, or returned from the board). Only those still in the hand count.
    pub(crate) public_hand: [Vec<usize>; 2],
}

/// splitmix64.
pub(crate) fn next_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn shuffle(cards: &mut [usize], state: &mut u64) {
    for i in (1..cards.len()).rev() {
        let j = (next_random(state) % (i as u64 + 1)) as usize;
        cards.swap(i, j);
    }
}

impl Game {
    pub fn new(options: Options) -> std::result::Result<(Game, Vec<String>), String> {
        let d = data();
        let free = options.free_power.unwrap_or(d.walk);
        if free >= d.powers.len() || free == d.flag {
            return Err("That can't be the free power.".into());
        }
        if options.target_score == 0 {
            return Err("The target score must be at least 1.".into());
        }
        if options.first_player.is_some_and(|seat| seat > 1) {
            return Err("Unknown first player.".into());
        }

        let mut rng = options.seed.unwrap_or_else(rand::random);
        let first = options
            .first_player
            .unwrap_or_else(|| (next_random(&mut rng) % 2) as Seat);
        let mut deck: Vec<usize> = (0..d.powers.len())
            .filter(|&p| p != d.flag && p != free && !options.removed.contains(&p))
            .collect();
        shuffle(&mut deck, &mut rng);

        let mut game = Game {
            options: options.clone(),
            phase: Phase::Draft,
            current: first,
            first_player: first,
            turn: 0,
            scores: [0, 0],
            units: Vec::new(),
            hands: [Vec::new(), Vec::new()],
            deck,
            discard: Vec::new(),
            faceup: Vec::new(),
            free_powers: vec![free],
            spawned_this_turn: false,
            actions_this_turn: 0,
            next_id: 1,
            next_z: 1,
            automatic: options.automatic,
            pending: None,
            rng,
            draws: 0,
            forbidden: [Vec::new(), Vec::new()],
            kills_this_turn: Vec::new(),
            kills_last_turn: Vec::new(),
            public_hand: [Vec::new(), Vec::new()],
        };
        game.scores[1 - first] = options.second_player_points;

        let mut log = vec![format!(
            "New game with {} rules. {} goes first. Free power: {}.",
            if options.automatic {
                "automatic"
            } else {
                "manual"
            },
            SEAT_NAMES[first],
            power_name(free)
        )];
        if options.free_power_y
            && let Some(power) = game.deck.pop()
        {
            game.free_powers.push(power);
            log.push(format!("Second free power: {}.", power_name(power)));
        }
        let mut flag_powers = vec![PowerRef {
            power: d.flag,
            card: false,
        }];
        if options.flag_power
            && let Some(power) = game.deck.pop()
        {
            flag_powers.push(PowerRef { power, card: true });
            log.push(format!("The flag also has {}.", power_name(power)));
        }
        game.add_unit(None, d.board.flag, 0, flag_powers);
        game.refill_faceup(&mut log);
        log.push(format!(
            "Draft: take turns taking cards until you each have {DRAFT_CARDS}. {} picks first.",
            SEAT_NAMES[first]
        ));
        Ok((game, log))
    }

    /// Applies an action atomically: on error the game is left unchanged.
    /// Returns log lines describing what happened.
    pub fn apply(&mut self, seat: Seat, action: &Action) -> Result<Vec<String>> {
        if let Action::SetMode { automatic } = *action {
            return self.set_mode(seat, automatic);
        }
        if self.automatic {
            return self.auto_apply(seat, action);
        }
        if matches!(action, Action::Choose { .. } | Action::Cancel) {
            return Err(hard("There's nothing to choose with manual rules."));
        }
        let mut next = self.clone();
        let mut log = Vec::new();
        next.apply_inner(seat, action, &mut log)?;
        next.fix_flag(&mut log);
        *self = next;
        Ok(log)
    }

    fn set_mode(&mut self, seat: Seat, automatic: bool) -> Result<Vec<String>> {
        if seat != self.current {
            return Err(hard("Only the current player can change the rules."));
        }
        if self.pending.is_some() {
            return Err(hard("Answer the current question first."));
        }
        if self.automatic == automatic {
            return Err(hard("Those rules are already in use."));
        }
        self.automatic = automatic;
        Ok(vec![format!(
            "{} switched to {} rules.",
            SEAT_NAMES[seat],
            if automatic { "automatic" } else { "manual" }
        )])
    }

    pub(crate) fn apply_inner(
        &mut self,
        seat: Seat,
        action: &Action,
        log: &mut Vec<String>,
    ) -> Result<()> {
        let d = data();
        let me = SEAT_NAMES[seat];
        if matches!(self.phase, Phase::Over { .. }) {
            return Err(hard("The game is over. Start a new game to play again."));
        }
        if seat != self.current {
            return Err(hard(format!(
                "It's {} turn.",
                possessive(SEAT_NAMES[self.current])
            )));
        }

        match *action {
            Action::TakeFaceup { slot } => {
                let card = *self
                    .faceup
                    .get(slot)
                    .ok_or_else(|| hard("There's no card there."))?;
                log.push(format!(
                    "{me} took {} from the faceup row{}.",
                    power_name(card),
                    if self.phase == Phase::Main {
                        " (manual draw)"
                    } else {
                        ""
                    }
                ));
                self.take_faceup(slot, log);
                self.add_to_hand(seat, card, true);
                self.after_take(seat, log);
            }

            Action::TakeDeck => {
                let mut notes = Vec::new();
                let card = self
                    .draw_card(&mut notes)
                    .ok_or_else(|| hard("The deck is empty."))?;
                log.push(format!(
                    "{me} drew a card from the deck{}.",
                    if self.phase == Phase::Main {
                        " (manual draw)"
                    } else {
                        ""
                    }
                ));
                log.extend(notes);
                self.add_to_hand(seat, card, false);
                self.after_take(seat, log);
            }

            Action::Spawn { cell, force } => {
                self.require_main()?;
                let kind = d
                    .board
                    .kind(cell)
                    .ok_or_else(|| hard("That's not a space on the board."))?;
                check(
                    kind == CellKind::Home(seat),
                    force,
                    "You can only spawn units in your own home spaces.",
                )?;
                check(
                    self.units_at(cell).is_empty(),
                    force,
                    "That space is occupied.",
                )?;
                check(
                    !self.spawned_this_turn,
                    force,
                    "You can only spawn one unit per turn.",
                )?;
                if self.free_label(seat).is_none() {
                    return Err(hard("You already have 5 units. Kill one first."));
                }
                let index = self.add_unit(Some(seat), cell, default_facing(seat), Vec::new());
                self.spawned_this_turn = true;
                log.push(format!(
                    "{me} spawned unit {}{}.",
                    self.units[index].label,
                    overridden(force)
                ));
            }

            Action::ApplyHand {
                card,
                unit,
                replace,
                force,
            } => {
                self.require_main()?;
                let pos = self.hands[seat]
                    .iter()
                    .position(|&c| c == card)
                    .ok_or_else(|| hard("That card isn't in your hand."))?;
                let index = self.unit_index(unit)?;
                self.check_home_unit(index, seat, force, "apply powers to")?;
                self.hands[seat].remove(pos);
                log.push(format!(
                    "{} gained {} from {} hand{}.",
                    cap(self.units[index].name()),
                    power_name(card),
                    possessive(me),
                    overridden(force)
                ));
                self.add_power(
                    index,
                    PowerRef {
                        power: card,
                        card: true,
                    },
                    replace,
                    log,
                )?;
            }

            Action::ApplyFree {
                power,
                unit,
                replace,
                force,
            } => {
                self.require_main()?;
                if !self.free_powers.contains(&power) {
                    return Err(hard("That isn't a free power."));
                }
                let index = self.unit_index(unit)?;
                self.check_home_unit(index, seat, force, "apply powers to")?;
                log.push(format!(
                    "{} gained {} (free power){}.",
                    cap(self.units[index].name()),
                    power_name(power),
                    overridden(force)
                ));
                self.add_power(index, PowerRef { power, card: false }, replace, log)?;
            }

            Action::Activate {
                unit,
                facing,
                power,
                force,
                ..
            } => {
                self.require_main()?;
                if facing >= 6 {
                    return Err(hard("Unknown direction."));
                }
                let index = self.unit_index(unit)?;
                let u = &self.units[index];
                check(
                    u.owner == Some(seat) || (u.owner.is_none() && u.is_flag()),
                    force,
                    "You can only activate your own units (and the flag).",
                )?;
                check(
                    self.is_top(index),
                    force,
                    "Units underneath a stack can't be activated.",
                )?;
                check(
                    u.activations < u.max_activations(),
                    force,
                    "That unit has already been activated this turn.",
                )?;
                let used = match power {
                    Some(i) => {
                        let p = u
                            .powers
                            .get(i)
                            .ok_or_else(|| hard("That unit doesn't have that power."))?;
                        check(
                            d.powers[p.power].kind == Kind::Active,
                            force,
                            "Only active powers can be activated.",
                        )?;
                        Some(p.power)
                    }
                    None => None,
                };
                let name = cap(u.name());
                let turned = turn_text(u.facing, facing);
                let u = &mut self.units[index];
                u.facing = facing;
                u.activations += 1;
                let how = overridden(force);
                log.push(match (turned, used) {
                    (Some(t), Some(p)) => format!(
                        "{name} activated, turning {t}, and used {}{how}: {}",
                        power_name(p),
                        d.powers[p].text
                    ),
                    (None, Some(p)) => {
                        format!(
                            "{name} activated and used {}{how}: {}",
                            power_name(p),
                            d.powers[p].text
                        )
                    }
                    (Some(t), None) => format!("{name} activated, turning {t}{how}."),
                    (None, None) => {
                        format!("{name} activated without turning or using a power{how}.")
                    }
                });
            }

            Action::RemovePower {
                unit,
                index: power_index,
                to_hand,
                force,
            } => {
                self.require_main()?;
                let index = self.unit_index(unit)?;
                let p = *self.units[index]
                    .powers
                    .get(power_index)
                    .ok_or_else(|| hard("That unit doesn't have that power."))?;
                if !to_hand {
                    self.check_home_unit(index, seat, force, "discard powers from")?;
                }
                let name = self.units[index].name();
                self.units[index].powers.remove(power_index);
                if !to_hand {
                    log.push(format!(
                        "{} discarded {}{}.",
                        cap(name),
                        power_name(p.power),
                        overridden(force)
                    ));
                    self.drop_power(p, None);
                } else if p.card {
                    log.push(format!(
                        "{me} returned {} from {name} to their hand.",
                        power_name(p.power)
                    ));
                    self.drop_power(p, Some(seat));
                } else {
                    log.push(format!("{} was removed from {name}.", power_name(p.power)));
                }
            }

            Action::EndTurn { score } => {
                self.require_main()?;
                let counts = self.flag_counts();
                let (mine, theirs) = (counts[seat], counts[1 - seat]);
                if score {
                    self.scores[seat] += 1;
                    log.push(format!(
                        "{me} ended their turn and scored (flag: {mine} vs {theirs}). {me} has {}.",
                        plural(self.scores[seat], "point")
                    ));
                } else {
                    log.push(format!(
                        "{me} ended their turn without scoring (flag: {mine} vs {theirs})."
                    ));
                }
                if self.scores[seat] >= self.options.target_score {
                    self.phase = Phase::Over { winner: seat };
                    log.push(format!("{me} wins!"));
                } else {
                    self.current = 1 - seat;
                    self.begin_turn(log);
                }
            }

            Action::Move { unit, cell, force } => {
                self.require_main()?;
                let index = self.unit_index(unit)?;
                let kind = d
                    .board
                    .kind(cell)
                    .ok_or_else(|| hard("That's not a space on the board."))?;
                if self.units[index].cell == cell {
                    return Err(hard("The unit is already there."));
                }
                if let CellKind::Home(home) = kind {
                    check(
                        self.units[index].owner == Some(home),
                        force,
                        "Only friendly units can be in a home space.",
                    )?;
                }
                let occupied = !self.units_at(cell).is_empty();
                check(
                    !occupied,
                    force,
                    "That space is occupied. Do it anyway to stack the units.",
                )?;
                let z = self.bump_z();
                let u = &mut self.units[index];
                u.cell = cell;
                u.z = z;
                log.push(format!(
                    "{} moved{}{}.",
                    cap(u.name()),
                    if occupied { " onto a stack" } else { "" },
                    overridden(force && !occupied)
                ));
                self.sort_units();
            }

            Action::Rotate { unit, facing } => {
                self.require_main()?;
                if facing >= 6 {
                    return Err(hard("Unknown direction."));
                }
                let index = self.unit_index(unit)?;
                let u = &mut self.units[index];
                let turned = turn_text(u.facing, facing)
                    .ok_or_else(|| hard("The unit already faces that way."))?;
                u.facing = facing;
                log.push(format!("{} turned {turned} (manual).", cap(u.name())));
            }

            Action::Kill { unit } => {
                self.require_main()?;
                let index = self.unit_index(unit)?;
                log.push(format!("{} was killed.", cap(self.units[index].name())));
                self.kill(index);
            }

            Action::CreateUnit { cell, owner, force } => {
                self.require_main()?;
                if owner.is_some_and(|o| o > 1) {
                    return Err(hard("Unknown player."));
                }
                let kind = d
                    .board
                    .kind(cell)
                    .ok_or_else(|| hard("That's not a space on the board."))?;
                if let CellKind::Home(home) = kind {
                    check(
                        owner == Some(home),
                        force,
                        "Only friendly units can be in a home space.",
                    )?;
                }
                check(
                    self.units_at(cell).is_empty(),
                    force,
                    "That space is occupied.",
                )?;
                if let Some(o) = owner
                    && self.free_label(o).is_none()
                {
                    return Err(hard(format!(
                        "{} already has 5 units. Kill one first.",
                        SEAT_NAMES[o]
                    )));
                }
                let index = self.add_unit(owner, cell, owner.map_or(0, default_facing), Vec::new());
                let what = match owner {
                    Some(o) => format!("{} unit {}", SEAT_NAMES[o], self.units[index].label),
                    None => "a neutral unit".into(),
                };
                log.push(format!("{me} created {what}{}.", overridden(force)));
            }

            Action::SetOwner { unit, owner } => {
                self.require_main()?;
                if owner.is_some_and(|o| o > 1) {
                    return Err(hard("Unknown player."));
                }
                let index = self.unit_index(unit)?;
                if self.units[index].owner == owner {
                    return Err(hard("The unit already belongs to them."));
                }
                let label = match owner {
                    Some(o) => self.free_label(o).ok_or_else(|| {
                        hard(format!(
                            "{} already has 5 units. Kill one first.",
                            SEAT_NAMES[o]
                        ))
                    })?,
                    None => "",
                };
                let before = cap(self.units[index].name());
                let u = &mut self.units[index];
                u.owner = owner;
                u.label = label;
                log.push(match owner {
                    Some(o) => format!(
                        "{before} now belongs to {} as unit {}.",
                        SEAT_NAMES[o], u.label
                    ),
                    None => format!("{before} is now neutral."),
                });
            }

            Action::MovePower {
                unit,
                index: power_index,
                to_unit,
                replace,
                swap,
            } => {
                self.require_main()?;
                let from = self.unit_index(unit)?;
                let to = self.unit_index(to_unit)?;
                if from == to {
                    return Err(hard("Pick a different unit."));
                }
                let p = *self.units[from]
                    .powers
                    .get(power_index)
                    .ok_or_else(|| hard("That unit doesn't have that power."))?;
                let (from_name, to_name) = (self.units[from].name(), self.units[to].name());
                if swap {
                    let other = replace.ok_or_else(|| hard("Choose a power to swap with."))?;
                    let q = *self.units[to]
                        .powers
                        .get(other)
                        .ok_or_else(|| hard("That unit doesn't have that power."))?;
                    self.units[from].powers[power_index] = q;
                    self.units[to].powers[other] = p;
                    log.push(format!(
                        "{} swapped {} for {to_name}'s {}.",
                        cap(from_name),
                        power_name(p.power),
                        power_name(q.power)
                    ));
                } else {
                    self.units[from].powers.remove(power_index);
                    log.push(format!(
                        "{} moved from {from_name} to {to_name}.",
                        power_name(p.power)
                    ));
                    self.add_power(to, p, replace, log)?;
                }
            }

            Action::DiscardCard { card } => {
                self.require_main()?;
                let pos = self.hands[seat]
                    .iter()
                    .position(|&c| c == card)
                    .ok_or_else(|| hard("That card isn't in your hand."))?;
                self.hands[seat].remove(pos);
                self.discard.push(card);
                log.push(format!(
                    "{me} discarded {} from their hand.",
                    power_name(card)
                ));
            }

            Action::Tokens { unit, delta } => {
                self.require_main()?;
                let index = self.unit_index(unit)?;
                let u = &mut self.units[index];
                u.tokens = (u.tokens as i64 + delta as i64).max(0) as u32;
                log.push(format!(
                    "{} has {}.",
                    cap(u.name()),
                    plural(u.tokens, "token")
                ));
            }

            Action::Choose { .. } | Action::Cancel | Action::SetMode { .. } => {
                return Err(hard("That isn't a manual action."));
            }

            Action::AdjustScore {
                seat: target,
                delta,
            } => {
                self.require_main()?;
                if target > 1 {
                    return Err(hard("Unknown player."));
                }
                self.scores[target] = (self.scores[target] as i64 + delta as i64).max(0) as u32;
                log.push(format!(
                    "{me} set {} score to {} (manual).",
                    possessive(SEAT_NAMES[target]),
                    self.scores[target]
                ));
                if self.scores[target] >= self.options.target_score {
                    self.phase = Phase::Over { winner: target };
                    log.push(format!("{} wins!", SEAT_NAMES[target]));
                }
            }
        }
        Ok(())
    }

    fn require_main(&self) -> Result<()> {
        match self.phase {
            Phase::Main => Ok(()),
            Phase::Draft => Err(hard(
                "Finish the draft first: take a faceup card or draw from the deck.",
            )),
            Phase::Draw { .. } => Err(hard(
                "Draw your cards first: take a faceup card or draw from the deck.",
            )),
            Phase::Over { .. } => Err(hard("The game is over.")),
        }
    }

    fn check_home_unit(&self, index: usize, seat: Seat, force: bool, verb: &str) -> Result<()> {
        let u = &self.units[index];
        check(
            u.owner == Some(seat),
            force,
            &format!("You can only {verb} your own units."),
        )?;
        check(
            data().board.kind(u.cell) == Some(CellKind::Home(seat)),
            force,
            &format!("You can only {verb} units in your home spaces."),
        )
    }

    pub(crate) fn begin_turn(&mut self, log: &mut Vec<String>) {
        self.turn += 1;
        self.spawned_this_turn = false;
        self.actions_this_turn = 0;
        for unit in &mut self.units {
            unit.activations = 0;
            unit.moved = 0;
        }
        self.kills_last_turn = std::mem::take(&mut self.kills_this_turn);
        log.push(format!("Turn {}: {}.", self.turn, SEAT_NAMES[self.current]));
        if !self.faceup.is_empty() {
            let card = self.faceup.remove(0);
            self.discard.push(card);
            log.push(format!(
                "{} was discarded from the faceup row.",
                power_name(card)
            ));
        }
        self.refill_faceup(log);
        let skip = self.options.skip_first_draw && self.turn == 1;
        self.phase = if !skip && self.can_draw() {
            Phase::Draw {
                remaining: DRAWS_PER_TURN,
            }
        } else {
            Phase::Main
        };
    }

    pub(crate) fn after_take(&mut self, seat: Seat, log: &mut Vec<String>) {
        match self.phase {
            Phase::Draft => {
                if self.hands.iter().all(|hand| hand.len() >= DRAFT_CARDS) || !self.can_draw() {
                    self.current = self.first_player;
                    self.begin_turn(log);
                } else if self.hands[1 - seat].len() < DRAFT_CARDS {
                    self.current = 1 - seat;
                }
            }
            Phase::Draw { remaining } => {
                self.phase = if remaining > 1 && self.can_draw() {
                    Phase::Draw {
                        remaining: remaining - 1,
                    }
                } else {
                    Phase::Main
                };
            }
            _ => {}
        }
    }

    pub(crate) fn can_draw(&self) -> bool {
        !self.deck.is_empty() || !self.discard.is_empty() || !self.faceup.is_empty()
    }

    pub(crate) fn draw_card(&mut self, log: &mut Vec<String>) -> Option<usize> {
        if self.deck.is_empty() && !self.discard.is_empty() {
            self.deck = std::mem::take(&mut self.discard);
            shuffle(&mut self.deck, &mut self.rng);
            log.push("The deck ran out, so the discard pile was shuffled into a new deck.".into());
        }
        let card = self.deck.pop();
        if card.is_some() {
            self.draws += 1;
        }
        card
    }

    /// Put a card in a player's hand. `public` says whether the other player
    /// sees which card it is.
    pub(crate) fn add_to_hand(&mut self, seat: Seat, card: usize, public: bool) {
        self.hands[seat].push(card);
        self.public_hand[seat].retain(|&c| c != card);
        if public {
            self.public_hand[seat].push(card);
        }
    }

    /// The cards in a player's hand that their opponent knows about.
    pub fn known_hand(&self, seat: Seat) -> Vec<usize> {
        self.public_hand[seat]
            .iter()
            .copied()
            .filter(|c| self.hands[seat].contains(c))
            .collect()
    }

    /// A copy of the game as `seat` might imagine it: everything they can see is
    /// the same, and everything hidden from them (the cards in their opponent's
    /// hand that they haven't seen, the order of the deck, and future shuffles)
    /// is sampled at random from what's consistent with what they've seen.
    /// Search bots play out several of these instead of peeking.
    pub fn determinize(&self, seat: Seat, seed: u64) -> Game {
        let mut game = self.clone();
        let mut rng = seed ^ 0x5eed_cafe_f00d_d00d;
        let opponent = 1 - seat;
        let known = self.known_hand(opponent);
        let hidden = self.hands[opponent].len() - known.len();
        let mut pool: Vec<usize> = self.deck.clone();
        pool.extend(self.hands[opponent].iter().filter(|c| !known.contains(c)));
        shuffle(&mut pool, &mut rng);
        let mut hand = known;
        hand.extend(pool.drain(..hidden));
        let resample = |g: &mut Game| {
            g.hands[opponent] = hand.clone();
            g.deck = pool.clone();
            g.rng = rng;
        };
        resample(&mut game);
        if let Some(pending) = game.pending.as_mut() {
            resample(&mut pending.base);
            resample(&mut pending.preview);
        }
        game
    }

    /// Take a card from the faceup row: the cards to its right slide left to
    /// fill the gap, and a new card is drawn on the right.
    pub(crate) fn take_faceup(&mut self, slot: usize, log: &mut Vec<String>) -> usize {
        let card = self.faceup.remove(slot);
        self.refill_faceup(log);
        card
    }

    fn refill_faceup(&mut self, log: &mut Vec<String>) {
        while self.faceup.len() < FACEUP_SLOTS {
            match self.draw_card(log) {
                Some(card) => self.faceup.push(card),
                None => break,
            }
        }
    }

    fn unit_index(&self, id: u32) -> Result<usize> {
        self.units
            .iter()
            .position(|u| u.id == id)
            .ok_or_else(|| hard("That unit no longer exists."))
    }

    /// Indices of the units in a space, bottom of the stack first.
    fn units_at(&self, cell: Cell) -> Vec<usize> {
        (0..self.units.len())
            .filter(|&i| self.units[i].cell == cell)
            .collect()
    }

    fn is_top(&self, index: usize) -> bool {
        self.units_at(self.units[index].cell).last() == Some(&index)
    }

    pub(crate) fn free_label(&self, seat: Seat) -> Option<&'static str> {
        LABELS[seat].into_iter().find(|label| {
            !self
                .units
                .iter()
                .any(|u| u.owner == Some(seat) && u.label == *label)
        })
    }

    pub(crate) fn bump_z(&mut self) -> u32 {
        self.next_z += 1;
        self.next_z
    }

    pub(crate) fn sort_units(&mut self) {
        self.units.sort_by_key(|u| u.z);
    }

    pub(crate) fn add_unit(
        &mut self,
        owner: Option<Seat>,
        cell: Cell,
        facing: usize,
        powers: Vec<PowerRef>,
    ) -> usize {
        let label = owner.and_then(|o| self.free_label(o)).unwrap_or("");
        let z = self.bump_z();
        self.units.push(Unit {
            id: self.next_id,
            owner,
            label,
            cell,
            facing,
            powers: powers.into(),
            tokens: 0,
            activations: 0,
            z,
            moved: 0,
        });
        self.next_id += 1;
        self.units.len() - 1
    }

    fn add_power(
        &mut self,
        index: usize,
        p: PowerRef,
        replace: Option<usize>,
        log: &mut Vec<String>,
    ) -> Result<()> {
        match replace {
            Some(r) => {
                let old = *self.units[index]
                    .powers
                    .get(r)
                    .ok_or_else(|| hard("That unit doesn't have that power."))?;
                self.units[index].powers[r] = p;
                log.push(format!(
                    "{} replaced {}.",
                    power_name(p.power),
                    power_name(old.power)
                ));
                self.drop_power(old, None);
            }
            None if self.units[index].powers.len() >= MAX_POWERS => {
                return Err(hard(format!(
                    "{} already has {MAX_POWERS} powers: choose one to replace.",
                    cap(self.units[index].name())
                )));
            }
            None => self.units[index].powers.push(p),
        }
        Ok(())
    }

    /// Where a removed power goes: real cards go to the discard pile (or a
    /// hand), free powers and the Flag power just disappear.
    fn drop_power(&mut self, p: PowerRef, to_hand: Option<Seat>) {
        if p.card {
            match to_hand {
                Some(seat) => self.add_to_hand(seat, p.power, true),
                None => self.discard.push(p.power),
            }
        }
    }

    fn kill(&mut self, index: usize) {
        let unit = self.units.remove(index);
        for p in unit.powers {
            self.drop_power(p, None);
        }
    }

    /// If the *Flag* power ever leaves the board, kill any unit in the F space
    /// and put a new neutral unit with the *Flag* power there.
    fn fix_flag(&mut self, log: &mut Vec<String>) {
        if self.units.iter().any(Unit::is_flag) {
            return;
        }
        let d = data();
        while let Some(&index) = self.units_at(d.board.flag).first() {
            log.push(format!(
                "{} was killed to make room for the flag.",
                cap(self.units[index].name())
            ));
            self.kill(index);
        }
        self.add_unit(
            None,
            d.board.flag,
            0,
            vec![PowerRef {
                power: d.flag,
                card: false,
            }],
        );
        log.push("The Flag power left the board, so a new flag was placed on F.".into());
    }

    /// How many units each player has adjacent to the *Flag* power, counting
    /// *Strong*. Other modifiers (*Weak*, *Friendly*, *Bridge*, ...) are left to
    /// the players.
    pub fn flag_counts(&self) -> [u32; 2] {
        let adjacent: HashSet<Cell> = self
            .units
            .iter()
            .filter(|u| u.is_flag())
            .flat_map(|u| u.cell.neighbors())
            .collect();
        let mut counts = [0, 0];
        for unit in &self.units {
            if let Some(owner) = unit.owner
                && !unit.is_flag()
                && adjacent.contains(&unit.cell)
            {
                counts[owner] += unit.weight();
            }
        }
        counts
    }

    /// What a player is allowed to see: everything except the deck order and
    /// the other player's hand. While a prompt is waiting, the board is shown as
    /// it is at the moment of the question, and only the deciding player sees
    /// the options. With automatic rules, the current player also gets their
    /// legal actions.
    pub fn view(&self, seat: Option<Seat>) -> Value {
        let shown = self.pending.as_ref().map(|p| &*p.preview).unwrap_or(self);
        let statuses = self.automatic.then(|| shown.statuses());
        let units: Vec<Value> = shown
            .units
            .iter()
            .map(|u| {
                let mut value = serde_json::to_value(u).expect("units serialize");
                value["max_activations"] = json!(u.max_activations());
                if let Some(status) = statuses.as_ref().and_then(|s| s.get(&u.id)) {
                    value["status"] = json!(status);
                }
                value
            })
            .collect();
        let prompt = self.pending.as_ref().map(|p| {
            if Some(p.seat) == seat {
                json!({ "seat": p.seat, "text": p.prompt.text, "options": p.prompt.options })
            } else {
                json!({ "seat": p.seat, "text": p.prompt.text })
            }
        });
        let legal = match seat {
            Some(s) if self.automatic => self.legal_actions(s),
            _ => Vec::new(),
        };
        let flag_counts = if self.automatic {
            shown.auto_flag_counts()
        } else {
            shown.flag_counts()
        };
        json!({
            "automatic": self.automatic,
            "phase": shown.phase,
            "current": shown.current,
            "first_player": shown.first_player,
            "turn": shown.turn,
            "scores": shown.scores,
            "target_score": shown.options.target_score,
            "units": units,
            "hand": seat.map(|s| &shown.hands[s]),
            "hand_counts": [shown.hands[0].len(), shown.hands[1].len()],
            "deck_count": shown.deck.len(),
            "discard": shown.discard,
            "faceup": shown.faceup,
            "free_powers": shown.free_powers,
            "spawned_this_turn": shown.spawned_this_turn,
            "flag_counts": flag_counts,
            "forbidden": shown.forbidden,
            "options": shown.options,
            "prompt": prompt,
            "legal": legal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power(name: &str) -> usize {
        data().powers.iter().position(|p| p.name == name).unwrap()
    }

    fn home(seat: Seat) -> Vec<Cell> {
        data()
            .board
            .cells
            .iter()
            .filter(|(_, k)| *k == CellKind::Home(seat))
            .map(|(c, _)| *c)
            .collect()
    }

    /// A game where Letters (seat 0) has finished drafting and drawing.
    fn started() -> Game {
        let options = Options {
            first_player: Some(0),
            automatic: false,
            ..Options::default()
        };
        let (mut game, _) = Game::new(options).unwrap();
        for _ in 0..DRAFT_CARDS {
            game.apply(0, &Action::TakeDeck).unwrap();
            game.apply(1, &Action::TakeDeck).unwrap();
        }
        assert_eq!(game.phase, Phase::Draw { remaining: 2 });
        game.apply(0, &Action::TakeFaceup { slot: 2 }).unwrap();
        game.apply(0, &Action::TakeDeck).unwrap();
        assert_eq!(game.phase, Phase::Main);
        game
    }

    fn spawn(game: &mut Game, seat: Seat, cell: Cell) -> u32 {
        game.apply(seat, &Action::Spawn { cell, force: true })
            .unwrap();
        game.units
            .iter()
            .find(|u| u.cell == cell && u.owner == Some(seat))
            .unwrap()
            .id
    }

    /// Put a specific card in a player's hand, wherever it currently is.
    fn give(game: &mut Game, seat: Seat, card: usize) {
        let [hand0, hand1] = &mut game.hands;
        for pile in [
            &mut game.deck,
            &mut game.discard,
            &mut game.faceup,
            hand0,
            hand1,
        ] {
            pile.retain(|&c| c != card);
        }
        game.hands[seat].push(card);
    }

    #[test]
    fn setup() {
        let d = data();
        let (game, _) = Game::new(Options {
            first_player: Some(1),
            automatic: false,
            ..Options::default()
        })
        .unwrap();
        assert_eq!(game.deck.len() + game.faceup.len(), d.powers.len() - 2);
        assert_eq!(game.faceup.len(), FACEUP_SLOTS);
        assert_eq!(game.scores, [1, 0]);
        assert_eq!(game.free_powers, vec![d.walk]);
        assert_eq!(game.units.len(), 1);
        assert!(game.units[0].is_flag() && game.units[0].cell == d.board.flag);
        assert_eq!(game.current, 1);
    }

    #[test]
    fn draft_and_turn_start() {
        let options = Options {
            first_player: Some(0),
            automatic: false,
            ..Options::default()
        };
        let (mut game, _) = Game::new(options).unwrap();
        assert!(game.apply(1, &Action::TakeDeck).is_err(), "not your turn");
        let faceup = game.faceup.clone();
        let next = *game.deck.last().unwrap();
        game.apply(0, &Action::TakeFaceup { slot: 2 }).unwrap();
        assert_eq!(game.hands[0], vec![faceup[2]]);
        // The cards to the right slide left, and the new card goes on the right.
        let expected = [faceup[0], faceup[1], faceup[3], faceup[4], faceup[5], next];
        assert_eq!(game.faceup, expected);
        assert_eq!(game.current, 1);
        for _ in 0..2 {
            game.apply(1, &Action::TakeDeck).unwrap();
            game.apply(0, &Action::TakeDeck).unwrap();
        }
        game.apply(1, &Action::TakeDeck).unwrap();
        // Draft over: Letters starts turn 1, and the leftmost faceup card was discarded.
        assert_eq!(game.turn, 1);
        assert_eq!(game.current, 0);
        assert_eq!(game.discard.len(), 1);
        assert_eq!(game.phase, Phase::Draw { remaining: 2 });
        assert!(
            game.apply(
                0,
                &Action::Spawn {
                    cell: home(0)[0],
                    force: false
                }
            )
            .is_err()
        );
    }

    #[test]
    fn skip_first_draw() {
        let options = Options {
            first_player: Some(1),
            skip_first_draw: true,
            automatic: false,
            ..Options::default()
        };
        let (mut game, _) = Game::new(options).unwrap();
        for _ in 0..DRAFT_CARDS {
            game.apply(1, &Action::TakeDeck).unwrap();
            game.apply(0, &Action::TakeDeck).unwrap();
        }
        assert_eq!(game.phase, Phase::Main);
        game.apply(1, &Action::EndTurn { score: false }).unwrap();
        assert_eq!(game.phase, Phase::Draw { remaining: 2 });
    }

    #[test]
    fn spawn_rules() {
        let mut game = started();
        let enemy_home = home(1)[0];
        let err = game
            .apply(
                0,
                &Action::Spawn {
                    cell: enemy_home,
                    force: false,
                },
            )
            .unwrap_err();
        assert!(err.forceable);
        let cell = home(0)[0];
        game.apply(0, &Action::Spawn { cell, force: false })
            .unwrap();
        assert_eq!(game.units.last().unwrap().label, "A");
        let err = game
            .apply(
                0,
                &Action::Spawn {
                    cell: home(0)[1],
                    force: false,
                },
            )
            .unwrap_err();
        assert!(err.forceable, "second spawn is a soft rule");
        game.apply(
            0,
            &Action::Spawn {
                cell: home(0)[1],
                force: true,
            },
        )
        .unwrap();
        assert_eq!(game.units.last().unwrap().label, "B");
    }

    #[test]
    fn apply_powers() {
        let mut game = started();
        let unit = spawn(&mut game, 0, home(0)[0]);
        let walk = data().walk;
        for _ in 0..MAX_POWERS {
            game.apply(
                0,
                &Action::ApplyFree {
                    power: walk,
                    unit,
                    replace: None,
                    force: false,
                },
            )
            .unwrap();
        }
        let card = game.hands[0][0];
        let full = Action::ApplyHand {
            card,
            unit,
            replace: None,
            force: false,
        };
        assert!(!game.apply(0, &full).unwrap_err().forceable);
        game.apply(
            0,
            &Action::ApplyHand {
                card,
                unit,
                replace: Some(1),
                force: false,
            },
        )
        .unwrap();
        let u = &game.units[game.unit_index(unit).unwrap()];
        assert_eq!(
            u.powers[1],
            PowerRef {
                power: card,
                card: true
            }
        );
        assert!(!game.hands[0].contains(&card));
        // The replaced power was free, so it doesn't go to the discard pile.
        assert!(!game.discard.contains(&walk));

        // Units outside the home spaces can't be given powers without overriding.
        game.apply(
            0,
            &Action::Move {
                unit,
                cell: Cell { x: 1, y: 2 },
                force: false,
            },
        )
        .unwrap();
        let card = game.hands[0][0];
        let apply = Action::ApplyHand {
            card,
            unit,
            replace: Some(0),
            force: false,
        };
        assert!(game.apply(0, &apply).unwrap_err().forceable);
        let discarded = game.discard.len();
        game.apply(
            0,
            &Action::RemovePower {
                unit,
                index: 1,
                to_hand: false,
                force: true,
            },
        )
        .unwrap();
        assert_eq!(game.discard.len(), discarded + 1);
    }

    #[test]
    fn failed_actions_change_nothing() {
        let mut game = started();
        let before = format!("{:?}", game);
        let unit = 999;
        assert!(game.apply(0, &Action::Kill { unit }).is_err());
        let card = game.hands[0][0];
        assert!(
            game.apply(
                0,
                &Action::ApplyHand {
                    card,
                    unit: 1,
                    replace: Some(5),
                    force: true
                }
            )
            .is_err()
        );
        assert_eq!(before, format!("{:?}", game));
    }

    #[test]
    fn activation_limits() {
        let mut game = started();
        let unit = spawn(&mut game, 0, home(0)[0]);
        let walk = data().walk;
        game.apply(
            0,
            &Action::ApplyFree {
                power: walk,
                unit,
                replace: None,
                force: false,
            },
        )
        .unwrap();
        game.apply(
            0,
            &Action::Activate {
                unit,
                facing: 4,
                power: Some(0),
                origin: None,
                force: false,
            },
        )
        .unwrap();
        let again = Action::Activate {
            unit,
            facing: 4,
            power: None,
            origin: None,
            force: false,
        };
        assert!(game.apply(0, &again).unwrap_err().forceable);

        // Fast allows a second activation.
        let fast = power("Fast");
        give(&mut game, 0, fast);
        game.apply(
            0,
            &Action::ApplyHand {
                card: fast,
                unit,
                replace: None,
                force: false,
            },
        )
        .unwrap();
        game.apply(0, &again).unwrap();

        // Passive powers can't be used.
        let mut game = started();
        let unit = spawn(&mut game, 0, home(0)[0]);
        give(&mut game, 0, fast);
        game.apply(
            0,
            &Action::ApplyHand {
                card: fast,
                unit,
                replace: None,
                force: false,
            },
        )
        .unwrap();
        let use_fast = Action::Activate {
            unit,
            facing: 0,
            power: Some(0),
            origin: None,
            force: false,
        };
        assert!(game.apply(0, &use_fast).unwrap_err().forceable);
    }

    #[test]
    fn flag_respawns() {
        let d = data();
        let mut game = started();
        let flag = game.units.iter().find(|u| u.is_flag()).unwrap().id;
        let unit = spawn(&mut game, 0, home(0)[0]);
        // Move the flag away and park a unit on F, then kill the flag.
        game.apply(
            0,
            &Action::Move {
                unit: flag,
                cell: Cell { x: 2, y: 3 },
                force: false,
            },
        )
        .unwrap();
        game.apply(
            0,
            &Action::Move {
                unit,
                cell: d.board.flag,
                force: false,
            },
        )
        .unwrap();
        game.apply(0, &Action::Kill { unit: flag }).unwrap();
        assert_eq!(game.units.len(), 1);
        assert!(game.units[0].is_flag() && game.units[0].cell == d.board.flag);
        assert!(!game.discard.contains(&d.flag));
    }

    #[test]
    fn flag_power_can_move_between_units() {
        let mut game = started();
        let flag = game.units.iter().find(|u| u.is_flag()).unwrap().id;
        let unit = spawn(&mut game, 0, home(0)[0]);
        game.apply(
            0,
            &Action::MovePower {
                unit: flag,
                index: 0,
                to_unit: unit,
                replace: None,
                swap: false,
            },
        )
        .unwrap();
        // The flag power is still on the board, so no new flag appears.
        assert_eq!(game.units.len(), 2);
        assert!(game.units.iter().find(|u| u.id == unit).unwrap().is_flag());
    }

    #[test]
    fn scoring_counts_strong() {
        let mut game = started();
        let a = spawn(&mut game, 0, home(0)[0]);
        let b = spawn(&mut game, 0, home(0)[1]);
        let strong = power("Strong");
        give(&mut game, 0, strong);
        game.apply(
            0,
            &Action::ApplyHand {
                card: strong,
                unit: a,
                replace: None,
                force: false,
            },
        )
        .unwrap();
        game.apply(
            0,
            &Action::Move {
                unit: a,
                cell: Cell { x: 1, y: 2 },
                force: false,
            },
        )
        .unwrap();
        game.apply(
            0,
            &Action::Move {
                unit: b,
                cell: Cell { x: -2, y: 3 },
                force: false,
            },
        )
        .unwrap();
        assert_eq!(game.flag_counts(), [3, 0]);

        game.apply(0, &Action::EndTurn { score: true }).unwrap();
        assert_eq!(game.scores, [1, 1]);
        assert_eq!(game.current, 1);
        assert_eq!(game.turn, 2);
        assert!(game.units.iter().all(|u| u.activations == 0));
    }

    #[test]
    fn winning() {
        let mut game = started();
        game.scores[0] = 4;
        game.apply(0, &Action::EndTurn { score: true }).unwrap();
        assert_eq!(game.phase, Phase::Over { winner: 0 });
        assert!(game.apply(0, &Action::TakeDeck).is_err());
    }

    #[test]
    fn deck_reshuffles() {
        let mut game = started();
        let mut all = std::mem::take(&mut game.deck);
        game.discard.append(&mut all);
        game.apply(0, &Action::TakeDeck).unwrap();
        assert!(game.discard.is_empty());
        assert!(!game.deck.is_empty());
    }

    #[test]
    fn hidden_information() {
        let game = started();
        let view = game.view(Some(1));
        assert_eq!(view["hand"].as_array().unwrap().len(), game.hands[1].len());
        assert_eq!(view["hand_counts"][0], json!(game.hands[0].len()));
        assert!(view.get("deck").is_none());
        assert!(game.view(None)["hand"].is_null());
    }
}
