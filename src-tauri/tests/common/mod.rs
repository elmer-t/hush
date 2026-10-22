#![allow(dead_code)]

use hush_lib::db::Vault;
use tempfile::TempDir;

pub const PASS: &str = "correct horse battery staple";

/// A fresh vault in its own temp dir; keep the dir alive as long as the vault.
pub fn vault() -> (TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(&dir.path().join("vault.hush"), PASS).unwrap();
    (dir, vault)
}
