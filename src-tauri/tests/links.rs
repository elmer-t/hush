mod common;

use hush_lib::db::Resolution;

#[test]
fn resolution_by_title_and_path() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("archive/2019", "Index", "").unwrap();
    let b = vault.create_note("misc", "Index", "").unwrap();
    let u = vault.create_note("misc", "Unique", "").unwrap();

    assert_eq!(vault.resolve_link("unique").unwrap(), Resolution::Unique(u));
    assert_eq!(vault.resolve_link("Index").unwrap(), Resolution::Ambiguous(vec![a, b]));
    assert_eq!(vault.resolve_link("2019/Index").unwrap(), Resolution::Unique(a));
    assert_eq!(vault.resolve_link("archive/2019/Index.md").unwrap(), Resolution::Unique(a));
    assert_eq!(vault.resolve_link("misc/Index").unwrap(), Resolution::Unique(b));
    assert_eq!(vault.resolve_link("nowhere/Index").unwrap(), Resolution::Missing);
    assert_eq!(vault.resolve_link("Nope").unwrap(), Resolution::Missing);
    assert_eq!(vault.link_text(a).unwrap(), "2019/Index");
    assert_eq!(vault.link_text(u).unwrap(), "Unique");
}

#[test]
fn exact_folder_wins_over_suffix() {
    let (_dir, mut vault) = common::vault();
    let root = vault.create_note("", "T", "").unwrap();
    let a = vault.create_note("a", "T", "").unwrap();
    let xa = vault.create_note("x/a", "T", "").unwrap();
    assert_eq!(vault.resolve_link("T").unwrap(), Resolution::Unique(root));
    assert_eq!(vault.resolve_link("a/T").unwrap(), Resolution::Unique(a));
    assert_eq!(vault.link_text(xa).unwrap(), "x/a/T");
}

#[test]
fn backlinks_and_outgoing() {
    let (_dir, mut vault) = common::vault();
    let target = vault.create_note("p", "Target", "").unwrap();
    let s1 = vault.create_note("", "One", "see [[Target]] and [[target#h|again]]").unwrap();
    let s2 = vault.create_note("", "Two", "[[p/Target]] [[Missing]]").unwrap();
    vault.create_note("", "Three", "`[[Target]]` in code").unwrap();

    let back: Vec<_> = vault.backlinks(target).unwrap().into_iter().map(|b| (b.note.id, b.ambiguous)).collect();
    assert_eq!(back, vec![(s1, false), (s2, false)]);

    let out: Vec<_> = vault.outgoing_links(s2).unwrap().into_iter().map(|l| (l.target, l.resolution)).collect();
    assert_eq!(out, vec![("Missing".into(), Resolution::Missing), ("p/Target".into(), Resolution::Unique(target))]);
}

#[test]
fn ambiguous_backlinks_are_flagged() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("a", "Dup", "").unwrap();
    // Imported content can already be ambiguous; creating "b/Dup" first would rewrite it.
    let src = vault.create_note("", "Src", "").unwrap();
    vault.create_note("b", "Dup", "").unwrap();
    vault.update_body(src, "[[Dup]]").unwrap();
    let back = vault.backlinks(a).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].note.id, src);
    assert!(back[0].ambiguous);
}

#[test]
fn rename_rewrites_links_keeping_heading_and_alias() {
    let (_dir, mut vault) = common::vault();
    let target = vault.create_note("", "Old", "").unwrap();
    let src = vault.create_note("", "Src", "a [[Old]] b [[old#Part|label]] c [[Other]]").unwrap();
    vault.rename_note(target, "", "New").unwrap();
    assert_eq!(vault.note(src).unwrap().body, "a [[New]] b [[New#Part|label]] c [[Other]]");
    assert_eq!(vault.backlinks(target).unwrap().len(), 1);
}

#[test]
fn rename_into_collision_uses_shortest_unique_path() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("x", "Name", "").unwrap();
    let target = vault.create_note("y", "Old", "").unwrap();
    let src = vault.create_note("", "Src", "[[Old]]").unwrap();
    vault.rename_note(target, "y", "Name").unwrap();
    assert_eq!(vault.note(src).unwrap().body, "[[y/Name]]");
    assert_eq!(vault.resolve_link("y/Name").unwrap(), Resolution::Unique(target));
}

#[test]
fn links_to_bystanders_survive_a_new_ambiguity() {
    let (_dir, mut vault) = common::vault();
    let bystander = vault.create_note("x", "Name", "").unwrap();
    let src = vault.create_note("", "Src", "[[Name]]").unwrap();
    // A new note with the same title would make [[Name]] ambiguous.
    vault.create_note("y", "Name", "").unwrap();
    assert_eq!(vault.note(src).unwrap().body, "[[x/Name]]");
    assert_eq!(vault.backlinks(bystander).unwrap()[0].note.id, src);
}

#[test]
fn move_rewrites_path_links() {
    let (_dir, mut vault) = common::vault();
    vault.create_note("a", "Dup", "").unwrap();
    let moved = vault.create_note("b", "Dup", "").unwrap();
    let src = vault.create_note("", "Src", "[[b/Dup]]").unwrap();
    vault.rename_note(moved, "c/d", "Dup").unwrap();
    assert_eq!(vault.note(src).unwrap().body, "[[d/Dup]]");
}

#[test]
fn self_links_follow_rename() {
    let (_dir, mut vault) = common::vault();
    let id = vault.create_note("", "Me", "I am [[Me]]").unwrap();
    vault.rename_note(id, "", "Myself").unwrap();
    assert_eq!(vault.note(id).unwrap().body, "I am [[Myself]]");
}
