mod common;

use std::time::Instant;

use hush_lib::db::{DEFAULT_TRASH_DAYS, Error, FolderContents, NewNote, Resolution, SearchQuery, TrashEntry, Vault};

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

fn live_titles(vault: &Vault) -> Vec<String> {
    let mut t = titles(vault, SearchQuery::default());
    t.sort();
    t
}

fn entries(vault: &Vault) -> Vec<TrashEntry> {
    vault.trash_entries(0, 100).unwrap()
}

fn group(vault: &Vault) -> i64 {
    match entries(vault).into_iter().find(|e| matches!(e, TrashEntry::Folder { .. })) {
        Some(TrashEntry::Folder { deleted_at, .. }) => deleted_at,
        _ => panic!("no folder entry"),
    }
}

#[test]
fn trash_folder_takes_everything_below_it_and_nothing_beside_it() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("Projects", "Top", "").unwrap();
    vault.create_note("Projects/a", "A", "").unwrap();
    vault.create_note("projects/A/deep/er/still", "Deep", "").unwrap();
    vault.create_note("Projectsx", "Beside", "").unwrap();
    vault.create_note("Projects-old", "Dash", "").unwrap();
    vault.create_note("Other/Projects", "Nested name", "").unwrap();
    vault.create_note("", "Root", "").unwrap();

    assert_eq!(vault.folder_contents("PROJECTS").unwrap(), FolderContents { notes: 3, subfolders: 4 });
    assert_eq!(vault.trash_folder("projects").unwrap(), 3);
    assert_eq!(live_titles(&vault), ["Beside", "Dash", "Nested name", "Root"]);
    assert!(!vault.folders().unwrap().iter().any(|f| f.to_lowercase().starts_with("projects/") || f == "Projects"));
    let e = entries(&vault);
    assert_eq!(e.len(), 1, "one entry for the folder: {e:?}");
    assert!(matches!(&e[0], TrashEntry::Folder { folder, notes: 3, .. } if folder == "projects"));
    assert_eq!(vault.trash_count().unwrap(), 3);
}

#[test]
fn trash_folder_leaves_a_sibling_with_a_longer_name_alone() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("a/b", "In", "").unwrap();
    vault.create_note("a/b/c", "Below", "").unwrap();
    vault.create_note("a/bc", "Sibling", "").unwrap();
    assert_eq!(vault.folder_contents("a/b").unwrap(), FolderContents { notes: 2, subfolders: 1 });
    assert_eq!(vault.trash_folder("a/b").unwrap(), 2);
    assert_eq!(live_titles(&vault), ["Sibling"]);
}

#[test]
fn trash_folder_hides_tags_attributes_and_links() {
    let (_dir, mut vault) = common::vault();
    let target = vault.create_note("", "Target", "").unwrap();
    vault.create_note("f", "In", "---\nmood: calm\n---\n#bird [[Target]]").unwrap();
    vault.trash_folder("f").unwrap();
    assert!(vault.backlinks(target).unwrap().is_empty());
    let facets = vault.facets().unwrap();
    assert!(facets.tags.is_empty() && facets.attribute_keys.is_empty());
}

#[test]
fn an_unknown_or_empty_folder_changes_nothing() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("a", "A", "").unwrap();
    assert_eq!(vault.folder_contents("nope").unwrap(), FolderContents { notes: 0, subfolders: 0 });
    assert_eq!(vault.trash_folder("nope").unwrap(), 0);
    assert!(entries(&vault).is_empty());
    assert!(matches!(vault.trash_folder(""), Err(Error::InvalidFolder(_))));
    assert!(matches!(vault.trash_folder("/"), Err(Error::InvalidFolder(_))));
    assert!(matches!(vault.folder_contents(""), Err(Error::InvalidFolder(_))));
    assert_eq!(live_titles(&vault), ["A"]);
}

#[test]
fn notes_already_in_the_trash_stay_separate_entries() {
    let (_dir, mut vault) = common::vault();
    let early = vault.create_note("f", "Early", "").unwrap();
    vault.create_note("f", "Live", "").unwrap();
    vault.trash_note(early).unwrap();
    vault.trash_folder("f").unwrap();
    let e = entries(&vault);
    assert_eq!(e.len(), 2);
    assert!(matches!(&e[0], TrashEntry::Folder { notes: 1, .. }), "newest first: {e:?}");
    assert!(matches!(&e[1], TrashEntry::Note(n) if n.id == early));

    vault.restore_folder(group(&vault)).unwrap();
    assert_eq!(live_titles(&vault), ["Live"], "the earlier note stays in the trash");
    assert_eq!(trashed(&vault), ["Early"]);
}

#[test]
fn restore_folder_brings_the_whole_group_back() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("p", "A", "#bird").unwrap();
    let b = vault.create_note("p/q", "B", "[[A]]").unwrap();
    vault.trash_folder("p").unwrap();
    assert_eq!(vault.restore_folder(group(&vault)).unwrap(), 2);
    assert!(entries(&vault).is_empty());
    assert_eq!((vault.note(a).unwrap().folder, vault.note(b).unwrap().folder), ("p".into(), "p/q".into()));
    assert_eq!(vault.note(a).unwrap().tags, ["bird"], "derived again");
    assert_eq!(vault.backlinks(a).unwrap().len(), 1);
    assert!(matches!(vault.restore_folder(1), Err(Error::NoteNotFound)));
}

#[test]
fn restore_folder_renames_clashing_notes() {
    let (_dir, mut vault) = common::vault();
    let plan = vault.create_note("p", "Plan", "old").unwrap();
    let other = vault.create_note("p", "Other", "").unwrap();
    vault.trash_folder("p").unwrap();
    let taken = vault.create_note("P", "plan", "new").unwrap();
    vault.create_note("p", "Plan (restored)", "").unwrap();
    let source = vault.create_note("", "Source", "see [[Plan]]").unwrap();

    vault.restore_folder(group(&vault)).unwrap();
    assert_eq!(vault.note(plan).unwrap().title, "Plan (restored 2)");
    assert_eq!(vault.note(other).unwrap().title, "Other");
    assert_eq!(vault.note(taken).unwrap().title, "plan");
    // The link meant the live note; it still does.
    assert_eq!(vault.resolve_link("Plan").unwrap(), Resolution::Unique(taken));
    assert_eq!(vault.note(source).unwrap().body, "see [[Plan]]");
}

#[test]
fn restore_folder_preserves_links_like_create() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("old", "Index", "").unwrap();
    vault.trash_folder("old").unwrap();
    let live = vault.create_note("new", "Index", "").unwrap();
    let source = vault.create_note("", "Source", "see [[Index|the index]]").unwrap();
    vault.restore_folder(group(&vault)).unwrap();
    assert_eq!(vault.note(source).unwrap().body, "see [[new/Index|the index]]");
    assert_eq!(vault.resolve_link("new/Index").unwrap(), Resolution::Unique(live));
}

#[test]
fn delete_forever_removes_the_whole_group() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("p", "A", "").unwrap();
    vault.create_note("p/q", "B", "").unwrap();
    let loose = vault.create_note("", "Loose", "").unwrap();
    vault.trash_note(loose).unwrap();
    vault.trash_folder("p").unwrap();
    assert_eq!(vault.delete_trash_folder(group(&vault)).unwrap(), 2);
    assert_eq!(trashed(&vault), ["Loose"]);
    assert_eq!(entries(&vault).len(), 1);
    assert_eq!(vault.trash_count().unwrap(), 1);
}

#[test]
fn purge_removes_the_group_after_the_retention_period() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("p", "A", "").unwrap();
    vault.create_note("p/q", "B", "").unwrap();
    vault.trash_folder("p").unwrap();
    let at = group(&vault);
    let days = i64::from(DEFAULT_TRASH_DAYS);
    assert_eq!(vault.purge_trash_at(at + days * DAY_MS - 1).unwrap(), 0);
    assert_eq!(entries(&vault).len(), 1);
    assert_eq!(vault.purge_trash_at(at + days * DAY_MS).unwrap(), 2);
    assert!(entries(&vault).is_empty());
}

#[test]
fn empty_trash_and_deleting_every_note_drop_the_group() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("p", "A", "").unwrap();
    vault.trash_folder("p").unwrap();
    assert_eq!(vault.empty_trash().unwrap(), 1);
    assert!(entries(&vault).is_empty());

    let b = vault.create_note("p", "B", "").unwrap();
    vault.trash_folder("p").unwrap();
    vault.delete_note(b).unwrap();
    assert!(entries(&vault).is_empty(), "an empty group isn't listed");
}

#[test]
fn trash_entries_are_paged() {
    let (_dir, mut vault) = common::vault();
    for i in 0..5 {
        let id = vault.create_note("", &format!("N{i}"), "").unwrap();
        vault.trash_note(id).unwrap();
    }
    vault.create_note("f", "In", "").unwrap();
    vault.trash_folder("f").unwrap();
    let first = vault.trash_entries(0, 4).unwrap();
    let rest = vault.trash_entries(4, 4).unwrap();
    assert_eq!((first.len(), rest.len()), (4, 2));
    assert!(matches!(first[0], TrashEntry::Folder { .. }));
    let ids: Vec<i64> = first.iter().chain(&rest).filter_map(|e| if let TrashEntry::Note(n) = e { Some(n.id) } else { None }).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(ids, sorted, "newest first");
}

/// Times for trash, count and restore of a 100k-note folder (issue #54). Ignored, so it
/// only runs on request:
///
/// ```text
/// cargo test --release --manifest-path src-tauri/Cargo.toml --test trash -- --ignored --nocapture
/// ```
#[test]
#[ignore = "timing: builds a 100k-note folder"]
fn folder_of_100k_notes_timings() {
    const NOTES: usize = 100_000;
    let (_dir, mut vault) = common::vault();
    let notes: Vec<(String, String, String)> = (0..NOTES)
        .map(|i| {
            let folder = format!("Big/{}/{}", i % 50, i % 7);
            (folder, format!("Note {i}"), format!("note {i} #t{} [[Note {}]] text", i % 20, (i + 1) % NOTES))
        })
        .collect();
    for chunk in notes.chunks(5_000) {
        let batch: Vec<NewNote> =
            chunk.iter().map(|(f, t, b)| NewNote { folder: f, title: t, body: b, created_at: 1, updated_at: 1 }).collect();
        vault.import_notes(&batch).unwrap();
    }
    for i in 0..10_000 {
        vault.import_notes(&[NewNote { folder: &format!("Other/{}", i % 100), title: &format!("O{i}"), body: "x", created_at: 1, updated_at: 1 }]).unwrap();
    }

    let t = Instant::now();
    let contents = vault.folder_contents("Big").unwrap();
    let count = t.elapsed();
    assert_eq!(contents.notes, NOTES);
    let t = Instant::now();
    assert_eq!(vault.trash_folder("Big").unwrap(), NOTES);
    let trash = t.elapsed();
    let t = Instant::now();
    let page = vault.trash_entries(0, 100).unwrap();
    let list = t.elapsed();
    assert_eq!(page.len(), 1);
    let t = Instant::now();
    assert_eq!(vault.restore_folder(group(&vault)).unwrap(), NOTES);
    let restore = t.elapsed();
    println!("100k-note folder: count {count:?} ({} subfolders), trash {trash:?}, trash list page {list:?}, restore {restore:?}", contents.subfolders);
}
