// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing extended attributes, for tests only (feature `test-support`, off by default).

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

/// Writes the extended attribute `name` of an open file, or returns the OS error.
///
/// # Errors
///
/// A name with a NUL byte, or any error the operating system reports (for example
/// `ENOTSUP` on a file system without extended attributes).
pub fn set_xattr(file: &File, name: &str, value: &[u8]) -> io::Result<()> {
    let name = CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: the name is a NUL-terminated string, the value pointer and length come from
    // one live slice, and the descriptor belongs to an open file borrowed for the call.
    #[cfg(target_os = "macos")]
    let rc = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    let rc = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Like [`set_xattr`], for the file at `path`. The file is opened for reading only.
///
/// # Errors
///
/// The file cannot be opened, or as [`set_xattr`].
pub fn set_xattr_at(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    set_xattr(&File::open(path)?, name, value)
}
