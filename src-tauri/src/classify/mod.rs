//! Per-tag classifiers ("heads"): one logistic regression per tag on the fixed note
//! vectors, trained on the user's own tags and feedback, on their machine (see
//! `docs/decisions.md`, AI classification, Tag classifiers). The embedding model never
//! changes; only these few hundred numbers per tag are learned.
//!
//! Hand-rolled on purpose: the ML crates don't clearly support per-example weights and
//! warm starts, and the whole thing is a loss, its gradient and L-BFGS.

pub mod job;
pub mod review;

/// A tag needs this many positive examples before it gets a head.
pub const MIN_POSITIVES: usize = 10;
/// And this many negative ones.
pub const MIN_NEGATIVES: usize = 10;
/// A head suggests its tag at or above this probability.
pub const PROBABILITY_THRESHOLD: f32 = 0.5;
/// Weight of a note on which the user rejected the tag.
pub const REJECTED_WEIGHT: f32 = 3.0;
/// Weight of a note that just doesn't have the tag: it may simply not have been tagged yet.
pub const UNTAGGED_WEIGHT: f32 = 1.0;
/// Folds for the cross-validated precision and recall stored with a head.
pub const FOLDS: usize = 5;
/// Part of every head's input fingerprint: bump it when training changes, so every
/// stored head is retrained.
pub const TRAINING_VERSION: u32 = 1;

/// A trained head: `probability(x) = sigmoid(weights · x + bias)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub weights: Vec<f32>,
    pub bias: f32,
}

impl Head {
    /// The probability that a note with vector `x` has the tag. A vector of another
    /// length (another model) gets 0.
    pub fn probability(&self, x: &[f32]) -> f32 {
        if x.len() != self.weights.len() {
            return 0.0;
        }
        let z: f64 = self.weights.iter().zip(x).map(|(w, x)| f64::from(*w) * f64::from(*x)).sum::<f64>()
            + f64::from(self.bias);
        sigmoid(z) as f32
    }
}

/// One training example: a note vector, whether the note has the tag, and how much it counts.
#[derive(Debug, Clone, Copy)]
pub struct Example<'a> {
    pub x: &'a [f32],
    pub positive: bool,
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// L2 penalty on the weights (not the bias), relative to the mean example loss. The
    /// default (0.03) was picked on synthetic notes embedded with bge: weaker penalties
    /// let a head memorise the untagged notes that belong to the tag, which are exactly
    /// the notes worth suggesting it on (docs/decisions.md, Tag classifiers).
    pub l2: f64,
    pub max_iterations: usize,
    /// Training stops once no gradient component is larger than this.
    pub tolerance: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self { l2: 3e-2, max_iterations: 500, tolerance: 1e-5 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trained {
    pub head: Head,
    /// L-BFGS iterations it took; 0 when the starting point was already good enough.
    pub iterations: usize,
}

/// Cross-validated quality at [`PROBABILITY_THRESHOLD`]. `None` when it isn't defined
/// (no note was predicted positive, or there were no positives).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quality {
    pub precision: Option<f32>,
    pub recall: Option<f32>,
}

/// Scales the weights so positives and negatives count the same in total, keeping the
/// ratios within each class (a rejection still counts [`REJECTED_WEIGHT`] times an
/// untagged note). Without it a tag on 1% of notes would never reach probability 0.5.
pub fn balance(examples: &mut [Example]) {
    let total = |positive: bool| examples.iter().filter(|e| e.positive == positive).map(|e| f64::from(e.weight)).sum::<f64>();
    let (pos, neg) = (total(true), total(false));
    if pos <= 0.0 || neg <= 0.0 {
        return;
    }
    let half = (pos + neg) / 2.0;
    for e in examples.iter_mut() {
        let scale = if e.positive { half / pos } else { half / neg };
        e.weight = (f64::from(e.weight) * scale) as f32;
    }
}

/// Trains a head on `examples` (all vectors the same length), starting from `start`
/// when given (a warm start from the previous head), else from zero. Deterministic.
pub fn train(examples: &[Example], start: Option<&Head>, config: &Config) -> Trained {
    let dims = examples.first().map_or(0, |e| e.x.len());
    let problem = Problem::new(examples, dims, config.l2);
    // Trained on centred vectors, which conditions the problem much better (every bge
    // vector shares a large common direction); the bias is converted back at the end.
    let mut params = vec![0.0; dims + 1];
    if let Some(head) = start.filter(|h| h.weights.len() == dims) {
        for (p, w) in params.iter_mut().zip(&head.weights) {
            *p = f64::from(*w);
        }
        params[dims] = f64::from(head.bias) + dot(&params[..dims], &problem.mean);
    }
    let iterations = lbfgs(&mut params, config, |p, g| problem.loss_and_gradient(p, g));
    let bias = params[dims] - dot(&params[..dims], &problem.mean);
    Trained { head: Head { weights: params[..dims].iter().map(|w| *w as f32).collect(), bias: bias as f32 }, iterations }
}

/// Stratified `folds`-fold cross-validation: each fold is predicted by a head trained
/// on the others (warm-started from `start`), and precision and recall are counted over
/// all held-out predictions. Weights only shape training; every note counts once here.
/// The folds are shuffled with `seed`. Returns `None` as soon as `keep_going` says to stop.
pub fn cross_validate(
    examples: &[Example],
    folds: usize,
    seed: u64,
    start: Option<&Head>,
    config: &Config,
    keep_going: &dyn Fn() -> bool,
) -> Option<Quality> {
    let mut rng = Rng(seed);
    let mut fold_of = vec![0; examples.len()];
    for positive in [true, false] {
        let mut idx: Vec<usize> = (0..examples.len()).filter(|&i| examples[i].positive == positive).collect();
        rng.shuffle(&mut idx);
        for (k, i) in idx.into_iter().enumerate() {
            fold_of[i] = k % folds.max(1);
        }
    }
    let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);
    for fold in 0..folds.max(1) {
        if !keep_going() {
            return None;
        }
        let mut train_set: Vec<Example> = Vec::new();
        let mut test_set: Vec<&Example> = Vec::new();
        for (e, f) in examples.iter().zip(&fold_of) {
            if *f == fold { test_set.push(e) } else { train_set.push(*e) }
        }
        if test_set.is_empty() || train_set.is_empty() {
            continue;
        }
        balance(&mut train_set);
        let head = train(&train_set, start, config).head;
        for e in test_set {
            match (head.probability(e.x) >= PROBABILITY_THRESHOLD, e.positive) {
                (true, true) => tp += 1,
                (true, false) => fp += 1,
                (false, true) => fn_ += 1,
                (false, false) => {}
            }
        }
    }
    let ratio = |a: usize, b: usize| (a + b > 0).then(|| a as f32 / (a + b) as f32);
    Some(Quality { precision: ratio(tp, fp), recall: ratio(tp, fn_) })
}

/// Weighted mean log loss plus the L2 penalty, over centred vectors.
struct Problem<'a> {
    examples: &'a [Example<'a>],
    dims: usize,
    mean: Vec<f64>,
    total: f64,
    l2: f64,
}

impl<'a> Problem<'a> {
    fn new(examples: &'a [Example<'a>], dims: usize, l2: f64) -> Self {
        let total: f64 = examples.iter().map(|e| f64::from(e.weight)).sum();
        let mut mean = vec![0.0; dims];
        if total > 0.0 {
            for e in examples {
                let w = f64::from(e.weight) / total;
                mean.iter_mut().zip(e.x).for_each(|(m, x)| *m += w * f64::from(*x));
            }
        }
        Self { examples, dims, mean, total: if total > 0.0 { total } else { 1.0 }, l2 }
    }

    /// The loss at `params` (weights, then the bias); writes the gradient into `grad`.
    fn loss_and_gradient(&self, params: &[f64], grad: &mut [f64]) -> f64 {
        let (w, b) = params.split_at(self.dims);
        let b = b[0];
        let shift = dot(w, &self.mean);
        grad.iter_mut().for_each(|g| *g = 0.0);
        let mut loss = 0.0;
        let mut residual_sum = 0.0;
        for e in self.examples {
            let weight = f64::from(e.weight) / self.total;
            let z: f64 = w.iter().zip(e.x).map(|(w, x)| w * f64::from(*x)).sum::<f64>() - shift + b;
            // log(1 + e^-z) for positives, log(1 + e^z) for negatives, computed stably.
            let signed = if e.positive { z } else { -z };
            loss += weight * softplus(-signed);
            let residual = weight * (sigmoid(z) - if e.positive { 1.0 } else { 0.0 });
            residual_sum += residual;
            grad[..self.dims].iter_mut().zip(e.x).for_each(|(g, x)| *g += residual * f64::from(*x));
        }
        // d/dw of the centring term: -mean * sum(residuals).
        grad[..self.dims].iter_mut().zip(&self.mean).for_each(|(g, m)| *g -= residual_sum * m);
        grad[self.dims] = residual_sum;
        let norm2: f64 = w.iter().map(|w| w * w).sum();
        loss += 0.5 * self.l2 * norm2;
        grad[..self.dims].iter_mut().zip(w).for_each(|(g, w)| *g += self.l2 * w);
        loss
    }
}

/// Minimises `f` (which returns the value and writes the gradient) from `x` in place
/// with L-BFGS and a backtracking (Armijo) line search. Returns the iterations taken.
fn lbfgs(x: &mut [f64], config: &Config, mut f: impl FnMut(&[f64], &mut [f64]) -> f64) -> usize {
    const MEMORY: usize = 10;
    let n = x.len();
    let mut g = vec![0.0; n];
    let mut fx = f(x, &mut g);
    let mut history: std::collections::VecDeque<(Vec<f64>, Vec<f64>, f64)> = std::collections::VecDeque::new();
    let mut next = vec![0.0; n];
    let mut g_next = vec![0.0; n];
    for iteration in 0..config.max_iterations {
        if g.iter().all(|v| v.abs() <= config.tolerance) {
            return iteration;
        }
        // Two-loop recursion: d = -H g.
        let mut d: Vec<f64> = g.iter().map(|v| -v).collect();
        let mut alphas = Vec::with_capacity(history.len());
        for (s, y, rho) in history.iter().rev() {
            let a = rho * dot(s, &d);
            d.iter_mut().zip(y).for_each(|(d, y)| *d -= a * y);
            alphas.push(a);
        }
        if let Some((s, y, _)) = history.back() {
            let gamma = dot(s, y) / dot(y, y);
            d.iter_mut().for_each(|d| *d *= gamma);
        }
        for ((s, y, rho), a) in history.iter().zip(alphas.iter().rev()) {
            let b = rho * dot(y, &d);
            d.iter_mut().zip(s).for_each(|(d, s)| *d += (a - b) * s);
        }
        let mut slope = dot(&g, &d);
        if slope >= 0.0 {
            // Not a descent direction (shouldn't happen): fall back to the gradient.
            history.clear();
            d = g.iter().map(|v| -v).collect();
            slope = dot(&g, &d);
        }
        let mut step = 1.0;
        let mut accepted = false;
        for _ in 0..40 {
            next.iter_mut().zip(x.iter()).zip(&d).for_each(|((n, x), d)| *n = x + step * d);
            let f_next = f(&next, &mut g_next);
            if f_next <= fx + 1e-4 * step * slope {
                fx = f_next;
                accepted = true;
                break;
            }
            step *= 0.5;
        }
        if !accepted {
            return iteration + 1;
        }
        let s: Vec<f64> = next.iter().zip(x.iter()).map(|(n, x)| n - x).collect();
        let y: Vec<f64> = g_next.iter().zip(&g).map(|(a, b)| a - b).collect();
        let sy = dot(&s, &y);
        if sy > 1e-12 {
            if history.len() == MEMORY {
                history.pop_front();
            }
            history.push_back((s, y, 1.0 / sy));
        }
        x.copy_from_slice(&next);
        g.copy_from_slice(&g_next);
    }
    config.max_iterations
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 { 1.0 / (1.0 + (-z).exp()) } else { z.exp() / (1.0 + z.exp()) }
}

/// `log(1 + e^z)` without overflow.
fn softplus(z: f64) -> f64 {
    if z > 30.0 { z } else { z.exp().ln_1p() }
}

/// SplitMix64: a tiny deterministic generator for shuffling folds.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }
}

/// A stable seed for a tag's folds, so retraining the same data gives the same quality.
pub fn seed_for(tag: &str) -> u64 {
    tag.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points around (1, 0) are positive, around (-1, 0) negative, with a little spread.
    fn toy(n: usize, seed: u64) -> Vec<(Vec<f32>, bool)> {
        let mut rng = Rng(seed);
        let mut unit = || (rng.next() % 1000) as f32 / 1000.0 - 0.5;
        (0..n)
            .map(|i| {
                let positive = i % 2 == 0;
                let x = if positive { 1.0 } else { -1.0 } + 0.6 * unit();
                (vec![x, unit(), unit()], positive)
            })
            .collect()
    }

    fn examples(data: &[(Vec<f32>, bool)]) -> Vec<Example<'_>> {
        data.iter().map(|(x, p)| Example { x, positive: *p, weight: 1.0 }).collect()
    }

    #[test]
    fn learns_a_linearly_separable_toy_set() {
        let data = toy(60, 1);
        let trained = train(&examples(&data), None, &Config::default());
        for (x, positive) in &data {
            let p = trained.head.probability(x);
            assert_eq!(p >= PROBABILITY_THRESHOLD, *positive, "p={p} for {x:?}");
        }
        assert!(trained.head.probability(&[2.0, 0.0, 0.0]) > 0.9);
        assert!(trained.head.probability(&[-2.0, 0.0, 0.0]) < 0.1);
        assert!(trained.iterations > 0 && trained.iterations < Config::default().max_iterations);
        assert_eq!(trained.head.probability(&[1.0, 0.0]), 0.0, "another model's vector");
    }

    /// Two overlapping points on a line: weighting one class more moves the boundary
    /// towards the other.
    #[test]
    fn class_weights_shift_the_decision() {
        let data: Vec<(Vec<f32>, bool)> =
            vec![(vec![0.0], false), (vec![0.4], false), (vec![0.6], true), (vec![1.0], true), (vec![0.45], true), (vec![0.55], false)];
        let config = Config { l2: 1e-2, ..Config::default() };
        let boundary = |negative_weight: f32| {
            let ex: Vec<Example> = data
                .iter()
                .map(|(x, p)| Example { x, positive: *p, weight: if *p { 1.0 } else { negative_weight } })
                .collect();
            let head = train(&ex, None, &config).head;
            -head.bias / head.weights[0]
        };
        let even = boundary(1.0);
        let heavy = boundary(REJECTED_WEIGHT);
        assert!((even - 0.5).abs() < 0.05, "symmetric data, boundary in the middle: {even}");
        assert!(heavy > even + 0.05, "heavier negatives push the boundary towards the positives: {heavy} vs {even}");

        // balance() makes a rare class count as much as a common one.
        let mut ex: Vec<Example> = data.iter().map(|(x, p)| Example { x, positive: *p, weight: 1.0 }).collect();
        ex[0].weight = 5.0;
        balance(&mut ex);
        let total = |p: bool| ex.iter().filter(|e| e.positive == p).map(|e| e.weight).sum::<f32>();
        assert!((total(true) - total(false)).abs() < 1e-5);
        assert!((ex[0].weight / ex[1].weight - 5.0).abs() < 1e-5, "ratios within a class are kept");
    }

    #[test]
    fn warm_start_converges_in_fewer_steps() {
        let data = toy(200, 2);
        let config = Config::default();
        let first = train(&examples(&data[..180]), None, &config);
        let cold = train(&examples(&data), None, &config);
        let warm = train(&examples(&data), Some(&first.head), &config);
        assert!(warm.iterations < cold.iterations, "warm {} vs cold {}", warm.iterations, cold.iterations);
        // Both reach the same optimum (the problem is convex).
        for (a, b) in warm.head.weights.iter().zip(&cold.head.weights) {
            assert!((a - b).abs() < 1e-2, "{a} vs {b}");
        }
        // Restarting from the optimum on the same data takes (almost) no steps.
        assert!(train(&examples(&data), Some(&cold.head), &config).iterations <= 1);
    }

    #[test]
    fn deterministic_with_a_seed() {
        let data = toy(80, 3);
        let ex = examples(&data);
        let config = Config::default();
        assert_eq!(train(&ex, None, &config), train(&ex, None, &config));
        let a = cross_validate(&ex, FOLDS, 42, None, &config, &|| true).unwrap();
        let b = cross_validate(&ex, FOLDS, 42, None, &config, &|| true).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, Quality { precision: Some(1.0), recall: Some(1.0) }, "separable data");
        assert_eq!(seed_for("dream"), seed_for("dream"));
        assert_ne!(seed_for("dream"), seed_for("Dream"));
        let mut x: Vec<u32> = (0..20).collect();
        let mut y = x.clone();
        Rng(7).shuffle(&mut x);
        Rng(7).shuffle(&mut y);
        assert_eq!(x, y);
        assert_ne!(x, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn cross_validation_stops_when_asked() {
        let data = toy(40, 4);
        assert_eq!(cross_validate(&examples(&data), FOLDS, 1, None, &Config::default(), &|| false), None);
    }

    #[test]
    fn noisy_labels_lower_cross_validated_quality() {
        let mut data = toy(100, 5);
        // Flip a fifth of the labels.
        for (i, d) in data.iter_mut().enumerate() {
            if i % 5 == 0 {
                d.1 = !d.1;
            }
        }
        let q = cross_validate(&examples(&data), FOLDS, 1, None, &Config::default(), &|| true).unwrap();
        assert!(q.precision.unwrap() < 0.95 && q.recall.unwrap() < 0.95, "{q:?}");
        assert!(q.precision.unwrap() > 0.6 && q.recall.unwrap() > 0.6, "{q:?}");
    }
}
