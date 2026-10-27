// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::fs::File;
use std::io::Read;
use std::path::Path;

const GREETING: &str = "Greetings, program. I fight for the users.\nEnd of Line";

/// A hidden nod, on purpose not in the help text or the docs: when `rezzer read` is given
/// the very binary that runs (or a byte-identical copy of it) and stderr is a terminal, two
/// lines go to stderr after the read. Nothing else changes: stdout, the exit code, the cache
/// and the MCP server are untouched, and scripts never see it because they have no terminal.
pub fn greeting(path: &Path, exe: &Path, stderr_is_terminal: bool) -> Option<&'static str> {
    (stderr_is_terminal && is_self(path, exe)).then_some(GREETING)
}

/// Called by the read command once the read is done.
pub fn after_read(path: &Path) {
    use std::io::IsTerminal;
    if let Ok(exe) = std::env::current_exe()
        && let Some(text) = greeting(path, &exe, std::io::stderr().is_terminal())
    {
        eprintln!("{text}");
    }
}

/// Whether `path` is the file at `exe` or a copy with the same length and the same bytes.
/// Any error means no.
fn is_self(path: &Path, exe: &Path) -> bool {
    same_file(path, exe).unwrap_or(false)
}

fn same_file(path: &Path, exe: &Path) -> std::io::Result<bool> {
    // Check the kind before opening: opening a named pipe would wait for a writer.
    if !std::fs::metadata(path)?.is_file() || !std::fs::metadata(exe)?.is_file() {
        return Ok(false);
    }
    let (a, b) = (File::open(path)?, File::open(exe)?);
    let (ma, mb) = (a.metadata()?, b.metadata()?);
    if !ma.is_file() || !mb.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if ma.dev() == mb.dev() && ma.ino() == mb.ino() {
            return Ok(true);
        }
    }
    #[cfg(not(unix))]
    if path.canonicalize()? == exe.canonicalize()? {
        return Ok(true);
    }
    if ma.len() != mb.len() {
        return Ok(false);
    }
    // Same length: compare in chunks, so no second copy of the binary is held in memory.
    let (mut a, mut b) = (std::io::BufReader::new(a), std::io::BufReader::new(b));
    let (mut x, mut y) = ([0u8; 64 * 1024], [0u8; 64 * 1024]);
    loop {
        let n = a.read(&mut x)?;
        if n == 0 {
            return Ok(b.read(&mut y)? == 0);
        }
        b.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const LINE: &str = "Greetings, program. I fight for the users.\nEnd of Line";

    fn exe_file(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("rezzer-fake");
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn the_same_file_is_self() {
        let d = tempfile::tempdir().unwrap();
        let exe = exe_file(d.path());
        assert!(is_self(&exe, &exe));
        assert_eq!(greeting(&exe, &exe, true), Some(LINE));
    }

    #[test]
    fn a_byte_identical_copy_is_self() {
        let d = tempfile::tempdir().unwrap();
        let exe = exe_file(d.path());
        let other = tempfile::tempdir().unwrap();
        let copy = other.path().join("renamed");
        fs::copy(&exe, &copy).unwrap();
        assert!(is_self(&copy, &exe));
    }

    #[test]
    fn one_changed_byte_is_not_self() {
        let d = tempfile::tempdir().unwrap();
        let exe = exe_file(d.path());
        let mut bytes = fs::read(&exe).unwrap();
        bytes[150_000] ^= 1;
        let copy = d.path().join("changed");
        fs::write(&copy, bytes).unwrap();
        assert!(!is_self(&copy, &exe));
    }

    #[test]
    fn another_length_is_not_self() {
        let d = tempfile::tempdir().unwrap();
        let exe = exe_file(d.path());
        let mut bytes = fs::read(&exe).unwrap();
        bytes.push(0);
        let copy = d.path().join("longer");
        fs::write(&copy, bytes).unwrap();
        assert!(!is_self(&copy, &exe));
    }

    #[test]
    fn a_missing_file_is_not_self() {
        let d = tempfile::tempdir().unwrap();
        let exe = exe_file(d.path());
        assert!(!is_self(&d.path().join("nope"), &exe));
        assert!(!is_self(&exe, &d.path().join("nope")));
    }

    #[test]
    fn no_terminal_means_no_greeting() {
        let d = tempfile::tempdir().unwrap();
        let exe = exe_file(d.path());
        assert_eq!(greeting(&exe, &exe, false), None);
    }
}
