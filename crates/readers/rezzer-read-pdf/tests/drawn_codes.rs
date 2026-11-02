// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Linear codes drawn as paths at fine modules: a code is listed with its own value, or named,
//! never listed with another value (ruling P6-X). All values are invented.

mod common;

use common::{Builder, markdown, modules, read};
use rezzer_model::ReadOutcome;

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

/// An A4 page with `content`, Helvetica as `/F1` and, with `photo`, a large photo that makes
/// the page need vision as a whole.
fn page(content: &str, photo: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let f1 = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let (draw, xobject) = if photo {
        let pixels: Vec<u8> = (0..16u8).map(|i| 40 + i * 11).collect();
        let p = b.stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &pixels,
        );
        (
            "q 300 0 0 250 150 250 cm /Photo Do Q\n",
            format!("/XObject << /Photo {p} 0 R >>"),
        )
    } else {
        ("", String::new())
    };
    let c = b.stream("", format!("{draw}{content}").as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {f1} 0 R >> {xobject} >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// Every code listed: kind and value.
fn codes(out: &ReadOutcome) -> Vec<(String, String)> {
    out.document
        .assets
        .iter()
        .flat_map(|a| a.codes.iter())
        .map(|c| (c.symbology.clone(), c.text.clone().unwrap_or_default()))
        .collect()
}

/// Whether a listing is the code drawn: the same kind and value, or a UPC-A drawn and read as
/// the EAN-13 with a leading 0 (or the other way round), which is the same code.
fn right(listed: &(String, String), kind: &str, text: &str) -> bool {
    let (s, t) = (listed.0.as_str(), listed.1.as_str());
    (s == kind && t == text)
        || (s == "EAN-13" && kind == "UPC-A" && t == format!("0{text}"))
        || (s == "UPC-A" && kind == "EAN-13" && format!("0{t}") == text)
}

#[test]
fn ean_and_upc_drawn_as_paths_at_fine_modules_are_never_listed_wrong() {
    // One filled rectangle a bar, modules of 0.55 to 0.70 points, at ten offsets of a tenth
    // of a point: the review of part 2f-1 found wrong digits with valid check digits (EAN-13
    // 4006381333931 listed as UPC-A 046312333931 at 0.62). Each read is the code, or the
    // code is named; on a business page and on a page that needs vision as a whole.
    use barcodes::Symbology as S;
    let mut wrong = Vec::new();
    for (s, text, kind) in [
        (S::Ean13, "4006381333931", "EAN-13"),
        (S::Ean13, "5901234123457", "EAN-13"),
        (S::Ean13, "0012345678905", "EAN-13"),
        (S::UpcA, "036000291452", "UPC-A"),
    ] {
        let w = elements(&modules(s, text).dark);
        for (m, photo) in [55, 57, 60, 62, 64, 67, 70]
            .into_iter()
            .flat_map(|m| [(m, false), (m, true)])
        {
            let module = m as f32 / 100.0;
            for x10 in 0..10 {
                let x0 = 100.0 + x10 as f32 / 10.0;
                let mut content =
                    String::from("BT /F1 9 Tf 72 760 Td (Lieferschein) Tj ET\nq 0 g\n");
                let mut x = x0;
                for (k, &e) in w.iter().enumerate() {
                    let wd = f32::from(e) * module;
                    if k % 2 == 0 {
                        content += &format!("{x:.3} 650 {wd:.3} 30 re\n");
                    }
                    x += wd;
                }
                content += "f Q\n";
                let out = read(&page(&content, photo));
                let listed = codes(&out);
                if listed.iter().any(|l| !right(l, kind, text)) {
                    wrong.push(format!(
                        "{kind} {text} at {module}, x {x0}, photo {photo}: {listed:?}"
                    ));
                }
                if listed.is_empty() {
                    let md = markdown(&out);
                    assert!(
                        md.contains("not decoded") || md.contains("not listed"),
                        "{md}"
                    );
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} wrong: {wrong:#?}", wrong.len());
}
