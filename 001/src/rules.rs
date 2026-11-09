//! The automatic rules: which actions are legal, and what every power does.
//!
//! Some actions need decisions from the current player while they resolve
//! (where a lifted unit goes, which power to stun, ...). Those are asked as
//! prompts, answered with `Action::Choose`. A half-resolved action is never
//! kept around: the engine records the answers so far and replays the action
//! from the state before it started. So an action happens completely or not at
//! all, and an option is only offered if some sequence of answers after it
//! completes the action.
//!
//! Where the rules are unclear, the decisions taken are listed in
//! RULES_DECISIONS.md.

use crate::data::{Cell, CellKind, Effect, Kind, Seat, data};
use crate::game::{
    Action, ActionError, Game, MAX_POWERS, Phase, PowerRef, SEAT_NAMES, Unit, cap, default_facing,
    hard, possessive, power_name, turn_text,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

/// How many replays to spend checking that a prompt's options can be completed.
const SEARCH_BUDGET: u32 = 300;
/// How deeply triggered effects (Leader, Spiky, ...) can nest.
const MAX_DEPTH: u32 = 40;

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Prompt {
    pub text: String,
    pub options: Vec<Opt>,
    /// The answer each offered option stands for. Options that can't lead to a
    /// complete action are left out.
    #[serde(skip)]
    answers: Vec<usize>,
}

/// One answer to a prompt. Options about a space, unit or power say so, so the
/// UI can highlight them.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Opt {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<Cell>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<usize>,
}

impl Opt {
    fn text(label: impl Into<String>) -> Opt {
        Opt {
            label: label.into(),
            ..Opt::default()
        }
    }
    fn cell(cell: Cell) -> Opt {
        Opt {
            label: cell_label(cell),
            cell: Some(cell),
            ..Opt::default()
        }
    }
    fn power(label: impl Into<String>, power: usize) -> Opt {
        Opt {
            label: label.into(),
            power: Some(power),
            ..Opt::default()
        }
    }
}

/// An action waiting for the current player to answer a prompt.
#[derive(Clone, Debug)]
pub struct Pending {
    pub(crate) base: Box<Game>,
    action: Action,
    pub seat: Seat,
    answers: Vec<usize>,
    pub prompt: Prompt,
    /// The game as it is at the moment of the question, for display.
    pub preview: Box<Game>,
}

enum Stop {
    /// The action needs another answer.
    Need(Prompt, Box<Game>),
    Illegal(String),
}

type R<T> = Result<T, Stop>;

fn illegal<T>(message: impl Into<String>) -> R<T> {
    Err(Stop::Illegal(message.into()))
}

struct Ctx {
    seat: Seat,
    answers: Vec<usize>,
    next: usize,
    log: Vec<String>,
    depth: u32,
}

impl Ctx {
    fn new(seat: Seat, answers: Vec<usize>) -> Ctx {
        Ctx {
            seat,
            answers,
            next: 0,
            log: Vec::new(),
            depth: 0,
        }
    }

    /// Ask the current player to pick one of the options (a single option is
    /// picked automatically).
    fn ask(&mut self, game: &Game, text: impl Into<String>, options: Vec<Opt>) -> R<usize> {
        match options.len() {
            0 => return illegal("There's nothing to choose from."),
            1 => return Ok(0),
            _ => {}
        }
        if let Some(&answer) = self.answers.get(self.next) {
            self.next += 1;
            return if answer < options.len() {
                Ok(answer)
            } else {
                illegal("That isn't one of the options.")
            };
        }
        let answers = (0..options.len()).collect();
        Err(Stop::Need(
            Prompt {
                text: text.into(),
                options,
                answers,
            },
            Box::new(game.clone()),
        ))
    }

    fn log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Distance in spaces between two cells (doubled-width coordinates).
pub fn hex_dist(a: Cell, b: Cell) -> u32 {
    let dx = (a.x - b.x).abs();
    let dy = (a.y - b.y).abs();
    (dy + ((dx - dy) / 2).max(0)) as u32
}

/// If `to` is in a straight line from `from`: the direction and the distance.
pub fn line_dir(from: Cell, to: Cell) -> Option<(usize, u32)> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if dy == 0 {
        if dx == 0 || dx % 2 != 0 {
            return None;
        }
        return Some((if dx > 0 { 0 } else { 3 }, (dx.abs() / 2) as u32));
    }
    if dx.abs() != dy.abs() {
        return None;
    }
    let dir = match (dx > 0, dy > 0) {
        (true, true) => 1,
        (false, true) => 2,
        (false, false) => 4,
        (true, false) => 5,
    };
    Some((dir, dy.unsigned_abs()))
}

fn on_board(cell: Cell) -> bool {
    data().board.kind(cell).is_some()
}

fn home_of(cell: Cell) -> Option<Seat> {
    match data().board.kind(cell) {
        Some(CellKind::Home(seat)) => Some(seat),
        _ => None,
    }
}

fn all_cells() -> impl Iterator<Item = Cell> {
    data().board.cells.iter().map(|(cell, _)| *cell)
}

/// "row 3, space 2", counting rows from the Letters side.
pub fn cell_label(cell: Cell) -> String {
    let board = &data().board;
    let top = board.cells.iter().map(|(c, _)| c.y).min().unwrap_or(0);
    let pos = board
        .cells
        .iter()
        .filter(|(c, _)| c.y == cell.y && c.x <= cell.x)
        .count();
    format!("row {}, space {}", cell.y - top + 1, pos)
}

fn effect_of(power: usize) -> Option<Effect> {
    data().effect(power)
}

fn is_active(power: usize) -> bool {
    data().powers[power].kind == Kind::Active
}

/// Powers whose effect involves "target unit" (only these care about *Remote*).
fn uses_targets(effect: Effect) -> bool {
    use Effect::*;
    matches!(
        effect,
        Push | Pull
            | Swap
            | Lift
            | Jump
            | Climb
            | Circle
            | Trade
            | Stun
            | Trigger
            | Dogpile
            | Backstab
            | ClickClick
            | Banish
            | Convert
            | Recall
            | Possess
    )
}

impl Unit {
    /// How many copies of a power the unit has.
    pub fn n(&self, effect: Effect) -> u32 {
        self.powers
            .iter()
            .filter(|p| effect_of(p.power) == Some(effect))
            .count() as u32
    }

    pub fn has(&self, effect: Effect) -> bool {
        self.n(effect) > 0
    }
}

// ---------------------------------------------------------------------------
// Passive effects that depend on the whole board
// ---------------------------------------------------------------------------

/// *Weak* and *Friendly*, which change how units count.
#[derive(Default)]
pub(crate) struct Mods {
    weak: HashMap<u32, u32>,
    friend: HashMap<u32, [bool; 2]>,
}

/// Everything passive powers do to other units.
#[derive(Default)]
pub(crate) struct Fx {
    mods: Mods,
    gripped: HashSet<u32>,
    jammed: HashSet<u32>,
    guarded: HashSet<u32>,
}

/// What a unit is currently affected by, for display.
#[derive(Serialize)]
pub struct Status {
    pub gripped: bool,
    pub jammed: bool,
    pub guarded: bool,
    pub outnumbered: bool,
    pub weight: u32,
    pub befriended: [bool; 2],
}

impl Game {
    fn unit(&self, id: u32) -> &Unit {
        self.units
            .iter()
            .find(|u| u.id == id)
            .expect("the unit exists")
    }

    fn unit_mut(&mut self, id: u32) -> &mut Unit {
        self.units
            .iter_mut()
            .find(|u| u.id == id)
            .expect("the unit exists")
    }

    fn find(&self, id: u32) -> R<&Unit> {
        match self.units.iter().find(|u| u.id == id) {
            Some(u) => Ok(u),
            None => illegal("That unit no longer exists."),
        }
    }

    fn alive(&self, id: u32) -> bool {
        self.units.iter().any(|u| u.id == id)
    }

    /// Ids of the units in a space, bottom of the stack first.
    fn ids_at(&self, cell: Cell) -> Vec<u32> {
        let mut units: Vec<&Unit> = self.units.iter().filter(|u| u.cell == cell).collect();
        units.sort_by_key(|u| u.z);
        units.into_iter().map(|u| u.id).collect()
    }

    fn top_at(&self, cell: Cell) -> Option<u32> {
        self.units
            .iter()
            .filter(|u| u.cell == cell)
            .max_by_key(|u| u.z)
            .map(|u| u.id)
    }

    fn on_top(&self, id: u32) -> bool {
        let u = self.unit(id);
        !self.units.iter().any(|o| o.cell == u.cell && o.z > u.z)
    }

    /// Units on top of their stacks. The others don't take part in anything
    /// except the unit cap.
    fn top_ids(&self) -> Vec<u32> {
        let mut ids = Vec::with_capacity(self.units.len());
        for u in &self.units {
            if !self.units.iter().any(|o| o.cell == u.cell && o.z > u.z) {
                ids.push(u.id);
            }
        }
        ids.sort_unstable();
        ids
    }

    /// The unit with the *Flag* power.
    fn flag_holder(&self) -> Option<u32> {
        self.units.iter().find(|u| u.is_flag()).map(|u| u.id)
    }

    /// Unit adjacency, including *Telepath* and *Bridge*.
    fn adjacent(&self, a: u32, b: u32) -> bool {
        if a == b {
            return false;
        }
        let (ua, ub) = (self.unit(a), self.unit(b));
        if hex_dist(ua.cell, ub.cell) == 1 {
            return true;
        }
        if ua.owner.is_some()
            && ua.owner == ub.owner
            && (ua.has(Effect::Telepath) || ub.has(Effect::Telepath))
        {
            return true;
        }
        self.units.iter().any(|c| {
            c.id != a
                && c.id != b
                && c.has(Effect::Bridge)
                && hex_dist(c.cell, ua.cell) == 1
                && hex_dist(c.cell, ub.cell) == 1
                && self.on_top(c.id)
        })
    }

    /// Top units adjacent to a unit.
    fn neighbours(&self, id: u32) -> Vec<u32> {
        self.top_ids()
            .into_iter()
            .filter(|&o| self.adjacent(id, o))
            .collect()
    }

    fn adjacent_to_flag(&self, id: u32) -> bool {
        self.flag_holder()
            .is_some_and(|f| f != id && self.adjacent(id, f))
    }

    /// Which players a unit counts for (its owner, and anyone befriending it).
    fn sides(&self, id: u32, mods: Option<&Mods>) -> [bool; 2] {
        let mut sides = [false, false];
        if let Some(owner) = self.unit(id).owner {
            sides[owner] = true;
        }
        if let Some(friend) = mods.and_then(|m| m.friend.get(&id)) {
            sides[0] |= friend[0];
            sides[1] |= friend[1];
        }
        sides
    }

    /// How many units a unit counts as (*Strong*, *Weak*).
    fn weight(&self, id: u32, mods: Option<&Mods>) -> u32 {
        let base = 1 + self.unit(id).n(Effect::Strong);
        let weak = mods.and_then(|m| m.weak.get(&id)).copied().unwrap_or(0);
        base.saturating_sub(weak)
    }

    fn outnumbered(&self, id: u32, mods: Option<&Mods>) -> bool {
        let me = self.sides(id, mods);
        let mut allies = self.weight(id, mods);
        let mut enemies = 0;
        for other in self.neighbours(id) {
            let them = self.sides(other, mods);
            let w = self.weight(other, mods);
            match me {
                [false, false] => {
                    if them == [false, false] {
                        allies += w
                    } else {
                        enemies += w
                    }
                }
                [true, true] => allies += w,
                _ => {
                    let side = if me[0] { 0 } else { 1 };
                    if them[side] {
                        allies += w
                    } else if them[1 - side] {
                        enemies += w
                    }
                }
            }
        }
        enemies > allies
    }

    /// The units a unit's powers target, nearest first. Without targeting
    /// powers that's the unit directly in front; with them, the union of what
    /// each one targets. `origin` is where the unit acts from (*Remote*).
    /// `mods` is None while working out *Weak* and *Friendly* themselves.
    fn targets(&self, user: u32, origin: Cell, facing: usize, mods: Option<&Mods>) -> Vec<u32> {
        use Effect::*;
        let u = self.unit(user);
        let anchor = if origin == u.cell {
            user
        } else {
            self.top_at(origin).unwrap_or(user)
        };
        let mut found: HashSet<u32> = HashSet::new();
        let mut modifiers: Vec<Effect> = u
            .powers
            .iter()
            .filter_map(|p| effect_of(p.power))
            .filter(|e| e.is_targeting())
            .collect();
        modifiers.dedup();
        if modifiers.is_empty() {
            found.extend(self.top_at(origin.step(facing)));
        }
        let tops = self.top_ids();
        for modifier in modifiers {
            match modifier {
                Reach => found.extend(self.top_at(origin.step(facing).step(facing))),
                Spin => found.extend(tops.iter().filter(|&&t| self.adjacent(anchor, t))),
                Beam => {
                    let mut cell = origin.step(facing);
                    while on_board(cell) {
                        found.extend(self.top_at(cell));
                        cell = cell.step(facing);
                    }
                }
                Chain => {
                    let mut seen = HashSet::from([anchor]);
                    let mut queue = VecDeque::from([anchor]);
                    while let Some(next) = queue.pop_front() {
                        for &t in &tops {
                            if !seen.contains(&t) && self.adjacent(next, t) {
                                seen.insert(t);
                                queue.push_back(t);
                                found.insert(t);
                            }
                        }
                    }
                }
                Halo => found.extend(
                    tops.iter()
                        .filter(|&&t| hex_dist(origin, self.unit(t).cell) == 2),
                ),
                Colorblind => found.extend(tops.iter().filter(|&&t| self.unit(t).owner.is_none())),
                Liability => {
                    if u.owner.is_some() {
                        found.extend(tops.iter().filter(|&&t| self.unit(t).owner == u.owner))
                    }
                }
                Bully => found.extend(tops.iter().filter(|&&t| self.outnumbered(t, mods))),
                Stalker => found.extend(tops.iter().filter(|&&t| self.neighbours(t).is_empty())),
                Spotlight => {
                    if let Some(flag) = self.flag_holder() {
                        found.extend(
                            tops.iter()
                                .filter(|&&t| t != flag && self.adjacent(t, flag)),
                        )
                    }
                }
                Vengeance => {
                    if let Some(owner) = u.owner {
                        for &(victim, killer) in &self.kills_last_turn {
                            if victim == owner
                                && self.alive(killer)
                                && self.on_top(killer)
                                && self.unit(killer).owner == Some(1 - owner)
                            {
                                found.insert(killer);
                            }
                        }
                    }
                }
                SixShooter => {
                    for dir in 0..6 {
                        let mut cell = origin.step(dir);
                        while on_board(cell) {
                            if let Some(t) = self.top_at(cell) {
                                found.insert(t);
                                break;
                            }
                            cell = cell.step(dir);
                        }
                    }
                }
                _ => {}
            }
        }
        found.remove(&user);
        let mut targets: Vec<u32> = found.into_iter().collect();
        targets.sort_by_key(|&t| (hex_dist(origin, self.unit(t).cell), t));
        targets
    }

    /// Work out what every passive power is currently doing.
    fn effects(&self) -> Fx {
        let tops = self.top_ids();
        let mut mods = Mods::default();
        for &h in &tops {
            let u = self.unit(h);
            let (weak, friendly) = (u.n(Effect::Weak), u.n(Effect::Friendly));
            if weak + friendly == 0 {
                continue;
            }
            for t in self.targets(h, u.cell, u.facing, None) {
                if weak > 0 {
                    *mods.weak.entry(t).or_default() += weak;
                }
                if let Some(owner) = u.owner
                    && friendly > 0
                    && self.unit(t).owner.is_none()
                {
                    mods.friend.entry(t).or_default()[owner] = true;
                }
            }
        }
        let mut fx = Fx {
            mods,
            ..Fx::default()
        };
        for &h in &tops {
            let u = self.unit(h);
            let (grip, jam, guard) = (
                u.has(Effect::Grip),
                u.has(Effect::Jammer),
                u.has(Effect::Lifeguard),
            );
            if !(grip || jam || guard) {
                continue;
            }
            for t in self.targets(h, u.cell, u.facing, Some(&fx.mods)) {
                if grip {
                    fx.gripped.insert(t);
                }
                if jam {
                    fx.jammed.insert(t);
                }
                if guard {
                    fx.guarded.insert(t);
                }
            }
        }
        fx
    }

    /// Status of every unit, for display.
    pub fn statuses(&self) -> HashMap<u32, Status> {
        let fx = self.effects();
        self.units
            .iter()
            .map(|u| {
                let status = Status {
                    gripped: fx.gripped.contains(&u.id),
                    jammed: fx.jammed.contains(&u.id),
                    guarded: fx.guarded.contains(&u.id),
                    outnumbered: self.on_top(u.id) && self.outnumbered(u.id, Some(&fx.mods)),
                    weight: self.weight(u.id, Some(&fx.mods)),
                    befriended: fx.mods.friend.get(&u.id).copied().unwrap_or_default(),
                };
                (u.id, status)
            })
            .collect()
    }

    /// Units adjacent to the flag for each player, counting *Strong*, *Weak*,
    /// *Friendly*, *Bridge* and *Telepath*.
    pub fn auto_flag_counts(&self) -> [u32; 2] {
        let Some(flag) = self.flag_holder() else {
            return [0, 0];
        };
        let fx = self.effects();
        let mut counts = [0, 0];
        for id in self.top_ids() {
            if id == flag || !self.adjacent(id, flag) {
                continue;
            }
            let sides = self.sides(id, Some(&fx.mods));
            let weight = self.weight(id, Some(&fx.mods));
            for seat in 0..2 {
                if sides[seat] {
                    counts[seat] += weight;
                }
            }
        }
        counts
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn run(base: &Game, action: &Action, seat: Seat, answers: &[usize]) -> (R<()>, Game, Vec<String>) {
    let mut game = base.clone();
    game.pending = None;
    let mut ctx = Ctx::new(seat, answers.to_vec());
    let result = game.resolve(seat, action, &mut ctx);
    (result, game, ctx.log)
}

/// Can the action be completed after these answers?
fn viable(base: &Game, action: &Action, seat: Seat, answers: Vec<usize>, budget: &mut u32) -> bool {
    if *budget == 0 {
        return true;
    }
    *budget -= 1;
    match run(base, action, seat, &answers).0 {
        Ok(()) => true,
        Err(Stop::Illegal(_)) => false,
        Err(Stop::Need(prompt, _)) => (0..prompt.options.len()).any(|i| {
            let mut next = answers.clone();
            next.push(i);
            viable(base, action, seat, next, budget)
        }),
    }
}

impl Game {
    /// Apply an action with automatic rules.
    pub(crate) fn auto_apply(
        &mut self,
        seat: Seat,
        action: &Action,
    ) -> Result<Vec<String>, ActionError> {
        match action {
            Action::Choose { index } => {
                let pending = self
                    .pending
                    .as_ref()
                    .ok_or_else(|| hard("There's nothing to choose."))?;
                if pending.seat != seat {
                    return Err(hard("It isn't your choice."));
                }
                let answer = *pending
                    .prompt
                    .answers
                    .get(*index)
                    .ok_or_else(|| hard("That isn't one of the options."))?;
                let mut answers = pending.answers.clone();
                answers.push(answer);
                let (base, action) = (pending.base.clone(), pending.action.clone());
                self.settle(base, action, seat, answers)
            }
            Action::Cancel => {
                let pending = self
                    .pending
                    .as_ref()
                    .ok_or_else(|| hard("There's nothing to cancel."))?;
                if pending.seat != seat {
                    return Err(hard("It isn't your choice."));
                }
                *self = (*pending.base).clone();
                Ok(vec![format!("{} changed their mind.", SEAT_NAMES[seat])])
            }
            _ => {
                if self.pending.is_some() {
                    return Err(hard("Answer the current question first."));
                }
                let base = Box::new(self.clone());
                self.settle(base, action.clone(), seat, Vec::new())
            }
        }
    }

    /// Replay the action with the answers so far: either it completes, or it
    /// becomes (or stays) pending on the next question.
    fn settle(
        &mut self,
        base: Box<Game>,
        action: Action,
        seat: Seat,
        mut answers: Vec<usize>,
    ) -> Result<Vec<String>, ActionError> {
        loop {
            let (result, game, log) = run(&base, &action, seat, &answers);
            match result {
                Ok(()) => {
                    *self = game;
                    self.pending = None;
                    return Ok(log);
                }
                Err(Stop::Illegal(message)) => return Err(hard(message)),
                Err(Stop::Need(mut prompt, preview)) => {
                    let mut budget = SEARCH_BUDGET;
                    let keep: Vec<usize> = (0..prompt.options.len())
                        .filter(|&i| {
                            let mut next = answers.clone();
                            next.push(i);
                            viable(&base, &action, seat, next, &mut budget)
                        })
                        .collect();
                    match keep.len() {
                        0 => return Err(hard("That can't be done: there's no way to finish it.")),
                        1 => {
                            answers.push(keep[0]);
                            continue;
                        }
                        _ => {}
                    }
                    prompt.options = keep.iter().map(|&i| prompt.options[i].clone()).collect();
                    prompt.answers = keep;
                    *self = (*base).clone();
                    self.pending = Some(Box::new(Pending {
                        base,
                        action,
                        seat,
                        answers,
                        prompt,
                        preview,
                    }));
                    return Ok(Vec::new());
                }
            }
        }
    }

    /// Whether an action could be completed now (with some answers to its prompts).
    pub fn is_legal(&self, seat: Seat, action: &Action) -> bool {
        if !self.automatic || self.pending.is_some() {
            return false;
        }
        let mut budget = SEARCH_BUDGET;
        viable(self, action, seat, Vec::new(), &mut budget)
    }

    /// Every legal action for a player, with automatic rules. While a prompt is
    /// waiting, the legal actions are its answers.
    pub fn legal_actions(&self, seat: Seat) -> Vec<Action> {
        if !self.automatic {
            return Vec::new();
        }
        if let Some(pending) = &self.pending {
            return if pending.seat == seat {
                (0..pending.prompt.options.len())
                    .map(|index| Action::Choose { index })
                    .collect()
            } else {
                Vec::new()
            };
        }
        if seat != self.current || matches!(self.phase, Phase::Over { .. }) {
            return Vec::new();
        }
        self.candidates(seat)
            .into_iter()
            .filter(|a| self.is_legal(seat, a))
            .collect()
    }

    /// Actions worth checking for legality.
    fn candidates(&self, seat: Seat) -> Vec<Action> {
        let mut out = Vec::new();
        match self.phase {
            Phase::Draft | Phase::Draw { .. } => {
                out.extend((0..self.faceup.len()).map(|slot| Action::TakeFaceup { slot }));
                if !self.deck.is_empty() || !self.discard.is_empty() {
                    out.push(Action::TakeDeck);
                }
            }
            Phase::Main => {
                if !self.spawned_this_turn {
                    for (cell, kind) in &data().board.cells {
                        if *kind == CellKind::Home(seat) && self.top_at(*cell).is_none() {
                            out.push(Action::Spawn {
                                cell: *cell,
                                force: false,
                            });
                        }
                    }
                }
                let tops = self.top_ids();
                let at_home: Vec<&Unit> = tops
                    .iter()
                    .map(|&id| self.unit(id))
                    .filter(|u| u.owner == Some(seat) && home_of(u.cell) == Some(seat))
                    .collect();
                let mut cards = self.hands[seat].clone();
                cards.sort();
                cards.dedup();
                for u in &at_home {
                    let replaces: Vec<Option<usize>> = if u.powers.len() < MAX_POWERS {
                        vec![None]
                    } else {
                        (0..u.powers.len()).map(Some).collect()
                    };
                    for &card in &cards {
                        for &replace in &replaces {
                            out.push(Action::ApplyHand {
                                card,
                                unit: u.id,
                                replace,
                                force: false,
                            });
                        }
                    }
                    for &power in &self.free_powers {
                        for &replace in &replaces {
                            out.push(Action::ApplyFree {
                                power,
                                unit: u.id,
                                replace,
                                force: false,
                            });
                        }
                    }
                    for index in 0..u.powers.len() {
                        out.push(Action::RemovePower {
                            unit: u.id,
                            index,
                            to_hand: false,
                            force: false,
                        });
                    }
                }
                for &id in &tops {
                    let u = self.unit(id);
                    let flag_ok = u.owner.is_none() && u.is_flag();
                    if (u.owner != Some(seat) && !flag_ok) || u.activations >= u.max_activations() {
                        continue;
                    }
                    let friends: Vec<u32> = if u.has(Effect::Remote) {
                        tops.iter()
                            .copied()
                            .filter(|&o| {
                                o != id && u.owner.is_some() && self.unit(o).owner == u.owner
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    for facing in 0..6 {
                        out.push(Action::Activate {
                            unit: id,
                            facing,
                            power: None,
                            origin: None,
                            force: false,
                        });
                        for (i, p) in u.powers.iter().enumerate() {
                            if !is_active(p.power) {
                                continue;
                            }
                            out.push(Action::Activate {
                                unit: id,
                                facing,
                                power: Some(i),
                                origin: None,
                                force: false,
                            });
                            if effect_of(p.power).is_some_and(uses_targets) {
                                for &origin in &friends {
                                    out.push(Action::Activate {
                                        unit: id,
                                        facing,
                                        power: Some(i),
                                        origin: Some(origin),
                                        force: false,
                                    });
                                }
                            }
                        }
                    }
                }
                out.push(Action::EndTurn { score: false });
            }
            Phase::Over { .. } => {}
        }
        out
    }

    /// The prompt waiting for an answer, if any.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.pending.as_ref().map(|p| &p.prompt)
    }
}

// ---------------------------------------------------------------------------
// Turn actions
// ---------------------------------------------------------------------------

impl Game {
    fn resolve(&mut self, seat: Seat, action: &Action, ctx: &mut Ctx) -> R<()> {
        if matches!(self.phase, Phase::Over { .. }) {
            return illegal("The game is over.");
        }
        if seat != self.current {
            return illegal(format!(
                "It's {} turn.",
                possessive(SEAT_NAMES[self.current])
            ));
        }
        let main = |game: &Game| match game.phase {
            Phase::Main => Ok(()),
            Phase::Draft => illegal("Finish the draft first."),
            Phase::Draw { .. } => illegal("Draw your cards first."),
            Phase::Over { .. } => illegal("The game is over."),
        };
        let me = SEAT_NAMES[seat];
        match *action {
            Action::TakeFaceup { .. } | Action::TakeDeck => {
                if self.phase == Phase::Main {
                    return illegal("You can only draw at the start of your turn.");
                }
                let mut log = Vec::new();
                self.apply_inner(seat, action, &mut log)
                    .map_err(|e| Stop::Illegal(e.message))?;
                ctx.log.extend(log);
            }

            Action::Spawn { cell, force } => {
                main(self)?;
                no_force(force)?;
                if data().board.kind(cell) != Some(CellKind::Home(seat)) {
                    return illegal("You can only spawn units in your own home spaces.");
                }
                if !self.ids_at(cell).is_empty() {
                    return illegal("That space is occupied.");
                }
                if self.spawned_this_turn {
                    return illegal("You can only spawn one unit per turn.");
                }
                self.spawned_this_turn = true;
                let id =
                    self.create_unit(Some(seat), cell, default_facing(seat), Vec::new(), ctx)?;
                if self.alive(id) {
                    ctx.log
                        .insert(0, format!("{me} spawned unit {}.", self.unit(id).label));
                }
            }

            Action::ApplyHand {
                card,
                unit,
                replace,
                force,
            } => {
                main(self)?;
                no_force(force)?;
                let Some(pos) = self.hands[seat].iter().position(|&c| c == card) else {
                    return illegal("That card isn't in your hand.");
                };
                if self.forbidden[seat].contains(&card) {
                    return illegal(format!("{} is forbidden this turn.", power_name(card)));
                }
                self.check_my_home_unit(unit, seat, "apply powers to")?;
                self.hands[seat].remove(pos);
                ctx.log(format!(
                    "{} gained {} from {} hand.",
                    cap(self.unit(unit).name()),
                    power_name(card),
                    possessive(me)
                ));
                self.equip(
                    unit,
                    PowerRef {
                        power: card,
                        card: true,
                    },
                    replace,
                    ctx,
                )?;
            }

            Action::ApplyFree {
                power,
                unit,
                replace,
                force,
            } => {
                main(self)?;
                no_force(force)?;
                if !self.free_powers.contains(&power) {
                    return illegal("That isn't a free power.");
                }
                self.check_my_home_unit(unit, seat, "apply powers to")?;
                let p = PowerRef { power, card: false };
                if replace.is_some_and(|r| self.unit(unit).powers.get(r) == Some(&p)) {
                    return illegal("That wouldn't change anything.");
                }
                ctx.log(format!(
                    "{} gained {} (free power).",
                    cap(self.unit(unit).name()),
                    power_name(power)
                ));
                self.equip(unit, p, replace, ctx)?;
            }

            Action::RemovePower {
                unit,
                index,
                to_hand,
                force,
            } => {
                main(self)?;
                no_force(force)?;
                if to_hand {
                    return illegal("Returning powers to your hand is a manual action.");
                }
                self.check_my_home_unit(unit, seat, "discard powers from")?;
                let u = self.unit(unit);
                let Some(&p) = u.powers.get(index) else {
                    return illegal("That unit doesn't have that power.");
                };
                let (owner, name) = (u.owner, u.name());
                self.unit_mut(unit).powers.remove(index);
                ctx.log(format!("{} discarded {}.", cap(name), power_name(p.power)));
                self.remove_from_board(p, owner, Some(unit), true, ctx)?;
            }

            Action::Activate {
                unit,
                facing,
                power,
                origin,
                force,
            } => {
                main(self)?;
                no_force(force)?;
                self.activate(seat, unit, facing, power, origin, ctx)?;
            }

            Action::EndTurn { .. } => {
                main(self)?;
                self.end_turn(seat, ctx)?;
            }

            Action::Choose { .. } | Action::Cancel | Action::SetMode { .. } => {
                return illegal("That can't be done now.");
            }

            _ => return illegal("That's only allowed with manual rules."),
        }
        self.restore_flag(ctx)
    }

    fn check_my_home_unit(&self, id: u32, seat: Seat, verb: &str) -> R<()> {
        let u = self.find(id)?;
        if u.owner != Some(seat) {
            return illegal(format!("You can only {verb} your own units."));
        }
        if home_of(u.cell) != Some(seat) {
            return illegal(format!("You can only {verb} units in your home spaces."));
        }
        if !self.on_top(id) {
            return illegal("That unit is underneath another one.");
        }
        Ok(())
    }

    /// Add a power to a unit at home, replacing one if it's full.
    fn equip(&mut self, id: u32, p: PowerRef, replace: Option<usize>, ctx: &mut Ctx) -> R<()> {
        let len = self.unit(id).powers.len();
        match replace {
            None if len >= MAX_POWERS => illegal(format!(
                "{} already has {MAX_POWERS} powers: choose one to replace.",
                cap(self.unit(id).name())
            )),
            None => {
                self.unit_mut(id).powers.push(p);
                Ok(())
            }
            Some(_) if len < MAX_POWERS => illegal("Only replace a power when the unit is full."),
            Some(r) if r >= len => illegal("That unit doesn't have that power."),
            Some(r) => {
                let old = std::mem::replace(&mut self.unit_mut(id).powers[r], p);
                ctx.log(format!(
                    "{} replaced {}.",
                    power_name(p.power),
                    power_name(old.power)
                ));
                let owner = self.unit(id).owner;
                self.remove_from_board(old, owner, Some(id), true, ctx)
            }
        }
    }

    fn activate(
        &mut self,
        seat: Seat,
        id: u32,
        facing: usize,
        power: Option<usize>,
        origin: Option<u32>,
        ctx: &mut Ctx,
    ) -> R<()> {
        if facing >= 6 {
            return illegal("Unknown direction.");
        }
        let u = self.find(id)?.clone();
        if !self.on_top(id) {
            return illegal("Units underneath a stack can't be activated.");
        }
        let flag_ok =
            u.owner.is_none() && u.is_flag() && u.powers.iter().any(|p| is_active(p.power));
        if u.owner != Some(seat) && !flag_ok {
            return illegal(
                "You can only activate your own units (and the flag, if it has an active power).",
            );
        }
        if u.activations >= u.max_activations() {
            return illegal("That unit has already been activated this turn.");
        }
        if power.is_none() && facing == u.facing {
            return illegal("Activating without turning or using a power wouldn't do anything.");
        }
        let origin_cell = match origin {
            None => u.cell,
            Some(o) => {
                if !u.has(Effect::Remote) {
                    return illegal("Only units with Remote can act from another space.");
                }
                let other = self.find(o)?;
                if o == id || other.owner.is_none() || other.owner != u.owner || !self.on_top(o) {
                    return illegal("Remote only works from the space of a friendly unit.");
                }
                let uses = power
                    .and_then(|i| u.powers.get(i))
                    .and_then(|p| effect_of(p.power))
                    .is_some_and(uses_targets);
                if !uses {
                    return illegal("Remote only changes which units a power targets.");
                }
                other.cell
            }
        };
        let name = cap(u.name());
        {
            let unit = self.unit_mut(id);
            unit.facing = facing;
            unit.activations += 1;
        }
        let turning = turn_text(u.facing, facing)
            .map(|t| format!(", turning {t},"))
            .unwrap_or_default();
        let Some(slot) = power else {
            ctx.log(format!(
                "{name} activated{}.",
                turning.trim_end_matches(',')
            ));
            return Ok(());
        };
        let Some(&p) = u.powers.get(slot) else {
            return illegal("That unit doesn't have that power.");
        };
        if !is_active(p.power) {
            return illegal("That's a passive power.");
        }
        if self.forbidden[seat].contains(&p.power) {
            return illegal(format!("{} is forbidden this turn.", power_name(p.power)));
        }
        if self.effects().jammed.contains(&id) {
            return illegal(format!("{name} is jammed and can't use active powers."));
        }
        let from = match origin {
            Some(o) => format!(" from {} space", possessive(&self.unit(o).name())),
            None => String::new(),
        };
        let text = &data().powers[p.power].text;
        ctx.log(format!(
            "{name} activated{turning} and used {}{from}: {text}",
            power_name(p.power)
        ));
        self.use_power(id, Some(slot), p.power, origin_cell, ctx)
    }

    fn end_turn(&mut self, seat: Seat, ctx: &mut Ctx) -> R<()> {
        // End-of-turn powers on the player's units (and on neutral units, which
        // have no turn of their own).
        let mut todo: Vec<(u32, Effect)> = Vec::new();
        for id in self.top_ids() {
            let u = self.unit(id);
            if u.owner == Some(seat) || u.owner.is_none() {
                for p in &u.powers {
                    if let Some(e @ (Effect::Heavy | Effect::Stinky)) = effect_of(p.power) {
                        todo.push((id, e));
                    }
                }
            }
        }
        loop {
            // Earlier effects can kill or bury units whose effects are still to come.
            todo.retain(|&(id, _)| self.alive(id) && self.on_top(id));
            if todo.is_empty() {
                break;
            }
            let mut choices = todo.clone();
            choices.dedup();
            let options = choices
                .iter()
                .map(|&(id, e)| Opt {
                    label: format!("{e:?} on {}", self.unit(id).name()),
                    unit: Some(id),
                    ..Opt::default()
                })
                .collect();
            let pick = choices[ctx.ask(self, "Which end-of-turn power happens next?", options)?];
            let index = todo
                .iter()
                .position(|&t| t == pick)
                .expect("the choice is in the list");
            let (id, effect) = todo.remove(index);
            match effect {
                Effect::Heavy => self.heavy(id, ctx)?,
                _ => self.stinky(id, ctx)?,
            }
        }

        let counts = self.auto_flag_counts();
        let (mine, theirs) = (counts[seat], counts[1 - seat]);
        let me = SEAT_NAMES[seat];
        if mine > theirs {
            self.scores[seat] += 1;
            ctx.log(format!(
                "{me} ended their turn and scored (flag: {mine} vs {theirs}). {me} has {} point{}.",
                self.scores[seat],
                if self.scores[seat] == 1 { "" } else { "s" }
            ));
        } else {
            ctx.log(format!(
                "{me} ended their turn without scoring (flag: {mine} vs {theirs})."
            ));
        }
        self.forbidden[seat].clear();
        if self.scores[seat] >= self.options.target_score {
            self.phase = Phase::Over { winner: seat };
            ctx.log(format!("{me} wins!"));
        } else {
            self.current = 1 - seat;
            let mut log = Vec::new();
            self.begin_turn(&mut log);
            ctx.log.extend(log);
        }
        Ok(())
    }

    /// *Heavy*: each unit two spaces away moves one space closer, if it can.
    fn heavy(&mut self, h: u32, ctx: &mut Ctx) -> R<()> {
        let centre = self.unit(h).cell;
        let others: Vec<u32> = self
            .top_ids()
            .into_iter()
            .filter(|&t| t != h && hex_dist(self.unit(t).cell, centre) == 2)
            .collect();
        for t in others {
            if !self.alive(h) || !self.on_top(h) {
                break;
            }
            if !self.alive(t) || !self.on_top(t) {
                continue;
            }
            let (centre, from) = (self.unit(h).cell, self.unit(t).cell);
            if hex_dist(from, centre) != 2 {
                continue;
            }
            let fx = self.effects();
            let dests: Vec<Cell> = centre
                .neighbors()
                .filter(|&c| hex_dist(c, from) == 1 && self.can_move(&fx, t, c, h, false))
                .collect();
            if dests.is_empty() {
                continue;
            }
            let name = self.unit(t).name();
            let i = ctx.ask(
                self,
                format!("Heavy: where does {name} move?"),
                dests.iter().map(|&c| Opt::cell(c)).collect(),
            )?;
            self.do_moves(&[(t, dests[i])], h, ctx)?;
        }
        Ok(())
    }

    /// *Stinky*: each adjacent unit moves one space directly away, if it can.
    fn stinky(&mut self, s: u32, ctx: &mut Ctx) -> R<()> {
        let centre = self.unit(s).cell;
        let pushes: Vec<(u32, Cell)> = (0..6)
            .filter_map(|d| {
                self.top_at(centre.step(d))
                    .map(|t| (t, centre.step(d).step(d)))
            })
            .collect();
        for (t, to) in pushes {
            if !self.alive(s) {
                break;
            }
            if self.alive(t) && self.on_top(t) && self.can_move(&self.effects(), t, to, s, false) {
                self.do_moves(&[(t, to)], s, ctx)?;
            }
        }
        Ok(())
    }
}

fn no_force(force: bool) -> R<()> {
    if force {
        illegal("Rules can't be overridden with automatic rules.")
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Moving, creating and killing units
// ---------------------------------------------------------------------------

impl Game {
    /// Can `by`'s power move unit `id` to `to`? (`by == id` when it moves itself.)
    fn can_move(&self, fx: &Fx, id: u32, to: Cell, by: u32, stack: bool) -> bool {
        let u = self.unit(id);
        if !on_board(to) || to == u.cell {
            return false;
        }
        if !stack && self.top_at(to).is_some() {
            return false;
        }
        if let Some(home) = home_of(to)
            && u.owner != Some(home)
        {
            return false;
        }
        if by == id {
            if fx.gripped.contains(&id) {
                return false;
            }
        } else if u.has(Effect::Stubborn) {
            return false;
        }
        if u.is_flag() && u.moved + hex_dist(u.cell, to) > 1 {
            return false;
        }
        true
    }

    /// Move units (all at once), then resolve what the moves trigger.
    fn do_moves(&mut self, moves: &[(u32, Cell)], by: u32, ctx: &mut Ctx) -> R<()> {
        let before = self.twitchy_snapshot();
        let mut vacated = Vec::new();
        for &(id, to) in moves {
            let z = self.bump_z();
            let u = self.unit_mut(id);
            let from = u.cell;
            u.moved += hex_dist(from, to);
            u.cell = to;
            u.z = z;
            vacated.push((id, from));
        }
        self.sort_units();
        for &(id, _) in &vacated {
            let name = cap(self.unit(id).name());
            ctx.log(if id == by {
                format!("{name} moved.")
            } else {
                format!("{name} was moved.")
            });
        }
        for (id, from) in vacated {
            self.after_move(id, from, by, ctx)?;
        }
        self.twitchy_check(before, ctx)
    }

    fn after_move(&mut self, id: u32, from: Cell, by: u32, ctx: &mut Ctx) -> R<()> {
        if ctx.depth >= MAX_DEPTH {
            return Ok(());
        }
        ctx.depth += 1;
        if by != id && self.alive(id) && self.alive(by) && self.unit(id).has(Effect::Spiky) {
            ctx.log(format!("{} is spiky.", cap(self.unit(id).name())));
            self.kill_units(vec![by], Some(id), true, ctx)?;
        }
        if self.alive(id) && self.unit(id).has(Effect::Leader) && self.ids_at(from).is_empty() {
            let fx = self.effects();
            let followers: Vec<u32> = self
                .neighbours(id)
                .into_iter()
                .filter(|&f| self.can_move(&fx, f, from, id, false))
                .collect();
            if !followers.is_empty() {
                let leader = self.unit(id).name();
                let options = followers.iter().map(|&f| self.unit_opt(f)).collect();
                let i = ctx.ask(
                    self,
                    format!("Leader: which unit follows {leader} into the space it left?"),
                    options,
                )?;
                self.do_moves(&[(followers[i], from)], id, ctx)?;
            }
        }
        if self.alive(id)
            && self.unit(id).has(Effect::Trail)
            && self.ids_at(from).is_empty()
            && home_of(from).is_none()
        {
            self.create_unit(None, from, 0, Vec::new(), ctx)?;
            ctx.log(format!(
                "{} left a neutral unit behind (Trail).",
                cap(self.unit(id).name())
            ));
        }
        ctx.depth -= 1;
        Ok(())
    }

    fn unit_opt(&self, id: u32) -> Opt {
        let u = self.unit(id);
        Opt {
            label: cap(u.name()),
            unit: Some(id),
            ..Opt::default()
        }
    }

    /// Ids of each *Twitchy* unit and the enemies next to it.
    fn twitchy_snapshot(&self) -> Vec<(u32, Vec<u32>)> {
        self.top_ids()
            .into_iter()
            .filter(|&t| self.unit(t).has(Effect::Twitchy) && self.unit(t).owner.is_some())
            .map(|t| (t, self.adjacent_enemies(t)))
            .collect()
    }

    fn adjacent_enemies(&self, id: u32) -> Vec<u32> {
        let u = self.unit(id);
        let Some(owner) = u.owner else {
            return Vec::new();
        };
        (0..6)
            .filter_map(|d| self.top_at(u.cell.step(d)))
            .filter(|&e| self.unit(e).owner == Some(1 - owner))
            .collect()
    }

    /// *Twitchy*: turn to face an enemy that has just become adjacent.
    fn twitchy_check(&mut self, before: Vec<(u32, Vec<u32>)>, ctx: &mut Ctx) -> R<()> {
        for (t, _) in self.twitchy_snapshot() {
            let old: &[u32] = before
                .iter()
                .find(|(id, _)| *id == t)
                .map(|(_, e)| e.as_slice())
                .unwrap_or(&[]);
            let new: Vec<u32> = self
                .adjacent_enemies(t)
                .into_iter()
                .filter(|e| !old.contains(e))
                .collect();
            if new.is_empty() {
                continue;
            }
            let name = cap(self.unit(t).name());
            let options = new.iter().map(|&e| self.unit_opt(e)).collect();
            let i = ctx.ask(
                self,
                format!("Twitchy: which enemy does {name} turn to face?"),
                options,
            )?;
            let (cell, target) = (self.unit(t).cell, self.unit(new[i]).cell);
            if let Some((dir, _)) = line_dir(cell, target) {
                self.unit_mut(t).facing = dir;
                ctx.log(format!(
                    "{name} turned to face {} (Twitchy).",
                    self.unit(new[i]).name()
                ));
            }
        }
        Ok(())
    }

    /// Create a unit. If its owner now has six, they kill one of theirs.
    fn create_unit(
        &mut self,
        owner: Option<Seat>,
        cell: Cell,
        facing: usize,
        powers: Vec<PowerRef>,
        ctx: &mut Ctx,
    ) -> R<u32> {
        let before = self.twitchy_snapshot();
        let index = self.add_unit(owner, cell, facing, powers);
        let id = self.units[index].id;
        self.sort_units();
        if let Some(owner) = owner {
            if self.unit(id).label.is_empty() {
                self.unit_mut(id).label = "+".into();
            }
            self.enforce_unit_cap(owner, ctx)?;
        }
        self.twitchy_check(before, ctx)?;
        Ok(id)
    }

    fn enforce_unit_cap(&mut self, owner: Seat, ctx: &mut Ctx) -> R<()> {
        let mine: Vec<u32> = self
            .units
            .iter()
            .filter(|u| u.owner == Some(owner))
            .map(|u| u.id)
            .collect();
        if mine.len() <= 5 {
            return Ok(());
        }
        let options = mine.iter().map(|&id| self.unit_opt(id)).collect();
        let i = ctx.ask(
            self,
            format!("{} has six units: which one dies?", SEAT_NAMES[owner]),
            options,
        )?;
        self.kill_units(vec![mine[i]], None, false, ctx)?;
        for id in mine {
            if self.alive(id) && self.unit(id).label == "+" {
                let label = self.free_label(owner).unwrap_or("+").to_string();
                self.unit_mut(id).label = label;
            }
        }
        Ok(())
    }

    /// Kill units at once. `killer` is the unit whose power did it (for
    /// *Vengeance*); `by_power` is false for the rules' own kills.
    fn kill_units(
        &mut self,
        ids: Vec<u32>,
        killer: Option<u32>,
        by_power: bool,
        ctx: &mut Ctx,
    ) -> R<()> {
        let before = self.twitchy_snapshot();
        let fx = self.effects();
        let pacifist = self
            .top_ids()
            .into_iter()
            .any(|p| self.unit(p).has(Effect::Pacifist) && self.adjacent_to_flag(p));
        let killer_owner = killer
            .filter(|&k| self.alive(k))
            .and_then(|k| self.unit(k).owner);
        let mut explosions: Vec<(u32, Vec<u32>)> = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id) || !self.alive(id) {
                continue;
            }
            let u = self.unit(id).clone();
            let name = cap(u.name());
            if by_power && fx.guarded.contains(&id) {
                ctx.log(format!("{name} can't be killed (Lifeguard)."));
                continue;
            }
            if by_power
                && (u.has(Effect::Phoenix) || pacifist)
                && let Some(owner) = u.owner
            {
                let homes: Vec<Cell> = all_cells()
                    .filter(|&c| home_of(c) == Some(owner) && self.top_at(c).is_none())
                    .collect();
                if !homes.is_empty() {
                    let why = if u.has(Effect::Phoenix) {
                        "Phoenix"
                    } else {
                        "Pacifist"
                    };
                    let options = homes.iter().map(|&c| Opt::cell(c)).collect();
                    let i = ctx.ask(
                        self,
                        format!("{why}: where does {name} return to?"),
                        options,
                    )?;
                    let z = self.bump_z();
                    let unit = self.unit_mut(id);
                    unit.cell = homes[i];
                    unit.z = z;
                    self.sort_units();
                    ctx.log(format!("{name} returned home instead of dying ({why})."));
                    continue;
                }
            }
            if u.has(Effect::Explosive) && self.on_top(id) {
                explosions.push((id, self.targets(id, u.cell, u.facing, Some(&fx.mods))));
            }
            if let (Some(k), Some(victim), Some(ko)) = (killer, u.owner, killer_owner)
                && ko != victim
            {
                self.kills_this_turn.push((victim, k));
            }
            self.units.retain(|x| x.id != id);
            ctx.log(format!("{name} was killed."));
            for p in u.powers {
                self.remove_from_board(p, u.owner, None, true, ctx)?;
            }
        }
        for (source, targets) in explosions {
            let targets: Vec<u32> = targets.into_iter().filter(|&t| self.alive(t)).collect();
            if !targets.is_empty() {
                ctx.log("It exploded (Explosive).");
                self.kill_units(targets, Some(source), true, ctx)?;
            }
        }
        self.twitchy_check(before, ctx)?;
        self.restore_flag(ctx)
    }

    /// A power leaves the board: to a *Vacuum* unit, a hand (*Recycle*), the
    /// discard pile, or (free powers) nowhere. The *Flag* power is handled by
    /// `restore_flag`. Powers knocked off a full unit skip *Vacuum* (`vacuum`
    /// false), so two full vacuums can't pass powers back and forth forever.
    fn remove_from_board(
        &mut self,
        p: PowerRef,
        owner: Option<Seat>,
        from: Option<u32>,
        vacuum: bool,
        ctx: &mut Ctx,
    ) -> R<()> {
        if p.power == data().flag {
            return Ok(());
        }
        if vacuum {
            let vacuums: Vec<u32> = self
                .top_ids()
                .into_iter()
                .filter(|&v| {
                    Some(v) != from && self.unit(v).has(Effect::Vacuum) && self.adjacent_to_flag(v)
                })
                .collect();
            if !vacuums.is_empty() {
                let options = vacuums.iter().map(|&v| self.unit_opt(v)).collect();
                let i = ctx.ask(
                    self,
                    format!("Vacuum: which unit gets {}?", power_name(p.power)),
                    options,
                )?;
                let v = vacuums[i];
                ctx.log(format!(
                    "{} went to {} (Vacuum).",
                    power_name(p.power),
                    self.unit(v).name()
                ));
                return self.give_power(v, p, ctx);
            }
        }
        if !p.card {
            return Ok(());
        }
        if let Some(owner) = owner
            && self.top_ids().into_iter().any(|r| {
                self.unit(r).owner == Some(owner)
                    && self.unit(r).has(Effect::Recycle)
                    && self.adjacent_to_flag(r)
            })
        {
            self.add_to_hand(owner, p.power, true);
            ctx.log(format!(
                "{} went back to {} hand (Recycle).",
                power_name(p.power),
                possessive(SEAT_NAMES[owner])
            ));
            return Ok(());
        }
        self.discard.push(p.power);
        Ok(())
    }

    /// Add a power to a unit, asking which power to replace if it's full.
    fn give_power(&mut self, id: u32, p: PowerRef, ctx: &mut Ctx) -> R<()> {
        if self.unit(id).powers.len() < MAX_POWERS {
            self.unit_mut(id).powers.push(p);
            return Ok(());
        }
        let name = self.unit(id).name();
        let options = self
            .unit(id)
            .powers
            .iter()
            .map(|q| Opt::power(power_name(q.power), q.power))
            .collect();
        let i = ctx.ask(
            self,
            format!(
                "{} is full: which power does {} replace?",
                cap(name.clone()),
                power_name(p.power)
            ),
            options,
        )?;
        let old = std::mem::replace(&mut self.unit_mut(id).powers[i], p);
        ctx.log(format!(
            "{} replaced {} on {name}.",
            power_name(p.power),
            power_name(old.power)
        ));
        let owner = self.unit(id).owner;
        self.remove_from_board(old, owner, Some(id), false, ctx)
    }

    /// If the *Flag* power is ever removed from the board, kill any unit on F
    /// and put a new flag there.
    fn restore_flag(&mut self, ctx: &mut Ctx) -> R<()> {
        if self.flag_holder().is_some() {
            return Ok(());
        }
        let f = data().board.flag;
        let on_f = self.ids_at(f);
        if !on_f.is_empty() {
            ctx.log("The Flag power left the board.");
            self.kill_units(on_f, None, false, ctx)?;
        }
        if self.flag_holder().is_some() {
            return Ok(());
        }
        let before = self.twitchy_snapshot();
        self.add_unit(
            None,
            f,
            0,
            vec![PowerRef {
                power: data().flag,
                card: false,
            }],
        );
        self.sort_units();
        ctx.log("A new flag was placed on F.");
        self.twitchy_check(before, ctx)
    }
}

// ---------------------------------------------------------------------------
// Powers
// ---------------------------------------------------------------------------

impl Game {
    /// Move the user itself somewhere (a single destination).
    fn move_self(&mut self, user: u32, to: Cell, why: &str, ctx: &mut Ctx) -> R<()> {
        if !self.can_move(&self.effects(), user, to, user, false) {
            return illegal(why.to_string());
        }
        self.do_moves(&[(user, to)], user, ctx)
    }

    /// Let the player pick where the user moves to.
    fn move_self_to_one_of(
        &mut self,
        user: u32,
        options: Vec<Cell>,
        text: &str,
        ctx: &mut Ctx,
    ) -> R<()> {
        let fx = self.effects();
        let options: Vec<Cell> = options
            .into_iter()
            .filter(|&c| self.can_move(&fx, user, c, user, false))
            .collect();
        if options.is_empty() {
            return illegal("There's nowhere to move to.");
        }
        let i = ctx.ask(self, text, options.iter().map(|&c| Opt::cell(c)).collect())?;
        self.do_moves(&[(user, options[i])], user, ctx)
    }

    /// For powers that act on a single target: if several units are targeted,
    /// the player picks one (only targets the power can work on are offered).
    fn pick_target(
        &mut self,
        user: u32,
        origin: Cell,
        text: &str,
        ok: impl Fn(&Game, u32) -> bool,
        ctx: &mut Ctx,
    ) -> R<u32> {
        let u = self.unit(user);
        let targets: Vec<u32> = self.targets(user, origin, u.facing, Some(&self.effects().mods));
        if targets.is_empty() {
            return illegal("There's no target.");
        }
        let targets: Vec<u32> = targets.into_iter().filter(|&t| ok(self, t)).collect();
        if targets.is_empty() {
            return illegal("The power can't work on its target.");
        }
        let options = targets.iter().map(|&t| self.unit_opt(t)).collect();
        Ok(targets[ctx.ask(self, text, options)?])
    }

    fn all_targets(&self, user: u32, origin: Cell) -> R<Vec<u32>> {
        let targets = self.targets(
            user,
            origin,
            self.unit(user).facing,
            Some(&self.effects().mods),
        );
        if targets.is_empty() {
            illegal("There's no target.")
        } else {
            Ok(targets)
        }
    }

    /// Use an active power. `slot` is where the power sits on the user (None
    /// when borrowed with *Trigger*). `origin` is where it acts from (*Remote*).
    fn use_power(
        &mut self,
        user: u32,
        slot: Option<usize>,
        power: usize,
        origin: Cell,
        ctx: &mut Ctx,
    ) -> R<()> {
        use Effect::*;
        let Some(effect) = effect_of(power) else {
            return illegal("That power has no automatic effect.");
        };
        let u = self.unit(user).clone();
        let (cell, facing) = (u.cell, u.facing);
        let ahead = cell.step(facing);
        match effect {
            Walk => self.move_self(user, ahead, "There's no empty space in front.", ctx),
            Run => self.move_self(
                user,
                ahead.step(facing),
                "The space one space in front isn't empty.",
                ctx,
            ),
            Rush => {
                let fx = self.effects();
                let mut to = None;
                let mut next = ahead;
                while self.can_move(&fx, user, next, user, false) {
                    to = Some(next);
                    next = next.step(facing);
                }
                match to {
                    Some(to) => self.do_moves(&[(user, to)], user, ctx),
                    None => illegal("Something is blocking the way."),
                }
            }
            Catapult => {
                let Some(owner) = u.owner else {
                    return illegal("Neutral units have no home.");
                };
                if home_of(cell) != Some(owner) {
                    return illegal("Catapult only works from your home space.");
                }
                let options = all_cells()
                    .filter(|&c| home_of(c) != Some(1 - owner))
                    .collect();
                self.move_self_to_one_of(user, options, "Catapult: where to?", ctx)
            }
            Bungee => {
                let Some(owner) = u.owner else {
                    return illegal("Neutral units have no home.");
                };
                let options = all_cells().filter(|&c| home_of(c) == Some(owner)).collect();
                self.move_self_to_one_of(user, options, "Bungee: which home space?", ctx)
            }
            Burrow => {
                let options = all_cells()
                    .filter(|&c| {
                        !c.neighbors()
                            .any(|n| self.units.iter().any(|o| o.id != user && o.cell == n))
                    })
                    .collect();
                self.move_self_to_one_of(user, options, "Burrow: where to?", ctx)
            }
            Circle => {
                let target = self.pick_target(
                    user,
                    origin,
                    "Circle around which unit?",
                    |g, t| {
                        let fx = g.effects();
                        g.unit(t)
                            .cell
                            .neighbors()
                            .any(|c| g.can_move(&fx, user, c, user, false))
                    },
                    ctx,
                )?;
                let options = self.unit(target).cell.neighbors().collect();
                self.move_self_to_one_of(user, options, "Circle: where to?", ctx)
            }
            Jump => {
                let landing = |g: &Game, t: u32| {
                    line_dir(cell, g.unit(t).cell).map(|(d, _)| g.unit(t).cell.step(d))
                };
                let target = self.pick_target(
                    user,
                    origin,
                    "Jump over which unit?",
                    |g, t| {
                        landing(g, t)
                            .is_some_and(|c| g.can_move(&g.effects(), user, c, user, false))
                    },
                    ctx,
                )?;
                let to = landing(self, target).expect("checked above");
                self.do_moves(&[(user, to)], user, ctx)
            }
            Climb => {
                let target = self.pick_target(
                    user,
                    origin,
                    "Climb on top of which unit?",
                    |g, t| g.can_move(&g.effects(), user, g.unit(t).cell, user, true),
                    ctx,
                )?;
                let to = self.unit(target).cell;
                self.do_moves(&[(user, to)], user, ctx)
            }
            Swap => {
                let ok = |g: &Game, t: u32| {
                    let fx = g.effects();
                    let there = g.unit(t).cell;
                    g.can_move(&fx, user, there, user, true) && g.can_move(&fx, t, cell, user, true)
                };
                let target = self.pick_target(user, origin, "Swap with which unit?", ok, ctx)?;
                let there = self.unit(target).cell;
                self.do_moves(&[(user, there), (target, cell)], user, ctx)
            }
            Push => {
                let targets = self.all_targets(user, origin)?;
                let mut first_space = None;
                for t in targets {
                    if !self.alive(user) {
                        break;
                    }
                    if !self.alive(t) || !self.on_top(t) {
                        continue;
                    }
                    let from = self.unit(t).cell;
                    let Some((dir, _)) = line_dir(cell, from) else {
                        return illegal("Push only works on units in a straight line.");
                    };
                    let to = from.step(dir);
                    if !self.can_move(&self.effects(), t, to, user, false) {
                        return illegal("There's no empty space behind the pushed unit.");
                    }
                    first_space.get_or_insert(from);
                    self.do_moves(&[(t, to)], user, ctx)?;
                }
                match first_space {
                    Some(space) if self.alive(user) => {
                        if !self.can_move(&self.effects(), user, space, user, true) {
                            return illegal("The unit can't follow into the space.");
                        }
                        self.do_moves(&[(user, space)], user, ctx)
                    }
                    _ => Ok(()),
                }
            }
            Pull => {
                let targets = self.all_targets(user, origin)?;
                let Some((dir, _)) = line_dir(cell, self.unit(targets[0]).cell) else {
                    return illegal("Pull needs the nearest target in a straight line.");
                };
                let back = cell.step((dir + 3) % 6);
                self.move_self(user, back, "There's no empty space behind.", ctx)?;
                for t in targets {
                    if !self.alive(user) {
                        break;
                    }
                    if !self.alive(t) || !self.on_top(t) {
                        continue;
                    }
                    if !self.can_move(&self.effects(), t, cell, user, true) {
                        return illegal("A target can't be pulled into that space.");
                    }
                    self.do_moves(&[(t, cell)], user, ctx)?;
                }
                Ok(())
            }
            Lift => {
                for t in self.all_targets(user, origin)? {
                    if !self.alive(user) {
                        break;
                    }
                    if !self.alive(t) || !self.on_top(t) {
                        continue;
                    }
                    let fx = self.effects();
                    let options: Vec<Cell> = self
                        .unit(t)
                        .cell
                        .neighbors()
                        .filter(|&c| self.can_move(&fx, t, c, user, false))
                        .collect();
                    if options.is_empty() {
                        return illegal("A target has nowhere to be lifted to.");
                    }
                    let name = self.unit(t).name();
                    let i = ctx.ask(
                        self,
                        format!("Lift: where does {name} go?"),
                        options.iter().map(|&c| Opt::cell(c)).collect(),
                    )?;
                    self.do_moves(&[(t, options[i])], user, ctx)?;
                }
                Ok(())
            }
            Banish => {
                for t in self.all_targets(user, origin)? {
                    if !self.alive(user) {
                        break;
                    }
                    if !self.alive(t) || !self.on_top(t) {
                        continue;
                    }
                    let Some(owner) = self.unit(t).owner else {
                        return illegal("Neutral units have no home to be banished to.");
                    };
                    let fx = self.effects();
                    let options: Vec<Cell> = all_cells()
                        .filter(|&c| {
                            home_of(c) == Some(owner) && self.can_move(&fx, t, c, user, false)
                        })
                        .collect();
                    if options.is_empty() {
                        return illegal("A target has no empty home space.");
                    }
                    let name = self.unit(t).name();
                    let i = ctx.ask(
                        self,
                        format!("Banish: where does {name} go?"),
                        options.iter().map(|&c| Opt::cell(c)).collect(),
                    )?;
                    self.do_moves(&[(t, options[i])], user, ctx)?;
                }
                Ok(())
            }
            Trade => {
                let mine = u.powers.clone();
                let target = self.pick_target(
                    user,
                    origin,
                    "Trade with which unit?",
                    |g, t| {
                        let theirs = &g.unit(t).powers;
                        !theirs.is_empty()
                            && mine
                                .iter()
                                .any(|p| !theirs.iter().any(|q| q.power == p.power))
                    },
                    ctx,
                )?;
                let theirs = self.unit(target).powers.clone();
                let mut pairs = Vec::new();
                for (i, p) in mine.iter().enumerate() {
                    if theirs.iter().any(|q| q.power == p.power) {
                        continue;
                    }
                    for (j, q) in theirs.iter().enumerate() {
                        pairs.push((
                            i,
                            j,
                            Opt::text(format!(
                                "Give {}, take {}",
                                power_name(p.power),
                                power_name(q.power)
                            )),
                        ));
                    }
                }
                let options = pairs.iter().map(|(_, _, o)| o.clone()).collect();
                let k = ctx.ask(self, "Trade: which powers?", options)?;
                let (i, j, _) = pairs[k];
                let (p, q) = (mine[i], theirs[j]);
                self.unit_mut(user).powers[i] = q;
                self.unit_mut(target).powers[j] = p;
                ctx.log(format!(
                    "{} traded {} for {}.",
                    cap(u.name()),
                    power_name(p.power),
                    power_name(q.power)
                ));
                Ok(())
            }
            Stun => {
                let targets = self.all_targets(user, origin)?;
                if targets.iter().any(|&t| self.unit(t).powers.is_empty()) {
                    return illegal("A target has no powers to remove.");
                }
                for t in targets {
                    if !self.alive(t) || self.unit(t).powers.is_empty() {
                        continue;
                    }
                    let name = self.unit(t).name();
                    let options = self
                        .unit(t)
                        .powers
                        .iter()
                        .map(|p| Opt::power(power_name(p.power), p.power))
                        .collect();
                    let i = ctx.ask(
                        self,
                        format!("Stun: which power does {name} lose?"),
                        options,
                    )?;
                    let p = self.unit_mut(t).powers.remove(i);
                    ctx.log(format!("{} lost {}.", cap(name), power_name(p.power)));
                    let owner = self.unit(t).owner;
                    self.remove_from_board(p, owner, Some(t), true, ctx)?;
                }
                Ok(())
            }
            Trigger => {
                let seat = ctx.seat;
                let usable = |g: &Game, t: u32| -> Vec<usize> {
                    g.unit(t)
                        .powers
                        .iter()
                        .map(|p| p.power)
                        .filter(|&p| {
                            is_active(p)
                                && !matches!(effect_of(p), Some(Trigger | Mystery) | None)
                                && !g.forbidden[seat].contains(&p)
                        })
                        .collect()
                };
                let target = self.pick_target(
                    user,
                    origin,
                    "Trigger which unit's power?",
                    |g, t| !usable(g, t).is_empty(),
                    ctx,
                )?;
                let powers = usable(self, target);
                let options = powers
                    .iter()
                    .map(|&p| Opt::power(power_name(p), p))
                    .collect();
                let p = powers[ctx.ask(self, "Trigger: which power?", options)?];
                ctx.log(format!("{} borrowed {}.", cap(u.name()), power_name(p)));
                self.use_power(user, None, p, origin, ctx)
            }
            Mystery => {
                let Some(slot) = slot else {
                    return illegal("Mystery can only replace itself.");
                };
                let seat = ctx.seat;
                let mut cards: Vec<usize> = self.hands[seat]
                    .iter()
                    .copied()
                    .filter(|c| !self.forbidden[seat].contains(c))
                    .collect();
                cards.sort();
                cards.dedup();
                if cards.is_empty() {
                    return illegal("You have no cards to replace it with.");
                }
                let options = cards
                    .iter()
                    .map(|&c| Opt::power(power_name(c), c))
                    .collect();
                let card = cards[ctx.ask(self, "Mystery: replace it with which card?", options)?];
                let pos = self.hands[seat]
                    .iter()
                    .position(|&c| c == card)
                    .expect("the card is in the hand");
                self.hands[seat].remove(pos);
                let old = std::mem::replace(
                    &mut self.unit_mut(user).powers[slot],
                    PowerRef {
                        power: card,
                        card: true,
                    },
                );
                ctx.log(format!("Mystery became {}.", power_name(card)));
                self.remove_from_board(old, u.owner, Some(user), true, ctx)
            }
            Dogpile => {
                let targets = self.all_targets(user, origin)?;
                let fx = self.effects();
                if !targets.iter().all(|&t| self.outnumbered(t, Some(&fx.mods))) {
                    return illegal("Every target has to be outnumbered.");
                }
                self.kill_units(targets, Some(user), true, ctx)
            }
            Backstab => {
                let targets = self.all_targets(user, origin)?;
                for &t in &targets {
                    let there = self.unit(t).cell;
                    let behind =
                        line_dir(cell, there).and_then(|(d, _)| self.top_at(there.step(d)));
                    let friendly =
                        behind.is_some_and(|b| u.owner.is_some() && self.unit(b).owner == u.owner);
                    if !friendly {
                        return illegal("Every target needs a friendly unit on its far side.");
                    }
                }
                self.kill_units(targets, Some(user), true, ctx)
            }
            ClickClick => {
                let targets = self.targets(user, origin, facing, Some(&self.effects().mods));
                let mut options = vec![Opt::text("Add a token")];
                if u.tokens > 0 && !targets.is_empty() {
                    options.push(Opt::text("Remove a token and kill the target"));
                }
                if ctx.ask(self, "Click Click:", options)? == 0 {
                    self.unit_mut(user).tokens += 1;
                    ctx.log(format!("{} added a token.", cap(u.name())));
                    Ok(())
                } else {
                    self.unit_mut(user).tokens -= 1;
                    self.kill_units(targets, Some(user), true, ctx)
                }
            }
            Neutralize => {
                let victims: Vec<u32> = self
                    .units
                    .iter()
                    .filter(|o| o.powers.is_empty())
                    .map(|o| o.id)
                    .collect();
                if victims.is_empty() {
                    return illegal("There are no powerless units.");
                }
                self.kill_units(victims, Some(user), true, ctx)
            }
            Nuke => {
                if !self.adjacent_to_flag(user) {
                    return illegal("Nuke only works next to the flag.");
                }
                let mut options = vec![Opt::text("Add a token")];
                if u.tokens >= 3 {
                    options.push(Opt::text("Remove 3 tokens and kill every unit"));
                }
                if ctx.ask(self, "Nuke:", options)? == 0 {
                    self.unit_mut(user).tokens += 1;
                    ctx.log(format!("{} added a token.", cap(u.name())));
                    Ok(())
                } else {
                    self.unit_mut(user).tokens -= 3;
                    let everyone = self.units.iter().map(|o| o.id).collect();
                    self.kill_units(everyone, Some(user), true, ctx)
                }
            }
            Mine => {
                if !self.adjacent_to_flag(user) {
                    return illegal("Mine only works next to the flag.");
                }
                let seat = ctx.seat;
                let mut options: Vec<Opt> = self
                    .faceup
                    .iter()
                    .map(|&c| Opt::power(format!("Take {}", power_name(c)), c))
                    .collect();
                if !self.deck.is_empty() || !self.discard.is_empty() {
                    options.push(Opt::text("Draw from the deck"));
                }
                if options.is_empty() {
                    return illegal("There are no cards left.");
                }
                let i = ctx.ask(self, "Mine: draw which card?", options)?;
                let mut log = Vec::new();
                if i < self.faceup.len() {
                    let card = self.take_faceup(i, &mut log);
                    self.add_to_hand(seat, card, true);
                    ctx.log(format!(
                        "{} took {} (Mine).",
                        SEAT_NAMES[seat],
                        power_name(card)
                    ));
                } else {
                    let card = self.draw_card(&mut log).expect("the deck isn't empty");
                    self.add_to_hand(seat, card, false);
                    ctx.log(format!("{} drew a card (Mine).", SEAT_NAMES[seat]));
                }
                ctx.log.extend(log);
                Ok(())
            }
            Forbid => {
                let opponent = 1 - ctx.seat;
                let flag = data().flag;
                let mut choices: Vec<usize> = (0..data().powers.len())
                    .filter(|&p| {
                        p != flag
                            && !self.free_powers.contains(&p)
                            && !self.forbidden[opponent].contains(&p)
                    })
                    .collect();
                choices.sort_by_key(|&p| power_name(p));
                let options = choices
                    .iter()
                    .map(|&p| Opt::power(power_name(p), p))
                    .collect();
                let p = choices[ctx.ask(self, "Forbid: name a power", options)?];
                self.forbidden[opponent].push(p);
                ctx.log(format!(
                    "{} can't apply or activate {} next turn.",
                    SEAT_NAMES[opponent],
                    power_name(p)
                ));
                Ok(())
            }
            Recall => {
                let targets = self.all_targets(user, origin)?;
                let Some(owner) = u.owner else {
                    return illegal("Neutral units have no friends to recall.");
                };
                if targets.iter().any(|&t| self.unit(t).owner != Some(owner)) {
                    return illegal("Recall only works on friendly units.");
                }
                for &t in &targets {
                    let powers = std::mem::take(&mut self.unit_mut(t).powers);
                    for p in powers {
                        if p.card {
                            self.add_to_hand(owner, p.power, true);
                        }
                    }
                }
                ctx.log(format!(
                    "{} powers went back to their hand.",
                    possessive(SEAT_NAMES[owner])
                ));
                self.kill_units(targets, Some(user), true, ctx)
            }
            Possess => {
                let target =
                    self.pick_target(user, origin, "Possess which unit?", |_, _| true, ctx)?;
                let powers = std::mem::take(&mut self.unit_mut(user).powers);
                ctx.log(format!(
                    "{} possessed {}.",
                    cap(u.name()),
                    self.unit(target).name()
                ));
                for p in powers {
                    if !self.alive(target) {
                        let owner = self.unit_opt_owner(user);
                        self.remove_from_board(p, owner, None, true, ctx)?;
                        continue;
                    }
                    self.give_power(target, p, ctx)?;
                }
                if self.alive(user) {
                    self.kill_units(vec![user], Some(user), true, ctx)?;
                }
                Ok(())
            }
            Split => {
                if let Some(home) = home_of(ahead)
                    && u.owner != Some(home)
                {
                    return illegal("The new unit can't go in an enemy home space.");
                }
                if !on_board(ahead) || self.top_at(ahead).is_some() {
                    return illegal("There's no empty space in front.");
                }
                let n = u.powers.len();
                let options = (0..1usize << n)
                    .map(|mask| {
                        let names: Vec<&str> = (0..n)
                            .filter(|i| mask & (1 << i) != 0)
                            .map(|i| power_name(u.powers[i].power))
                            .collect();
                        Opt::text(if names.is_empty() {
                            "Give it no powers".into()
                        } else {
                            format!("Give it {}", names.join(" + "))
                        })
                    })
                    .collect();
                let mask = ctx.ask(self, "Split: which powers does the new unit get?", options)?;
                let given: Vec<PowerRef> = (0..n)
                    .filter(|i| mask & (1 << i) != 0)
                    .map(|i| u.powers[i])
                    .collect();
                let kept: Vec<PowerRef> = (0..n)
                    .filter(|i| mask & (1 << i) == 0)
                    .map(|i| u.powers[i])
                    .collect();
                self.unit_mut(user).powers = kept;
                let id = self.create_unit(u.owner, ahead, facing, given, ctx)?;
                if self.alive(id) {
                    ctx.log(format!(
                        "{} split off {}.",
                        cap(u.name()),
                        self.unit(id).name()
                    ));
                }
                Ok(())
            }
            Convert => {
                let targets = self.all_targets(user, origin)?;
                let Some(owner) = u.owner else {
                    return illegal("A neutral unit can't convert units.");
                };
                if targets.iter().any(|&t| self.unit(t).owner.is_some()) {
                    return illegal("Convert only works on neutral units.");
                }
                for t in targets {
                    if !self.alive(t) {
                        continue;
                    }
                    let before = self.twitchy_snapshot();
                    let label = self.free_label(owner).unwrap_or("+").to_string();
                    let unit = self.unit_mut(t);
                    unit.owner = Some(owner);
                    unit.label = label.clone();
                    ctx.log(format!(
                        "A neutral unit became {} unit {label}.",
                        SEAT_NAMES[owner]
                    ));
                    self.enforce_unit_cap(owner, ctx)?;
                    self.twitchy_check(before, ctx)?;
                }
                Ok(())
            }
            _ => illegal("That's a passive power."),
        }
    }

    fn unit_opt_owner(&self, id: u32) -> Option<Seat> {
        self.units.iter().find(|u| u.id == id).and_then(|u| u.owner)
    }
}

#[cfg(test)]
mod tests;
