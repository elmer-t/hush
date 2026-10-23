mod common;

use hush_lib::db::{DateField, Facets, SearchQuery, Vault};
use hush_lib::markdown::parse_date;

fn titles(vault: &Vault, q: &SearchQuery) -> Vec<String> {
    vault.search(q).unwrap().into_iter().map(|h| h.note.title).collect()
}

fn text(t: &str) -> SearchQuery {
    SearchQuery { text: t.into(), ..Default::default() }
}

fn seeded() -> (tempfile::TempDir, Vault) {
    let (dir, mut vault) = common::vault();
    vault
        .create_note(
            "journal/2019",
            "Lighthouse",
            "---\nauthor: Ada\ntopic: Travel\ncreated: 2019-06-01\nmood: calm\n---\nWe walked to the café. #jr",
        )
        .unwrap();
    vault
        .create_note(
            "journal/2020",
            "Dream of the sea",
            "---\nauthor: Bob\ntopic: Dreams\ncreated: 2020-01-15\n---\nA lighthouse in the fog. #dr #jr/night",
        )
        .unwrap();
    vault.create_note("work", "Meeting", "---\ncreated: 2021-02-02\n---\nBudget review").unwrap();
    (dir, vault)
}

#[test]
fn text_search_ranks_title_matches_first_with_snippets() {
    let (_dir, vault) = seeded();
    let hits = vault.search(&text("lighthouse")).unwrap();
    assert_eq!(hits.iter().map(|h| h.note.title.as_str()).collect::<Vec<_>>(), vec!["Lighthouse", "Dream of the sea"]);
    assert_eq!(hits[0].snippet, None, "title-only match has no body snippet");
    let snippet = hits[1].snippet.as_deref().unwrap();
    assert!(snippet.contains("\u{2}lighthouse\u{3}"), "{snippet}");
}

#[test]
fn prefix_diacritics_and_syntax_safety() {
    let (_dir, vault) = seeded();
    assert_eq!(titles(&vault, &text("ligh")), vec!["Lighthouse", "Dream of the sea"]);
    assert_eq!(titles(&vault, &text("cafe")), vec!["Lighthouse"]);
    assert_eq!(titles(&vault, &text("budget AND")), Vec::<String>::new());
    assert!(vault.search(&text(r#"" ( * NEAR OR"#)).is_ok());
}

#[test]
fn empty_text_lists_newest_first() {
    let (_dir, vault) = seeded();
    assert_eq!(titles(&vault, &text("")), vec!["Meeting", "Dream of the sea", "Lighthouse"]);
}

#[test]
fn filters() {
    let (_dir, vault) = seeded();
    let q = |f: fn(&mut SearchQuery)| {
        let mut q = SearchQuery::default();
        f(&mut q);
        titles(&vault, &q)
    };
    assert_eq!(q(|q| q.tags = vec!["jr".into()]), vec!["Dream of the sea", "Lighthouse"]);
    assert_eq!(q(|q| q.tags = vec!["JR".into(), "dr".into()]), vec!["Dream of the sea"]);
    assert_eq!(q(|q| q.topic = Some("travel".into())), vec!["Lighthouse"]);
    assert_eq!(q(|q| q.author = Some("bob".into())), vec!["Dream of the sea"]);
    assert_eq!(q(|q| q.folder = Some("journal".into())), vec!["Dream of the sea", "Lighthouse"]);
    assert_eq!(q(|q| q.folder = Some("journal/2019/".into())), vec!["Lighthouse"]);
    assert_eq!(q(|q| q.attributes = vec![("Mood".into(), "CALM".into())]), vec!["Lighthouse"]);
    assert_eq!(
        q(|q| {
            q.from = parse_date("2020-01-01");
            q.to = parse_date("2021-01-01");
        }),
        vec!["Dream of the sea"]
    );
    assert_eq!(
        q(|q| {
            q.date_field = DateField::Updated;
            q.to = parse_date("2000-01-01");
        }),
        Vec::<String>::new()
    );
}

#[test]
fn text_and_filters_combine() {
    let (_dir, vault) = seeded();
    let q = SearchQuery { text: "lighthouse".into(), author: Some("Ada".into()), ..Default::default() };
    assert_eq!(titles(&vault, &q), vec!["Lighthouse"]);
}

#[test]
fn search_follows_edits_and_deletes() {
    let (_dir, mut vault) = seeded();
    let id = vault.search(&text("budget")).unwrap()[0].note.id;
    vault.update_body(id, "Forecast").unwrap();
    assert!(titles(&vault, &text("budget")).is_empty());
    assert_eq!(titles(&vault, &text("forecast")), vec!["Meeting"]);
    vault.rename_note(id, "work", "Planning").unwrap();
    assert_eq!(titles(&vault, &text("planning")), vec!["Planning"]);
    vault.delete_note(id).unwrap();
    assert!(titles(&vault, &text("forecast")).is_empty());
}

#[test]
fn limit_and_offset() {
    let (_dir, vault) = seeded();
    let q = SearchQuery { limit: Some(1), offset: 1, ..Default::default() };
    assert_eq!(titles(&vault, &q), vec!["Dream of the sea"]);
}

#[test]
fn facets_list_values_in_use() {
    let (_dir, mut vault) = seeded();
    let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        vault.facets().unwrap(),
        Facets {
            tags: strings(&["dr", "jr", "jr/night"]),
            topics: strings(&["Dreams", "Travel"]),
            authors: strings(&["Ada", "Bob"]),
            attribute_keys: strings(&["mood"]),
        }
    );
    assert_eq!(vault.attribute_values("MOOD").unwrap(), strings(&["calm"]));

    let id = vault.search(&text("dream")).unwrap()[0].note.id;
    vault.delete_note(id).unwrap();
    let facets = vault.facets().unwrap();
    assert_eq!(facets.tags, strings(&["jr"]), "tags of deleted notes are not offered");
    assert_eq!(facets.authors, strings(&["Ada"]));
}
