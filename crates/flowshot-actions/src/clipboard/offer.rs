//! Clipboard offer construction.
//!
//! Pure builders for the MIME-typed payloads placed on the clipboard:
//! image offers (`image/png` always, `image/jpeg` appended when
//! `[save].clipboard_format = 'jpeg'`), path offers (`text/plain` +
//! `text/uri-list`), and the combined image+path offer used by the
//! `copy-path` action's "uri-list appended" rule.

use std::path::Path;

use flowshot_core::config::ClipboardFormat;

/// `image/png` MIME type.
pub const MIME_PNG: &str = "image/png";
/// `image/jpeg` MIME type.
pub const MIME_JPEG: &str = "image/jpeg";
/// `text/uri-list` MIME type (RFC 2483).
pub const MIME_URI_LIST: &str = "text/uri-list";
/// `text/plain` MIME type.
pub const MIME_TEXT_PLAIN: &str = "text/plain";

/// The MIME type a [`ClipboardFormat`] is offered under.
#[must_use]
pub const fn image_mime(format: ClipboardFormat) -> &'static str {
    match format {
        ClipboardFormat::Png => MIME_PNG,
        ClipboardFormat::Jpeg => MIME_JPEG,
    }
}

/// One MIME-typed payload inside a clipboard offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferEntry {
    /// MIME type this entry is offered under.
    pub mime: String,
    /// Raw bytes served for [`Self::mime`].
    pub data: Box<[u8]>,
}

impl OfferEntry {
    /// Build an entry from a MIME type and owned bytes.
    #[must_use]
    pub fn new(mime: impl Into<String>, data: impl Into<Box<[u8]>>) -> Self {
        Self {
            mime: mime.into(),
            data: data.into(),
        }
    }
}

/// A complete clipboard offer: one or more MIME-typed payloads.
///
/// Entry order matters: the first `text/*` entry seeds the automatic
/// plain-text variants (`text/plain;charset=utf-8`, `STRING`,
/// `UTF8_STRING`, `TEXT`) that `wl-clipboard-rs` adds on top.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardOffer {
    /// The offered payloads, in offer order.
    pub entries: Vec<OfferEntry>,
}

/// Image offer: `image/png` always; `image/jpeg` appended when the
/// `[save].clipboard_format` config selects JPEG.
#[must_use]
pub fn image_offer(png: Vec<u8>, jpeg: Option<Vec<u8>>) -> ClipboardOffer {
    let mut entries = vec![OfferEntry::new(MIME_PNG, png)];
    if let Some(jpeg) = jpeg {
        entries.push(OfferEntry::new(MIME_JPEG, jpeg));
    }
    ClipboardOffer { entries }
}

/// Path offer: `text/plain` (bare path) first so the automatic text
/// variants carry the readable path, `text/uri-list` (CRLF-terminated
/// `file:` URI per RFC 2483) appended.
///
/// Non-UTF-8 or relative paths get the `text/plain` entry only (warned).
#[must_use]
pub fn path_offer(path: &Path) -> ClipboardOffer {
    let mut entries = vec![OfferEntry::new(
        MIME_TEXT_PLAIN,
        path.as_os_str().as_encoded_bytes(),
    )];
    if let Some(uri) = file_uri(path) {
        entries.push(OfferEntry::new(
            MIME_URI_LIST,
            format!("{uri}\r\n").into_bytes(),
        ));
    } else {
        tracing::warn!(
            path = %path.display(),
            "path is relative or not valid UTF-8; offering text/plain only"
        );
    }
    ClipboardOffer { entries }
}

/// Combined offer for `copy-path` after an image copy: the image entries
/// with the path entries appended ("`text/uri-list` appended when
/// `copy-path` in effective set + saved").
#[must_use]
pub fn image_and_path_offer(png: Vec<u8>, jpeg: Option<Vec<u8>>, path: &Path) -> ClipboardOffer {
    let mut offer = image_offer(png, jpeg);
    offer.entries.extend(path_offer(path).entries);
    offer
}

/// The `file:` URI for an absolute UTF-8 path, percent-encoded per
/// RFC 3986 (unreserved set `A-Za-z0-9-._~` plus `/` pass through).
///
/// Returns `None` for relative or non-UTF-8 paths.
#[must_use]
pub fn file_uri(path: &Path) -> Option<String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_str()?;
    let mut uri = String::with_capacity(text.len() + "file://".len());
    uri.push_str("file://");
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(*byte));
            }
            other => {
                uri.push('%');
                uri.push(char::from(HEX[usize::from(other >> 4)]));
                uri.push(char::from(HEX[usize::from(other & 0x0f)]));
            }
        }
    }
    Some(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_mime_maps_formats() {
        assert_eq!(image_mime(ClipboardFormat::Png), "image/png");
        assert_eq!(image_mime(ClipboardFormat::Jpeg), "image/jpeg");
    }

    #[test]
    fn image_offer_is_png_only_by_default() {
        let offer = image_offer(vec![1, 2], None);
        let mimes: Vec<&str> = offer.entries.iter().map(|e| e.mime.as_str()).collect();
        assert_eq!(mimes, ["image/png"]);
        assert_eq!(offer.entries[0].data.as_ref(), &[1, 2]);
    }

    #[test]
    fn image_offer_appends_jpeg_after_png() {
        let offer = image_offer(vec![1], Some(vec![2]));
        let mimes: Vec<&str> = offer.entries.iter().map(|e| e.mime.as_str()).collect();
        assert_eq!(mimes, ["image/png", "image/jpeg"]);
    }

    #[test]
    fn path_offer_puts_plain_before_uri_list() {
        let offer = path_offer(Path::new("/tmp/shot 1.png"));
        let mimes: Vec<&str> = offer.entries.iter().map(|e| e.mime.as_str()).collect();
        assert_eq!(mimes, ["text/plain", "text/uri-list"]);
        assert_eq!(offer.entries[0].data.as_ref(), b"/tmp/shot 1.png");
        assert_eq!(
            offer.entries[1].data.as_ref(),
            b"file:///tmp/shot%201.png\r\n"
        );
    }

    #[test]
    fn path_offer_relative_path_is_plain_only() {
        let offer = path_offer(Path::new("shots/x.png"));
        assert_eq!(offer.entries.len(), 1);
        assert_eq!(offer.entries[0].mime, "text/plain");
    }

    #[test]
    fn combined_offer_appends_path_after_images() {
        let offer = image_and_path_offer(vec![1], Some(vec![2]), Path::new("/tmp/s.png"));
        let mimes: Vec<&str> = offer.entries.iter().map(|e| e.mime.as_str()).collect();
        assert_eq!(
            mimes,
            ["image/png", "image/jpeg", "text/plain", "text/uri-list"]
        );
    }

    #[test]
    fn file_uri_percent_encodes_reserved_and_unicode() {
        assert_eq!(
            file_uri(Path::new("/a b/#?%.png")).as_deref(),
            Some("file:///a%20b/%23%3F%25.png")
        );
        assert_eq!(
            file_uri(Path::new("/tmp/ünïcødé.png")).as_deref(),
            Some("file:///tmp/%C3%BCn%C3%AFc%C3%B8d%C3%A9.png")
        );
        // Unreserved set passes through.
        assert_eq!(
            file_uri(Path::new("/a-b_c.d~e/f")).as_deref(),
            Some("file:///a-b_c.d~e/f")
        );
    }

    #[test]
    fn file_uri_rejects_relative_paths() {
        assert_eq!(file_uri(Path::new("relative/x.png")), None);
    }

    #[cfg(unix)]
    #[test]
    fn file_uri_rejects_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt as _;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe.png"));
        assert_eq!(file_uri(path), None);
        // The plain-text entry still carries the raw bytes.
        let offer = path_offer(path);
        assert_eq!(offer.entries.len(), 1);
        assert_eq!(offer.entries[0].data.as_ref(), b"/tmp/\xff\xfe.png");
    }
}
