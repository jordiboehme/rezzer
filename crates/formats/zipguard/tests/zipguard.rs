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
    // Each entry counts once, however often it is read.
    assert_eq!(archive.unpacked(), (6 + text.len()) as u64);
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
fn each_entry_counts_once() {
    let bytes = build(&[
        ("a", &[1u8; 600], CompressionMethod::Stored),
        ("b", &[2u8; 390], CompressionMethod::Stored),
    ]);
    let limits = Limits {
        max_total_bytes: 1000,
        ..Limits::default()
    };
    let mut archive = Archive::open(&bytes, limits).unwrap();
    assert_eq!(archive.measure("a").unwrap(), 600);
    assert_eq!(archive.measure("b").unwrap(), 390);
    // The archive is near its total limit, and its entries can still be read again.
    for _ in 0..3 {
        assert_eq!(archive.read("a").unwrap().len(), 600);
        assert_eq!(archive.read("b").unwrap().len(), 390);
    }
    assert_eq!(archive.unpacked(), 990);
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

#[test]
fn duplicate_entry_names_are_damaged() {
    let mut bytes = build(&[
        ("a.xml", b"first", CompressionMethod::Stored),
        ("b.xml", b"second", CompressionMethod::Stored),
    ]);
    // Rename b.xml to a.xml in its local and central headers.
    let mut patched = 0;
    for i in 0..bytes.len() - 5 {
        if &bytes[i..i + 5] == b"b.xml" {
            bytes[i] = b'a';
            patched += 1;
        }
    }
    assert_eq!(patched, 2);
    match Archive::open(&bytes, Limits::default()) {
        Err(ZipError::Damaged(why)) => assert!(why.contains("duplicate"), "{why}"),
        Err(e) => panic!("{e}"),
        Ok(a) => panic!("opened with {:?}", a.names()),
    }
}

#[test]
fn central_name_unlike_its_local_header_is_damaged() {
    let mut bytes = build(&[("a.xml", b"first", CompressionMethod::Stored)]);
    // Only the local header says x.xml.
    let at = bytes.windows(5).position(|w| w == b"a.xml").unwrap();
    bytes[at] = b'x';
    assert!(matches!(
        Archive::open(&bytes, Limits::default()),
        Err(ZipError::Damaged(_))
    ));
}

/// CRC-32 (IEEE), bit by bit: enough for the few bytes of a hand-built archive.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A stored archive written by hand, so each entry can carry an extra field the zip
/// writer would not make: `(name, data, extra field id, extra field data)`. The extra field
/// is in the local header and in the central directory.
fn hand_built(entries: &[(&str, &[u8], u16, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, id, field) in entries {
        let offset = u32::try_from(out.len()).unwrap();
        let mut extra = Vec::new();
        extra.extend_from_slice(&id.to_le_bytes());
        extra.extend_from_slice(&u16::try_from(field.len()).unwrap().to_le_bytes());
        extra.extend_from_slice(field);
        let crc = crc32(data);
        let size = u32::try_from(data.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let extra_len = u16::try_from(extra.len()).unwrap();
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u32.to_le_bytes()); // time and date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&extra_len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&extra);
        out.extend_from_slice(data);
        // Central directory header.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&extra_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&extra);
    }
    let start = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(entries.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The alignment extra fields: 0xa11e from Apache Commons Compress and 0xd935 from
/// Android's ZFile. Both always hold at least the 2-byte alignment. A shorter field is
/// malformed data that no known tool writes; zip refuses such an archive as damaged
/// ("Alignment extra field too short") since 9.1.3, and zipguard passes that on as
/// [`ZipError::Damaged`]. A normal field of either kind opens and its entry reads.
#[test]
fn alignment_extra_fields_short_and_normal() {
    let bytes = hand_built(&[
        ("commons.txt", b"commons field", 0xa11e, &[4, 0]),
        ("android.txt", b"android field", 0xd935, &[4, 0, 0, 0]),
    ]);
    let mut archive = Archive::open(&bytes, Limits::default()).expect("the archive opens");
    assert_eq!(archive.names(), ["commons.txt", "android.txt"]);
    assert_eq!(archive.read("commons.txt").unwrap(), b"commons field");
    assert_eq!(archive.read("android.txt").unwrap(), b"android field");

    for id in [0xa11e, 0xd935] {
        for field in [&[][..], &[4][..]] {
            let bytes = hand_built(&[
                ("normal.txt", b"normal field", 0xa11e, &[4, 0]),
                ("short.txt", b"short field", id, field),
            ]);
            match Archive::open(&bytes, Limits::default()) {
                Err(ZipError::Damaged(why)) => {
                    assert!(why.contains("Alignment extra field too short"), "{why}");
                }
                Err(other) => panic!("{id:#06x} with {} bytes: {other}", field.len()),
                Ok(_) => panic!("{id:#06x} with {} bytes opened", field.len()),
            }
        }
    }
}
