// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Barcodes printed with a barcode font: the bars are glyphs of an embedded font, not a
//! picture and not drawn paths. Their glyph text is no text of the page; the code is read from
//! the bars or named as not decoded. The fonts are built here (see `common::barfont`); every
//! value is invented.

mod common;

use common::barfont::{
    BAR_UNITS, Glyphs, Kind, PATTERNS, Set, bar_codes, bar_ink_width, bar_width, code_of,
    code128_string, code128_values, code128_width, page_with_bar_fonts, page_with_fonts,
    pattern_rects, pdf_string, program,
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
                // Each code is a small picture of its own, named by the box of its bars (a
                // code of a small picture has its place in the picture's words, not a box of
                // its own, as a code drawn as paths has).
                let high = f32::from(BAR_UNITS) / 1000.0 * size;
                let (w1, w2) = (
                    code128_width("2026004711", size),
                    code128_width("4006381333931", size),
                );
                for (text, bars) in [
                    ("2026004711", [380.0, 735.0, 380.0 + w1, 735.0 + high]),
                    ("4006381333931", [585.0 - w2, 680.0, 585.0, 680.0 + high]),
                ] {
                    let words = format!(
                        "Code 128, box {}, {} to {}, {}",
                        bars[0].round(),
                        bars[1].round(),
                        bars[2].round(),
                        bars[3].round()
                    );
                    assert!(md.contains(&words), "{case}: {text}: {words} in {md}");
                }
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
fn fonts_whose_names_say_codes_but_draw_letters_keep_their_text() {
    // Ruling of part 2f-1 fix round 2: a name never takes a run on its own. OCR-B and MICR
    // fonts of a barcode vendor, and names with a code inside a word or as a word, that draw
    // letters (blocks here): the coding line of a payment slip stays text, and no code is
    // claimed. At 1cb34b0 the names took the runs and the values were gone.
    let letter = |v: u8| {
        if v == 0 {
            vec![]
        } else {
            vec![(50, 0, 550, 700)]
        }
    };
    for name in [
        "IDAutomationOCRb",
        "IDAutomationMICR",
        "Ocean13",
        "Bean#2013",
        "BarcodeSans",
        "Mono-C128",
        "Code128",
        "BC#20C128#20HD#20Wide",
        "Barcode",
    ] {
        let content = "BT /F1 10 Tf 72 760 Td (Zahlung an:) Tj ET\n\
                       BT /F3 10 Tf 150 760 Td (Konto 1234567890 Betrag 12,50 EUR) Tj ET\n";
        let out = read(&page_with_fonts(
            content,
            &[("F3", name, &program(Some(&letter)))],
        ));
        let md = markdown(&out);
        assert!(
            all_text(&out).contains("Konto 1234567890 Betrag 12,50 EUR"),
            "{name}: {md}"
        );
        assert!(out.document.assets.is_empty(), "{name}: {md}");
        assert!(!md.contains("barcode font"), "{name}: {md}");
    }
}

#[test]
fn a_human_readable_bar_font_is_taken_by_its_bars() {
    // Bars with digits under them in the same glyph (blocks under the bars here), as Libre
    // Barcode 128 Text draws them: the bar part is the evidence, named or not.
    let readable = |v: u8| {
        let (bars, _) = pattern_rects(v);
        let mut r: Vec<(i16, i16, i16, i16)> = bars
            .iter()
            .map(|&(x0, _, x1, y1)| (x0, 300, x1, y1))
            .collect();
        r.push((60, 0, 300, 200));
        r
    };
    for name in [
        "Libre#20Barcode#20128#20Text",
        "LibreBarcode128Text-Regular",
    ] {
        let content = format!(
            "BT /F1 10 Tf 72 760 Td (Lieferschein 0000) Tj ET\n\
             BT /F3 12 Tf 100 600 Td ({}) Tj ET\n",
            code128_string("2026004711")
        );
        let out = read(&page_with_fonts(
            &content,
            &[("F3", name, &program(Some(&readable)))],
        ));
        assert_eq!(
            codes(&out),
            [("Code 128".to_string(), "2026004711".to_string())],
            "{name}: {}",
            markdown(&out)
        );
        assert_no_glyph_text(&out, &["2026004711"], name);
    }
}

#[test]
fn codes_of_two_widths_in_fonts_known_only_by_their_bars_are_read() {
    // Code 39 and ITF have two widths. In a font whose name says nothing they leaked their
    // glyph text, glued to the value beside them, with no code and no word (part 2f-1 review
    // I1); now their structure is the evidence. Equal bars stay text.
    use barcodes::Symbology as S;
    for (s, text, kind) in [
        (S::Code39, "ABC-1234", "Code 39"),
        (S::Code39, "R7", "Code 39"),
        (S::Itf, "12345670", "ITF"),
        (S::Itf, "0042", "ITF"),
    ] {
        let w = elements(&common::modules(s, text).dark);
        let glyphs = pair_glyphs(&w);
        for k in [Kind::TrueTypeSubset, Kind::Type1, Kind::Cff] {
            for size in [18.0, 30.0, 42.0] {
                let content = format!(
                    "BT /F1 9 Tf 300 640 Td (Artikel-Nr.) Tj ET\n\
                     BT /F1 9 Tf 360 640 Td (A-0042) Tj ET\n\
                     BT /F3 {size} Tf 300 600 Td ({}) Tj ET\n\
                     BT /F1 9 Tf 72 600 Td (Menge 2) Tj ET\n",
                    pdf_string(&glyphs)
                );
                let out = read(&page_with_bar_fonts(
                    &content,
                    &[("F3", "ShopBars", Set::Pairs, k)],
                ));
                let case = format!("{kind} {text} {k:?} {size}");
                let all = all_text(&out);
                let g = String::from_utf8_lossy(&glyphs).to_string();
                assert!(!all.contains(&g[..6]), "{case}: {all}");
                assert!(
                    all.contains("Menge 2") && all.contains("A-0042"),
                    "{case}: {all}"
                );
                let found = codes(&out);
                let md = markdown(&out);
                assert!(
                    found == [(kind.to_string(), text.to_string())]
                        || (found.is_empty()
                            && md.contains("barcode font")
                            && md.contains("not decoded")),
                    "{case}: {found:?} {md}"
                );
            }
        }
    }
}

#[test]
fn a_code_shown_a_glyph_at_a_time_is_one_code() {
    // A font known only by its bars, one glyph a show (`(A) Tj (B) Tj ...`): the shows that
    // continue one another on one baseline are looked at as one run (round 1 concern 3).
    let glyphs = bar_codes(Set::Pairs, "2026004711");
    let shows: String = glyphs
        .iter()
        .map(|&c| format!("({}) Tj ", pdf_string(&[c])))
        .collect();
    let content = format!(
        "BT /F1 9 Tf 72 640 Td (Rechnungs-Nr.) Tj ET\n\
         BT /F3 30 Tf 160 600 Td {shows}ET\n\
         BT /F1 9 Tf 72 560 Td (Danke) Tj ET\n"
    );
    let out = read(&page_with_bar_fonts(
        &content,
        &[("F3", "ShopBars", Set::Pairs, Kind::TrueTypeFull)],
    ));
    assert_eq!(
        codes(&out),
        [("Code 128".to_string(), "2026004711".to_string())],
        "{}",
        markdown(&out)
    );
    let g = String::from_utf8_lossy(&glyphs).to_string();
    assert!(!all_text(&out).contains(&g[..4]), "{}", all_text(&out));
}

#[test]
fn equal_bars_in_a_font_known_only_by_its_bars_stay_text() {
    // A row of equal bars (a ruler, a decoration) in the one-bar-a-glyph font: one width is no
    // code.
    let out = read(&page_with_bar_fonts(
        "BT /F1 10 Tf 72 760 Td (Kapitel 1) Tj ET\n\
         BT /F3 12 Tf 72 740 Td (AAAAAAAAAAAAAAAAAAAAAAAA) Tj ET\n",
        &[("F3", "ShopBars", Set::Pairs, Kind::TrueTypeFull)],
    ));
    assert!(
        all_text(&out).contains("AAAAAAAAAAAAAAAAAAAAAAAA"),
        "{}",
        all_text(&out)
    );
    assert!(out.document.assets.is_empty());
}

/// The bar and space widths of the dark modules `dark`, from the first dark one to the last.
fn elements(dark: &[bool]) -> Vec<u8> {
    let first = dark.iter().position(|&d| d).expect("a bar");
    let last = dark.iter().rposition(|&d| d).expect("a bar");
    let mut out = Vec::new();
    let (mut cur, mut n) = (dark[first], 0u8);
    for &d in &dark[first..=last] {
        if d == cur {
            n += 1;
        } else {
            out.push(n);
            (cur, n) = (d, 1);
        }
    }
    out.push(n);
    out
}

/// The codes of the one-bar-a-glyph font ([`Set::Pairs`]) for elements `w` (bar, space, ...):
/// `A` to `P` for a bar of 1 to 4 modules and its space of 1 to 4, `Q` for a last bar of 2.
fn pair_glyphs(w: &[u8]) -> Vec<u8> {
    w.chunks(2)
        .map(|p| match p.get(1) {
            Some(&sp) => b'A' + (p[0] - 1) * 4 + (sp - 1),
            None if p[0] == 2 => b'Q',
            // A last bar of another width: with a space of one module after it.
            None => b'A' + (p[0] - 1) * 4,
        })
        .collect()
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
        // Read from the page, the code has the box of its bars and its place in words.
        let c = &out.document.assets[0].codes[0];
        let high = f32::from(BAR_UNITS) / 1000.0 * 12.0;
        let bars = [
            100.0,
            650.0,
            100.0 + code128_width("2026004711", 12.0),
            650.0 + high,
        ];
        let b = c.bounds.expect("a box");
        assert!(
            b.iter()
                .zip(bars)
                .all(|(got, want)| (got - want).abs() <= 1.0),
            "{font}: boxed {b:?}, the bars are {bars:?}"
        );
        assert_eq!(c.place.as_deref(), Some("top left of the page"), "{font}");
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

#[test]
fn glyphs_of_barcode_fonts_past_the_cap_are_said() {
    // More glyphs of bars than a page looks at (50,000), drawn off the page: the note says
    // the codes there are looked for only among the first ones, and the rest stay text.
    let line: String = "A".repeat(1000);
    let mut content = String::from(
        "BT /F1 10 Tf 72 700 Td (Lieferschein 0000) Tj ET\nBT /F3 2 Tf 3 TL -90000 400 Td\n",
    );
    for _ in 0..51 {
        content += &format!("({line}) Tj T*\n");
    }
    content += "ET\n";
    let out = read(&page_with_fonts(
        &content,
        &[("F3", "Code128", &program(None))],
    ));
    let said: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert!(
        said.iter().any(|w| w.starts_with(
            "codes printed in a barcode font on page 1 are looked for only among the first \
             50000 glyphs drawn as bars"
        )),
        "{said:?}"
    );
}

/// The invoice of [`invoice`] with the codes in a bar font of `set` in the structure `kind`
/// named `name` (as written in the PDF) at `size`, the values `gap` points after the first
/// code. Returns the file and the glyph codes of both codes.
fn invoice_in(set: Set, kind: Kind, name: &str, size: f32, gap: f32) -> (Vec<u8>, [Vec<u8>; 2]) {
    let (first, second) = ("2026004711", "4006381333931");
    let w1 = bar_width(set, first, size);
    let w2 = bar_ink_width(set, second, size);
    let (c1, c2) = (bar_codes(set, first), bar_codes(set, second));
    let content = format!(
        "BT /F2 14 Tf 72 790 Td (Rechnung) Tj ET\n\
         BT /F1 9 Tf 72 760 Td (Muster Versand GmbH) Tj ET\n\
         BT /F1 9 Tf 72 748 Td (Beispielweg 1, 12345 Musterstadt) Tj ET\n\
         BT /F1 9 Tf 300 760 Td (Rechnungs-Nr.) Tj ET\n\
         BT /F1 9 Tf {x2:.2} 760 Td ({first}) Tj ET\n\
         BT /F3 {size} Tf 380 735 Td ({s1}) Tj ET\n\
         BT /F1 9 Tf 300 735 Td (Auftrags-Nr.) Tj ET\n\
         BT /F1 9 Tf {x2:.2} 735 Td (0000000000000 / 01.01.2026) Tj ET\n\
         BT /F1 9 Tf 300 715 Td (Kunden-Nr.) Tj ET\n\
         BT /F1 9 Tf {x2:.2} 715 Td (K-000123) Tj ET\n\
         BT /F2 9 Tf 300 680 Td (Referenz:) Tj ET\n\
         BT /F2 9 Tf 345 680 Td (RE-0001) Tj ET\n\
         BT /F1 9 Tf {xe:.2} 686 Td (EAN:) Tj ET\n\
         BT /F3 {size} Tf {x3:.2} 680 Td ({s2}) Tj ET\n\
         BT /F1 10 Tf 72 600 Td (Artikel) Tj 200 0 Td (Menge) Tj 100 0 Td (Preis) Tj ET\n\
         BT /F1 10 Tf 72 585 Td (Stuhl) Tj 200 0 Td (2) Tj 100 0 Td (40,00 EUR) Tj ET\n",
        s1 = pdf_string(&c1),
        s2 = pdf_string(&c2),
        x2 = 380.0 + w1 + gap,
        x3 = 585.0 - w2,
        xe = 561.0 - w2,
    );
    (
        page_with_bar_fonts(&content, &[("F3", name, set, kind)]),
        [c1, c2],
    )
}

/// Asserts the read of [`invoice_in`]: both codes listed, no glyph text glued anywhere.
fn assert_invoice(out: &ReadOutcome, glyphs: &[Vec<u8>; 2], case: &str) {
    let md = markdown(out);
    let all = all_text(out);
    for g in glyphs {
        let s: String = g.iter().map(|&c| char::from(c)).collect();
        for w in s.as_bytes().windows(6) {
            let w = String::from_utf8_lossy(w);
            assert!(
                !all.contains(w.as_ref()),
                "{case}: glyph text {w:?} in {all}"
            );
        }
    }
    assert!(
        md.contains("| Auftrags-Nr. | 0000000000000 / 01.01.2026 |"),
        "{case}: {md}"
    );
    assert!(md.contains("RE-0001**"), "{case}: {md}");
    let mut found = codes(out);
    found.sort();
    assert_eq!(
        found,
        [
            ("Code 128".to_string(), "2026004711".to_string()),
            ("Code 128".to_string(), "4006381333931".to_string()),
        ],
        "{case}: {md}"
    );
}

#[test]
fn the_structure_of_a_real_shop_barcode_font_is_taken() {
    // A simple TrueType subset named `BC C128 HD Wide` behind its prefix, WinAnsi, one bar
    // and its space a glyph (widths 33 to 134 units, a module of 16.7), cap height 0,
    // descent 0: at 3679bdb its glyph text leaked and no code was listed (neither its name nor
    // its one-bar glyphs were evidence). At 36 points the page render's crop read the second
    // code as `4-(63)1%3!931` before the fix; the code is now read from its own render first,
    // and a read must be what the glyphs print.
    for size in [24.0, 36.0] {
        let (file, glyphs) = invoice_in(
            Set::Pairs,
            Kind::TrueTypeSubset,
            "BC#20C128#20HD#20Wide",
            size,
            8.0,
        );
        assert_invoice(&read(&file), &glyphs, &format!("BC C128 HD Wide at {size}"));
    }
}

#[test]
fn bar_fonts_of_every_structure_are_taken() {
    // Both glyph sets in a TrueType subset, a full TrueType, a Type1 and a CFF program, named
    // with a short form of the code (`C128`), or with a name that says nothing (taken by its
    // bars).
    for set in [Set::Pairs, Set::Symbols] {
        for kind in [
            Kind::TrueTypeSubset,
            Kind::TrueTypeFull,
            Kind::Type1,
            Kind::Cff,
        ] {
            for name in ["BC#20C128#20HD#20Wide", "ShopBars"] {
                let size = if set == Set::Pairs { 30.0 } else { 12.0 };
                let (file, glyphs) = invoice_in(set, kind, name, size, 8.0);
                assert_invoice(&read(&file), &glyphs, &format!("{set:?} {kind:?} {name}"));
            }
        }
    }
}

#[test]
fn the_widths_of_a_font_alone_are_no_evidence() {
    // A font whose glyphs are letters (curves cut into squares here: squares and a slanted
    // block) with the widths of a bar font, named for nothing: its text stays text.
    let out = read(&page_with_bar_fonts(
        "BT /F1 10 Tf 72 700 Td (Lieferschein 0000) Tj ET\n\
         BT /F3 12 Tf 100 600 Td (ABCDEFGHIJKLMNOPQ) Tj ET\n",
        &[("F3", "ShopSans", Set::Pairs, Kind::TrueTypeFull)],
    ));
    // The pairs font is bars: it is taken (glyph evidence) - the control.
    assert!(!all_text(&out).contains("ABCDEFGH"), "{}", all_text(&out));
    let squares = |v: u8| {
        let x = i16::from(v % 4) * 10;
        vec![(x, 0, x + 300, 300), (x + 400, 400, x + 700, 700)]
    };
    let out = read(&shown_in("ShopSans", Some(&squares), "ABCDEFGHIJKLMNOPQ"));
    assert!(
        all_text(&out).contains("ABCDEFGHIJKLMNOPQ"),
        "{}",
        all_text(&out)
    );
    assert!(out.document.assets.is_empty(), "{:?}", out.document.assets);
}

#[test]
fn the_glyph_program_of_a_real_shop_barcode_font_is_taken() {
    // Fix round 3: the real font's glyph program. 2048 units an em, one rectangle a glyph at
    // x 0, 1559 high, its bars a little wider than whole modules (36, 72, 107, 143 for a
    // module of 34.3) so its spaces are a little narrower, PDF widths rounded down, a full bar
    // and a white glyph of 2 modules, and a glyph of two bars far apart after a quiet zone;
    // shown at 45 points as one show. At 6062133 its glyph text was glued into the values
    // and no code was listed.
    for kind in [Kind::TrueTypeSubset, Kind::TrueTypeFull] {
        for name in ["BC#20C128#20HD#20Wide", "ShopBars"] {
            for size in [45.0, 44.997, 30.0] {
                let (file, glyphs) = invoice_in(Set::Hd, kind, name, size, 8.0);
                assert_invoice(&read(&file), &glyphs, &format!("{kind:?} {name} {size}"));
            }
        }
    }
}
