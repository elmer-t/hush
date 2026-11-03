//! Optional models: download (complete, resumed, from a server without ranges, with a
//! wrong checksum, cancelled), manual import and guarded removal. Downloads go to a
//! local HTTP server; Hugging Face itself isn't reachable from the test machine.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hush_lib::db::Vault;
use hush_lib::embed::download::{self, DownloadError, Downloader, ImportError};
use hush_lib::embed::models::{self, sha256_hex};
use hush_lib::embed::{self, ChunkPooling, ModelDescriptor, ModelFile, TokenPooling};
use hush_lib::session::Session;

/// How the test server answers.
#[derive(Clone, Default)]
struct Behaviour {
    /// Ignore `Range` and always send the whole file.
    no_ranges: bool,
    /// Close the connection after this many body bytes, on the first request only.
    drop_first_after: Option<usize>,
    /// Answer every request with this status and no body.
    status: Option<u16>,
}

/// Each request's `Range` header and the body bytes sent.
type Requests = Arc<Mutex<Vec<(Option<String>, usize)>>>;

/// A one-file HTTP/1.1 server on localhost; records each request's `Range` header and
/// the body bytes it sent.
struct Server {
    url: String,
    requests: Requests,
}

fn serve(body: Vec<u8>, behaviour: Behaviour) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model.onnx", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(stream) = stream else { continue };
            answer(stream, &body, &behaviour, n == 0, &log);
        }
    });
    Server { url, requests }
}

fn answer(mut stream: TcpStream, body: &[u8], b: &Behaviour, first: bool, log: &Requests) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range:") {
            range = Some(v.trim().to_string());
        }
    }
    if let Some(status) = b.status {
        let _ = write!(stream, "HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        log.lock().unwrap().push((range, 0));
        return;
    }
    let start = match &range {
        Some(r) if !b.no_ranges => r.strip_prefix("bytes=").and_then(|r| r.trim_end_matches('-').parse::<usize>().ok()),
        _ => None,
    };
    let (head, slice) = match start {
        Some(s) if s >= body.len() => {
            let _ = write!(stream, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            log.lock().unwrap().push((range, 0));
            return;
        }
        Some(s) => (
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {s}-{}/{}\r\nConnection: close\r\n\r\n",
                body.len() - s,
                body.len() - 1,
                body.len()
            ),
            &body[s..],
        ),
        None => (
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
            body,
        ),
    };
    let slice = match b.drop_first_after {
        Some(n) if first => &slice[..n],
        _ => slice,
    };
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(slice);
    let _ = stream.flush();
    log.lock().unwrap().push((range, slice.len()));
}

/// A body big enough for several progress reports.
fn body() -> Vec<u8> {
    (0..600_000u32).map(|i| (i % 251) as u8).collect()
}

fn sha(bytes: &[u8]) -> &'static str {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f"), bytes).unwrap();
    Box::leak(sha256_hex(&dir.path().join("f")).unwrap().into_boxed_str())
}

fn file(url: &str, bytes: &[u8]) -> ModelFile {
    ModelFile { name: "m.onnx", url: Box::leak(url.to_string().into_boxed_str()), sha256: sha(bytes), size: bytes.len() as u64 }
}

fn fetch(dir: &Path, file: &ModelFile, cancel: &AtomicBool) -> Result<(), DownloadError> {
    Downloader::allowing_http().file(dir, file, cancel, &mut |_| {})
}

fn not_cancelled() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn a_complete_download_is_verified_and_renamed() {
    let bytes = body();
    let server = serve(bytes.clone(), Behaviour::default());
    let dir = tempfile::tempdir().unwrap();
    let f = file(&server.url, &bytes);
    let mut seen = Vec::new();
    Downloader::allowing_http().file(dir.path(), &f, &not_cancelled(), &mut |d| seen.push(d)).unwrap();
    assert_eq!(std::fs::read(dir.path().join("m.onnx")).unwrap(), bytes);
    assert!(!download::part_path(dir.path(), &f).exists());
    assert_eq!(seen.last(), Some(&(bytes.len() as u64)));
    assert!(seen.len() > 2, "progress along the way: {seen:?}");
    // Already there: no second request.
    fetch(dir.path(), &f, &not_cancelled()).unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_dropped_download_resumes_with_a_range_request() {
    let bytes = body();
    let server = serve(bytes.clone(), Behaviour { drop_first_after: Some(200_000), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let f = file(&server.url, &bytes);

    let first = fetch(dir.path(), &f, &not_cancelled());
    assert!(matches!(first, Err(DownloadError::Dropped(_))), "{first:?}");
    assert!(!dir.path().join("m.onnx").exists(), "never renamed unverified");
    let kept = std::fs::metadata(download::part_path(dir.path(), &f)).unwrap().len();
    assert!(kept > 0 && kept <= 200_000, "{kept}");

    fetch(dir.path(), &f, &not_cancelled()).unwrap();
    assert_eq!(std::fs::read(dir.path().join("m.onnx")).unwrap(), bytes);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests[1].0.as_deref(), Some(format!("bytes={kept}-").as_str()));
    assert_eq!(requests[1].1, bytes.len() - kept as usize, "only the rest was sent");
}

#[test]
fn a_server_without_ranges_restarts_the_file() {
    let bytes = body();
    let server =
        serve(bytes.clone(), Behaviour { no_ranges: true, drop_first_after: Some(100_000), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let f = file(&server.url, &bytes);
    assert!(fetch(dir.path(), &f, &not_cancelled()).is_err());
    fetch(dir.path(), &f, &not_cancelled()).unwrap();
    assert_eq!(std::fs::read(dir.path().join("m.onnx")).unwrap(), bytes);
    let requests = server.requests.lock().unwrap();
    assert!(requests[1].0.is_some(), "it asked for the rest");
    assert_eq!(requests[1].1, bytes.len(), "and got the whole file, which replaced the part");
}

#[test]
fn a_cancelled_download_keeps_its_part_and_continues() {
    let bytes = body();
    let server = serve(bytes.clone(), Behaviour::default());
    let dir = tempfile::tempdir().unwrap();
    let f = file(&server.url, &bytes);
    let cancel = AtomicBool::new(false);
    let r = Downloader::allowing_http().file(dir.path(), &f, &cancel, &mut |d| {
        if d > 0 {
            cancel.store(true, Ordering::SeqCst);
        }
    });
    assert!(matches!(r, Err(DownloadError::Cancelled)), "{r:?}");
    let kept = std::fs::metadata(download::part_path(dir.path(), &f)).unwrap().len();
    assert!(kept > 0 && kept < bytes.len() as u64, "{kept}");
    fetch(dir.path(), &f, &not_cancelled()).unwrap();
    assert_eq!(std::fs::read(dir.path().join("m.onnx")).unwrap(), bytes);
    assert_eq!(server.requests.lock().unwrap()[1].0.as_deref(), Some(format!("bytes={kept}-").as_str()));
}

#[test]
fn a_wrong_checksum_deletes_the_file_and_it_never_loads() {
    let bytes = body();
    let server = serve(bytes.clone(), Behaviour::default());
    let dir = tempfile::tempdir().unwrap();
    let f = ModelFile { sha256: sha(b"something else"), ..file(&server.url, &bytes) };
    let r = fetch(dir.path(), &f, &not_cancelled());
    match r {
        Err(DownloadError::Checksum { expected, actual, .. }) => {
            assert_eq!(expected, f.sha256);
            assert_eq!(actual, sha(&bytes));
        }
        r => panic!("{r:?}"),
    }
    assert!(!download::part_path(dir.path(), &f).exists());
    assert!(!dir.path().join("m.onnx").exists());
}

#[test]
fn http_errors_are_reported_with_their_status() {
    let server = serve(Vec::new(), Behaviour { status: Some(404), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let r = fetch(dir.path(), &file(&server.url, b"x"), &not_cancelled());
    assert!(matches!(r, Err(DownloadError::Status(404))), "{r:?}");
    assert!(r.unwrap_err().to_string().contains("HTTP 404"));
}

#[test]
fn nothing_listening_is_a_connection_error() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let dir = tempfile::tempdir().unwrap();
    let r = fetch(dir.path(), &file(&format!("http://127.0.0.1:{port}/m"), b"x"), &not_cancelled());
    assert!(matches!(r, Err(DownloadError::Connect(_))), "{r:?}");
}

/// A model whose two files are `model` and `tokenizer`.
fn descriptor(model: &[u8], tokenizer: &[u8], bundled: bool) -> &'static ModelDescriptor {
    Box::leak(Box::new(ModelDescriptor {
        id: "stub",
        name: "Stub model",
        licence: "MIT",
        licence_url: "",
        bundled,
        model: ModelFile { name: "stub.onnx", url: "https://example.invalid/m", sha256: sha(model), size: model.len() as u64 },
        tokenizer: ModelFile {
            name: "stub.tokenizer.json",
            url: "https://example.invalid/t",
            sha256: sha(tokenizer),
            size: tokenizer.len() as u64,
        },
        dims: 4,
        max_tokens: 8,
        token_pooling: TokenPooling::Mean,
        chunk_pooling: ChunkPooling::Mean,
        prefix: "",
        query_prefix: "",
        suggestion_threshold: 0.5,
        gpu: false,
        batch_across_notes: false,
    }))
}

#[test]
fn import_accepts_the_right_files_by_checksum_and_refuses_others() {
    let d = descriptor(b"model bytes", b"{\"tokenizer\":1}", false);
    let picked = tempfile::tempdir().unwrap();
    let models = tempfile::tempdir().unwrap();
    // Names as downloaded by hand from the model's page.
    std::fs::write(picked.path().join("model_quantized.onnx"), b"model bytes").unwrap();
    std::fs::write(picked.path().join("tokenizer.json"), b"{\"tokenizer\":1}").unwrap();
    std::fs::write(picked.path().join("other.json"), b"{}").unwrap();

    assert_eq!(download::import_file(models.path(), d, &picked.path().join("model_quantized.onnx")).unwrap(), "stub.onnx");
    assert_eq!(
        download::import_file(models.path(), d, &picked.path().join("tokenizer.json")).unwrap(),
        "stub.tokenizer.json"
    );
    let checks = models::FileChecks::default();
    assert!(checks.availability(models.path(), d).is_available());

    match download::import_file(models.path(), d, &picked.path().join("other.json")) {
        Err(e @ ImportError::WrongFile { .. }) => {
            let message = e.to_string();
            assert!(message.contains(d.tokenizer.sha256) && message.contains(sha(b"{}")), "{message}");
        }
        r => panic!("{r:?}"),
    }
    let names: Vec<_> = std::fs::read_dir(models.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 2, "nothing else was copied: {names:?}");
}

#[test]
fn removal_is_refused_for_the_bundled_model_and_the_one_the_open_vault_uses() {
    let bundled = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    let optional = embed::MODELS.iter().find(|m| !m.bundled).unwrap();
    let session = Session::default();
    assert!(models::removal_blocked(&session, bundled).unwrap().unwrap().contains("ships with Hush"));
    assert_eq!(models::removal_blocked(&session, optional).unwrap(), None, "locked: nothing uses it");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Journal.hush");
    let vault = Vault::create(&path, common::PASS).unwrap();
    session.unlock(vault, path).unwrap();
    assert_eq!(models::removal_blocked(&session, optional).unwrap(), None);
    session.with_vault(|v| v.set_embedding_model(optional.id)).unwrap();
    let reason = models::removal_blocked(&session, optional).unwrap().unwrap();
    assert!(reason.contains("“Journal”"), "{reason}");

    // Removing deletes the files and any part.
    let d = descriptor(b"m", b"t", false);
    let models = tempfile::tempdir().unwrap();
    std::fs::write(models.path().join("stub.onnx"), b"m").unwrap();
    std::fs::write(models.path().join("stub.tokenizer.json.part"), b"t").unwrap();
    assert_eq!(download::remove_files(models.path(), d).unwrap(), 2);
    assert_eq!(std::fs::read_dir(models.path()).unwrap().count(), 0);
}

#[test]
fn a_model_download_reports_progress_over_both_files() {
    let model = body();
    let tokenizer = b"{\"t\":1}".to_vec();
    let server = serve(model.clone(), Behaviour::default());
    let tok_server = serve(tokenizer.clone(), Behaviour::default());
    let mut d = *descriptor(&model, &tokenizer, false);
    d.model.url = Box::leak(server.url.clone().into_boxed_str());
    d.tokenizer.url = Box::leak(tok_server.url.clone().into_boxed_str());
    let dir = tempfile::tempdir().unwrap();
    let mut last = None;
    Downloader::allowing_http().model(dir.path(), &d, &not_cancelled(), &mut |p| last = Some(p)).unwrap();
    let last = last.unwrap();
    assert_eq!((last.done, last.total), ((model.len() + tokenizer.len()) as u64, (model.len() + tokenizer.len()) as u64));
    assert!(models::FileChecks::default().availability(dir.path(), &d).is_available());
}
