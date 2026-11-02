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

/// The bars of Code 128 symbol value `v` as rectangles of [`program`], and its advance.
pub fn pattern_rects(v: u8) -> (Vec<Rect>, u16) {
    pattern_bars(PATTERNS[v as usize])
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

// Fonts of other structures (Task 15b part 2f-1 fix round 1): the structure of a real shop's
// barcode font (a simple TrueType subset, `BC C128 HD Wide`, one bar and its space a glyph),
// and the same glyphs as a full TrueType, a Type1 and a CFF program.

/// What the glyphs of a bar font are: Code 128 symbols (11 modules a glyph, as above), or
/// one bar and the space after it a glyph, as `BC C128 HD Wide` draws them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Set {
    Symbols,
    Pairs,
    /// Code 128 symbols with the bars raised over a digit under them in the same glyph (a
    /// block here), as a human-readable bar font draws them.
    Readable,
    /// Letters (a block a glyph, the space empty): a text font, as an OCR-B or MICR font.
    Letters,
    /// The glyph program of a real shop's `BC C128 HD Wide` (fix round 3), 2048 units an em:
    /// each glyph one rectangle at x 0, 1559 high, its bar 36, 72, 107 or 143 units wide (1 to
    /// 4 modules of about 34.3, the bars a little wider than whole modules) and its advance 3
    /// to 6 modules; a full bar of 2 modules with no space (`Q`), a white glyph of 2 modules
    /// (`R`), and a glyph of two bars far apart (`S`, advance 1024). Its PDF widths are
    /// rounded down to the 1000-unit em as the real file's are.
    Hd,
}

/// Font units of a module of [`Set::Hd`], its bars, and its height, in 2048 units an em.
pub const HD_MODULE: f64 = 34.3;
pub const HD_BARS: [i16; 4] = [36, 72, 107, 143];
pub const HD_HEIGHT: i16 = 1559;

/// The advance of `k` modules in [`Set::Hd`], in its units, and its PDF width.
fn hd_advance(k: u8) -> (u16, u16) {
    let units = (f64::from(k) * HD_MODULE).round() as u16;
    let pdf = match units {
        69 => 33,
        103 => 50,
        137 => 66,
        172 => 83,
        206 => 100,
        240 => 117,
        274 => 134,
        u => u * 1000 / 2048,
    };
    (units, pdf)
}

/// The PDF width (1000 units an em) of a glyph of `set` that advances `adv` font units.
fn pdf_width(set: Set, adv: u16) -> u16 {
    match set {
        Set::Hd if adv == 1024 => 500,
        Set::Hd => hd_advance((f64::from(adv) / HD_MODULE).round() as u8).1,
        _ => adv,
    }
}

/// The units an em of the fonts of `set`.
fn units_per_em(set: Set) -> u16 {
    if set == Set::Hd { 2048 } else { 1000 }
}

/// The structure of a bar font's program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A simple TrueType font, its name behind a six-letter subset prefix.
    TrueTypeSubset,
    /// A simple TrueType font without a subset prefix.
    TrueTypeFull,
    /// A Type1 font program (`/FontFile`, eexec encrypted).
    Type1,
    /// A CFF program (`/FontFile3 /Subtype /Type1C`).
    Cff,
}

/// Font units of a module of [`Set::Pairs`] (as the real font: widths 33, 50, 66, 83 and so on)
/// and the height of its bars (its ascent).
pub const PAIR_MODULE: f32 = 50.0 / 3.0;
pub const PAIR_HEIGHT: i16 = 761;

/// One glyph of a bar font: its WinAnsi code, its rectangles and its advance in font units.
pub struct BarGlyph {
    pub code: u8,
    pub rects: Vec<Rect>,
    pub adv: u16,
}

/// The code of the glyph of a bar of `b` modules and the space of `s` after it in
/// [`Set::Pairs`] (`A` to `P`), and `Q` for the last bar of the stop (2 modules, no space).
fn pair_code(b: u8, s: u8) -> u8 {
    if s == 0 {
        b'Q'
    } else {
        b'A' + (b - 1) * 4 + (s - 1)
    }
}

/// The glyphs of a font of `set`.
pub fn bar_glyphs(set: Set) -> Vec<BarGlyph> {
    match set {
        Set::Readable => (0..107u8)
            .map(|v| {
                let (rects, adv) = pattern_bars(PATTERNS[v as usize]);
                let mut rects: Vec<Rect> = rects
                    .iter()
                    .map(|&(x0, _, x1, y1)| (x0, 300, x1, y1))
                    .collect();
                rects.push((60, 0, 300, 200));
                BarGlyph {
                    code: code_of(v),
                    rects,
                    adv,
                }
            })
            .collect(),
        Set::Hd => {
            let mut out = Vec::new();
            for b in 1..=4u8 {
                for sp in 1..=4u8 {
                    out.push(BarGlyph {
                        code: pair_code(b, sp),
                        rects: vec![(0, 0, HD_BARS[usize::from(b - 1)], HD_HEIGHT)],
                        adv: hd_advance(b + sp).0,
                    });
                }
            }
            out.push(BarGlyph {
                code: b'Q',
                rects: vec![(0, 0, 69, HD_HEIGHT)],
                adv: 69,
            });
            out.push(BarGlyph {
                code: b'R',
                rects: Vec::new(),
                adv: 69,
            });
            out.push(BarGlyph {
                code: b'S',
                rects: vec![(128, 0, 196, HD_HEIGHT), (828, 0, 896, HD_HEIGHT)],
                adv: 1024,
            });
            out
        }
        Set::Letters => (32..=126u8)
            .map(|c| BarGlyph {
                code: c,
                rects: if c == b' ' {
                    Vec::new()
                } else {
                    vec![(50, 0, 550, 700)]
                },
                adv: 600,
            })
            .collect(),
        Set::Symbols => (0..107u8)
            .map(|v| {
                let (rects, adv) = pattern_bars(PATTERNS[v as usize]);
                BarGlyph {
                    code: code_of(v),
                    rects,
                    adv,
                }
            })
            .collect(),
        Set::Pairs => {
            let mut out = Vec::new();
            let at = |k: u8| (f32::from(k) * PAIR_MODULE).round() as i16;
            for b in 1..=4u8 {
                for s in 1..=4u8 {
                    out.push(BarGlyph {
                        code: pair_code(b, s),
                        rects: vec![(0, 0, at(b), PAIR_HEIGHT)],
                        adv: at(b + s) as u16,
                    });
                }
            }
            out.push(BarGlyph {
                code: b'Q',
                rects: vec![(0, 0, at(2), PAIR_HEIGHT)],
                adv: at(2) as u16,
            });
            out
        }
    }
}

/// The codes `text` is shown with as a Code 128 in a font of `set`.
pub fn bar_codes(set: Set, text: &str) -> Vec<u8> {
    let values = code128_values(text);
    match set {
        Set::Symbols | Set::Readable => values.into_iter().map(code_of).collect(),
        Set::Letters => text.bytes().collect(),
        Set::Pairs | Set::Hd => {
            let mut out = Vec::new();
            for v in values {
                let w: Vec<u8> = PATTERNS[v as usize].bytes().map(|b| b - b'0').collect();
                for p in w.chunks(2) {
                    out.push(pair_code(p[0], p.get(1).copied().unwrap_or(0)));
                }
            }
            if set == Set::Hd {
                // A bar of 2 and its space of 2 as the full bar and the white glyph, and after
                // the stop a quiet zone of white glyphs and the glyph of two bars.
                if let Some(k) = out.iter().position(|&c| c == pair_code(2, 2)) {
                    out.splice(k..=k, *b"QR");
                }
                out.extend([b'R'; 6]);
                out.push(b'S');
            }
            out
        }
    }
}

/// `codes` as the body of a PDF string.
pub fn pdf_string(codes: &[u8]) -> String {
    let mut s = String::new();
    for &c in codes {
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

/// The width in points of `text` shown as a Code 128 in a font of `set` at `size`: the sum of
/// the advances of its glyphs.
pub fn bar_width(set: Set, text: &str, size: f32) -> f32 {
    let glyphs = bar_glyphs(set);
    bar_codes(set, text)
        .iter()
        .map(|c| {
            glyphs
                .iter()
                .find(|g| g.code == *c)
                .map_or(0.0, |g| f32::from(pdf_width(set, g.adv)))
        })
        .sum::<f32>()
        / 1000.0
        * size
}

/// The width of the ink of `text` as a Code 128 in a font of `set` at `size`: to the right of
/// the last bar.
pub fn bar_ink_width(set: Set, text: &str, size: f32) -> f32 {
    codes_ink_width(set, &bar_codes(set, text), size)
}

/// The width of the ink of the code of `text` alone: for [`Set::Hd`] without the quiet zone
/// and the glyph of two bars after it.
pub fn code_ink_width(set: Set, text: &str, size: f32) -> f32 {
    let codes = bar_codes(set, text);
    let n = if set == Set::Hd {
        codes.len() - 7
    } else {
        codes.len()
    };
    codes_ink_width(set, &codes[..n], size)
}

/// The width of the ink of the glyph codes `codes` in a font of `set` at `size`.
pub fn codes_ink_width(set: Set, codes: &[u8], size: f32) -> f32 {
    let glyphs = bar_glyphs(set);
    let mut x = 0.0;
    let mut right = 0.0f32;
    for c in codes {
        if let Some(g) = glyphs.iter().find(|g| g.code == *c) {
            let em = f32::from(units_per_em(set)) / 1000.0;
            for r in &g.rects {
                right = right.max(x + f32::from(r.2) / em);
            }
            x += f32::from(pdf_width(set, g.adv));
        }
    }
    right / 1000.0 * size
}

/// The height of the bars of a font of `set` at `size`.
pub fn bar_height(set: Set, size: f32) -> f32 {
    let h = match set {
        Set::Symbols | Set::Readable => BAR_UNITS,
        Set::Pairs => PAIR_HEIGHT,
        Set::Letters => 700,
        Set::Hd => return f32::from(HD_HEIGHT) / 2048.0 * size,
    };
    f32::from(h) / 1000.0 * size
}

/// The Adobe glyph name of WinAnsi code `c`, for the codes the bar fonts use.
fn glyph_name(c: u8) -> String {
    const ASCII: [&str; 95] = [
        "space",
        "exclam",
        "quotedbl",
        "numbersign",
        "dollar",
        "percent",
        "ampersand",
        "quotesingle",
        "parenleft",
        "parenright",
        "asterisk",
        "plus",
        "comma",
        "hyphen",
        "period",
        "slash",
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "colon",
        "semicolon",
        "less",
        "equal",
        "greater",
        "question",
        "at",
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "H",
        "I",
        "J",
        "K",
        "L",
        "M",
        "N",
        "O",
        "P",
        "Q",
        "R",
        "S",
        "T",
        "U",
        "V",
        "W",
        "X",
        "Y",
        "Z",
        "bracketleft",
        "backslash",
        "bracketright",
        "asciicircum",
        "underscore",
        "grave",
        "a",
        "b",
        "c",
        "d",
        "e",
        "f",
        "g",
        "h",
        "i",
        "j",
        "k",
        "l",
        "m",
        "n",
        "o",
        "p",
        "q",
        "r",
        "s",
        "t",
        "u",
        "v",
        "w",
        "x",
        "y",
        "z",
        "braceleft",
        "bar",
        "braceright",
        "asciitilde",
    ];
    const HIGH: [&str; 12] = [
        "Atilde",
        "Adieresis",
        "Aring",
        "AE",
        "Ccedilla",
        "Egrave",
        "Eacute",
        "Ecircumflex",
        "Edieresis",
        "Igrave",
        "Iacute",
        "Icircumflex",
    ];
    match c {
        32..=126 => ASCII[usize::from(c - 32)].to_string(),
        195..=206 => HIGH[usize::from(c - 195)].to_string(),
        _ => format!("g{c}"),
    }
}

/// A TrueType program of `glyphs`, mapped from the Unicode character of each code (WinAnsi,
/// Latin-1 for the codes used), ascent `ascent`.
fn truetype(glyphs: &[BarGlyph], ascent: i16, upem: u16) -> Vec<u8> {
    let n = glyphs.len() as u16 + 1;
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    let mut hmtx = Vec::new();
    be32(&mut loca, 0);
    be32(&mut loca, 0);
    be16(&mut hmtx, 500);
    be16(&mut hmtx, 0);
    for g in glyphs {
        glyf.extend(rect_glyph(&g.rects));
        be32(&mut loca, glyf.len() as u32);
        be16(&mut hmtx, g.adv);
        be16(&mut hmtx, 0);
    }
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, upem);
    head.extend([0u8; 16]);
    for v in [0u16, 0, upem, ascent as u16] {
        be16(&mut head, v);
    }
    be16(&mut head, 0);
    be16(&mut head, 8);
    be16(&mut head, 2);
    be16(&mut head, 1);
    be16(&mut head, 0);
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, ascent as u16);
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
    // cmap format 4: one segment for each glyph, in code order, and the end.
    let mut segs: Vec<(u16, u16)> = glyphs
        .iter()
        .enumerate()
        .map(|(k, g)| (u16::from(g.code), k as u16 + 1))
        .collect();
    segs.sort_unstable();
    segs.push((0xFFFF, 0));
    let mut sub = Vec::new();
    let segx2 = (segs.len() * 2) as u16;
    be16(&mut sub, 4);
    be16(&mut sub, 16 + 8 * segs.len() as u16);
    be16(&mut sub, 0);
    be16(&mut sub, segx2);
    be16(&mut sub, 2);
    be16(&mut sub, 0);
    be16(&mut sub, 0);
    for s in &segs {
        be16(&mut sub, s.0);
    }
    be16(&mut sub, 0);
    for s in &segs {
        be16(&mut sub, s.0);
    }
    for s in &segs {
        be16(
            &mut sub,
            if s.0 == 0xFFFF {
                1
            } else {
                s.1.wrapping_sub(s.0)
            },
        );
    }
    for _ in &segs {
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

/// The path of a glyph as relative moves: for each rectangle a move to its lower left corner
/// and four lines around it back to that corner. `mv(dx, dy)` and `ln(dx, dy)` write them.
fn rect_moves(rects: &[Rect], mut mv: impl FnMut(i32, i32), mut ln: impl FnMut(i32, i32)) {
    let (mut x, mut y) = (0i32, 0i32);
    for &(x0, y0, x1, y1) in rects {
        let (x0, y0, x1, y1) = (i32::from(x0), i32::from(y0), i32::from(x1), i32::from(y1));
        mv(x0 - x, y0 - y);
        ln(0, y1 - y0);
        ln(x1 - x0, 0);
        ln(0, y0 - y1);
        ln(x0 - x1, 0);
        (x, y) = (x0, y0);
    }
}

/// A CFF INDEX of `items`, offsets of 4 bytes.
fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, items.len() as u16);
    if items.is_empty() {
        return out;
    }
    out.push(4);
    let mut off = 1u32;
    be32(&mut out, off);
    for i in items {
        off += i.len() as u32;
        be32(&mut out, off);
    }
    for i in items {
        out.extend(i);
    }
    out
}

/// A Type 2 charstring number (the 16-bit form).
fn t2_num(out: &mut Vec<u8>, v: i32) {
    out.push(28);
    be16(out, v as i16 as u16);
}

/// A CFF program named `name` of `glyphs`: glyph names from WinAnsi, charset format 0.
fn cff(name: &str, glyphs: &[BarGlyph]) -> Vec<u8> {
    let names: Vec<String> = glyphs.iter().map(|g| glyph_name(g.code)).collect();
    let mut charstrings = vec![{
        let mut c = Vec::new();
        t2_num(&mut c, 500);
        c.push(14);
        c
    }];
    for g in glyphs {
        let mut c = Vec::new();
        let mut first = true;
        let cell = std::cell::RefCell::new(&mut c);
        rect_moves(
            &g.rects,
            |dx, dy| {
                let mut c = cell.borrow_mut();
                if first {
                    t2_num(&mut c, i32::from(g.adv));
                    first = false;
                }
                t2_num(&mut c, dx);
                t2_num(&mut c, dy);
                c.push(21);
            },
            |dx, dy| {
                let mut c = cell.borrow_mut();
                t2_num(&mut c, dx);
                t2_num(&mut c, dy);
                c.push(5);
            },
        );
        if g.rects.is_empty() {
            t2_num(&mut c, i32::from(g.adv));
        }
        c.push(14);
        charstrings.push(c);
    }
    let strings: Vec<Vec<u8>> = names.iter().map(|n| n.as_bytes().to_vec()).collect();
    let header = vec![1u8, 0, 4, 4];
    let name_index = cff_index(&[name.as_bytes().to_vec()]);
    let string_index = cff_index(&strings);
    let gsubr = cff_index(&[]);
    let mut charset = vec![0u8];
    for k in 0..glyphs.len() {
        be16(&mut charset, 391 + k as u16);
    }
    let cs_index = cff_index(&charstrings);
    // A Private DICT of nominalWidthX 0.
    let private: Vec<u8> = vec![139, 21];
    // The Top DICT holds three offsets as 5-byte numbers: its size is known before them.
    let dict = |charset_at: u32, cs_at: u32, private_at: u32| {
        let mut d = Vec::new();
        let num = |d: &mut Vec<u8>, v: u32| {
            d.push(29);
            be32(d, v);
        };
        num(&mut d, charset_at);
        d.push(15);
        num(&mut d, cs_at);
        d.push(17);
        num(&mut d, private.len() as u32);
        num(&mut d, private_at);
        d.push(18);
        d
    };
    let top_len = cff_index(&[dict(0, 0, 0)]).len();
    let start = header.len() + name_index.len() + top_len + string_index.len() + gsubr.len();
    let charset_at = start as u32;
    let cs_at = charset_at + charset.len() as u32;
    let private_at = cs_at + cs_index.len() as u32;
    let top = cff_index(&[dict(charset_at, cs_at, private_at)]);
    let mut out = header;
    out.extend(name_index);
    out.extend(top);
    out.extend(string_index);
    out.extend(gsubr);
    out.extend(charset);
    out.extend(cs_index);
    out.extend(private);
    out
}

/// A Type 1 charstring number.
fn t1_num(out: &mut Vec<u8>, v: i32) {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => {
            let w = v - 108;
            out.push((w / 256 + 247) as u8);
            out.push((w % 256) as u8);
        }
        -1131..=-108 => {
            let w = -v - 108;
            out.push((w / 256 + 251) as u8);
            out.push((w % 256) as u8);
        }
        _ => {
            out.push(255);
            be32(out, v as u32);
        }
    }
}

/// `data` encrypted as Type 1 does it with key `r`, after `skip` leading zero bytes.
fn t1_encrypt(data: &[u8], mut r: u16, skip: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + skip);
    for &p in std::iter::repeat_n(&0u8, skip).chain(data) {
        let c = p ^ (r >> 8) as u8;
        r = (u16::from(c).wrapping_add(r))
            .wrapping_mul(52845)
            .wrapping_add(22719);
        out.push(c);
    }
    out
}

/// A Type 1 program named `name` of `glyphs`, with the lengths of its clear, encrypted and
/// trailing parts.
fn type1(name: &str, glyphs: &[BarGlyph]) -> (Vec<u8>, usize, usize, usize) {
    let mut clear = format!(
        "%!PS-AdobeFont-1.0: {name} 001\n12 dict begin\n/FontName /{name} def\n/FontType 1 def\n\
         /PaintType 0 def\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
         /FontBBox {{0 0 1000 1600}} readonly def\n\
         /Encoding 256 array\n0 1 255 {{1 index exch /.notdef put}} for\n"
    );
    for g in glyphs {
        clear += &format!("dup {} /{} put\n", g.code, glyph_name(g.code));
    }
    clear += "readonly def\ncurrentdict end\ncurrentfile eexec\n";
    let mut private = format!(
        "dup /Private 8 dict dup begin\n/RD{{string currentfile exch readstring pop}}executeonly def\n\
         /ND{{noaccess def}}executeonly def\n/NP{{noaccess put}}executeonly def\n/lenIV 4 def\n\
         /MinFeature{{16 16}}def\n/password 5839 def\n/BlueValues [] ND\n\
         2 index /CharStrings {} dict dup begin\n",
        glyphs.len() + 1
    )
    .into_bytes();
    let charstring = |name: &str, adv: u16, rects: &[Rect], out: &mut Vec<u8>| {
        let mut c = Vec::new();
        t1_num(&mut c, 0);
        t1_num(&mut c, i32::from(adv));
        c.push(13);
        let cell = std::cell::RefCell::new(&mut c);
        rect_moves(
            rects,
            |dx, dy| {
                let mut c = cell.borrow_mut();
                t1_num(&mut c, dx);
                t1_num(&mut c, dy);
                c.push(21);
            },
            |dx, dy| {
                let mut c = cell.borrow_mut();
                t1_num(&mut c, dx);
                t1_num(&mut c, dy);
                c.push(5);
            },
        );
        if !rects.is_empty() {
            c.push(9);
        }
        c.push(14);
        let enc = t1_encrypt(&c, 4330, 4);
        out.extend(format!("/{name} {} RD ", enc.len()).as_bytes());
        out.extend(enc);
        out.extend(b" ND\n");
    };
    charstring(".notdef", 500, &[], &mut private);
    for g in glyphs {
        charstring(&glyph_name(g.code), g.adv, &g.rects, &mut private);
    }
    private.extend(
        b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\n\
          mark currentfile closefile\n",
    );
    let enc = t1_encrypt(&private, 55665, 4);
    let mut trailer = String::new();
    for _ in 0..8 {
        trailer += &"0".repeat(64);
        trailer.push('\n');
    }
    trailer += "cleartomark\n";
    let mut out = clear.clone().into_bytes();
    out.extend(&enc);
    out.extend(trailer.as_bytes());
    (out, clear.len(), enc.len(), trailer.len())
}

/// Adds a bar font of `set` in the structure `kind` named `name` (written into the PDF as
/// given, so a space is `#20`; a subset prefix is added for [`Kind::TrueTypeSubset`]) to `b`;
/// returns its object number. For [`Set::Pairs`] the dictionaries are those of the real
/// font: widths for codes 0 to 255 (5 where no glyph is), WinAnsi, ascent 761, cap height 0,
/// descent 0, flags 32.
pub fn add_bar_font(b: &mut Builder, name: &str, set: Set, kind: Kind) -> u32 {
    let glyphs = bar_glyphs(set);
    let base = match kind {
        Kind::TrueTypeSubset => format!("ABCDEE+{name}"),
        _ => name.to_string(),
    };
    let (file, subtype) = match kind {
        Kind::TrueTypeSubset | Kind::TrueTypeFull => {
            let p = truetype(&glyphs, PAIR_HEIGHT.max(BAR_UNITS), units_per_em(set));
            let ff = b.stream(&format!("/Length1 {}", p.len()), &p);
            (format!("/FontFile2 {ff} 0 R"), "TrueType")
        }
        Kind::Cff => {
            let plain: String = base.replace("#20", "");
            let p = cff(&plain, &glyphs);
            let ff = b.stream("/Subtype /Type1C", &p);
            (format!("/FontFile3 {ff} 0 R"), "Type1")
        }
        Kind::Type1 => {
            let plain: String = base.replace("#20", "");
            let (p, l1, l2, l3) = type1(&plain, &glyphs);
            let ff = b.stream(&format!("/Length1 {l1} /Length2 {l2} /Length3 {l3}"), &p);
            (format!("/FontFile {ff} 0 R"), "Type1")
        }
    };
    let (ascent, bbox) = match set {
        Set::Symbols | Set::Readable | Set::Letters => (BAR_UNITS, format!("0 0 1000 {BAR_UNITS}")),
        Set::Pairs => (PAIR_HEIGHT, format!("0 0 438 {PAIR_HEIGHT}")),
        Set::Hd => (761, "0 0 438 761".to_string()),
    };
    let desc = b.add(format!(
        "<< /Type /FontDescriptor /FontName /{base} /Flags 32 /FontBBox [{bbox}] \
         /ItalicAngle 0 /Ascent {ascent} /Descent 0 /CapHeight 0 /StemV 0 {file} >>"
    ));
    let widths: Vec<String> = (0u16..=255)
        .map(|c| {
            glyphs
                .iter()
                .find(|g| u16::from(g.code) == c)
                .map_or("5".to_string(), |g| pdf_width(set, g.adv).to_string())
        })
        .collect();
    b.add(format!(
        "<< /Type /Font /Subtype /{subtype} /BaseFont /{base} /FirstChar 0 /LastChar 255 \
         /Widths [{}] /Encoding /WinAnsiEncoding /FontDescriptor {desc} 0 R >>",
        widths.join(" ")
    ))
}

/// A one-page A4 file with `content`, Helvetica as `/F1`, Helvetica-Bold as `/F2`, and the bar
/// fonts `fonts` (resource name, base name, glyph set, structure).
pub fn page_with_bar_fonts(content: &str, fonts: &[(&str, &str, Set, Kind)]) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let f1 = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let f2 = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let mut res = format!("/F1 {f1} 0 R /F2 {f2} 0 R");
    for (key, name, set, kind) in fonts {
        let f = add_bar_font(&mut b, name, *set, *kind);
        res += &format!(" /{key} {f} 0 R");
    }
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << {res} >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// The bar fonts a print form may show (see `print_form`): resource name, base name as
/// written, glyph set and structure. `FC` and `FN` are Code 128 symbol fonts named for codes
/// and not; `FP` to `FS` draw one bar and its space a glyph, as `BC C128 HD Wide` does, in
/// each structure; `FO` and `FM` are letter fonts with names that look like codes; `FH` is a
/// human-readable bar font; `FX` and `FY` have the glyph program of a real shop's font
/// ([`Set::Hd`]), named and not.
pub const BAR_FONTS: [(&str, &str, Set, Kind); 11] = [
    ("FC", "Code128", Set::Symbols, Kind::TrueTypeSubset),
    ("FN", "ShopBars", Set::Symbols, Kind::TrueTypeSubset),
    (
        "FP",
        "BC#20C128#20HD#20Wide",
        Set::Pairs,
        Kind::TrueTypeSubset,
    ),
    ("FQ", "ShopBars", Set::Pairs, Kind::Type1),
    ("FR", "BC#20C128#20HD#20Wide", Set::Pairs, Kind::Cff),
    ("FS", "ShopBars", Set::Pairs, Kind::TrueTypeFull),
    ("FO", "IDAutomationOCRb", Set::Letters, Kind::TrueTypeSubset),
    ("FM", "Ocean13", Set::Letters, Kind::Cff),
    (
        "FH",
        "Libre#20Barcode#20128#20Text",
        Set::Readable,
        Kind::TrueTypeSubset,
    ),
    ("FX", "BC#20C128#20HD#20Wide", Set::Hd, Kind::TrueTypeSubset),
    ("FY", "ShopBars", Set::Hd, Kind::TrueTypeFull),
];

/// The codes of the one-bar-a-glyph font ([`Set::Pairs`]) for elements `w` (bar, space,
/// ...): `A` to `P` for a bar of 1 to 4 modules and its space of 1 to 4, `Q` for a last bar
/// of 2, and a last bar of another width with a space of one module.
pub fn pair_codes(w: &[u8]) -> Vec<u8> {
    w.chunks(2)
        .map(|p| match p.get(1) {
            Some(&sp) => pair_code(p[0], sp),
            None if p[0] == 2 => b'Q',
            None => pair_code(p[0], 1),
        })
        .collect()
}

/// The bar and space widths of the dark modules `dark`, from the first dark one to the last.
pub fn elements(dark: &[bool]) -> Vec<u8> {
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
