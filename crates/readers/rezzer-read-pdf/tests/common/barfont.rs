// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Code 128 barcode font built for the tests: a TrueType program whose glyphs are the bars
//! of the Code 128 symbols, as the free barcode fonts draw them. Symbol value `v` stands at
//! the character `v + 32` for values 0 to 94 and at `v + 100` for 95 to 106 (WinAnsi `Ã` to
//! `Î`), so a code shown in it reads as odd characters (`Í%"$"*%!)Î`). No downloaded font is
//! used.

use super::Builder;

/// The bar and space widths of each Code 128 symbol in modules, bar first; the stop has 7.
pub const PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

/// The symbol values of `text` as a Code 128 the way a shop prints it with a barcode font:
/// code set C (pairs of digits) when `text` is an even number of digits, else code set B;
/// with the start, the check symbol and the stop.
pub fn code128_values(text: &str) -> Vec<u8> {
    let digits = text.bytes().all(|b| b.is_ascii_digit());
    let mut v: Vec<u8> = if digits && text.len().is_multiple_of(2) {
        let mut v = vec![105];
        v.extend(
            text.as_bytes()
                .chunks(2)
                .map(|p| (p[0] - b'0') * 10 + (p[1] - b'0')),
        );
        v
    } else {
        let mut v = vec![104];
        v.extend(text.bytes().map(|b| b - 32));
        v
    };
    let check = v
        .iter()
        .enumerate()
        .map(|(i, &s)| u32::from(s) * (i as u32).max(1))
        .sum::<u32>()
        % 103;
    v.push(check as u8);
    v.push(106);
    v
}

/// The character code of symbol value `v` in the font.
pub fn code_of(v: u8) -> u8 {
    if v < 95 { v + 32 } else { v + 100 }
}

/// `text` as a Code 128 in the font, as a PDF string body for `Tj` (bytes escaped).
pub fn code128_string(text: &str) -> String {
    let mut s = String::new();
    for v in code128_values(text) {
        let c = code_of(v);
        match c {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(char::from(c));
            }
            32..=126 => s.push(char::from(c)),
            _ => s.push_str(&format!("\\{c:03o}")),
        }
    }
    s
}

/// A rectangle of a glyph in font units: x0, y0, x1, y1.
pub type Rect = (i16, i16, i16, i16);

/// The rectangles of the glyph of each symbol value, for a font of another shape.
pub type Glyphs<'a> = &'a dyn Fn(u8) -> Vec<Rect>;

/// Font units of one module of the font, and the height of its bars.
pub const MODULE_UNITS: i16 = 60;
pub const BAR_UNITS: i16 = 1600;

/// The width of `text` as a Code 128 in the font at `size`, in points.
pub fn code128_width(text: &str, size: f32) -> f32 {
    let modules: u32 = code128_values(text)
        .iter()
        .map(|&v| {
            PATTERNS[v as usize]
                .bytes()
                .map(|b| u32::from(b - b'0'))
                .sum::<u32>()
        })
        .sum();
    modules as f32 * f32::from(MODULE_UNITS) / 1000.0 * size
}

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend(x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend(x.to_be_bytes());
}

/// A glyph of the `glyf` table made of the rectangles `rects` (x0, y0, x1, y1 in font
/// units), each one contour of four on-curve points.
fn rect_glyph(rects: &[(i16, i16, i16, i16)]) -> Vec<u8> {
    let mut g = Vec::new();
    if rects.is_empty() {
        return g;
    }
    let xmin = rects.iter().map(|r| r.0).min().unwrap_or(0);
    let ymin = rects.iter().map(|r| r.1).min().unwrap_or(0);
    let xmax = rects.iter().map(|r| r.2).max().unwrap_or(0);
    let ymax = rects.iter().map(|r| r.3).max().unwrap_or(0);
    be16(&mut g, rects.len() as u16);
    for v in [xmin, ymin, xmax, ymax] {
        be16(&mut g, v as u16);
    }
    for k in 0..rects.len() {
        be16(&mut g, (k * 4 + 3) as u16);
    }
    be16(&mut g, 0);
    let points: Vec<(i16, i16)> = rects
        .iter()
        .flat_map(|&(x0, y0, x1, y1)| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)])
        .collect();
    g.extend(std::iter::repeat_n(1u8, points.len()));
    let (mut px, mut py) = (0i16, 0i16);
    for &(x, _) in &points {
        be16(&mut g, (x - px) as u16);
        px = x;
    }
    for &(_, y) in &points {
        be16(&mut g, (y - py) as u16);
        py = y;
    }
    while g.len() % 4 != 0 {
        g.push(0);
    }
    g
}

/// The bars of a pattern (`212222`) as rectangles, and the advance in font units.
fn pattern_bars(pattern: &str) -> (Vec<(i16, i16, i16, i16)>, u16) {
    let mut x = 0i16;
    let mut bars = Vec::new();
    for (k, w) in pattern.bytes().enumerate() {
        let w = i16::from(w - b'0') * MODULE_UNITS;
        if k % 2 == 0 {
            bars.push((x, 0, x + w, BAR_UNITS));
        }
        x += w;
    }
    (bars, x as u16)
}

/// The TrueType program: glyph 0 empty, glyph `v + 1` the bars of symbol value `v`, mapped
/// from the Unicode character of its code (WinAnsi, which is Latin-1 for `Ã` to `Î`).
/// With `glyphs`, glyph `v + 1` is `glyphs(v)` instead (for fonts of other shapes).
pub fn program(glyphs: Option<Glyphs<'_>>) -> Vec<u8> {
    let n = 108u16;
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    let mut hmtx = Vec::new();
    // Glyph 0 is empty: it starts and ends at 0.
    be32(&mut loca, 0);
    be32(&mut loca, 0);
    be16(&mut hmtx, 600);
    be16(&mut hmtx, 0);
    for v in 0..107u8 {
        let (bars, adv) = pattern_bars(PATTERNS[v as usize]);
        let rects = match glyphs {
            Some(f) => f(v),
            None => bars,
        };
        glyf.extend(rect_glyph(&rects));
        be32(&mut loca, glyf.len() as u32);
        be16(&mut hmtx, adv);
        be16(&mut hmtx, 0);
    }
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, 1000);
    head.extend([0u8; 16]);
    for v in [0u16, 0, 1000, BAR_UNITS as u16] {
        be16(&mut head, v);
    }
    be16(&mut head, 0);
    be16(&mut head, 8);
    be16(&mut head, 2);
    be16(&mut head, 1);
    be16(&mut head, 0);
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, BAR_UNITS as u16);
    be16(&mut hhea, 0);
    be16(&mut hhea, 0);
    be16(&mut hhea, 1000);
    hhea.extend([0u8; 22]);
    be16(&mut hhea, n);
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, n);
    be16(&mut maxp, 64);
    be16(&mut maxp, 16);
    maxp.extend([0u8; 22]);
    // cmap: format 4, segments U+0020..U+007E to glyphs 1.., U+00C3..U+00CE to 96.., end.
    let segs: [(u16, u16, u16); 3] = [(0x20, 0x7E, 1), (0xC3, 0xCE, 96), (0xFFFF, 0xFFFF, 0)];
    let mut sub = Vec::new();
    let segx2 = (segs.len() * 2) as u16;
    be16(&mut sub, 4);
    be16(&mut sub, 16 + 8 * segs.len() as u16);
    be16(&mut sub, 0);
    be16(&mut sub, segx2);
    be16(&mut sub, 4);
    be16(&mut sub, 1);
    be16(&mut sub, segx2 - 4);
    for s in segs {
        be16(&mut sub, s.1);
    }
    be16(&mut sub, 0);
    for s in segs {
        be16(&mut sub, s.0);
    }
    for s in segs {
        let delta = if s.0 == 0xFFFF {
            1
        } else {
            s.2.wrapping_sub(s.0)
        };
        be16(&mut sub, delta);
    }
    for _ in segs {
        be16(&mut sub, 0);
    }
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 1);
    be32(&mut cmap, 12);
    cmap.extend(sub);
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend([0u8; 28]);
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"post", post),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, tables.len() as u16);
    be16(&mut out, 128);
    be16(&mut out, 3);
    be16(&mut out, 0);
    let mut offset = 12 + 16 * tables.len() as u32;
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset);
        be32(&mut out, data.len() as u32);
        body.extend(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() as u32 + body.len() as u32;
    }
    out.extend(body);
    out
}

/// Adds the font named `name` (a subset prefix is added) with the program `program` to `b`
/// as a simple TrueType font in WinAnsi; returns its object number.
pub fn add_font(b: &mut Builder, name: &str, program: &[u8]) -> u32 {
    let ff = b.stream(&format!("/Length1 {}", program.len()), program);
    let desc = b.add(format!(
        "<< /Type /FontDescriptor /FontName /ABCDEF+{name} /Flags 32 \
         /FontBBox [0 0 1000 {BAR_UNITS}] /ItalicAngle 0 /Ascent {BAR_UNITS} /Descent 0 \
         /CapHeight {BAR_UNITS} /StemV 80 /FontFile2 {ff} 0 R >>"
    ));
    let widths: Vec<String> = (32u8..=206)
        .map(|c| {
            let v = if (32..=126).contains(&c) {
                Some(c - 32)
            } else if (195..=206).contains(&c) {
                Some(c - 100)
            } else {
                None
            };
            v.map_or("0".to_string(), |v| {
                pattern_bars(PATTERNS[v as usize]).1.to_string()
            })
        })
        .collect();
    b.add(format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+{name} /FirstChar 32 \
         /LastChar 206 /Widths [{}] /Encoding /WinAnsiEncoding /FontDescriptor {desc} 0 R >>",
        widths.join(" ")
    ))
}

/// A one-page A4 file with `content`, Helvetica as `/F1`, Helvetica-Bold as `/F2` and the
/// fonts `fonts` (resource name, base name, program) beside them.
pub fn page_with_fonts(content: &str, fonts: &[(&str, &str, &[u8])]) -> Vec<u8> {
    page_of(content, fonts, false)
}

/// [`page_with_fonts`] with a photo, 4 by 4 gray pixels as `/Photo`, that `content` may draw.
pub fn page_with_photo(content: &str, fonts: &[(&str, &str, &[u8])]) -> Vec<u8> {
    page_of(content, fonts, true)
}

fn page_of(content: &str, fonts: &[(&str, &str, &[u8])], photo: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let f1 = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let f2 = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let mut res = format!("/F1 {f1} 0 R /F2 {f2} 0 R");
    for (key, name, program) in fonts {
        let f = add_font(&mut b, name, program);
        res += &format!(" /{key} {f} 0 R");
    }
    let xobjects = if photo {
        let pixels: Vec<u8> = (0..16u8).map(|i| 40 + i * 11).collect();
        let p = b.stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
             /BitsPerComponent 8",
            &pixels,
        );
        format!("/XObject << /Photo {p} 0 R >>")
    } else {
        String::new()
    };
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << {res} >> {xobjects} >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}
