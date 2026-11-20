//! The operating system's regional-format locale, for showing dates and times (#202).
//!
//! The webview's own default is not this: WebView2 follows the Windows region only when
//! its language matches the display language, so English Windows with a Dutch region
//! formatted dates the US way. The frontend takes this name and hands it to `Intl`
//! (`src/lib/dates.ts`), which also tidies the Unix spelling (`nl_NL.UTF-8`).

/// The locale the OS formats dates with, as the OS names it, or `None` when it has none
/// to offer (the frontend then keeps the webview's default).
///
/// Windows: the regional format (Settings → Time & language → Language & region), not
/// the display language. Elsewhere: `LC_ALL`, `LC_TIME`, then `LANG`, the order the C
/// library uses for dates. macOS apps started from Finder have none of the three set.
pub fn os_locale() -> Option<String> {
    #[cfg(windows)]
    {
        windows_locale()
    }
    #[cfg(not(windows))]
    {
        from_env(|name| std::env::var(name).ok())
    }
}

#[cfg(windows)]
fn windows_locale() -> Option<String> {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    // LOCALE_NAME_MAX_LENGTH, terminator included.
    let mut name = [0u16; 85];
    // SAFETY: the buffer is valid for its whole length for the duration of the call.
    let written = unsafe { GetUserDefaultLocaleName(&mut name) };
    // The count includes the terminator; 0 is failure.
    let len = usize::try_from(written).ok()?.checked_sub(1)?;
    let name = String::from_utf16(name.get(..len)?).ok()?;
    (!name.is_empty()).then_some(name)
}

/// The first of `LC_ALL`, `LC_TIME` and `LANG` that is set to something.
#[cfg_attr(windows, allow(dead_code))]
fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LC_ALL", "LC_TIME", "LANG"].into_iter().filter_map(var).find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }

    #[test]
    fn the_date_setting_wins_over_the_language() {
        let vars = [("LANG", "en_US.UTF-8"), ("LC_TIME", "nl_NL.UTF-8")];
        assert_eq!(from_env(env(&vars)).as_deref(), Some("nl_NL.UTF-8"));
    }

    #[test]
    fn lc_all_overrides_the_date_setting() {
        let vars = [("LC_TIME", "nl_NL.UTF-8"), ("LC_ALL", "de_DE.UTF-8")];
        assert_eq!(from_env(env(&vars)).as_deref(), Some("de_DE.UTF-8"));
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let vars = [("LC_ALL", ""), ("LANG", "en_GB.UTF-8")];
        assert_eq!(from_env(env(&vars)).as_deref(), Some("en_GB.UTF-8"));
    }

    #[test]
    fn nothing_set_gives_no_locale() {
        assert_eq!(from_env(env(&[])), None);
    }
}
