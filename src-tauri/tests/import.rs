mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

use hush_lib::db::{Resolution, SearchQuery, Vault};
use hush_lib::import::rules::{Rule, Source, Target};
use hush_lib::import::{self, ImportReport, PrepareError};
use hush_lib::session::{Session, SessionError};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/vault");
/// 2010-01-01T00:00:00Z: every fixture copy gets this mtime, so dates that fall back to it are stable.
const MTIME: u64 = 1_262_304_000;

/// A copy of the fixture vault with fixed modification times.
fn fixture() -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
                let file = fs::File::options().write(true).open(&target).unwrap();
                file.set_modified(UNIX_EPOCH + Duration::from_secs(MTIME))
                    .unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    copy(Path::new(FIXTURE), dir.path());
    dir
}

fn unlocked() -> (tempfile::TempDir, Session) {
    let (dir, vault) = common::vault();
    let session = Session::default();
    session
        .unlock(vault, dir.path().join("vault.hush"))
        .unwrap();
    (dir, session)
}

fn run(session: &Session, source: &Path, dry_run: bool) -> ImportReport {
    import::run(
        session,
        source,
        "",
        dry_run,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}

fn date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .format("%Y-%m-%d %H:%M UTC")
        .to_string()
}

fn titles(vault: &Vault, q: SearchQuery) -> Vec<String> {
    let mut t: Vec<String> = vault
        .search(&q)
        .unwrap()
        .into_iter()
        .map(|h| h.note.title)
        .collect();
    t.sort();
    t
}

fn render(report: &ImportReport) -> String {
    let mut out = format!(
        "files: {}\nimported: {}\nwarnings: {}\ncancelled: {}\n",
        report.files, report.imported, report.warning_count, report.cancelled
    );
    for u in &report.unsupported {
        out += &format!("not imported {}: {}\n", u.format, u.count);
    }
    for s in &report.skipped {
        out += &format!("skipped {}: {}\n", s.path, s.message);
    }
    for w in &report.warnings {
        out += &format!("warning {}: {}\n", w.path, w.message);
    }
    for l in &report.ambiguous_links {
        out += &format!("ambiguous link in {}: [[{}]]\n", l.note, l.target);
    }
    out
}

#[test]
fn fixture_files_are_cleaned() {
    let source = fixture();
    let rules: Vec<_> = Rule::defaults()
        .iter()
        .map(|r| r.compile().unwrap())
        .collect();
    let scan = import::scan(source.path()).unwrap();
    assert_eq!(scan.files.len(), 29, "hidden files are left out");
    assert_eq!(scan.page_assets, 2, "ferry_files/ belongs to ferry.html");
    let mut unsupported = Vec::new();
    for rel in &scan.files {
        let p = match import::prepare(source.path(), rel, &rules) {
            Ok(p) => p,
            Err(PrepareError::Unsupported) => {
                unsupported.push(rel.to_string_lossy().replace('\\', "/"));
                continue;
            }
            Err(e) => panic!("{}: {e}", rel.display()),
        };
        let mut out = format!(
            "folder: {}\ntitle: {}\ncreated: {} ({:?})\nupdated: {}\n",
            p.folder,
            p.title,
            date(p.created_at),
            p.created_from,
            date(p.updated_at)
        );
        for w in &p.warnings {
            out += &format!("warning: {w}\n");
        }
        out += "-----\n";
        out += &p.body;
        let name: String = p
            .path
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        insta::assert_snapshot!(name, out);
    }
    assert_eq!(
        unsupported,
        [
            "archive/docs/brief.doc",
            "archive/docs/fake.txt",
            "archive/docs/scan.pdf",
            "attachments/photo.png"
        ]
    );
}

#[test]
fn page_folders_with_documents_are_imported() {
    let source = tempfile::tempdir().unwrap();
    for file in [
        "Arc.htm",
        "Arc_files/sitter.html",
        "Arc_files/PandP.jpg",
        "Arc_files/more/jamie.html",
        "page.html",
        "page_files/style.css",
        "page_files/logo.gif",
    ] {
        let path = source.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "<html><body>text</body></html>").unwrap();
    }
    let scan = import::scan(source.path()).unwrap();
    let files: Vec<String> = scan
        .files
        .iter()
        .map(|f| f.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(
        files,
        [
            "Arc.htm",
            "Arc_files/PandP.jpg",
            "Arc_files/more/jamie.html",
            "Arc_files/sitter.html",
            "page.html"
        ]
    );
    assert_eq!(
        scan.page_assets, 2,
        "page_files/ only holds the page's styles and images"
    );
}

#[test]
fn import_into_vault() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let report = run(&session, source.path(), false);
    insta::assert_snapshot!(render(&report));

    session
        .with_vault(|v| {
            let tag = |t: &str| SearchQuery {
                tags: vec![t.into()],
                ..Default::default()
            };
            assert_eq!(
                titles(v, tag("journal")),
                [
                    "2004-03-12 Morning walk",
                    "2004-04-01 Rain",
                    "2005-01-02 New year"
                ],
                "label codes in file names become tags and are stripped from the title"
            );
            assert_eq!(titles(v, tag("dream")), ["2004-03-14 Flying"]);
            let jan = SearchQuery {
                author: Some("Jan de Vries".into()),
                ..Default::default()
            };
            assert_eq!(titles(v, jan).len(), 3);
            let email = SearchQuery {
                attributes: vec![("author_email".into(), "jan@example.org".into())],
                ..Default::default()
            };
            assert_eq!(titles(v, email).len(), 4);
            let topic = SearchQuery {
                topic: Some("archive".into()),
                ..Default::default()
            };
            assert_eq!(
                titles(v, topic).len(),
                12,
                "topic defaults to the top-level folder"
            );
            let letters = SearchQuery {
                folder: Some("archive/letters".into()),
                ..Default::default()
            };
            assert!(
                titles(v, letters).contains(&"letter to Jan (txt)".to_string()),
                "same-name files are kept apart"
            );
            let cafe = SearchQuery {
                text: "café".into(),
                ..Default::default()
            };
            assert_eq!(
                titles(v, cafe).len(),
                4,
                "read from Windows-1252 text and HTML, UTF-16 and RTF"
            );
            let text = SearchQuery {
                text: "herons".into(),
                ..Default::default()
            };
            assert_eq!(titles(v, text), ["2004-03-12 Morning walk", "Notes"]);
            assert!(matches!(
                v.resolve_link("2004-03-12 Morning walk")?,
                Resolution::Unique(_)
            ));
            Ok(())
        })
        .unwrap();
}

#[test]
fn dry_run_writes_nothing_and_predicts_the_import() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let dry = run(&session, source.path(), true);
    assert!(dry.dry_run);
    assert!(!dry.samples.is_empty());
    assert!(dry.samples.iter().all(|s| s.before != s.after));
    assert!(
        dry.ambiguous_links.is_empty(),
        "links are only resolved after a real import"
    );
    assert!(
        session
            .with_vault(|v| v.search(&SearchQuery::default()))
            .unwrap()
            .is_empty()
    );

    let real = run(&session, source.path(), false);
    assert_eq!(dry.imported, real.imported);
    assert_eq!(dry.warnings, real.warnings);
    assert!(real.samples.is_empty());
}

#[test]
fn reimport_skips_existing_notes() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let first = run(&session, source.path(), false);
    let second = run(&session, source.path(), false);
    assert_eq!(second.imported, 0);
    assert_eq!(second.skipped.len(), first.imported + first.skipped.len());
    assert_eq!(second.skipped_count, second.skipped.len());
    assert_eq!(second.folder_count, second.folders.len());
    let dry = run(&session, source.path(), true);
    assert_eq!(dry.skipped, second.skipped);
}

#[test]
fn cancel_stops_before_the_next_batch() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let report = import::run(
        &session,
        source.path(),
        "",
        false,
        &AtomicBool::new(true),
        |_| {},
    )
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(report.imported, 0);
}

#[test]
fn progress_is_reported() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let mut seen = Vec::new();
    import::run(
        &session,
        source.path(),
        "",
        false,
        &AtomicBool::new(false),
        |p| seen.push((p.done, p.total)),
    )
    .unwrap();
    assert_eq!(seen.first(), Some(&(0, 29)));
    assert_eq!(seen.last(), Some(&(29, 29)));
}

#[test]
fn locked_vault_stops_the_import() {
    let source = fixture();
    let (_dir, session) = unlocked();
    session.lock();
    let err = import::run(
        &session,
        source.path(),
        "",
        false,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(err, import::ImportError::Session(SessionError::Locked)),
        "{err}"
    );
}

#[test]
fn source_folder_is_not_modified() {
    fn listing(dir: &Path, out: &mut BTreeMap<String, (Vec<u8>, std::time::SystemTime)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                listing(&path, out);
            } else {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                out.insert(
                    path.display().to_string(),
                    (fs::read(&path).unwrap(), modified),
                );
            }
        }
    }
    let source = fixture();
    let (_dir, session) = unlocked();
    let mut before = BTreeMap::new();
    listing(source.path(), &mut before);
    run(&session, source.path(), true);
    run(&session, source.path(), false);
    let mut after = BTreeMap::new();
    listing(source.path(), &mut after);
    assert!(before == after);
}

#[test]
fn missing_source_folder_is_an_error() {
    let (_dir, session) = unlocked();
    let missing = Path::new(FIXTURE).join("does-not-exist");
    assert!(
        import::run(
            &session,
            &missing,
            "",
            true,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
}

#[test]
fn rules_are_stored_in_the_vault() {
    let (dir, mut vault) = common::vault();
    assert_eq!(vault.import_rules().unwrap(), Rule::defaults());

    let rule = Rule {
        name: "Mood".into(),
        enabled: true,
        source: Source::Body,
        lines: 0,
        pattern: r"^Mood: (\w+)$".into(),
        target: Target::Attribute("mood".into()),
        map: vec![("ok".into(), "fine".into())],
        strip: true,
    };
    vault.set_import_rules(std::slice::from_ref(&rule)).unwrap();
    assert_eq!(vault.import_rules().unwrap(), std::slice::from_ref(&rule));

    let bad = Rule {
        pattern: "(unclosed".into(),
        ..rule.clone()
    };
    assert!(vault.set_import_rules(&[bad]).is_err());
    assert_eq!(
        vault.import_rules().unwrap(),
        std::slice::from_ref(&rule),
        "a rejected save changes nothing"
    );

    vault.close().unwrap();
    let vault = Vault::open(&dir.path().join("vault.hush"), common::PASS).unwrap();
    assert_eq!(vault.import_rules().unwrap(), [rule]);
}

#[test]
fn import_stops_if_another_vault_is_unlocked() {
    let source = fixture();
    let (dir, session) = unlocked();
    let other = dir.path().join("other.hush");
    Vault::create(&other, common::PASS)
        .unwrap()
        .close()
        .unwrap();
    let result = import::run(
        &session,
        source.path(),
        "",
        false,
        &AtomicBool::new(false),
        |p| {
            if p.done == 0 {
                session
                    .unlock(Vault::open(&other, common::PASS).unwrap(), other.clone())
                    .unwrap();
            }
        },
    );
    assert!(matches!(
        result,
        Err(import::ImportError::Session(SessionError::Locked))
    ));
    assert!(
        session
            .with_vault(|v| v.search(&SearchQuery::default()))
            .unwrap()
            .is_empty()
    );
}

fn run_into(session: &Session, source: &Path, target: &str, dry_run: bool) -> ImportReport {
    import::run(
        session,
        source,
        target,
        dry_run,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}

#[test]
fn target_folder_is_suggested_from_the_source_name() {
    assert_eq!(import::suggest_target(r"D:\Archive\OldVault"), "OldVault");
    assert_eq!(import::suggest_target(r"D:\Archive\OldVault\"), "OldVault");
    assert_eq!(import::suggest_target("/home/me/Old Vault/"), "Old Vault");
    assert_eq!(
        import::suggest_target(r"D:\"),
        "",
        "a drive root suggests the vault root"
    );
    assert_eq!(import::suggest_target(""), "");
}

#[test]
fn import_under_a_target_folder() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let dry = run_into(&session, source.path(), " OldVault/ ", true);
    assert_eq!(dry.target, "OldVault");
    assert_eq!(dry.existing_in_target, 0);
    assert!(
        dry.folders
            .iter()
            .all(|f| f.folder == "OldVault" || f.folder.starts_with("OldVault/"))
    );
    assert!(dry.samples.iter().all(|s| s.folder.starts_with("OldVault")));
    let real = run_into(&session, source.path(), "OldVault", false);
    assert_eq!(
        dry.folders, real.folders,
        "the preview shows the final folders"
    );

    // Folders are prefixed; topics stay the top-level folder relative to the source,
    // and a file in the source root gets none.
    let notes = session
        .with_vault(|v| {
            v.search(&SearchQuery {
                limit: Some(1000),
                ..Default::default()
            })
        })
        .unwrap();
    let mut out: Vec<String> = notes
        .iter()
        .map(|h| {
            format!(
                "{}/{} topic={}",
                h.note.folder,
                h.note.title,
                h.note.topic.as_deref().unwrap_or("-")
            )
        })
        .collect();
    out.sort();
    let mut folders = String::new();
    for f in &real.folders {
        folders += &format!("{} ({})\n", f.folder, f.count);
    }
    insta::assert_snapshot!(format!("{folders}-----\n{}", out.join("\n")));
    assert!(out.contains(&"OldVault/README topic=-".to_string()));
}

#[test]
fn import_into_an_existing_target_reports_the_merge() {
    let source = fixture();
    let (_dir, session) = unlocked();
    let first = run_into(&session, source.path(), "OldVault", false);
    assert_eq!(first.already_exist, 0);

    let dry = run_into(&session, source.path(), "OldVault", true);
    assert_eq!(dry.existing_in_target, first.imported);
    assert_eq!(dry.imported, 0);
    assert_eq!(
        dry.already_exist, first.imported,
        "every note would be skipped as already there"
    );

    let elsewhere = run_into(&session, source.path(), "Other", true);
    assert_eq!(elsewhere.existing_in_target, 0);
    assert_eq!(elsewhere.already_exist, 0);
    assert_eq!(elsewhere.imported, first.imported);

    let root = run_into(&session, source.path(), "", true);
    assert_eq!(
        root.existing_in_target, 0,
        "the vault root is not a merge target"
    );
    session
        .with_vault(|v| {
            assert_eq!(v.count_in_folder("OldVault")?, first.imported);
            assert_eq!(v.count_in_folder("OldVault/journal")?, 6);
            assert_eq!(
                v.count_in_folder("OldVaul")?,
                0,
                "a prefix of a folder name is another folder"
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn oversized_files_are_skipped_or_unsupported_by_content() {
    let source = tempfile::tempdir().unwrap();
    let big = import::MAX_TEXT_BYTES as usize + 1;
    fs::write(source.path().join("log.txt"), "line\n".repeat(big / 5 + 1)).unwrap();
    let mut binary = vec![b'x'; big];
    binary[10] = 0;
    fs::write(source.path().join("dump.txt"), binary).unwrap();
    let (_dir, session) = unlocked();
    let report = run(&session, source.path(), true);
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].path, "log.txt");
    assert!(
        report.skipped[0].message.starts_with("too large"),
        "{}",
        report.skipped[0].message
    );
    assert_eq!(
        report.unsupported,
        vec![import::FormatCount {
            format: ".txt".into(),
            count: 1
        }]
    );
}

#[test]
fn notes_are_written_in_file_order_across_batches() {
    let source = tempfile::tempdir().unwrap();
    let files = import::BATCH_SIZE + 500;
    for i in 0..files {
        fs::write(
            source.path().join(format!("n{i:05}.txt")),
            format!("note {i}"),
        )
        .unwrap();
    }
    let (_dir, session) = unlocked();
    let mut seen = Vec::new();
    let report = import::run(
        &session,
        source.path(),
        "",
        false,
        &AtomicBool::new(false),
        |p| seen.push(p.done),
    )
    .unwrap();
    assert_eq!(report.imported, files);
    assert_eq!(seen, vec![0, import::BATCH_SIZE, files]);
    let notes = session
        .with_vault(|v| v.folder_notes("", 0, files as u32))
        .unwrap();
    assert_eq!(notes.len(), files);
    assert!(
        notes
            .windows(2)
            .all(|w| w[0].title < w[1].title && w[0].id < w[1].id)
    );
}
