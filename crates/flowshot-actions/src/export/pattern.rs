//! Filename pattern expansion and sanitization.

use chrono::{DateTime, Local};

/// Default filename pattern (strftime-style).
pub const DEFAULT_PATTERN: &str = "%F_%H-%M";

/// Expand a strftime-style pattern using the given timestamp.
///
/// A trailing bare `%` (not followed by a format specifier) is stripped.
#[must_use]
pub fn expand_pattern(pattern: &str, now: DateTime<Local>) -> String {
    // Strip trailing bare % (not part of %%)
    let trimmed = if pattern.ends_with('%') && !pattern.ends_with("%%") {
        &pattern[..pattern.len() - 1]
    } else {
        pattern
    };
    now.format(trimmed).to_string()
}

/// Sanitize a filename for safe filesystem use.
///
/// - `/` is replaced with U+2044 (fraction slash)
/// - `:` is replaced with `-`
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    name.replace('/', "\u{2044}").replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime, TimeZone};

    fn fixed_time() -> DateTime<Local> {
        let date = NaiveDate::from_ymd_opt(2026, 9, 25);
        let time = NaiveTime::from_hms_opt(14, 30, 45);
        match (date, time) {
            (Some(d), Some(t)) => {
                let naive = d.and_time(t);
                Local
                    .from_local_datetime(&naive)
                    .single()
                    .unwrap_or_else(Local::now)
            }
            _ => Local::now(),
        }
    }

    #[test]
    fn default_pattern_expands() {
        let result = expand_pattern(DEFAULT_PATTERN, fixed_time());
        assert_eq!(result, "2026-09-25_14-30");
    }

    #[test]
    fn trailing_percent_is_stripped() {
        let result = expand_pattern("%Y%", fixed_time());
        assert_eq!(result, "2026");
    }

    #[test]
    fn double_percent_not_stripped() {
        let result = expand_pattern("%%", fixed_time());
        assert_eq!(result, "%");
    }

    #[test]
    fn unicode_in_pattern_preserved() {
        let result = expand_pattern("截图_%H-%M", fixed_time());
        assert_eq!(result, "截图_14-30");
    }

    #[test]
    fn sanitize_replaces_slash_with_fraction() {
        assert_eq!(sanitize_filename("a/b/c"), "a\u{2044}b\u{2044}c");
    }

    #[test]
    fn sanitize_replaces_colon_with_dash() {
        assert_eq!(sanitize_filename("12:30:45"), "12-30-45");
    }

    #[test]
    fn sanitize_combined() {
        assert_eq!(
            sanitize_filename("2026/09/25 14:30"),
            "2026\u{2044}09\u{2044}25 14-30"
        );
    }
}
