mod common;

use common::PASS;
use hush_lib::db::{Error, Vault};

#[test]
fn create_lock_unlock_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");

    let mut vault = Vault::create(&path, PASS).unwrap();
    let id = vault.create_note("", "Secret", "the launch code is swordfish").unwrap();
    vault.set_setting("auto_lock_minutes", "5").unwrap();
    vault.close().unwrap();

    let vault = Vault::open(&path, PASS).unwrap();
    assert_eq!(vault.note(id).unwrap().body, "the launch code is swordfish");
    assert_eq!(vault.setting("auto_lock_minutes").unwrap().as_deref(), Some("5"));
    assert_eq!(vault.setting("missing").unwrap(), None);
}

#[test]
fn no_plaintext_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let mut vault = Vault::create(&path, PASS).unwrap();
    vault.create_note("", "Diary", "swordfish").unwrap();

    // Check the main file and the WAL while the vault is still open.
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(!bytes.windows(9).any(|w| w == b"swordfish"));
        assert!(!bytes.windows(5).any(|w| w == b"Diary"));
    }
    vault.close().unwrap();
}

#[test]
fn wrong_passphrase_is_rejected() {
    let (dir, vault) = common::vault();
    vault.close().unwrap();
    let path = dir.path().join("vault.hush");
    assert!(matches!(Vault::open(&path, "nope"), Err(Error::WrongPassphrase)));
    assert!(Vault::open(&path, PASS).is_ok());
}

#[test]
fn create_and_open_guard_paths_and_passphrase() {
    let (dir, vault) = common::vault();
    vault.close().unwrap();
    let path = dir.path().join("vault.hush");
    assert!(matches!(Vault::create(&path, PASS), Err(Error::VaultExists)));
    assert!(matches!(Vault::open(&dir.path().join("missing.hush"), PASS), Err(Error::VaultNotFound)));
    assert!(matches!(Vault::create(&dir.path().join("x.hush"), ""), Err(Error::EmptyPassphrase)));
}

#[test]
fn non_vault_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "just some text, definitely not a database").unwrap();
    assert!(matches!(Vault::open(&path, PASS), Err(Error::WrongPassphrase)));
}

#[test]
fn change_passphrase() {
    let (dir, mut vault) = common::vault();
    let path = dir.path().join("vault.hush");
    let id = vault.create_note("", "Kept", "still here").unwrap();
    assert!(matches!(vault.verify_passphrase("nope"), Err(Error::WrongPassphrase)));
    vault.verify_passphrase(PASS).unwrap();
    vault.change_passphrase("new passphrase").unwrap();
    vault.verify_passphrase("new passphrase").unwrap();
    vault.close().unwrap();

    assert!(matches!(Vault::open(&path, PASS), Err(Error::WrongPassphrase)));
    let vault = Vault::open(&path, "new passphrase").unwrap();
    assert_eq!(vault.note(id).unwrap().body, "still here");
}

#[test]
fn vault_open_twice_fails_until_the_first_closes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let first = Vault::create(&path, PASS).unwrap();
    assert!(hush_lib::db::lock_path(&path).is_file(), "the lock file sits next to the vault");
    assert!(matches!(Vault::open(&path, PASS), Err(Error::VaultInUse)));
    assert!(matches!(Vault::open(&path, "wrong"), Err(Error::VaultInUse)), "checked before the passphrase");
    first.close().unwrap();

    let second = Vault::open(&path, PASS).unwrap();
    assert!(matches!(Vault::open(&path, PASS), Err(Error::VaultInUse)));
    drop(second);
    Vault::open(&path, PASS).unwrap();
}

#[test]
fn locking_the_session_releases_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let session = hush_lib::session::Session::default();
    session.unlock(Vault::create(&path, PASS).unwrap(), path.clone()).unwrap();
    assert!(matches!(Vault::open(&path, PASS), Err(Error::VaultInUse)));
    assert!(session.lock());
    Vault::open(&path, PASS).unwrap();
}

#[test]
fn other_vaults_are_not_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let _a = Vault::create(&dir.path().join("a.hush"), PASS).unwrap();
    Vault::create(&dir.path().join("b.hush"), PASS).unwrap();
}

/// Env var naming the vault [`child_holds_vault`] opens.
const CHILD_VAULT: &str = "HUSH_TEST_CHILD_VAULT";

/// Run by [`vault_held_by_another_process`] in a child process: opens the vault, says
/// so, and keeps it open until killed.
#[test]
#[ignore = "helper process for vault_held_by_another_process"]
fn child_holds_vault() {
    let Ok(path) = std::env::var(CHILD_VAULT) else { return };
    let _vault = Vault::open(std::path::Path::new(&path), PASS).unwrap();
    println!("holding");
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[test]
fn vault_held_by_another_process() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    Vault::create(&path, PASS).unwrap().close().unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_holds_vault", "--ignored", "--nocapture", "--test-threads=1"])
        .env(CHILD_VAULT, &path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("holding") {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "child exited before opening the vault");
    }
    assert!(matches!(Vault::open(&path, PASS), Err(Error::VaultInUse)));

    // A crash: the OS releases the lock when the process dies.
    child.kill().unwrap();
    child.wait().unwrap();
    Vault::open(&path, PASS).unwrap();
}
