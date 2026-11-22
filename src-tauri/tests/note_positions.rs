mod common;

use hush_lib::config::Config;
use hush_lib::db::{NOTE_POSITIONS_SETTING, NotePosition, Vault};

fn at(anchor: u32, head: u32, top: u32) -> NotePosition {
    NotePosition { anchor, head, top }
}

/// `n` notes, each opened once, so the last is at the front of the recently opened list.
fn opened_notes(vault: &mut Vault, n: usize) -> Vec<i64> {
    let ids: Vec<i64> = (0..n).map(|i| vault.create_note("", &format!("N{i}"), "body").unwrap()).collect();
    for &id in &ids {
        vault.record_opened(id).unwrap();
    }
    ids
}

#[test]
fn a_position_is_kept_per_note_and_survives_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let mut vault = Vault::create(&path, common::PASS).unwrap();
    let ids = opened_notes(&mut vault, 2);
    assert_eq!(vault.note_position(ids[0]).unwrap(), None);

    vault.set_note_position(ids[0], at(120, 140, 80)).unwrap();
    vault.set_note_position(ids[1], at(5, 5, 0)).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), Some(at(120, 140, 80)));
    vault.set_note_position(ids[0], at(7, 7, 3)).unwrap();
    vault.close().unwrap();

    let vault = Vault::open(&path, common::PASS).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), Some(at(7, 7, 3)));
    assert_eq!(vault.note_position(ids[1]).unwrap(), Some(at(5, 5, 0)));
}

#[test]
fn only_notes_in_the_recently_opened_list_get_a_position() {
    let (_dir, mut vault) = common::vault();
    let never_opened = vault.create_note("", "Never opened", "").unwrap();
    vault.set_note_position(never_opened, at(1, 2, 3)).unwrap();
    assert_eq!(vault.note_position(never_opened).unwrap(), None);
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None, "nothing was written");
}

#[test]
fn a_note_that_drops_off_the_list_loses_its_position() {
    let (_dir, mut vault) = common::vault();
    vault.set_recent_opened_count(2).unwrap();
    let ids = opened_notes(&mut vault, 2);
    vault.set_note_position(ids[0], at(10, 10, 10)).unwrap();
    vault.set_note_position(ids[1], at(20, 20, 20)).unwrap();

    let third = vault.create_note("", "Third", "").unwrap();
    vault.record_opened(third).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), None, "pushed off the list");
    assert_eq!(vault.note_position(ids[1]).unwrap(), Some(at(20, 20, 20)));
    let stored = vault.setting(NOTE_POSITIONS_SETTING).unwrap().unwrap();
    assert!(!stored.contains(&format!("\"{}\"", ids[0])), "{stored}");

    // Lowering the count cuts the positions with the list.
    vault.set_recent_opened_count(1).unwrap();
    assert_eq!(vault.note_position(ids[1]).unwrap(), None);
}

#[test]
fn clearing_the_list_clears_the_positions() {
    let (_dir, mut vault) = common::vault();
    let ids = opened_notes(&mut vault, 2);
    vault.set_note_position(ids[0], at(1, 1, 1)).unwrap();
    vault.clear_recent_opened().unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), None);
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None);
}

/// History off is a privacy choice: where you were in a note goes against it.
#[test]
fn with_history_off_no_position_is_written() {
    let (_dir, mut vault) = common::vault();
    let ids = opened_notes(&mut vault, 1);
    vault.set_note_position(ids[0], at(1, 1, 1)).unwrap();
    vault.set_recent_opened_count(0).unwrap();
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None, "turning history off deletes them");
    vault.record_opened(ids[0]).unwrap();
    vault.set_note_position(ids[0], at(2, 2, 2)).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), None);
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None);
}

#[test]
fn the_setting_is_on_by_default_and_off_deletes_the_positions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.hush");
    let mut vault = Vault::create(&path, common::PASS).unwrap();
    assert!(vault.restore_position().unwrap());
    let ids = opened_notes(&mut vault, 1);
    vault.set_note_position(ids[0], at(9, 9, 9)).unwrap();

    vault.set_restore_position(false).unwrap();
    assert!(!vault.restore_position().unwrap());
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None, "the stored positions are gone");
    assert_eq!(vault.note_position(ids[0]).unwrap(), None);
    vault.set_note_position(ids[0], at(3, 3, 3)).unwrap();
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None, "and none is written while it is off");
    vault.close().unwrap();

    let vault = Vault::open(&path, common::PASS).unwrap();
    assert!(!vault.restore_position().unwrap(), "the choice is kept in the vault");
    vault.set_restore_position(true).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), None, "turning it on again starts empty");
    vault.set_note_position(ids[0], at(4, 4, 4)).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), Some(at(4, 4, 4)));
}

/// A value this version can't read is no positions, and the next write replaces it.
#[test]
fn an_unreadable_stored_value_is_replaced() {
    let (_dir, mut vault) = common::vault();
    let ids = opened_notes(&mut vault, 1);
    for stored in ["", "not json", "[]", r#"{"x":{"anchor":1,"head":1,"top":1}}"#, r#"{"1":{"anchor":-1}}"#] {
        vault.set_setting(NOTE_POSITIONS_SETTING, stored).unwrap();
        assert_eq!(vault.note_position(ids[0]).unwrap(), None, "{stored:?}");
    }
    vault.set_note_position(ids[0], at(6, 6, 6)).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), Some(at(6, 6, 6)));
}

#[test]
fn storing_the_same_position_again_writes_nothing() {
    let (_dir, mut vault) = common::vault();
    let ids = opened_notes(&mut vault, 1);
    assert!(vault.set_note_position(ids[0], at(1, 2, 3)).unwrap());
    assert!(!vault.set_note_position(ids[0], at(1, 2, 3)).unwrap());
    assert!(vault.set_note_position(ids[0], at(1, 2, 4)).unwrap());
}

/// A note deleted for good is dropped from the list, and its position with it.
#[test]
fn a_deleted_note_loses_its_position() {
    let (_dir, mut vault) = common::vault();
    let ids = opened_notes(&mut vault, 2);
    vault.set_note_position(ids[0], at(1, 1, 1)).unwrap();
    vault.trash_note(ids[0]).unwrap();
    assert_eq!(vault.note_position(ids[0]).unwrap(), Some(at(1, 1, 1)), "a trashed note keeps it for a restore");
    vault.delete_note(ids[0]).unwrap();
    vault.recent_opened_notes().unwrap();
    assert_eq!(vault.setting(NOTE_POSITIONS_SETTING).unwrap(), None);
}

/// Which notes you read, and where in them, is vault data: the plaintext app config,
/// written as on unlock, is byte for byte what it is without stored positions.
#[test]
fn config_json_holds_no_positions() {
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.hush");
    let config_path = dir.path().join("config.json");
    let config = Config::new(config_path.clone());
    let mut vault = Vault::create(&vault_path, common::PASS).unwrap();
    config.update(|c| c.remember(&vault_path)).unwrap();
    let without = std::fs::read(&config_path).unwrap();

    let ids = opened_notes(&mut vault, 1);
    vault.set_note_position(ids[0], at(424242, 434343, 414141)).unwrap();
    vault.set_restore_position(true).unwrap();
    vault.close().unwrap();
    config.update(|c| c.remember(&vault_path)).unwrap();

    let with = std::fs::read(&config_path).unwrap();
    assert_eq!(with, without);
    let text = String::from_utf8(with).unwrap();
    for word in ["424242", "434343", "414141", "position"] {
        assert!(!text.contains(word), "{text}");
    }
}
