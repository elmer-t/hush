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
    vault.change_passphrase("new passphrase").unwrap();
    vault.close().unwrap();

    assert!(matches!(Vault::open(&path, PASS), Err(Error::WrongPassphrase)));
    let vault = Vault::open(&path, "new passphrase").unwrap();
    assert_eq!(vault.note(id).unwrap().body, "still here");
}
