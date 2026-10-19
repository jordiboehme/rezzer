// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opening a file or taking bytes, hashing and building the [`Source`].

use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rezzer_limits::Limits;
use rezzer_model::{Bytes, Source};
use sha2::{Digest, Sha256};

use crate::{OpenError, detect};

pub(crate) fn open(path: &Path, limits: &Limits) -> Result<Source, OpenError> {
    let shown = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| io_error(&shown, &e))?;
    if !meta.is_file() {
        return Err(OpenError::NotAFile(shown));
    }
    check_size(meta.len(), limits)?;
    let file = File::open(path).map_err(|e| io_error(&shown, &e))?;
    // Size again from the open handle: the file may have changed since the stat.
    let size = file.metadata().map_err(|e| io_error(&shown, &e))?.len();
    check_size(size, limits)?;
    let bytes = if size == 0 {
        // Mapping an empty file fails on some platforms.
        Bytes::Owned(Arc::new(Vec::new()))
    } else {
        // SAFETY: the map is read-only. Another process truncating the file while it is
        // mapped can still fault the process; that is the documented cost of memmap and is
        // accepted for files the user asked us to read.
        let map = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| io_error(&shown, &e))?;
        check_size(map.len() as u64, limits)?;
        Bytes::Mapped(Arc::new(map))
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| shown.clone());
    let modified = meta.modified().ok().and_then(rfc3339);
    Ok(build(Some(path), name, bytes, modified))
}

pub(crate) fn from_bytes(name: &str, bytes: Vec<u8>, limits: &Limits) -> Result<Source, OpenError> {
    check_size(bytes.len() as u64, limits)?;
    Ok(build(
        None,
        name.to_string(),
        Bytes::Owned(Arc::new(bytes)),
        None,
    ))
}

fn build(path: Option<&Path>, name: String, bytes: Bytes, modified: Option<String>) -> Source {
    let slice = bytes.as_slice();
    let sha256 = hex::encode(Sha256::digest(slice));
    let size = slice.len() as u64;
    let (format, detection) = detect(&name, slice);
    Source {
        path: path.map(Path::to_path_buf),
        name,
        bytes,
        sha256,
        size,
        format,
        detection,
        modified,
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
    fn rfc3339_before_epoch_is_none() {
        assert!(rfc3339(UNIX_EPOCH - Duration::from_secs(1)).is_none());
    }
}
