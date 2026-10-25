//! Note embeddings: one vector per note and model, kept in the vault because a vector
//! gives away about as much as the text it was made from. The background job in
//! [`crate::embed::job`] keeps them up to date; this module only stores them.
//!
//! Two settings drive it: `embedding_model` is the model the job embeds with, and
//! `embedding_model_ready` the model whose vectors are complete. Everything that reads
//! vectors uses the ready model, so switching models never leaves the user without them.

use rusqlite::{OptionalExtension, params};
use zeroize::Zeroizing;

use super::{Result, Vault};
use crate::embed;

pub const EMBEDDING_MODEL_SETTING: &str = "embedding_model";
pub const EMBEDDING_MODEL_READY_SETTING: &str = "embedding_model_ready";

/// The text of a note as it is embedded, with its hash.
pub struct NoteText {
    pub id: i64,
    pub text: Zeroizing<String>,
    pub hash: [u8; 32],
}

/// A stored vector, as the related-notes index loads it.
pub struct StoredVector {
    pub id: i64,
    pub hash: [u8; 32],
    pub vector: Zeroizing<Vec<f32>>,
}

/// Stores a vector unless the note was trashed or deleted since its text was read.
const WRITE_EMBEDDING: &str = "INSERT INTO note_embeddings(note_id, model, content_hash, vector)
     SELECT id, ?2, ?3, ?4 FROM notes WHERE id = ?1 AND deleted_at IS NULL
     ON CONFLICT(note_id, model) DO UPDATE SET content_hash = excluded.content_hash, vector = excluded.vector";

/// A queued note's text, with the queue version it was read at.
pub struct QueuedText {
    pub note: NoteText,
    pub version: i64,
}

/// A vector made from a [`QueuedText`].
pub struct QueuedVector {
    pub id: i64,
    pub version: i64,
    pub hash: [u8; 32],
    pub vector: Vec<f32>,
}

/// One page of the embedding queue (see [`Vault::queued_embeddings`]).
#[derive(Default)]
pub struct QueuePage {
    /// Queued notes whose vector is missing or was made from other text.
    pub stale: Vec<QueuedText>,
    /// Queued notes that needed no embedding and left the queue.
    pub settled: usize,
    /// Of those, the ones without a vector now (emptied or trashed).
    pub dropped: Vec<i64>,
    /// Where to continue, or `None` when this was the last page.
    pub next: Option<QueueCursor>,
}

/// Where [`Vault::queued_embeddings`] continues: the queue is read most recently
/// updated note first, so the notes in use get their vectors first. Start from the
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueCursor {
    updated_at: i64,
    id: i64,
}

impl Default for QueueCursor {
    fn default() -> Self {
        Self { updated_at: i64::MAX, id: i64::MAX }
    }
}

/// Up to this many queued notes are ordered by sorting them; a longer queue is read by
/// walking `notes_updated` newest first and keeping the queued notes.
const QUEUE_SORT_MAX: i64 = 5_000;

/// One page of a scan for notes whose vector is missing or out of date.
#[derive(Debug, Default)]
pub struct EmbeddingScan {
    /// Notes that need (re-)embedding.
    pub stale: Vec<i64>,
    /// Notes in the page that have text to embed, stale or not.
    pub embeddable: usize,
    /// The id to continue after, or `None` when this was the last page.
    pub next: Option<i64>,
}

impl Vault {
    /// The model new vectors are made with.
    pub fn embedding_model(&self) -> Result<String> {
        Ok(self.setting(EMBEDDING_MODEL_SETTING)?.unwrap_or_else(|| embed::DEFAULT_MODEL.to_string()))
    }

    /// Switching to another model queues every live note for it.
    pub fn set_embedding_model(&self, model: &str) -> Result<()> {
        if self.setting(EMBEDDING_MODEL_SETTING)?.as_deref() == Some(model) {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![EMBEDDING_MODEL_SETTING, model],
        )?;
        tx.execute(
            "INSERT INTO embedding_dirty(note_id) SELECT id FROM notes WHERE deleted_at IS NULL
             ON CONFLICT(note_id) DO UPDATE SET version = version + 1",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The model whose vectors cover every note, if any has finished yet.
    pub fn embedding_model_ready(&self) -> Result<Option<String>> {
        self.setting(EMBEDDING_MODEL_READY_SETTING)
    }

    /// Scans up to `limit` live notes with an id above `after`, in id order, for ones
    /// whose `model` vector is missing or was made from different text. Notes with
    /// nothing to embed lose any vector they had.
    pub fn scan_embeddings(&mut self, model: &str, after: i64, limit: u32) -> Result<EmbeddingScan> {
        let tx = self.conn.transaction()?;
        let mut scan = EmbeddingScan::default();
        let mut empty = Vec::new();
        let mut seen = 0;
        {
            let mut stmt = tx.prepare_cached(
                "SELECT n.id, n.title, n.body, e.content_hash FROM notes n
                 LEFT JOIN note_embeddings e ON e.note_id = n.id AND e.model = ?1
                 WHERE n.deleted_at IS NULL AND n.id > ?2 ORDER BY n.id LIMIT ?3",
            )?;
            let mut rows = stmt.query(params![model, after, limit])?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let title: String = r.get(1)?;
                let body = Zeroizing::new(r.get::<_, String>(2)?);
                let stored: Option<Vec<u8>> = r.get(3)?;
                seen += 1;
                scan.next = Some(id);
                match embed::note_text(&title, &body) {
                    Some(text) => {
                        scan.embeddable += 1;
                        if stored.as_deref() != Some(&embed::content_hash(&text)[..]) {
                            scan.stale.push(id);
                        }
                    }
                    None if stored.is_some() => empty.push(id),
                    None => {}
                }
            }
        }
        if seen < limit {
            scan.next = None;
        }
        for id in empty {
            tx.prepare_cached("DELETE FROM note_embeddings WHERE note_id = ?1 AND model = ?2")?.execute(params![id, model])?;
        }
        tx.commit()?;
        Ok(scan)
    }

    /// Up to `limit` queued notes after `after`, most recently updated first. Those
    /// whose text still matches their `model` vector, that have no text or that are
    /// trashed leave the queue here (losing any vector they had); the rest are returned
    /// with their text, to embed and write with [`Vault::write_queued_embeddings`].
    pub fn queued_embeddings(&mut self, model: &str, after: &QueueCursor, limit: u32) -> Result<QueuePage> {
        let tx = self.conn.transaction()?;
        let mut page = QueuePage::default();
        let mut settled = Vec::new();
        // The page's notes in order. A short queue is sorted; for a long one the index
        // on `updated_at` gives the order and the queue is looked up per note.
        let queued: i64 =
            tx.query_row(&format!("SELECT count(*) FROM (SELECT 1 FROM embedding_dirty LIMIT {})", QUEUE_SORT_MAX + 1), [], |r| r.get(0))?;
        let order: Vec<(i64, i64)> = {
            let sql = if queued <= QUEUE_SORT_MAX {
                "SELECT n.id, n.updated_at FROM embedding_dirty q CROSS JOIN notes n ON n.id = q.note_id
                 WHERE (n.updated_at, n.id) < (?1, ?2) ORDER BY n.updated_at DESC, n.id DESC LIMIT ?3"
            } else {
                "SELECT n.id, n.updated_at FROM notes n INDEXED BY notes_updated
                 WHERE (n.updated_at, n.id) < (?1, ?2) AND EXISTS (SELECT 1 FROM embedding_dirty q WHERE q.note_id = n.id)
                 ORDER BY n.updated_at DESC, n.id DESC LIMIT ?3"
            };
            let mut stmt = tx.prepare_cached(sql)?;
            stmt.query_map(params![after.updated_at, after.id, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        if order.len() == limit as usize {
            page.next = order.last().map(|&(id, updated_at)| QueueCursor { updated_at, id });
        }
        {
            let mut stmt = tx.prepare_cached(
                "SELECT q.version, n.title, n.body, n.deleted_at IS NULL, e.content_hash
                 FROM embedding_dirty q JOIN notes n ON n.id = q.note_id
                 LEFT JOIN note_embeddings e ON e.note_id = q.note_id AND e.model = ?1
                 WHERE q.note_id = ?2",
            )?;
            for &(id, _) in &order {
                let mut rows = stmt.query(params![model, id])?;
                let Some(r) = rows.next()? else { continue };
                let version: i64 = r.get(0)?;
                let live: bool = r.get(3)?;
                let stored: Option<Vec<u8>> = r.get(4)?;
                let text = if live {
                    let title: String = r.get(1)?;
                    let body = Zeroizing::new(r.get::<_, String>(2)?);
                    embed::note_text(&title, &body)
                } else {
                    None
                };
                match text {
                    Some(text) => {
                        let hash = embed::content_hash(&text);
                        if stored.as_deref() == Some(&hash[..]) {
                            settled.push((id, version, None));
                        } else {
                            page.stale.push(QueuedText { note: NoteText { id, text, hash }, version });
                        }
                    }
                    None => settled.push((id, version, Some(stored.is_some()))),
                }
            }
        }
        // `lost`: no vector now; `Some(true)` if it had one to delete.
        for &(id, version, lost) in &settled {
            if let Some(had) = lost {
                if had {
                    tx.prepare_cached("DELETE FROM note_embeddings WHERE note_id = ?1 AND model = ?2")?
                        .execute(params![id, model])?;
                }
                page.dropped.push(id);
            }
            tx.prepare_cached("DELETE FROM embedding_dirty WHERE note_id = ?1 AND version = ?2")?.execute(params![id, version])?;
        }
        tx.commit()?;
        page.settled = settled.len();
        Ok(page)
    }

    /// Stores `model` vectors for queued notes, as [`Vault::write_embeddings`], and takes
    /// each note off the queue unless it was queued again since its text was read.
    pub fn write_queued_embeddings(&mut self, model: &str, vectors: &[QueuedVector]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut write = tx.prepare_cached(WRITE_EMBEDDING)?;
            let mut done = tx.prepare_cached("DELETE FROM embedding_dirty WHERE note_id = ?1 AND version = ?2")?;
            for v in vectors {
                write.execute(params![v.id, model, &v.hash[..], embed::vector_to_blob(&v.vector)])?;
                done.execute(params![v.id, v.version])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Queues `ids` for the embedding job (the unlock safety-net scan uses it).
    pub fn queue_embeddings(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO embedding_dirty(note_id) SELECT id FROM notes WHERE id = ?1
                 ON CONFLICT(note_id) DO UPDATE SET version = version + 1",
            )?;
            for id in ids {
                stmt.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Notes deleted for good since the last call (the record is cleared).
    pub fn take_deleted_notes(&mut self) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        let ids = tx
            .prepare_cached("SELECT note_id FROM embedding_gone")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        tx.execute("DELETE FROM embedding_gone", [])?;
        tx.commit()?;
        Ok(ids)
    }

    /// How many notes are queued for the embedding job.
    pub fn embedding_queue_len(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row("SELECT count(*) FROM embedding_dirty", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Whether the embedding job has anything to do: queued or deleted notes, a model
    /// switch to finish, or tag descriptions without a vector from the ready model. Cheap; the
    /// idle job asks once a second.
    pub fn embedding_pending(&self) -> Result<bool> {
        if self.conn.prepare_cached("SELECT 1 FROM embedding_dirty LIMIT 1")?.exists([])?
            || self.conn.prepare_cached("SELECT 1 FROM embedding_gone LIMIT 1")?.exists([])?
        {
            return Ok(true);
        }
        let model = self.embedding_model()?;
        if self.embedding_model_ready()?.as_deref() != Some(model.as_str()) {
            return Ok(true);
        }
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM tag_descriptions WHERE vector IS NULL OR model IS NOT ?1 LIMIT 1")?
            .exists([&model])?)
    }

    /// The text to embed for each of `ids` that is live and not empty.
    pub fn embedding_texts(&self, ids: &[i64]) -> Result<Vec<NoteText>> {
        let mut stmt = self.conn.prepare_cached("SELECT title, body FROM notes WHERE id = ?1 AND deleted_at IS NULL")?;
        let mut texts = Vec::with_capacity(ids.len());
        for &id in ids {
            let Some((title, body)) = stmt
                .query_row([id], |r| Ok((r.get::<_, String>(0)?, Zeroizing::new(r.get::<_, String>(1)?))))
                .optional()?
            else {
                continue;
            };
            if let Some(text) = embed::note_text(&title, &body) {
                let hash = embed::content_hash(&text);
                texts.push(NoteText { id, text, hash });
            }
        }
        Ok(texts)
    }

    /// Stores `model` vectors, each with the hash of the text it was made from. Notes
    /// trashed or deleted since their text was read are left without one.
    pub fn write_embeddings(&mut self, model: &str, vectors: &[(i64, [u8; 32], Vec<f32>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(WRITE_EMBEDDING)?;
            for (id, hash, vector) in vectors {
                stmt.execute(params![id, model, &hash[..], embed::vector_to_blob(vector)])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Marks `model` as the one whose vectors are complete and deletes every other
    /// model's vectors. Called by the job once every note has a `model` vector.
    pub fn finish_embedding_model(&mut self, model: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![EMBEDDING_MODEL_READY_SETTING, model],
        )?;
        tx.execute("DELETE FROM note_embeddings WHERE model <> ?1", [model])?;
        tx.commit()?;
        Ok(())
    }

    /// A note's vector from the ready model, if it has one.
    pub fn note_embedding(&self, id: i64) -> Result<Option<Vec<f32>>> {
        let Some(model) = self.embedding_model_ready()? else { return Ok(None) };
        let blob: Option<Vec<u8>> = self
            .conn
            .prepare_cached(
                "SELECT e.vector FROM note_embeddings e JOIN notes n ON n.id = e.note_id
                 WHERE e.note_id = ?1 AND e.model = ?2 AND n.deleted_at IS NULL",
            )?
            .query_row(params![id, model], |r| r.get(0))
            .optional()?;
        Ok(blob.map(|b| embed::blob_to_vector(&b)))
    }

    /// Every live note with a `model` vector, with the hash of the text it was made
    /// from, in id order. Cheap enough to call after every change: no vectors.
    pub fn embedding_hashes(&self, model: &str) -> Result<Vec<(i64, [u8; 32])>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT e.note_id, e.content_hash FROM note_embeddings e JOIN notes n ON n.id = e.note_id
             WHERE e.model = ?1 AND n.deleted_at IS NULL ORDER BY e.note_id",
        )?;
        let mut rows = stmt.query([model])?;
        let mut hashes = Vec::new();
        while let Some(r) = rows.next()? {
            let hash: Vec<u8> = r.get(1)?;
            if let Ok(hash) = <[u8; 32]>::try_from(hash.as_slice()) {
                hashes.push((r.get(0)?, hash));
            }
        }
        Ok(hashes)
    }

    /// The `model` vectors of those of `ids` that are live and have one, each with the
    /// hash of the text it was made from.
    pub fn embedding_vectors(&self, model: &str, ids: &[i64]) -> Result<Vec<StoredVector>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT e.content_hash, e.vector FROM note_embeddings e JOIN notes n ON n.id = e.note_id
             WHERE e.note_id = ?1 AND e.model = ?2 AND n.deleted_at IS NULL",
        )?;
        let mut vectors = Vec::with_capacity(ids.len());
        for &id in ids {
            let Some((hash, blob)) = stmt
                .query_row(params![id, model], |r| Ok((r.get::<_, Vec<u8>>(0)?, Zeroizing::new(r.get::<_, Vec<u8>>(1)?))))
                .optional()?
            else {
                continue;
            };
            let Ok(hash) = <[u8; 32]>::try_from(hash.as_slice()) else { continue };
            vectors.push(StoredVector { id, hash, vector: Zeroizing::new(embed::blob_to_vector(&blob)) });
        }
        Ok(vectors)
    }

    /// How many vectors are stored for `model`.
    pub fn embedding_count(&self, model: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row("SELECT count(*) FROM note_embeddings WHERE model = ?1", [model], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// A counter that grows with every row this connection inserts, updates or deletes,
    /// so the job can tell cheaply whether anything changed since it last looked.
    pub fn change_count(&self) -> u64 {
        self.conn.total_changes()
    }
}
