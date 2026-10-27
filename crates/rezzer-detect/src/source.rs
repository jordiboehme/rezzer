// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opening a file or taking bytes, hashing and building the [`Source`], with the facts the
//! file system keeps about the file: modification and birth time, the download address and
//! tags.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rezzer_limits::Limits;
use rezzer_model::{Bytes, Source};
use sha2::{Digest, Sha256};

use crate::{Detector, OpenError};

/// Files up to this size are read into memory; larger ones are memory-mapped.
const IN_MEMORY_MAX: u64 = 64 * 1024 * 1024;

pub(crate) fn open(
    path: &Path,
    limits: &Limits,
    detector: &Detector<'_>,
) -> Result<Source, OpenError> {
    let shown = path.display().to_string();
    let file = open_file(path).map_err(|e| {
        if path.is_dir() {
            OpenError::NotAFile(shown.clone())
        } else {
            io_error(&shown, &e)
        }
    })?;
    // Type and size come from the open handle only, so a path swapped between two calls
    // cannot slip a device, a FIFO or a larger file past the checks.
    let handle_meta = file.metadata().map_err(|e| io_error(&shown, &e))?;
    if !handle_meta.is_file() {
        return Err(OpenError::NotAFile(shown));
    }
    let size = handle_meta.len();
    check_size(size, limits)?;
    let bytes = if size <= IN_MEMORY_MAX {
        // Small files are read into memory: no mapping cost, and no fault if another
        // process truncates the file while it is being read. At most the stat size is read;
        // one more byte means the file grew while reading.
        let mut buf = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        (&file)
            .take(size + 1)
            .read_to_end(&mut buf)
            .map_err(|e| io_error(&shown, &e))?;
        if buf.len() as u64 > size {
            return Err(OpenError::Io(format!(
                "{shown}: file changed while reading"
            )));
        }
        Bytes::Owned(Arc::new(buf))
    } else {
        // SAFETY: the map is read-only, but the bytes behind it belong to the file system.
        // Another process truncating the file while it is mapped can fault this process
        // (SIGBUS), and a concurrent write changes the bytes under the `&[u8]`, so the hash,
        // the detection and the later read can disagree. That is the documented cost of
        // memmap and is accepted for large files the user asked us to read.
        let map = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| io_error(&shown, &e))?;
        check_size(map.len() as u64, limits)?;
        Bytes::Mapped(Arc::new(map))
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| shown.clone());
    let facts = FsFacts {
        modified: handle_meta.modified().ok().and_then(rfc3339),
        created: handle_meta.created().ok().and_then(rfc3339),
        downloaded_from: downloaded_from(&file, path),
        file_tags: file_tags(&file),
    };
    Ok(build(Some(path), name, None, bytes, facts, detector))
}

/// What the file system knows about a file beside its bytes. Every field is empty when the
/// system has no value or reading it failed: these facts never fail an open.
#[derive(Default)]
struct FsFacts {
    modified: Option<String>,
    created: Option<String>,
    downloaded_from: Option<String>,
    file_tags: Vec<String>,
}

/// macOS: the download addresses, a binary plist array (the file, then the page).
#[cfg(unix)]
const WHERE_FROMS: &str = "com.apple.metadata:kMDItemWhereFroms";
/// macOS: the Finder tags, a binary plist array of `name\n<colour>` entries.
#[cfg(unix)]
const FINDER_TAGS: &str = "com.apple.metadata:_kMDItemUserTags";
/// freedesktop: the download address, plain text.
#[cfg(unix)]
const XDG_ORIGIN_URL: &str = "user.xdg.origin.url";
/// freedesktop: the tags, comma-separated text.
#[cfg(unix)]
const XDG_TAGS: &str = "user.xdg.tags";

/// One extended attribute of the open file. Missing, too large, unsupported or any error
/// gives None.
#[cfg(unix)]
fn xattr(file: &File, name: &str) -> Option<Vec<u8>> {
    fileattrs::read_xattr(file, name, fileattrs::MAX_ATTR_BYTES)
        .ok()
        .flatten()
}

/// The address the file was downloaded from: the first `kMDItemWhereFroms` entry, else
/// `user.xdg.origin.url`, else (Windows) the `HostUrl` of the `Zone.Identifier` stream.
/// Only a safe address is kept, see [`safe_url`].
#[cfg(unix)]
fn downloaded_from(file: &File, _path: &Path) -> Option<String> {
    let first = xattr(file, WHERE_FROMS)
        .and_then(|bytes| fileattrs::string_array(&bytes).ok())
        .and_then(|entries| entries.into_iter().next());
    if let Some(first) = first {
        // The first entry is the file itself; a later one (the page) is never taken instead,
        // and an unsafe first entry is not replaced by the other attribute.
        return safe_url(&first);
    }
    // A missing, unreadable or empty WhereFroms value: the freedesktop attribute counts.
    let bytes = xattr(file, XDG_ORIGIN_URL)?;
    let text = std::str::from_utf8(&bytes).ok()?;
    safe_url(text.trim_matches(|c: char| c == '\0' || c.is_whitespace()))
}

#[cfg(windows)]
fn downloaded_from(_file: &File, path: &Path) -> Option<String> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    let mut bytes = Vec::new();
    File::open(stream)
        .ok()?
        .take(fileattrs::MAX_ATTR_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    zone_download(&bytes)
}

#[cfg(not(any(unix, windows)))]
fn downloaded_from(_file: &File, _path: &Path) -> Option<String> {
    None
}

/// The download address in the bytes of a `Zone.Identifier` stream. A stream over
/// [`fileattrs::MAX_ATTR_BYTES`] gives None.
#[cfg_attr(not(windows), allow(dead_code))]
fn zone_download(bytes: &[u8]) -> Option<String> {
    if bytes.len() > fileattrs::MAX_ATTR_BYTES {
        return None;
    }
    let text = fileattrs::zone_text(bytes)?;
    safe_url(&fileattrs::zone_host_url(&text)?)
}

/// The Finder tags, else (when they are missing, broken or empty) the `user.xdg.tags`, as
/// written: no empty names, no repeats, at most [`fileattrs::MAX_TAGS`].
#[cfg(unix)]
fn file_tags(file: &File) -> Vec<String> {
    let finder = xattr(file, FINDER_TAGS)
        .map(|bytes| fileattrs::finder_tags(&bytes))
        .unwrap_or_default();
    if !finder.is_empty() {
        return finder;
    }
    xattr(file, XDG_TAGS)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|text| fileattrs::xdg_tags(&text))
        .unwrap_or_default()
}

#[cfg(not(unix))]
fn file_tags(_file: &File) -> Vec<String> {
    Vec::new()
}

/// Addresses longer than this are not kept.
const MAX_URL_BYTES: usize = 2048;

/// `raw` when it is an `http`, `https` or `ftp` address with a host, with any user and
/// password removed; else None. An address over [`MAX_URL_BYTES`], or with white space, a
/// backslash (which browsers read as `/`, so the host would be another), a control character
/// or an invisible format character (Unicode Cf, for example U+202E) is unsafe too.
fn safe_url(raw: &str) -> Option<String> {
    if raw.len() > MAX_URL_BYTES
        || raw
            .chars()
            .any(|c| c == '\\' || c.is_control() || c.is_whitespace() || is_format_char(c))
    {
        return None;
    }
    let (scheme, rest) = raw.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https" | "ftp") {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if host.is_empty() || host.starts_with(':') {
        return None;
    }
    Some(format!("{scheme}://{host}{tail}"))
}

/// Unicode general category Cf (format characters), as of Unicode 16.0.
///
/// To refresh for a newer Unicode version, list the code points whose second field is `Cf`
/// in that version's `UnicodeData.txt` (`https://www.unicode.org/Public/<version>/ucd/`),
/// join neighbours into ranges and replace the table. Unicode 14 added U+0890 and U+0891,
/// Unicode 15 extended U+13430..U+13438 to U+1343F; nothing changed in 16.0.
fn is_format_char(c: char) -> bool {
    matches!(
        u32::from(c),
        0xAD | 0x600..=0x605
            | 0x61C
            | 0x6DD
            | 0x70F
            | 0x890..=0x891
            | 0x8E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// Opens for reading. On Unix with `O_NONBLOCK`, so opening a FIFO returns at once instead of
/// waiting for a writer; regular files ignore the flag.
fn open_file(path: &Path) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    opts.open(path)
}

pub(crate) fn from_bytes(
    name: &str,
    mime: Option<&str>,
    bytes: Vec<u8>,
    limits: &Limits,
    detector: &Detector<'_>,
) -> Result<Source, OpenError> {
    check_size(bytes.len() as u64, limits)?;
    Ok(build(
        None,
        name.to_string(),
        mime,
        Bytes::Owned(Arc::new(bytes)),
        FsFacts::default(),
        detector,
    ))
}

fn build(
    path: Option<&Path>,
    name: String,
    mime: Option<&str>,
    bytes: Bytes,
    facts: FsFacts,
    detector: &Detector<'_>,
) -> Source {
    let slice = bytes.as_slice();
    let sha256 = rezzer_model::text::hex(&Sha256::digest(slice));
    let size = slice.len() as u64;
    let (format, detection) = detector(&name, mime, slice);
    Source {
        path: path.map(Path::to_path_buf),
        name,
        bytes,
        sha256,
        size,
        format,
        detection,
        modified: facts.modified,
        created: facts.created,
        downloaded_from: facts.downloaded_from,
        file_tags: facts.file_tags,
    }
}

fn check_size(size: u64, limits: &Limits) -> Result<(), OpenError> {
    if size > limits.max_file_bytes {
        return Err(OpenError::TooLarge {
            size,
            max: limits.max_file_bytes,
        });
    }
    Ok(())
}

fn io_error(shown: &str, e: &std::io::Error) -> OpenError {
    match e.kind() {
        ErrorKind::NotFound => OpenError::NotFound(shown.to_string()),
        _ => OpenError::Io(format!("{shown}: {e}")),
    }
}

/// Formats a time as RFC 3339 in UTC with whole seconds, for example
/// `2026-10-19T08:30:00Z`. Times before 1970 give None.
pub(crate) fn rfc3339(t: SystemTime) -> Option<String> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    ))
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> String {
        rfc3339(UNIX_EPOCH + Duration::from_secs(secs)).unwrap()
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(951_868_799), "2000-02-29T23:59:59Z");
        assert_eq!(at(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(at(1_709_208_000), "2024-02-29T12:00:00Z");
        assert_eq!(at(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    #[test]
    fn large_file_is_mapped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let f = File::create(&path).unwrap();
        f.set_len(IN_MEMORY_MAX + 1).unwrap();
        let src = open(&path, &Limits::default(), &crate::detect_with_hints).unwrap();
        assert!(matches!(src.bytes, Bytes::Mapped(_)), "{:?}", src.bytes);
        assert_eq!(src.size, IN_MEMORY_MAX + 1);
    }

    #[test]
    fn unsafe_where_froms_are_dropped() {
        assert_eq!(safe_url("javascript:alert(1)"), None);
        assert_eq!(
            safe_url("https://user:pw@example.org/a.pdf").as_deref(),
            Some("https://example.org/a.pdf")
        );
        assert_eq!(
            safe_url("HTTPS://example.org/a@b.pdf").as_deref(),
            Some("https://example.org/a@b.pdf")
        );
        assert_eq!(
            safe_url("ftp://me@example.org:21").as_deref(),
            Some("ftp://example.org:21")
        );
        assert_eq!(
            safe_url("http://example.org?q=a@b").as_deref(),
            Some("http://example.org?q=a@b")
        );
        for bad in [
            "file:///etc/passwd",
            "data:text/html,x",
            "https://",
            "https://user@/a",
            "https://example.org/a b",
            "https://example.org/\nx",
            "example.org/a.pdf",
            "",
            "https://evil.example\\@good.example/x",
            "https://example.org/a\\b",
            "https://example.org/\u{202E}fdp.exe",
            "https://exa\u{200B}mple.org/",
            "https://example.org/\u{FEFF}",
            "https://example.org/\u{7F}",
        ] {
            assert_eq!(safe_url(bad), None, "{bad:?}");
        }
        let long = format!("https://example.org/{}", "a".repeat(64 * 1024));
        assert_eq!(safe_url(&long), None);
        let edge = format!("https://example.org/{}", "a".repeat(MAX_URL_BYTES - 20));
        assert_eq!(edge.len(), MAX_URL_BYTES);
        assert_eq!(safe_url(&edge).as_deref(), Some(edge.as_str()));
        assert_eq!(
            safe_url("https://example.org/Gr%C3%B6%C3%9Fe/日本.pdf").as_deref(),
            Some("https://example.org/Gr%C3%B6%C3%9Fe/日本.pdf")
        );
    }

    #[test]
    fn zone_identifier_gives_the_host_url() {
        let utf8 = b"[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.org/\r\nHostUrl=https://u:p@example.org/a.pdf\r\n";
        assert_eq!(
            zone_download(utf8).as_deref(),
            Some("https://example.org/a.pdf")
        );
        assert_eq!(
            zone_download(b"[ZoneTransfer]\r\nHostUrl=about:internet\r\n"),
            None
        );
        let mut big = utf8.to_vec();
        big.resize(fileattrs::MAX_ATTR_BYTES + 1, b' ');
        assert_eq!(zone_download(&big), None);
    }

    #[test]
    fn rfc3339_before_epoch_is_none() {
        assert!(rfc3339(UNIX_EPOCH - Duration::from_secs(1)).is_none());
    }
}
