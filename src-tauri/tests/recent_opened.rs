mod common;

use std::path::Path;

use hush_lib::config::Config;
use hush_lib::db::{DEFAULT_RECENT_OPENED_COUNT, Error, Vault};

fn recent(vault: &Vault) -> Vec<String> {
    vault.recent_opened_notes().unwrap().into_iter().map(|n| n.title).collect()
}

fn notes(vault: &mut Vault, n: usize) -> Vec<i64> {
    (0..n).map(|i| vault.create_note("", &format!("N{i}"), "").unwrap()).collect()
}

#[test]
fn most_recent_first_without_duplicates() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 3);
    assert!(recent(&vault).is_empty());
    for &id in &[ids[0], ids[1], ids[2], ids[0]] {
        vault.record_opened(id).unwrap();
    }
    assert_eq!(recent(&vault), ["N0", "N2", "N1"]);
    vault.record_opened(ids[0]).unwrap();
    assert_eq!(recent(&vault), ["N0", "N2", "N1"], "reopening the current note changes nothing");
}

#[test]
fn keeps_at_most_the_count_and_truncates_when_it_is_lowered() {
    let (_dir, mut vault) = common::vault();
    assert_eq!(vault.recent_opened_count().unwrap(), DEFAULT_RECENT_OPENED_COUNT);
    let ids = notes(&mut vault, 12);
    for &id in &ids {
        vault.record_opened(id).unwrap();
    }
    assert_eq!(recent(&vault).len(), 10);
    assert_eq!(recent(&vault)[0], "N11");
    vault.set_recent_opened_count(3).unwrap();
    assert_eq!(recent(&vault), ["N11", "N10", "N9"]);
    // Raising it again doesn't bring the dropped ones back.
    vault.set_recent_opened_count(50).unwrap();
    assert_eq!(recent(&vault), ["N11", "N10", "N9"]);
    assert!(matches!(vault.set_recent_opened_count(51), Err(Error::InvalidRecentOpenedCount)));
    assert_eq!(vault.recent_opened_count().unwrap(), 50);
}

#[test]
fn zero_records_nothing_and_clears_the_list() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 2);
    vault.record_opened(ids[0]).unwrap();
    vault.set_recent_opened_count(0).unwrap();
    assert!(recent(&vault).is_empty());
    vault.record_opened(ids[1]).unwrap();
    assert!(recent(&vault).is_empty());
    vault.set_recent_opened_count(5).unwrap();
    assert!(recent(&vault).is_empty(), "nothing was recorded while it was off");
}

#[test]
fn clear_empties_the_list() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 2);
    vault.record_opened(ids[0]).unwrap();
    vault.clear_recent_opened().unwrap();
    assert!(recent(&vault).is_empty());
}

#[test]
fn trashed_notes_are_hidden_and_come_back_on_restore() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 3);
    for &id in &ids {
        vault.record_opened(id).unwrap();
    }
    vault.trash_note(ids[1]).unwrap();
    assert_eq!(recent(&vault), ["N2", "N0"]);
    vault.restore_note(ids[1]).unwrap();
    assert_eq!(recent(&vault), ["N2", "N1", "N0"], "back in its old place");
}

#[test]
fn purged_notes_are_pruned() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 3);
    for &id in &ids {
        vault.record_opened(id).unwrap();
    }
    vault.delete_note(ids[1]).unwrap();
    assert_eq!(recent(&vault), ["N2", "N0"]);
    let stored = vault.setting("recent_opened").unwrap().unwrap();
    assert_eq!(stored, format!("[{},{}]", ids[2], ids[0]), "the stored list lost the id");
}

#[test]
fn rename_and_move_keep_the_entry() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 2);
    vault.record_opened(ids[0]).unwrap();
    vault.record_opened(ids[1]).unwrap();
    vault.rename_note(ids[0], "elsewhere/deep", "Renamed").unwrap();
    let list = vault.recent_opened_notes().unwrap();
    assert_eq!(list.iter().map(|n| (n.folder.as_str(), n.title.as_str())).collect::<Vec<_>>(), [("", "N1"), ("elsewhere/deep", "Renamed")]);
}

#[test]
fn an_unreadable_stored_list_is_treated_as_empty() {
    let (_dir, mut vault) = common::vault();
    let ids = notes(&mut vault, 1);
    vault.set_setting("recent_opened", "not json").unwrap();
    assert!(recent(&vault).is_empty());
    vault.record_opened(ids[0]).unwrap();
    assert_eq!(recent(&vault), ["N0"]);
}

#[test]
fn each_vault_has_its_own_list_and_it_survives_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (a_path, b_path) = (dir.path().join("a.hush"), dir.path().join("b.hush"));
    let mut a = Vault::create(&a_path, common::PASS).unwrap();
    let mut b = Vault::create(&b_path, common::PASS).unwrap();
    let in_a = a.create_note("", "In A", "").unwrap();
    b.create_note("", "In B", "").unwrap();
    a.record_opened(in_a).unwrap();
    assert!(recent(&b).is_empty());
    a.close().unwrap();
    let a = Vault::open(&a_path, common::PASS).unwrap();
    assert_eq!(recent(&a), ["In A"]);
}

/// The list lives in the encrypted vault only: the plaintext app config, written as on
/// unlock, holds neither note ids nor titles after notes are opened.
#[test]
fn config_json_holds_no_note_ids_or_titles() {
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.hush");
    let config = Config::new(dir.path().join("config.json"));
    let mut vault = Vault::create(&vault_path, common::PASS).unwrap();
    config.update(|c| c.remember(&vault_path)).unwrap();
    let titles = ["Secret plan", "Diary entry"];
    let ids: Vec<i64> = titles.iter().map(|t| vault.create_note("", t, "").unwrap()).collect();
    for &id in &ids {
        vault.record_opened(id).unwrap();
    }
    vault.close().unwrap();
    config.update(|c| c.remember(&vault_path)).unwrap();

    let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
    for t in titles {
        assert!(!text.contains(t), "{text}");
    }
    assert!(!text.contains("recent_opened"), "{text}");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["recent_vaults"], "vault paths only");
    assert_eq!(json["recent_vaults"][0].as_str().map(Path::new), Some(vault_path.as_path()));
}
