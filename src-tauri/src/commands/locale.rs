//! The operating system's date and time format (#202).

/// The locale the OS formats dates with (`nl-NL`, `nl_NL.UTF-8`), or `null`. Asked once,
/// before the first render.
#[tauri::command]
pub fn os_locale() -> Option<String> {
    crate::locale::os_locale()
}
