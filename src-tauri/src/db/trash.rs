//! The trash: deleting a note moves it here, from where it can be restored. Trashed
//! notes keep their row but lose their tag, attribute, link and embedding rows, and every other
//! query skips them, so they vanish from lists, search, filters and links. Deleting
//! from the trash is a real delete (`secure_delete` is on).
//!
//! A folder moved to the trash as a whole is one entry: a `trash_folders` row whose
//! `deleted_at` its notes share. Every trash operation gets its own `deleted_at`, so that
//! key alone finds the group's notes. Folder operations are set-based, one statement per
//! table, so a folder of 100k notes is one transaction of a handful of statements.
//!
//! Every real delete then gives the freed pages back to the disk once the vault was
//! compacted (see [`super::compact`]).

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::notes::{IN_SUBTREE, derive, subtree_params, title_taken};
use super::{Error, Result, Vault, links, now_ms};
use crate::markdown;

/// How many days a note stays in the trash before it is deleted on unlock; 0 = never.
pub const TRASH_DAYS_SETTING: &str = "trash_days";
pub const DEFAULT_TRASH_DAYS: u32 = 30;
pub const MAX_TRASH_DAYS: u32 = 3650;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedNote {
    pub id: i64,
    /// The folder it was deleted from, and is restored to.
    pub folder: String,
    pub title: String,
    pub deleted_at: i64,
}

/// One row of the trash list: a folder deleted as a whole, or a note on its own.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TrashEntry {
    #[serde(rename_all = "camelCase")]
    Folder {
        /// Identifies the group: its notes share this `deleted_at`.
        deleted_at: i64,
        folder: String,
        notes: usize,
    },
    #[serde(rename_all = "camelCase")]
    Note(TrashedNote),
}

/// What deleting a folder would move to the trash, for the confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderContents {
    /// Live notes in the folder and below it.
    pub notes: usize,
    /// Folders below it that hold notes, or hold folders that do.
    pub subfolders: usize,
}

/// A `deleted_at` for a new trash operation: now, but never one an existing operation
/// uses, so a folder group can be found by its key alone.
fn trash_time(conn: &Connection) -> Result<i64> {
    let latest: Option<i64> = conn.query_row("SELECT max(deleted_at) FROM notes", [], |r| r.get(0))?;
    Ok(latest.map_or(now_ms(), |l| now_ms().max(l + 1)))
}

/// Drops folder groups whose notes are all gone (purged, deleted or restored).
pub(super) fn forget_empty_groups(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM trash_folders WHERE NOT EXISTS (SELECT 1 FROM notes n WHERE n.deleted_at = trash_folders.deleted_at)",
        [],
    )?;
    Ok(())
}

impl Vault {
    /// Moves a note to the trash.
    pub fn trash_note(&mut self, id: i64) -> Result<()> {
        self.trash_notes(&[id])
    }

    /// Moves several notes to the trash in one transaction: all or nothing, so an
    /// unknown (or already trashed) id changes nothing. The single entry point for
    /// bulk delete.
    pub fn trash_notes(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let now = trash_time(&tx)?;
        for &id in ids {
            if tx.execute("UPDATE notes SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, now])?
                == 0
            {
                return Err(Error::NoteNotFound);
            }
            for table in ["note_tags", "note_attributes", "note_embeddings"] {
                tx.execute(&format!("DELETE FROM {table} WHERE note_id = ?1"), [id])?;
            }
            tx.execute("DELETE FROM links WHERE from_note_id = ?1", [id])?;
            super::recent::forget_last_opened(&tx, id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The live notes and subfolders in `folder` and below it, for the confirmation before
    /// it is deleted. Both read a range of the `folder` index.
    pub fn folder_contents(&self, folder: &str) -> Result<FolderContents> {
        let folder = markdown::normalize_folder(folder);
        if folder.is_empty() {
            return Err(Error::InvalidFolder("the top level can't be deleted".into()));
        }
        let (a, b, c) = subtree_params(&folder);
        let notes: i64 = self.conn.query_row(
            &format!("SELECT count(*) FROM notes WHERE deleted_at IS NULL AND {IN_SUBTREE}"),
            params![a, b, c],
            |r| r.get(0),
        )?;
        // `a/b/c/d` holding notes makes `c` and `c/d` subfolders of `a/b`.
        // Only the range part of `IN_SUBTREE`: the folder itself isn't its own subfolder.
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT folder FROM notes WHERE deleted_at IS NULL AND folder > ?1 AND folder < ?2")?;
        let mut below = HashSet::new();
        let prefix = folder.chars().count() + 1;
        for row in stmt.query_map(params![b, c], |r| r.get::<_, String>(0))? {
            let rest: String = row?.chars().skip(prefix).collect::<String>().to_lowercase();
            let mut path = String::new();
            for part in rest.split('/') {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(part);
                below.insert(path.clone());
            }
        }
        Ok(FolderContents { notes: notes as usize, subfolders: below.len() })
    }

    /// Moves `folder` with every live note in it and below it to the trash, in one
    /// transaction, as one trash entry. Notes already in the trash stay separate entries.
    /// Returns how many notes were trashed; a folder without live notes changes nothing.
    pub fn trash_folder(&mut self, folder: &str) -> Result<usize> {
        let folder = markdown::normalize_folder(folder);
        if folder.is_empty() {
            return Err(Error::InvalidFolder("the top level can't be deleted".into()));
        }
        let tx = self.conn.transaction()?;
        let now = trash_time(&tx)?;
        let (a, b, c) = subtree_params(&folder);
        let trashed = tx.execute(
            &format!("UPDATE notes SET deleted_at = ?4 WHERE deleted_at IS NULL AND {IN_SUBTREE}"),
            params![a, b, c, now],
        )?;
        if trashed == 0 {
            return Ok(0);
        }
        // The group's notes are exactly those with `deleted_at = now` (see `trash_time`).
        for (table, column) in
            [("note_tags", "note_id"), ("note_attributes", "note_id"), ("note_embeddings", "note_id"), ("links", "from_note_id")]
        {
            tx.execute(
                &format!("DELETE FROM {table} WHERE {column} IN (SELECT id FROM notes WHERE deleted_at = ?1)"),
                [now],
            )?;
        }
        super::recent::forget_last_opened_trashed_at(&tx, now)?;
        tx.execute("INSERT INTO trash_folders(deleted_at, folder) VALUES (?1, ?2)", params![now, folder])?;
        tx.commit()?;
        Ok(trashed)
    }

    /// Restores every note of a folder group into its original folder, in one
    /// transaction. Clashing titles get the "(restored)" rule of [`Vault::restore_note`],
    /// and links that resolved uniquely before keep their meaning, judged once over the
    /// whole group. Returns how many notes came back.
    pub fn restore_folder(&mut self, deleted_at: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        if !tx.prepare("SELECT 1 FROM trash_folders WHERE deleted_at = ?1")?.exists([deleted_at])? {
            return Err(Error::NoteNotFound);
        }
        let mut notes: Vec<(i64, String, String)> = {
            let mut stmt = tx.prepare("SELECT id, folder, title FROM notes WHERE deleted_at = ?1 ORDER BY id")?;
            stmt.query_map([deleted_at], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };

        // The group's notes were live together, so they don't clash with each other; only
        // with live notes, and a renamed one with any title already given out here.
        let path = |folder: &str, title: &str| (folder.to_lowercase(), title.to_lowercase());
        let mut renames: Vec<(usize, String)> = Vec::new();
        let mut taken: HashSet<(String, String)> = HashSet::new();
        {
            let mut live = tx.prepare_cached("SELECT 1 FROM notes WHERE folder = ?1 AND title = ?2 AND deleted_at IS NULL")?;
            for (i, (_, folder, title)) in notes.iter().enumerate() {
                if live.exists(params![folder, title])? {
                    renames.push((i, String::new()));
                } else {
                    taken.insert(path(folder, title));
                }
            }
            for (i, new_title) in &mut renames {
                let (_, folder, title) = &notes[*i];
                let mut n = 1;
                *new_title = loop {
                    let candidate =
                        if n == 1 { format!("{title} (restored)") } else { format!("{title} (restored {n})") };
                    if !taken.contains(&path(folder, &candidate)) && !live.exists(params![folder, candidate])? {
                        break candidate;
                    }
                    n += 1;
                };
                taken.insert(path(folder, new_title));
            }
        }
        for (i, new_title) in &renames {
            notes[*i].2 = new_title.clone();
        }

        let mut titles: Vec<&str> = notes.iter().map(|(_, _, t)| t.as_str()).collect();
        titles.sort_unstable_by_key(|t| t.to_lowercase());
        titles.dedup_by_key(|t| t.to_lowercase());
        links::preserving_links(&tx, &titles, |conn| {
            let mut rename = conn.prepare_cached("UPDATE notes SET title = ?2 WHERE id = ?1")?;
            for (i, new_title) in &renames {
                rename.execute(params![notes[*i].0, new_title])?;
            }
            conn.execute("UPDATE notes SET deleted_at = NULL WHERE deleted_at = ?1", [deleted_at]).map_err(title_taken)?;
            let mut body = conn.prepare_cached("SELECT body FROM notes WHERE id = ?1")?;
            for (id, _, _) in &notes {
                let text: String = body.query_row([id], |r| r.get(0))?;
                derive(conn, *id, &text)?;
            }
            Ok(())
        })?;
        tx.execute("DELETE FROM trash_folders WHERE deleted_at = ?1", [deleted_at])?;
        tx.commit()?;
        Ok(notes.len())
    }

    /// Deletes every note of a folder group for good. Returns how many.
    pub fn delete_trash_folder(&mut self, deleted_at: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        if tx.execute("DELETE FROM trash_folders WHERE deleted_at = ?1", [deleted_at])? == 0 {
            return Err(Error::NoteNotFound);
        }
        let deleted = tx.execute("DELETE FROM notes WHERE deleted_at = ?1", [deleted_at])?;
        tx.commit()?;
        // The delete is done; space not given back now is given back next time.
        let _ = self.give_back_free_pages();
        Ok(deleted)
    }

    /// One page of the trash, most recently deleted first: each folder group as one
    /// entry, and every other trashed note on its own.
    pub fn trash_entries(&self, offset: u32, limit: u32) -> Result<Vec<TrashEntry>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT * FROM (
                 SELECT NULL AS id, g.folder, NULL AS title, g.deleted_at,
                        (SELECT count(*) FROM notes n WHERE n.deleted_at = g.deleted_at) AS notes
                 FROM trash_folders g
                 UNION ALL
                 SELECT n.id, n.folder, n.title, n.deleted_at, 1 FROM notes n
                 WHERE n.deleted_at IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM trash_folders g WHERE g.deleted_at = n.deleted_at)
             )
             ORDER BY deleted_at DESC, id DESC LIMIT ?1 OFFSET ?2",
        )?;
        let entries = stmt
            .query_map(params![limit, offset], |r| {
                let deleted_at = r.get(3)?;
                Ok(match r.get::<_, Option<i64>>(0)? {
                    Some(id) => TrashEntry::Note(TrashedNote { id, folder: r.get(1)?, title: r.get(2)?, deleted_at }),
                    None => TrashEntry::Folder { deleted_at, folder: r.get(1)?, notes: r.get::<_, i64>(4)? as usize },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(entries)
    }

    /// How many notes are in the trash, folder groups included.
    pub fn trash_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row("SELECT count(*) FROM notes WHERE deleted_at IS NOT NULL", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Trashed notes, most recently deleted first.
    pub fn trashed_notes(&self) -> Result<Vec<TrashedNote>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, folder, title, deleted_at FROM notes WHERE deleted_at IS NOT NULL
             ORDER BY deleted_at DESC, id DESC",
        )?;
        let notes = stmt
            .query_map([], |r| Ok(TrashedNote { id: r.get(0)?, folder: r.get(1)?, title: r.get(2)?, deleted_at: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(notes)
    }

    /// Restores a trashed note to its folder. If a live note has taken its folder and
    /// title meanwhile, it comes back as "Title (restored)" (or "(restored 2)", …).
    /// Like creating a note, links that resolved uniquely before keep their meaning.
    /// Returns the title it was restored under.
    pub fn restore_note(&mut self, id: i64) -> Result<String> {
        let tx = self.conn.transaction()?;
        let (title, body): (String, String) = tx
            .query_row("SELECT title, body FROM notes WHERE id = ?1 AND deleted_at IS NOT NULL", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?
            .ok_or(Error::NoteNotFound)?;
        let mut n = 1;
        let restored = loop {
            let candidate = match n {
                1 => title.clone(),
                2 => format!("{title} (restored)"),
                n => format!("{title} (restored {})", n - 1),
            };
            let outcome = links::preserving_links(&tx, &[&candidate], |conn| {
                conn.execute(
                    "UPDATE notes SET title = ?2, deleted_at = NULL WHERE id = ?1",
                    params![id, candidate],
                )
                .map_err(title_taken)?;
                derive(conn, id, &body)
            });
            match outcome {
                Ok(()) => break candidate,
                Err(Error::TitleTaken) => n += 1,
                Err(e) => return Err(e),
            }
        };
        forget_empty_groups(&tx)?;
        tx.commit()?;
        Ok(restored)
    }

    /// Deletes every trashed note for good. Returns how many.
    pub fn empty_trash(&mut self) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let deleted = tx.execute("DELETE FROM notes WHERE deleted_at IS NOT NULL", [])?;
        tx.execute("DELETE FROM trash_folders", [])?;
        tx.commit()?;
        // The delete is done; space not given back now is given back next time.
        let _ = self.give_back_free_pages();
        Ok(deleted)
    }

    /// The trash retention in days; 0 means trashed notes are kept until deleted by hand.
    pub fn trash_days(&self) -> Result<u32> {
        Ok(self
            .setting(TRASH_DAYS_SETTING)?
            .and_then(|v| v.parse().ok())
            .filter(|d| *d <= MAX_TRASH_DAYS)
            .unwrap_or(DEFAULT_TRASH_DAYS))
    }

    pub fn set_trash_days(&self, days: u32) -> Result<()> {
        if days > MAX_TRASH_DAYS {
            return Err(Error::InvalidTrashDays);
        }
        self.set_setting(TRASH_DAYS_SETTING, &days.to_string())
    }

    /// Deletes trashed notes older than the retention period at `now` (Unix millis).
    /// Run on unlock. Returns how many were deleted.
    pub fn purge_trash_at(&mut self, now: i64) -> Result<usize> {
        let days = self.trash_days()?;
        if days == 0 {
            return Ok(0);
        }
        let cutoff = now - i64::from(days) * DAY_MS;
        // A folder group's notes share `deleted_at`, so it goes as a whole.
        let tx = self.conn.transaction()?;
        let deleted = tx.execute("DELETE FROM notes WHERE deleted_at IS NOT NULL AND deleted_at <= ?1", [cutoff])?;
        tx.execute("DELETE FROM trash_folders WHERE deleted_at <= ?1", [cutoff])?;
        tx.commit()?;
        // The delete is done; space not given back now is given back next time.
        let _ = self.give_back_free_pages();
        Ok(deleted)
    }

    pub fn purge_trash(&mut self) -> Result<usize> {
        self.purge_trash_at(now_ms())
    }

    /// Pretends a trashed note was deleted `days` ago.
    #[cfg(test)]
    pub(crate) fn backdate_trash(&self, id: i64, days: i64) -> Result<()> {
        self.conn.execute("UPDATE notes SET deleted_at = deleted_at - ?2 WHERE id = ?1", params![id, days * DAY_MS])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::create(&dir.path().join("v.hush"), "pass").unwrap();
        (dir, vault)
    }

    fn count(vault: &Vault, sql: &str) -> i64 {
        vault.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn plan(vault: &Vault, sql: &str) -> String {
        let mut stmt = vault.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        // Unbound parameters are NULL, which doesn't change the plan.
        let mut rows = stmt.raw_query();
        let mut out = Vec::new();
        while let Some(r) = rows.next().unwrap() {
            out.push(r.get::<_, String>(3).unwrap());
        }
        out.join("\n")
    }

    /// Trashing a folder drops the derived rows of its notes, the vectors too, and
    /// leaves the sibling folder's notes alone.
    #[test]
    fn trash_folder_removes_derived_rows() {
        let (_dir, mut v) = vault();
        let inside = v.create_note("a/b", "In", "---\nmood: calm\n---\n#bird [[Out]]").unwrap();
        let sibling = v.create_note("a/bc", "Out", "#bird [[In]]").unwrap();
        for id in [inside, sibling] {
            v.conn.execute("INSERT INTO note_embeddings VALUES (?1, 'm', x'00', x'0000803f')", [id]).unwrap();
        }
        assert_eq!(v.trash_folder("a/b").unwrap(), 1);
        for table in ["note_tags", "note_attributes", "note_embeddings"] {
            assert_eq!(count(&v, &format!("SELECT count(*) FROM {table} WHERE note_id = {inside}")), 0, "{table}");
        }
        assert_eq!(count(&v, &format!("SELECT count(*) FROM links WHERE from_note_id = {inside}")), 0);
        assert_eq!(count(&v, &format!("SELECT count(*) FROM note_tags WHERE note_id = {sibling}")), 1);
        assert_eq!(count(&v, &format!("SELECT count(*) FROM note_embeddings WHERE note_id = {sibling}")), 1);
        assert_eq!(count(&v, &format!("SELECT count(*) FROM links WHERE from_note_id = {sibling}")), 1);
        assert_eq!(count(&v, &format!("SELECT count(*) FROM embedding_dirty WHERE note_id = {inside}")), 1, "queued");
    }

    /// The folder match reads a range of an index on `folder`, never the whole table.
    #[test]
    fn folder_queries_use_the_folder_index() {
        let (_dir, v) = vault();
        let live = plan(&v, &format!("SELECT count(*) FROM notes WHERE deleted_at IS NULL AND {IN_SUBTREE}"));
        assert!(live.contains("notes_folder_title (folder>? AND folder<?)") && !live.contains("SCAN notes"), "{live}");
        let update = plan(&v, &format!("UPDATE notes SET deleted_at = ?4 WHERE deleted_at IS NULL AND {IN_SUBTREE}"));
        assert!(update.contains("notes_folder_title (folder>? AND folder<?)") && !update.contains("SCAN notes"), "{update}");
        let distinct = plan(&v, "SELECT DISTINCT folder FROM notes WHERE deleted_at IS NULL AND folder > ?1 AND folder < ?2");
        assert!(distinct.contains("notes_folder_title (folder>? AND folder<?)") && !distinct.contains("SCAN notes"), "{distinct}");
    }

    /// A group's notes are found by `deleted_at` through its index.
    #[test]
    fn group_queries_use_the_deleted_index() {
        let (_dir, v) = vault();
        let rows = plan(&v, "SELECT id FROM notes WHERE deleted_at = ?1");
        assert!(rows.contains("notes_deleted"), "{rows}");
    }

    #[test]
    fn a_folder_trash_never_shares_deleted_at_with_another_trash() {
        let (_dir, mut v) = vault();
        let loose = v.create_note("", "Loose", "").unwrap();
        v.create_note("f", "In", "").unwrap();
        v.trash_note(loose).unwrap();
        v.trash_folder("f").unwrap();
        let other = v.create_note("f", "Later", "").unwrap();
        v.trash_note(other).unwrap();
        assert_eq!(count(&v, "SELECT count(DISTINCT deleted_at) FROM notes"), 3);
    }
}
