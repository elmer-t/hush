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

    pub fn set_embedding_model(&self, model: &str) -> Result<()> {
        self.set_setting(EMBEDDING_MODEL_SETTING, model)
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
            let mut stmt = tx.prepare_cached(
                "INSERT INTO note_embeddings(note_id, model, content_hash, vector)
                 SELECT id, ?2, ?3, ?4 FROM notes WHERE id = ?1 AND deleted_at IS NULL
                 ON CONFLICT(note_id, model) DO UPDATE SET content_hash = excluded.content_hash, vector = excluded.vector",
            )?;
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
