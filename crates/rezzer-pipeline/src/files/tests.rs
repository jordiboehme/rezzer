// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::{
    ASSET_TYPES, Files, OTHER_EXTENSION, asset_extension, check_paths, folder_stem_on,
    is_asset_name, main_name_on, write_files, write_new,
};
use std::borrow::Cow;

use crate::config::CacheOs;
use crate::folder::Folder;
use crate::names::legal_name;

#[test]
fn every_confirmed_type_has_its_extension() {
    // Every type the bytes can confirm is written under an extension Crystalline takes,
    // and the cleanup knows that name (ruling P5-AJ).
    for mime in rezzer_detect::CONFIRMED_MEDIA_TYPES {
        let ext = ASSET_TYPES
            .iter()
            .find(|(m, _)| *m == mime)
            .map(|(_, e)| *e);
        let ext = ext.unwrap_or_else(|| panic!("{mime} has no extension"));
        assert!(is_asset_name(&format!("attachment-1.{ext}")), "{ext}");
        assert!(is_asset_name(&format!("page-2-pic-3.{ext}")), "{ext}");
        assert!(!is_asset_name(&format!("logo.{ext}")), "{ext}");
    }
    assert!(is_asset_name("page-1.bin"));
    assert!(is_asset_name("page-12-code-1.png"));
    for name in [
        "page-.png",
        "page-1-pic-.png",
        "page-1-x-2.png",
        "pages-1.png",
        "attachment-a.png",
    ] {
        assert!(!is_asset_name(name), "{name}");
    }
    assert!(!is_asset_name("page-1.csv"));
    assert!(!is_asset_name("page-1.md"));
    assert_eq!(asset_extension(b"\x89PNG\r\n\x1a\n"), "png");
    assert_eq!(asset_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), "jpg");
    assert_eq!(asset_extension(b"GIF87a"), "gif");
    assert_eq!(asset_extension(b"%PDF-1.4\n"), "pdf");
    // A name in the bytes counts for nothing: this is text.
    assert_eq!(asset_extension(b"x.png"), "txt");
    assert_eq!(asset_extension(b"a,b\n1,2\n"), "txt");
    assert_eq!(asset_extension(b"{\"a\": 1}"), "json");
    assert_eq!(asset_extension(b"\x00\x01binary"), OTHER_EXTENSION);
}

fn files<'a>(main_markdown: &'a str) -> Files<'a> {
    Files {
        source: "/src/notes.txt",
        sha256: "00",
        main: "notes.txt",
        folder: "notes.txt.parts",
        main_markdown,
        parts: Vec::new(),
        json: None,
    }
}

/// Without `replace` nothing is removed: an entry that is there fails the write.
#[test]
fn a_free_path_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let folder = Folder::create(dir.path()).unwrap();
    let path = dir.path().join("a.md");
    write_new(&folder, "a.md", b"first", false).unwrap();
    let err = write_new(&folder, "a.md", b"second", false).unwrap_err();
    assert!(err.to_string().contains("appeared"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    write_new(&folder, "a.md", b"third", true).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"third");
}

/// A file that appears after the check (another run at the same time, a file of the
/// user's) is not replaced by the write that follows.
#[test]
fn a_file_that_appears_after_the_check_stays() {
    let dir = tempfile::tempdir().unwrap();
    let ours = "---\ntype: source\nresource: \"/src/notes.txt\"\ngenerated: { by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }\n---\n\nnew\n";
    let f = files(ours);
    let folder = Folder::create(dir.path()).unwrap();
    check_paths(Some(&folder), &f).unwrap();
    let main = dir.path().join("notes.txt.md");
    let other = "---\ntype: source\nresource: \"/other/notes.txt\"\ngenerated: { by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }\nsource_sha256: \"11\"\n---\n\nother\n";
    for appeared in ["# mine\n", other] {
        std::fs::write(&main, appeared).unwrap();
        let err = write_files(&folder, &f).unwrap_err();
        assert!(err.contains("notes.txt.md"), "{err}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), appeared);
    }
    // Rezzer's own output for this source is replaced.
    std::fs::write(&main, ours.replace("new", "old")).unwrap();
    write_files(&folder, &f).unwrap();
    assert_eq!(std::fs::read_to_string(&main).unwrap(), ours);
}

/// A Markdown file of Rezzer's for `/src/x.txt`, a part when `part`.
#[cfg(unix)]
fn ours(part: bool) -> String {
    let part = if part { "part: 1\n" } else { "" };
    format!(
        "---\ntype: source\nresource: \"/src/x.txt\"\n{part}generated: {{ by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }}\n---\n\nx\n"
    )
}

/// A handle opened before the swap keeps removing in the folder it opened: a folder swapped
/// in under its name loses no file.
#[cfg(unix)]
#[test]
fn a_swapped_folder_loses_no_file_elsewhere() {
    use std::collections::HashSet;
    let root = tempfile::tempdir().unwrap();
    let out = Folder::create(&root.path().join("out")).unwrap();
    let victim = root.path().join("victim");
    std::fs::create_dir(&victim).unwrap();
    std::fs::write(victim.join("page-3.png"), "keep").unwrap();
    let assets = out.sub("x.assets", true).unwrap();
    std::fs::write(root.path().join("out/x.assets/page-4.png"), "old").unwrap();
    let moved = root.path().join("out/moved");
    std::fs::rename(root.path().join("out/x.assets"), &moved).unwrap();
    std::os::unix::fs::symlink(&victim, root.path().join("out/x.assets")).unwrap();
    super::remove_stale(&assets, &HashSet::new(), super::Stale::Assets).unwrap();
    assert!(!moved.join("page-4.png").exists());
    assert_eq!(std::fs::read(victim.join("page-3.png")).unwrap(), b"keep");
    // A parts handle the same way: only the folder it opened loses a part.
    std::fs::write(victim.join("chapter-1.md"), ours(true)).unwrap();
    std::fs::write(moved.join("chapter-2.md"), ours(true)).unwrap();
    let f = Files {
        source: "/src/x.txt",
        ..files("")
    };
    let source = super::SourceId::of(&f);
    super::remove_stale(&assets, &HashSet::new(), super::Stale::Parts(&source)).unwrap();
    assert!(!moved.join("chapter-2.md").exists());
    assert!(victim.join("chapter-1.md").exists());
}

/// A symlink in the output folder is never opened: it is no file of Rezzer's, and a write
/// that may replace removes the link itself and leaves its target as it is.
#[cfg(unix)]
#[test]
fn a_symlinked_file_is_never_opened() {
    use std::ffi::OsStr;
    let root = tempfile::tempdir().unwrap();
    let out = Folder::create(&root.path().join("out")).unwrap();
    let victim = root.path().join("victim.md");
    std::fs::write(&victim, ours(false)).unwrap();
    let link = root.path().join("out/x.txt.md");
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    assert!(out.open_file(OsStr::new("x.txt.md")).unwrap().is_none());
    let f = Files {
        source: "/src/x.txt",
        ..files("")
    };
    let source = super::SourceId::of(&f);
    let found = super::holder(&out, OsStr::new("x.txt.md"), &source, super::Kind::Main);
    assert_eq!(found, super::Holder::Foreign);
    write_new(&out, "x.txt.md", b"new", true).unwrap();
    assert!(!std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(std::fs::read(&link).unwrap(), b"new");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), ours(false));
}

/// The assets folder is opened once for the pictures, and the cleanup uses that handle: a
/// swap in between removes nothing from the folder the link points at.
#[cfg(unix)]
#[test]
fn a_swap_between_pictures_and_cleaning_is_caught() {
    use rezzer_model::{Asset, Blob, Confidence, Detection, Document, FormatId, SourceInfo};
    let root = tempfile::tempdir().unwrap();
    let out = Folder::create(&root.path().join("out")).unwrap();
    let assets = root.path().join("out/x.txt.assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(assets.join("page-2.png"), "old").unwrap();
    let victim = root.path().join("victim");
    std::fs::create_dir(&victim).unwrap();
    for name in ["page-1.png", "page-2.png"] {
        std::fs::write(victim.join(name), "keep").unwrap();
    }
    let mut document = Document::new(SourceInfo {
        path: None,
        name: "x.txt".into(),
        sha256: "0".repeat(64),
        size: 0,
        format: FormatId::Text,
        detection: Detection {
            confidence: Confidence::Likely,
            evidence: "test".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
        encoding: None,
        reader: None,
    });
    document.assets.push(Asset {
        id: "page-1".into(),
        path: None,
        sha256: String::new(),
        mime: "image/png".into(),
        caption: None,
        alt: None,
        needs_vision: true,
        codes: Vec::new(),
        code_unread: false,
    });
    let blobs = vec![Blob {
        asset: "page-1".into(),
        bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
    }];
    let planned = super::plan_assets(&mut document, blobs, "x.txt");
    let (written, lost, handle) = super::write_assets(&mut document, planned, &out, "x.txt");
    assert!(!lost);
    assert!(handle.is_some());
    let moved = root.path().join("out/moved");
    std::fs::rename(&assets, &moved).unwrap();
    std::os::unix::fs::symlink(&victim, &assets).unwrap();
    super::clean_assets(&out, handle.as_ref(), "x.txt", &written).unwrap();
    let mut left: Vec<_> = std::fs::read_dir(&moved)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    left.sort();
    assert_eq!(left, ["page-1.png"]);
    for name in ["page-1.png", "page-2.png"] {
        assert_eq!(std::fs::read(victim.join(name)).unwrap(), b"keep", "{name}");
    }
    // Without a handle the cleanup opens the folder itself and refuses the link.
    super::clean_assets(&out, None, "x.txt", &Default::default()).unwrap();
    assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 2);
}

#[test]
fn a_name_like_a_hashed_stem_gets_its_own_hash() {
    use super::folder_stem;
    let hashed_tail = |s: &str| {
        s.len() > 9
            && s.as_bytes()[s.len() - 9] == b'-'
            && s[s.len() - 8..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let y = folder_stem("a b.pdf");
    assert!(hashed_tail(&y), "{y}");
    assert_ne!(folder_stem(&y), y);
    let upper = folder_stem(&y.to_uppercase());
    assert!(!upper.eq_ignore_ascii_case(&y), "{upper} and {y}");
    assert_eq!(folder_stem("report-2024.pdf"), "report-2024.pdf");
    assert_eq!(folder_stem("x-deadbee"), "x-deadbee");
    let own = folder_stem("x-DEADBEEF");
    assert!(own.starts_with("x-DEADBEEF-"), "{own}");
    assert!(hashed_tail(&own), "{own}");
}

/// Every value of [`CacheOs`]: the names Rezzer writes follow the rules of one of them.
const EVERY_OS: [CacheOs; 3] = [CacheOs::Unix, CacheOs::MacOs, CacheOs::Windows];

/// Whether `got` is `base` plus `-` and the name hash of `original`.
fn hashed_from(got: &str, base: &str, original: &str) -> bool {
    got == format!("{base}-{}", super::name_hash(original))
}

#[test]
fn windows_forbids_these_characters() {
    for c in [
        ':', '<', '>', '"', '|', '?', '*', '\u{0}', '\u{1}', '\u{1f}',
    ] {
        let source = format!("a{c}b.txt");
        let main = main_name_on(&source, CacheOs::Windows);
        assert!(hashed_from(&main, "a_b.txt", &source), "{source:?}: {main}");
        let stem = folder_stem_on(&source, CacheOs::Windows);
        assert!(hashed_from(&stem, "a_b.txt", &source), "{source:?}: {stem}");
        for os in [CacheOs::Unix, CacheOs::MacOs] {
            // As today: a control character is replaced and hashed, the others stay.
            let main = main_name_on(&source, os);
            if c.is_control() {
                assert!(hashed_from(&main, "a_b.txt", &source), "{source:?}: {main}");
            } else {
                assert_eq!(main, source);
            }
            assert_eq!(folder_stem_on(&source, os), stem, "{source:?}");
        }
    }
}

#[test]
fn windows_takes_no_trailing_dot_or_space() {
    for (source, base) in [
        ("a.", "a_"),
        ("a ", "a_"),
        ("a. .", "a___"),
        ("report.pdf.", "report.pdf_"),
        // The leading dot became `_` already; the trailing dots follow on Windows only.
        ("...", "___"),
    ] {
        let main = main_name_on(source, CacheOs::Windows);
        assert!(hashed_from(&main, base, source), "{source:?}: {main}");
        for os in [CacheOs::Unix, CacheOs::MacOs] {
            let main = main_name_on(source, os);
            if source == "..." {
                assert!(hashed_from(&main, "_..", source), "{main}");
            } else {
                assert_eq!(main, source);
            }
        }
    }
    // The folder stem has no space (it became `_` already); a trailing dot is new on Windows.
    let stem = folder_stem_on("a.", CacheOs::Windows);
    assert!(hashed_from(&stem, "a_", "a."), "{stem}");
    assert_eq!(folder_stem_on("a.", CacheOs::Unix), "a.");
    // The rule gives a name it leaves alone: each of its cases applied twice gives the same.
    for name in [
        "a:b<c>d\"e|f?g*h\u{1}.txt",
        "a. .",
        "a ",
        "...",
        "CON",
        "con .txt",
        "Lpt9.md",
        "com0.pdf",
        "COM\u{b9}.txt",
        "nul. ",
        "report.pdf",
    ] {
        let once = legal_name(name, CacheOs::Windows).into_owned();
        assert_eq!(
            legal_name(&once, CacheOs::Windows),
            Cow::Borrowed(once.as_str()),
            "{name:?}"
        );
    }
}

#[test]
fn windows_device_names_are_replaced() {
    for (source, base) in [
        ("CON", "_"),
        ("con.txt", "_.txt"),
        ("Lpt9.md", "_.md"),
        ("COM1.pdf", "_.pdf"),
        ("nul.tar.gz", "_.tar.gz"),
        ("Prn", "_"),
        ("aUx.docx", "_.docx"),
        ("con .txt", "_ .txt"),
        ("com\u{b9}.txt", "_.txt"),
        ("LPT\u{b3}", "_"),
        ("LPT0.txt", "_.txt"),
        ("com0.pdf", "_.pdf"),
    ] {
        let main = main_name_on(source, CacheOs::Windows);
        assert!(hashed_from(&main, base, source), "{source:?}: {main}");
        for os in [CacheOs::Unix, CacheOs::MacOs] {
            assert_eq!(main_name_on(source, os), source);
        }
    }
    // The parts and assets folders are `<stem>.parts` and `<stem>.assets`, so a stem that
    // starts with a device name is replaced too, with one hash, also when it is hashed for
    // another reason (`con.md` ends in `.md`).
    for (source, base) in [
        ("con.txt", "_.txt"),
        ("CON", "_"),
        ("con.md", "_.md"),
        ("nul.tar.gz", "_.tar.gz"),
    ] {
        let stem = folder_stem_on(source, CacheOs::Windows);
        assert!(hashed_from(&stem, base, source), "{source:?}: {stem}");
    }
    assert_eq!(folder_stem_on("con.txt", CacheOs::Unix), "con.txt");
    assert_eq!(folder_stem_on("CON", CacheOs::MacOs), "CON");
    // `con .txt` has its space replaced in the folder stem, so `con_.txt` is no device name.
    assert_eq!(
        folder_stem_on("con .txt", CacheOs::Windows),
        folder_stem_on("con .txt", CacheOs::Unix)
    );
}

#[test]
fn names_that_only_look_like_a_device_stay() {
    for source in [
        "CONSOLE.txt",
        "COM10.pdf",
        "LPT00.txt",
        "a.con",
        "COM.txt",
        "xcon.txt",
        "report.pdf",
        "Q3 report (final) #2.pdf",
    ] {
        for os in EVERY_OS {
            assert_eq!(main_name_on(source, os), source, "{os:?}");
            assert_eq!(
                folder_stem_on(source, os),
                folder_stem_on(source, CacheOs::Unix),
                "{os:?}"
            );
        }
    }
    assert_eq!(folder_stem_on("COM10.pdf", CacheOs::Windows), "COM10.pdf");
}

#[test]
fn the_windows_hash_is_from_the_original_name() {
    let colon = main_name_on("a:b.txt", CacheOs::Windows);
    let question = main_name_on("a?b.txt", CacheOs::Windows);
    assert_ne!(colon, question);
    assert_ne!(
        main_name_on("con.txt", CacheOs::Windows),
        main_name_on("nul.txt", CacheOs::Windows)
    );
    assert_ne!(
        folder_stem_on("con.txt", CacheOs::Windows),
        folder_stem_on("nul.txt", CacheOs::Windows)
    );
    // A name that is legal already is not changed.
    assert_eq!(main_name_on("a_b.txt", CacheOs::Windows), "a_b.txt");
}

#[test]
fn a_changed_windows_name_cannot_be_copied_by_another_source() {
    // `con.txt` gets the folder stem `_.txt-<hash>`. A source named after that stem looks
    // hashed, so it gets a hash of its own, also in upper case: the two never share an
    // assets folder, so neither run removes the other one's pictures as stale.
    for source in ["con.txt", "a:b.txt", "a."] {
        let stem = folder_stem_on(source, CacheOs::Windows);
        let copy = folder_stem_on(&stem, CacheOs::Windows);
        assert_ne!(copy, stem, "{source}");
        let upper = folder_stem_on(&stem.to_uppercase(), CacheOs::Windows);
        assert!(!upper.eq_ignore_ascii_case(&stem), "{upper} and {stem}");
    }
}

#[test]
fn windows_names_keep_the_length_bounds() {
    for name in [
        format!("{}:.txt", "a".repeat(500)),
        format!("con.{}", "日".repeat(100)),
        format!("{}.", "b".repeat(300)),
        "a:".to_string(),
        "NUL".to_string(),
    ] {
        for os in EVERY_OS {
            let got = main_name_on(&name, os);
            assert!(got.len() <= rezzer_render::MAIN_NAME_MAX, "{got}");
            assert!(
                got.chars().count() <= name.chars().count().max(4) + 9,
                "{got}"
            );
            if os == CacheOs::Windows {
                assert!(!got.contains(':') && !got.ends_with(['.', ' ']), "{got}");
            }
        }
    }
}
