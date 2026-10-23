//! App config kept outside any vault. It holds vault paths only, never content or key material.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub recent_vaults: Vec<PathBuf>,
}

impl AppConfig {
    /// Loads the config; a missing or unreadable file gives the default.
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes to a temp file first so a crash never leaves a half-written config.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    /// Moves `vault` to the front of the recent list.
    pub fn remember(&mut self, vault: &Path) {
        self.forget(vault);
        self.recent_vaults.insert(0, vault.to_path_buf());
        self.recent_vaults.truncate(MAX_RECENT);
    }

    pub fn forget(&mut self, vault: &Path) {
        self.recent_vaults.retain(|p| !same_path(p, vault));
    }
}

/// Windows paths are case-insensitive.
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
    } else {
        a == b
    }
}

/// The config and the file it lives in, as Tauri state.
pub struct Config {
    path: PathBuf,
    data: Mutex<AppConfig>,
}

impl Config {
    pub fn load(path: PathBuf) -> Self {
        let data = Mutex::new(AppConfig::load(&path));
        Self { path, data }
    }

    pub fn get(&self) -> MutexGuard<'_, AppConfig> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Applies `f` and saves the result.
    pub fn update(&self, f: impl FnOnce(&mut AppConfig)) -> io::Result<()> {
        let mut data = self.get();
        f(&mut data);
        data.save(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_moves_to_front_dedupes_and_caps() {
        let mut config = AppConfig::default();
        for i in 0..12 {
            config.remember(Path::new(&format!("v{i}.hush")));
        }
        config.remember(Path::new("v5.hush"));
        assert_eq!(config.recent_vaults.len(), MAX_RECENT);
        assert_eq!(config.recent_vaults[0], Path::new("v5.hush"));
        assert_eq!(config.recent_vaults.iter().filter(|p| p.ends_with("v5.hush")).count(), 1);
        config.forget(Path::new("v5.hush"));
        assert_eq!(config.recent_vaults[0], Path::new("v11.hush"));
    }

    #[test]
    fn save_load_roundtrip_and_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.json");
        assert_eq!(AppConfig::load(&path), AppConfig::default());

        let mut config = AppConfig::default();
        config.remember(Path::new("a.hush"));
        config.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path), config);

        fs::write(&path, "{ not json").unwrap();
        assert_eq!(AppConfig::load(&path), AppConfig::default());
    }
}
