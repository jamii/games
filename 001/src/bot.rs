//! Bots that play 001 with automatic rules.
//!
//! - `RandomAgent` picks uniformly among the legal actions.
//! - `GreedyAgent` picks the action whose result `evaluate` likes best.
//! - `MctsAgent` searches. It runs Monte Carlo tree search over single actions,
//!   with `evaluate` at the leaves and to set move priors. It searches several
//!   samples of the hidden information (`Game::determinize`) and adds up the
//!   results, so it never looks at cards it shouldn't know.
//!
//! All bots stop taking actions after `action_cap` actions in a turn. Without a
//! cap, search can wander into pointless loops (applying and discarding free
//! powers forever).

use crate::data::{Effect, Seat};
use crate::game::{Action, Game, Phase};
use crate::rules::hex_dist;

pub const DEFAULT_ACTION_CAP: u32 = 16;

pub trait Agent: Send {
    fn name(&self) -> String;
    fn choose(&mut self, game: &Game, seat: Seat) -> Action;
}

/// Who has to act now: the player answering a prompt, or the current player.
pub fn to_move(game: &Game) -> Seat {
    game.pending
        .as_ref()
        .map(|p| p.seat)
        .unwrap_or(game.current)
}

/// The legal actions, except that after `cap` actions in a turn the only
/// option is to end it (prompts can still be answered).
pub fn bot_actions(game: &Game, seat: Seat, cap: u32) -> Vec<Action> {
    if game.pending.is_none() && game.phase == Phase::Main && game.actions_this_turn >= cap {
        return vec![Action::EndTurn { score: false }];
    }
    game.legal_actions(seat)
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

/// How good the position looks for `seat`, from -1 (lost) to 1 (won).
pub fn evaluate(game: &Game, seat: Seat) -> f32 {
    evaluate_with(game, seat, &Weights::default())
}

pub fn evaluate_with(game: &Game, seat: Seat, w: &Weights) -> f32 {
    let shown = game.pending.as_ref().map(|p| &*p.preview).unwrap_or(game);
    if let Phase::Over { winner } = shown.phase {
        return if winner == seat { 1.0 } else { -1.0 };
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
        return 0.97;
    }
    let need_me = target - expected[me];
    let need_them = target - expected[them];
    let scoring_now =
        (expected[me] - shown.scores[me] as f32) - (expected[them] - shown.scores[them] as f32);

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
            let active = crate::data::data().powers[p.power].kind == crate::data::Kind::Active;
            powers[owner] += match (p.card, active) {
                (false, _) => 0.3,
                (true, false) => 1.0,
                (true, true) => 1.3,
            };
        }
        if u.powers.iter().any(|p| {
            crate::data::data()
                .effect(p.power)
                .is_some_and(moves_itself)
        }) {
            mobility[owner] += 1.0;
        }
        if let Some(f) = flag_cell
            && !u.is_flag()
        {
            presence[owner] += match hex_dist(u.cell, f) {
                1 => 1.0,
                2 => 0.45,
                3 => 0.2,
                _ => 0.05,
            };
        }
    }
    let diff = |a: [f32; 2]| a[me] - a[them];
    let z = w.points * (need_them - need_me)
        + w.scoring_now * scoring_now
        + w.flag_margin * (flag[me] as f32 - flag[them] as f32)
        + w.presence * diff(presence)
        + w.units * diff(units)
        + w.powers * diff(powers)
        + w.mobility * diff(mobility)
        // Cards in hand are options, but there's only so much room to use them.
        + w.hand * (shown.hands[me].len().min(6) as f32 - shown.hands[them].len().min(6) as f32);
    (w.scale * z).tanh() * 0.95
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
fn judge(game: &Game, seat: Seat, action: &Action) -> f32 {
    match game.after(seat, action) {
        Some(next) => evaluate(&next, seat),
        None => -2.0,
    }
}

pub struct GreedyAgent {
    rng: Rng,
    pub action_cap: u32,
}

impl GreedyAgent {
    pub fn new(seed: u64) -> GreedyAgent {
        GreedyAgent {
            rng: Rng::new(seed),
            action_cap: DEFAULT_ACTION_CAP,
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
            let score = judge(game, seat, action) + self.rng.unit() * 1e-3;
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
    fn new(game: Game) -> Node {
        let mover = to_move(&game);
        let terminal = match game.phase {
            Phase::Over { winner } => Some(if winner == 0 { 1.0 } else { -1.0 }),
            _ => None,
        };
        let value = terminal.unwrap_or_else(|| evaluate(&game, 0));
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

    fn expand(&self, nodes: &mut [Node], index: usize) {
        let node = &mut nodes[index];
        node.expanded = true;
        let actions = bot_actions(&node.game, node.mover, self.config.action_cap);
        // Priors: a softmax over how good each action's result looks.
        let scores: Vec<f32> = actions
            .iter()
            .map(|a| judge(&node.game, node.mover, a))
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
    fn search(&self, game: Game, simulations: u32) -> (Vec<Action>, Vec<u32>, Vec<f32>) {
        let mut nodes = vec![Node::new(game)];
        self.expand(&mut nodes, 0);
        for _ in 0..simulations {
            let mut path: Vec<(usize, usize)> = Vec::new();
            let mut index = 0;
            let value = loop {
                if let Some(t) = nodes[index].terminal {
                    break t;
                }
                if !nodes[index].expanded {
                    self.expand(&mut nodes, index);
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
                        nodes.push(Node::new(next));
                        nodes[index].edges[e].child = Some(child);
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
        format!("mcts{}x{}", self.config.simulations, self.config.samples)
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
            let (found, v, q) = self.search(sample, per_sample);
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
        let pick = if self.config.temperature <= 0.0 {
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

    #[test]
    fn evaluation_is_symmetric() {
        let game = new_game(1);
        for seat in 0..2 {
            assert!((evaluate(&game, seat) + evaluate(&game, 1 - seat)).abs() < 1e-5);
        }
    }
}
