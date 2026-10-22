// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bounded reads of extended attributes through an open file handle.

use std::ffi::CString;
use std::fs::File;
use std::io;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::fd::AsRawFd;

/// How often a read is tried again when the attribute grew between the size query and the
/// read.
const ATTEMPTS: usize = 3;

/// Reads the extended attribute `name` of an open file.
///
/// The size is asked first, so nothing larger than `max` bytes is ever read. Returns
/// `Ok(None)` when the attribute is missing, larger than `max`, or the file system has no
/// extended attributes. On systems other than macOS and Linux it always returns
/// `Ok(None)`.
///
/// # Errors
///
/// A name with a NUL byte, or any other error the operating system reports.
pub fn read_xattr(file: &File, name: &str, max: usize) -> io::Result<Option<Vec<u8>>> {
    let name = CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    for _ in 0..ATTEMPTS {
        let size = match get(file, &name, &mut [])? {
            Some(size) => size,
            None => return Ok(None),
        };
        if size > max {
            return Ok(None);
        }
        let mut buf = vec![0u8; size];
        match get(file, &name, &mut buf) {
            Ok(Some(read)) if read <= size => {
                buf.truncate(read);
                return Ok(Some(buf));
            }
            // An empty buffer makes the read another size query: the attribute grew.
            Ok(Some(_)) if size == 0 => continue,
            Ok(_) => return Ok(None),
            Err(e) if e.raw_os_error() == Some(libc::ERANGE) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// One `fgetxattr` call. An empty buffer asks for the size. `Ok(None)` when the attribute
/// is missing or not supported.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn get(file: &File, name: &CString, buf: &mut [u8]) -> io::Result<Option<usize>> {
    let (ptr, len) = if buf.is_empty() {
        (std::ptr::null_mut(), 0)
    } else {
        (buf.as_mut_ptr().cast::<libc::c_void>(), buf.len())
    };
    // SAFETY: the name is NUL-terminated, the buffer pointer and length come from one live
    // mutable slice (or are null and 0 for the size query), and the descriptor belongs to
    // an open file borrowed for the whole call.
    #[cfg(target_os = "macos")]
    let rc = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), ptr, len, 0, 0) };
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    let rc = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), ptr, len) };

    if let Ok(size) = usize::try_from(rc) {
        return Ok(Some(size));
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        #[cfg(target_os = "macos")]
        Some(libc::ENOATTR) => Ok(None),
        #[cfg(target_os = "linux")]
        Some(libc::ENODATA) => Ok(None),
        Some(libc::ENOTSUP) => Ok(None),
        _ => Err(error),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get(_file: &File, _name: &CString, _buf: &mut [u8]) -> io::Result<Option<usize>> {
    Ok(None)
}
