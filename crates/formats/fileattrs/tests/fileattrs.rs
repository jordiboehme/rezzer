// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Binary plist string arrays, Zone.Identifier text and extended attributes.
//!
//! Most plists are built here byte by byte from the documented layout. The files in
//! `tests/data/` are real bytes made once on macOS 27 and committed; no test runs a tool:
//!
//! - `where_froms_plutil.bplist`: `plutil -convert binary1` of an XML array with two URLs
//!   (both longer than 14 characters, so their lengths follow the marker)
//! - `tags_plutil.bplist`: `plutil -convert binary1` of `Größe\n6`, `Red\n6` and `日本`
//! - `tags_finder.bplist`: the `com.apple.metadata:_kMDItemUserTags` attribute the system
//!   wrote after setting the tags `Größe` and `Blue` through `NSURLTagNamesKey`
//! - `integers_plutil.bplist`: `plutil -convert binary1` of an array with the integers
//!   18446744073709551615 and -1

use fileattrs::{
    BplistError, MAX_ATTR_BYTES, MAX_TAGS, finder_tags, string_array, xdg_tags, zone_host_url,
    zone_text,
};

// ---------------------------------------------------------------------------------------
// Building plists

/// An ASCII string object, with the length in the marker or after it.
fn ascii(text: &str) -> Vec<u8> {
    let mut object = marker(0x5, text.len());
    object.extend_from_slice(text.as_bytes());
    object
}

/// A UTF-16BE string object.
fn utf16(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut object = marker(0x6, units.len());
    for unit in units {
        object.extend_from_slice(&unit.to_be_bytes());
    }
    object
}

/// An array object with one-byte references.
fn array(refs: &[u8]) -> Vec<u8> {
    let mut object = marker(0xA, refs.len());
    object.extend_from_slice(refs);
    object
}

/// A marker byte, with an integer length object after it when the count is 15 or more.
fn marker(kind: u8, count: usize) -> Vec<u8> {
    if count < 15 {
        return vec![(kind << 4) | count as u8];
    }
    let mut out = vec![(kind << 4) | 0x0F];
    if count <= 0xFF {
        out.push(0x10);
        out.push(count as u8);
    } else if count <= 0xFFFF {
        out.push(0x11);
        out.extend_from_slice(&(count as u16).to_be_bytes());
    } else {
        out.push(0x12);
        out.extend_from_slice(&(count as u32).to_be_bytes());
    }
    out
}

/// Header, objects, an offset table of `offset_size`-byte entries and the trailer.
fn plist(objects: &[Vec<u8>], top: u64, offset_size: u8, ref_size: u8) -> Vec<u8> {
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::new();
    for object in objects {
        offsets.push(out.len() as u64);
        out.extend_from_slice(object);
    }
    let table_start = out.len() as u64;
    for offset in offsets {
        let bytes = offset.to_be_bytes();
        out.extend_from_slice(&bytes[8 - offset_size as usize..]);
    }
    out.extend_from_slice(&[0; 6]);
    out.push(offset_size);
    out.push(ref_size);
    out.extend_from_slice(&(objects.len() as u64).to_be_bytes());
    out.extend_from_slice(&top.to_be_bytes());
    out.extend_from_slice(&table_start.to_be_bytes());
    out
}

/// The error `string_array` gives for bytes that must be rejected.
fn error(bytes: &[u8]) -> BplistError {
    string_array(bytes).expect_err("hostile bytes were accepted")
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

// ---------------------------------------------------------------------------------------
// Binary plist

#[test]
fn where_froms_two_urls() {
    let bytes = plist(
        &[
            array(&[1, 2]),
            ascii("https://a.example/f.pdf"),
            ascii("https://a.example/"),
        ],
        0,
        1,
        1,
    );
    assert_eq!(
        string_array(&bytes).unwrap(),
        ["https://a.example/f.pdf", "https://a.example/"]
    );
}

#[test]
fn short_strings_carry_their_length_in_the_marker() {
    let bytes = plist(&[array(&[1, 2]), ascii("a"), ascii("")], 0, 1, 1);
    assert_eq!(string_array(&bytes).unwrap(), ["a", ""]);
}

#[test]
fn utf16_tag_names() {
    let bytes = plist(
        &[array(&[1, 2]), utf16("Größe\n6"), ascii("Red\n6")],
        0,
        1,
        1,
    );
    assert_eq!(string_array(&bytes).unwrap(), ["Größe\n6", "Red\n6"]);
}

#[test]
fn top_object_need_not_be_first() {
    // Two-byte offsets and references, the array last.
    let mut top = marker(0xA, 2);
    top.extend_from_slice(&[0, 0, 0, 0]);
    let bytes = plist(&[ascii("x"), top], 1, 2, 2);
    assert_eq!(string_array(&bytes).unwrap(), ["x", "x"]);
}

#[test]
fn utf8_strings_are_read() {
    let mut object = vec![0x70 | 7];
    object.extend_from_slice("Größe".as_bytes());
    let bytes = plist(&[array(&[1]), object], 0, 1, 1);
    assert_eq!(string_array(&bytes).unwrap(), ["Größe"]);
}

#[test]
fn non_strings_are_skipped() {
    let int = vec![0x10, 7];
    let data = vec![0x42, 1, 2];
    let bytes = plist(
        &[array(&[1, 2, 3, 4]), int, data, vec![0x09], ascii("ok")],
        0,
        1,
        1,
    );
    assert_eq!(string_array(&bytes).unwrap(), ["ok"]);
}

#[test]
fn invalid_text_is_skipped() {
    let bad_ascii = vec![0x51, 0xC3];
    let lone_surrogate = vec![0x61, 0xD8, 0x00];
    let bad_utf8 = vec![0x71, 0xFF];
    let bytes = plist(
        &[
            array(&[1, 2, 3, 4]),
            bad_ascii,
            lone_surrogate,
            bad_utf8,
            ascii("ok"),
        ],
        0,
        1,
        1,
    );
    assert_eq!(string_array(&bytes).unwrap(), ["ok"]);
}

#[test]
fn long_strings_take_their_length_from_an_integer() {
    let long = "x".repeat(300);
    let bytes = plist(&[array(&[1, 2]), ascii(&long), utf16(&long)], 0, 2, 1);
    assert_eq!(string_array(&bytes).unwrap(), [long.clone(), long]);
}

#[test]
fn sixteen_byte_length_integer_is_read() {
    let mut object = vec![0x5F, 0x14];
    object.extend_from_slice(&[0; 15]);
    object.push(3);
    object.extend_from_slice(b"abc");
    let bytes = plist(&[array(&[1]), object], 0, 1, 1);
    assert_eq!(string_array(&bytes).unwrap(), ["abc"]);

    // A length that does not fit in 64 bits is an error.
    let mut huge = vec![0x5F, 0x14, 1];
    huge.extend_from_slice(&[0; 15]);
    let bytes = plist(&[array(&[1]), huge], 0, 1, 1);
    assert_eq!(error(&bytes), BplistError::BadLength);
}

#[test]
fn bplist_rejects_cycles_and_bad_offsets() {
    // An array that holds itself.
    let selfish = plist(&[array(&[0])], 0, 1, 1);
    assert!(matches!(string_array(&selfish), Err(BplistError::Cycle)));

    // A cycle through another array.
    let loop_two = plist(&[array(&[1]), array(&[0])], 0, 1, 1);
    assert!(matches!(string_array(&loop_two), Err(BplistError::Cycle)));

    // An offset past the end.
    let mut past = plist(&[array(&[1]), ascii("a")], 0, 1, 1);
    let table = past.len() - 32 - 2;
    past[table + 1] = 0xF0;
    assert_eq!(error(&past), BplistError::BadOffset);

    // An object reference size of 0.
    let mut zero_ref = plist(&[array(&[1]), ascii("a")], 0, 1, 1);
    let at = zero_ref.len() - 25;
    zero_ref[at] = 0;
    assert_eq!(error(&zero_ref), BplistError::BadTrailer);

    // An offset int size of 0 and one above 8.
    for size in [0, 9] {
        let mut bad = plist(&[array(&[1]), ascii("a")], 0, 1, 1);
        let at = bad.len() - 26;
        bad[at] = size;
        assert_eq!(error(&bad), BplistError::BadTrailer, "offset size {size}");
    }

    // A reference to an object that does not exist.
    let dangling = plist(&[array(&[5]), ascii("a")], 0, 1, 1);
    assert_eq!(error(&dangling), BplistError::BadReference);

    // A string longer than the object area.
    let long = plist(&[array(&[1]), vec![0x5E, b'a']], 0, 1, 1);
    assert_eq!(error(&long), BplistError::BadLength);

    // An array that claims more references than there are bytes.
    let wide = plist(
        &[vec![
            0xAF, 0x13, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]],
        0,
        1,
        1,
    );
    assert_eq!(error(&wide), BplistError::BadLength);

    // A negative length.
    let negative = plist(&[vec![0xAF, 0x13, 0xFF, 0, 0, 0, 0, 0, 0, 0]], 0, 1, 1);
    assert_eq!(error(&negative), BplistError::BadLength);

    // A length that is not an integer object.
    let not_int = plist(&[vec![0xAF, 0x51, b'a']], 0, 1, 1);
    assert_eq!(error(&not_int), BplistError::BadLength);

    // A top index outside the object table.
    let top = plist(&[array(&[])], 1, 1, 1);
    assert_eq!(error(&top), BplistError::BadTrailer);

    // Object count larger than the data.
    let mut count = plist(&[array(&[])], 0, 1, 1);
    let at = count.len() - 24;
    count[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(error(&count), BplistError::BadTrailer);

    // Offset table start past the trailer.
    let mut start = plist(&[array(&[])], 0, 1, 1);
    let at = start.len() - 8;
    start[at..].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(error(&start), BplistError::BadTrailer);

    // Too short, wrong header, other version.
    assert_eq!(error(b""), BplistError::TooShort);
    assert_eq!(error(&[0; 39]), BplistError::TooShort);
    assert_eq!(error(&[0; 40]), BplistError::NotBplist00);
    let mut other = plist(&[array(&[])], 0, 1, 1);
    other[7] = b'1';
    assert!(matches!(
        string_array(&other),
        Err(BplistError::NotBplist00)
    ));

    // A top object that is not an array.
    let not_array = plist(&[ascii("a")], 0, 1, 1);
    assert!(matches!(
        string_array(&not_array),
        Err(BplistError::NotAnArray)
    ));
}

#[test]
fn shared_objects_are_not_cycles() {
    let bytes = plist(&[array(&[1, 1, 2]), ascii("a"), array(&[1, 1])], 0, 1, 1);
    assert_eq!(string_array(&bytes).unwrap(), ["a", "a"]);
}

#[test]
fn nesting_deeper_than_eight_is_an_error() {
    // Array 0 holds array 1, which holds array 2, and so on.
    let depth = |levels: u8| {
        let mut objects: Vec<Vec<u8>> = (0..levels).map(|i| array(&[i + 1])).collect();
        objects.push(ascii("leaf"));
        plist(&objects, 0, 1, 1)
    };
    assert_eq!(string_array(&depth(8)).unwrap(), Vec::<String>::new());
    assert!(matches!(string_array(&depth(9)), Err(BplistError::TooDeep)));
}

#[test]
fn wide_shared_nesting_is_bounded() {
    // Eight levels, each array referring to the next one 200 times: a small file that
    // would expand to 200^7 visits without a budget.
    let mut objects: Vec<Vec<u8>> = (0..8u8).map(|i| array(&[i + 1; 200])).collect();
    objects.push(ascii("leaf"));
    let bytes = plist(&objects, 0, 1, 1);
    assert!(matches!(
        string_array(&bytes),
        Err(BplistError::TooManyObjects)
    ));
}

#[test]
fn bplist_item_cap() {
    let mut objects = vec![];
    let count = fileattrs::MAX_ITEMS;
    let mut top = marker(0xA, count);
    top.extend((0..count).flat_map(|_| 1u16.to_be_bytes()));
    objects.push(top);
    objects.push(ascii("a"));
    let at_cap = plist(&objects, 0, 2, 2);
    assert_eq!(string_array(&at_cap).unwrap().len(), count);

    let mut over = marker(0xA, count + 1);
    over.extend((0..=count).flat_map(|_| 1u16.to_be_bytes()));
    objects[0] = over;
    let over_cap = plist(&objects, 0, 2, 2);
    assert!(matches!(
        string_array(&over_cap),
        Err(BplistError::TooManyItems)
    ));
}

#[test]
fn bplist_text_cap() {
    let big = "x".repeat(fileattrs::MAX_TEXT_BYTES / 2 + 1);
    let bytes = plist(&[array(&[1, 1]), ascii(&big)], 0, 4, 1);
    assert!(matches!(
        string_array(&bytes),
        Err(BplistError::TooMuchText)
    ));
}

#[test]
fn skipped_invalid_text_counts_toward_the_text_cap() {
    // One string of 64 KiB whose last byte is not ASCII, referenced 1,024 times: each
    // reference is charged, so the reader stops at 1 MiB instead of scanning 64 MiB.
    let mut bad = "x".repeat(64 * 1024 - 1).into_bytes();
    bad.push(0xC3);
    let mut object = marker(0x5, bad.len());
    object.extend_from_slice(&bad);
    let mut top = marker(0xA, fileattrs::MAX_ITEMS);
    top.extend((0..fileattrs::MAX_ITEMS).flat_map(|_| 1u16.to_be_bytes()));
    let bytes = plist(&[top, object], 0, 4, 2);
    assert_eq!(error(&bytes), BplistError::TooMuchText);

    // Under the cap the invalid string is skipped as before.
    let bytes = plist(&[array(&[1, 1, 2]), vec![0x51, 0xC3], ascii("ok")], 0, 1, 1);
    assert_eq!(string_array(&bytes).unwrap(), ["ok"]);
}

#[test]
fn a_shared_string_is_returned_per_reference() {
    let bytes = plist(&[array(&[1, 2, 1]), utf16("Größe"), ascii("b")], 0, 1, 1);
    assert_eq!(string_array(&bytes).unwrap(), ["Größe", "b", "Größe"]);
}

#[test]
fn real_where_froms_from_plutil() {
    assert_eq!(
        string_array(&fixture("where_froms_plutil.bplist")).unwrap(),
        [
            "https://example.com/files/report.pdf",
            "https://example.com/"
        ]
    );
}

#[test]
fn real_tags_from_plutil() {
    assert_eq!(
        string_array(&fixture("tags_plutil.bplist")).unwrap(),
        ["Größe\n6", "Red\n6", "日本"]
    );
}

#[test]
fn real_tags_from_finder() {
    assert_eq!(
        string_array(&fixture("tags_finder.bplist")).unwrap(),
        ["Größe\n0", "Blue\n0"]
    );
}

#[test]
fn real_integers_are_skipped() {
    assert_eq!(
        string_array(&fixture("integers_plutil.bplist")).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn truncated_real_files_never_panic() {
    for name in [
        "where_froms_plutil.bplist",
        "tags_plutil.bplist",
        "tags_finder.bplist",
        "integers_plutil.bplist",
    ] {
        let bytes = fixture(name);
        for end in 0..bytes.len() {
            let _ = string_array(&bytes[..end]);
        }
        for at in 0..bytes.len() {
            for value in [0x00, 0x0F, 0x7F, 0xFF] {
                let mut changed = bytes.clone();
                changed[at] = value;
                let _ = string_array(&changed);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Tags

#[test]
fn finder_tags_drop_the_colour() {
    assert_eq!(
        finder_tags(&fixture("tags_plutil.bplist")),
        ["Größe", "Red", "日本"]
    );
    assert_eq!(
        finder_tags(&fixture("tags_finder.bplist")),
        ["Größe", "Blue"]
    );
}

#[test]
fn finder_tags_keep_names_without_a_colour_number() {
    let bytes = plist(
        &[
            array(&[1, 2, 3, 4, 5]),
            ascii("a\nb\n12"),
            ascii("two\nlines"),
            ascii("\n6"),
            ascii("Red\n6"),
            ascii("Red"),
        ],
        0,
        1,
        1,
    );
    assert_eq!(finder_tags(&bytes), ["a\nb", "two\nlines", "Red"]);
}

#[test]
fn finder_tags_of_broken_or_empty_values_are_empty() {
    assert!(finder_tags(b"").is_empty());
    assert!(finder_tags(&plist(&[array(&[0])], 0, 1, 1)).is_empty());
    assert!(finder_tags(&plist(&[array(&[])], 0, 1, 1)).is_empty());
}

#[test]
fn tags_stop_at_the_cap_quickly() {
    // A hostile 64 KiB value of distinct short names: only the first 64 are kept, and the
    // work stays linear.
    let names: Vec<String> = (0..20_000).map(|i| format!("{i:x}")).collect();
    let mut text = names.join(",");
    text.truncate(MAX_ATTR_BYTES);
    let started = std::time::Instant::now();
    for _ in 0..100 {
        assert_eq!(xdg_tags(&text), names[..MAX_TAGS]);
    }
    let repeats = "a,".repeat(MAX_ATTR_BYTES / 2);
    for _ in 0..100 {
        assert_eq!(xdg_tags(&repeats), ["a"]);
    }
    let spent = started.elapsed();
    assert!(spent < std::time::Duration::from_secs(2), "{spent:?}");
}

#[test]
fn xdg_tags_split_at_commas() {
    assert_eq!(
        xdg_tags("work, parent/child,,Größe ,work"),
        ["work", "parent/child", "Größe"]
    );
    assert!(xdg_tags("").is_empty());
    assert!(xdg_tags(" , ").is_empty());
}

// ---------------------------------------------------------------------------------------
// Zone.Identifier

#[test]
fn zone_host_url_utf8() {
    let text = "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://a.example/\r\nHostUrl=https://a.example/f.zip\r\n";
    assert_eq!(
        zone_host_url(text).as_deref(),
        Some("https://a.example/f.zip")
    );
}

#[test]
fn zone_host_url_utf16_with_bom() {
    let text = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://a.example/Größe.zip\r\n";
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let decoded = zone_text(&bytes).unwrap();
    assert_eq!(decoded, text);
    assert_eq!(
        zone_host_url(&decoded).as_deref(),
        Some("https://a.example/Größe.zip")
    );
}

#[test]
fn zone_text_drops_a_utf8_bom_and_replaces_bad_bytes() {
    assert_eq!(zone_text(b"\xEF\xBB\xBFab").as_deref(), Some("ab"));
    assert_eq!(zone_text(b"a\xFFb").as_deref(), Some("a\u{FFFD}b"));
}

#[test]
fn zone_keys_are_case_insensitive() {
    let text = "[zonetransfer]\nhosturl = https://a.example/x \n";
    assert_eq!(zone_host_url(text).as_deref(), Some("https://a.example/x"));
}

#[test]
fn zone_value_keeps_later_equals_signs() {
    let text = "[ZoneTransfer]\nHostUrl=https://a.example/?a=1&b=2\n";
    assert_eq!(
        zone_host_url(text).as_deref(),
        Some("https://a.example/?a=1&b=2")
    );
}

#[test]
fn zone_without_host_url_is_none() {
    assert_eq!(zone_host_url("[ZoneTransfer]\r\nZoneId=3\r\n"), None);
    assert_eq!(zone_host_url("HostUrl=https://a.example/\r\n"), None);
    assert_eq!(
        zone_host_url("[Other]\r\nHostUrl=https://a.example/\r\n[ZoneTransfer]\r\nZoneId=3\r\n"),
        None
    );
    assert_eq!(zone_host_url("[ZoneTransfer]\r\nHostUrl=\r\n"), None);
    assert_eq!(zone_host_url(""), None);
}

// ---------------------------------------------------------------------------------------
// Extended attributes

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod xattrs {
    use super::*;
    use fileattrs::read_xattr;
    use fileattrs::test_support::set_xattr as set;

    /// Errors that mean this file system cannot hold the attribute.
    fn unsupported(error: &std::io::Error) -> bool {
        matches!(
            error.raw_os_error(),
            Some(libc::ENOTSUP | libc::E2BIG | libc::ENOSPC | libc::EPERM)
        )
    }

    #[test]
    fn xattr_round_trip() {
        let file = tempfile::tempfile().unwrap();
        if let Err(error) = set(&file, "user.rezzer.test", b"hello") {
            if unsupported(&error) {
                eprintln!("skipped: no extended attributes here ({error})");
                return;
            }
            panic!("{error}");
        }
        let value = read_xattr(&file, "user.rezzer.test", MAX_ATTR_BYTES).unwrap();
        assert_eq!(value.as_deref(), Some(&b"hello"[..]));

        set(&file, "user.rezzer.empty", b"").unwrap();
        let empty = read_xattr(&file, "user.rezzer.empty", MAX_ATTR_BYTES).unwrap();
        assert_eq!(empty.as_deref(), Some(&b""[..]));
    }

    #[test]
    fn xattr_size_is_capped() {
        // Linux file systems cap a value at 64 KiB or less, so a refused write skips.
        let file = tempfile::tempfile().unwrap();
        let big = vec![b'x'; 10 * 1024 * 1024];
        if let Err(error) = set(&file, "user.rezzer.big", &big) {
            eprintln!("skipped: this file system refuses a 10 MB attribute ({error})");
            return;
        }
        assert_eq!(
            read_xattr(&file, "user.rezzer.big", MAX_ATTR_BYTES).unwrap(),
            None
        );
        assert_eq!(read_xattr(&file, "user.rezzer.big", 4).unwrap(), None);
    }

    #[test]
    fn small_xattr_over_max_is_none() {
        let file = tempfile::tempfile().unwrap();
        let value = [b'y'; 100];
        if let Err(error) = set(&file, "user.rezzer.small", &value) {
            if unsupported(&error) {
                eprintln!("skipped: no extended attributes here ({error})");
                return;
            }
            panic!("{error}");
        }
        assert_eq!(read_xattr(&file, "user.rezzer.small", 99).unwrap(), None);
        assert_eq!(read_xattr(&file, "user.rezzer.small", 0).unwrap(), None);
        let read = read_xattr(&file, "user.rezzer.small", 100).unwrap();
        assert_eq!(read.as_deref(), Some(&value[..]));
    }

    #[test]
    fn missing_xattr_is_none() {
        let file = tempfile::tempfile().unwrap();
        assert_eq!(
            read_xattr(&file, "user.rezzer.missing", MAX_ATTR_BYTES).unwrap(),
            None
        );
    }

    #[test]
    fn name_with_nul_is_an_error() {
        let file = tempfile::tempfile().unwrap();
        assert!(read_xattr(&file, "user.a\0b", MAX_ATTR_BYTES).is_err());
    }
}
