// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::{
    ASSET_TYPES, Files, OTHER_EXTENSION, asset_extension, check_paths, is_asset_name, write_files,
    write_new,
};
use crate::folder::Folder;

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
