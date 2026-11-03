mod common;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hush_lib::db::{NewNote, TagFeedback, Vault};
use hush_lib::db::SearchQuery;
use hush_lib::discover::{self, DiscoverError, Discovery, Options, Scope};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::onnx::OnnxEmbedder;
use hush_lib::embed::{self, EmbedError, Embedder, ModelDescriptor};
use hush_lib::import;
use hush_lib::session::Session;
use tempfile::TempDir;

const MODEL: &str = "fake-4";

/// An unlocked session on a fresh vault.
fn session() -> (TempDir, Session, u64) {
    let (dir, vault) = common::vault();
    let session = Session::default();
    let generation = session.unlock(vault, dir.path().join("vault.hush")).unwrap();
    (dir, session, generation)
}

/// Stores `vector` as the note's `model` vector, as the embedding job would.
fn set_vector(v: &mut Vault, model: &str, id: i64, vector: &[f32]) {
    let text = v.embedding_texts(&[id]).unwrap().pop().expect("note has text");
    v.write_embeddings(model, &[(id, text.hash, vector.to_vec())]).unwrap();
}

/// A unit vector near axis `axis` of 4, nudged by `jitter`.
fn near(axis: usize, jitter: f32) -> Vec<f32> {
    let mut v = vec![jitter; 4];
    v[axis] = 1.0;
    embed::normalize(&mut v);
    v
}

fn run(session: &Session, generation: u64, options: Options) -> Discovery {
    discover::run(session, generation, &options, &|| false).unwrap()
}

/// Ids of each group, sorted, groups sorted by their first id.
fn partition(d: &Discovery) -> Vec<Vec<i64>> {
    let mut groups: Vec<Vec<i64>> = d
        .groups
        .iter()
        .map(|g| {
            let mut ids: Vec<i64> = g.notes.iter().map(|n| n.id).collect();
            ids.sort();
            assert_eq!(ids, g.members, "every member is listed in a group this small");
            assert_eq!(g.size, ids.len());
            ids
        })
        .collect();
    groups.sort();
    groups
}

/// Three themes, three notes each, with vectors near three axes.
fn themed_notes(session: &Session) -> [Vec<i64>; 3] {
    let themes = [
        ("ferry", "The ferry left the harbour late; ferry crossing was rough."),
        ("garden", "Planted tomatoes in the garden; the garden needs water."),
        ("budget", "Budget meeting about next year's budget and costs."),
    ];
    session
        .with_vault(|v| {
            let mut out: [Vec<i64>; 3] = Default::default();
            for (axis, (word, body)) in themes.iter().enumerate() {
                for i in 0..3 {
                    let id = v.create_note(word, &format!("{word} {i}"), &format!("{body} Today was fine."))?;
                    set_vector(v, MODEL, id, &near(axis, 0.05 * i as f32));
                    out[axis].push(id);
                }
            }
            v.finish_embedding_model(MODEL)?;
            Ok(out)
        })
        .unwrap()
}

#[test]
fn groups_notes_by_meaning_without_trashed_notes() {
    let (_dir, session, generation) = session();
    let themes = themed_notes(&session);
    // A trashed note, and a note whose vector was trashed with it, are left out.
    let trashed = session
        .with_vault(|v| {
            let id = v.create_note("", "trashed ferry", "ferry ferry harbour")?;
            set_vector(v, MODEL, id, &near(0, 0.0));
            v.trash_note(id)?;
            Ok(id)
        })
        .unwrap();

    let d = run(&session, generation, Options { groups: Some(3), scope: Scope::All });
    assert_eq!(d.model.as_deref(), Some(MODEL));
    assert_eq!((d.notes, d.k), (9, 3));
    let mut expected: Vec<Vec<i64>> = themes.to_vec();
    expected.sort();
    assert_eq!(partition(&d), expected);
    assert!(!d.groups.iter().flat_map(|g| &g.notes).any(|n| n.id == trashed));

    for g in &d.groups {
        // Nearest the centre first.
        assert!(g.notes.windows(2).all(|w| w[0].score >= w[1].score));
        // The theme's word is the most distinctive; words in every note come late.
        assert_eq!(g.words[0], g.notes[0].folder, "{:?}", g.words);
        assert!(!g.words[..3].iter().any(|w| w == "today" || w == "fine"), "{:?}", g.words);
        assert!(g.words.len() <= discover::WORDS_PER_GROUP);
    }

    // Deterministic: the same run gives the same result.
    assert_eq!(run(&session, generation, Options { groups: Some(3), scope: Scope::All }), d);
    // Without a number, the default (at least 5, at most the number of notes).
    assert_eq!(run(&session, generation, Options::default()).k, 5);
}

#[test]
fn untagged_only_leaves_out_notes_with_any_tag() {
    let (_dir, session, generation) = session();
    let themes = themed_notes(&session);
    session
        .with_vault(|v| {
            v.update_body(themes[0][0], "---\ntags: [boats]\n---\nThe ferry left the harbour late.")?;
            // An inline tag counts too.
            v.update_body(themes[1][0], "Planted tomatoes in the #garden today.")?;
            Ok(())
        })
        .unwrap();
    let d = run(&session, generation, Options { groups: Some(3), scope: Scope::Untagged });
    let ids: Vec<i64> = partition(&d).concat();
    assert_eq!(d.notes, 7);
    assert!(!ids.contains(&themes[0][0]) && !ids.contains(&themes[1][0]));
}

/// `per` notes for each of three themes, in folders `ferry`, `garden/<n>` and `budget`,
/// with vectors near three axes; the theme's notes by id.
fn many_themed_notes(session: &Session, per: usize) -> [Vec<i64>; 3] {
    session
        .with_vault(|v| {
            let mut out: [Vec<i64>; 3] = Default::default();
            for i in 0..per {
                for (axis, word) in ["ferry", "garden", "budget"].iter().enumerate() {
                    let folder = if axis == 1 { format!("garden/{}", i % 3) } else { word.to_string() };
                    let id = v.create_note(&folder, &format!("{word} {i}"), &format!("All about the {word}, part {i}."))?;
                    set_vector(v, MODEL, id, &near(axis, 0.01 * (i % 7) as f32));
                    out[axis].push(id);
                }
            }
            v.finish_embedding_model(MODEL)?;
            Ok(out)
        })
        .unwrap()
}

#[test]
fn notes_outside_the_sample_join_the_nearest_group() {
    let (_dir, session, generation) = session();
    let themes = many_themed_notes(&session, 40);
    let options = Options { groups: Some(3), scope: Scope::All };
    let d = discover::run_sampled(&session, generation, &options, 12, &|| false).unwrap();
    assert_eq!((d.notes, d.sampled, d.k), (120, 12, 3));
    let mut found: Vec<Vec<i64>> = d.groups.iter().map(|g| g.members.clone()).collect();
    found.sort();
    let mut expected = themes.to_vec();
    expected.sort();
    assert_eq!(found, expected);
    for g in &d.groups {
        assert_eq!(g.size, 40);
        // The listed notes are the nearest of all members, not only of the sample.
        assert_eq!(g.notes.len(), 40.min(discover::GROUP_NOTES));
        assert!(g.notes.windows(2).all(|w| w[0].score >= w[1].score));
        assert!(g.notes.iter().all(|n| g.members.contains(&n.id)));
    }
    // Sampling is deterministic: the same notes give the same groups.
    assert_eq!(discover::run_sampled(&session, generation, &options, 12, &|| false).unwrap(), d);
    // A trashed note is in no group, even when it was not sampled.
    let trashed = themes[2][39];
    session.with_vault(|v| v.trash_note(trashed)).unwrap();
    let d = discover::run_sampled(&session, generation, &options, 12, &|| false).unwrap();
    assert_eq!(d.notes, 119);
    assert!(!d.groups.iter().any(|g| g.members.contains(&trashed)));
}

#[test]
fn folder_and_search_scopes_group_only_their_notes() {
    let (_dir, session, generation) = session();
    let themes = many_themed_notes(&session, 9);
    // `garden` and its subfolders, but not `gardening` next to it.
    let outside = session
        .with_vault(|v| {
            let id = v.create_note("gardening", "tools", "All about the garden tools.")?;
            set_vector(v, MODEL, id, &near(1, 0.0));
            Ok(id)
        })
        .unwrap();
    let members = |scope: Scope| -> Vec<i64> {
        let d = run(&session, generation, Options { groups: Some(2), scope });
        let mut ids: Vec<i64> = d.groups.iter().flat_map(|g| g.members.clone()).collect();
        ids.sort();
        assert_eq!(d.notes, ids.len());
        ids
    };
    assert_eq!(members(Scope::Folder { folder: "Garden".into() }), themes[1]);
    let sub: Vec<i64> = themes[1].iter().copied().skip(1).step_by(3).collect();
    assert_eq!(members(Scope::Folder { folder: "garden/1".into() }), sub);
    assert_eq!(members(Scope::Folder { folder: "nowhere".into() }), Vec::<i64>::new());
    // The root is every note.
    assert_eq!(members(Scope::Folder { folder: "".into() }).len(), 28);

    // A search: its text and its filters, every match.
    let query = SearchQuery { text: "budget".into(), ..Default::default() };
    assert_eq!(members(Scope::Search { query }), themes[2]);
    let query = SearchQuery { text: "garden".into(), folder: Some("gardening".into()), ..Default::default() };
    assert_eq!(members(Scope::Search { query }), [outside]);
}

#[test]
fn select_all_tags_every_member_and_records_feedback_for_each() {
    let (_dir, session, generation) = session();
    many_themed_notes(&session, 30);
    let d = discover::run_sampled(&session, generation, &Options { groups: Some(3), scope: Scope::All }, 9, &|| false)
        .unwrap();
    let group = &d.groups[0];
    assert!(group.members.len() > group.notes.len().min(9));
    let report = session.with_vault(|v| v.create_tag_from_group("theme", "one theme", &group.members)).unwrap();
    assert_eq!(report.changed, group.members);
    session
        .with_vault(|v| {
            assert_eq!(v.tag_list()?.into_iter().find(|t| t.tag == "theme").unwrap().notes, group.members.len());
            for &id in &group.members {
                assert!(v.note(id)?.tags.contains(&"theme".to_string()));
                let feedback: Vec<(String, bool)> =
                    v.tag_feedback(id)?.into_iter().map(|TagFeedback { tag, accepted, .. }| (tag, accepted)).collect();
                assert_eq!(feedback, [("theme".to_string(), true)]);
            }
            // A member trashed since the run is left out by `live_notes`.
            v.trash_note(group.members[0])?;
            assert_eq!(v.live_notes(&group.members)?, group.members[1..]);
            Ok(())
        })
        .unwrap();
}

#[test]
fn nothing_to_group_before_a_model_is_ready() {
    let (_dir, session, generation) = session();
    session.with_vault(|v| v.create_note("", "A", "text")).unwrap();
    let d = run(&session, generation, Options::default());
    assert_eq!(d, Discovery { model: None, notes: 0, sampled: 0, k: 0, groups: Vec::new() });

    // A ready model but no vectors: no groups, no error.
    session.with_vault(|v| v.finish_embedding_model(MODEL)).unwrap();
    let d = run(&session, generation, Options::default());
    assert_eq!((d.notes, d.groups.len()), (0, 0));
}

#[test]
fn stops_on_cancel_and_lock_without_counting_as_activity() {
    let (_dir, session, generation) = session();
    themed_notes(&session);
    let idle = session.idle_for().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    run(&session, generation, Options::default());
    assert!(session.idle_for().unwrap() >= idle + Duration::from_millis(50), "the run bumped activity");

    let cancelled = discover::run(&session, generation, &Options::default(), &|| true);
    assert!(matches!(cancelled, Err(DiscoverError::Cancelled)));

    // A run for an earlier unlock never touches the vault opened after it.
    session.lock();
    assert!(matches!(discover::run(&session, generation, &Options::default(), &|| false), Err(DiscoverError::Locked)));
    let (vault_dir, vault) = common::vault();
    let next = session.unlock(vault, vault_dir.path().join("vault.hush")).unwrap();
    assert_ne!(next, generation);
    assert!(matches!(discover::run(&session, generation, &Options::default(), &|| false), Err(DiscoverError::Locked)));
}

#[test]
fn create_tag_from_group_writes_description_frontmatter_and_feedback() {
    let (_dir, session, _) = session();
    let themes = themed_notes(&session);
    let [plain, crlf, broken, already, rejected, unchosen] = session
        .with_vault(|v| {
            let crlf = v.create_note("", "crlf", "---\r\nauthor: Ada\r\n---\r\nFerry notes.\r\n")?;
            let broken = v.create_note("", "broken", "---\ntags: [a\n---\nFerry.")?;
            let already = v.create_note("", "already", "---\ntags: [boats]\n---\nFerry.")?;
            let rejected = v.create_note("", "rejected", "Ferry again.")?;
            v.reject_tag_suggestion(rejected, "boats")?;
            Ok([themes[0][0], crlf, broken, already, rejected, themes[0][1]])
        })
        .unwrap();

    let ids = [plain, crlf, broken, already, rejected];
    let report = session
        .with_vault(|v| v.create_tag_from_group(" #boats ", "  ferries, harbours and crossings ", &ids))
        .unwrap();
    assert_eq!(report.changed, [plain, crlf, rejected]);
    assert_eq!(report.unchanged, 1);
    assert_eq!(report.unreadable.iter().map(|n| n.id).collect::<Vec<_>>(), [broken]);

    session
        .with_vault(|v| {
            let info = v.tag_list()?.into_iter().find(|t| t.tag == "boats").unwrap();
            assert_eq!(info.description, "ferries, harbours and crossings");
            assert_eq!(info.notes, 4);
            assert_eq!(v.note(plain)?.body, "---\ntags: [boats]\n---\nThe ferry left the harbour late; ferry crossing was rough. Today was fine.");
            assert_eq!(v.note(crlf)?.body, "---\r\nauthor: Ada\r\ntags: [boats]\r\n---\r\nFerry notes.\r\n");
            assert_eq!(v.note(broken)?.body, "---\ntags: [a\n---\nFerry.");
            for id in [plain, crlf, already, rejected] {
                let feedback: Vec<(String, bool)> =
                    v.tag_feedback(id)?.into_iter().map(|TagFeedback { tag, accepted, .. }| (tag, accepted)).collect();
                assert_eq!(feedback, [("boats".to_string(), true)], "note {id}");
            }
            assert!(v.tag_feedback(broken)?.is_empty());
            assert!(v.tag_feedback(unchosen)?.is_empty());
            assert!(!v.note(unchosen)?.tags.contains(&"boats".to_string()));
            Ok(())
        })
        .unwrap();

    // Tags are case-sensitive: `Boats` is another tag. An empty description leaves the
    // existing one alone; no notes just saves nothing but the description.
    session.with_vault(|v| v.create_tag_from_group("Boats", "", &[unchosen])).unwrap();
    session.with_vault(|v| v.create_tag_from_group("boats", "", &[])).unwrap();
    let tags = session.with_vault(|v| v.tag_list()).unwrap();
    let boats: Vec<(&str, usize, &str)> =
        tags.iter().filter(|t| t.tag.eq_ignore_ascii_case("boats")).map(|t| (t.tag.as_str(), t.notes, t.description.as_str())).collect();
    assert_eq!(boats, [("Boats", 1, ""), ("boats", 4, "ferries, harbours and crossings")]);

    // All or nothing: an unknown note changes nothing.
    let err = session.with_vault(|v| v.create_tag_from_group("sea", "the sea", &[plain, 999_999]));
    assert!(err.is_err());
    session
        .with_vault(|v| {
            assert!(!v.tag_list()?.iter().any(|t| t.tag == "sea"));
            assert!(!v.note(plain)?.tags.contains(&"sea".to_string()));
            Ok(())
        })
        .unwrap();
}

/// How long grouping 10,000 notes takes, end to end (reading vectors and texts,
/// k-means with the default 50 groups, distinctive words). The vectors are synthetic
/// (384 dimensions), so this measures speed, not quality. Run in
/// release for numbers comparable to the app:
///
/// `cargo test --release --manifest-path src-tauri/Cargo.toml --test discover_tags -- --ignored --nocapture ten_thousand`
#[test]
#[ignore = "timing; prints measurements"]
fn ten_thousand_notes_group_in_under_ten_seconds() {
    // Clustered vectors converge in a few rounds; vectors without any structure are the
    // slow case for k-means.
    for structured in [true, false] {
        ten_thousand(structured);
    }
}

fn ten_thousand(structured: bool) {
    let (_dir, session, generation) = session();
    const N: usize = 10_000;
    const DIMS: usize = 384;
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
    };
    let centres: Vec<Vec<f32>> = (0..60).map(|_| (0..DIMS).map(|_| rand()).collect()).collect();
    let vocabulary: Vec<String> = (0..5_000).map(|i| format!("word{i}")).collect();
    let bodies: Vec<String> = (0..N)
        .map(|i| {
            let c = i % 60;
            (0..150)
                .map(|j| vocabulary[(c * 80 + (j * 7 + i) % 80 + if j % 3 == 0 { 2_500 } else { 0 }) % 5_000].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let titles: Vec<String> = (0..N).map(|i| format!("note {i}")).collect();
    let start = Instant::now();
    session
        .with_vault(|v| {
            let notes: Vec<NewNote> = (0..N)
                .map(|i| NewNote { folder: "bulk", title: &titles[i], body: &bodies[i], created_at: 0, updated_at: 0 })
                .collect();
            let ids: Vec<i64> = v.import_notes(&notes)?.into_iter().map(|r| r.unwrap()).collect();
            let texts = v.embedding_texts(&ids)?;
            let vectors: Vec<(i64, [u8; 32], Vec<f32>)> = texts
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut x: Vec<f32> = if structured {
                        centres[i % 60].iter().map(|c| c + 0.5 * rand()).collect()
                    } else {
                        (0..DIMS).map(|_| rand()).collect()
                    };
                    embed::normalize(&mut x);
                    (t.id, t.hash, x)
                })
                .collect();
            v.write_embeddings(MODEL, &vectors)?;
            v.finish_embedding_model(MODEL)
        })
        .unwrap();
    println!("set-up (not measured): {:.1} s", start.elapsed().as_secs_f32());

    let start = Instant::now();
    let d = run(&session, generation, Options::default());
    let took = start.elapsed();
    println!(
        "{}: grouped {} notes into {} groups in {:.2} s ({} build); largest {}, smallest {}",
        if structured { "60 noisy centres" } else { "no structure" },
        d.notes,
        d.k,
        took.as_secs_f32(),
        if cfg!(debug_assertions) { "debug" } else { "release" },
        d.groups.first().map_or(0, |g| g.size),
        d.groups.last().map_or(0, |g| g.size),
    );
    assert_eq!((d.notes, d.k), (N, 50));
    // A debug build (`tauri dev`) is several times slower; the limit is for the app.
    if !cfg!(debug_assertions) {
        assert!(took < Duration::from_secs(10), "took {took:?}");
    }
}

/// Groups the fixture vault with the real model and prints each group, then checks
/// that the journal (`jr`) and dream (`dr`) notes mostly land in their own groups. Run
/// it with the bundled model (`npm run fetch-model` first):
///
/// `cargo test --manifest-path src-tauri/Cargo.toml --test discover_tags -- --ignored --nocapture fixture`
///
/// `HUSH_MEASURE_ONNX=<file>` uses another ONNX export of the same model instead (put
/// it in `src-tauri/models/`; its checksum isn't checked, the tokenizer's still is).
#[test]
#[ignore = "needs the real model; prints the groups"]
fn fixture_vault_journal_and_dream_notes_group_together() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let mut descriptor: ModelDescriptor = *embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    if let Ok(file) = std::env::var("HUSH_MEASURE_ONNX") {
        use sha2::Digest;
        let path = Path::new(&file);
        assert_eq!(path.parent().unwrap().canonicalize().unwrap(), dir.canonicalize().unwrap(), "put it in {}", dir.display());
        let sha: String = sha2::Sha256::digest(std::fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
        descriptor.model.name = Box::leak(path.file_name().unwrap().to_string_lossy().into_owned().into_boxed_str());
        descriptor.model.sha256 = Box::leak(sha.into_boxed_str());
    }
    if !dir.join(descriptor.model.name).is_file() || !dir.join(descriptor.tokenizer.name).is_file() {
        panic!("model files missing in {}; run `npm run fetch-model`", dir.display());
    }
    let descriptor: &'static ModelDescriptor = Box::leak(Box::new(descriptor));
    struct Source(Arc<dyn Embedder>);
    impl EmbedderSource for Source {
        fn embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
            Ok(self.0.clone())
        }
    }
    let source = Source(Arc::new(OnnxEmbedder::load(&dir, descriptor).unwrap()));

    let (_dir, session, generation) = session();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
    import::run(&session, &fixture, "", false, &AtomicBool::new(false), |_| {}).unwrap();
    // Two passes: the first embeds every note, the second makes the model ready.
    for _ in 0..2 {
        job::pass(&session, generation, &source, &mut |_| {}).unwrap();
    }

    let d = run(&session, generation, Options::default());
    let tags = |id: i64| session.with_vault(|v| Ok(v.note(id)?.tags)).unwrap();
    println!("{} notes, {} groups (model {})", d.notes, d.k, descriptor.model.name);
    let mut journal: Vec<usize> = Vec::new();
    let mut dream: Vec<usize> = Vec::new();
    for (g, group) in d.groups.iter().enumerate() {
        println!("\n== group {} ({} notes): {}", g + 1, group.size, group.words.join(", "));
        for n in &group.notes {
            let t = tags(n.id);
            let kind = if t.iter().any(|t| t == "journal") {
                journal.push(g);
                " [jr]"
            } else if t.iter().any(|t| t == "dream" || t == "dr") {
                dream.push(g);
                " [dr]"
            } else {
                ""
            };
            println!("  {:.3} {}/{}{kind}", n.score, n.folder, n.title);
        }
    }
    // "Mostly in their own group": most of each kind share one group.
    let majority = |groups: &[usize]| -> (usize, usize) {
        let mut best = (0, 0);
        for &g in groups {
            let n = groups.iter().filter(|&&x| x == g).count();
            if n > best.1 {
                best = (g, n);
            }
        }
        best
    };
    let (jg, jn) = majority(&journal);
    let (dg, dn) = majority(&dream);
    println!("\njournal notes: {jn} of {} in group {}; dream notes: {dn} of {} in group {}", journal.len(), jg + 1, dream.len(), dg + 1);
    assert!(jn * 2 > journal.len() && dn * 2 > dream.len());
}
