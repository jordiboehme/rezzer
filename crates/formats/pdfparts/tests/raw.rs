// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Measuring the streams of a file that does not open, for a parser that reads it as it is.

use pdfparts::{Limits, OpenError, RawStreams, measure_stream};

mod common;

fn never() -> bool {
    false
}

fn limits() -> Limits {
    Limits {
        max_stream_bytes: 64 << 20,
        max_total_bytes: 64 << 20,
        ..Limits::default()
    }
}

/// A file without a usable cross-reference table: objects only.
fn objects(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    for p in parts {
        out.extend(p);
        out.push(b'\n');
    }
    out
}

fn stream(n: u32, dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("{n} 0 obj\n").into_bytes();
    v.extend(common::stream_body(dict, data));
    v.extend(b"\nendobj");
    v
}

#[test]
fn every_stream_is_measured() {
    let packed = common::zlib(&[b'x'; 5000]);
    let file = objects(&[
        b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj".to_vec(),
        stream(4, "/Filter /FlateDecode", &packed),
        stream(5, "", b"plain"),
    ]);
    let raw = RawStreams::measure(&file, &limits(), &never).unwrap();
    assert_eq!(raw.size(4, 0), Some(5000));
    assert_eq!(raw.size(5, 0), Some(5));
    assert_eq!(raw.size(1, 0), None);
    assert_eq!(raw.unpacked(), 5005);
}

#[test]
fn a_bomb_in_an_earlier_copy_of_an_object_is_found() {
    // The second copy of object 4 is small; a parser that follows an old offset gets the
    // first one.
    let bomb = common::zlib_fill(0, 200 << 20);
    let file = objects(&[
        stream(4, "/Filter /FlateDecode", &bomb),
        stream(4, "", b"small"),
    ]);
    match RawStreams::measure(&file, &limits(), &never) {
        Err(OpenError::Limit(why)) => assert!(why.contains("stream 4 0"), "{why}"),
        other => panic!("{:?}", other.map(|r| r.unpacked())),
    }
}

#[test]
fn filters_named_indirectly_are_not_measured() {
    let file = objects(&[
        b"3 0 obj /FlateDecode endobj".to_vec(),
        stream(4, "/Filter 3 0 R", b"data"),
    ]);
    match RawStreams::measure(&file, &limits(), &never) {
        Err(OpenError::Limit(why)) => assert!(why.contains("indirectly"), "{why}"),
        other => panic!("{:?}", other.map(|r| r.unpacked())),
    }
}

#[test]
fn an_encrypted_file_is_not_measured() {
    let file = objects(&[
        b"1 0 obj << /Type /Catalog >> endobj".to_vec(),
        b"trailer << /Root 1 0 R /Encrypt 2 0 R >>".to_vec(),
    ]);
    assert!(matches!(
        RawStreams::measure(&file, &limits(), &never),
        Err(OpenError::UnsupportedEncryption(_))
    ));
}

#[test]
fn one_stream_is_measured_by_its_filter_names() {
    let packed = common::zlib(&[b'y'; 300]);
    assert_eq!(
        measure_stream(&packed, &[b"FlateDecode"], &[], &limits(), u64::MAX, &never),
        Ok(300)
    );
    let bomb = common::zlib_fill(0, 100 << 20);
    assert!(measure_stream(&bomb, &[b"Fl"], &[], &limits(), u64::MAX, &never).is_err());
}

/// A zlib stream that inflate refuses at its first copy (from 32,768 bytes back).
fn far_copy_first() -> Vec<u8> {
    // Header, then bits: final fixed block, length code 257, distance code 29, 13 extra bits,
    // end of block.
    vec![0x78, 0x9c, 0x03, 0xde, 0xff, 0x0f, 0x00]
}

#[test]
fn a_damaged_stream_is_marked_and_a_damaged_object_stream_refuses_the_file() {
    let file = objects(&[stream(4, "/Filter /FlateDecode", &far_copy_first())]);
    let raw = RawStreams::measure(&file, &limits(), &never).unwrap();
    assert!(raw.broken(4, 0));
    assert!(raw.any_broken());
    let file = objects(&[stream(
        4,
        "/Type /ObjStm /N 1 /First 4 /Filter /FlateDecode",
        &far_copy_first(),
    )]);
    match RawStreams::measure(&file, &limits(), &never) {
        Err(OpenError::Damaged(why)) => assert!(why.contains("decoding error"), "{why}"),
        other => panic!("{:?}", other.map(|r| r.unpacked())),
    }
}

#[test]
fn every_extent_a_parser_may_take_is_measured() {
    // The data holds `endstream` in a stored block; the real end is `endstreamX` after the
    // stated length, followed by data that unpacks past the caps.
    let mut data = vec![0x78, 0x9c, 0x00];
    let inner = b"\nendstream\n";
    data.extend((inner.len() as u16).to_le_bytes());
    data.extend((!(inner.len() as u16)).to_le_bytes());
    data.extend(inner);
    data.extend(common::zlib_fill(0, 200 << 20).iter().skip(2));
    let mut obj = format!(
        "4 0 obj\n<< /Filter /FlateDecode /Length {} >>\nstream\n",
        data.len()
    )
    .into_bytes();
    obj.extend(&data);
    obj.extend(b"\nendstreamX\nendobj");
    let file = objects(&[obj]);
    assert!(
        matches!(
            RawStreams::measure(&file, &limits(), &never),
            Err(OpenError::Limit(_))
        ),
        "the length-based extent is a bomb"
    );
    // An indirect length with two definitions: the first one, which a parser following an
    // old offset takes, covers the whole data.
    let mut obj = b"4 0 obj\n<< /Filter /FlateDecode /Length 9 0 R >>\nstream\n".to_vec();
    obj.extend(&data);
    obj.extend(b"\nendstreamX\nendobj");
    let file = objects(&[
        format!("9 0 obj {} endobj", data.len()).into_bytes(),
        obj,
        b"9 0 obj 5 endobj".to_vec(),
    ]);
    assert!(
        matches!(
            RawStreams::measure(&file, &limits(), &never),
            Err(OpenError::Limit(_))
        ),
        "every definition of the length is tried"
    );
}

#[test]
fn a_stream_is_measured_with_its_decode_parameters() {
    // EarlyChange 0 decodes LZW codes one code later; measured with it, the size is right.
    let names: [&[u8]; 1] = [b"LZWDecode"];
    let parms = [pdfparts::FilterParms {
        early_change: false,
    }];
    // Clear code, 'a', end of data, in 9-bit codes, MSB first: 256, 97, 257.
    let data = [0x80, 0x18, 0x60, 0x20, 0x00];
    assert_eq!(
        measure_stream(&data, &names, &parms, &limits(), u64::MAX, &never),
        Ok(1)
    );
}

#[test]
fn indirect_lengths_are_looked_up_once() {
    // 80,000 streams, each with its length in an object of its own.
    let mut out = b"%PDF-1.4\n".to_vec();
    for i in 0..80_000u32 {
        let (s, l) = (2 * i + 1, 2 * i + 2);
        out.extend(format!("{s} 0 obj\n<< /Length {l} 0 R >>\nstream\nab\nendstream\nendobj\n{l} 0 obj 2 endobj\n").as_bytes());
    }
    let start = std::time::Instant::now();
    let raw = RawStreams::measure(&out, &limits(), &never).unwrap();
    assert_eq!(raw.size(1, 0), Some(2));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_length_in_an_object_stream_keeps_the_fallback_out() {
    // Object 9 lives in an object stream: the scan finds no header for it.
    let file = objects(&[
        stream(5, "/Type /ObjStm /N 1 /First 4", b"9 0 2"),
        b"4 0 obj\n<< /Length 9 0 R >>\nstream\nab\nendstream\nendobj".to_vec(),
    ]);
    match RawStreams::measure(&file, &limits(), &never) {
        Err(OpenError::Limit(why)) => assert!(why.contains("object 9"), "{why}"),
        other => panic!("{:?}", other.map(|r| r.unpacked())),
    }
}
