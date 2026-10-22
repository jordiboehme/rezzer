// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Files with incremental updates: a base written by lopdf and update sections written as
//! bytes.

use std::collections::BTreeMap;

use lopdf::xref::XrefType;
use lopdf::{Document, Object, Stream, dictionary};

pub fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// The offset after the last `%%EOF` and its line end.
pub fn after_last_eof(bytes: &[u8]) -> usize {
    let at = bytes.windows(5).rposition(|w| w == b"%%EOF").unwrap() + 5;
    match &bytes[at..] {
        [b'\r', b'\n', ..] => at + 2,
        [b'\r' | b'\n', ..] => at + 1,
        _ => at,
    }
}

/// The offset the last `startxref` names.
pub fn last_startxref(bytes: &[u8]) -> usize {
    let at = bytes.windows(9).rposition(|w| w == b"startxref").unwrap() + 9;
    let digits: String = bytes[at..]
        .iter()
        .skip_while(|c| c.is_ascii_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .map(|c| char::from(*c))
        .collect();
    digits.parse().unwrap()
}

/// A one-page document written by lopdf: catalog 1, pages 2, page 3, content 4, info 5.
pub fn base(content: &[u8]) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    doc.objects.insert(
        (1, 0),
        Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => (2, 0) }),
    );
    doc.objects.insert(
        (2, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference((3, 0))], "Count" => 1,
        }),
    );
    doc.objects.insert(
        (3, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => (2, 0),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => (4, 0),
        }),
    );
    doc.objects.insert(
        (4, 0),
        Object::Stream(Stream::new(dictionary! {}, content.to_vec())),
    );
    doc.objects.insert(
        (5, 0),
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Base"),
            "ModDate" => Object::string_literal("D:20260101120000Z"),
        }),
    );
    doc.max_id = 5;
    doc.trailer.set("Root", (1, 0));
    doc.trailer.set("Info", (5, 0));
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out
}

/// Appends an update section with `objects` (number, body) and a classic table.
pub fn update(file: &mut Vec<u8>, objects: &[(u32, Vec<u8>)], size: u32) {
    update_with(
        file,
        objects,
        &format!("/Size {size} /Root 1 0 R /Info 5 0 R"),
    );
}

/// The same with the trailer keys `keys` (and `/Prev`).
pub fn update_with(file: &mut Vec<u8>, objects: &[(u32, Vec<u8>)], keys: &str) {
    let prev = last_startxref(file);
    if !file.ends_with(b"\n") {
        file.push(b'\n');
    }
    let mut offs = BTreeMap::new();
    for (n, body) in objects {
        offs.insert(*n, file.len());
        file.extend(format!("{n} 0 obj\n").as_bytes());
        file.extend(body);
        file.extend(b"\nendobj\n");
    }
    let xref = file.len();
    file.extend(b"xref\n");
    for (n, off) in &offs {
        file.extend(format!("{n} 1\n{off:010} 00000 n \n").as_bytes());
    }
    file.extend(
        format!("trailer\n<< {keys} /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
}

pub fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend(data);
    v.extend(b"\nendstream");
    v
}

pub fn info(date: &str) -> Vec<u8> {
    format!("<< /Title (Base) /ModDate (D:{date}) >>").into_bytes()
}
