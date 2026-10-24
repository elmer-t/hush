mod common;

use hush_lib::db::{DEFAULT_TRASH_DAYS, Error, Resolution, SearchQuery, Vault};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn titles(vault: &Vault, q: SearchQuery) -> Vec<String> {
    vault.search(&q).unwrap().into_iter().map(|h| h.note.title).collect()
}

fn trashed(vault: &Vault) -> Vec<String> {
    vault.trashed_notes().unwrap().into_iter().map(|n| n.title).collect()
}

#[test]
fn trash_and_restore() {
    let (_dir, mut vault) = common::vault();
    let id = vault.create_note("journal", "Day", "---\ntopic: life\n---\nherons #bird").unwrap();
    vault.trash_note(id).unwrap();

    assert!(matches!(vault.note(id), Err(Error::NoteNotFound)), "a trashed note can't be opened");
    assert!(matches!(vault.update_body(id, "late save"), Err(Error::NoteNotFound)));
    assert!(matches!(vault.rename_note(id, "", "Moved"), Err(Error::NoteNotFound)));
    assert!(matches!(vault.trash_note(id), Err(Error::NoteNotFound)), "already in the trash");
    let t = vault.trashed_notes().unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].id, t[0].folder.as_str(), t[0].title.as_str()), (id, "journal", "Day"));
    assert!(vault.folders().unwrap().is_empty());

    assert_eq!(vault.restore_note(id).unwrap(), "Day");
    let note = vault.note(id).unwrap();
    assert_eq!((note.folder.as_str(), note.title.as_str()), ("journal", "Day"));
    assert_eq!(note.tags, ["bird"], "tags are derived again");
    assert_eq!(titles(&vault, SearchQuery { text: "herons".into(), ..Default::default() }), ["Day"]);
    assert!(trashed(&vault).is_empty());
    assert!(matches!(vault.restore_note(id), Err(Error::NoteNotFound)), "not in the trash");
}

#[test]
fn trashed_note_is_hidden_from_search_filters_and_facets() {
    let (_dir, mut vault) = common::vault();
    let body = "---\nauthor: Jan\ntopic: life\nmood: calm\n---\nherons #bird";
    let id = vault.create_note("journal", "Day", body).unwrap();
    vault.create_note("", "Other", "nothing").unwrap();
    vault.trash_note(id).unwrap();

    let q = |f: fn(&mut SearchQuery)| {
        let mut q = SearchQuery::default();
        f(&mut q);
        q
    };
    assert_eq!(titles(&vault, SearchQuery::default()), ["Other"]);
    assert!(titles(&vault, q(|q| q.text = "herons".into())).is_empty());
    assert!(titles(&vault, q(|q| q.text = "Day".into())).is_empty(), "titles too");
    assert!(titles(&vault, q(|q| q.tags = vec!["bird".into()])).is_empty());
    assert!(titles(&vault, q(|q| q.topic = Some("life".into()))).is_empty());
    assert!(titles(&vault, q(|q| q.author = Some("Jan".into()))).is_empty());
    assert!(titles(&vault, q(|q| q.folder = Some("journal".into()))).is_empty());
    assert!(titles(&vault, q(|q| q.attributes = vec![("mood".into(), "calm".into())])).is_empty());
    let facets = vault.facets().unwrap();
    assert!(facets.tags.is_empty() && facets.topics.is_empty() && facets.authors.is_empty());
    assert!(facets.attribute_keys.is_empty());
}

#[test]
fn trashed_note_is_hidden_from_links_and_backlinks() {
    let (_dir, mut vault) = common::vault();
    let target = vault.create_note("p", "Target", "").unwrap();
    let source = vault.create_note("", "Source", "[[Target]] and [[Other]]").unwrap();
    let other = vault.create_note("", "Other", "").unwrap();

    vault.trash_note(target).unwrap();
    assert_eq!(vault.resolve_link("Target").unwrap(), Resolution::Missing, "shows as unresolved");
    let out: Vec<_> = vault.outgoing_links(source).unwrap().into_iter().map(|l| l.resolution).collect();
    assert_eq!(out, [Resolution::Unique(other), Resolution::Missing]);

    // A trashed note's own links are not backlinks.
    vault.trash_note(source).unwrap();
    assert!(vault.backlinks(other).unwrap().is_empty());

    vault.restore_note(source).unwrap();
    vault.restore_note(target).unwrap();
    assert_eq!(vault.resolve_link("Target").unwrap(), Resolution::Unique(target));
    assert_eq!(vault.backlinks(target).unwrap().len(), 1);
    assert_eq!(vault.backlinks(other).unwrap().len(), 1);
}

#[test]
fn a_new_note_can_take_the_name_of_a_trashed_one() {
    let (_dir, mut vault) = common::vault();
    let old = vault.create_note("a", "Plan", "old").unwrap();
    vault.trash_note(old).unwrap();
    let new = vault.create_note("a", "plan", "new").unwrap();
    assert_ne!(old, new);
    assert_eq!(vault.resolve_link("Plan").unwrap(), Resolution::Unique(new));
    assert!(vault.note_exists("a", "Plan").unwrap());
    let renamed = vault.create_note("a", "Spare", "").unwrap();
    vault.rename_note(renamed, "a", "Plan 2").unwrap();
}

#[test]
fn restore_with_a_name_conflict() {
    let (_dir, mut vault) = common::vault();
    let first = vault.create_note("a", "Plan", "first").unwrap();
    vault.trash_note(first).unwrap();
    let taken = vault.create_note("a", "Plan", "taken").unwrap();
    let second = vault.create_note("a", "Plan (restored)", "").unwrap();
    vault.trash_note(second).unwrap();
    vault.create_note("a", "Plan (restored)", "").unwrap();

    assert_eq!(vault.restore_note(first).unwrap(), "Plan (restored 2)");
    assert_eq!(vault.note(first).unwrap().title, "Plan (restored 2)");
    assert_eq!(vault.note(first).unwrap().body, "first");
    assert_eq!(vault.restore_note(second).unwrap(), "Plan (restored) (restored)");
    assert_eq!(vault.note(taken).unwrap().title, "Plan");
}

#[test]
fn restore_preserves_links_like_create() {
    let (_dir, mut vault) = common::vault();
    let trashed = vault.create_note("old", "Index", "").unwrap();
    vault.trash_note(trashed).unwrap();
    let live = vault.create_note("new", "Index", "").unwrap();
    let source = vault.create_note("", "Source", "see [[Index|the index]]").unwrap();
    assert_eq!(vault.resolve_link("Index").unwrap(), Resolution::Unique(live));

    vault.restore_note(trashed).unwrap();
    assert_eq!(vault.note(source).unwrap().body, "see [[new/Index|the index]]");
    assert_eq!(vault.resolve_link("new/Index").unwrap(), Resolution::Unique(live));
}

#[test]
fn delete_forever_from_trash_and_directly() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("", "A", "").unwrap();
    let b = vault.create_note("", "B", "").unwrap();
    vault.trash_note(a).unwrap();
    vault.delete_note(a).unwrap();
    assert!(trashed(&vault).is_empty());
    assert!(matches!(vault.restore_note(a), Err(Error::NoteNotFound)));

    // Delete forever on a live note skips the trash.
    vault.delete_note(b).unwrap();
    assert!(trashed(&vault).is_empty());
    assert!(titles(&vault, SearchQuery::default()).is_empty());
    assert!(matches!(vault.delete_note(b), Err(Error::NoteNotFound)));
}

#[test]
fn empty_trash_deletes_only_trashed_notes() {
    let (_dir, mut vault) = common::vault();
    for t in ["A", "B"] {
        let id = vault.create_note("", t, "").unwrap();
        vault.trash_note(id).unwrap();
    }
    vault.create_note("", "Live", "").unwrap();
    assert_eq!(vault.empty_trash().unwrap(), 2);
    assert!(trashed(&vault).is_empty());
    assert_eq!(titles(&vault, SearchQuery::default()), ["Live"]);
    assert_eq!(vault.empty_trash().unwrap(), 0);
}

#[test]
fn auto_purge_after_the_retention_period() {
    let (_dir, mut vault) = common::vault();
    assert_eq!(vault.trash_days().unwrap(), DEFAULT_TRASH_DAYS);
    let id = vault.create_note("", "Old", "").unwrap();
    vault.trash_note(id).unwrap();
    let deleted_at = vault.trashed_notes().unwrap()[0].deleted_at;

    let days = i64::from(DEFAULT_TRASH_DAYS);
    assert_eq!(vault.purge_trash_at(deleted_at + days * DAY_MS - 1).unwrap(), 0);
    assert_eq!(trashed(&vault), ["Old"]);
    assert_eq!(vault.purge_trash_at(deleted_at + days * DAY_MS).unwrap(), 1);
    assert!(trashed(&vault).is_empty());

    vault.set_trash_days(2).unwrap();
    assert_eq!(vault.trash_days().unwrap(), 2);
    assert!(matches!(vault.set_trash_days(100_000), Err(Error::InvalidTrashDays)));
    let id = vault.create_note("", "Newer", "").unwrap();
    vault.trash_note(id).unwrap();
    let deleted_at = vault.trashed_notes().unwrap()[0].deleted_at;
    assert_eq!(vault.purge_trash_at(deleted_at + 2 * DAY_MS).unwrap(), 1);
}

#[test]
fn auto_purge_zero_means_never() {
    let (_dir, mut vault) = common::vault();
    vault.set_trash_days(0).unwrap();
    let id = vault.create_note("", "Keep", "").unwrap();
    vault.trash_note(id).unwrap();
    assert_eq!(vault.purge_trash_at(i64::MAX / 2).unwrap(), 0);
    assert_eq!(trashed(&vault), ["Keep"]);
}

#[test]
fn trashed_notes_are_left_out_of_folder_counts_and_the_tree() {
    let (_dir, mut vault) = common::vault();
    let id = vault.create_note("OldVault/a", "T", "").unwrap();
    let live = vault.create_note("OldVault/a", "Live", "").unwrap();
    vault.trash_note(id).unwrap();
    assert_eq!(vault.count_in_folder("OldVault").unwrap(), 1);
    assert_eq!(vault.count_in_folder("").unwrap(), 1);
    let tree: Vec<i64> = vault.folder_notes("OldVault/a", 0, 500).unwrap().iter().map(|n| n.id).collect();
    assert_eq!(tree, [live]);
}
