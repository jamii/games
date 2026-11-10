//! Play many bot-vs-bot games in parallel and write one JSON line per game,
//! for balance analysis (see tools/analyze.py).
//!
//!   cargo run --release --bin selfplay -- --games 100 --agents mcts:200x4,mcts:200x4 --out games.jsonl
//!
//! Options:
//!   --games N              number of games (default 20)
//!   --first-seed S         seeds S, S+1, ... (default 0); the same seeds give
//!                          the same deals, so variants can be compared game by game
//!   --threads T            worker threads (default: all cores)
//!   --agents A,B           agents: random, greedy, mcts:SIMSxSAMPLES (default mcts:200x4 for both)
//!                          When the agents differ, they swap seats every other game.
//!   --max-turns M          stop a game after M turns and count it as unfinished (default 80)
//!   --second-player-points K
//!   --target-score K
//!   --remove NAME[,NAME]   leave these cards out of the deck
//!   --fixed-flag           rules variant: no power can move the flag
//!   --buried-count         rules variant: buried units still count at the flag
//!   --give WHO:NAME[,...]  leave these cards out of the deck and give them to
//!                          the first or second player (WHO is first or second)
//!                          when the draft ends. Compare with --remove NAME on
//!                          the same seeds to measure what a card is worth.
//!   --variant LABEL        label stored with each game (default: built from the options)
//!   --out FILE             append results here (default: print to stdout)
//!   --positions FILE       also append a sample of positions (the start of every
//!                          turn, from both players' points of view) with who won,
//!                          for tools/fit_eval.py
//!   --weights FILE         learned evaluation (from tools/fit_eval.py) for the
//!                          agents greedyL and mctsL:SIMSxSAMPLES
//!   --net FILE             neural network (from tools/train.py) for the agents
//!                          policy (no search) and mctsN:SIMSxSAMPLES. To pit two
//!                          networks against each other, add @FILE to an agent
//!                          instead, eg mctsN:200x4@net3.json
//!   --samples FILE         append training data for tools/train.py: every
//!                          searched decision, with the search's visit counts
//!                          and who won
//!   --noise E              mix E of Dirichlet noise into the root priors (eg 0.25)
//!   --explore-turns T      for the first T turns, pick actions in proportion to
//!                          their visits instead of the most visited (default 0)

use game_001::bot::{
    Agent, Evaluator, GreedyAgent, Learned, MctsAgent, MctsConfig, PolicyAgent, RandomAgent,
    features, to_move,
};
use game_001::data::data;
use game_001::encode::{encode_actions, observe};
use game_001::game::{Action, Game, Options, Phase};
use game_001::net::Net;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Instant;

/// Settings shared by all games.
struct Setup {
    learned: Option<Arc<Learned>>,
    net: Option<Arc<Net>>,
    /// Networks named in agent specs (AGENT@FILE).
    nets: BTreeMap<String, Arc<Net>>,
    noise: f32,
    explore_turns: u32,
    samples: bool,
}

fn agent(full_spec: &str, seed: u64, setup: &Setup) -> Box<dyn Agent> {
    let (spec, net_path) = match full_spec.split_once('@') {
        Some((spec, path)) => (spec, Some(path)),
        None => (full_spec, None),
    };
    let learned = &setup.learned;
    let net = || match net_path {
        Some(path) => setup.nets[path].clone(),
        None => setup
            .net
            .clone()
            .unwrap_or_else(|| panic!("{spec} needs --net")),
    };
    let learned_eval = || {
        Evaluator::Learned(
            learned
                .clone()
                .unwrap_or_else(|| panic!("{spec} needs --weights")),
        )
    };
    match spec {
        "random" => Box::new(RandomAgent::new(seed)),
        "greedy" => Box::new(GreedyAgent::new(seed)),
        "policy" => Box::new(PolicyAgent {
            net: net(),
            action_cap: game_001::bot::DEFAULT_ACTION_CAP,
        }),
        "greedyL" => {
            let mut a = GreedyAgent::new(seed);
            a.eval = learned_eval();
            Box::new(a)
        }
        _ => {
            let mut config = MctsConfig::default();
            let rest = if let Some(rest) = spec.strip_prefix("mctsL") {
                config.eval = learned_eval();
                rest
            } else if let Some(rest) = spec.strip_prefix("mctsN") {
                config.eval = Evaluator::Net(net());
                // The network's priors are already a distribution over actions.
                rest
            } else {
                spec.strip_prefix("mcts")
                    .unwrap_or_else(|| panic!("unknown agent {spec}"))
            };
            config.root_noise = setup.noise;
            if setup.explore_turns > 0 {
                config.temperature = 1.0;
                config.temperature_turns = setup.explore_turns;
            }
            if let Some(numbers) = rest.strip_prefix(':') {
                let (sims, samples) = numbers.split_once('x').unwrap_or((numbers, "4"));
                config.simulations = sims.parse().expect("simulations");
                config.samples = samples.parse().expect("samples");
            }
            Box::new(MctsAgent::new(config, seed))
        }
    }
}

/// Stop a game after this many decisions, in case something loops.
const MAX_DECISIONS: u32 = 5000;

fn power_index(name: &str) -> usize {
    data()
        .powers
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("no power called {name}"))
}

#[derive(Default)]
struct Stats {
    drafted: [Vec<usize>; 2],
    taken_faceup: [Vec<usize>; 2],
    deck_draws: [u32; 2],
    applied: [Vec<usize>; 2],
    free_applied: [u32; 2],
    used: [BTreeMap<String, u32>; 2],
    spawned: [u32; 2],
    units_lost: [u32; 2],
    decisions: [u32; 2],
    seconds: [f64; 2],
}

fn owned(game: &Game) -> [u32; 2] {
    let mut n = [0, 0];
    for u in &game.units {
        if let Some(o) = u.owner {
            n[o] += 1;
        }
    }
    n
}

/// A position from one player's point of view, and the turn it's from.
type Sample = (usize, u32, Vec<f32>);

/// A searched decision, for training a network: (seat, encoded decision).
type Decision = (usize, Value);

/// Play one game, recording what each player did.
fn play(
    seed: u64,
    options: &Options,
    specs: &[String; 2],
    max_turns: u32,
    setup: &Setup,
) -> (Value, Vec<Sample>, Vec<Decision>) {
    let (mut game, _) = Game::new(Options {
        seed: Some(seed),
        ..options.clone()
    })
    .unwrap();
    // Swap seats every other game when the agents differ, so neither gets a
    // particular seat more often.
    let swap = specs[0] != specs[1] && seed % 2 == 1;
    let seat_spec = |seat: usize| specs[if swap { 1 - seat } else { seat }].clone();
    let mut agents: [Box<dyn Agent>; 2] = [
        agent(&seat_spec(0), seed * 2 + 11, setup),
        agent(&seat_spec(1), seed * 2 + 12, setup),
    ];
    let mut stats = Stats::default();
    let mut samples: Vec<Sample> = Vec::new();
    let mut decisions: Vec<Decision> = Vec::new();
    // Every action, to replay a game that hit an engine bug.
    let mut history: Vec<(usize, Action)> = Vec::new();
    let started = Instant::now();
    while !matches!(game.phase, Phase::Over { .. }) && game.turn <= max_turns {
        let seat = to_move(&game);
        let t = Instant::now();
        let action = agents[seat].choose(&game, seat);
        let took = t.elapsed().as_secs_f64();
        stats.seconds[seat] += took;
        let total = stats.decisions[0] + stats.decisions[1];
        if total > 0 && total % 500 == 0 {
            eprintln!(
                "long game: seed {seed}, {total} decisions, turn {}, phase {:?}, prompt {:?}, next {action:?}",
                game.turn,
                game.phase,
                game.prompt().map(|p| &p.text)
            );
        }
        if total >= MAX_DECISIONS {
            eprintln!("seed {seed}: stopped after {total} decisions");
            break;
        }
        if took > 2.0 {
            eprintln!(
                "slow decision: seed {seed}, turn {}, {:.1}s, {} units, {} cards in hands",
                game.turn,
                took,
                game.units.len(),
                game.hands[0].len() + game.hands[1].len()
            );
        }
        stats.decisions[seat] += 1;
        if setup.samples
            && let Some(search) = agents[seat].last_search()
        {
            let obs = observe(&game, seat);
            let encoded = encode_actions(&game, seat, &obs, &search.actions);
            let total: u32 = search.visits.iter().sum();
            let q: f32 = search
                .visits
                .iter()
                .zip(&search.values)
                .filter(|(v, _)| **v > 0)
                .map(|(v, q)| *v as f32 * q)
                .sum::<f32>()
                / total.max(1) as f32;
            decisions.push((
                seat,
                json!({
                    "seed": seed,
                    "turn": game.turn,
                    "seat": seat,
                    "obs": obs,
                    "actions": encoded,
                    "visits": search.visits,
                    "q": q,
                    "layout": game_001::encode::layout(),
                }),
            ));
        }
        match &action {
            Action::TakeFaceup { slot } => {
                let card = game.faceup[*slot];
                if game.phase == Phase::Draft {
                    stats.drafted[seat].push(card);
                } else {
                    stats.taken_faceup[seat].push(card);
                }
            }
            Action::TakeDeck => stats.deck_draws[seat] += 1,
            Action::ApplyHand { card, .. } => stats.applied[seat].push(*card),
            Action::ApplyFree { .. } => stats.free_applied[seat] += 1,
            Action::Spawn { .. } => stats.spawned[seat] += 1,
            Action::Activate {
                unit,
                power: Some(i),
                ..
            } => {
                if let Some(u) = game.units.iter().find(|u| u.id == *unit) {
                    let name = data().powers[u.powers[*i].power].name.clone();
                    *stats.used[seat].entry(name).or_default() += 1;
                }
            }
            _ => {}
        }
        let before = owned(&game);
        if let Err(e) = game.apply(seat, &action) {
            // A bug in the rules engine (a legal action that can't be carried
            // out). Stop this game rather than the whole run.
            eprintln!(
                "seed {seed}: ENGINE BUG: legal action {action:?} failed: {} (game stopped)",
                e.message
            );
            if std::env::var_os("GAME_DEBUG").is_some() {
                history.push((seat, action.clone()));
                let path = format!("bug-seed-{seed}.json");
                let dump = json!({ "options": options, "seed": seed, "actions": history });
                std::fs::write(&path, dump.to_string()).expect("write the bug report");
                eprintln!("wrote the game's actions to {path}");
            }
            break;
        }
        history.push((seat, action.clone()));
        let after = owned(&game);
        for s in 0..2 {
            stats.units_lost[s] += before[s].saturating_sub(after[s]);
        }
        if matches!(action, Action::EndTurn { .. }) && game.pending.is_none() {
            for s in 0..2 {
                if let Ok(f) = features(&game, s) {
                    samples.push((s, game.turn, f));
                }
            }
        }
    }
    let names = |cards: &Vec<usize>| -> Vec<String> {
        cards
            .iter()
            .map(|&c| data().powers[c].name.clone())
            .collect()
    };
    let winner = match game.phase {
        Phase::Over { winner } => Some(winner),
        _ => None,
    };
    let final_units: Vec<Vec<Value>> = (0..2)
        .map(|s| {
            game.units
                .iter()
                .filter(|u| u.owner == Some(s))
                .map(|u| {
                    json!(
                        u.powers
                            .iter()
                            .map(|p| data().powers[p.power].name.clone())
                            .collect::<Vec<_>>()
                    )
                })
                .collect()
        })
        .collect();
    let record = json!({
        "seed": seed,
        "agents": [seat_spec(0), seat_spec(1)],
        "first_player": game.first_player,
        "winner": winner,
        "scores": game.scores,
        "turns": game.turn,
        "seconds": started.elapsed().as_secs_f64(),
        "decisions": stats.decisions,
        "think_seconds": stats.seconds,
        "drafted": [names(&stats.drafted[0]), names(&stats.drafted[1])],
        "taken_faceup": [names(&stats.taken_faceup[0]), names(&stats.taken_faceup[1])],
        "deck_draws": stats.deck_draws,
        "applied": [names(&stats.applied[0]), names(&stats.applied[1])],
        "free_applied": stats.free_applied,
        "used": stats.used,
        "spawned": stats.spawned,
        "units_lost": stats.units_lost,
        "final_units": final_units,
        "final_hands": [names(&game.hands[0]), names(&game.hands[1])],
    });
    let z = |seat: usize| match winner {
        Some(w) if w == seat => 1.0,
        Some(_) => -1.0,
        None => 0.0,
    };
    for (seat, d) in decisions.iter_mut() {
        d["z"] = json!(z(*seat));
    }
    (record, samples, decisions)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let games: u64 = arg("--games").map_or(20, |s| s.parse().expect("--games"));
    let first: u64 = arg("--first-seed").map_or(0, |s| s.parse().expect("--first-seed"));
    let threads: usize = arg("--threads")
        .map(|s| s.parse().expect("--threads"))
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let specs: Vec<String> = arg("--agents")
        .unwrap_or_else(|| "mcts:200x4,mcts:200x4".into())
        .split(',')
        .map(String::from)
        .collect();
    let specs: [String; 2] = [specs[0].clone(), specs.get(1).unwrap_or(&specs[0]).clone()];
    let max_turns: u32 = arg("--max-turns").map_or(80, |s| s.parse().expect("--max-turns"));
    let mut options = Options::default();
    if let Some(k) = arg("--second-player-points") {
        options.second_player_points = k.parse().expect("--second-player-points");
    }
    if let Some(k) = arg("--target-score") {
        options.target_score = k.parse().expect("--target-score");
    }
    if let Some(names) = arg("--remove") {
        options.removed = names.split(',').map(power_index).collect();
    }
    if args.iter().any(|a| a == "--fixed-flag") {
        options.fixed_flag = true;
    }
    if args.iter().any(|a| a == "--buried-count") {
        options.buried_count = true;
    }
    if let Some(gifts) = arg("--give") {
        for gift in gifts.split(',') {
            match gift.split_once(':') {
                Some(("first", name)) => options.given_first.push(power_index(name)),
                Some(("second", name)) => options.given_second.push(power_index(name)),
                _ => panic!("--give expects first:CARD or second:CARD, not {gift}"),
            }
        }
    }
    let variant = arg("--variant").unwrap_or_else(|| {
        let mut v = format!(
            "p2={},target={}",
            options.second_player_points, options.target_score
        );
        if let Some(r) = arg("--remove") {
            v += &format!(",remove={r}");
        }
        if let Some(g) = arg("--give") {
            v += &format!(",give={g}");
        }
        if options.fixed_flag {
            v += ",fixed-flag";
        }
        if options.buried_count {
            v += ",buried-count";
        }
        v
    });
    let learned = arg("--weights")
        .map(|path| Arc::new(Learned::load(&path).unwrap_or_else(|e| panic!("{e}"))));
    let setup = Setup {
        learned,
        net: arg("--net").map(|path| Arc::new(Net::load(&path).unwrap_or_else(|e| panic!("{e}")))),
        nets: specs
            .iter()
            .filter_map(|s| s.split_once('@'))
            .map(|(_, path)| {
                let net = Net::load(path).unwrap_or_else(|e| panic!("{e}"));
                (path.to_string(), Arc::new(net))
            })
            .collect(),
        noise: arg("--noise").map_or(0.0, |s| s.parse().expect("--noise")),
        explore_turns: arg("--explore-turns").map_or(0, |s| s.parse().expect("--explore-turns")),
        samples: arg("--samples").is_some(),
    };
    let mut sample_file = arg("--samples").map(|path| {
        std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("--samples"),
        )
    });
    let mut positions = arg("--positions").map(|path| {
        std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("--positions"),
        )
    });
    let mut out: Box<dyn Write> = match arg("--out") {
        Some(path) => Box::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("--out"),
        ),
        None => Box::new(std::io::stdout()),
    };

    eprintln!(
        "{games} games ({variant}, {} vs {}) on {threads} threads",
        specs[0], specs[1]
    );
    let next = AtomicU64::new(first);
    let (tx, rx) = mpsc::channel::<(Value, Vec<Sample>, Vec<Decision>)>();
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let (tx, next, options, specs, setup) = (tx.clone(), &next, &options, &specs, &setup);
            scope.spawn(move || {
                loop {
                    let seed = next.fetch_add(1, Ordering::SeqCst);
                    if seed >= first + games {
                        break;
                    }
                    let _ = tx.send(play(seed, options, specs, max_turns, setup));
                }
            });
        }
        drop(tx);
        let mut done = 0;
        let mut wins: BTreeMap<String, u32> = BTreeMap::new();
        for (mut record, samples, decisions) in rx {
            record["variant"] = json!(variant);
            if let Some(w) = sample_file.as_mut() {
                for (_, mut d) in decisions {
                    d["variant"] = json!(variant);
                    writeln!(w, "{d}").expect("write samples");
                }
                w.flush().expect("flush samples");
            }
            if let (Some(w), Some(winner)) = (positions.as_mut(), record["winner"].as_u64()) {
                for (seat, turn, f) in samples {
                    let line = json!({ "x": f, "y": (seat as u64 == winner) as u8, "turn": turn, "variant": variant });
                    writeln!(w, "{line}").expect("write positions");
                }
                w.flush().expect("flush positions");
            }
            writeln!(out, "{record}").expect("write");
            out.flush().expect("flush");
            done += 1;
            let who = match record["winner"].as_u64() {
                Some(w) => record["agents"][w as usize]
                    .as_str()
                    .unwrap_or("?")
                    .to_string(),
                None => "unfinished".into(),
            };
            *wins.entry(who).or_default() += 1;
            eprintln!(
                "[{done}/{games}] seed {} won by {} ({}-{} in {} turns, {:.0}s) | totals {wins:?} | {:.1} min",
                record["seed"],
                record["winner"],
                record["scores"][0],
                record["scores"][1],
                record["turns"],
                record["seconds"].as_f64().unwrap_or(0.0),
                started.elapsed().as_secs_f64() / 60.0
            );
        }
    });
}
