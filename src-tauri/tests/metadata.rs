mod common;

use hush_lib::db::{Error, MetadataEdit, SearchQuery, Vault};

fn add(tag: &str) -> MetadataEdit {
    MetadataEdit::AddTag(tag.into())
}

fn body(vault: &Vault, id: i64) -> String {
    vault.note(id).unwrap().body
}

/// So a bump of `updated_at` (unix millis) is visible.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

fn search_ids(vault: &Vault, q: SearchQuery) -> Vec<i64> {
    let mut ids: Vec<i64> = vault.search(&q).unwrap().into_iter().map(|h| h.note.id).collect();
    ids.sort();
    ids
}

#[test]
fn add_tag_creates_frontmatter_or_key_and_skips_notes_that_have_it() {
    let (_dir, mut vault) = common::vault();
    let bare = vault.create_note("", "Bare", "Text").unwrap();
    let no_tags = vault.create_note("", "No tags", "---\nauthor: Ada\n---\nText").unwrap();
    let has_it = vault.create_note("", "Has it", "---\ntags: [x, journal]\n---\nText").unwrap();
    let other_case = vault.create_note("", "Other case", "---\ntags: [Journal]\n---\nText").unwrap();
    let before = vault.note(has_it).unwrap().updated_at;
    tick();

    let report = vault.edit_notes_metadata(&[bare, no_tags, has_it, other_case], &add("#journal")).unwrap();
    assert_eq!(report.changed, vec![bare, no_tags, other_case]);
    assert_eq!(report.unchanged, 1);

    assert_eq!(body(&vault, bare), "---\ntags: [journal]\n---\nText");
    assert_eq!(body(&vault, no_tags), "---\nauthor: Ada\ntags: [journal]\n---\nText");
    assert_eq!(body(&vault, has_it), "---\ntags: [x, journal]\n---\nText");
    // Tags are case-sensitive, so `Journal` doesn't count as already having `journal`.
    assert_eq!(body(&vault, other_case), "---\ntags: [Journal, journal]\n---\nText");
    assert_eq!(vault.note(has_it).unwrap().updated_at, before, "unchanged note not bumped");
    assert!(vault.note(bare).unwrap().updated_at > before, "changed note bumped");
    assert_eq!(vault.note(no_tags).unwrap().tags, vec!["journal"]);

    let q = SearchQuery { tags: vec!["journal".into()], ..Default::default() };
    assert_eq!(search_ids(&vault, q), vec![bare, no_tags, has_it, other_case]);
}

#[test]
fn add_tag_folds_a_singular_tag_key_into_tags() {
    let (_dir, mut vault) = common::vault();
    let id = vault.create_note("", "N", "---\ntag: a\ntopic: T\n---\n").unwrap();
    vault.edit_notes_metadata(&[id], &add("b")).unwrap();
    assert_eq!(body(&vault, id), "---\ntopic: T\ntags: [a, b]\n---\n");
}

#[test]
fn remove_tag_drops_it_from_frontmatter_and_reports_inline_ones() {
    let (_dir, mut vault) = common::vault();
    let other = vault.create_note("", "Other", "---\ntags: [dr, keep]\n---\nText").unwrap();
    let only = vault.create_note("", "Only", "---\nauthor: Ada\ntags: [DR]\n---\nText").unwrap();
    let alone = vault.create_note("", "Alone", "---\ntags: [dr]\n---\nText").unwrap();
    let inline = vault.create_note("", "Inline", "---\ntags: [dr]\n---\nSee #dr").unwrap();
    let inline_only = vault.create_note("", "Inline only", "Just #dr").unwrap();

    let report = vault
        .edit_notes_metadata(&[other, only, alone, inline, inline_only], &MetadataEdit::RemoveTag("dr".into()))
        .unwrap();
    // `DR` is a different tag from `dr`, so `only` keeps it.
    assert_eq!(report.changed, vec![other, alone, inline]);
    assert_eq!(report.unchanged, 2);
    let still: Vec<&str> = report.still_inline.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(still, vec!["Inline", "Inline only"]);

    assert_eq!(body(&vault, other), "---\ntags: [keep]\n---\nText");
    assert_eq!(body(&vault, only), "---\nauthor: Ada\ntags: [DR]\n---\nText", "other case untouched");
    assert_eq!(body(&vault, alone), "Text", "empty frontmatter removed");
    assert_eq!(body(&vault, inline), "See #dr", "inline tag untouched");
    assert_eq!(vault.note(inline).unwrap().tags, vec!["dr"]);

    let q = SearchQuery { tags: vec!["dr".into()], ..Default::default() };
    assert_eq!(search_ids(&vault, q), vec![inline, inline_only]);
}

#[test]
fn set_topic_and_author_replace_or_remove_the_value() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("", "A", "---\ntopic: Old\nauthor: Ada\n---\nText").unwrap();
    let b = vault.create_note("", "B", "Text").unwrap();
    let c = vault.create_note("", "C", "---\ntopic: Life\n---\n").unwrap();

    let report = vault.edit_notes_metadata(&[a, b, c], &MetadataEdit::SetTopic(" Life ".into())).unwrap();
    assert_eq!(report.changed, vec![a, b]);
    assert_eq!(report.unchanged, 1);
    assert_eq!(body(&vault, a), "---\ntopic: Life\nauthor: Ada\n---\nText");
    assert_eq!(body(&vault, b), "---\ntopic: Life\n---\nText");
    let q = SearchQuery { topic: Some("Life".into()), ..Default::default() };
    assert_eq!(search_ids(&vault, q), vec![a, b, c]);

    vault.edit_notes_metadata(&[a, b], &MetadataEdit::SetAuthor("Grace Hopper".into())).unwrap();
    assert_eq!(vault.note(a).unwrap().author.as_deref(), Some("Grace Hopper"));
    let q = SearchQuery { author: Some("Grace Hopper".into()), ..Default::default() };
    assert_eq!(search_ids(&vault, q), vec![a, b]);

    let report = vault.edit_notes_metadata(&[a, b, c], &MetadataEdit::SetAuthor("".into())).unwrap();
    assert_eq!(report.changed, vec![a, b]);
    assert_eq!(body(&vault, a), "---\ntopic: Life\n---\nText");
    assert_eq!(vault.note(a).unwrap().author, None);
    vault.edit_notes_metadata(&[b], &MetadataEdit::SetTopic("".into())).unwrap();
    assert_eq!(body(&vault, b), "Text");
    assert_eq!(vault.note(b).unwrap().topic, None);
}

#[test]
fn an_unknown_id_fails_the_whole_call_and_changes_nothing() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("", "A", "Text").unwrap();
    let trashed = vault.create_note("", "T", "Text").unwrap();
    vault.trash_note(trashed).unwrap();
    for bad in [9999, trashed] {
        assert!(matches!(vault.edit_notes_metadata(&[a, bad], &add("x")), Err(Error::NoteNotFound)));
        assert_eq!(body(&vault, a), "Text");
        assert!(vault.note(a).unwrap().tags.is_empty());
    }
}

#[test]
fn invalid_tags_are_rejected() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("", "A", "Text").unwrap();
    for tag in ["", " # ", "two words", "a,b"] {
        assert!(matches!(vault.edit_notes_metadata(&[a], &add(tag)), Err(Error::InvalidTag(_))), "{tag:?}");
    }
}

#[test]
fn crlf_bodies_keep_crlf() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("", "A", "---\r\ntopic: T\r\n---\r\nLine 1\r\nLine 2").unwrap();
    let b = vault.create_note("", "B", "Line 1\r\nLine 2").unwrap();
    vault.edit_notes_metadata(&[a, b], &add("x")).unwrap();
    assert_eq!(body(&vault, a), "---\r\ntopic: T\r\ntags: [x]\r\n---\r\nLine 1\r\nLine 2");
    assert_eq!(body(&vault, b), "---\r\ntags: [x]\r\n---\r\nLine 1\r\nLine 2");
}

#[test]
fn notes_with_unreadable_frontmatter_are_skipped_and_reported() {
    let (_dir, mut vault) = common::vault();
    let broken = "---\ntags: [a\n---\nText";
    let bad = vault.create_note("", "Bad", broken).unwrap();
    let good = vault.create_note("", "Good", "Text").unwrap();
    let report = vault.edit_notes_metadata(&[bad, good], &add("x")).unwrap();
    assert_eq!(report.changed, vec![good]);
    assert_eq!(report.unreadable.iter().map(|n| n.id).collect::<Vec<_>>(), vec![bad]);
    assert_eq!(body(&vault, bad), broken);
}

#[test]
fn links_are_rederived() {
    let (_dir, mut vault) = common::vault();
    let target = vault.create_note("", "Target", "").unwrap();
    let a = vault.create_note("", "A", "See [[Target]]").unwrap();
    vault.edit_notes_metadata(&[a], &add("x")).unwrap();
    assert_eq!(vault.backlinks(target).unwrap().len(), 1);
}
