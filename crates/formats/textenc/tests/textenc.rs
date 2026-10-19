// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use textenc::{LineEnding, decode};

#[test]
fn utf8_without_bom() {
    let d = decode("Grüße\n".as_bytes());
    assert_eq!(d.text, "Grüße\n");
    assert_eq!(d.encoding, "UTF-8");
    assert!(!d.had_bom);
    assert!(d.confident);
    assert_eq!(d.line_ending, LineEnding::Lf);
}

#[test]
fn utf8_bom_is_removed() {
    let d = decode(b"\xEF\xBB\xBFhi");
    assert_eq!(d.text, "hi");
    assert_eq!(d.encoding, "UTF-8");
    assert!(d.had_bom);
    assert_eq!(d.line_ending, LineEnding::None);
}

#[test]
fn utf16le_with_bom() {
    let d = decode(&[0xFF, 0xFE, 0x41, 0x00, 0x0A, 0x00]);
    assert_eq!(d.text, "A\n");
    assert_eq!(d.encoding, "UTF-16LE");
    assert!(d.had_bom);
    assert!(d.confident);
}

#[test]
fn utf16be_with_bom() {
    let d = decode(&[0xFE, 0xFF, 0x00, 0x41]);
    assert_eq!(d.text, "A");
    assert_eq!(d.encoding, "UTF-16BE");
    assert!(d.had_bom);
}

#[test]
fn cp1252_curly_quotes() {
    let d = decode(&[0x93, 0x48, 0x69, 0x94]);
    assert_eq!(d.text, "\u{201C}Hi\u{201D}");
    assert_eq!(d.encoding, "windows-1252");
    assert!(!d.had_bom);
}

#[test]
fn latin1_with_few_high_bytes_decodes() {
    let mut bytes = b"The quick brown fox jumps over the lazy dog. ".repeat(7);
    bytes.truncate(300);
    bytes.push(0xE4);
    let d = decode(&bytes);
    assert!(d.text.contains('ä'));
    assert_eq!(d.encoding, "windows-1252");
}

#[test]
fn shift_jis_japanese() {
    let d = decode(&[0x82, 0xB1, 0x82, 0xF1, 0x82, 0xC9, 0x82, 0xBF, 0x82, 0xCD]);
    assert_eq!(d.text, "こんにちは");
    assert_eq!(d.encoding, "Shift_JIS");
}

#[test]
fn crlf_detected() {
    assert_eq!(decode(b"a\r\nb\r\n").line_ending, LineEnding::CrLf);
}

#[test]
fn cr_and_mixed_and_none_detected() {
    assert_eq!(decode(b"a\rb\r").line_ending, LineEnding::Cr);
    assert_eq!(decode(b"a\r\nb\nc").line_ending, LineEnding::Mixed);
    assert_eq!(decode(b"abc").line_ending, LineEnding::None);
}

#[test]
fn broken_utf16_is_not_confident() {
    // BOM, then a lone high surrogate.
    let d = decode(&[0xFF, 0xFE, 0x00, 0xD8]);
    assert!(!d.confident);
    assert!(d.text.contains('\u{FFFD}'));
}

#[test]
fn empty_input() {
    let d = decode(b"");
    assert_eq!(d.text, "");
    assert_eq!(d.encoding, "UTF-8");
    assert!(d.confident);
}

#[test]
fn non_ascii_only_after_the_first_mib_still_decodes() {
    let mut bytes = b"plain ascii line\n".repeat(70_000);
    assert!(bytes.len() > 1024 * 1024);
    let sjis = [0x82, 0xB1, 0x82, 0xF1, 0x82, 0xC9, 0x82, 0xBF, 0x82, 0xCD];
    for _ in 0..50 {
        bytes.extend_from_slice(&sjis);
        bytes.push(b'\n');
    }
    let d = decode(&bytes);
    assert_eq!(d.encoding, "Shift_JIS");
    assert!(d.confident);
    assert!(d.text.ends_with("こんにちは\n"));
}

#[test]
fn iso_2022_jp() {
    let d = decode(b"\x1B$B$3$s$K$A$O\x1B(B\n");
    assert_eq!(d.encoding, "ISO-2022-JP");
    assert_eq!(d.text, "こんにちは\n");
}
