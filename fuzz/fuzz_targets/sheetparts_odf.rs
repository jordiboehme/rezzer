// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! sheetparts' ODF reader on arbitrary part XML. The input is cut at NUL bytes into
//! content.xml, styles.xml, meta.xml and the manifest of a small package, stored without
//! compression; an input that starts with `PK` is used as the package itself. It must never
//! panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sheetparts::Limits;

#[path = "common/package.rs"]
mod package;

const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";

fn defaults() -> Vec<(&'static str, String)> {
    vec![
        ("content.xml", format!("<office:document-content xmlns:office=\"{OFFICE}\" xmlns:table=\"{TABLE}\" xmlns:text=\"{TEXT}\"><office:body><office:spreadsheet><table:table table:name=\"S\"/></office:spreadsheet></office:body></office:document-content>")),
        ("styles.xml", format!("<office:document-styles xmlns:office=\"{OFFICE}\"/>")),
        ("meta.xml", format!("<office:document-meta xmlns:office=\"{OFFICE}\"/>")),
        ("META-INF/manifest.xml", "<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"/>".to_string()),
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet".to_string()),
    ]
}

fuzz_target!(|data: &[u8]| {
    let bytes = package::build(data, &defaults());
    let Ok(mut zip) = zipguard::Archive::open(&bytes, package::ZIP_LIMITS) else {
        return;
    };
    let limits = Limits {
        max_depth: 64,
        max_items: 100_000,
    };
    let _ = sheetparts::odf::read(&mut zip, &limits, &|| false);
});
