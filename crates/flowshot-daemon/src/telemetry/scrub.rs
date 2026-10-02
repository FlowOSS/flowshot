//! The pure path scrubber behind the sanitization hook.
//!
//! Rules (documented choice per the telemetry spec):
//!
//! - `/home/<user>/...` -> `~/...` - the explicit `$HOME` prefix AND any
//!   other user's home segment (error messages can quote paths `FlowShot`
//!   never owned).
//! - Temp-dir paths are truncated to `<dir>/<redacted>`: the basename is
//!   DROPPED, not hashed - `FlowShot`'s temp names carry pid+nonce
//!   (`flowshot-session-<pid>-<nonce>.json`), so a hash would correlate
//!   nothing worth correlating while still costing a digest. The literal
//!   `/tmp` rule always applies; a custom `TMPDIR` prefix is passed in.
//! - Rewrites happen per whitespace-delimited token, so paths embedded in
//!   prose (`... (/home/alice/x.png): io error`) are caught, and a
//!   path-component boundary check keeps `/var/tmp` from matching the
//!   `/tmp` rule mid-component.

use std::borrow::Cow;

/// The replacement for a redacted temp-dir basename.
const REDACTED: &str = "<redacted>";

/// Scrubs `text` against the `home` and custom-temp prefixes (`None` =
/// the `/tmp` default, which the literal rule always covers).
pub(crate) fn scrub_paths<'t>(
    text: &'t str,
    home: Option<&str>,
    temp: Option<&str>,
) -> Cow<'t, str> {
    let home = home.filter(|home| !home.is_empty());
    let temp = temp.filter(|temp| !temp.is_empty() && *temp != "/tmp");
    let candidate = text.contains("/home/")
        || text.contains("/tmp/")
        || home.is_some_and(|home| text.contains(home))
        || temp.is_some_and(|temp| text.contains(temp));
    if !candidate {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let (token, separator) = match chunk.find(char::is_whitespace) {
            Some(index) => match (chunk.get(..index), chunk.get(index..)) {
                (Some(token), Some(separator)) => (token, separator),
                _ => (chunk, ""),
            },
            None => (chunk, ""),
        };
        match scrub_token(token, home, temp) {
            Cow::Owned(owned) => {
                changed = true;
                out.push_str(&owned);
            }
            Cow::Borrowed(borrowed) => out.push_str(borrowed),
        }
        out.push_str(separator);
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

fn scrub_token<'t>(token: &'t str, home: Option<&str>, temp: Option<&str>) -> Cow<'t, str> {
    let mut current: Cow<'t, str> = Cow::Borrowed(token);
    if let Some(temp) = temp
        && let Some(truncated) = truncate_after(&current, temp)
    {
        current = Cow::Owned(truncated);
    }
    if let Some(truncated) = truncate_after(&current, "/tmp") {
        current = Cow::Owned(truncated);
    }
    if let Some(home) = home
        && current.contains(home)
    {
        current = Cow::Owned(current.replace(home, "~"));
    }
    if current.contains("/home/") {
        current = Cow::Owned(replace_home_segments(&current));
    }
    current
}

/// Rewrites the first path-component `<prefix>/...` occurrence to
/// `<prefix>/<redacted>`, dropping the rest of the token (the DROP rule).
fn truncate_after(token: &str, prefix: &str) -> Option<String> {
    for (index, _) in token.match_indices(prefix) {
        if !at_path_boundary(token, index) {
            continue;
        }
        let after = index + prefix.len();
        if token.get(after..).is_some_and(|rest| rest.starts_with('/')) {
            let head = token.get(..after)?;
            return Some(format!("{head}/{REDACTED}"));
        }
    }
    None
}

/// True when `index` starts a path component (token start, or preceded by
/// a non-path character like `=`, `(`, `,`, `:`).
fn at_path_boundary(token: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let Some(before) = token.get(..index) else {
        return false;
    };
    match before.chars().next_back() {
        Some(character) => {
            !character.is_ascii_alphanumeric() && !matches!(character, '_' | '.' | '-' | '~' | '/')
        }
        None => true,
    }
}

/// Rewrites every `/home/<user>` segment to `~`, keeping the path tail
/// (`/home/alice/x.png` -> `~/x.png`; a bare `/home/alice` -> `~`).
fn replace_home_segments(input: &str) -> String {
    const HOME: &str = "/home/";
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = rest.find(HOME) {
        let (Some(head), Some(after)) = (rest.get(..index), rest.get(index + HOME.len()..)) else {
            break;
        };
        let segment_len = after.find('/').unwrap_or(after.len());
        out.push_str(head);
        if segment_len == 0 {
            // An empty segment (`/home/`): keep it verbatim and advance.
            out.push('/');
            let Some(next) = rest.get(index + 1..) else {
                break;
            };
            rest = next;
            continue;
        }
        out.push('~');
        let Some(tail) = after.get(segment_len..) else {
            break;
        };
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const HOME: Option<&str> = Some("/home/alice");

    #[test]
    fn home_prefix_becomes_tilde() {
        assert_eq!(scrub_paths("/home/alice/x.png", HOME, None), "~/x.png");
        assert_eq!(scrub_paths("/home/alice", HOME, None), "~");
        // Any user's home is scrubbed, not just $HOME.
        assert_eq!(
            scrub_paths("/home/bob/docs/a.txt", HOME, None),
            "~/docs/a.txt"
        );
        assert_eq!(
            scrub_paths(
                "io error at (/home/alice/secret/file.png): denied",
                HOME,
                None
            ),
            "io error at (~/secret/file.png): denied"
        );
    }

    #[test]
    fn tmp_basenames_are_dropped() {
        assert_eq!(
            scrub_paths("/tmp/flowshot-session-42-7.json", HOME, None),
            "/tmp/<redacted>"
        );
        assert_eq!(scrub_paths("/tmp", HOME, None), "/tmp");
        assert_eq!(scrub_paths("/tmpdir/x", HOME, None), "/tmpdir/x");
        // /var/tmp must NOT match the /tmp rule mid-component...
        assert_eq!(
            scrub_paths("/var/tmp/x/flowshot.png", HOME, None),
            "/var/tmp/x/flowshot.png"
        );
        // ...but a custom TMPDIR prefix truncates.
        assert_eq!(
            scrub_paths("/var/tmp/x/flowshot.png", HOME, Some("/var/tmp/x")),
            "/var/tmp/x/<redacted>"
        );
        // An embedded /tmp path after a separator is still caught.
        assert_eq!(
            scrub_paths("spec=/tmp/flowshot-1.json", HOME, None),
            "spec=/tmp/<redacted>"
        );
    }

    #[test]
    fn scrubbing_leaves_clean_text_borrowed() {
        let text = "capture failed: no backend available";
        assert!(matches!(scrub_paths(text, HOME, None), Cow::Borrowed(_)));
    }

    #[test]
    fn home_under_a_custom_prefix_is_scrubbed() {
        // $HOME outside /home (e.g. a container root) still maps to ~.
        assert_eq!(
            scrub_paths("/root/.config/flowshot.toml", Some("/root"), None),
            "~/.config/flowshot.toml"
        );
    }
}
