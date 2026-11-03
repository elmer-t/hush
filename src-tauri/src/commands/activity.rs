//! The Activity panel: what the background jobs are doing (see [`crate::activity`]),
//! sent to the frontend as one throttled event, plus the counts the panel shows and a
//! note's internals for the details panel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use super::Result;
use super::embeddings::Indexer;
use crate::activity::{HeadInfo, Snapshot};
use crate::db::NoteInternals;
use crate::embed::job::sleep_while_open;
use crate::session::Session;

/// Emitted with the whole [`Snapshot`] after a change, at most once per [`EMIT_INTERVAL`].
pub const ACTIVITY_EVENT: &str = "activity-changed";
/// At most four snapshots a second, however busy the jobs are.
const EMIT_INTERVAL: Duration = Duration::from_millis(250);
/// How often the counts are refreshed while the panel is open.
const STATS_INTERVAL: Duration = Duration::from_secs(2);

/// Whether the Activity panel is open; the counts are only refreshed then.
#[derive(Default)]
pub struct ActivityPanel {
    open: AtomicBool,
}

/// Starts the thread that sends snapshots to the frontend; runs for the app's lifetime.
pub fn start_emitter(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        session.activity().run_emitter(EMIT_INTERVAL, || false, |snapshot| {
            let _ = app.emit(ACTIVITY_EVENT, snapshot);
        });
    });
}

/// Starts the counts refresher for unlock `generation`. It stops by itself when the
/// vault locks.
pub fn start(app: &AppHandle, generation: u64) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        loop {
            if app.state::<ActivityPanel>().open.load(Ordering::SeqCst) {
                refresh_stats(&app, &session, generation);
            }
            if !sleep_while_open(&session, generation, STATS_INTERVAL) {
                return;
            }
        }
    });
}

/// Counts what the panel shows. The vault is read through the quiet accessor, so this
/// never postpones auto-lock, and only through index lookups (see `db::internals`).
fn refresh_stats(app: &AppHandle, session: &Session, generation: u64) {
    let Ok((notes, queued, model, ready, with_vector, heads, schema)) = session.with_vault_quiet(generation, |v| {
        let model = v.embedding_model()?;
        Ok((
            v.live_note_count()?,
            v.embedding_queue_len()?,
            model.clone(),
            v.embedding_model_ready()?,
            v.embedding_count(&model)?,
            v.tag_head_times()?,
            v.schema_version()?,
        ))
    }) else {
        return;
    };
    let (loaded, cores_used, cores_total) = app.state::<Indexer>().model_state(&model);
    let (related_notes, related_bytes) = match session.related_index().as_ref() {
        Some(index) => (Some(index.len() as u64), Some(index.memory_bytes() as u64)),
        None => (None, None),
    };
    session.activity().reporter(generation).stats(|s| {
        s.counted_at = Some(crate::db::now_ms());
        s.notes = Some(notes as u64);
        s.queued = Some(queued as u64);
        s.with_vector = Some(with_vector as u64);
        s.model = Some(model);
        s.ready_model = ready;
        s.model_loaded = loaded;
        s.cores_used = Some(cores_used as u64);
        s.cores_total = Some(cores_total as u64);
        s.related_notes = related_notes;
        s.related_bytes = related_bytes;
        s.heads = heads.into_iter().map(|h| HeadInfo { tag: h.tag, model: h.model, trained_at: h.trained_at }).collect();
        s.schema_version = Some(schema);
    });
}

/// Everything the Activity panel shows; changes arrive as [`ACTIVITY_EVENT`].
#[tauri::command]
pub fn activity_status(session: State<'_, Session>) -> Snapshot {
    session.activity().snapshot()
}

/// The panel opened or closed: counts are refreshed (at once, then on a timer) only
/// while it is open.
#[tauri::command]
pub fn set_activity_panel_open(app: AppHandle, panel: State<'_, ActivityPanel>, open: bool) {
    let was = panel.open.swap(open, Ordering::SeqCst);
    if open && !was {
        let session = app.state::<Session>();
        let generation = session.generation();
        if generation != 0 {
            refresh_stats(&app, &session, generation);
        }
    }
}

/// What Hush stores and derives for note `id`, for the details panel.
#[tauri::command]
pub fn note_internals(session: State<'_, Session>, id: i64) -> Result<NoteInternals> {
    Ok(session.with_vault(|v| v.note_internals(id))?)
}
