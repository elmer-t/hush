//! App config kept outside any vault. It holds vault paths only, never content or key material.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub recent_vaults: Vec<PathBuf>,
    /// The indexing job's model runs on every core instead of half of them. A choice
    /// for this machine, so it lives here rather than in a vault.
    #[serde(default)]
    pub index_all_cores: bool,
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

/// The config file, as Tauri state. Several Hush processes can run at once (each on its
/// own vault) and share this file, so it is read fresh every time instead of cached,
/// and every change is a locked read-modify-write.
pub struct Config {
    path: PathBuf,
    /// Serialises updates within this process; the file lock does so across processes.
    writing: Mutex<()>,
}

impl Config {
    pub fn new(path: PathBuf) -> Self {
        Self { path, writing: Mutex::new(()) }
    }

    /// The config as it is on disk now, including changes by other instances.
    pub fn get(&self) -> AppConfig {
        AppConfig::load(&self.path)
    }

    /// Applies `f` to the config as it is on disk and saves the result, holding an OS
    /// lock (`config.json.lock`) so a concurrent update by another instance is never lost.
    pub fn update(&self, f: impl FnOnce(&mut AppConfig)) -> io::Result<()> {
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = self.lock()?;
        let mut data = AppConfig::load(&self.path);
        f(&mut data);
        data.save(&self.path)
    }

    /// Blocks until no other process is updating the config. Released when dropped.
    fn lock(&self) -> io::Result<File> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.with_extension("json.lock"))?;
        file.lock()?;
        Ok(file)
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

    #[test]
    fn concurrent_updates_from_two_instances_are_all_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Two `Config`s on one file stand in for two processes.
        let a = Config::new(path.clone());
        let b = Config::new(path.clone());
        std::thread::scope(|s| {
            for (config, name) in [(&a, "a"), (&b, "b")] {
                s.spawn(move || {
                    for i in 0..5 {
                        config.update(|c| c.remember(Path::new(&format!("{name}{i}.hush")))).unwrap();
                    }
                });
            }
        });
        let recent = a.get().recent_vaults;
        assert_eq!(recent.len(), MAX_RECENT);
        for name in ["a", "b"] {
            for i in 0..5 {
                assert!(recent.contains(&PathBuf::from(format!("{name}{i}.hush"))), "{name}{i} was lost");
            }
        }
        assert_eq!(b.get(), a.get(), "each instance sees the other's changes");
    }
}
