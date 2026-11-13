//! The note every new vault starts with (#177): what to know before the vault fills
//! up, above all to choose the embedding model before a large import, because
//! switching later embeds every note again.

use super::{Result, Vault};

/// The note's title; it goes in the root folder.
pub const README_TITLE: &str = "README";

/// The note's body. Kept as a Markdown file so it can be edited without touching code.
pub const README_BODY: &str = include_str!("readme.md");

impl Vault {
    /// Adds the README note. Called once, when the app creates a vault: never on open,
    /// so a deleted README stays deleted and existing vaults don't get one.
    pub fn add_readme(&mut self) -> Result<i64> {
        self.create_note("", README_TITLE, README_BODY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::MODELS;

    fn new_vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::create(&dir.path().join("v.hush"), "test passphrase").unwrap();
        (dir, vault)
    }

    #[test]
    fn adds_one_note_in_the_root_folder() {
        let (_dir, mut vault) = new_vault();
        let id = vault.add_readme().unwrap();
        let note = vault.note(id).unwrap();
        assert_eq!(note.title, README_TITLE);
        assert_eq!(note.folder, "");
        assert_eq!(note.body, README_BODY);
        let notes: i64 = vault.conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0)).unwrap();
        assert_eq!(notes, 1);
    }

    /// The examples (`[[Note title]]`, `#tag`) are in code spans, so the README makes
    /// no tag and no unresolved link in a vault that has nothing else yet.
    #[test]
    fn makes_no_tags_and_no_links() {
        let (_dir, mut vault) = new_vault();
        let id = vault.add_readme().unwrap();
        let tags: i64 = vault.conn.query_row("SELECT count(*) FROM note_tags", [], |r| r.get(0)).unwrap();
        assert_eq!(tags, 0);
        assert!(vault.outgoing_links(id).unwrap().is_empty());
    }

    /// The model list is hand-written: it must name every model Settings → Model
    /// offers, by the name shown there, and no other.
    #[test]
    fn names_exactly_the_models_in_the_catalogue() {
        for model in MODELS {
            assert!(README_BODY.contains(&format!("- **{}**", model.name)), "the README doesn't name {}", model.name);
        }
        let listed = README_BODY.lines().filter(|l| l.starts_with("- **")).count();
        assert_eq!(listed, MODELS.len(), "the README lists a model that isn't in MODELS");
    }
}
