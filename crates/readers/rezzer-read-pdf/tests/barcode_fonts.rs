// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Barcodes printed with a barcode font: the bars are glyphs of an embedded font, not a
//! picture and not drawn paths. Their glyph text is no text of the page; the code is read from
//! the bars or named as not decoded. The fonts are built here (see `common::barfont`); every
//! value is invented.

mod common;

use common::barfont::{
    Glyphs, PATTERNS, code_of, code128_string, code128_values, code128_width, page_with_fonts,
    program,
};
use common::{all_text, markdown, read};
use rezzer_model::ReadOutcome;

/// Every code listed in `out`: kind and value.
fn codes(out: &ReadOutcome) -> Vec<(String, String)> {
    out.document
        .assets
        .iter()
        .flat_map(|a| a.codes.iter())
        .map(|c| (c.symbology.clone(), c.text.clone().unwrap_or_default()))
        .collect()
}

/// The text of the glyphs of `text` as a Code 128 in the font: odd characters (`Í4: O+(Î`).
fn glyph_text(text: &str) -> String {
    code128_values(text)
        .into_iter()
        .map(|v| char::from(code_of(v)))
        .collect()
}

/// Asserts that no text of the glyphs of the codes `texts` stands in the read: not the
/// start and stop characters, and no three glyph characters in a row that are not all
/// digits (a decoded value may stand in the read).
fn assert_no_glyph_text(out: &ReadOutcome, texts: &[&str], case: &str) {
    let md = markdown(out);
    let all = all_text(out);
    for t in texts {
        let g: Vec<char> = glyph_text(t).chars().collect();
        for w in g.windows(3) {
            if w.iter().all(char::is_ascii_digit) {
                continue;
            }
            let s: String = w.iter().collect();
            assert!(!all.contains(&s), "{case}: glyph text {s:?} in {all:?}");
        }
    }
    for c in ['Ì', 'Í', 'Î'] {
        assert!(!md.contains(c), "{case}: {c} in {md}");
    }
}

/// The info box of an online-shop invoice (right half of an A4 page): rows of labels and
/// values, a Code 128 of `first` in the font `/F3` at `size` under the invoice number row
/// whose baseline is the order number row's, between the labels and the values, and a Code
/// 128 of `second` beside `EAN:` whose baseline is the bold `Referenz:` row's. The values
/// start `gap` points after the first code.
fn invoice(font: &str, size: f32, gap: f32) -> Vec<u8> {
    let (first, second) = ("2026004711", "4006381333931");
    let w1 = code128_width(first, size);
    let w2 = code128_width(second, size);
    let content = format!(
        "BT /F2 14 Tf 72 790 Td (Rechnung) Tj ET\n\
         BT /F1 9 Tf 72 760 Td (Muster Versand GmbH) Tj ET\n\
         BT /F1 9 Tf 72 748 Td (Beispielweg 1, 12345 Musterstadt) Tj ET\n\
         BT /F1 9 Tf 300 760 Td (Rechnungs-Nr.) Tj ET\n\
         BT /F1 9 Tf {x2:.2} 760 Td ({first}) Tj ET\n\
         BT /F3 {size} Tf 380 735 Td ({c1}) Tj ET\n\
         BT /F1 9 Tf 300 735 Td (Auftrags-Nr.) Tj ET\n\
         BT /F1 9 Tf {x2:.2} 735 Td (0000000000000 / 01.01.2026) Tj ET\n\
         BT /F1 9 Tf 300 715 Td (Kunden-Nr.) Tj ET\n\
         BT /F1 9 Tf {x2:.2} 715 Td (K-000123) Tj ET\n\
         BT /F2 9 Tf 300 680 Td (Referenz:) Tj ET\n\
         BT /F2 9 Tf 345 680 Td (RE-0001) Tj ET\n\
         BT /F1 9 Tf {xe:.2} 686 Td (EAN:) Tj ET\n\
         BT /F3 {size} Tf {x3:.2} 680 Td ({c2}) Tj ET\n\
         BT /F1 10 Tf 72 600 Td (Artikel) Tj 200 0 Td (Menge) Tj 100 0 Td (Preis) Tj ET\n\
         BT /F1 10 Tf 72 585 Td (Stuhl) Tj 200 0 Td (2) Tj 100 0 Td (40,00 EUR) Tj ET\n",
        c1 = code128_string(first),
        c2 = code128_string(second),
        x2 = 380.0 + w1 + gap,
        x3 = 585.0 - w2,
        xe = 561.0 - w2,
    );
    page_with_fonts(&content, &[("F3", font, &program(None))])
}

#[test]
fn codes_printed_in_a_barcode_font_are_listed_and_their_glyphs_are_no_text() {
    // Today's read glued the glyph text into the order number row and after the bold
    // reference, and listed no code. The font is known by its name (`Code128`) or, with a
    // name that says nothing, by its glyphs: upright bars of whole modules.
    for font in ["Code128", "ShopBars"] {
        for size in [10.0, 12.0, 16.0] {
            for gap in [5.0, 12.0] {
                let case = format!("{font} at {size}, gap {gap}");
                let out = read(&invoice(font, size, gap));
                let md = markdown(&out);
                assert_no_glyph_text(&out, &["2026004711", "4006381333931"], &case);
                assert!(
                    md.contains("| Auftrags-Nr. | 0000000000000 / 01.01.2026 |"),
                    "{case}: {md}"
                );
                assert!(md.contains("RE-0001**"), "{case}: {md}");
                let mut found = codes(&out);
                found.sort();
                assert_eq!(
                    found,
                    [
                        ("Code 128".to_string(), "2026004711".to_string()),
                        ("Code 128".to_string(), "4006381333931".to_string()),
                    ],
                    "{case}: {md}"
                );
                assert!(!md.contains("not decoded"), "{case}: {md}");
            }
        }
    }
}

/// A one-page file that shows `shown` (a PDF string body) in a font named `font` with the
/// glyphs `glyphs` (see [`program`]) at 12 points, at (100, 600), and a line of Helvetica.
fn shown_in(font: &str, glyphs: Option<Glyphs<'_>>, shown: &str) -> Vec<u8> {
    let content = format!(
        "BT /F1 10 Tf 72 700 Td (Lieferschein 0000) Tj ET\n\
         BT /F3 12 Tf 100 600 Td ({shown}) Tj ET\n"
    );
    page_with_fonts(&content, &[("F3", font, &program(glyphs))])
}

/// The glyph text of the Code 128 of `text` with a wrong check symbol: bars of a code that
/// no decoder reads.
fn wrong_check(text: &str) -> String {
    let mut v = code128_values(text);
    let n = v.len();
    v[n - 2] = (v[n - 2] + 1) % 103;
    v.into_iter()
        .map(|v| match code_of(v) {
            c @ (b'(' | b')' | b'\\') => format!("\\{}", char::from(c)),
            c @ 32..=126 => char::from(c).to_string(),
            c => format!("\\{c:03o}"),
        })
        .collect()
}

#[test]
fn a_code_in_a_barcode_font_that_is_not_decoded_is_named() {
    // Bars of a Code 128 whose check symbol is wrong: no value, and the read says what is
    // there (P6-X: never a wrong value, never silent).
    for font in ["Code128", "ShopBars"] {
        let out = read(&shown_in(font, None, &wrong_check("2026004711")));
        let md = markdown(&out);
        assert!(codes(&out).is_empty(), "{font}: {:?}", codes(&out));
        assert!(
            md.contains("a barcode printed in a barcode font, box 100, 600 to "),
            "{font}: {md}"
        );
        assert!(md.contains(", not decoded"), "{font}: {md}");
        let a = &out.document.assets[0];
        assert!(a.code_unread && a.needs_vision, "{font}: {a:?}");
        for c in ['Ì', 'Í', 'Î'] {
            assert!(!md.contains(c), "{font}: {md}");
        }
        assert!(all_text(&out).contains("Lieferschein 0000"), "{font}: {md}");
    }
}

#[test]
fn a_font_named_for_codes_that_draws_no_bars_gives_no_code() {
    // A font named `Barcode` whose glyphs are squares: its name is the evidence, so its text
    // is no text; its squares give no code, and the read says so.
    let squares = |v: u8| {
        let x = i16::from(v % 4) * 100;
        vec![(x, 0, x + 300, 300), (x + 400, 400, x + 600, 600)]
    };
    let out = read(&shown_in("Barcode", Some(&squares), "ABCDEF"));
    let md = markdown(&out);
    assert!(codes(&out).is_empty(), "{:?}", codes(&out));
    assert!(
        md.contains("a barcode printed in a barcode font") && md.contains("not decoded"),
        "{md}"
    );
    assert!(!all_text(&out).contains("ABCDEF"), "{md}");
}

#[test]
fn odd_characters_in_a_text_font_stay_text() {
    // The characters a barcode font shows, in Helvetica: no evidence of a code.
    let content = "BT /F1 10 Tf 72 700 Td (Lieferschein 0000) Tj ET\n\
                   BT /F1 12 Tf 100 600 Td (%\"$\"*%!\\)-%) Tj ET\n";
    let out = read(&page_with_fonts(content, &[]));
    assert!(
        all_text(&out).contains("%\"$\"*%!)-%"),
        "{}",
        all_text(&out)
    );
    assert!(out.document.assets.is_empty(), "{:?}", out.document.assets);
}

#[test]
fn text_fonts_whose_glyphs_look_like_bars_stay_text() {
    // Glyphs of one bar each (`l`, `I`, `|`): never a code.
    let one = |v: u8| {
        let w = 60 + i16::from(v % 3) * 60;
        vec![(100, 0, 100 + w, 700)]
    };
    // Glyphs of two bars whose widths are no whole modules (a double bar sign).
    let two = |v: u8| {
        let w = 60 + i16::from(v % 2) * 90;
        vec![(0, 0, 60, 900), (60 + w, 0, 120 + w + 30, 900)]
    };
    // Glyphs of two bars of different heights.
    let uneven = |v: u8| {
        let x = i16::from(v % 3) * 60;
        vec![(0, 0, 60, 900), (120 + x, 0, 180 + x, 700)]
    };
    let fonts: [(&str, Glyphs<'_>); 3] = [
        ("one bar", &one),
        ("two odd bars", &two),
        ("uneven bars", &uneven),
    ];
    for (name, glyphs) in fonts {
        let out = read(&shown_in("ShopSans", Some(glyphs), "abcdefgh"));
        assert!(
            all_text(&out).contains("abcdefgh"),
            "{name}: {}",
            all_text(&out)
        );
        assert!(
            out.document.assets.is_empty(),
            "{name}: {:?}",
            out.document.assets
        );
    }
}

#[test]
fn a_code_in_a_barcode_font_turned_a_quarter_is_read() {
    // A Code 128 printed up the right edge of a page, in a font known by name and in one
    // known by its glyphs.
    for font in ["Code 128", "ShopBars"] {
        for (turn, at) in [("0 1 -1 0", "560 300"), ("0 -1 1 0", "540 600")] {
            let content = format!(
                "BT /F1 10 Tf 72 700 Td (Lieferschein 0000) Tj ET\n\
                 BT /F3 12 Tf {turn} {at} Tm ({}) Tj ET\n",
                code128_string("2026004711")
            );
            let font = font.replace(' ', "#20");
            let out = read(&page_with_fonts(&content, &[("F3", &font, &program(None))]));
            assert_eq!(
                codes(&out),
                [("Code 128".to_string(), "2026004711".to_string())],
                "{font} {turn}: {}",
                markdown(&out)
            );
            assert_no_glyph_text(&out, &["2026004711"], &font);
        }
    }
}

#[test]
fn the_test_font_draws_code_128() {
    // The font's patterns are those of the barcodes crate's own encoder.
    let m = common::modules(barcodes::Symbology::Code128, "A1");
    let mut want = Vec::new();
    for v in code128_values("A1") {
        for (k, w) in PATTERNS[v as usize].bytes().enumerate() {
            want.extend(std::iter::repeat_n(k % 2 == 0, usize::from(w - b'0')));
        }
    }
    let dark: Vec<bool> = m.dark.clone();
    let at = dark.iter().position(|&d| d).unwrap_or(0);
    assert_eq!(&dark[at..at + want.len()], want.as_slice());
}

#[test]
fn codes_in_a_barcode_font_on_a_page_that_needs_vision_are_read_or_named() {
    // A photo makes the page need vision as a whole: the code's region is read from the page
    // render, then finer. A code read is listed with the page; one not decoded is named.
    use common::barfont::page_with_photo;
    let photo = "q 300 0 0 250 150 300 cm /Photo Do Q\n";
    for font in ["Code128", "ShopBars"] {
        let good = format!(
            "{photo}BT /F1 10 Tf 72 780 Td (Lieferschein 0000) Tj ET\n\
             BT /F3 12 Tf 100 650 Td ({}) Tj ET\n",
            code128_string("2026004711")
        );
        let out = read(&page_with_photo(&good, &[("F3", font, &program(None))]));
        assert_eq!(
            codes(&out),
            [("Code 128".to_string(), "2026004711".to_string())],
            "{font}: {}",
            markdown(&out)
        );
        assert_no_glyph_text(&out, &["2026004711"], font);
        let bad = format!(
            "{photo}BT /F1 10 Tf 72 780 Td (Lieferschein 0000) Tj ET\n\
             BT /F3 12 Tf 100 650 Td ({}) Tj ET\n",
            wrong_check("2026004711")
        );
        let out = read(&page_with_photo(&bad, &[("F3", font, &program(None))]));
        let md = markdown(&out);
        assert!(codes(&out).is_empty(), "{font}: {md}");
        assert!(
            md.contains("a barcode printed in a barcode font, top left of the page, not decoded"),
            "{font}: {md}"
        );
    }
}
