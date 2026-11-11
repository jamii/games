//! Bots that play 001 with automatic rules.
//!
//! - `RandomAgent` picks uniformly among the legal actions.
//! - `GreedyAgent` picks the action whose result `evaluate` likes best.
//! - `MctsAgent` searches. It runs Monte Carlo tree search over single actions,
//!   with `evaluate` at the leaves and to set move priors. It searches several
//!   samples of the hidden information (`Game::determinize`) and adds up the
//!   results, so it never looks at cards it shouldn't know.
//!
//! All bots stop taking actions after `action_cap` actions in a turn, so a
//! turn can't go on forever (eg with *Split*, which makes a new unit that can
//! split again), and they don't go round in circles (see `bot_actions`).

use crate::data::{Effect, Seat};
use crate::game::{Action, Game, Phase};
use crate::net::Net;
use crate::rules::hex_dist;

pub const DEFAULT_ACTION_CAP: u32 = 16;

/// Set GAME_DEBUG=1 to log slow searches.
fn debug() -> bool {
    static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("GAME_DEBUG").is_some())
}

pub trait Agent: Send {
    fn name(&self) -> String;
    fn choose(&mut self, game: &Game, seat: Seat) -> Action;
    /// What the last `choose` searched, if it searched (for training data).
    fn last_search(&self) -> Option<&SearchResult> {
        None
    }
}

/// Who has to act now: the player answering a prompt, or the current player.
pub fn to_move(game: &Game) -> Seat {
    game.pending
        .as_ref()
        .map(|p| p.seat)
        .unwrap_or(game.current)
}

/// The legal actions, except that after `cap` actions in a turn the only
/// option is to end it (prompts can still be answered), and that bots don't
/// waste actions going round in circles:
///
/// - they don't go back to a position they were already in this turn (with
///   an action that needs no answers on the way);
/// - they don't undo their own changes to a unit's powers (see `takes_back`).
///
/// Without these rules the trained bot spent many turns adding the free
/// *Walk* to a unit and discarding it again until it ran out of actions. Its
/// network learned to like those moves from self-play games that were full of
/// them, and the search plays its most visited move, so it kept choosing them
/// even when it valued another move more.
pub fn bot_actions(game: &Game, seat: Seat, cap: u32) -> Vec<Action> {
    if game.pending.is_none() && game.phase == Phase::Main && game.actions_this_turn >= cap {
        return vec![Action::EndTurn { score: false }];
    }
    let mut actions = game.legal_actions(seat);
    if game.pending.is_none() && game.phase == Phase::Main {
        actions.retain(|a| {
            !takes_back(game, a)
                && !(only_changes_powers(a)
                    && game
                        .after_unprompted(seat, a)
                        .is_some_and(|next| game.repeats_position(&next)))
        });
    }
    actions
}

/// Whether an action only moves powers between units and hands. These are
/// the only actions that can bring a turn back to an earlier position (with
/// *Recycle*, even a card can go back to the hand). Every other one leaves a
/// mark for the rest of the turn: a spawn, or an activation.
fn only_changes_powers(action: &Action) -> bool {
    matches!(
        action,
        Action::ApplyFree { .. } | Action::ApplyHand { .. } | Action::RemovePower { .. }
    )
}

/// Whether an action undoes a change the current player made to a unit's
/// powers earlier in the turn: discarding or replacing a power they put on the
/// unit, or putting back one they took off it. It isn't undoing if *Vacuum*
/// catches the power taken off, or if the unit has used a power since (it may
/// have been that one).
///
/// Bots don't do this: it only uses up actions (and cards). Without the rule
/// the trained bot, even once it couldn't go back to an earlier position,
/// still added *Walk* to a unit two or three times and then discarded it
/// again, or put a card on a unit only to discard it.
fn takes_back(game: &Game, action: &Action) -> bool {
    let (unit, added, replace) = match *action {
        Action::ApplyFree {
            power,
            unit,
            replace,
            ..
        } => (unit, Some(power), replace),
        Action::ApplyHand {
            card,
            unit,
            replace,
            ..
        } => (unit, Some(card), replace),
        Action::RemovePower { unit, index, .. } => (unit, None, Some(index)),
        _ => return false,
    };
    let removed = replace.and_then(|i| {
        let u = game.units.iter().find(|u| u.id == unit)?;
        Some(u.powers.get(i)?.power)
    });
    added.is_some_and(|p| game.power_removed(unit, p))
        || removed.is_some_and(|p| game.power_added(unit, p) && !game.vacuum_catches(unit))
}

/// A small, fast, seedable random number generator (xorshift64*).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Weights for `evaluate`. The defaults are hand-tuned guesses.
#[derive(Clone, Debug)]
pub struct Weights {
    /// Points still needed to win (theirs minus ours).
    pub points: f32,
    /// Whoever leads at the flag when the current turn ends scores.
    pub scoring_now: f32,
    /// Lead at the flag (units next to it).
    pub flag_margin: f32,
    /// Units near the flag, weighted by distance.
    pub presence: f32,
    pub units: f32,
    pub powers: f32,
    /// Units with a way to move.
    pub mobility: f32,
    pub hand: f32,
    /// How quickly the evaluation saturates towards a certain win or loss.
    pub scale: f32,
}

impl Default for Weights {
    fn default() -> Weights {
        Weights {
            points: 1.0,
            scoring_now: 0.8,
            flag_margin: 0.25,
            presence: 0.2,
            units: 0.1,
            powers: 0.12,
            mobility: 0.08,
            hand: 0.02,
            scale: 0.45,
        }
    }
}

fn moves_itself(effect: Effect) -> bool {
    use Effect::*;
    matches!(
        effect,
        Walk | Run | Rush | Catapult | Bungee | Burrow | Circle | Jump | Climb | Swap | Push | Pull
    )
}

/// Names of the general features at the start of `features`. After them come,
/// for each card, (my units with it) − (their units with it), then for each
/// card, whether it's in my hand.
pub const BASE_FEATURES: [&str; 9] = [
    "points ahead",
    "scoring now",
    "flag margin",
    "presence",
    "units",
    "powers",
    "mobility",
    "hand",
    "my turn",
];

/// What a position looks like to `seat`, as numbers: only things `seat` can
/// see. None when the game is over, or when `seat` is about to win this turn
/// (`Some(1.0)` from `certain`).
pub fn features(game: &Game, seat: Seat) -> Result<Vec<f32>, f32> {
    let shown = game.pending.as_ref().map(|p| &*p.preview).unwrap_or(game);
    if let Phase::Over { winner } = shown.phase {
        return Err(if winner == seat { 1.0 } else { -1.0 });
    }
    let (me, them) = (seat, 1 - seat);
    let flag = shown.auto_flag_counts();
    // The point the current player is on course to score at the end of this turn.
    let mut expected = [shown.scores[0] as f32, shown.scores[1] as f32];
    let cur = shown.current;
    if shown.phase == Phase::Main && flag[cur] > flag[1 - cur] {
        expected[cur] += 1.0;
    }
    let target = shown.options.target_score as f32;
    if expected[me] >= target {
        return Err(0.97);
    }
    let need_me = target - expected[me];
    let need_them = target - expected[them];
    let scoring_now =
        (expected[me] - shown.scores[me] as f32) - (expected[them] - shown.scores[them] as f32);

    let d = crate::data::data();
    let cards = d.powers.len();
    let mut f = vec![0f32; BASE_FEATURES.len() + 2 * cards];
    let flag_cell = shown.flag_cell();
    let (mut presence, mut units, mut powers, mut mobility) =
        ([0f32; 2], [0f32; 2], [0f32; 2], [0f32; 2]);
    for u in shown.top_units() {
        let Some(owner) = u.owner else { continue };
        units[owner] += 1.0;
        // Each different power counts once: a card is worth more than a free
        // power, and an active card a bit more again.
        let mut seen: Vec<usize> = Vec::new();
        for p in &u.powers {
            if seen.contains(&p.power) {
                continue;
            }
            seen.push(p.power);
            f[BASE_FEATURES.len() + p.power] += if owner == me { 1.0 } else { -1.0 };
            let active = d.powers[p.power].kind == crate::data::Kind::Active;
            powers[owner] += match (p.card, active) {
                (false, _) => 0.3,
                (true, false) => 1.0,
                (true, true) => 1.3,
            };
        }
        if u.powers
            .iter()
            .any(|p| d.effect(p.power).is_some_and(moves_itself))
        {
            mobility[owner] += 1.0;
        }
        if let Some(c) = flag_cell
            && !u.is_flag()
        {
            presence[owner] += match hex_dist(u.cell, c) {
                1 => 1.0,
                2 => 0.45,
                3 => 0.2,
                _ => 0.05,
            };
        }
    }
    for &card in &shown.hands[me] {
        f[BASE_FEATURES.len() + cards + card] = 1.0;
    }
    let diff = |a: [f32; 2]| a[me] - a[them];
    f[0] = need_them - need_me;
    f[1] = scoring_now;
    f[2] = flag[me] as f32 - flag[them] as f32;
    f[3] = diff(presence);
    f[4] = diff(units);
    f[5] = diff(powers);
    f[6] = diff(mobility);
    // Cards in hand are options, but there's only so much room to use them.
    f[7] = shown.hands[me].len().min(6) as f32 - shown.hands[them].len().min(6) as f32;
    f[8] = if shown.current == me { 1.0 } else { -1.0 };
    Ok(f)
}

/// How good the position looks for `seat`, from -1 (lost) to 1 (won).
pub fn evaluate(game: &Game, seat: Seat) -> f32 {
    evaluate_with(game, seat, &Weights::default())
}

pub fn evaluate_with(game: &Game, seat: Seat, w: &Weights) -> f32 {
    let f = match features(game, seat) {
        Ok(f) => f,
        Err(v) => return v,
    };
    let z = w.points * f[0]
        + w.scoring_now * f[1]
        + w.flag_margin * f[2]
        + w.presence * f[3]
        + w.units * f[4]
        + w.powers * f[5]
        + w.mobility * f[6]
        + w.hand * f[7];
    (w.scale * z).tanh() * 0.95
}

/// An evaluation learned from self-play (tools/fit_eval.py): logistic
/// regression on `features`, predicting whether `seat` wins.
#[derive(Clone, Debug)]
pub struct Learned {
    pub weights: Vec<f32>,
    pub bias: f32,
}

impl Learned {
    /// Reads the JSON written by tools/fit_eval.py.
    pub fn load(path: &str) -> Result<Learned, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let weights: Vec<f32> = json["weights"]
            .as_array()
            .ok_or("no weights")?
            .iter()
            .map(|w| w.as_f64().unwrap_or(0.0) as f32)
            .collect();
        let expected = BASE_FEATURES.len() + 2 * crate::data::data().powers.len();
        if weights.len() != expected {
            return Err(format!(
                "{path}: expected {expected} weights, found {}",
                weights.len()
            ));
        }
        Ok(Learned {
            weights,
            bias: json["bias"].as_f64().unwrap_or(0.0) as f32,
        })
    }

    /// From -1 to 1, like `evaluate`.
    pub fn evaluate(&self, game: &Game, seat: Seat) -> f32 {
        match features(game, seat) {
            Ok(f) => {
                let z: f32 =
                    self.bias + f.iter().zip(&self.weights).map(|(x, w)| x * w).sum::<f32>();
                // 2 * sigmoid(z) - 1, kept a little away from certainty.
                (z / 2.0).tanh() * 0.95
            }
            Err(v) => v,
        }
    }
}

/// Which evaluation a bot uses.
#[derive(Clone, Debug)]
pub enum Evaluator {
    Hand(Weights),
    Learned(std::sync::Arc<Learned>),
    /// A neural network (stage 2). In search it also gives the move priors.
    Net(std::sync::Arc<Net>),
}

impl Default for Evaluator {
    fn default() -> Evaluator {
        Evaluator::Hand(Weights::default())
    }
}

impl Evaluator {
    pub fn evaluate(&self, game: &Game, seat: Seat) -> f32 {
        match self {
            Evaluator::Hand(w) => evaluate_with(game, seat, w),
            Evaluator::Learned(l) => l.evaluate(game, seat),
            Evaluator::Net(net) => {
                if let Phase::Over { winner } = game.phase {
                    return if winner == seat { 1.0 } else { -1.0 };
                }
                let mover = to_move(game);
                let (value, _) = net_eval(net, game, mover, DEFAULT_ACTION_CAP);
                if mover == seat { value } else { -value }
            }
        }
    }
}

/// The network's value (for `mover`, who must be the player to move) and
/// the legal actions with their priors.
pub fn net_eval(net: &Net, game: &Game, mover: Seat, cap: u32) -> (f32, Vec<(Action, f32)>) {
    let actions = bot_actions(game, mover, cap);
    let obs = crate::encode::observe(game, mover);
    let encoded = crate::encode::encode_actions(game, mover, &obs, &actions);
    let out = net.forward(&obs, &encoded);
    let max = out.logits.iter().copied().fold(f32::MIN, f32::max);
    let weights: Vec<f32> = out.logits.iter().map(|l| (l - max).exp()).collect();
    let sum: f32 = weights.iter().sum();
    let priors = actions
        .into_iter()
        .zip(weights)
        .map(|(a, w)| (a, w / sum))
        .collect();
    (out.value, priors)
}

/// Plays the network's favourite action, without searching.
pub struct PolicyAgent {
    pub net: std::sync::Arc<Net>,
    pub action_cap: u32,
}

impl Agent for PolicyAgent {
    fn name(&self) -> String {
        "policy".into()
    }
    fn choose(&mut self, game: &Game, seat: Seat) -> Action {
        let (_, priors) = net_eval(&self.net, game, seat, self.action_cap);
        priors
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(a, _)| a)
            .unwrap_or(Action::EndTurn { score: false })
    }
}

// ---------------------------------------------------------------------------
// Simple agents
// ---------------------------------------------------------------------------

pub struct RandomAgent {
    rng: Rng,
    pub action_cap: u32,
}

impl RandomAgent {
    pub fn new(seed: u64) -> RandomAgent {
        RandomAgent {
            rng: Rng::new(seed),
            action_cap: 8,
        }
    }
}

impl Agent for RandomAgent {
    fn name(&self) -> String {
        "random".into()
    }
    fn choose(&mut self, game: &Game, seat: Seat) -> Action {
        let legal = bot_actions(game, seat, self.action_cap);
        legal[self.rng.below(legal.len())].clone()
    }
}

/// The result of applying an action, as `seat` would judge it.
fn judge(eval: &Evaluator, game: &Game, seat: Seat, action: &Action) -> f32 {
    match game.after(seat, action) {
        Some(next) => eval.evaluate(&next, seat),
        None => -2.0,
    }
}

pub struct GreedyAgent {
    rng: Rng,
    pub action_cap: u32,
    pub eval: Evaluator,
}

impl GreedyAgent {
    pub fn new(seed: u64) -> GreedyAgent {
        GreedyAgent {
            rng: Rng::new(seed),
            action_cap: DEFAULT_ACTION_CAP,
            eval: Evaluator::default(),
        }
    }
}

impl Agent for GreedyAgent {
    fn name(&self) -> String {
        "greedy".into()
    }
    fn choose(&mut self, game: &Game, seat: Seat) -> Action {
        let legal = bot_actions(game, seat, self.action_cap);
        let mut best = (f32::MIN, 0);
        for (i, action) in legal.iter().enumerate() {
            // A little noise breaks ties at random.
            let score = judge(&self.eval, game, seat, action) + self.rng.unit() * 1e-3;
            if score > best.0 {
                best = (score, i);
            }
        }
        legal[best.1].clone()
    }
}

// ---------------------------------------------------------------------------
// Monte Carlo tree search
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct MctsConfig {
    /// Simulations per decision, split across the samples.
    pub simulations: u32,
    /// How many samples of the hidden information to search.
    pub samples: u32,
    /// Exploration constant (PUCT).
    pub exploration: f32,
    /// Softmax temperature for turning evaluations into move priors.
    pub prior_temperature: f32,
    /// 0 plays the most-visited action; higher values sample more widely.
    pub temperature: f32,
    pub action_cap: u32,
    pub eval: Evaluator,
    /// Stop a search early once this many actions have been evaluated per
    /// simulation on average, so crowded positions (with hundreds of legal
    /// actions) don't take minutes. Deterministic, unlike a time limit.
    pub work_per_simulation: u32,
    /// Mix this much Dirichlet noise into the root's priors (0 for none), so
    /// self-play for training explores. AlphaZero used 0.25.
    pub root_noise: f32,
    /// Play by `temperature` only up to this turn, and pick the most visited
    /// action after it.
    pub temperature_turns: u32,
}

impl Default for MctsConfig {
    fn default() -> MctsConfig {
        MctsConfig {
            simulations: 200,
            samples: 4,
            exploration: 1.5,
            prior_temperature: 0.15,
            temperature: 0.0,
            action_cap: DEFAULT_ACTION_CAP,
            eval: Evaluator::default(),
            work_per_simulation: 40,
            root_noise: 0.0,
            temperature_turns: u32::MAX,
        }
    }
}

struct Node {
    game: Game,
    mover: Seat,
    /// Value for seat 0 if the game is over.
    terminal: Option<f32>,
    expanded: bool,
    visits: u32,
    /// Evaluation for seat 0 when the node was created.
    value: f32,
    edges: Vec<Edge>,
}

struct Edge {
    action: Action,
    prior: f32,
    child: Option<usize>,
    visits: u32,
    /// Sum of values from the point of view of the node's mover.
    total: f32,
}

impl Node {
    fn new(game: Game, eval: &Evaluator, cap: u32) -> Node {
        let mover = to_move(&game);
        let terminal = match game.phase {
            Phase::Over { winner } => Some(if winner == 0 { 1.0 } else { -1.0 }),
            _ => None,
        };
        if let (Evaluator::Net(net), None) = (eval, terminal) {
            // One network call gives the value and the priors, so the node is
            // expanded straight away (as in AlphaZero).
            let (value, priors) = net_eval(net, &game, mover, cap);
            let edges = priors
                .into_iter()
                .map(|(action, prior)| Edge {
                    action,
                    prior,
                    child: None,
                    visits: 0,
                    total: 0.0,
                })
                .collect();
            return Node {
                game,
                mover,
                terminal,
                expanded: true,
                visits: 0,
                value: if mover == 0 { value } else { -value },
                edges,
            };
        }
        let value = terminal.unwrap_or_else(|| eval.evaluate(&game, 0));
        Node {
            game,
            mover,
            terminal,
            expanded: false,
            visits: 0,
            value,
            edges: Vec::new(),
        }
    }
}

/// What a search found: each candidate action with its visit count and mean
/// value for the player choosing.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub actions: Vec<Action>,
    pub visits: Vec<u32>,
    pub values: Vec<f32>,
}

pub struct MctsAgent {
    pub config: MctsConfig,
    rng: Rng,
    /// The last search, for analysis.
    pub last: Option<SearchResult>,
}

impl MctsAgent {
    pub fn new(config: MctsConfig, seed: u64) -> MctsAgent {
        MctsAgent {
            config,
            rng: Rng::new(seed),
            last: None,
        }
    }

    /// Expand a node, returning how many actions it evaluated.
    fn expand(&self, nodes: &mut [Node], index: usize) -> u64 {
        // Only timed when debugging: browsers have no clock for Instant.
        let started = debug().then(std::time::Instant::now);
        let work = self.expand_inner(nodes, index);
        if let Some(started) = started
            && started.elapsed().as_millis() > 100
        {
            let node = &nodes[index];
            eprintln!(
                "slow expansion: {} ms, {} actions, {} units, phase {:?}, prompt {:?}",
                started.elapsed().as_millis(),
                work,
                node.game.units.len(),
                node.game.phase,
                node.game.prompt().map(|p| &p.text),
            );
        }
        work
    }

    fn expand_inner(&self, nodes: &mut [Node], index: usize) -> u64 {
        let node = &mut nodes[index];
        node.expanded = true;
        let actions = bot_actions(&node.game, node.mover, self.config.action_cap);
        // Priors: a softmax over how good each action's result looks.
        let scores: Vec<f32> = actions
            .iter()
            .map(|a| judge(&self.config.eval, &node.game, node.mover, a))
            .collect();
        let max = scores.iter().copied().fold(f32::MIN, f32::max);
        let weights: Vec<f32> = scores
            .iter()
            .map(|s| ((s - max) / self.config.prior_temperature).exp())
            .collect();
        let sum: f32 = weights.iter().sum();
        node.edges = actions
            .into_iter()
            .zip(weights)
            .map(|(action, w)| Edge {
                action,
                prior: w / sum,
                child: None,
                visits: 0,
                total: 0.0,
            })
            .collect();
        node.edges.len() as u64
    }

    fn select(&self, node: &Node) -> usize {
        let sqrt_n = (node.visits as f32 + 1.0).sqrt();
        let own_value = if node.mover == 0 {
            node.value
        } else {
            -node.value
        };
        // Unvisited actions start a little below the node's own value.
        let first_play = own_value - 0.1;
        let mut best = (f32::MIN, 0);
        for (i, e) in node.edges.iter().enumerate() {
            let q = if e.visits > 0 {
                e.total / e.visits as f32
            } else {
                first_play
            };
            let u = self.config.exploration * e.prior * sqrt_n / (1.0 + e.visits as f32);
            if q + u > best.0 {
                best = (q + u, i);
            }
        }
        best.1
    }

    /// Search one sample of the game, returning visits and values for the
    /// root's actions.
    fn search(
        &self,
        game: Game,
        simulations: u32,
        noise_seed: u64,
    ) -> (Vec<Action>, Vec<u32>, Vec<f32>) {
        let cap = self.config.action_cap;
        let mut nodes = vec![Node::new(game, &self.config.eval, cap)];
        let mut work = if nodes[0].expanded {
            nodes[0].edges.len() as u64
        } else {
            self.expand(&mut nodes, 0)
        };
        if self.config.root_noise > 0.0 && nodes[0].edges.len() > 1 {
            let mut rng = Rng::new(noise_seed);
            let noise = dirichlet(&mut rng, 0.3, nodes[0].edges.len());
            let eps = self.config.root_noise;
            for (e, n) in nodes[0].edges.iter_mut().zip(noise) {
                e.prior = (1.0 - eps) * e.prior + eps * n;
            }
        }
        let budget = simulations as u64 * self.config.work_per_simulation as u64;
        for _ in 0..simulations {
            if work > budget {
                break;
            }
            let mut path: Vec<(usize, usize)> = Vec::new();
            let mut index = 0;
            let value = loop {
                if let Some(t) = nodes[index].terminal {
                    break t;
                }
                if !nodes[index].expanded {
                    work += self.expand(&mut nodes, index);
                    break nodes[index].value;
                }
                let e = self.select(&nodes[index]);
                path.push((index, e));
                match nodes[index].edges[e].child {
                    Some(child) => index = child,
                    None => {
                        let mut next = nodes[index].game.clone();
                        let mover = nodes[index].mover;
                        let action = nodes[index].edges[e].action.clone();
                        if next.apply(mover, &action).is_err() {
                            // Shouldn't happen: the action was legal.
                            break if mover == 0 { -1.0 } else { 1.0 };
                        }
                        let child = nodes.len();
                        nodes.push(Node::new(next, &self.config.eval, cap));
                        nodes[index].edges[e].child = Some(child);
                        if let Evaluator::Net(_) = self.config.eval {
                            work += nodes[child].edges.len() as u64;
                        }
                        break nodes[child].terminal.unwrap_or(nodes[child].value);
                    }
                }
            };
            for (n, e) in path {
                let node = &mut nodes[n];
                node.visits += 1;
                let edge = &mut node.edges[e];
                edge.visits += 1;
                edge.total += if node.mover == 0 { value } else { -value };
            }
        }
        let root = &nodes[0];
        let actions = root.edges.iter().map(|e| e.action.clone()).collect();
        let visits = root.edges.iter().map(|e| e.visits).collect();
        let values = root
            .edges
            .iter()
            .map(|e| {
                if e.visits > 0 {
                    e.total / e.visits as f32
                } else {
                    f32::NAN
                }
            })
            .collect();
        (actions, visits, values)
    }
}

impl Agent for MctsAgent {
    fn name(&self) -> String {
        let learned = match self.config.eval {
            Evaluator::Hand(_) => "",
            Evaluator::Learned(_) => "L",
            Evaluator::Net(_) => "N",
        };
        format!(
            "mcts{learned}:{}x{}",
            self.config.simulations, self.config.samples
        )
    }

    fn choose(&mut self, game: &Game, seat: Seat) -> Action {
        let actions = bot_actions(game, seat, self.config.action_cap);
        if actions.len() == 1 {
            self.last = None;
            return actions[0].clone();
        }
        let samples = self.config.samples.max(1);
        let per_sample = (self.config.simulations / samples).max(1);
        let mut visits = vec![0u32; actions.len()];
        let mut totals = vec![0f32; actions.len()];
        for _ in 0..samples {
            let sample = game.determinize(seat, self.rng.next_u64());
            // Only draw when needed, so searches without noise are unchanged.
            let noise_seed = if self.config.root_noise > 0.0 {
                self.rng.next_u64()
            } else {
                0
            };
            let (found, v, q) = self.search(sample, per_sample, noise_seed);
            for (i, action) in found.iter().enumerate() {
                if let Some(j) = actions.iter().position(|a| a == action) {
                    visits[j] += v[i];
                    if v[i] > 0 {
                        totals[j] += q[i] * v[i] as f32;
                    }
                }
            }
        }
        let values: Vec<f32> = (0..actions.len())
            .map(|i| {
                if visits[i] > 0 {
                    totals[i] / visits[i] as f32
                } else {
                    f32::NAN
                }
            })
            .collect();
        let pick = if self.config.temperature <= 0.0 || game.turn > self.config.temperature_turns {
            (0..actions.len())
                .max_by_key(|&i| (visits[i], (values[i].max(-9.0) * 1e4) as i64))
                .unwrap_or(0)
        } else {
            let weights: Vec<f64> = visits
                .iter()
                .map(|&v| (v as f64).powf(1.0 / self.config.temperature as f64))
                .collect();
            let sum: f64 = weights.iter().sum();
            let mut r = self.rng.unit() as f64 * sum;
            let mut pick = 0;
            for (i, w) in weights.iter().enumerate() {
                if r < *w {
                    pick = i;
                    break;
                }
                r -= w;
            }
            pick
        };
        self.last = Some(SearchResult {
            actions: actions.clone(),
            visits,
            values,
        });
        actions[pick].clone()
    }

    fn last_search(&self) -> Option<&SearchResult> {
        self.last.as_ref()
    }
}

/// A sample from a symmetric Dirichlet distribution.
fn dirichlet(rng: &mut Rng, alpha: f32, n: usize) -> Vec<f32> {
    let xs: Vec<f32> = (0..n).map(|_| gamma(rng, alpha)).collect();
    let sum: f32 = xs.iter().sum::<f32>().max(1e-12);
    xs.into_iter().map(|x| x / sum).collect()
}

/// Marsaglia and Tsang's method, with the usual boost for shape < 1.
fn gamma(rng: &mut Rng, shape: f32) -> f32 {
    if shape < 1.0 {
        let u = rng.unit().max(1e-12);
        return gamma(rng, shape + 1.0) * u.powf(1.0 / shape);
    }
    let d = shape - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    loop {
        // A standard normal by Box-Muller.
        let (u1, u2) = (rng.unit().max(1e-12), rng.unit());
        let x = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
        let v = (1.0 + c * x).powi(3);
        if v <= 0.0 {
            continue;
        }
        let u = rng.unit().max(1e-12);
        if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
            return d * v;
        }
    }
}

// ---------------------------------------------------------------------------
// Playing games
// ---------------------------------------------------------------------------

/// Play one game between two agents (`agents[seat]` plays that seat). Returns
/// the finished game, or the game as it stood after `max_turns` turns.
pub fn play(mut game: Game, agents: &mut [Box<dyn Agent>; 2], max_turns: u32) -> Game {
    while !matches!(game.phase, Phase::Over { .. }) && game.turn <= max_turns {
        let seat = to_move(&game);
        let action = agents[seat].choose(&game, seat);
        game.apply(seat, &action).unwrap_or_else(|e| {
            panic!(
                "{} chose an illegal action {action:?}: {}",
                agents[seat].name(),
                e.message
            )
        });
    }
    game
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Options;

    fn new_game(seed: u64) -> Game {
        Game::new(Options {
            seed: Some(seed),
            ..Options::default()
        })
        .unwrap()
        .0
    }

    #[test]
    fn bots_finish_games_with_legal_moves() {
        for seed in 0..3 {
            let mut agents: [Box<dyn Agent>; 2] = [
                Box::new(GreedyAgent::new(seed)),
                Box::new(RandomAgent::new(seed + 1)),
            ];
            play(new_game(seed), &mut agents, 30);
            let config = MctsConfig {
                simulations: 24,
                samples: 2,
                ..MctsConfig::default()
            };
            let mut agents: [Box<dyn Agent>; 2] = [
                Box::new(MctsAgent::new(config.clone(), seed)),
                Box::new(RandomAgent::new(seed + 1)),
            ];
            play(new_game(seed), &mut agents, 10);
        }
    }

    /// Letters' turn, main phase, empty hands. Letters' home is (-1, 0),
    /// (1, 0) and (0, 1); the flag is on (0, 3).
    fn main_phase() -> Game {
        let (mut game, _) = Game::new(Options {
            first_player: Some(0),
            seed: Some(7),
            ..Options::default()
        })
        .unwrap();
        game.phase = Phase::Main;
        game.turn = 1;
        game.current = 0;
        game.hands = [Vec::new(), Vec::new()];
        game
    }

    fn power(name: &str) -> usize {
        crate::data::data()
            .powers
            .iter()
            .position(|p| p.name == name)
            .unwrap()
    }

    /// A Letters unit with these powers (cards, except *Walk*).
    fn put(game: &mut Game, x: i32, y: i32, powers: &[&str]) -> u32 {
        let powers = powers
            .iter()
            .map(|&name| crate::game::PowerRef {
                power: power(name),
                card: name != "Walk",
            })
            .collect();
        let index = game.add_unit(Some(0), crate::data::Cell { x, y }, 0, powers);
        let id = game.units[index].id;
        game.sort_units();
        id
    }

    fn add_walk(unit: u32) -> Action {
        Action::ApplyFree {
            power: power("Walk"),
            unit,
            replace: None,
            force: false,
        }
    }

    fn discard(unit: u32, index: usize) -> Action {
        Action::RemovePower {
            unit,
            index,
            to_hand: false,
            force: false,
        }
    }

    fn bot_can(game: &Game, action: &Action) -> bool {
        assert!(
            game.legal_actions(0).contains(action),
            "{action:?} is legal"
        );
        bot_actions(game, 0, DEFAULT_ACTION_CAP).contains(action)
    }

    #[test]
    fn bots_dont_undo_their_free_power_changes() {
        let turn = |unit| Action::Activate {
            unit,
            facing: 1,
            power: None,
            origin: None,
            force: false,
        };
        let mut game = main_phase();
        let bare = put(&mut game, -1, 0, &[]);
        let walker = put(&mut game, 1, 0, &["Walk"]);
        let other = put(&mut game, 0, 1, &[]);
        game.apply(0, &add_walk(bare)).unwrap();
        assert!(
            !bot_can(&game, &discard(bare, 0)),
            "doesn't discard the Walk it just added"
        );
        // Even when something else happened in between, so that discarding it
        // wouldn't go back to an earlier position.
        game.apply(0, &turn(other)).unwrap();
        assert!(!bot_can(&game, &discard(bare, 0)));
        assert!(bot_can(&game, &add_walk(bare)), "can add a second Walk");
        assert!(
            bot_can(&game, &discard(walker, 0)),
            "can discard a Walk the unit started the turn with"
        );
        game.apply(0, &discard(walker, 0)).unwrap();
        game.apply(0, &add_walk(other)).unwrap();
        assert!(
            !bot_can(&game, &add_walk(walker)),
            "doesn't add back the Walk it discarded"
        );

        game.apply(0, &turn(bare)).unwrap();
        assert!(
            !bot_can(&game, &discard(bare, 0)),
            "turning doesn't use the Walk"
        );
        game.apply(0, &Action::EndTurn { score: false }).unwrap();
        assert!(game.power_changes.is_empty() && game.positions_this_turn.is_empty());

        // Once the unit has used a power, it may have been the free one.
        let mut game = main_phase();
        let walker = put(&mut game, -1, 0, &[]);
        game.apply(0, &add_walk(walker)).unwrap();
        let walk_home = Action::Activate {
            unit: walker,
            facing: 1,
            power: Some(0),
            origin: None,
            force: false,
        };
        game.apply(0, &walk_home).unwrap();
        assert!(bot_can(&game, &discard(walker, 0)));
    }

    #[test]
    fn bots_dont_swap_free_powers_back() {
        // With a second free power, one can replace the other.
        let mut game = main_phase();
        game.free_powers.push(power("Run"));
        let apply = |power_name: &str, unit: u32, replace: usize| Action::ApplyFree {
            power: power(power_name),
            unit,
            replace: Some(replace),
            force: false,
        };
        let a = put(&mut game, -1, 0, &["Stun", "Grip"]);
        game.apply(0, &add_walk(a)).unwrap();
        assert!(
            !bot_can(&game, &apply("Run", a, 2)),
            "Run over the new Walk"
        );
        game.apply(0, &apply("Run", a, 0)).unwrap();
        assert!(
            !bot_can(&game, &apply("Walk", a, 0)),
            "Walk over the new Run"
        );

        // A Walk replaced by Run is gone, so it doesn't come back either.
        let b = put(&mut game, 1, 0, &["Stun", "Grip", "Walk"]);
        game.apply(0, &apply("Run", b, 2)).unwrap();
        assert!(!bot_can(&game, &apply("Walk", b, 0)));
    }

    #[test]
    fn bots_can_feed_free_powers_to_vacuum() {
        // A Walk that Vacuum catches goes to the flag: that's not undoing.
        for vacuum in [&["Vacuum"][..], &["Vacuum", "Stun", "Grip"]] {
            let mut game = main_phase();
            put(&mut game, -2, 3, vacuum);
            let bare = put(&mut game, -1, 0, &[]);
            game.apply(0, &add_walk(bare)).unwrap();
            assert!(bot_can(&game, &discard(bare, 0)), "{vacuum:?}");
            if vacuum.len() == 1 {
                game.apply(0, &discard(bare, 0)).unwrap();
                assert!(bot_can(&game, &add_walk(bare)), "and can do it again");
            }
        }
    }

    #[test]
    fn bots_dont_discard_cards_they_just_applied() {
        let mut game = main_phase();
        let bare = put(&mut game, -1, 0, &[]);
        let stun = power("Stun");
        game.hands[0] = vec![stun];
        let apply = Action::ApplyHand {
            card: stun,
            unit: bare,
            replace: None,
            force: false,
        };
        game.apply(0, &apply).unwrap();
        assert!(!bot_can(&game, &discard(bare, 0)));

        // With Recycle at the flag the card would go back to the hand, so
        // this could go on forever.
        put(&mut game, -2, 3, &["Recycle"]);
        game.apply(0, &discard(bare, 0)).unwrap();
        assert_eq!(game.hands[0], vec![stun], "Recycle returned it");
        assert!(!bot_can(&game, &apply), "doesn't put it back");
    }

    #[test]
    fn bots_dont_go_back_to_earlier_positions() {
        // With the flag next to Letters' home and a Vacuum unit there, a Walk
        // can go round from one unit to the Vacuum unit and off the board.
        let mut game = main_phase();
        let flag = game.units.iter().position(|u| u.is_flag()).unwrap();
        game.units[flag].cell = crate::data::Cell { x: 1, y: 2 };
        let vacuum = put(&mut game, 0, 1, &["Vacuum"]);
        let bare = put(&mut game, -1, 0, &[]);
        game.apply(0, &add_walk(bare)).unwrap();
        game.apply(0, &discard(bare, 0)).unwrap();
        let powers = |game: &Game| {
            game.units
                .iter()
                .find(|u| u.id == vacuum)
                .unwrap()
                .powers
                .len()
        };
        assert_eq!(powers(&game), 2, "Vacuum caught it");
        assert!(!bot_can(&game, &discard(vacuum, 1)), "back to the start");
    }

    #[test]
    fn evaluation_is_symmetric() {
        let game = new_game(1);
        for seat in 0..2 {
            assert!((evaluate(&game, seat) + evaluate(&game, 1 - seat)).abs() < 1e-5);
        }
    }
}
