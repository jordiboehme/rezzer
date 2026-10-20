// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};
use zipguard::{Archive, Limits, ZipError};

fn build(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data, method) in entries {
        let options = SimpleFileOptions::default().compression_method(*method);
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Replaces every little-endian u32 `from` that follows a header signature with `to`
/// at the given offset from the signature.
fn patch_u32(bytes: &mut [u8], signature: [u8; 4], offset: usize, from: u32, to: u32) -> usize {
    let mut hits = 0;
    for i in 0..bytes.len().saturating_sub(offset + 4) {
        if bytes[i..i + 4] == signature {
            let at = i + offset;
            if bytes[at..at + 4] == from.to_le_bytes() {
                bytes[at..at + 4].copy_from_slice(&to.to_le_bytes());
                hits += 1;
            }
        }
    }
    hits
}

#[test]
fn reads_a_stored_and_a_deflated_entry() {
    let text = "hello hello hello hello ".repeat(20);
    let bytes = build(&[
        ("a.txt", b"stored", CompressionMethod::Stored),
        ("dir/b.txt", text.as_bytes(), CompressionMethod::Deflated),
    ]);
    let mut archive = Archive::open(&bytes, Limits::default()).unwrap();
    assert_eq!(archive.names(), vec!["a.txt", "dir/b.txt"]);
    assert!(archive.contains("a.txt"));
    assert!(!archive.contains("nope"));
    assert_eq!(archive.read("a.txt").unwrap(), b"stored");
    assert_eq!(archive.read("dir/b.txt").unwrap(), text.as_bytes());
    assert_eq!(archive.measure("dir/b.txt").unwrap(), text.len() as u64);
    assert_eq!(archive.unpacked(), (6 + 2 * text.len()) as u64);
    assert_eq!(
        archive.read("nope"),
        Err(ZipError::Missing("nope".to_string()))
    );
}

#[test]
fn declared_size_lie_is_caught() {
    let big = vec![b'x'; 10_000_000];
    let mut bytes = build(&[("lie.txt", &big, CompressionMethod::Deflated)]);
    // Local header: size at 22. Central directory: size at 24.
    let local = patch_u32(
        &mut bytes,
        [0x50, 0x4b, 0x03, 0x04],
        22,
        big.len() as u32,
        10,
    );
    let central = patch_u32(
        &mut bytes,
        [0x50, 0x4b, 0x01, 0x02],
        24,
        big.len() as u32,
        10,
    );
    assert_eq!((local, central), (1, 1));
    let limits = Limits {
        max_entry_bytes: 1 << 20,
        max_ratio: u64::MAX / 2,
        ..Limits::default()
    };
    let mut archive = Archive::open(&bytes, limits).unwrap();
    assert!(matches!(
        archive.read("lie.txt"),
        Err(ZipError::Limit { .. })
    ));
    assert!(matches!(
        archive.measure("lie.txt"),
        Err(ZipError::Limit { .. })
    ));
    // Nothing past the limit was counted.
    assert!(archive.unpacked() <= 2 * (1 << 20));
}

#[test]
fn ratio_bomb_stops() {
    let data = vec![0u8; 200_000];
    let bytes = build(&[("bomb.bin", &data, CompressionMethod::Deflated)]);
    let mut archive = Archive::open(&bytes, Limits::default()).unwrap();
    match archive.read("bomb.bin") {
        Err(ZipError::Limit { what }) => assert!(what.contains("ratio"), "{what}"),
        other => panic!("expected a ratio limit, got {other:?}"),
    }
    // A generous ratio lets the same entry through.
    let limits = Limits {
        max_ratio: 100_000,
        ..Limits::default()
    };
    let mut archive = Archive::open(&bytes, limits).unwrap();
    assert_eq!(archive.read("bomb.bin").unwrap().len(), 200_000);
}

#[test]
fn total_budget_spans_entries() {
    let bytes = build(&[
        ("a", &[1u8; 600], CompressionMethod::Stored),
        ("b", &[2u8; 600], CompressionMethod::Stored),
    ]);
    let limits = Limits {
        max_total_bytes: 1000,
        ..Limits::default()
    };
    let mut archive = Archive::open(&bytes, limits).unwrap();
    assert_eq!(archive.measure("a").unwrap(), 600);
    assert!(matches!(archive.read("b"), Err(ZipError::Limit { .. })));
    assert_eq!(archive.unpacked(), 1000);
}

#[test]
fn entry_count_is_limited() {
    let bytes = build(&[
        ("a", b"1", CompressionMethod::Stored),
        ("b", b"2", CompressionMethod::Stored),
    ]);
    let limits = Limits {
        max_entries: 1,
        ..Limits::default()
    };
    assert!(matches!(
        Archive::open(&bytes, limits),
        Err(ZipError::Limit { .. })
    ));
}

#[test]
fn encrypted_entry_is_reported() {
    let mut bytes = build(&[("secret.txt", b"classified", CompressionMethod::Stored)]);
    // Set the encryption bit (bit 0) of the general purpose flags in both headers.
    for (signature, at) in [([0x50, 0x4b, 0x03, 0x04], 6), ([0x50, 0x4b, 0x01, 0x02], 8)] {
        let i = bytes.windows(4).position(|w| w == signature).unwrap();
        bytes[i + at] |= 1;
    }
    let mut archive = Archive::open(&bytes, Limits::default()).unwrap();
    assert_eq!(
        archive.read("secret.txt"),
        Err(ZipError::Encrypted("secret.txt".to_string()))
    );
    assert_eq!(archive.unpacked(), 0);
}

#[test]
fn traversal_name_is_damaged() {
    for name in [
        "../evil.txt",
        "a/../../evil.txt",
        "/abs.txt",
        "C:/win.txt",
        "a\\..\\b",
    ] {
        let bytes = build(&[(name, b"x", CompressionMethod::Stored)]);
        assert!(
            matches!(
                Archive::open(&bytes, Limits::default()),
                Err(ZipError::Damaged(_))
            ),
            "{name}"
        );
    }
    let bytes = build(&[("ok\0bad", b"x", CompressionMethod::Stored)]);
    assert!(matches!(
        Archive::open(&bytes, Limits::default()),
        Err(ZipError::Damaged(_))
    ));
    // A dot-dot inside a name part is fine.
    let bytes = build(&[("a..b/c.txt", b"x", CompressionMethod::Stored)]);
    assert!(Archive::open(&bytes, Limits::default()).is_ok());
}

#[test]
fn not_a_zip() {
    for bytes in [&b""[..], b"hello world, not a zip at all", &[0u8; 100][..]] {
        assert!(matches!(
            Archive::open(bytes, Limits::default()),
            Err(ZipError::NotZip)
        ));
    }
}

#[test]
fn damaged_entry_data_is_damaged() {
    let mut bytes = build(&[("a.txt", b"some stored text here", CompressionMethod::Stored)]);
    // Flip a data byte so the checksum fails.
    let i = bytes.windows(9).position(|w| w == b"some stor").unwrap();
    bytes[i] ^= 0xff;
    let mut archive = Archive::open(&bytes, Limits::default()).unwrap();
    assert!(matches!(archive.read("a.txt"), Err(ZipError::Damaged(_))));
}

#[test]
fn errors_display() {
    assert_eq!(ZipError::NotZip.to_string(), "not a ZIP archive");
    assert!(
        ZipError::Limit { what: "x".into() }
            .to_string()
            .contains("limit")
    );
}
