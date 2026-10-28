// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Marks between boxed digits at every size, comb fields and labels half a point smaller.

use super::*;

/// A page of `content` with Helvetica as `/F1`, Helvetica-Bold as `/F2` and Courier as `/F3`.
pub(crate) fn three_font_page(content: &str) -> Vec<u8> {
    let mut b = common::Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = |b: &mut common::Builder, name: &str| {
        b.add(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>"
        ))
    };
    let (f1, f2, f3) = (
        font(&mut b, "Helvetica"),
        font(&mut b, "Helvetica-Bold"),
        font(&mut b, "Courier"),
    );
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /Resources << /Font << \
         /F1 {f1} 0 R /F2 {f2} 0 R /F3 {f3} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

pub(crate) fn shown(font: &str, size: f32, x: f32, y: f32, text: &str) -> String {
    format!("BT /{font} {size} Tf {x:.3} {y:.3} Td ({text}) Tj ET\n")
}

/// Digits of `size` in Courier, each in a stroked box of its own, with a filled mark 0.24
/// wide and `mark` long on the baseline after each digit of `after`.
pub(crate) fn boxed_digits(
    x: f32,
    y: f32,
    digits: &str,
    size: f32,
    mark: f32,
    after: &[usize],
) -> String {
    let pitch = 0.6 * size;
    let mut out = String::new();
    for (k, d) in digits.chars().enumerate() {
        let bx = x + k as f32 * pitch;
        out += &format!(
            "0 0 0 RG 0.4 w {bx:.3} {:.3} {pitch:.3} {:.3} re S\n",
            y - 0.3 * size,
            1.4 * size
        );
        out += &shown("F3", size, bx, y, &d.to_string());
        if after.contains(&k) {
            out += &format!(
                "0 0 0 rg {:.3} {:.3} 0.24 {mark:.3} re f\n",
                bx + pitch - 0.12,
                y - 0.05 * size
            );
        }
    }
    out
}

/// An amount and a date in boxed digits of `size`, their marks `ratio` of the size long, and
/// amounts and dates written as text that set the separators.
pub(crate) fn marked_form(size: f32, ratio: f32) -> Vec<u8> {
    let mut c = shown("F2", 12.0, 60.0, 760.0, "Probebeleg");
    c += &shown("F1", 0.7 * size, 60.0, 720.0, "Betrag");
    c += &boxed_digits(60.0, 700.0, "123456", size, ratio * size, &[3]);
    c += &shown("F1", 0.7 * size, 300.0, 720.0, "Datum");
    c += &boxed_digits(300.0, 700.0, "01022026", size, ratio * size, &[1, 3]);
    c += &shown(
        "F1",
        9.0,
        60.0,
        580.0,
        "Vormonat 1.111,22 vom 01.01.2026 und 2.222,33 vom 02.01.2026",
    );
    three_font_page(&c)
}

/// Each of `numbers` (printed digits, read value) reads with its separators, or its digits
/// are named in a loss: a number never stands without its drawn separators and unnamed.
pub(crate) fn read_or_named(out: &ReadOutcome, numbers: &[(&str, &str)]) -> Vec<bool> {
    let text = all_text(out);
    numbers
        .iter()
        .map(|(digits, value)| {
            let named = out
                .document
                .losses
                .iter()
                .any(|l| l.what.contains(&format!("the number {digits} ")));
            assert!(
                text.contains(value) || named,
                "{digits}: neither {value} nor a loss in {text} {:?}",
                out.document.losses
            );
            text.contains(value)
        })
        .collect()
}

/// Drawn marks of 0.22 and 0.30 of the digit size are read as separators at every size from 6
/// to 14 points. Marks of 0.36 of the size, longer than a separator of their digits may be,
/// are named in a loss with their number, never dropped without a word.
#[test]
fn marks_between_digits_are_read_or_named_at_every_size() {
    let numbers = [("123456", "1234,56"), ("01022026", "01.02.2026")];
    for size in [6.0, 8.0, 10.0, 12.0, 14.0] {
        for ratio in [0.22, 0.30] {
            let out = read(&marked_form(size, ratio));
            assert_eq!(
                read_or_named(&out, &numbers),
                [true, true],
                "{size} {ratio}: {:?}",
                out.document.losses
            );
        }
        let out = read(&marked_form(size, 0.36));
        read_or_named(&out, &numbers);
    }
}

/// A comb field: 9.5 point digits in a row of cells whose dividers are filled bars `width`
/// wide and 3 points high, rising from the cells' bottom rule.
pub(crate) fn comb_field(width: f32) -> Vec<u8> {
    let size = 9.5;
    let pitch = 0.6 * size * 1.3;
    let (x, y) = (60.0, 700.0);
    let digits = "12345678";
    let mut c = shown("F2", 12.0, 60.0, 760.0, "Kontoblatt");
    c += &shown("F1", 0.7 * size, 60.0, 720.0, "Kundennummer");
    c += &format!(
        "0 0 0 RG 0.4 w {x:.3} {:.3} m {:.3} {:.3} l S\n",
        y - 0.3 * size,
        x + pitch * digits.len() as f32,
        y - 0.3 * size
    );
    for (k, d) in digits.chars().enumerate() {
        let at = x + k as f32 * pitch;
        c += &shown(
            "F3",
            size,
            at + (pitch - 0.6 * size) / 2.0,
            y,
            &d.to_string(),
        );
        if k > 0 {
            c += &format!(
                "0 0 0 rg {:.3} {:.3} {width:.3} 3 re f\n",
                at - width / 2.0,
                y - 0.3 * size
            );
        }
    }
    c += &shown(
        "F1",
        9.0,
        60.0,
        580.0,
        "Vormonat 1.111,22 vom 01.01.2026 und 2.222,33 vom 02.01.2026",
    );
    three_font_page(&c)
}

/// The dividers of a comb field drawn as bars 0.8 or 1 point wide are thicker than a mark of
/// 9.5 point digits: they are no separators, and no loss names one.
#[test]
fn thick_comb_dividers_are_no_marks() {
    for width in [0.8, 1.0] {
        let out = read(&comb_field(width));
        let marks: Vec<&str> = out
            .document
            .losses
            .iter()
            .map(|l| l.what.as_str())
            .filter(|w| w.contains("drawn as a mark"))
            .collect();
        assert!(marks.is_empty(), "{width}: {marks:?}");
        let text: String = all_text(&out).split_whitespace().collect();
        assert!(text.contains("12345678"), "{width}: {text}");
    }
}

/// Labels half a point smaller than their values (8 over 8.5, 9 over 9.5, 10 over 10.5) in a
/// row of fields between rules are labels: each pairs with its value.
#[test]
fn labels_half_a_point_smaller_pair() {
    for size in [8.0, 9.0, 10.0] {
        let value = size + 0.5;
        let mut c = String::from("0 0 0 RG 0.5 w\n");
        let (x0, x1, top) = (60.0, 420.0, 760.0);
        let bottom = top - 2.6 * value;
        for y in [top, bottom] {
            c += &format!("{x0} {y} m {x1} {y} l S\n");
        }
        for x in [x0, 180.0, 300.0, x1] {
            c += &format!("{x} {bottom} m {x} {top} l S\n");
        }
        for (k, (l, v)) in [("Name", "Muster"), ("Ort", "Musterstadt"), ("Nr", "4711")]
            .iter()
            .enumerate()
        {
            let x = 64.0 + 120.0 * k as f32;
            c += &shown("F1", size, x, top - 1.1 * size, l);
            c += &shown("F1", value, x, bottom + 0.4 * value, v);
        }
        let out = read(&three_font_page(&c));
        let rows: Vec<String> = read_tables(&out)
            .iter()
            .flatten()
            .map(|r| r.join(": "))
            .collect();
        for want in ["Name: Muster", "Ort: Musterstadt", "Nr: 4711"] {
            assert!(rows.iter().any(|r| r == want), "{size}: {want} in {rows:?}");
        }
    }
}
