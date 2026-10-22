//! Proves the bundled SQLCipher build encrypts, rejects a wrong key, and ships FTS5.

use rusqlite::Connection;

fn open(path: &std::path::Path, key: &str) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "key", key).unwrap();
    conn
}

#[test]
fn encrypts_and_rejects_wrong_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");

    let conn = open(&path, "correct horse");
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(body);
         INSERT INTO t(body) VALUES ('a secret sentence');",
    )
    .unwrap();
    let hits: i64 = conn
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'secret'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hits, 1);
    drop(conn);

    let raw = std::fs::read(&path).unwrap();
    assert!(!raw.windows(6).any(|w| w == b"secret"), "plaintext found on disk");
    assert!(!raw.starts_with(b"SQLite format 3"), "file is not encrypted");

    let wrong = open(&path, "wrong");
    assert!(wrong.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_err());
}
