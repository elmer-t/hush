//! The tag rules (`markdown::is_tag` and what `markdown::parse` skips) and the background
//! pass that re-derives an existing vault's tags under them (`retag`).

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hush_lib::activity::{Job, State};
use hush_lib::db::{TAG_RULES_SETTING, Vault};
use hush_lib::markdown;
use hush_lib::retag::{self, RetagError};
use hush_lib::session::Session;
use regex::Regex;
use tempfile::TempDir;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/vault");
const POLL: Duration = Duration::from_millis(10);

fn session() -> (TempDir, PathBuf, Session, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();
    (dir, path, session, generation)
}

/// What a vault saved under the old rules holds: `tags` on note `id`, written on a
/// second connection, as the old parser would have derived them.
fn add_old_tags(path: &Path, id: i64, tags: &[&str]) {
    let raw = rusqlite::Connection::open(path).unwrap();
    raw.pragma_update(None, "key", common::PASS).unwrap();
    for tag in tags {
        raw.execute("INSERT INTO tags(name) VALUES (?1) ON CONFLICT(name) DO NOTHING", [tag]).unwrap();
        raw.execute("INSERT INTO note_tags(note_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2", rusqlite::params![id, tag])
            .unwrap();
    }
}

fn add_head(path: &Path, tag: &str) {
    let raw = rusqlite::Connection::open(path).unwrap();
    raw.pragma_update(None, "key", common::PASS).unwrap();
    raw.execute("INSERT INTO tag_heads VALUES (?1, 'm', x'0000803f', 0.5, 10, 12, 0.9, NULL, 3, x'00')", [tag]).unwrap();
}

fn tag_names(session: &Session) -> Vec<String> {
    session.with_vault(|v| v.tag_list()).unwrap().into_iter().map(|t| t.tag).collect()
}

#[test]
fn the_pass_drops_junk_tags_and_leaves_bodies_alone() {
    let (_dir, path, session, g) = session();
    let bodies = [
        "[A](#a) | [Z](#z) #work",
        "<p style=\"color: #fff\">x</p>\ncolor: #fff; border: #1e1e1e\n#work",
        "Some C:\n\n    #include <stdio.h>\n\nsee #1-3 and #12/13, #1st",
        "---\ntags: [1, a, ok]\n---\nNothing inline #café",
    ];
    let ids: Vec<i64> =
        bodies.iter().enumerate().map(|(i, b)| session.with_vault(|v| v.create_note("", &format!("N{i}"), b)).unwrap()).collect();
    add_old_tags(&path, ids[0], &["a", "z"]);
    add_old_tags(&path, ids[1], &["fff", "1e1e1e"]);
    add_old_tags(&path, ids[2], &["include", "1-3", "12/13", "1st"]);
    add_old_tags(&path, ids[3], &["1", "a"]);
    add_head(&path, "fff");
    add_head(&path, "work");
    session.with_vault(|v| v.set_tag_description("fff", "white")).unwrap();
    let before: Vec<_> = ids.iter().map(|id| session.with_vault(|v| v.note(*id)).unwrap()).collect();
    assert!(tag_names(&session).contains(&"1e1e1e".to_string()), "the old vault lists junk");

    let idle = session.idle_for().unwrap();
    let done = retag::run(&session, g, POLL).unwrap().expect("a vault without the setting needs the pass");
    assert!(session.idle_for().unwrap() >= idle, "the pass counted as activity");

    assert_eq!(done.checked, 4);
    assert_eq!(done.changed, 4);
    assert_eq!(done.end.heads_dropped, 1);
    // `fff` stays listed only for its description (the user's own words).
    assert_eq!(tag_names(&session), ["café", "fff", "ok", "work"]);
    let heads: Vec<String> = session.with_vault(|v| v.tag_heads()).unwrap().into_iter().map(|h| h.tag).collect();
    assert_eq!(heads, ["work"]);
    for (id, old) in ids.iter().zip(&before) {
        let now = session.with_vault(|v| v.note(*id)).unwrap();
        assert_eq!(now.body.as_bytes(), old.body.as_bytes(), "bodies are byte-identical");
        assert_eq!(now.updated_at, old.updated_at, "notes aren't touched");
    }
    assert_eq!(session.with_vault(|v| v.note(ids[3])).unwrap().tags, ["café", "ok"]);

    assert_eq!(retag::run(&session, g, POLL).unwrap(), None, "once per rules version");
    let version = session.with_vault(|v| v.setting(TAG_RULES_SETTING)).unwrap();
    assert_eq!(version, Some(markdown::TAG_RULES_VERSION.to_string()));
    assert_eq!(session.activity().status(Job::Retag).state, State::Idle);
    assert!(session.activity().status(Job::Retag).last_run.unwrap().summary.contains("4 changed"));
}

#[test]
fn a_new_rules_version_runs_the_pass_again() {
    let (_dir, path, session, g) = session();
    let id = session.with_vault(|v| v.create_note("", "N", "#ok")).unwrap();
    assert!(retag::run(&session, g, POLL).unwrap().is_some());
    assert_eq!(retag::run(&session, g, POLL).unwrap(), None);
    // A vault last re-derived under older rules.
    session.with_vault(|v| v.set_setting(TAG_RULES_SETTING, "0")).unwrap();
    add_old_tags(&path, id, &["x"]);
    let done = retag::run(&session, g, POLL).unwrap().unwrap();
    assert_eq!((done.checked, done.changed), (1, 1));
    assert_eq!(tag_names(&session), ["ok"]);
}

#[test]
fn a_pass_cut_off_by_a_lock_continues_where_it_was() {
    let (_dir, path, session, g) = session();
    let n = retag::PAGE as usize + 3;
    let ids: Vec<i64> = session
        .with_vault(|v| (0..n).map(|i| v.create_note("", &format!("N{i}"), "#ok")).collect::<Result<_, _>>())
        .unwrap();
    add_old_tags(&path, ids[0], &["x1x"]);
    // The first page, then the vault locks.
    let page = session.with_vault(|v| v.retag_page(0, retag::PAGE)).unwrap();
    assert_eq!(page.next, Some(ids[retag::PAGE as usize - 1]));
    session.lock();
    assert!(matches!(retag::run(&session, g, POLL), Err(RetagError::Locked)));

    let g = session.unlock(Vault::open(&path, common::PASS).unwrap(), path.clone()).unwrap();
    assert_eq!(session.with_vault(|v| v.retag_start()).unwrap(), Some(ids[retag::PAGE as usize - 1]));
    let done = retag::run(&session, g, POLL).unwrap().unwrap();
    assert_eq!(done.checked, 3, "only the rest");
    assert_eq!(tag_names(&session), ["ok"]);
}

#[test]
fn the_pass_waits_for_an_import() {
    let (_dir, _path, session, g) = session();
    session.with_vault(|v| v.create_note("", "N", "#ok")).unwrap();
    std::thread::scope(|s| {
        let import = session.begin_import();
        let pass = s.spawn(|| retag::run(&session, g, POLL));
        let paused = || session.activity().status(Job::Retag).state == State::Paused;
        let start = std::time::Instant::now();
        while !paused() {
            assert!(start.elapsed() < Duration::from_secs(10), "never paused");
            std::thread::sleep(POLL);
        }
        drop(import);
        assert!(pass.join().unwrap().unwrap().is_some());
    });
}

// ---------------------------------------------------------------------------------------
// Diagnostic (issue #122): which tags the old rules found that the new ones reject.

static OLD_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[\s(,;])#([\p{L}\p{N}_\-/]+)").unwrap());
static OLD_INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]+`").unwrap());

/// The tags the rules before #122 derived from `body` (a copy of the old parser's tag
/// part: fenced and inline code skipped, only all-digit tags rejected, frontmatter tags
/// taken as written).
fn old_tags(body: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        if !tags.iter().any(|x| x == t) {
            tags.push(t.to_string());
        }
    };
    let start = match markdown::split_frontmatter(body) {
        Some((yaml, end)) => {
            let fm = markdown::parse(&format!("---\n{yaml}---\n"));
            fm.frontmatter_tags.iter().for_each(|t| push(t));
            end
        }
        None => 0,
    };
    let content = &body[start..];
    let mut code: Vec<std::ops::Range<usize>> = Vec::new();
    let (mut fence, mut pos): (Option<(usize, &str)>, usize) = (None, 0);
    for line in content.split_inclusive('\n') {
        let t = line.trim_start();
        let marker = if line.len() - t.len() > 3 {
            None
        } else if t.starts_with("```") {
            Some("```")
        } else if t.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        match (fence, marker) {
            (None, Some(m)) => fence = Some((pos, m)),
            (Some((s, open)), Some(m)) if open == m => {
                code.push(s..pos + line.len());
                fence = None;
            }
            _ => {}
        }
        pos += line.len();
    }
    if let Some((s, _)) = fence {
        code.push(s..content.len());
    }
    code.extend(OLD_INLINE_CODE.find_iter(content).map(|m| m.range()));
    for caps in OLD_TAG.captures_iter(content) {
        let m = caps.get(1).unwrap();
        let tag = m.as_str().trim_end_matches('/');
        if code.iter().any(|r| r.contains(&m.start())) || tag.is_empty() || tag.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        push(tag);
    }
    tags
}

/// Imports `source` into a fresh vault and returns every note's `(folder/title, body)`.
fn imported(source: &Path) -> Vec<(String, String)> {
    let (_dir, _path, session, _) = session();
    hush_lib::import::run(&session, source, "", false, &AtomicBool::new(false), |_| {}).unwrap();
    session
        .with_vault(|v| {
            let mut notes = Vec::new();
            for folder in v.folders()? {
                for s in v.folder_notes(&folder, 0, u32::MAX)? {
                    notes.push((format!("{}/{}", s.folder, s.title), v.note(s.id)?.body));
                }
            }
            Ok(notes)
        })
        .unwrap()
}

/// Lists the tags the old rules derived that the new ones reject, with their note
/// counts and one example note each, as a Markdown table:
///
/// ```text
/// cargo test --manifest-path src-tauri/Cargo.toml --test tag_rules -- --ignored --nocapture
/// ```
///
/// It runs over the fixture vault, and also over the folder in `HUSH_TAG_REPORT_SOURCE`
/// (an Obsidian vault or any folder Hush can import), which is imported into a
/// throwaway vault: nothing is written next to it.
#[test]
#[ignore]
fn tag_rules_report() {
    let mut sources = vec![PathBuf::from(FIXTURE)];
    if let Ok(extra) = std::env::var("HUSH_TAG_REPORT_SOURCE") {
        sources.push(PathBuf::from(extra));
    }
    for source in sources {
        let notes = imported(&source);
        let mut rejected: BTreeMap<String, (usize, String)> = BTreeMap::new();
        let (mut kept, mut all) = (0, 0);
        for (name, body) in &notes {
            let new = markdown::parse(body).tags;
            for tag in old_tags(body) {
                all += 1;
                if new.contains(&tag) {
                    kept += 1;
                } else {
                    let e = rejected.entry(tag).or_insert((0, name.clone()));
                    e.0 += 1;
                }
            }
        }
        let mut rows: Vec<_> = rejected.into_iter().collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        println!("\n### {} ({} notes, {all} note-tag pairs under the old rules, {kept} kept)\n", source.display(), notes.len());
        println!("| Rejected tag | Notes | Example note |\n|---|---|---|");
        for (tag, (n, example)) in &rows {
            println!("| `#{tag}` | {n} | {example} |");
        }
        if rows.is_empty() {
            println!("| (none) | | |");
        }
    }
}
