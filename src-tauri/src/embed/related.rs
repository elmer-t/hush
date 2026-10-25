//! Related notes: the notes whose vectors are closest to an open note's. The vectors of
//! the ready model (`embedding_model_ready`) are kept in memory in one contiguous
//! buffer, so a lookup is a brute-force scan that never touches the vault. The index
//! lives in the [`Session`] next to the vault, is dropped and zeroed on lock, and is
//! kept in sync by the embedding job (see [`sync`]).

use std::collections::{HashMap, HashSet};

use zeroize::{Zeroize, Zeroizing};

use super::descriptor;
use crate::db::{NoteSummary, StoredVector};
use crate::session::{Session, SessionError};

/// Vectors read per vault access while (re)loading the index.
const LOAD_PAGE: usize = 500;

/// The in-memory vectors of one unlock. Vectors are stored unit-length, so the dot
/// product is the cosine similarity.
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
    /// `ids.len() * dims` floats, row after row.
    vectors: Zeroizing<Vec<f32>>,
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
                let len = self.vectors.len() + self.dims;
                self.vectors.resize(len, 0.0);
                self.hashes.push([0; 32]);
                self.ids.push(id);
                self.rows.insert(id, self.ids.len() - 1);
                self.ids.len() - 1
            }
        };
        self.hashes[row] = hash;
        let d = self.dims;
        let slot = &mut self.vectors[row * d..(row + 1) * d];
        slot.copy_from_slice(vector);
        super::normalize(slot);
    }

    /// Removes a note's vector: the last row moves into its place.
    fn remove(&mut self, id: i64) {
        let Some(row) = self.rows.remove(&id) else { return };
        let last = self.ids.len() - 1;
        let d = self.dims;
        if row != last {
            self.vectors.copy_within(last * d..(last + 1) * d, row * d);
            self.hashes[row] = self.hashes[last];
            self.ids[row] = self.ids[last];
            self.rows.insert(self.ids[row], row);
        }
        // Zeroed before truncating: the spare capacity is only wiped when the buffer is dropped.
        self.vectors[last * d..].zeroize();
        self.hashes[last].zeroize();
        self.vectors.truncate(last * d);
        self.hashes.truncate(last);
        self.ids.truncate(last);
    }

    /// Up to `k` notes most similar to `id`, most similar first, `id` itself left out.
    /// `None` when `id` has no vector.
    pub fn nearest(&self, id: i64, k: usize) -> Option<Vec<i64>> {
        let &row = self.rows.get(&id)?;
        let d = self.dims;
        let query = &self.vectors[row * d..(row + 1) * d];
        let mut scores: Vec<(f32, usize)> = self
            .vectors
            .chunks_exact(d)
            .enumerate()
            .filter(|&(i, _)| i != row)
            .map(|(i, v)| (dot(query, v), i))
            .collect();
        let k = k.min(scores.len());
        if k == 0 {
            return Some(Vec::new());
        }
        // Highest score first; ties by row, so the order is stable.
        let order = |a: &(f32, usize), b: &(f32, usize)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1));
        if k < scores.len() {
            scores.select_nth_unstable_by(k - 1, order);
            scores.truncate(k);
        }
        scores.sort_unstable_by(order);
        Some(scores.into_iter().map(|(_, i)| self.ids[i]).collect())
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
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
pub fn sync(session: &Session, generation: u64) -> Result<bool, SessionError> {
    let (ready, live) = session.with_vault_quiet(generation, |v| {
        let Some(model) = v.embedding_model_ready()? else { return Ok((None, Vec::new())) };
        let hashes = v.embedding_hashes(&model)?;
        Ok((Some(model), hashes))
    })?;

    let (rebuild, remove, load) = {
        let index = session.related_index();
        match index.as_ref() {
            Some(index) if index.model == ready => {
                let live_ids: HashSet<i64> = live.iter().map(|(id, _)| *id).collect();
                let remove: Vec<i64> = index.ids.iter().copied().filter(|id| !live_ids.contains(id)).collect();
                let load: Vec<i64> =
                    live.iter().filter(|(id, hash)| index.hash(*id) != Some(hash)).map(|(id, _)| *id).collect();
                (false, remove, load)
            }
            _ => (true, Vec::new(), live.iter().map(|(id, _)| *id).collect()),
        }
    };
    if !rebuild && remove.is_empty() && load.is_empty() {
        return Ok(false);
    }

    let mut loaded: Vec<StoredVector> = Vec::with_capacity(load.len());
    if let Some(model) = &ready {
        for page in load.chunks(LOAD_PAGE) {
            loaded.extend(session.with_vault_quiet(generation, |v| v.embedding_vectors(model, page))?);
        }
    }

    session
        .update_related_index(generation, |index| {
            if rebuild || index.is_none() {
                *index = Some(RelatedIndex::new(ready.clone(), live.len()));
            }
            let Some(index) = index.as_mut() else { return };
            for &id in &remove {
                index.remove(id);
            }
            for v in &loaded {
                index.upsert(v.id, v.hash, &v.vector);
            }
        })
        .ok_or(SessionError::Locked)?;
    Ok(true)
}

/// Up to `limit` live notes most similar to note `id`, most similar first. `None` when
/// the note has no vector yet (empty, not indexed yet, or no model is ready).
///
/// The scan holds only the index; the vault is taken afterwards, to look up the
/// summaries. It does not count as activity: the frontend also asks after a background
/// re-embed. Notes trashed or deleted since the job last synced the index are skipped.
pub fn related_notes(session: &Session, id: i64, limit: usize) -> Result<Option<Vec<NoteSummary>>, SessionError> {
    let generation = session.generation();
    if generation == 0 {
        return Err(SessionError::Locked);
    }
    let mut k = limit;
    loop {
        let Some(ids) = session.related_index().as_ref().and_then(|index| index.nearest(id, k)) else {
            return Ok(None);
        };
        let exhausted = ids.len() < k;
        let mut notes = session.with_vault_quiet(generation, |v| v.note_summaries(&ids))?;
        if notes.len() >= limit || exhausted {
            notes.truncate(limit);
            return Ok(Some(notes));
        }
        k *= 2;
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
        for (id, v) in &rows {
            let row = index.rows[id];
            let mut expected = v.to_vec();
            crate::embed::normalize(&mut expected);
            assert_eq!(&index.vectors[row * 3..row * 3 + 3], &expected[..]);
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
}
