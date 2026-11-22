mod common;

use hush_lib::config::{AppConfig, Config};
use hush_lib::db::{Error, VAULT_ICON_SETTING, Vault, VaultIcon};

fn icon(icon: &str, color: &str) -> VaultIcon {
    VaultIcon { icon: icon.to_string(), color: color.to_string() }
}

#[test]
fn a_new_vault_has_no_icon() {
    let (_dir, vault) = common::vault();
    assert_eq!(vault.vault_icon().unwrap(), None);
    assert_eq!(vault.setting(VAULT_ICON_SETTING).unwrap(), None);
}

#[test]
fn the_icon_is_kept_in_the_settings_table_and_survives_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_vault_icon(Some(&icon("anchor", "blue"))).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("anchor", "blue")));
    assert_eq!(vault.setting(VAULT_ICON_SETTING).unwrap().as_deref(), Some(r#"{"icon":"anchor","color":"blue"}"#));
    vault.close().unwrap();

    let vault = Vault::open(&path, common::PASS).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("anchor", "blue")));
    vault.set_vault_icon(Some(&icon("ship", "red"))).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("ship", "red")));
}

#[test]
fn none_removes_the_icon() {
    let (_dir, vault) = common::vault();
    vault.set_vault_icon(Some(&icon("star", "amber"))).unwrap();
    vault.set_vault_icon(None).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), None);
    assert_eq!(vault.setting(VAULT_ICON_SETTING).unwrap(), None, "the key is gone, not emptied");
    // Removing an icon that is not there is fine.
    vault.set_vault_icon(None).unwrap();
}

#[test]
fn ids_that_are_not_lowercase_letters_digits_and_hyphens_are_rejected() {
    let (_dir, vault) = common::vault();
    vault.set_vault_icon(Some(&icon("book", "green"))).unwrap();
    let long = "a".repeat(33);
    for bad in ["", "Book", "book open", "book_open", "bóók", "../x", "<svg>", "a\"b", long.as_str()] {
        assert!(matches!(vault.set_vault_icon(Some(&icon(bad, "green"))), Err(Error::InvalidVaultIcon)), "icon {bad:?}");
        assert!(matches!(vault.set_vault_icon(Some(&icon("book", bad))), Err(Error::InvalidVaultIcon)), "color {bad:?}");
    }
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("book", "green")), "a rejected icon changes nothing");
    // The longest and the oddest IDs that are allowed. Rust does not know the icon list.
    vault.set_vault_icon(Some(&icon(&"a".repeat(32), "0-9"))).unwrap();
}

/// A value another version wrote that this one can't read is no icon, not an error: the
/// vault must still unlock.
#[test]
fn an_unreadable_stored_value_reads_as_no_icon() {
    let (_dir, vault) = common::vault();
    for stored in ["", "not json", "{}", r#"{"icon":"Book","color":"green"}"#, r#"{"icon":"book"}"#] {
        vault.set_setting(VAULT_ICON_SETTING, stored).unwrap();
        assert_eq!(vault.vault_icon().unwrap(), None, "{stored:?}");
    }
    // Fields a newer version added are ignored.
    vault.set_setting(VAULT_ICON_SETTING, r#"{"icon":"book","color":"green","shape":"round"}"#).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("book", "green")));
}

/// The icon lives in the encrypted vault only (decided 2026-11-22, #241): the plaintext
/// app config, written as on unlock, is byte for byte what it is for a vault without one.
#[test]
fn config_json_is_the_same_with_and_without_an_icon() {
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.hush");
    let config_path = dir.path().join("config.json");
    let config = Config::new(config_path.clone());

    let vault = Vault::create(&vault_path, common::PASS).unwrap();
    config.update(|c| c.remember(&vault_path)).unwrap();
    let without = std::fs::read(&config_path).unwrap();

    vault.set_vault_icon(Some(&icon("anchor", "violet"))).unwrap();
    vault.close().unwrap();
    // Lock and unlock again, as the app does.
    let vault = Vault::open(&vault_path, common::PASS).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("anchor", "violet")));
    config.update(|c| c.remember(&vault_path)).unwrap();

    let with = std::fs::read(&config_path).unwrap();
    assert_eq!(with, without);
    let text = String::from_utf8(with).unwrap();
    for word in ["anchor", "violet", "icon", "color"] {
        assert!(!text.contains(word), "{text}");
    }
    // No field of the config can hold one either.
    let fields = serde_json::to_value(AppConfig::default()).unwrap();
    let mut keys: Vec<&str> = fields.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["index_all_cores", "index_gpu_off", "index_rates", "memory_security", "recent_vaults", "window_dark"]);
}

/// The icon follows the vault file: a copy under another name shows it after unlock.
#[test]
fn the_icon_follows_a_copied_vault_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_vault_icon(Some(&icon("leaf", "green"))).unwrap();
    vault.close().unwrap();
    let copy = dir.path().join("elsewhere").join("renamed.hush");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(&path, &copy).unwrap();
    let vault = Vault::open(&copy, common::PASS).unwrap();
    assert_eq!(vault.vault_icon().unwrap(), Some(icon("leaf", "green")));
}
