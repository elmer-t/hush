//! Spherical k-means: k-means on unit vectors with cosine similarity, seeded with
//! k-means++ from a fixed seed, so the same vectors always give the same groups.
//! Plain `std`, no dependencies.

/// Most rounds of assigning and re-centring; it usually settles well before this.
pub const MAX_ROUNDS: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct Clusters {
    /// The group of each point.
    pub assignments: Vec<usize>,
    /// Unit-length centre of each group, `k * dims` floats.
    pub centroids: Vec<f32>,
    pub rounds: usize,
}

/// Groups `n` unit vectors of `dims` floats (`points` holds them back to back) into
/// `k` groups (at most `n`). `stop` is checked between rounds and while seeding;
/// returns `None` once it says so.
pub fn kmeans(points: &[f32], dims: usize, k: usize, seed: u64, stop: &dyn Fn() -> bool) -> Option<Clusters> {
    let n = if dims == 0 { 0 } else { points.len() / dims };
    let k = k.min(n);
    if k == 0 {
        return Some(Clusters { assignments: vec![0; n], centroids: Vec::new(), rounds: 0 });
    }
    let point = |i: usize| &points[i * dims..(i + 1) * dims];
    let mut rng = SplitMix64(seed);
    let mut centroids = Vec::with_capacity(k * dims);

    // Greedy k-means++ (as scikit-learn does it): each next centre is drawn with
    // probability proportional to the squared distance from the nearest centre so far
    // (2 - 2·cos on unit vectors); of 2 + ln(k) such draws, the one that brings the
    // points closest to their centres wins. Plain k-means++ too often puts two seeds
    // in one group and leaves two others sharing one.
    let trials = 2 + (k as f64).ln() as usize;
    let first = rng.below(n);
    centroids.extend_from_slice(point(first));
    let mut nearest: Vec<f32> = (0..n).map(|i| distance(point(i), point(first))).collect();
    while centroids.len() < k * dims {
        if stop() {
            return None;
        }
        let total: f64 = nearest.iter().map(|&d| f64::from(d)).sum();
        let mut best: Option<(f64, usize, Vec<f32>)> = None;
        for _ in 0..trials {
            let pick = if total > 0.0 {
                let mut target = rng.unit() * total;
                let mut pick = n - 1;
                for (i, &d) in nearest.iter().enumerate() {
                    target -= f64::from(d);
                    if target < 0.0 && d > 0.0 {
                        pick = i;
                        break;
                    }
                }
                pick
            } else {
                // Every point sits on a centre already (duplicates): any point will do.
                rng.below(n)
            };
            let candidate: Vec<f32> = (0..n).map(|i| nearest[i].min(distance(point(i), point(pick)))).collect();
            let cost: f64 = candidate.iter().map(|&d| f64::from(d)).sum();
            if best.as_ref().is_none_or(|b| cost < b.0) {
                best = Some((cost, pick, candidate));
            }
        }
        let (_, pick, candidate) = best.expect("at least one trial");
        centroids.extend_from_slice(point(pick));
        nearest = candidate;
    }

    let mut assignments = vec![usize::MAX; n];
    let mut rounds = 0;
    while rounds < MAX_ROUNDS {
        if stop() {
            return None;
        }
        rounds += 1;
        let (next, best) = assign(points, dims, &centroids);
        let changed = next != assignments;
        assignments = next;
        if !changed {
            break;
        }
        centroids = recentre(points, dims, k, &mut assignments, &best);
    }
    Some(Clusters { assignments, centroids, rounds })
}

/// Squared Euclidean distance between unit vectors.
fn distance(a: &[f32], b: &[f32]) -> f32 {
    (2.0 - 2.0 * dot(a, b)).max(0.0)
}

/// Dot product, in eight running sums so the compiler can use SIMD (a single sum must
/// be added in order, which keeps it scalar). The hot loop of the whole run.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut sums = [0.0f32; 8];
    let (a8, b8) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = a8.remainder().iter().zip(b8.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in a8.zip(b8) {
        for i in 0..8 {
            sums[i] += x[i] * y[i];
        }
    }
    sums.iter().sum::<f32>() + tail
}

/// The nearest centre of every point, and its similarity to it. Split over half the
/// cores (like the embedding model), so the UI keeps its share; each point is
/// independent, so the result doesn't depend on the split.
pub fn assign(points: &[f32], dims: usize, centroids: &[f32]) -> (Vec<usize>, Vec<f32>) {
    let n = points.len() / dims;
    let nearest = |chunk: &[f32]| -> Vec<(usize, f32)> {
        chunk
            .chunks_exact(dims)
            .map(|p| {
                centroids.chunks_exact(dims).enumerate().fold((0, f32::MIN), |best, (c, centre)| {
                    let s = dot(p, centre);
                    if s > best.1 { (c, s) } else { best }
                })
            })
            .collect()
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
    let per = n.div_ceil(threads).max(256);
    let results: Vec<(usize, f32)> = if n <= per {
        nearest(points)
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = points.chunks(per * dims).map(|chunk| s.spawn(move || nearest(chunk))).collect();
            handles.into_iter().flat_map(|h| h.join().expect("k-means worker panicked")).collect()
        })
    };
    results.into_iter().unzip()
}

/// New centres: the normalised mean of each group's points. A group left empty takes
/// the point farthest from its own centre (from a group that has more than one point),
/// so every group keeps at least one note.
fn recentre(points: &[f32], dims: usize, k: usize, assignments: &mut [usize], best: &[f32]) -> Vec<f32> {
    let mut sizes = vec![0usize; k];
    for &a in assignments.iter() {
        sizes[a] += 1;
    }
    let mut taken = vec![false; assignments.len()];
    for c in 0..k {
        if sizes[c] > 0 {
            continue;
        }
        let far = (0..assignments.len())
            .filter(|&i| !taken[i] && sizes[assignments[i]] > 1)
            .min_by(|&a, &b| best[a].total_cmp(&best[b]).then(a.cmp(&b)));
        if let Some(i) = far {
            sizes[assignments[i]] -= 1;
            sizes[c] = 1;
            assignments[i] = c;
            taken[i] = true;
        }
    }
    let mut centroids = vec![0.0f32; k * dims];
    for (i, &a) in assignments.iter().enumerate() {
        let centre = &mut centroids[a * dims..(a + 1) * dims];
        centre.iter_mut().zip(&points[i * dims..(i + 1) * dims]).for_each(|(c, x)| *c += x);
    }
    centroids.chunks_exact_mut(dims).for_each(crate::embed::normalize);
    centroids
}

/// A small, fast, seedable generator (SplitMix64); plenty for picking seeds.
pub(crate) struct SplitMix64(pub(crate) u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `per` noisy points around each of `centres` unit vectors in `dims` dimensions.
    fn blobs(centres: usize, per: usize, dims: usize, noise: f32, seed: u64) -> (Vec<f32>, Vec<usize>) {
        let mut rng = SplitMix64(seed);
        let mut points = Vec::new();
        let mut truth = Vec::new();
        let centre_vectors: Vec<Vec<f32>> = (0..centres)
            .map(|_| {
                let mut v: Vec<f32> = (0..dims).map(|_| rng.unit() as f32 - 0.5).collect();
                crate::embed::normalize(&mut v);
                v
            })
            .collect();
        for i in 0..centres * per {
            let c = i % centres;
            let mut v: Vec<f32> =
                centre_vectors[c].iter().map(|x| x + noise * (rng.unit() as f32 - 0.5)).collect();
            crate::embed::normalize(&mut v);
            points.extend(v);
            truth.push(c);
        }
        (points, truth)
    }

    /// Whether `found` groups points exactly as `truth` does (group numbers may differ).
    fn same_partition(found: &[usize], truth: &[usize]) -> bool {
        (0..found.len()).all(|i| (0..found.len()).all(|j| (found[i] == found[j]) == (truth[i] == truth[j])))
    }

    #[test]
    fn separates_well_separated_clusters() {
        let (points, truth) = blobs(5, 40, 32, 0.15, 7);
        for seed in 0..20 {
            let c = kmeans(&points, 32, 5, seed, &|| false).unwrap();
            assert!(same_partition(&c.assignments, &truth), "seed {seed}");
            assert!(c.rounds < MAX_ROUNDS);
            // Centres are unit length.
            for centre in c.centroids.chunks_exact(32) {
                assert!((dot(centre, centre) - 1.0).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn same_seed_same_groups() {
        // Overlapping clusters, so different seeds could well disagree.
        let (points, _) = blobs(8, 50, 16, 1.5, 3);
        let a = kmeans(&points, 16, 8, 99, &|| false).unwrap();
        let b = kmeans(&points, 16, 8, 99, &|| false).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn every_group_gets_a_point_even_with_duplicates() {
        // Three distinct points, each repeated: asking for 5 groups still fills 5.
        let mut points = Vec::new();
        for i in 0..12 {
            let mut v = vec![0.0f32; 4];
            v[i % 3] = 1.0;
            points.extend(v);
        }
        let c = kmeans(&points, 4, 5, 1, &|| false).unwrap();
        for g in 0..5 {
            assert!(c.assignments.contains(&g), "group {g} is empty: {:?}", c.assignments);
        }
    }

    #[test]
    fn k_is_capped_at_the_number_of_points_and_stop_is_honoured() {
        let (points, _) = blobs(2, 2, 8, 0.1, 1);
        let c = kmeans(&points, 8, 10, 1, &|| false).unwrap();
        assert_eq!(c.centroids.len(), 4 * 8);
        assert!(kmeans(&points, 8, 2, 1, &|| true).is_none());
        assert_eq!(kmeans(&[], 8, 3, 1, &|| false).unwrap().assignments, Vec::<usize>::new());
    }
}
