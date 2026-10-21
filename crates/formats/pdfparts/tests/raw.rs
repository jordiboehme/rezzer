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
        measure_stream(&packed, &[b"FlateDecode"], &limits(), u64::MAX, &never),
        Ok(300)
    );
    let bomb = common::zlib_fill(0, 100 << 20);
    assert!(measure_stream(&bomb, &[b"Fl"], &limits(), u64::MAX, &never).is_err());
}
