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

#[test]
fn move_notes_keeps_links_pointing_at_the_same_notes() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("inbox", "Alpha", "").unwrap();
    let b = vault.create_note("drafts", "Beta", "").unwrap();
    let src = vault.create_note("", "Src", "[[Alpha]] [[drafts/Beta|b]]").unwrap();

    vault.move_notes(&[a, b], "archive/2020").unwrap();
    assert_eq!(vault.note(a).unwrap().folder, "archive/2020");
    assert_eq!(vault.note(b).unwrap().folder, "archive/2020");
    // `[[drafts/Beta]]` no longer matches anything, so it was rewritten; `[[Alpha]]` still resolves.
    assert_eq!(vault.note(src).unwrap().body, "[[Alpha]] [[Beta|b]]");
    assert_eq!(vault.resolve_link("Alpha").unwrap(), Resolution::Unique(a));
    assert_eq!(vault.resolve_link("Beta").unwrap(), Resolution::Unique(b));
}

#[test]
fn move_folder_keeps_links_pointing_at_the_same_notes() {
    let (_dir, mut vault) = common::vault();
    let a = vault.create_note("work/inbox", "Alpha", "").unwrap();
    let b = vault.create_note("work/inbox/old", "Beta", "").unwrap();
    let other = vault.create_note("home/inbox", "Alpha", "").unwrap();
    let src = vault.create_note("", "Src", "[[work/inbox/Alpha]] [[inbox/old/Beta|b]] [[Beta]]").unwrap();

    assert_eq!(vault.move_folder("work/inbox", "archive").unwrap(), "archive/inbox");
    assert_eq!(vault.note(b).unwrap().folder, "archive/inbox/old");
    // `inbox/old/Beta` still matches by suffix and `Beta` by title; the full path changed.
    assert_eq!(vault.note(src).unwrap().body, "[[archive/inbox/Alpha]] [[inbox/old/Beta|b]] [[Beta]]");
    assert_eq!(vault.resolve_link("archive/inbox/Alpha").unwrap(), Resolution::Unique(a));
    assert_eq!(vault.resolve_link("home/inbox/Alpha").unwrap(), Resolution::Unique(other));
}

#[test]
fn move_notes_rewrites_each_changed_link_once_against_the_final_state() {
    let (_dir, mut vault) = common::vault();
    // Bystanders reached through a folder suffix: `[[T/N]]` → q/T/N, `[[T/M]]` → q/T/M.
    let n = vault.create_note("q/T", "N", "").unwrap();
    let m = vault.create_note("q/T", "M", "").unwrap();
    let src = vault.create_note("", "Src", "[[T/N]] and [[T/M#h]]").unwrap();
    let x = vault.create_note("x", "N", "").unwrap();
    let y = vault.create_note("y", "M", "").unwrap();
    let before = vault.note(src).unwrap().updated_at;

    // Moving both into `T` would make `T/N` and `T/M` mean the moved notes (exact folder wins).
    vault.move_notes(&[x, y], "T").unwrap();
    let body = vault.note(src).unwrap().body;
    assert_eq!(body, "[[q/T/N]] and [[q/T/M#h]]", "both links rewritten in one pass");
    assert_eq!(vault.resolve_link("q/T/N").unwrap(), Resolution::Unique(n));
    assert_eq!(vault.resolve_link("q/T/M").unwrap(), Resolution::Unique(m));
    assert!(vault.note(src).unwrap().updated_at >= before);
}
