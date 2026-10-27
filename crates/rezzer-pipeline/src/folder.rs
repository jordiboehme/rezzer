// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Folders opened once, for every write, read and removal under the output folder.
//!
//! A [`Folder`] on Unix holds an open handle of its folder (`O_DIRECTORY`) and does each step
//! relative to it (`openat`, `mkdirat`, `unlinkat`, `fstatat`), so a folder that someone swaps
//! for a symlink or for another folder after it was opened takes no write, read or removal
//! elsewhere. Every method takes one path component, never a path: an empty name, `.`, `..`
//! or a name with `/` or a backslash (off Unix also a `:`) is refused with
//! [`ErrorKind::InvalidInput`]. The last component is opened with `O_NOFOLLOW`, so that one
//! flag guards the whole path. Only the output folder itself is opened by its path
//! ([`Folder::open`], [`Folder::create`]): it may be a symlink the user chose, and it is
//! resolved once there. It must be readable as well as writable, since it is opened for
//! reading (`O_RDONLY | O_DIRECTORY`).
//!
//! Known Windows limit (0.1.0): on other systems a `Folder` keeps its path, and each call
//! checks the entry with `symlink_metadata` and then opens it by name, as before. A folder
//! under the output folder that someone swaps for a link between that check and the call can
//! still redirect a write or a removal there. Handle-relative calls on Windows need
//! `NtCreateFile` with a root handle, which this code does not use yet.

use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// What an entry of a folder is, never following a symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EntryKind {
    /// A regular file.
    File,
    /// A directory (not a symlink to one).
    Dir,
    /// Anything else: a symlink, a FIFO, a socket, a device.
    Other,
}

/// A folder opened once (see the module docs).
#[derive(Debug)]
pub(crate) struct Folder {
    /// The path for messages; on Unix it is never opened again.
    path: PathBuf,
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
}

/// Whether `name` is one plain path component: not empty, `.` or `..`, and without `/` or a
/// backslash. With `drive`, a `:` is refused too, so a Windows name such as `C:x.md` cannot
/// name another drive's folder.
fn plain(name: &OsStr, drive: bool) -> bool {
    let bytes = name.as_encoded_bytes();
    !(bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes
            .iter()
            .any(|b| matches!(b, b'/' | b'\\') || (drive && *b == b':')))
}

/// `name` when it is one path component, else an [`ErrorKind::InvalidInput`] error. Off Unix
/// the name must also parse as exactly one normal component, so joining it to the folder's
/// path never drops or leaves that path.
fn component(name: &OsStr) -> io::Result<&OsStr> {
    if !(plain(name, cfg!(not(unix))) && one_normal(name)) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} is not a plain file name", name.display()),
        ));
    }
    Ok(name)
}

/// On Unix [`plain`] is the whole rule: the kernel takes the name as it is.
#[cfg(unix)]
fn one_normal(_: &OsStr) -> bool {
    true
}

/// Whether `name` parses as exactly one normal component (no drive, root or share prefix).
#[cfg(not(unix))]
fn one_normal(name: &OsStr) -> bool {
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none()
}

/// The error for a subfolder that is a symlink, a file or anything but a plain directory.
fn not_plain(name: &str) -> io::Error {
    io::Error::new(
        ErrorKind::NotADirectory,
        format!("{name} exists and is not a plain directory"),
    )
}

/// The error for a file that was not there at the check but is now.
fn appeared() -> io::Error {
    io::Error::other("it appeared while Rezzer was writing and is not replaced")
}

impl Folder {
    /// The folder's path, for messages only.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Makes the output folder `path` and its parents when missing, then opens it as
    /// [`Folder::open`] does. Called only after the check passed, so a failed check makes
    /// nothing.
    pub(crate) fn create(path: &Path) -> io::Result<Folder> {
        std::fs::create_dir_all(path)?;
        Folder::open(path)?.ok_or_else(|| io::Error::from(ErrorKind::NotFound))
    }
}

#[cfg(unix)]
mod unix {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags};
    use rustix::io::Errno;

    use super::{EntryKind, Folder, appeared, component, not_plain};

    /// The kind of a file type, a symlink as [`EntryKind::Other`].
    fn kind_of(file_type: FileType) -> EntryKind {
        match file_type {
            FileType::RegularFile => EntryKind::File,
            FileType::Directory => EntryKind::Dir,
            _ => EntryKind::Other,
        }
    }

    impl Folder {
        /// Opens the output folder `path` (one `open` with `O_DIRECTORY`, following a
        /// symlink the user chose); `None` when it is missing.
        pub(crate) fn open(path: &Path) -> io::Result<Option<Folder>> {
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
            match rustix::fs::openat(CWD, path, flags, Mode::empty()) {
                Ok(fd) => Ok(Some(Folder {
                    path: path.to_path_buf(),
                    fd,
                })),
                Err(Errno::NOENT) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }

        /// What `name` is (`fstatat` without following a symlink); `None` when it is missing.
        pub(crate) fn kind(&self, name: &OsStr) -> io::Result<Option<EntryKind>> {
            let name = component(name)?;
            match rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => Ok(Some(kind_of(FileType::from_raw_mode(stat.st_mode)))),
                Err(Errno::NOENT) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }

        /// Opens the subfolder `name`, made first when `create` (an existing one is fine). A
        /// symlink, a file or anything but a plain directory there is refused.
        pub(crate) fn sub(&self, name: &str, create: bool) -> io::Result<Folder> {
            let os = component(OsStr::new(name))?;
            if create {
                match rustix::fs::mkdirat(&self.fd, os, Mode::from_raw_mode(0o777)) {
                    Ok(()) | Err(Errno::EXIST) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            match rustix::fs::openat(&self.fd, os, flags, Mode::empty()) {
                Ok(fd) => Ok(Folder {
                    path: self.path.join(name),
                    fd,
                }),
                // A symlink gives `ELOOP`, a file `ENOTDIR` (Linux may give either for a link).
                Err(Errno::LOOP | Errno::NOTDIR) => Err(not_plain(name)),
                Err(e) => Err(e.into()),
            }
        }

        /// Opens the regular file `name` for reading, never through a symlink; `None` when it
        /// is a symlink or anything but a regular file. `O_NONBLOCK` keeps a FIFO from
        /// waiting for a writer; it does nothing for a regular file.
        pub(crate) fn open_file(&self, name: &OsStr) -> io::Result<Option<File>> {
            let name = component(name)?;
            let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
            let fd = match rustix::fs::openat(&self.fd, name, flags, Mode::empty()) {
                Ok(fd) => fd,
                Err(Errno::LOOP) => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            let stat = rustix::fs::fstat(&fd)?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                return Ok(None);
            }
            Ok(Some(File::from(fd)))
        }

        /// Creates the file `name` exclusively, never through a symlink, and writes `bytes`.
        /// An entry that is there fails it ("it appeared while Rezzer was writing").
        pub(crate) fn create_new(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
            use std::io::Write;
            let os = component(OsStr::new(name))?;
            let flags =
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = match rustix::fs::openat(&self.fd, os, flags, Mode::from_raw_mode(0o666)) {
                Ok(fd) => fd,
                Err(Errno::EXIST) => return Err(appeared()),
                Err(e) => return Err(e.into()),
            };
            File::from(fd).write_all(bytes)
        }

        /// Removes the file `name`; a symlink itself is removed, never its target.
        pub(crate) fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            let name = component(name)?;
            Ok(rustix::fs::unlinkat(&self.fd, name, AtFlags::empty())?)
        }

        /// Removes the empty folder `name`; a symlink or a folder with entries stays.
        pub(crate) fn remove_dir(&self, name: &str) -> io::Result<()> {
            let name = component(OsStr::new(name))?;
            Ok(rustix::fs::unlinkat(&self.fd, name, AtFlags::REMOVEDIR)?)
        }

        /// The entries of the folder with their kinds, without `.` and `..`.
        pub(crate) fn entries(&self) -> io::Result<Vec<(OsString, EntryKind)>> {
            let mut entries = Vec::new();
            for entry in Dir::read_from(&self.fd)? {
                let entry = entry?;
                let name = OsStr::from_bytes(entry.file_name().to_bytes());
                if name == "." || name == ".." {
                    continue;
                }
                let kind = match entry.file_type() {
                    // Some file systems do not say: ask without following. A listed name is
                    // one component already, also one with a backslash (a legal Unix name).
                    FileType::Unknown => {
                        match rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                            Ok(stat) => kind_of(FileType::from_raw_mode(stat.st_mode)),
                            // Gone since the listing.
                            Err(Errno::NOENT) => continue,
                            Err(e) => return Err(e.into()),
                        }
                    }
                    file_type => kind_of(file_type),
                };
                entries.push((name.to_os_string(), kind));
            }
            Ok(entries)
        }
    }
}

/// Elsewhere a [`Folder`] keeps its path: each call checks the entry by name first (the
/// known Windows limit in the module docs).
#[cfg(not(unix))]
mod by_path {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io::{self, ErrorKind};
    use std::path::Path;

    use super::{EntryKind, Folder, appeared, component, not_plain};

    /// The kind of a file type, a symlink as [`EntryKind::Other`].
    fn kind_of(file_type: std::fs::FileType) -> EntryKind {
        if file_type.is_file() {
            EntryKind::File
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::Other
        }
    }

    impl Folder {
        /// The output folder `path` when it is a folder (a link the user chose is followed);
        /// `None` when it is missing.
        pub(crate) fn open(path: &Path) -> io::Result<Option<Folder>> {
            match std::fs::metadata(path) {
                Ok(meta) if meta.is_dir() => Ok(Some(Folder {
                    path: path.to_path_buf(),
                })),
                Ok(_) => Err(io::Error::new(
                    ErrorKind::NotADirectory,
                    "it exists and is not a directory",
                )),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        /// What `name` is, without following a symlink; `None` when it is missing.
        pub(crate) fn kind(&self, name: &OsStr) -> io::Result<Option<EntryKind>> {
            match std::fs::symlink_metadata(self.path.join(component(name)?)) {
                Ok(meta) => Ok(Some(kind_of(meta.file_type()))),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        /// The subfolder `name`, made first when `create`; anything but a plain directory
        /// there is refused.
        pub(crate) fn sub(&self, name: &str, create: bool) -> io::Result<Folder> {
            let path = self.path.join(component(OsStr::new(name))?);
            if create {
                match std::fs::create_dir(&path) {
                    Err(e) if e.kind() != ErrorKind::AlreadyExists => return Err(e),
                    _ => {}
                }
            }
            match self.kind(OsStr::new(name))? {
                Some(EntryKind::Dir) => Ok(Folder { path }),
                Some(_) => Err(not_plain(name)),
                None => Err(io::Error::from(ErrorKind::NotFound)),
            }
        }

        /// The regular file `name`, opened for reading; `None` for anything else.
        pub(crate) fn open_file(&self, name: &OsStr) -> io::Result<Option<File>> {
            if self.kind(name)? != Some(EntryKind::File) {
                return Ok(None);
            }
            let file = File::open(self.path.join(name))?;
            Ok(file.metadata()?.is_file().then_some(file))
        }

        /// Creates the file `name` exclusively and writes `bytes`.
        pub(crate) fn create_new(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
            use std::io::Write;
            let path = self.path.join(component(OsStr::new(name))?);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| match e.kind() {
                    ErrorKind::AlreadyExists => appeared(),
                    _ => e,
                })?;
            file.write_all(bytes)
        }

        /// Removes the file `name`; a symlink itself is removed, never its target.
        pub(crate) fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            std::fs::remove_file(self.path.join(component(name)?))
        }

        /// Removes the empty folder `name`.
        pub(crate) fn remove_dir(&self, name: &str) -> io::Result<()> {
            std::fs::remove_dir(self.path.join(component(OsStr::new(name))?))
        }

        /// The entries of the folder with their kinds.
        pub(crate) fn entries(&self) -> io::Result<Vec<(OsString, EntryKind)>> {
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&self.path)? {
                let entry = entry?;
                entries.push((entry.file_name(), kind_of(entry.file_type()?)));
            }
            Ok(entries)
        }
    }
}

#[cfg(test)]
mod rule_tests {
    use std::ffi::OsStr;

    use super::plain;

    /// The name rule of every system, checked on any host.
    #[test]
    fn only_one_plain_component_is_a_name() {
        for name in ["a/b", "..", ".", "", "a\\b", "/a", "a/"] {
            for drive in [false, true] {
                assert!(!plain(OsStr::new(name), drive), "{name:?} {drive}");
            }
        }
        for name in ["a.md", "page-1.png", ".hidden", "a b.md", "a..b"] {
            assert!(plain(OsStr::new(name), true), "{name:?}");
        }
        // A `:` names a drive on Windows; on Unix it is a plain char.
        for name in ["C:x.md", "C:", "a:b.md"] {
            assert!(!plain(OsStr::new(name), true), "{name:?}");
            assert!(plain(OsStr::new(name), false), "{name:?}");
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn a_drive_name_is_refused() {
        for name in ["C:x.md", "C:", "\\\\server\\share"] {
            let err = super::component(OsStr::new(name)).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{name:?}");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsStr;
    use std::io::ErrorKind;
    use std::os::unix::fs::symlink;

    use super::{EntryKind, Folder};

    /// A handle keeps pointing at the folder it opened: a swap of its name for a symlink
    /// redirects no write.
    #[test]
    fn a_swapped_parts_folder_takes_no_write_elsewhere() {
        let root = tempfile::tempdir().unwrap();
        let out = Folder::create(root.path()).unwrap();
        let victim = root.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        let parts = out.sub("x.parts", true).unwrap();
        std::fs::rename(root.path().join("x.parts"), root.path().join("moved")).unwrap();
        symlink(&victim, root.path().join("x.parts")).unwrap();
        parts.create_new("a.md", b"part").unwrap();
        assert_eq!(
            std::fs::read(root.path().join("moved/a.md")).unwrap(),
            b"part"
        );
        assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
        // A handle opened now refuses the link.
        let err = out.sub("x.parts", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotADirectory, "{err}");
        assert_eq!(
            err.to_string(),
            "x.parts exists and is not a plain directory"
        );
    }

    #[test]
    fn a_symlinked_subfolder_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let out = Folder::create(&root.path().join("out")).unwrap();
        let victim = root.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        symlink(&victim, root.path().join("out/x.assets")).unwrap();
        std::fs::write(root.path().join("out/y.parts"), "mine").unwrap();
        for name in ["x.assets", "y.parts"] {
            for create in [true, false] {
                let err = out.sub(name, create).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::NotADirectory, "{name}: {err}");
                assert_eq!(
                    err.to_string(),
                    format!("{name} exists and is not a plain directory")
                );
            }
        }
        assert_eq!(
            out.kind(OsStr::new("x.assets")).unwrap(),
            Some(EntryKind::Other)
        );
        assert_eq!(
            out.kind(OsStr::new("y.parts")).unwrap(),
            Some(EntryKind::File)
        );
        assert_eq!(out.kind(OsStr::new("none")).unwrap(), None);
        // A missing folder is not made without `create`.
        let err = out.sub("z.parts", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
        assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
        // An existing plain folder is fine, also with `create`.
        out.sub("z.parts", true).unwrap();
        out.sub("z.parts", true).unwrap();
        assert_eq!(
            out.kind(OsStr::new("z.parts")).unwrap(),
            Some(EntryKind::Dir)
        );
    }

    /// The output folder itself may be a symlink the user chose: it is resolved once.
    #[test]
    fn an_output_folder_that_is_a_symlink_is_used() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        let out = Folder::open(&link).unwrap().unwrap();
        assert_eq!(out.path(), link);
        out.create_new("a.md", b"a").unwrap();
        assert_eq!(std::fs::read(real.join("a.md")).unwrap(), b"a");
        let out = Folder::create(&link).unwrap();
        out.sub("a.parts", true)
            .unwrap()
            .create_new("b.md", b"b")
            .unwrap();
        assert_eq!(std::fs::read(real.join("a.parts/b.md")).unwrap(), b"b");
        // A missing folder is no handle; one under a file is an error, not a missing one.
        assert!(Folder::open(&root.path().join("none")).unwrap().is_none());
        std::fs::write(root.path().join("file"), "mine").unwrap();
        let err = Folder::open(&root.path().join("file/out")).err().unwrap();
        assert_ne!(err.kind(), ErrorKind::NotFound, "{err}");
        assert!(Folder::create(&root.path().join("file/out")).is_err());
    }

    #[test]
    fn a_name_with_a_separator_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let out = Folder::create(root.path()).unwrap();
        // Each of these would reach an existing entry if it were taken as a path.
        std::fs::create_dir_all(root.path().join("a/b")).unwrap();
        std::fs::write(root.path().join("a/b/c"), "keep").unwrap();
        let refused = |result: std::io::Result<()>, name: &str, what: &str| {
            let err = result.unwrap_err();
            assert_eq!(
                err.kind(),
                ErrorKind::InvalidInput,
                "{what} {name:?}: {err}"
            );
        };
        for name in ["a/b", "a/b/c", "..", ".", "", "a\\b", "/a"] {
            let os = OsStr::new(name);
            refused(out.kind(os).map(drop), name, "kind");
            refused(out.sub(name, false).map(drop), name, "sub");
            refused(out.sub(name, true).map(drop), name, "sub create");
            refused(out.open_file(os).map(drop), name, "open_file");
            refused(out.create_new(name, b"x"), name, "create_new");
            refused(out.remove_file(os), name, "remove_file");
            refused(out.remove_dir(name), name, "remove_dir");
        }
        assert_eq!(std::fs::read(root.path().join("a/b/c")).unwrap(), b"keep");
        let names: Vec<_> = out.entries().unwrap().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["a"]);
    }

    /// A FIFO swapped in for a file is not opened in a way that waits for a writer.
    #[test]
    fn a_fifo_is_never_waited_for() {
        let root = tempfile::tempdir().unwrap();
        let out = Folder::create(root.path()).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(root.path().join("a.md").as_os_str().as_bytes());
        // SAFETY: a valid NUL-terminated path; mkfifo only reads it.
        assert_eq!(unsafe { libc::mkfifo(path.unwrap().as_ptr(), 0o600) }, 0);
        assert_eq!(
            out.kind(OsStr::new("a.md")).unwrap(),
            Some(EntryKind::Other)
        );
        assert!(out.open_file(OsStr::new("a.md")).unwrap().is_none());
    }

    #[test]
    fn entries_give_each_kind_without_following() {
        let root = tempfile::tempdir().unwrap();
        let out = Folder::create(root.path()).unwrap();
        out.create_new("file.md", b"x").unwrap();
        out.sub("dir", true).unwrap();
        symlink(root.path().join("dir"), root.path().join("link")).unwrap();
        let mut entries = out.entries().unwrap();
        entries.sort();
        let expected = [
            ("dir", EntryKind::Dir),
            ("file.md", EntryKind::File),
            ("link", EntryKind::Other),
        ]
        .map(|(n, k)| (n.into(), k));
        assert_eq!(entries, expected);
        // A symlink is removed itself, never its target; an empty folder goes.
        out.remove_file(OsStr::new("link")).unwrap();
        assert!(root.path().join("dir").is_dir());
        out.remove_dir("dir").unwrap();
        out.remove_file(OsStr::new("file.md")).unwrap();
        assert!(out.entries().unwrap().is_empty());
        // `create_new` never replaces.
        out.create_new("a.md", b"first").unwrap();
        let err = out.create_new("a.md", b"second").unwrap_err();
        assert_eq!(
            err.to_string(),
            "it appeared while Rezzer was writing and is not replaced"
        );
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut out.open_file(OsStr::new("a.md")).unwrap().unwrap(),
            &mut text,
        )
        .unwrap();
        assert_eq!(text, "first");
    }
}
