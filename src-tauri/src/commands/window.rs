//! Showing the main window on start (#239).
//!
//! The window is created hidden (`visible: false` in `tauri.conf.json`), so the
//! window-state plugin restores its size and position before anyone sees it. The
//! frontend calls `window_ready` once its first render is in the DOM. Until the page
//! paints, the window shows its background colour, which is set here to the theme's.
//!
//! The theme is chosen in the webview's `localStorage`, which can't be read before the
//! page loads. So the frontend reports the scheme it resolved (`set_window_scheme`) and
//! it is kept in the app config for the next start.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::window::Color;
use tauri::{AppHandle, Manager, State, Theme, WebviewWindow};

use super::Result;
use crate::config::Config;

/// How long the frontend gets to ask for the window before it is shown anyway, so a
/// frontend that fails to load never leaves a process without a window.
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

/// `--bg` in `src/app.css`, light and dark; a test keeps them equal.
const BG_LIGHT: (u8, u8, u8) = (0xf6, 0xf6, 0xf3);
const BG_DARK: (u8, u8, u8) = (0x11, 0x13, 0x12);

/// Whether the main window has been shown since the app started.
#[derive(Default)]
pub struct StartupWindow {
    shown: AtomicBool,
}

impl StartupWindow {
    /// True for the first caller only. The page reloads on lock and asks again; a second
    /// show would pull a window the user had put in the background to the front.
    fn claim(&self) -> bool {
        !self.shown.swap(true, Ordering::SeqCst)
    }
}

fn background(dark: bool) -> Color {
    let (r, g, b) = if dark { BG_DARK } else { BG_LIGHT };
    Color(r, g, b, 255)
}

fn show_once(app: &AppHandle) {
    if !app.state::<StartupWindow>().claim() {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Called in `setup`, while the window is still hidden: gives it the background of the
/// scheme the last run ended in (the OS scheme on a first start), and starts the fallback.
pub fn prepare(app: &AppHandle, stored_dark: Option<bool>) {
    if let Some(window) = app.get_webview_window("main") {
        let dark = stored_dark.unwrap_or_else(|| matches!(window.theme(), Ok(Theme::Dark)));
        let _ = window.set_background_color(Some(background(dark)));
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        show_once(&app);
    });
}

/// The frontend's first render is in the DOM: the window can be shown.
#[tauri::command]
pub fn window_ready(app: AppHandle) {
    show_once(&app);
}

/// The frontend resolved its scheme (at start, and on every later change). The window's
/// background follows, and the scheme is stored for the next start.
#[tauri::command(async)]
pub fn set_window_scheme(window: WebviewWindow, config: State<'_, Config>, dark: bool) -> Result<()> {
    let _ = window.set_background_color(Some(background(dark)));
    if config.get().window_dark != Some(dark) {
        config.update(|c| c.window_dark = Some(dark))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_claim_shows_the_window() {
        let startup = StartupWindow::default();
        assert!(startup.claim());
        assert!(!startup.claim());
        assert!(!startup.claim());
    }

    fn hex((r, g, b): (u8, u8, u8)) -> String {
        format!("--bg: #{r:02x}{g:02x}{b:02x};")
    }

    /// The window's background is the page's, or the first frame differs from the rest.
    #[test]
    fn the_backgrounds_are_the_ones_in_app_css() {
        let css = include_str!("../../../src/app.css");
        let values: Vec<&str> = css.lines().map(str::trim).filter(|l| l.starts_with("--bg:")).collect();
        // Light once; dark twice (chosen, and following the OS).
        assert_eq!(values, [hex(BG_LIGHT), hex(BG_DARK), hex(BG_DARK)]);
    }
}
