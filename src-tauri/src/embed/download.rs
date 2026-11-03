//! Getting an optional model onto this computer: downloading it from the URL pinned in
//! its descriptor, or importing files the user already has. This is Hush's only network
//! code (see `docs/decisions.md`, Optional models): it runs only when the user clicks
//! Download, requests only the descriptor's URLs over HTTPS, and sends nothing but a
//! plain GET (with a `Range` header when resuming). No note content, vault path or
//! identifier is ever part of a request.
//!
//! A file is written to `<name>.part` and renamed to its final name only once the whole
//! file matches the pinned SHA-256, so an unverified file is never loaded. A `.part`
//! left by a cancel, a crash or a dropped connection is continued with a `Range`
//! request next time.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use super::models::sha256_hex;
use super::{ModelDescriptor, ModelFile};

/// Progress is reported at most once per this many bytes (and at the end of each file).
const REPORT_EVERY: u64 = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("only HTTPS downloads are allowed ({0})")]
    NotHttps(String),
    #[error("couldn't find {0}: the computer may be offline, or DNS isn't working")]
    HostNotFound(String),
    #[error("couldn't connect to {0}: the computer may be offline, or a firewall blocks it")]
    Connect(String),
    #[error("the connection dropped ({0}); Download again to continue where it stopped")]
    Dropped(String),
    #[error("the download timed out; Download again to continue where it stopped")]
    Timeout,
    #[error("secure connection failed: {0} (a proxy or antivirus that inspects HTTPS can cause this)")]
    Tls(String),
    #[error("the proxy refused the connection: {0}")]
    Proxy(String),
    #[error("the server answered HTTP {0}")]
    Status(u16),
    #[error("the download failed: {0}")]
    Other(String),
    #[error("could not write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("{file} doesn't match its pinned checksum (expected {expected}, got {actual}); it was deleted")]
    Checksum { file: String, expected: String, actual: String },
    #[error("cancelled")]
    Cancelled,
}

/// How far a download has got, over all of a model's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// Downloads model files. One per app; cheap to share.
pub struct Downloader {
    agent: Agent,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new()
    }
}

impl Downloader {
    /// HTTPS only (redirects too), through the proxy the environment or, on Windows,
    /// the system settings name, checking certificates against the system's store.
    pub fn new() -> Self {
        Self::with_https_only(true)
    }

    /// Also allows plain HTTP, for tests against a local server.
    #[doc(hidden)]
    pub fn allowing_http() -> Self {
        Self::with_https_only(false)
    }

    fn with_https_only(https_only: bool) -> Self {
        let config = Agent::config_builder()
            .https_only(https_only)
            .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
            .user_agent("Hush")
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build();
        Self { agent: config.into() }
    }

    /// Downloads every file of `model` that `dir` doesn't have yet (verified), reporting
    /// progress over all of them. Returns early with [`DownloadError::Cancelled`] once
    /// `cancel` is set; what was fetched so far stays in `.part` files.
    pub fn model(
        &self,
        dir: &Path,
        model: &ModelDescriptor,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), DownloadError> {
        let files = [model.tokenizer, model.model];
        let total = files.iter().map(|f| f.size).sum();
        let mut before = 0;
        for file in files {
            self.file(dir, &file, cancel, &mut |done| progress(Progress { done: before + done, total }))?;
            before += file.size;
        }
        Ok(())
    }

    /// Downloads one file into `dir` (see the module docs). `progress` gets the bytes of
    /// this file on disk so far.
    pub fn file(
        &self,
        dir: &Path,
        file: &ModelFile,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), DownloadError> {
        let target = dir.join(file.name);
        if fs::metadata(&target).is_ok_and(|m| m.len() == file.size) && sha256_hex(&target).is_ok_and(|h| h == file.sha256)
        {
            progress(file.size);
            return Ok(());
        }
        fs::create_dir_all(dir).map_err(|source| DownloadError::Write { path: dir.to_path_buf(), source })?;
        let part = part_path(dir, file);
        let write_err = |source| DownloadError::Write { path: part.clone(), source };

        let mut have = fs::metadata(&part).map_or(0, |m| m.len());
        if file.size > 0 && have > file.size {
            fs::remove_file(&part).map_err(write_err)?;
            have = 0;
        }
        // A full `.part` (the rename didn't happen) needs no request, only the check.
        if have == 0 || file.size == 0 || have < file.size {
            self.fetch(file, &part, have, cancel, progress)?;
        }

        let actual = sha256_hex(&part).map_err(write_err)?;
        if actual != file.sha256 {
            let _ = fs::remove_file(&part);
            return Err(DownloadError::Checksum {
                file: file.name.to_string(),
                expected: file.sha256.to_string(),
                actual,
            });
        }
        fs::rename(&part, &target).map_err(|source| DownloadError::Write { path: target.clone(), source })?;
        Ok(())
    }

    /// Fetches `file.url` into `part`, continuing after its first `have` bytes when the
    /// server supports ranges and starting over when it doesn't.
    fn fetch(
        &self,
        file: &ModelFile,
        part: &Path,
        have: u64,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), DownloadError> {
        if !file.url.starts_with("https://") && self.agent.config().https_only() {
            return Err(DownloadError::NotHttps(file.url.to_string()));
        }
        let host = host(file.url);
        let mut request = self.agent.get(file.url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = match request.call() {
            // The `.part` is already whole (or longer than the server's file): the
            // checksum decides.
            Err(ureq::Error::StatusCode(416)) if have > 0 => return Ok(()),
            r => r.map_err(|e| request_error(e, &host))?,
        };
        let resumed = response.status().as_u16() == 206 && content_range_start(&response) == Some(have);
        let write_err = |source| DownloadError::Write { path: part.to_path_buf(), source };
        let mut out = if resumed {
            OpenOptions::new().append(true).open(part).map_err(write_err)?
        } else {
            // The server sent the whole file (it ignores ranges, or this is the start).
            File::create(part).map_err(write_err)?
        };
        let mut done = if resumed { have } else { 0 };
        progress(done);
        let mut body = response.into_body().into_reader();
        let mut buf = vec![0; 64 * 1024];
        let mut reported = done;
        loop {
            if cancel.load(Ordering::SeqCst) {
                out.flush().map_err(write_err)?;
                return Err(DownloadError::Cancelled);
            }
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    // Keep what arrived, for the next attempt to continue from.
                    let _ = out.flush();
                    return Err(read_error(e));
                }
            };
            out.write_all(&buf[..n]).map_err(write_err)?;
            done += n as u64;
            if done - reported >= REPORT_EVERY {
                reported = done;
                progress(done);
            }
        }
        out.sync_all().map_err(write_err)?;
        progress(done);
        Ok(())
    }
}

/// Where a file is written while it downloads or is copied in.
pub fn part_path(dir: &Path, file: &ModelFile) -> PathBuf {
    dir.join(format!("{}.part", file.name))
}

fn host(url: &str) -> String {
    url.split("://").nth(1).and_then(|rest| rest.split(['/', '?']).next()).unwrap_or(url).to_string()
}

/// The first byte of a 206 response's `Content-Range: bytes <start>-<end>/<size>`.
fn content_range_start(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    let value = response.headers().get("content-range")?.to_str().ok()?;
    value.strip_prefix("bytes ")?.split('-').next()?.trim().parse().ok()
}

fn request_error(e: ureq::Error, host: &str) -> DownloadError {
    use ureq::Error as E;
    match e {
        E::StatusCode(code) => DownloadError::Status(code),
        E::HostNotFound => DownloadError::HostNotFound(host.to_string()),
        E::ConnectionFailed => DownloadError::Connect(host.to_string()),
        E::Timeout(_) => DownloadError::Timeout,
        E::RequireHttpsOnly(url) => DownloadError::NotHttps(url),
        E::Tls(m) => DownloadError::Tls(m.to_string()),
        E::Rustls(e) => DownloadError::Tls(e.to_string()),
        E::ConnectProxyFailed(m) => DownloadError::Proxy(m),
        E::InvalidProxyUrl => DownloadError::Proxy("the proxy setting isn't a valid URL".into()),
        E::Io(e) => match e.kind() {
            io::ErrorKind::ConnectionRefused => DownloadError::Connect(host.to_string()),
            io::ErrorKind::TimedOut => DownloadError::Timeout,
            _ => read_error(e),
        },
        e => DownloadError::Other(e.to_string()),
    }
}

fn read_error(e: io::Error) -> DownloadError {
    match e.kind() {
        io::ErrorKind::TimedOut => DownloadError::Timeout,
        _ => {
            // ureq wraps its own errors (a timeout while reading the body) in io errors.
            match e.get_ref().and_then(|inner| inner.downcast_ref::<ureq::Error>()) {
                Some(ureq::Error::Timeout(_)) => DownloadError::Timeout,
                _ => DownloadError::Dropped(e.to_string()),
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{name} isn't a file of {model}: its checksum is {actual}, expected {expected}")]
    WrongFile { name: String, model: String, expected: String, actual: String },
    #[error("could not read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("could not write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
}

/// Copies `source`, which must match one of `model`'s files by checksum (whatever its
/// name), into `dir` under that file's name. A file matching neither is refused with
/// the expected and actual checksums. Returns the name it was stored under.
pub fn import_file(dir: &Path, model: &ModelDescriptor, source: &Path) -> Result<&'static str, ImportError> {
    let actual = sha256_hex(source).map_err(|e| ImportError::Read { path: source.to_path_buf(), source: e })?;
    let files = [model.model, model.tokenizer];
    let Some(file) = files.iter().find(|f| f.sha256 == actual) else {
        // Name the file it was most likely meant to be, by its extension.
        let json = source.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let expected = if json { model.tokenizer.sha256 } else { model.model.sha256 };
        return Err(ImportError::WrongFile {
            name: source.file_name().map_or_else(|| source.display().to_string(), |n| n.to_string_lossy().into_owned()),
            model: model.name.to_string(),
            expected: expected.to_string(),
            actual,
        });
    };
    let write_err = |path: &Path| {
        let path = path.to_path_buf();
        move |source| ImportError::Write { path, source }
    };
    fs::create_dir_all(dir).map_err(write_err(dir))?;
    let part = part_path(dir, file);
    let target = dir.join(file.name);
    fs::copy(source, &part).map_err(write_err(&part))?;
    // Checked again after the copy: what lands in the folder is what was verified.
    match sha256_hex(&part) {
        Ok(h) if h == file.sha256 => {}
        Ok(h) => {
            let _ = fs::remove_file(&part);
            return Err(ImportError::WrongFile {
                name: file.name.to_string(),
                model: model.name.to_string(),
                expected: file.sha256.to_string(),
                actual: h,
            });
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            return Err(ImportError::Read { path: part, source: e });
        }
    }
    fs::rename(&part, &target).map_err(write_err(&target))?;
    Ok(file.name)
}

/// Deletes `model`'s files (and any `.part`s) from `dir`. Returns the bytes freed.
pub fn remove_files(dir: &Path, model: &ModelDescriptor) -> io::Result<u64> {
    let mut freed = 0;
    for file in [model.model, model.tokenizer] {
        for path in [dir.join(file.name), part_path(dir, &file)] {
            match fs::metadata(&path) {
                Ok(m) => {
                    fs::remove_file(&path)?;
                    freed += m.len();
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_content_range() {
        assert_eq!(host("https://huggingface.co/Xenova/x/resolve/main/a.onnx"), "huggingface.co");
        assert_eq!(host("http://127.0.0.1:8080/f?x=1"), "127.0.0.1:8080");
    }

    #[test]
    fn plain_http_is_refused_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let file = ModelFile { name: "f", url: "http://example.invalid/f", sha256: "", size: 1 };
        let r = Downloader::new().file(dir.path(), &file, &AtomicBool::new(false), &mut |_| {});
        assert!(matches!(r, Err(DownloadError::NotHttps(_))), "{r:?}");
        assert!(!part_path(dir.path(), &file).exists());
    }
}
