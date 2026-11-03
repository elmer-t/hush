//! Dates as found in imported files: header lines, file names and frontmatter.

use std::sync::LazyLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use regex::Regex;

use crate::markdown::parse_date;

static NAME_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^0-9])((\d{4})-?(\d{2})-?(\d{2}))(?:[^0-9]|$)").unwrap());

/// Like [`parse_date`], plus mail-style (RFC 2822) dates, slashes and dots, and
/// spelled-out months (`2 January 2005`, `January 2, 2005`). Day-first for
/// numeric dates with the year last. Dates without an offset are taken as UTC.
pub fn parse_loose_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(ms) = parse_date(s) {
        return Some(ms);
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.timestamp_millis());
    }
    for fmt in ["%Y/%m/%d %H:%M:%S", "%Y/%m/%d %H:%M", "%d-%m-%Y %H:%M", "%d/%m/%Y %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc().timestamp_millis());
        }
    }
    let s = s.replace(',', "");
    ["%Y/%m/%d", "%Y.%m.%d", "%d-%m-%Y", "%d/%m/%Y", "%d.%m.%Y", "%d %B %Y", "%d %b %Y", "%B %d %Y", "%b %d %Y"]
        .iter()
        .find_map(|fmt| NaiveDate::parse_from_str(&s, fmt).ok())
        .map(midnight)
}

/// A `YYYY-MM-DD` or `YYYYMMDD` date in a file name.
pub fn date_in_name(name: &str) -> Option<i64> {
    NAME_DATE.captures_iter(name).find_map(|c| {
        let (y, m, d) = (c[2].parse().ok()?, c[3].parse().ok()?, c[4].parse().ok()?);
        NaiveDate::from_ymd_opt(y, m, d).map(midnight)
    })
}

/// For frontmatter: `2004-03-12` at midnight UTC, RFC 3339 otherwise.
pub fn format_date(ms: i64) -> String {
    let Some(dt) = DateTime::<Utc>::from_timestamp_millis(ms) else { return String::new() };
    if ms.rem_euclid(86_400_000) == 0 {
        dt.format("%Y-%m-%d").to_string()
    } else {
        dt.to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

fn midnight(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_dates() {
        let day = parse_date("2005-01-02");
        for s in ["2005-01-02", "2005/01/02", "2005.01.02", "02-01-2005", "02/01/2005", "2 January 2005", "January 2, 2005", "2 Jan 2005"] {
            assert_eq!(parse_loose_date(s), day, "{s}");
        }
        assert_eq!(parse_loose_date("Fri, 12 Mar 2004 07:45:00 +0100"), parse_date("2004-03-12T06:45:00Z"));
        assert_eq!(parse_loose_date("someday"), None);
    }

    #[test]
    fn dates_in_names() {
        assert_eq!(date_in_name("2004-03-12 jr Walk"), parse_date("2004-03-12"));
        assert_eq!(date_in_name("meeting 20210202"), parse_date("2021-02-02"));
        assert_eq!(date_in_name("thread-0001"), None);
        assert_eq!(date_in_name("2004-13-45 and 123456789"), None);
    }

    #[test]
    fn formatting() {
        assert_eq!(format_date(parse_date("2004-03-12").unwrap()), "2004-03-12");
        assert_eq!(format_date(parse_date("2004-03-12T06:45:00Z").unwrap()), "2004-03-12T06:45:00Z");
        assert_eq!(parse_date(&format_date(1_079_073_900_000)), Some(1_079_073_900_000));
    }
}
