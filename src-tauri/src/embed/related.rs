//! Related notes: the notes whose vectors are closest to an open note's. The vectors of
//! the ready model (`embedding_model_ready`) are kept in memory as `i8` (one byte per
//! dimension, with a scale per note) in one contiguous buffer, so a lookup is a
//! brute-force scan that never touches the vault; the best [`CANDIDATES`] are then
//! reranked with their exact `f32` vectors from the vault. The index lives in the
//! [`Session`] next to the vault, is dropped and zeroed on lock, and is kept in sync by
//! the embedding job (see [`update`] and [`sync`]).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use super::descriptor;
use crate::activity::{Job, count};
use crate::db::{NoteSummary, Reasons, StoredVector};
use crate::session::{Session, SessionError};

/// Vectors read per vault access while (re)loading the index.
const LOAD_PAGE: usize = 500;
/// Notes the `i8` scan hands to the exact rerank.
pub const CANDIDATES: usize = 100;
/// Fewest rows per scan thread; below this a thread costs more than it saves.
const MIN_ROWS_PER_THREAD: usize = 20_000;

/// The in-memory vectors of one unlock. Each unit vector `x` is stored as `q · scale`
/// with `q` in `i8` and `scale = max|x| / 127`, so the scan compares integers and
/// multiplies once per note.
pub struct RelatedIndex {
    /// The model the vectors come from; `None` while no model is ready.
    model: Option<String>,
    /// Length of each vector; 0 until known (a model without a descriptor takes the
    /// length of its first vector).
    dims: usize,
    ids: Vec<i64>,
    /// The hash of the text each vector was made from, to tell re-embedded notes.
    hashes: Zeroizing<Vec<[u8; 32]>>,
    /// Row of each note in `ids`, `hashes` and `vectors`.
    rows: HashMap<i64, usize>,
    /// `ids.len() * dims` quantised values, row after row.
    vectors: Zeroizing<Vec<i8>>,
    /// The scale of each row.
    scales: Zeroizing<Vec<f32>>,
}

impl RelatedIndex {
    fn new(model: Option<String>, capacity: usize) -> Self {
        let dims = model.as_deref().and_then(descriptor).map_or(0, |d| d.dims);
        Self {
            model,
            dims,
            ids: Vec::with_capacity(capacity),
            hashes: Zeroizing::new(Vec::with_capacity(capacity)),
            rows: HashMap::with_capacity(capacity),
            vectors: Zeroizing::new(Vec::with_capacity(capacity * dims)),
            scales: Zeroizing::new(Vec::with_capacity(capacity)),
        }
    }

    /// The model the index serves, if one is ready.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// How many notes have a vector in the index.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Roughly the memory the index holds, in bytes: its buffers' capacity and the
    /// row map's entries.
    pub fn memory_bytes(&self) -> usize {
        self.vectors.capacity()
            + self.hashes.capacity() * 32
            + self.scales.capacity() * size_of::<f32>()
            + self.ids.capacity() * size_of::<i64>()
            // Key, value and about one control byte per bucket.
            + self.rows.capacity() * (size_of::<i64>() + size_of::<usize>() + 1)
    }

    pub fn contains(&self, id: i64) -> bool {
        self.rows.contains_key(&id)
    }

    fn hash(&self, id: i64) -> Option<&[u8; 32]> {
        self.rows.get(&id).map(|&row| &self.hashes[row])
    }

    /// Adds or replaces a note's vector. A vector of the wrong length is ignored.
    fn upsert(&mut self, id: i64, hash: [u8; 32], vector: &[f32]) {
        if self.dims == 0 && self.ids.is_empty() {
            self.dims = vector.len();
        }
        if vector.len() != self.dims || self.dims == 0 {
            return;
        }
        let row = match self.rows.get(&id) {
            Some(&row) => row,
            None => {
                reserve(&mut self.vectors, self.dims);
                reserve(&mut self.hashes, 1);
                reserve(&mut self.scales, 1);
                let len = self.vectors.len() + self.dims;
                self.vectors.resize(len, 0);
                self.hashes.push([0; 32]);
                self.scales.push(0.0);
                self.ids.push(id);
                self.rows.insert(id, self.ids.len() - 1);
                self.ids.len() - 1
            }
        };
        self.hashes[row] = hash;
        let d = self.dims;
        let mut unit = Zeroizing::new(vector.to_vec());
        super::normalize(&mut unit);
        self.scales[row] = quantize(&unit, &mut self.vectors[row * d..(row + 1) * d]);
    }

    /// Removes a note's vector: the last row moves into its place.
    fn remove(&mut self, id: i64) {
        let Some(row) = self.rows.remove(&id) else { return };
        let last = self.ids.len() - 1;
        let d = self.dims;
        if row != last {
            self.vectors.copy_within(last * d..(last + 1) * d, row * d);
            self.hashes[row] = self.hashes[last];
            self.scales[row] = self.scales[last];
            self.ids[row] = self.ids[last];
            self.rows.insert(self.ids[row], row);
        }
        // Zeroed before truncating: the spare capacity is only wiped when the buffer is dropped.
        self.vectors[last * d..].zeroize();
        self.hashes[last].zeroize();
        self.scales[last].zeroize();
        self.vectors.truncate(last * d);
        self.hashes.truncate(last);
        self.scales.truncate(last);
        self.ids.truncate(last);
    }

    /// `score` applied to every vector in the index, as `(note id, score)` in no
    /// particular order. The vectors are unit length, as stored note vectors are, give
    /// or take the `i8` rounding (under 0.4% of the largest component).
    pub fn score_all(&self, score: impl Fn(&[f32]) -> f32) -> Vec<(i64, f32)> {
        if self.dims == 0 {
            return Vec::new();
        }
        let mut row = Zeroizing::new(vec![0.0f32; self.dims]);
        self.vectors
            .chunks_exact(self.dims)
            .zip(self.scales.iter())
            .zip(&self.ids)
            .map(|((q, &scale), id)| {
                row.iter_mut().zip(q).for_each(|(x, &q)| *x = f32::from(q) * scale);
                (*id, score(&row))
            })
            .collect()
    }

    /// Up to `k` notes most similar to `id` by the `i8` vectors, most similar first,
    /// `id` itself left out: candidates for [`rerank`]. `None` when `id` has no vector.
    pub fn nearest(&self, id: i64, k: usize) -> Option<Vec<i64>> {
        let &row = self.rows.get(&id)?;
        let d = self.dims;
        let query = &self.vectors[row * d..(row + 1) * d];
        Some(self.best(query, k, |i| i != row))
    }

    /// Up to `k` notes closest to `query` (any length-`dims` vector, such as a search
    /// query's) by the `i8` vectors, most similar first: candidates for [`rerank`].
    /// With `allowed`, only those notes are scored, so a filtered search finds its best
    /// matches even when none of them are among the vault's best.
    pub fn search(&self, query: &[f32], k: usize, allowed: Option<&Allowed>) -> Vec<i64> {
        if query.len() != self.dims || self.dims == 0 {
            return Vec::new();
        }
        let mut unit = Zeroizing::new(query.to_vec());
        super::normalize(&mut unit);
        let mut q = Zeroizing::new(vec![0i8; self.dims]);
        // The query's own scale is the same for every row, so it doesn't change the order.
        quantize(&unit, &mut q);
        match allowed {
            Some(allowed) => self.best(&q, k, |i| allowed.contains(self.ids[i])),
            None => self.best(&q, k, |_| true),
        }
    }

    /// The ids of the best `k` rows for `query` (`i8`, as stored) among the rows `keep`
    /// accepts, best first; ties by row, so the order is stable. Large indexes are
    /// scanned on several threads.
    fn best(&self, query: &[i8], k: usize, keep: impl Fn(usize) -> bool + Sync) -> Vec<i64> {
        let d = self.dims;
        let n = self.ids.len();
        if k == 0 || n == 0 || d == 0 {
            return Vec::new();
        }
        // The best `k` of rows `start..`, in a bounded heap: the worst kept on top.
        let scan = |start: usize, rows: &[i8]| -> Vec<(Score, Reverse<usize>)> {
            let mut heap: BinaryHeap<Reverse<(Score, Reverse<usize>)>> = BinaryHeap::with_capacity(k + 1);
            for (i, v) in rows.chunks_exact(d).enumerate() {
                let i = start + i;
                if !keep(i) {
                    continue;
                }
                let entry = (Score(idot(query, v) as f32 * self.scales[i]), Reverse(i));
                if heap.len() < k {
                    heap.push(Reverse(entry));
                } else if heap.peek().is_some_and(|worst| entry > worst.0) {
                    heap.pop();
                    heap.push(Reverse(entry));
                }
            }
            heap.into_iter().map(|r| r.0).collect()
        };
        let threads = std::thread::available_parallelism()
            .map_or(1, |c| (c.get() / 2).max(1))
            .min(n.div_ceil(MIN_ROWS_PER_THREAD));
        let mut best: Vec<(Score, Reverse<usize>)> = if threads <= 1 {
            scan(0, &self.vectors)
        } else {
            let per = n.div_ceil(threads);
            let scan = &scan;
            std::thread::scope(|s| {
                let handles: Vec<_> = self
                    .vectors
                    .chunks(per * d)
                    .enumerate()
                    .map(|(t, rows)| s.spawn(move || scan(t * per, rows)))
                    .collect();
                handles.into_iter().flat_map(|h| h.join().expect("related-notes scan panicked")).collect()
            })
        };
        // Highest score first; ties by row, so the order is stable.
        best.sort_unstable_by(|a, b| b.cmp(a));
        best.truncate(k);
        best.into_iter().map(|(_, Reverse(i))| self.ids[i]).collect()
    }
}

/// The notes a filtered search may return: a bitset over note ids, so the index scan
/// checks a note with one lookup. Either the notes listed, or every note but them.
pub struct Allowed {
    bits: Vec<u64>,
    except: bool,
}

impl Allowed {
    /// Only the notes `ids`.
    pub fn new(ids: &[i64]) -> Self {
        Self { bits: bitset(ids), except: false }
    }

    /// Every note but `ids`.
    pub fn except(ids: &[i64]) -> Self {
        Self { bits: bitset(ids), except: true }
    }

    pub fn contains(&self, id: i64) -> bool {
        let listed = id >= 0 && self.bits.get(id as usize / 64).is_some_and(|w| w & (1 << (id as usize % 64)) != 0);
        listed != self.except
    }
}

fn bitset(ids: &[i64]) -> Vec<u64> {
    let max = ids.iter().copied().filter(|&id| id >= 0).max().unwrap_or(0) as usize;
    let mut bits = vec![0u64; max / 64 + 1];
    for &id in ids.iter().filter(|&&id| id >= 0) {
        bits[id as usize / 64] |= 1 << (id as usize % 64);
    }
    bits
}

/// A score that orders like a number, so `(Score, Reverse(row))` compares higher for a
/// better score and, at equal score, for a lower row.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Score(f32);

impl Eq for Score {}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Writes `x` as `i8`s into `q` and returns the scale (`max|x| / 127`; 0 for a zero
/// vector).
fn quantize(x: &[f32], q: &mut [i8]) -> f32 {
    let max = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if max == 0.0 || !max.is_finite() {
        q.fill(0);
        return 0.0;
    }
    let scale = max / 127.0;
    for (q, v) in q.iter_mut().zip(x) {
        *q = (v / scale).round().clamp(-127.0, 127.0) as i8;
    }
    scale
}

/// Integer dot product in sixteen running sums, so the compiler can use SIMD.
fn idot(a: &[i8], b: &[i8]) -> i32 {
    let mut sums = [0i32; 16];
    let (a16, b16) = (a.chunks_exact(16), b.chunks_exact(16));
    let tail: i32 = a16.remainder().iter().zip(b16.remainder()).map(|(&x, &y)| i32::from(x) * i32::from(y)).sum();
    for (x, y) in a16.zip(b16) {
        for i in 0..16 {
            sums[i] += i32::from(x[i]) * i32::from(y[i]);
        }
    }
    sums.iter().sum::<i32>() + tail
}

pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
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

/// `candidates` (most similar first by the `i8` scan) reordered by the exact cosine of
/// their `f32` vectors to `query`, the first `k`, with that cosine. Candidates without
/// a vector in `vectors` (trashed or deleted since the index was synced) are left out;
/// ties keep the scan's order.
pub fn rerank(query: &[f32], candidates: &[i64], vectors: &[StoredVector], k: usize) -> Vec<(i64, f32)> {
    let mut q = Zeroizing::new(query.to_vec());
    super::normalize(&mut q);
    let mut scored: Vec<(f32, usize, i64)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(pos, &id)| {
            let v = vectors.iter().find(|v| v.id == id)?;
            if v.vector.len() != q.len() {
                return None;
            }
            let mut unit = Zeroizing::new(v.vector.to_vec());
            super::normalize(&mut unit);
            Some((dot(&q, &unit), pos, id))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(k).map(|(score, _, id)| (id, score)).collect()
}

/// Makes room for `additional` more elements without letting `Vec` reallocate, which
/// would free the old buffer without zeroing it: the contents move to a new buffer and
/// the old one is zeroed as it drops.
fn reserve<T: Zeroize + Copy>(v: &mut Zeroizing<Vec<T>>, additional: usize) {
    if v.capacity() - v.len() >= additional {
        return;
    }
    let mut grown = Vec::with_capacity((v.capacity() * 2).max(v.len() + additional).max(64));
    grown.extend_from_slice(v);
    // The old buffer is a `Zeroizing`, so it is wiped when it drops here.
    *v = Zeroizing::new(grown);
}

/// Brings the index of unlock `generation` up to date with the vault: rebuilds it when
/// the ready model changed (or on the first call after unlock), otherwise loads new and
/// re-embedded vectors and drops those of notes that lost theirs (trashed, deleted,
/// emptied). The vault is only held to read; never while the index is written.
/// Returns whether the index changed.
///
/// The embedding job only calls this to build the index; after that it hands
/// [`update`] the notes each pass changed. The full comparison here is for callers
/// that don't know what changed.
pub fn sync(session: &Session, generation: u64) -> Result<bool, SessionError> {
    let ready = session.with_vault_quiet(generation, |v| v.embedding_model_ready())?;
    let built = session.related_index().as_ref().map(|index| index.model.clone());
    if built.as_ref() != Some(&ready) {
        return rebuild(session, generation, ready);
    }
    let Some(model) = ready else { return Ok(false) };
    let live = session.with_vault_quiet(generation, |v| v.embedding_hashes(&model))?;
    let (remove, load) = {
        let index = session.related_index();
        let Some(index) = index.as_ref() else { return Ok(false) };
        let live_ids: HashSet<i64> = live.iter().map(|(id, _)| *id).collect();
        let remove: Vec<i64> = index.ids.iter().copied().filter(|id| !live_ids.contains(id)).collect();
        let load: Vec<i64> = live.iter().filter(|(id, hash)| index.hash(*id) != Some(hash)).map(|(id, _)| *id).collect();
        (remove, load)
    };
    if remove.is_empty() && load.is_empty() {
        return Ok(false);
    }
    let activity = session.activity().reporter(generation);
    activity.start(Job::Related, format!("loading {} vectors, dropping {}", count(load.len() as u64), count(remove.len() as u64)));
    let mut loaded: Vec<StoredVector> = Vec::with_capacity(load.len());
    for page in load.chunks(LOAD_PAGE) {
        loaded.extend(session.with_vault_quiet(generation, |v| v.embedding_vectors(&model, page))?);
    }
    let size = session
        .update_related_index(generation, |index| {
            let Some(index) = index.as_mut() else { return 0 };
            for &id in &remove {
                index.remove(id);
            }
            for v in &loaded {
                index.upsert(v.id, v.hash, &v.vector);
            }
            index.len()
        })
        .ok_or(SessionError::Locked)?;
    activity.finish_quietly(
        Job::Related,
        format!("updated {} notes, removed {}; {} in the index", count(loaded.len() as u64), count(remove.len() as u64), count(size as u64)),
    );
    Ok(true)
}

/// Builds the index for `ready` from scratch: the stored vectors are read front to back
/// in pages and quantised into a new index as they arrive, so no copy of every `f32`
/// vector is held; the old index keeps serving until the new one replaces it.
fn rebuild(session: &Session, generation: u64, ready: Option<String>) -> Result<bool, SessionError> {
    let activity = session.activity().reporter(generation);
    let (mut fresh, total) = match &ready {
        Some(model) => {
            activity.start(Job::Related, format!("rebuilding index for {model}"));
            let total = session.with_vault_quiet(generation, |v| v.embedding_count(model))?;
            (RelatedIndex::new(ready.clone(), total), total)
        }
        None => (RelatedIndex::new(None, 0), 0),
    };
    if let Some(model) = &ready {
        let mut after = 0;
        let mut loaded = 0;
        loop {
            let (page, next) =
                session.with_vault_quiet(generation, |v| v.embedding_vector_page(model, after, LOAD_PAGE))?;
            loaded += page.len();
            for v in &page {
                fresh.upsert(v.id, v.hash, &v.vector);
            }
            activity.progress(
                Job::Related,
                format!("rebuilding index: {} / {} vectors", count(loaded as u64), count(total.max(loaded) as u64)),
                loaded as u64,
                Some(total.max(loaded) as u64),
            );
            match next {
                Some(n) => after = n,
                None => break,
            }
        }
    }
    let (size, bytes) = (fresh.len(), fresh.memory_bytes());
    session.update_related_index(generation, |index| *index = Some(fresh)).ok_or(SessionError::Locked)?;
    match &ready {
        Some(model) => activity.finish(
            Job::Related,
            format!("rebuilt index: {} vectors of {model}, {:.1} MB", count(size as u64), bytes as f64 / 1_048_576.0),
        ),
        None => activity.idle(Job::Related, Some("no model is ready yet")),
    }
    Ok(true)
}

/// Brings the index up to date for the notes in `ids` only (the ones the job just
/// embedded, emptied, saw trashed or deleted): loads their vectors, or drops them if
/// they have none. Falls back to [`sync`] when the index isn't built for the ready
/// model yet. Returns whether the index changed.
pub fn update(session: &Session, generation: u64, ids: &[i64]) -> Result<bool, SessionError> {
    let ready = session.with_vault_quiet(generation, |v| v.embedding_model_ready())?;
    let built = session.related_index().as_ref().map(|index| index.model.clone());
    if built.as_ref() != Some(&ready) {
        return sync(session, generation);
    }
    let Some(model) = ready else { return Ok(false) };
    if ids.is_empty() {
        return Ok(false);
    }
    let mut loaded: Vec<StoredVector> = Vec::with_capacity(ids.len());
    for page in ids.chunks(LOAD_PAGE) {
        loaded.extend(session.with_vault_quiet(generation, |v| v.embedding_vectors(&model, page))?);
    }
    let found: HashSet<i64> = loaded.iter().map(|v| v.id).collect();
    let size = session
        .update_related_index(generation, |index| {
            let Some(index) = index.as_mut() else { return 0 };
            for id in ids.iter().filter(|id| !found.contains(id)) {
                index.remove(*id);
            }
            for v in &loaded {
                index.upsert(v.id, v.hash, &v.vector);
            }
            index.len()
        })
        .ok_or(SessionError::Locked)?;
    let summary = format!(
        "updated {} notes, removed {}; {} in the index",
        count(found.len() as u64),
        count((ids.len() - found.len()) as u64),
        count(size as u64)
    );
    // A big pass is worth a log entry; the one after an autosave isn't.
    let activity = session.activity().reporter(generation);
    if ids.len() >= 10 {
        activity.finish(Job::Related, summary);
    } else {
        activity.finish_quietly(Job::Related, summary);
    }
    Ok(true)
}

/// A related note as the Related panel shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedNote {
    pub note: NoteSummary,
    /// Exact cosine similarity of the two notes' stored vectors.
    pub score: f32,
    pub reasons: Reasons,
}

/// Up to `limit` live notes most similar to note `id`, most similar first, with their
/// score and the other reasons they relate. `None` when the note has no vector yet
/// (empty, not indexed yet, or no model is ready).
///
/// The `i8` scan holds only the index and picks [`CANDIDATES`] (or more for a long
/// list); the vault is taken afterwards, to rerank them by their exact vectors, look
/// up the summaries and work out the reasons for the ones kept. It does not count as
/// activity: the frontend also asks after a background re-embed. Notes trashed or
/// deleted since the job last synced the index are skipped.
pub fn related_notes(session: &Session, id: i64, limit: usize) -> Result<Option<Vec<RelatedNote>>, SessionError> {
    let generation = session.generation();
    if generation == 0 {
        return Err(SessionError::Locked);
    }
    let mut k = limit.max(CANDIDATES);
    loop {
        let found = {
            let index = session.related_index();
            index.as_ref().and_then(|index| Some((index.model.clone()?, index.nearest(id, k)?)))
        };
        let Some((model, candidates)) = found else { return Ok(None) };
        let exhausted = candidates.len() < k;
        let related = session.with_vault_quiet(generation, |v| {
            let mut wanted = Vec::with_capacity(candidates.len() + 1);
            wanted.push(id);
            wanted.extend_from_slice(&candidates);
            let vectors = v.embedding_vectors(&model, &wanted)?;
            let Some(query) = vectors.iter().find(|v| v.id == id) else { return Ok(None) };
            let ranked = rerank(&query.vector, &candidates, &vectors, limit);
            let ids: Vec<i64> = ranked.iter().map(|(id, _)| *id).collect();
            let notes = v.note_summaries(&ids)?;
            if notes.len() < limit && !exhausted {
                return Ok(Some(None));
            }
            let reasons = v.relation_reasons(id, &notes.iter().map(|n| n.id).collect::<Vec<_>>())?;
            let scores: HashMap<i64, f32> = ranked.into_iter().collect();
            Ok(Some(Some(
                notes
                    .into_iter()
                    .zip(reasons)
                    .map(|(note, reasons)| RelatedNote { score: scores[&note.id], note, reasons })
                    .collect::<Vec<_>>(),
            )))
        })?;
        match related {
            None => return Ok(None),
            Some(Some(notes)) => return Ok(Some(notes)),
            // Some candidates were trashed since the index was synced: look further.
            Some(None) => k *= 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(vectors: &[(i64, [f32; 3])]) -> RelatedIndex {
        let mut index = RelatedIndex::new(Some("test".into()), 0);
        for (id, v) in vectors {
            index.upsert(*id, [*id as u8; 32], v);
        }
        index
    }

    #[test]
    fn a_query_vector_finds_the_closest_notes_among_those_allowed() {
        let index = index(&[(1, [1.0, 0.0, 0.0]), (2, [0.9, 0.1, 0.0]), (3, [0.0, 1.0, 0.0]), (70, [0.5, 0.5, 0.0])]);
        assert_eq!(index.search(&[2.0, 0.0, 0.0], 3, None), vec![1, 2, 70], "any length: it is normalised");
        let allowed = Allowed::new(&[3, 70, 999]);
        assert!(allowed.contains(70) && !allowed.contains(1) && !allowed.contains(-1) && !allowed.contains(5000));
        assert_eq!(index.search(&[1.0, 0.0, 0.0], 3, Some(&allowed)), vec![70, 3]);
        let except = Allowed::except(&[1, 70]);
        assert!(!except.contains(70) && except.contains(2) && except.contains(5000));
        assert_eq!(index.search(&[1.0, 0.0, 0.0], 3, Some(&except)), vec![2, 3]);
        assert_eq!(index.search(&[1.0, 0.0], 3, None), Vec::<i64>::new(), "wrong length");
        assert_eq!(index.search(&[1.0, 0.0, 0.0], 3, Some(&Allowed::new(&[]))), Vec::<i64>::new());
    }

    #[test]
    fn removing_a_row_keeps_the_others_intact() {
        // 100 rows, so the buffers grow several times on the way.
        let mut rows: Vec<(i64, [f32; 3])> = (1..=100).map(|i| (i, [i as f32, 1.0, 0.0])).collect();
        let mut index = index(&rows);
        for id in [1, 50, 100, 2] {
            index.remove(id);
            rows.retain(|(r, _)| *r != id);
        }
        index.remove(999);
        assert_eq!(index.len(), rows.len());
        assert_eq!(index.vectors.len(), rows.len() * 3);
        assert_eq!(index.scales.len(), rows.len());
        for (id, v) in &rows {
            let row = index.rows[id];
            let mut expected = v.to_vec();
            crate::embed::normalize(&mut expected);
            let stored: Vec<f32> = index.vectors[row * 3..row * 3 + 3].iter().map(|&q| f32::from(q) * index.scales[row]).collect();
            for (s, e) in stored.iter().zip(&expected) {
                assert!((s - e).abs() < 0.005, "{stored:?} vs {expected:?}");
            }
            assert_eq!(index.hash(*id), Some(&[*id as u8; 32]));
        }
    }

    #[test]
    fn upsert_replaces_and_skips_vectors_of_the_wrong_length() {
        let mut index = index(&[(1, [1.0, 0.0, 0.0]), (2, [0.0, 1.0, 0.0]), (3, [0.0, 0.0, 1.0])]);
        assert_eq!(index.nearest(1, 1), Some(vec![2]), "a tie goes to the earlier row");
        index.upsert(3, [0; 32], &[1.0, 0.1, 0.0]);
        index.upsert(4, [0; 32], &[1.0, 0.0]);
        assert_eq!(index.len(), 3);
        assert_eq!(index.nearest(1, 5), Some(vec![3, 2]));
        assert_eq!(index.nearest(4, 5), None);
    }

    /// Pseudo-random unit vectors (xorshift), `n` of `dims`.
    fn random_vectors(n: usize, dims: usize, mut seed: u64) -> Vec<Vec<f32>> {
        (0..n)
            .map(|_| {
                let mut v: Vec<f32> = (0..dims)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        (seed % 20_000) as f32 / 10_000.0 - 1.0
                    })
                    .collect();
                crate::embed::normalize(&mut v);
                v
            })
            .collect()
    }

    #[test]
    fn quantised_scan_and_rerank_match_exact_cosine() {
        // Half the vectors near one of 20 centres, so many neighbours are close calls.
        const N: usize = 10_000;
        const DIMS: usize = 384;
        let centres = random_vectors(20, DIMS, 7);
        let noise = random_vectors(N, DIMS, 11);
        let vectors: Vec<Vec<f32>> = noise
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut v: Vec<f32> = if i % 2 == 0 {
                    centres[i % 20].iter().zip(n).map(|(c, x)| c + 0.6 * x).collect()
                } else {
                    n.clone()
                };
                crate::embed::normalize(&mut v);
                v
            })
            .collect();
        let mut index = RelatedIndex::new(Some("test".into()), N);
        for (i, v) in vectors.iter().enumerate() {
            index.upsert(i as i64, [0; 32], v);
        }
        let stored: Vec<StoredVector> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| StoredVector { id: i as i64, hash: [0; 32], vector: Zeroizing::new(v.clone()) })
            .collect();
        for query in (0..N).step_by(N / 40) {
            let exact: Vec<(f32, i64)> = {
                let mut all: Vec<(f32, i64)> = (0..N)
                    .filter(|&i| i != query)
                    .map(|i| (dot(&vectors[query], &vectors[i]), i as i64))
                    .collect();
                all.sort_by(|a, b| b.0.total_cmp(&a.0));
                all
            };
            let candidates = index.nearest(query as i64, CANDIDATES).unwrap();
            assert_eq!(candidates.len(), CANDIDATES);
            let found: Vec<i64> = rerank(&vectors[query], &candidates, &stored, 10).into_iter().map(|(id, _)| id).collect();
            // The same ten, in the same order; a different note only where it ties
            // (within float rounding) with the exact one at that place.
            for (place, (&got, &(score, want))) in found.iter().zip(&exact[..10]).enumerate() {
                if got != want {
                    let got_score = dot(&vectors[query], &vectors[got as usize]);
                    assert!((got_score - score).abs() < 1e-6, "query {query}, place {place}: {got} instead of {want}");
                }
            }
        }
    }

    #[test]
    fn the_scan_splits_over_threads_without_changing_the_result() {
        // Enough rows for several scan threads (where the machine has the cores).
        let n = MIN_ROWS_PER_THREAD * 3 + 17;
        let vectors = random_vectors(n, 16, 3);
        let mut index = RelatedIndex::new(Some("test".into()), n);
        for (i, v) in vectors.iter().enumerate() {
            index.upsert(i as i64, [0; 32], v);
        }
        let threaded = index.nearest(5, 50).unwrap();
        // The same ranking as one pass over every row by the same i8 scores.
        let q = &index.vectors[5 * 16..6 * 16];
        let mut all: Vec<(f32, usize)> = (0..n)
            .filter(|&i| i != 5)
            .map(|i| (idot(q, &index.vectors[i * 16..(i + 1) * 16]) as f32 * index.scales[i], i))
            .collect();
        all.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let single: Vec<i64> = all[..50].iter().map(|&(_, i)| i as i64).collect();
        assert_eq!(threaded, single);
    }

    #[test]
    fn quantising_keeps_zero_vectors_and_signs() {
        let mut q = [0i8; 4];
        assert_eq!(quantize(&[0.0; 4], &mut q), 0.0);
        assert_eq!(q, [0; 4]);
        let scale = quantize(&[0.5, -0.5, 0.25, 0.0], &mut q);
        assert_eq!(q, [127, -127, 64, 0]);
        assert!((scale - 0.5 / 127.0).abs() < 1e-9);
        assert_eq!(idot(&[127; 20], &[-1; 20]), -127 * 20);
    }
}
