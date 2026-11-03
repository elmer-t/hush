mod common;

use hush_lib::db::{Error, Resolution};

#[test]
fn create_derives_metadata_from_body() {
    let (_dir, mut vault) = common::vault();
    let id = vault
        .create_note(
            "/journal/2019/",
            "March 4",
            "---\nauthor: Ada\ntopic: Life\ncreated: 2019-03-04\ntags: [jr]\n---\nToday #dream",
        )
        .unwrap();
    let note = vault.note(id).unwrap();
    assert_eq!(note.folder, "journal/2019");
    assert_eq!(note.author.as_deref(), Some("Ada"));
    assert_eq!(note.topic.as_deref(), Some("Life"));
    assert_eq!(note.tags, vec!["dream", "jr"]);
    assert_eq!(
        note.created_at,
        hush_lib::markdown::parse_date("2019-03-04").unwrap()
    );
}

#[test]
fn update_rederives_metadata() {
    let (_dir, mut vault) = common::vault();
    let id = vault.create_note("", "N", "#old").unwrap();
    let created = vault.note(id).unwrap().created_at;
    vault.update_body(id, "#new [[Other]]").unwrap();
    let note = vault.note(id).unwrap();
    assert_eq!(note.tags, vec!["new"]);
    assert_eq!(note.created_at, created);
    assert_eq!(
        vault.outgoing_links(id).unwrap()[0].resolution,
        Resolution::Missing
    );
}

#[test]
fn titles_are_unique_per_folder_case_insensitive() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("a", "Note", "").unwrap();
    assert!(matches!(
        vault.create_note("A", "note", ""),
        Err(Error::TitleTaken)
    ));
    vault.create_note("b", "Note", "").unwrap();
    assert_eq!(vault.folders().unwrap(), vec!["a", "b"]);
}

#[test]
fn untitled_notes_get_the_first_free_title() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_untitled("x", "").unwrap();
    let b = vault.create_untitled("x", "").unwrap();
    let c = vault.create_untitled("y", "").unwrap();
    vault.delete_note(a).unwrap();
    let d = vault.create_untitled("x", "").unwrap();
    let titles: Vec<String> = [b, c, d]
        .iter()
        .map(|id| vault.note(*id).unwrap().title)
        .collect();
    assert_eq!(titles, vec!["Untitled 2", "Untitled", "Untitled"]);
}

#[test]
fn invalid_titles_are_rejected() {
    let (_dir, mut vault) = common::vault();
    for bad in ["", "  ", "a/b", "a|b", "a#b", "[[x]]"] {
        assert!(
            matches!(vault.create_note("", bad, ""), Err(Error::InvalidTitle(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn delete() {
    let (_dir, mut vault) = common::vault();
    let target = vault.create_note("", "Target", "").unwrap();
    let source = vault.create_note("", "Source", "[[Target]]").unwrap();
    vault.delete_note(target).unwrap();
    assert!(matches!(vault.note(target), Err(Error::NoteNotFound)));
    assert!(matches!(
        vault.delete_note(target),
        Err(Error::NoteNotFound)
    ));
    assert_eq!(
        vault.outgoing_links(source).unwrap()[0].resolution,
        Resolution::Missing
    );
}

#[test]
fn folder_notes_are_the_folders_own_notes_only() {
    let (_dir, mut vault) = common::vault();
    let a2 = vault.create_note("a", "beta", "").unwrap();
    let a1 = vault.create_note("a", "Alpha", "").unwrap();
    vault.create_note("a/b", "Nested", "").unwrap();
    vault.create_note("ab", "Sibling", "").unwrap();
    let root = vault.create_note("", "Root", "").unwrap();

    let ids = |folder: &str| -> Vec<i64> {
        vault
            .folder_notes(folder, 0, 500)
            .unwrap()
            .iter()
            .map(|n| n.id)
            .collect()
    };
    assert_eq!(
        ids("a"),
        [a1, a2],
        "not a/b or ab; sorted by title ignoring case"
    );
    assert_eq!(
        ids("A/"),
        [a1, a2],
        "the folder is normalised and case-insensitive"
    );
    assert_eq!(ids(""), [root], "only root notes");
    assert!(ids("missing").is_empty());

    let page = |offset| -> Vec<i64> {
        vault
            .folder_notes("a", offset, 1)
            .unwrap()
            .iter()
            .map(|n| n.id)
            .collect()
    };
    assert_eq!(page(0), [a1]);
    assert_eq!(page(1), [a2]);
    assert!(page(2).is_empty());
}

fn folders_of(vault: &hush_lib::db::Vault, ids: &[i64]) -> Vec<String> {
    ids.iter()
        .map(|id| vault.note(*id).unwrap().folder)
        .collect()
}

#[test]
fn move_notes_clash_with_an_existing_note_moves_nothing() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("a", "Ideas", "").unwrap();
    let b = vault.create_note("b", "Plans", "").unwrap();
    vault.create_note("target", "ideas", "").unwrap();
    let err = vault.move_notes(&[b, a], "target").unwrap_err();
    assert!(
        matches!(&err, Error::MoveClash(t) if t == &["Ideas"]),
        "{err}"
    );
    assert!(err.to_string().contains("“Ideas”"));
    assert_eq!(folders_of(&vault, &[a, b]), ["a", "b"]);
}

#[test]
fn move_notes_clash_inside_the_batch_moves_nothing() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("a", "Ideas", "").unwrap();
    let b = vault.create_note("b", "IDEAS", "").unwrap();
    let c = vault.create_note("c", "Other", "").unwrap();
    let err = vault.move_notes(&[a, b, c], "target").unwrap_err();
    assert!(
        matches!(&err, Error::MoveClash(t) if t == &["IDEAS"]),
        "{err}"
    );
    assert_eq!(folders_of(&vault, &[a, b, c]), ["a", "b", "c"]);
}

#[test]
fn move_notes_already_in_the_target_are_no_ops() {
    let (_dir, mut vault) = common::vault();
    let here = vault.create_note("Target", "Here", "").unwrap();
    let there = vault.create_note("elsewhere", "There", "").unwrap();
    let before = vault.note(here).unwrap().updated_at;
    vault.move_notes(&[here, there, there], "/target/").unwrap();
    assert_eq!(folders_of(&vault, &[here, there]), ["Target", "target"]);
    assert_eq!(vault.note(here).unwrap().updated_at, before, "not touched");
    vault.move_notes(&[], "x").unwrap();
}

#[test]
fn move_notes_unknown_id_moves_nothing() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("a", "A", "").unwrap();
    assert!(matches!(
        vault.move_notes(&[a, 9999], "b"),
        Err(Error::NoteNotFound)
    ));
    assert_eq!(folders_of(&vault, &[a]), ["a"]);
}

#[test]
fn move_folder_moves_everything_below_it_and_keeps_the_structure() {
    let (_dir, mut vault) = common::vault();
    let own = vault.create_note("a/b", "Own", "").unwrap();
    let deep = vault.create_note("A/b/x/y", "Deep", "").unwrap();
    let parent = vault.create_note("a", "Parent", "").unwrap();
    let sibling = vault.create_note("a/bc", "Sibling", "").unwrap();
    let trashed = vault.create_note("a/b", "Trashed", "").unwrap();
    vault.trash_note(trashed).unwrap();

    assert_eq!(
        vault.move_folder("/A/B/", "c").unwrap(),
        "c/B",
        "named as passed"
    );
    assert_eq!(
        folders_of(&vault, &[own, deep, parent, sibling]),
        ["c/B", "c/B/x/y", "a", "a/bc"]
    );
    assert_eq!(vault.folders().unwrap(), ["a", "a/bc", "c/B", "c/B/x/y"]);

    assert_eq!(
        vault.move_folder("c/b", "").unwrap(),
        "b",
        "to the top level"
    );
    assert_eq!(folders_of(&vault, &[own, deep]), ["b", "b/x/y"]);
    assert_eq!(
        vault.move_folder("missing", "c").unwrap(),
        "c/missing",
        "a folder without notes moves nothing"
    );
}

#[test]
fn move_folder_into_itself_or_its_parent_is_refused_or_a_no_op() {
    let (_dir, mut vault) = common::vault();
    let n = vault.create_note("a/b", "N", "").unwrap();
    let before = vault.note(n).unwrap().updated_at;
    for (from, to) in [("a/b", "a/b"), ("a/b", "A/B/c"), ("", "x")] {
        assert!(
            matches!(
                vault.move_folder(from, to),
                Err(Error::InvalidFolderMove(_))
            ),
            "{from:?} -> {to:?}"
        );
    }
    assert_eq!(vault.move_folder("a/b", "A").unwrap(), "a/b");
    assert_eq!(folders_of(&vault, &[n]), ["a/b"]);
    assert_eq!(vault.note(n).unwrap().updated_at, before, "not touched");
}

#[test]
fn move_folder_clash_moves_nothing() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("a/b", "Ideas", "").unwrap();
    let b = vault.create_note("a/b/x", "Plans", "").unwrap();
    let kept = vault.create_note("c/b/x", "plans", "").unwrap();
    let err = vault.move_folder("a/b", "c").unwrap_err();
    assert!(
        matches!(&err, Error::MoveClash(t) if t == &["Plans"]),
        "{err}"
    );
    assert_eq!(folders_of(&vault, &[a, b, kept]), ["a/b", "a/b/x", "c/b/x"]);
}

#[test]
fn move_folder_merges_into_an_existing_folder_and_may_reuse_its_own_paths() {
    let (_dir, mut vault) = common::vault();
    let there = vault.create_note("c/b", "There", "").unwrap();
    let here = vault.create_note("a/b", "Here", "").unwrap();
    vault.move_folder("a/b", "c").unwrap();
    assert_eq!(folders_of(&vault, &[there, here]), ["c/b", "c/b"]);

    // `b/b` to the top level is `b`: `b/b/b/N` takes `b/b/N`'s old path as that note leaves it.
    let outer = vault.create_note("b/b", "N", "").unwrap();
    let inner = vault.create_note("b/b/b", "N", "").unwrap();
    assert_eq!(vault.move_folder("b/b", "").unwrap(), "b");
    assert_eq!(folders_of(&vault, &[outer, inner]), ["b", "b/b"]);
}

#[test]
fn bulk_delete_moves_all_to_the_trash_or_nothing() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("", "A", "").unwrap();
    let b = vault.create_note("x", "B", "").unwrap();
    let c = vault.create_note("", "C", "").unwrap();

    assert!(matches!(
        vault.trash_notes(&[a, 9999]),
        Err(Error::NoteNotFound)
    ));
    assert!(
        vault.trashed_notes().unwrap().is_empty(),
        "an unknown id deletes nothing"
    );
    vault.note(a).unwrap();

    vault.trash_notes(&[a, b]).unwrap();
    let mut trashed: Vec<i64> = vault
        .trashed_notes()
        .unwrap()
        .iter()
        .map(|n| n.id)
        .collect();
    trashed.sort();
    assert_eq!(trashed, [a, b]);
    assert!(matches!(vault.note(a), Err(Error::NoteNotFound)));
    vault.note(c).unwrap();
}
