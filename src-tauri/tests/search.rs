mod common;

use hush_lib::db::{DateField, Facets, NewNote, SearchQuery, Vault};
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
    assert_eq!(q(|q| q.tags = vec!["jr".into(), "dr".into()]), vec!["Dream of the sea"]);
    assert_eq!(q(|q| q.tags = vec!["JR".into()]), Vec::<String>::new(), "tags are case-sensitive");
    assert_eq!(q(|q| q.tags = vec!["Jr".into()]), Vec::<String>::new(), "also for nested tags");
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

#[test]
fn short_prefixes_use_the_prefix_index_and_one_letter_is_a_whole_word() {
    let (_dir, mut vault) = seeded();
    vault.create_note("", "Letters", "a b c").unwrap();
    // Two and three characters are prefixes, as before.
    assert_eq!(titles(&vault, &text("li")), vec!["Lighthouse", "Dream of the sea"]);
    assert_eq!(titles(&vault, &text("lig")), vec!["Lighthouse", "Dream of the sea"]);
    assert_eq!(titles(&vault, &text("bu")), vec!["Meeting"]);
    // One letter matches only the word itself, not every word starting with it.
    assert_eq!(titles(&vault, &text("l")), Vec::<String>::new());
    assert_eq!(titles(&vault, &text("b")), vec!["Letters"]);
    assert_eq!(titles(&vault, &text("walked t")), Vec::<String>::new());
    assert_eq!(titles(&vault, &text("walked th")), vec!["Lighthouse"]);
}

#[test]
fn a_very_broad_query_lists_the_newest_matches() {
    use hush_lib::db::{NewNote, RANK_LIMIT};
    let (_dir, mut vault) = common::vault();
    let titles_owned: Vec<String> = (0..=RANK_LIMIT).map(|i| format!("Note {i}")).collect();
    let notes: Vec<NewNote> = titles_owned
        .iter()
        .map(|t| NewNote { folder: "", title: t, body: "common words here", created_at: 1, updated_at: 1 })
        .collect();
    vault.import_notes(&notes).unwrap();
    let rare = vault.create_note("", "Rare", "common and rare words").unwrap();

    let page = vault.search_page(&SearchQuery { text: "common".into(), limit: Some(3), ..Default::default() }).unwrap();
    assert!(page.newest_first);
    let ids: Vec<i64> = page.hits.iter().map(|h| h.note.id).collect();
    assert_eq!(ids, [rare, rare - 1, rare - 2], "newest (highest id) first");
    assert!(page.hits[0].snippet.as_deref().unwrap().contains("\u{2}common\u{3}"));
    // Next page continues in the same order; trashed notes stay out.
    vault.trash_note(rare - 3).unwrap();
    let next = vault
        .search_page(&SearchQuery { text: "common".into(), limit: Some(2), offset: 3, ..Default::default() })
        .unwrap();
    assert_eq!(next.hits.iter().map(|h| h.note.id).collect::<Vec<_>>(), [rare - 4, rare - 5]);

    // Narrow again: ranked.
    let page = vault.search_page(&text("rare")).unwrap();
    assert!(!page.newest_first);
    assert_eq!(page.hits.len(), 1);
}

/// A search still waiting for the vault when a newer one starts doesn't run at all.
#[test]
fn a_search_superseded_while_waiting_returns_nothing() {
    use hush_lib::session::Session;
    let (dir, vault) = seeded();
    let session = Session::default();
    session.unlock(vault, dir.path().join("vault.hush")).unwrap();
    let (older, newer) = std::thread::scope(|s| {
        // Hold the vault (as a save would) while two searches queue up behind it.
        session
            .with_vault(|_| {
                let older = s.spawn(|| session.search(|v, stop| v.search_page_until(&text("lighthouse"), stop)).unwrap());
                std::thread::sleep(std::time::Duration::from_millis(50));
                let newer = s.spawn(|| session.search(|v, stop| v.search_page_until(&text("budget"), stop)).unwrap());
                std::thread::sleep(std::time::Duration::from_millis(50));
                Ok((older, newer))
            })
            .map(|(o, n)| (o.join().unwrap(), n.join().unwrap()))
            .unwrap()
    });
    assert!(older.is_none(), "cancelled, not an error");
    assert_eq!(newer.unwrap().hits.len(), 1);
}

/// A newer search interrupts one still running on the vault; the older one returns
/// nothing instead of an error.
#[test]
fn a_newer_search_interrupts_a_running_one() {
    use hush_lib::db::NewNote;
    use hush_lib::session::Session;
    let (dir, mut vault) = common::vault();
    // Enough text that ranking a broad prefix takes a while (debug build).
    let body: String = (0..300).map(|i| format!("word{} ", i % 97)).collect();
    let titles: Vec<String> = (0..5_000).map(|i| format!("N{i}")).collect();
    let notes: Vec<NewNote> =
        titles.iter().map(|t| NewNote { folder: "", title: t, body: &body, created_at: 1, updated_at: 1 }).collect();
    vault.import_notes(&notes).unwrap();
    let session = Session::default();
    session.unlock(vault, dir.path().join("vault.hush")).unwrap();

    let mut interrupted = false;
    for _ in 0..5 {
        let (older, newer) = std::thread::scope(|s| {
            let older = s.spawn(|| session.search(|v, stop| v.search_page_until(&text("wo"), stop)));
            while !session.search_running() && !older.is_finished() {
                std::thread::yield_now();
            }
            let newer = session.search(|v, stop| v.search_page_until(&text("word5"), stop));
            (older.join().unwrap(), newer)
        });
        assert!(older.is_ok(), "an interrupted search is not an error");
        assert_eq!(newer.unwrap().unwrap().hits.len(), 200);
        if older.unwrap().is_none() {
            interrupted = true;
            break;
        }
    }
    assert!(interrupted, "the running search was never interrupted");
    // The connection works normally afterwards.
    assert_eq!(session.with_vault(|v| v.search(&text("word7"))).unwrap().len(), 200);
}

#[test]
fn min_rating_filter() {
    let (_dir, mut vault) = common::vault();
    for (title, rating) in [("One", "1"), ("Three", "3"), ("Four", "4"), ("Five", "5"), ("Half", "4.5")] {
        vault.create_note("", title, &format!("---\nrating: {rating}\n---\nlighthouse")).unwrap();
    }
    vault.create_note("", "Unrated", "lighthouse").unwrap();
    let q = |min: Option<u8>, t: &str| SearchQuery { min_rating: min, text: t.into(), ..Default::default() };
    let mut four = titles(&vault, &q(Some(4), ""));
    four.sort();
    assert_eq!(four, ["Five", "Four"]);
    let mut with_text = titles(&vault, &q(Some(4), "lighthouse"));
    with_text.sort();
    assert_eq!(with_text, ["Five", "Four"]);
    assert_eq!(titles(&vault, &q(Some(1), "")).len(), 4, "unrated and invalid never match");
    assert_eq!(titles(&vault, &q(None, "")).len(), 6);
    assert_eq!(titles(&vault, &q(Some(0), "")).len(), 6, "0 is no filter");
    let rated: Vec<Option<u8>> = vault.search(&q(Some(5), "")).unwrap().into_iter().map(|h| h.note.rating).collect();
    assert_eq!(rated, [Some(5)], "summaries carry the rating");
}

/// The "★ 4+" filter alone at 1M notes, 1% of them rated (issue #56). Ignored, so it
/// only runs on request:
///
/// ```text
/// cargo test --release --manifest-path src-tauri/Cargo.toml --test search -- --ignored --nocapture
/// ```
#[test]
#[ignore = "timing: builds a 1M-note vault"]
fn min_rating_filter_at_1m_notes() {
    const NOTES: usize = 1_000_000;
    let (_dir, mut vault) = common::vault();
    let notes: Vec<(String, String, String)> = (0..NOTES)
        .map(|i| {
            let body = if i % 100 == 0 { format!("---\nrating: {}\n---\nnote {i}", i / 100 % 5 + 1) } else { format!("note {i}") };
            (format!("f{}", i % 50_000), format!("Note {i}"), body)
        })
        .collect();
    for chunk in notes.chunks(10_000) {
        let batch: Vec<NewNote> = chunk
            .iter()
            .enumerate()
            .map(|(j, (f, t, b))| NewNote { folder: f, title: t, body: b, created_at: j as i64, updated_at: j as i64 })
            .collect();
        vault.import_notes(&batch).unwrap();
    }
    let q = SearchQuery { min_rating: Some(4), limit: Some(500), ..Default::default() };
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let hits = vault.search(&q).unwrap();
        times.push(t.elapsed());
        assert_eq!(hits.len(), 500);
        assert!(hits.iter().all(|h| h.note.rating >= Some(4)));
    }
    times.sort();
    println!("min_rating = 4 at 1M notes (10k rated, 4k at 4+): first page median {:?}, all {times:?}", times[2]);
}
