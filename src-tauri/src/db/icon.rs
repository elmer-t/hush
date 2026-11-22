//! The vault's icon, shown next to its name in the top bar while it is unlocked (#241).
//! It is kept in the vault's encrypted `settings` table and nowhere else: a copy outside
//! the vault would tell what the vault is for before the passphrase is entered (see
//! `docs/decisions.md`). So it is not shown on the unlock screen.
//!
//! An icon is two IDs, a shape and a colour. The lists of both belong to the frontend:
//! this side only checks the form, and an ID a frontend doesn't know draws the Hush mark.

use serde::{Deserialize, Serialize};

use super::{Error, Result, Vault};

/// JSON `{"icon":"<id>","color":"<id>"}`. No key means no icon.
pub const VAULT_ICON_SETTING: &str = "vault_icon";
const MAX_ID_CHARS: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultIcon {
    pub icon: String,
    pub color: String,
}

impl VaultIcon {
    fn valid(&self) -> bool {
        valid_id(&self.icon) && valid_id(&self.color)
    }
}

/// `[a-z0-9-]{1,32}`.
fn valid_id(id: &str) -> bool {
    (1..=MAX_ID_CHARS).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Vault {
    /// The vault's icon. A stored value this version can't read counts as none, so a
    /// vault another version wrote to still opens.
    pub fn vault_icon(&self) -> Result<Option<VaultIcon>> {
        Ok(self
            .setting(VAULT_ICON_SETTING)?
            .and_then(|v| serde_json::from_str::<VaultIcon>(&v).ok())
            .filter(VaultIcon::valid))
    }

    /// Stores the icon; `None` removes it.
    pub fn set_vault_icon(&self, icon: Option<&VaultIcon>) -> Result<()> {
        match icon {
            Some(icon) if !icon.valid() => Err(Error::InvalidVaultIcon),
            Some(icon) => {
                self.set_setting(VAULT_ICON_SETTING, &serde_json::to_string(icon).expect("an icon serialises"))
            }
            None => self.delete_setting(VAULT_ICON_SETTING),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No migration: a vault with an icon has the tables, columns and schema version of
    /// one without, so an older Hush opens it.
    #[test]
    fn setting_an_icon_changes_no_schema() {
        let dir = tempfile::tempdir().unwrap();
        let schema = |name: &str, set: bool| -> (i64, Vec<String>) {
            let vault = Vault::create(&dir.path().join(name), "correct horse battery staple").unwrap();
            if set {
                vault.set_vault_icon(Some(&VaultIcon { icon: "key".into(), color: "teal".into() })).unwrap();
            }
            let version = vault.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
            let mut stmt = vault.conn.prepare("SELECT coalesce(sql, name) FROM sqlite_master ORDER BY name").unwrap();
            let sql = stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            (version, sql)
        };
        assert_eq!(schema("with.hush", true), schema("without.hush", false));
    }
}
