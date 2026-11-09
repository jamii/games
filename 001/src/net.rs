//! A small transformer that scores positions and actions (stage 2 of BOT.md),
//! run on the CPU. It's trained by tools/train.py, which writes the weights as
//! JSON; this file mirrors the model in that script, layer for layer.
//!
//! Input: the tokens from `encode` (one for the game, one per unit). Each is
//! turned into a vector by adding up learned feature vectors, then a few
//! transformer layers let the tokens look at each other. The game token gives
//! the value (from -1, lost, to 1, won, for the player to move). Each legal
//! action gets a score (a logit) from its own features, the units it
//! involves and the game token; a softmax over the scores gives the policy.

use crate::encode::{ActionToken, Layout, Observation, Token, layout};
use serde_json::Value;

#[derive(Clone, Debug)]
struct Linear {
    /// [out][in], row-major, like PyTorch.
    w: Vec<f32>,
    b: Option<Vec<f32>>,
    inputs: usize,
    outputs: usize,
}

impl Linear {
    fn apply(&self, x: &[f32], out: &mut [f32]) {
        debug_assert_eq!(x.len(), self.inputs);
        for (o, y) in out.iter_mut().enumerate().take(self.outputs) {
            let row = &self.w[o * self.inputs..(o + 1) * self.inputs];
            *y = dot(row, x) + self.b.as_ref().map_or(0.0, |b| b[o]);
        }
    }
    fn call(&self, x: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0; self.outputs];
        self.apply(x, &mut out);
        out
    }
}

/// A dot product written so the compiler can vectorise it.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let chunks = a.len() / 8;
    for i in 0..chunks {
        let (x, y) = (&a[i * 8..i * 8 + 8], &b[i * 8..i * 8 + 8]);
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut sum: f32 = acc.iter().sum();
    for i in chunks * 8..a.len() {
        sum += a[i] * b[i];
    }
    sum
}

#[derive(Clone, Debug)]
struct Norm {
    w: Vec<f32>,
    b: Vec<f32>,
}

impl Norm {
    fn apply(&self, x: &[f32]) -> Vec<f32> {
        let n = x.len() as f32;
        let mean = x.iter().sum::<f32>() / n;
        let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + 1e-5).sqrt();
        x.iter()
            .zip(self.w.iter().zip(&self.b))
            .map(|(v, (w, b))| (v - mean) * inv * w + b)
            .collect()
    }
}

#[derive(Clone, Debug)]
struct Block {
    norm1: Norm,
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    norm2: Norm,
    ff1: Linear,
    ff2: Linear,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub layout: Layout,
    d: usize,
    heads: usize,
    emb_global: Vec<f32>,
    emb_unit: Vec<f32>,
    emb_action: Vec<f32>,
    blocks: Vec<Block>,
    norm: Norm,
    value1: Linear,
    value2: Linear,
    from_unit: Linear,
    from_target: Linear,
    from_global: Linear,
    policy1: Linear,
    policy2: Linear,
}

/// The network's output for one position.
#[derive(Clone, Debug)]
pub struct Output {
    /// For the player whose observation it was, from -1 to 1.
    pub value: f32,
    /// One per action, in the order given.
    pub logits: Vec<f32>,
}

impl Net {
    /// Reads the JSON written by tools/train.py.
    pub fn load(path: &str) -> Result<Net, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        Net::from_json(&json).map_err(|e| format!("{path}: {e}"))
    }

    pub fn from_json(json: &Value) -> Result<Net, String> {
        let want = layout();
        let l = &json["layout"];
        let got = |k: &str| l[k].as_u64().unwrap_or(0) as usize;
        if (got("global"), got("unit"), got("action"), got("cards"))
            != (want.global, want.unit, want.action, want.cards)
        {
            return Err(format!(
                "the weights are for a different encoding ({l}, now {want:?}): were cards \
                 or the board changed? Train again."
            ));
        }
        let config = &json["config"];
        let d = config["d"].as_u64().ok_or("no config.d")? as usize;
        let heads = config["heads"].as_u64().ok_or("no config.heads")? as usize;
        let layers = config["layers"].as_u64().ok_or("no config.layers")? as usize;
        let tensors = &json["tensors"];
        let tensor = |name: &str| -> Result<(Vec<usize>, Vec<f32>), String> {
            let t = &tensors[name];
            let shape: Vec<usize> = t["shape"]
                .as_array()
                .ok_or_else(|| format!("missing tensor {name}"))?
                .iter()
                .map(|v| v.as_u64().unwrap_or(0) as usize)
                .collect();
            let data: Vec<f32> = t["data"]
                .as_array()
                .ok_or_else(|| format!("missing data for {name}"))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            if data.len() != shape.iter().product::<usize>() {
                return Err(format!(
                    "{name}: shape {shape:?} but {} numbers",
                    data.len()
                ));
            }
            Ok((shape, data))
        };
        let linear = |name: &str, bias: bool| -> Result<Linear, String> {
            let (shape, w) = tensor(&format!("{name}.weight"))?;
            let b = if bias {
                Some(tensor(&format!("{name}.bias"))?.1)
            } else {
                None
            };
            Ok(Linear {
                w,
                b,
                inputs: shape[1],
                outputs: shape[0],
            })
        };
        let norm = |name: &str| -> Result<Norm, String> {
            Ok(Norm {
                w: tensor(&format!("{name}.weight"))?.1,
                b: tensor(&format!("{name}.bias"))?.1,
            })
        };
        let mut blocks = Vec::new();
        for i in 0..layers {
            let p = format!("blocks.{i}");
            blocks.push(Block {
                norm1: norm(&format!("{p}.norm1"))?,
                q: linear(&format!("{p}.q"), true)?,
                k: linear(&format!("{p}.k"), true)?,
                v: linear(&format!("{p}.v"), true)?,
                o: linear(&format!("{p}.o"), true)?,
                norm2: norm(&format!("{p}.norm2"))?,
                ff1: linear(&format!("{p}.ff1"), true)?,
                ff2: linear(&format!("{p}.ff2"), true)?,
            });
        }
        let net = Net {
            layout: want,
            d,
            heads,
            emb_global: tensor("emb_global.weight")?.1,
            emb_unit: tensor("emb_unit.weight")?.1,
            emb_action: tensor("emb_action.weight")?.1,
            blocks,
            norm: norm("norm")?,
            value1: linear("value1", true)?,
            value2: linear("value2", true)?,
            from_unit: linear("from_unit", false)?,
            from_target: linear("from_target", false)?,
            from_global: linear("from_global", false)?,
            policy1: linear("policy1", true)?,
            policy2: linear("policy2", true)?,
        };
        net.verify(&json["checks"])?;
        Ok(net)
    }

    /// tools/train.py stores a few positions with its outputs. Getting the same
    /// outputs here shows this file and the script agree on the model.
    fn verify(&self, checks: &Value) -> Result<(), String> {
        let Some(checks) = checks.as_array() else {
            return Ok(());
        };
        for (n, check) in checks.iter().enumerate() {
            let obs: Observation = serde_json::from_value(check["obs"].clone())
                .map_err(|e| format!("check {n}: {e}"))?;
            let actions: Vec<ActionToken> = serde_json::from_value(check["actions"].clone())
                .map_err(|e| format!("check {n}: {e}"))?;
            let out = self.forward(&obs, &actions);
            let mut expected = vec![check["value"].as_f64().unwrap_or(0.0) as f32];
            expected.extend(
                check["logits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32),
            );
            let mut got = vec![out.value];
            got.extend(&out.logits);
            let worst = expected
                .iter()
                .zip(&got)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            if expected.len() != got.len() || worst > 1e-3 {
                return Err(format!(
                    "check {n}: this network computes different outputs from \
                     tools/train.py (off by {worst}): src/net.rs and the script disagree"
                ));
            }
        }
        Ok(())
    }

    fn embed(&self, table: &[f32], token: &Token) -> Vec<f32> {
        let d = self.d;
        let mut x = vec![0f32; d];
        for (&id, &v) in token.ids.iter().zip(&token.vals) {
            let row = &table[id as usize * d..(id as usize + 1) * d];
            for (xi, r) in x.iter_mut().zip(row) {
                *xi += v * r;
            }
        }
        x
    }

    pub fn forward(&self, obs: &Observation, actions: &[ActionToken]) -> Output {
        let d = self.d;
        let mut xs: Vec<Vec<f32>> = Vec::with_capacity(obs.units.len() + 1);
        xs.push(self.embed(&self.emb_global, &obs.global));
        for u in &obs.units {
            xs.push(self.embed(&self.emb_unit, u));
        }
        let n = xs.len();
        let dh = d / self.heads;
        let scale = 1.0 / (dh as f32).sqrt();
        let mut scores = vec![0f32; n];
        for block in &self.blocks {
            let hs: Vec<Vec<f32>> = xs.iter().map(|x| block.norm1.apply(x)).collect();
            let qs: Vec<Vec<f32>> = hs.iter().map(|h| block.q.call(h)).collect();
            let ks: Vec<Vec<f32>> = hs.iter().map(|h| block.k.call(h)).collect();
            let vs: Vec<Vec<f32>> = hs.iter().map(|h| block.v.call(h)).collect();
            let mut attended = vec![0f32; d];
            let mut out = vec![0f32; d];
            for i in 0..n {
                attended.iter_mut().for_each(|a| *a = 0.0);
                for h in 0..self.heads {
                    let r = h * dh..(h + 1) * dh;
                    let q = &qs[i][r.clone()];
                    let mut max = f32::MIN;
                    for j in 0..n {
                        scores[j] = dot(q, &ks[j][r.clone()]) * scale;
                        max = max.max(scores[j]);
                    }
                    let mut sum = 0.0;
                    for s in scores.iter_mut() {
                        *s = (*s - max).exp();
                        sum += *s;
                    }
                    for j in 0..n {
                        let w = scores[j] / sum;
                        for (a, v) in attended[r.clone()].iter_mut().zip(&vs[j][r.clone()]) {
                            *a += w * v;
                        }
                    }
                }
                block.o.apply(&attended, &mut out);
                for (x, o) in xs[i].iter_mut().zip(&out) {
                    *x += o;
                }
            }
            for x in xs.iter_mut() {
                let h = block.norm2.apply(x);
                let mut f = block.ff1.call(&h);
                f.iter_mut().for_each(|v| *v = v.max(0.0));
                block.ff2.apply(&f, &mut out);
                for (xi, o) in x.iter_mut().zip(&out) {
                    *xi += o;
                }
            }
        }
        let xs: Vec<Vec<f32>> = xs.iter().map(|x| self.norm.apply(x)).collect();
        let global = &xs[0];
        let mut hidden = self.value1.call(global);
        hidden.iter_mut().for_each(|v| *v = v.max(0.0));
        let value = self.value2.call(&hidden)[0].tanh();

        let context = self.from_global.call(global);
        let unit_parts: Vec<Vec<f32>> = xs[1..].iter().map(|u| self.from_unit.call(u)).collect();
        let target_parts: Vec<Option<Vec<f32>>> = {
            let mut parts = vec![None; n - 1];
            for a in actions {
                if a.target >= 0 && parts[a.target as usize].is_none() {
                    parts[a.target as usize] =
                        Some(self.from_target.call(&xs[1 + a.target as usize]));
                }
            }
            parts
        };
        let mut hidden = vec![0f32; self.policy1.outputs];
        let logits = actions
            .iter()
            .map(|a| {
                let mut e = self.embed(&self.emb_action, &a.token);
                for (ei, c) in e.iter_mut().zip(&context) {
                    *ei += c;
                }
                if a.unit >= 0 {
                    for (ei, u) in e.iter_mut().zip(&unit_parts[a.unit as usize]) {
                        *ei += u;
                    }
                }
                if a.target >= 0
                    && let Some(t) = &target_parts[a.target as usize]
                {
                    for (ei, u) in e.iter_mut().zip(t) {
                        *ei += u;
                    }
                }
                e.iter_mut().for_each(|v| *v = v.max(0.0));
                self.policy1.apply(&e, &mut hidden);
                hidden.iter_mut().for_each(|v| *v = v.max(0.0));
                self.policy2.call(&hidden)[0]
            })
            .collect();
        Output { value, logits }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A network with random weights, as tools/train.py would write it.
    pub(crate) fn random_net(seed: u64) -> Net {
        let l = layout();
        let (d, heads, layers, ff) = (16, 2, 2, 32);
        let mut rng = crate::bot::Rng::new(seed);
        let mut tensors = serde_json::Map::new();
        let mut add = |name: &str, shape: Vec<usize>| {
            let n: usize = shape.iter().product();
            let data: Vec<f32> = (0..n).map(|_| rng.unit() - 0.5).collect();
            tensors.insert(
                name.into(),
                serde_json::json!({ "shape": shape, "data": data }),
            );
        };
        add("emb_global.weight", vec![l.global, d]);
        add("emb_unit.weight", vec![l.unit, d]);
        add("emb_action.weight", vec![l.action, d]);
        for i in 0..layers {
            for norm in ["norm1", "norm2"] {
                add(&format!("blocks.{i}.{norm}.weight"), vec![d]);
                add(&format!("blocks.{i}.{norm}.bias"), vec![d]);
            }
            for lin in ["q", "k", "v", "o"] {
                add(&format!("blocks.{i}.{lin}.weight"), vec![d, d]);
                add(&format!("blocks.{i}.{lin}.bias"), vec![d]);
            }
            add(&format!("blocks.{i}.ff1.weight"), vec![ff, d]);
            add(&format!("blocks.{i}.ff1.bias"), vec![ff]);
            add(&format!("blocks.{i}.ff2.weight"), vec![d, ff]);
            add(&format!("blocks.{i}.ff2.bias"), vec![d]);
        }
        add("norm.weight", vec![d]);
        add("norm.bias", vec![d]);
        add("value1.weight", vec![d, d]);
        add("value1.bias", vec![d]);
        add("value2.weight", vec![1, d]);
        add("value2.bias", vec![1]);
        for p in ["from_unit", "from_target", "from_global"] {
            add(&format!("{p}.weight"), vec![d, d]);
        }
        add("policy1.weight", vec![d, d]);
        add("policy1.bias", vec![d]);
        add("policy2.weight", vec![1, d]);
        add("policy2.bias", vec![1]);
        let json = serde_json::json!({
            "layout": l,
            "config": { "d": d, "heads": heads, "layers": layers, "ff": ff },
            "tensors": tensors,
        });
        Net::from_json(&json).unwrap()
    }

    #[test]
    fn a_random_network_gives_finite_outputs() {
        let net = random_net(1);
        let (game, _) = crate::game::Game::new(crate::game::Options {
            seed: Some(1),
            ..Default::default()
        })
        .unwrap();
        let obs = crate::encode::observe(&game, 0);
        let actions = crate::bot::bot_actions(&game, 0, 16);
        let encoded = crate::encode::encode_actions(&game, 0, &obs, &actions);
        let out = net.forward(&obs, &encoded);
        assert!(out.value.abs() <= 1.0);
        assert_eq!(out.logits.len(), actions.len());
        assert!(out.logits.iter().all(|l| l.is_finite()));
    }
}
