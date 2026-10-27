// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::{
    ASSET_TYPES, Files, OTHER_EXTENSION, asset_extension, check_paths, is_asset_name, write_files,
    write_new,
};

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
    let path = dir.path().join("a.md");
    write_new(&path, b"first", false).unwrap();
    let err = write_new(&path, b"second", false).unwrap_err();
    assert!(err.to_string().contains("appeared"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    write_new(&path, b"third", true).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"third");
}

/// A file that appears after the check (another run at the same time, a file of the
/// user's) is not replaced by the write that follows.
#[test]
fn a_file_that_appears_after_the_check_stays() {
    let dir = tempfile::tempdir().unwrap();
    let ours = "---\ntype: source\nresource: \"/src/notes.txt\"\ngenerated: { by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }\n---\n\nnew\n";
    let f = files(ours);
    check_paths(dir.path(), &f).unwrap();
    let main = dir.path().join("notes.txt.md");
    let other = "---\ntype: source\nresource: \"/other/notes.txt\"\ngenerated: { by: rezzer/0.1.0, at: 2026-10-26T00:00:00Z }\nsource_sha256: \"11\"\n---\n\nother\n";
    for appeared in ["# mine\n", other] {
        std::fs::write(&main, appeared).unwrap();
        let err = write_files(dir.path(), &f).unwrap_err();
        assert!(err.contains("notes.txt.md"), "{err}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), appeared);
    }
    // Rezzer's own output for this source is replaced.
    std::fs::write(&main, ours.replace("new", "old")).unwrap();
    write_files(dir.path(), &f).unwrap();
    assert_eq!(std::fs::read_to_string(&main).unwrap(), ours);
}
