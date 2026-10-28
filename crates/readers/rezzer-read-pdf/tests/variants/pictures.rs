// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shapes with pictures and codes (Tasks 14 and 15): template logos in the logo places
//! of a business page, and a Code 128 drawn as bars, its module and its height varied on
//! their own (one shape for each pair), on a business page and beside a photo that makes the
//! page need vision, and two equal codes closer than their quiet zone. The controls are read
//! along the axes that apply to them: the scales, the body font, the turns and one or two
//! pages; the grid of modules and heights upright and turned at scale 1.

use crate::axes::{
    ALL, Axes, Data, Labels, Look, Next, Pages, Scale, Shape, Sheet, Size, Turn, Variant,
};
use crate::common::{FormPage, code128_bars, logo, modules, show};
use crate::shapes::Built;

/// One line of the body text.
const BODY_LINE: &str = "die Firma liefert die bestellte Ware nach den vereinbarten Bedingungen";

/// The axes of a page with pictures or codes.
const RENDERED: Axes = Axes {
    size: &[Size::Real],
    data: &[Data::CourierBold, Data::Proportional],
    labels: &[Labels::Helvetica],
    look: &[Look::Real],
    next: &[Next::None],
    ..ALL
};

/// A page of `template` over a body of 7 point text in the variant's plain font.
fn page_of(v: &Variant, template: String) -> Built {
    let (plain, _) = v.faces();
    let mut t = template;
    let mut ink = 0;
    let mut y = 500.0;
    while ink < 600 {
        t += &show(plain, 7.0, 40.0, y, BODY_LINE);
        ink += BODY_LINE.chars().filter(|c| !c.is_whitespace()).count();
        y -= 7.0 * 1.3;
    }
    let mut b = Built::new(&v.geo());
    b.page = FormPage {
        template: t,
        ..FormPage::default()
    };
    b
}

/// Task 14: three template logos, 100 by 50 at the top left, 100 by 40 at the right just
/// under the top band, and 60 by 30 at the foot: together over 2 % of the page, each under
/// it. No page needs vision.
fn logos(v: &Variant) -> Sheet {
    let t = logo(40.0, 770.0, 100.0, 50.0)
        + &logo(455.0, 692.0, 100.0, 40.0)
        + &logo(40.0, 30.0, 60.0, 30.0);
    let mut s = Sheet::new(page_of(v, t));
    s.want.vision = Some(false);
    s
}

/// Where the codes of a shape stand: on a business page whose only picture is the code (it is
/// read on its own), or beside a photo that makes the page need vision as a whole (the page
/// render is read, then the code's region finer).
#[derive(Clone, Copy, PartialEq)]
enum On {
    Business,
    Photo,
}

/// The text of every code of these shapes.
const CODE: &str = "0000000001";

/// The width of the Code 128 of [`CODE`] at `module`, from its first to its last module.
fn code_width(module: f32) -> f32 {
    modules(barcodes::Symbology::Code128, CODE).width as f32 * module
}

/// The box of the bars of the code drawn by `code128_bars(CODE, x, y, module, height)`: from
/// the left edge of its first dark module to the right edge of its last one.
fn bars_box(x: f32, y: f32, module: f32, height: f32) -> [f32; 4] {
    let m = modules(barcodes::Symbology::Code128, CODE);
    let dark: Vec<u32> = (0..m.width).filter(|&c| m.dark[c as usize]).collect();
    [
        x + dark[0] as f32 * module,
        y,
        x + (dark[dark.len() - 1] + 1) as f32 * module,
        y + height,
    ]
}

/// A page of `on` with Code 128s of [`CODE`] drawn as bars, `module` points a module and
/// `height` points high, the first with its lower left corner at the right of the head of
/// the page, `apart` more each `gap` points after the one before: side by side (`beside`) or
/// one over the other. Each is found once, with its text, boxed by its bars.
fn codes_page(
    v: &Variant,
    on: On,
    (module, height): (f32, f32),
    apart: usize,
    gap: f32,
    beside: bool,
) -> Sheet {
    let (x, y) = (330.0, 790.0 - height);
    let mut t = String::new();
    let mut bars = Vec::new();
    for k in 0..=apart {
        let (cx, cy) = if beside {
            (x + k as f32 * (code_width(module) + gap), y)
        } else {
            (x, y - k as f32 * (height + gap))
        };
        t += &code128_bars(CODE, cx, cy, module, height);
        bars.push(bars_box(cx, cy, module, height));
    }
    let mut built = page_of(v, t);
    if on == On::Photo {
        built.page.photo = Some([40.0, 560.0, 200.0, 150.0]);
    }
    let mut s = Sheet::new(built);
    for b in bars {
        s.want.codes.push(("Code 128".into(), CODE.into()));
        s.want.code_bars.push(b);
    }
    s
}

/// Task 15b part 2e-3: `n` different Code 128s of module 1 and 30 points high, three a row
/// under the body of a page beside a photo (more than the 8 parts a page renders again and the
/// 10 more it names): each is found once, and with all found nothing says a code is unread.
fn many_codes(v: &Variant, n: usize) -> Sheet {
    let mut t = String::new();
    let mut want = Vec::new();
    for k in 0..n {
        let (x, y) = (40.0 + (k % 3) as f32 * 180.0, 40.0 + (k / 3) as f32 * 45.0);
        let text = format!("{:010}", k + 1);
        t += &code128_bars(&text, x, y, 1.0, 30.0);
        let m = modules(barcodes::Symbology::Code128, &text);
        let dark: Vec<u32> = (0..m.width).filter(|&c| m.dark[c as usize]).collect();
        let bars = [
            x + dark[0] as f32,
            y,
            x + (dark[dark.len() - 1] + 1) as f32,
            y + 30.0,
        ];
        want.push((text, bars));
    }
    let mut built = page_of(v, t);
    built.page.photo = Some([40.0, 560.0, 200.0, 150.0]);
    let mut s = Sheet::new(built);
    for (text, bars) in want {
        s.want.codes.push(("Code 128".into(), text));
        s.want.code_bars.push(bars);
    }
    s
}

/// Task 15b part 2e-3: a printed picture over the body of a page beside a photo, its soft
/// shades (waves across it) as a halftone of round dots on a lattice turned 45 degrees,
/// [`SCREEN`] pixels apart at one pixel a point (an inline picture): no code, and nothing said
/// of one.
fn halftone(v: &Variant) -> Sheet {
    let (w, h) = (515u32, 340u32);
    let mut hex = String::with_capacity((w * h * 2) as usize + h as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let tone =
                (0.5 + 0.25 * (fx * 9.0).sin() * (fy * 7.0).cos() + 0.2 * ((fx + fy) * 13.0).sin())
                    .clamp(0.0, 1.0);
            let (u, t) = ((x + y) as f32 / SCREEN, (x as f32 - y as f32) / SCREEN);
            let (du, dt) = (u.rem_euclid(1.0) - 0.5, t.rem_euclid(1.0) - 0.5);
            let dark = (du * du + dt * dt).sqrt() < (1.0 - tone) * 0.6;
            hex += if dark { "14" } else { "EB" };
        }
        hex.push('\n');
    }
    let t = format!(
        "q {w} 0 0 {h} 40 60 cm\nBI /W {w} /H {h} /CS /G /BPC 8 /F /AHx ID\n{hex}>\nEI\nQ\n"
    );
    let mut built = page_of(v, t);
    built.page.photo = Some([40.0, 560.0, 200.0, 150.0]);
    Sheet::new(built)
}

/// The pitch of the dots of [`halftone`], in pixels of its picture.
const SCREEN: f32 = 4.2;

/// The screens of [`halftone_sweep`]: round dots, square dots, or lines.
#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Dots,
    Squares,
    Lines,
}

/// The pitches of the screens of [`halftone_sweep`], in pixels of its picture, one row of
/// tiles each.
const SWEEP_PITCHES: [f32; 7] = [2.0, 3.0, 3.5, 4.2, 5.0, 6.0, 8.0];

/// Task 15b part 2e-3 fix round 2: a scanned page (one inline picture over all of it, `res`
/// pixels a point, under the page's text) printed as halftone screens of `screen`, soft shades
/// in waves, in 7 by 7 tiles: each row of tiles one pitch of [`SWEEP_PITCHES`], each column
/// one angle from 0 to 90 degrees in steps of 15. The tiles touch (`band` false: one screen
/// over the whole page, to its edges, whose pitch and angle change) or are bands with paper
/// between them. Blurred `blur`
/// times by a 3 by 3 box. No code, and nothing said of one.
fn halftone_sweep(v: &Variant, screen: Screen, res: u32, blur: u32, band: bool) -> Sheet {
    let (x0, y0, wp, hp) = (0.0f32, 0.0f32, 595.0f32, 842.0f32);
    let (w, h) = ((wp * res as f32) as u32, (hp * res as f32) as u32);
    let (tw, th) = (w / 7, h / 7);
    let mut g = vec![235u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let (col, row) = ((x / tw).min(6), (y / th).min(6));
            if band && (y % th) > th / 2 {
                continue;
            }
            let pitch = SWEEP_PITCHES[row as usize];
            let (s, c) = (col as f32 * 15.0).to_radians().sin_cos();
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let tone = (0.5
                + 0.3 * (fx * 11.0).sin() * (fy * 5.0).cos()
                + 0.15 * ((fx - fy) * 17.0).sin())
            .clamp(0.0, 1.0);
            let (u, t) = (
                (x as f32 * c + y as f32 * s) / pitch,
                (-(x as f32) * s + y as f32 * c) / pitch,
            );
            let dark = dark_at(screen, u, t, tone);
            if dark {
                g[(y * w + x) as usize] = 20;
            }
        }
    }
    let t = inline_gray(&mut g, (w, h), blur, [x0, y0, wp, hp]);
    Sheet::new(page_of(v, t))
}

/// Task 15b part 2e-3 fix round 2: a scanned page of 1 pixel a point (one inline picture over
/// all of it, under the page's text) printed as one halftone screen of `screen`, `pitch`
/// pixels apart, turned `angle` degrees, over the whole page or (`band`) a band from a third
/// to a half of its height, soft shades in waves, blurred `blur` times. No code, and nothing
/// said of one.
fn screen_page(
    v: &Variant,
    screen: Screen,
    (pitch, angle): (f32, f32),
    blur: u32,
    band: bool,
) -> Sheet {
    let (w, h) = (595u32, 842u32);
    let (s, c) = angle.to_radians().sin_cos();
    let mut g = vec![240u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if band && !(y > h / 3 && y < h / 2) {
                continue;
            }
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let tone = (0.5
                + 0.3 * (fx * 11.0).sin() * (fy * 5.0).cos()
                + 0.15 * ((fx - fy) * 17.0).sin())
            .clamp(0.0, 1.0);
            let (u, t) = (
                (x as f32 * c + y as f32 * s) / pitch,
                (-(x as f32) * s + y as f32 * c) / pitch,
            );
            if dark_at(screen, u, t, tone) {
                g[(y * w + x) as usize] = 20;
            }
        }
    }
    Sheet::new(page_of(
        v,
        inline_gray(&mut g, (w, h), blur, [0.0, 0.0, 595.0, 842.0]),
    ))
}

/// Whether the screen `screen` is dark at (`u`, `t`) (in pitches) for a shade `tone` (0 dark,
/// 1 light).
fn dark_at(screen: Screen, u: f32, t: f32, tone: f32) -> bool {
    let (du, dt) = (u.rem_euclid(1.0) - 0.5, t.rem_euclid(1.0) - 0.5);
    match screen {
        Screen::Dots => (du * du + dt * dt).sqrt() < (1.0 - tone) * 0.6,
        Screen::Squares => du.abs().max(dt.abs()) < (1.0 - tone) * 0.5,
        Screen::Lines => u.rem_euclid(1.0) < 1.0 - tone,
    }
}

/// The gray pixels `g` (`w` by `h`), blurred `blur` times by a 3 by 3 box, as an inline
/// picture drawn over `at` (left, bottom, width, height in points).
fn inline_gray(g: &mut [u8], (w, h): (u32, u32), blur: u32, at: [f32; 4]) -> String {
    for _ in 0..blur {
        let src = g.to_vec();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let mut t = 0u32;
                for dy in 0..3 {
                    for dx in 0..3 {
                        t += u32::from(src[((y + dy - 1) * w + x + dx - 1) as usize]);
                    }
                }
                g[(y * w + x) as usize] = (t / 9) as u8;
            }
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut z, g).expect("deflate");
    let data = z.finish().expect("deflate");
    let mut hex = String::with_capacity(data.len() * 2 + data.len() / 32);
    for (k, b) in data.iter().enumerate() {
        hex += &format!("{b:02X}");
        if k % 40 == 39 {
            hex.push('\n');
        }
    }
    let [x0, y0, wp, hp] = at;
    format!(
        "q {wp} 0 0 {hp} {x0} {y0} cm\nBI /W {w} /H {h} /CS /G /BPC 8 /F [/AHx /Fl] ID\n\
         {hex}>\nEI\nQ\n"
    )
}

/// What a scanned page the decoder read a false code from is printed with (Task 15b part
/// 2e-6), soft shades in waves on paper.
#[derive(Clone, Copy)]
enum Print {
    /// Lines standing up, `pitch` pixels apart, at 1 pixel a point.
    Lines(f32),
    /// Round dots on an upright lattice `pitch` pixels apart, at `res` pixels a point, blurred
    /// `blur` times.
    Dots(f32, u32, u32),
}

/// Task 15b part 2e-6: a scanned page (one inline picture over all of it, under the page's
/// text) printed with `print`, as pages the decoder read an EAN-13, a UPC-A or empty QR codes
/// from. No code, and nothing said of one.
fn false_reads(v: &Variant, print: Print) -> Sheet {
    let res = match print {
        Print::Lines(_) => 1,
        Print::Dots(_, res, _) => res,
    };
    let (w, h) = (595 * res, 842 * res);
    let mut g = vec![235u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let tone =
                (0.5 + 0.25 * (fx * 9.0).sin() * (fy * 7.0).cos() + 0.2 * ((fx + fy) * 13.0).sin())
                    .clamp(0.0, 1.0);
            let dark = match print {
                Print::Lines(pitch) => (x as f32 / pitch).fract() < 1.0 - tone,
                Print::Dots(pitch, ..) => {
                    let (du, dt) = (
                        (x as f32 / pitch).rem_euclid(1.0) - 0.5,
                        (y as f32 / pitch).rem_euclid(1.0) - 0.5,
                    );
                    (du * du + dt * dt).sqrt() < (1.0 - tone) * 0.6
                }
            };
            if dark {
                g[(y * w + x) as usize] = 20;
            }
        }
    }
    let blur = match print {
        Print::Lines(_) => 0,
        Print::Dots(_, _, blur) => blur,
    };
    Sheet::new(page_of(
        v,
        inline_gray(&mut g, (w, h), blur, [0.0, 0.0, 595.0, 842.0]),
    ))
}

/// The lines of [`letters`]: stems, rows of strokes and invented invoice words.
const LETTERS: [&str; 6] = [
    "Illinois Hill Mill Lillehammer Illl lll III 1111 1111",
    "Rechnung Nr. 0000000001 vom 01.01.2000, Kundennummer 0000000001",
    "|||| || | ||| |||| | || ||| | |||| || | ||| || | ||||",
    "Il1|lI1|Il1|lI1| IIIIIIIIIIIIIIIIIIIIIIIIIIII llllllllllllllllllllllll",
    "MUSTER GMBH  HILLHEIM  ILLINGEN  IIIIII  HHHHHH  NNNNNN  UUUUUU",
    "mmmmmm nnnnnn uuuuuu hhhhhh iiiiii jjjjjj ffff tttt rrrr",
];

/// Task 15b part 2e-6: a page beside a photo, drawn twice as large, with lines of stems and
/// strokes in the face `face` at 5 to 36 points (as on a page the decoder read an EAN-13 from
/// in Times-Roman). No code, and nothing said of one.
fn letters(v: &Variant, face: &str) -> Sheet {
    let mut t = String::new();
    let mut y = 700.0;
    'sizes: for size in [
        5.0f32, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 24.0, 36.0,
    ] {
        for line in LETTERS {
            if y < size + 10.0 {
                break 'sizes;
            }
            t += &format!("BT /{face} {size} Tf 20 {y} Td ({line}) Tj ET\n");
            y -= size * 1.25;
        }
    }
    let mut built = Built::new(&v.geo());
    built.page = FormPage {
        template: t,
        scale: 2.0,
        photo: Some([420.0, 720.0, 160.0, 100.0]),
        ..FormPage::default()
    };
    let mut s = Sheet::new(built);
    // The first two lines, the largest text at the top, may be read as the page's title.
    s.want.may_title = vec![format!("{} {}", LETTERS[0], LETTERS[1])];
    s
}

/// Task 15b part 2e-6: a printed Code 128 of [`CODE`] (a picture of 2 pixels a point, 3 pixels a
/// module, bars 80 pixels long) beside a photo, crossed by a dark line `rule` pixels wide past
/// both its ends (`lying`: the code turned a quarter). It is found once; the page says a dark
/// line runs across its bars, as two equal codes one over the other with a dark line between
/// them look the same.
fn ruled_code(v: &Variant, rule: u32, lying: bool) -> Sheet {
    let m = modules(barcodes::Symbology::Code128, CODE);
    let (len, bar, px, quiet) = (m.width * 3, 80u32, 3u32, 30u32);
    let (gw, gh) = (len + 2 * quiet, bar + 2 * quiet);
    let (w, h) = if lying { (gh, gw) } else { (gw, gh) };
    let mut g = vec![235u8; (w * h) as usize];
    let mut paint = |along: std::ops::Range<u32>, across: std::ops::Range<u32>| {
        for a in along {
            for t in across.clone() {
                let (x, y) = if lying { (t, a) } else { (a, t) };
                g[(y * w + x) as usize] = 0;
            }
        }
    };
    for c in (0..m.width).filter(|&c| m.dark[c as usize]) {
        paint(quiet + c * px..quiet + c * px + px, quiet..quiet + bar);
    }
    paint(10..gw - 10, quiet + bar / 2..quiet + bar / 2 + rule);
    let t = inline_gray(
        &mut g,
        (w, h),
        0,
        [300.0, 500.0, w as f32 / 2.0, h as f32 / 2.0],
    );
    let mut built = page_of(v, t);
    built.page.photo = Some([40.0, 560.0, 200.0, 150.0]);
    let mut s = Sheet::new(built);
    s.want.codes.push(("Code 128".into(), CODE.into()));
    s
}

/// The shapes of pages the decoder read false codes from: `$name => draw`.
macro_rules! false_reads {
    ($($name:literal => $draw:expr,)*) => {
        vec![$(
            Shape {
                name: $name,
                axes: SWEEP,
                draw: |v| ($draw(v), None),
            },
        )*]
    };
}

/// The axes of [`halftone_sweep`]: the page once, as it is.
const SWEEP: Axes = Axes {
    scale: &[Scale::S100],
    data: &[Data::CourierBold],
    turn: &[Turn::None],
    pages: &[Pages::One],
    ..RENDERED
};

/// The shapes of single halftone screens: `$name => (screen, pitch, angle, blur, band)`.
macro_rules! screens {
    ($($name:literal => ($screen:expr, $pitch:literal, $angle:literal, $blur:literal, $band:literal),)*) => {
        vec![$(
            Shape {
                name: $name,
                axes: SWEEP,
                draw: |v| (screen_page(v, $screen, ($pitch, $angle), $blur, $band), None),
            },
        )*]
    };
}

/// The shapes of the halftone sweep: `$name => (screen, res, blur, band)`.
macro_rules! sweep {
    ($($name:literal => ($screen:expr, $res:literal, $blur:literal, $band:literal),)*) => {
        vec![$(
            Shape {
                name: $name,
                axes: SWEEP,
                draw: |v| (halftone_sweep(v, $screen, $res, $blur, $band), None),
            },
        )*]
    };
}

/// Task 15b: a ruler of 100 ticks, 3 points long, 2 points apart, every fifth 1 point wide and
/// the others half a point, at the head of a page `on`: no code, and nothing said of one.
fn ruler(v: &Variant, on: On) -> Sheet {
    let mut t = String::from("q 0 g\n");
    for k in 0..100 {
        let w = if k % 5 == 0 { 1.0 } else { 0.5 };
        t += &format!("{:.1} 760 {w} 3 re\n", 60.0 + k as f32 * 2.0);
    }
    t += "f Q\n";
    let mut built = page_of(v, t);
    if on == On::Photo {
        built.page.photo = Some([40.0, 560.0, 200.0, 150.0]);
    }
    let mut s = Sheet::new(built);
    if on == On::Business {
        s.want.vision = Some(false);
    }
    s
}

/// Task 15: a Code 128 of `module` points a module and `height` points high, drawn as bars at
/// the right of the head of a business page. It is found once, with its text.
fn barcode(v: &Variant, module: f32, height: f32) -> Sheet {
    codes_page(v, On::Business, (module, height), 0, 0.0, true)
}

/// The axes of the module and height grid: at scale 1 (the scale changes module and height
/// together, as the controls of the full axes show), upright and turned a quarter (its bars
/// then lie down).
const GRID: Axes = Axes {
    scale: &[Scale::S100],
    data: &[Data::CourierBold],
    turn: &[Turn::None, Turn::Left],
    pages: &[Pages::One],
    ..RENDERED
};

/// The shapes of a grid of modules and heights: `$name => (on, module, height)`, each read
/// along [`GRID`].
macro_rules! grid {
    ($($name:literal => ($on:expr, $module:literal, $height:literal),)*) => {
        vec![$(
            Shape {
                name: $name,
                axes: GRID,
                draw: |v| (codes_page(v, $on, ($module, $height), 0, 0.0, true), None),
            },
        )*]
    };
}

/// The shapes of two equal codes of module 0.8 close to each other:
/// `$name => (on, height, gap, beside)`.
macro_rules! twins {
    ($($name:literal => ($on:expr, $height:literal, $gap:literal, $beside:literal),)*) => {
        vec![$(
            Shape {
                name: $name,
                axes: GRID,
                draw: |v| (codes_page(v, $on, (0.8, $height), 1, $gap, $beside), None),
            },
        )*]
    };
}

/// The shapes with pictures and codes.
pub(crate) fn shapes() -> Vec<Shape> {
    let mut all = vec![
        Shape {
            name: "template logos",
            axes: RENDERED,
            draw: |v| (logos(v), None),
        },
        Shape {
            name: "barcode, module 0.8, height 20",
            axes: RENDERED,
            draw: |v| (barcode(v, 0.8, 20.0), None),
        },
        Shape {
            name: "barcode, module 1.0, height 30",
            axes: RENDERED,
            draw: |v| (barcode(v, 1.0, 30.0), None),
        },
        Shape {
            name: "barcode, module 1.2, height 20",
            axes: RENDERED,
            draw: |v| (barcode(v, 1.2, 20.0), None),
        },
        Shape {
            name: "barcode, module 1.2, height 30",
            axes: RENDERED,
            draw: |v| (barcode(v, 1.2, 30.0), None),
        },
        Shape {
            name: "barcode, module 1.2, height 80",
            axes: RENDERED,
            draw: |v| (barcode(v, 1.2, 80.0), None),
        },
        Shape {
            name: "barcode, module 1.2, height 40",
            axes: RENDERED,
            draw: |v| (barcode(v, 1.2, 40.0), None),
        },
        Shape {
            name: "barcode, module 0.8, height 60",
            axes: RENDERED,
            draw: |v| (barcode(v, 0.8, 60.0), None),
        },
        Shape {
            name: "a ruler of short ticks",
            axes: RENDERED,
            draw: |v| (ruler(v, On::Business), None),
        },
        Shape {
            name: "a ruler of short ticks beside a photo",
            axes: RENDERED,
            draw: |v| (ruler(v, On::Photo), None),
        },
        Shape {
            name: "barcode beside a photo, module 1.2, height 40",
            axes: RENDERED,
            draw: |v| {
                let s = codes_page(v, On::Photo, (1.2, 40.0), 0, 0.0, true);
                (s, None)
            },
        },
    ];
    // Task 15b: the module and the height varied on their own, on both kinds of page (the
    // pairs the controls above read along every axis are left out of the business grid).
    all.extend(grid! {
        "barcode, module 0.5, height 3" => (On::Business, 0.5, 3.0),
        "barcode, module 0.5, height 5" => (On::Business, 0.5, 5.0),
        "barcode, module 0.5, height 8" => (On::Business, 0.5, 8.0),
        "barcode, module 0.5, height 20" => (On::Business, 0.5, 20.0),
        "barcode, module 0.5, height 30" => (On::Business, 0.5, 30.0),
        "barcode, module 0.5, height 40" => (On::Business, 0.5, 40.0),
        "barcode, module 0.5, height 80" => (On::Business, 0.5, 80.0),
        "barcode, module 0.5, height 150" => (On::Business, 0.5, 150.0),
        "barcode, module 0.8, height 3" => (On::Business, 0.8, 3.0),
        "barcode, module 0.8, height 5" => (On::Business, 0.8, 5.0),
        "barcode, module 0.8, height 8" => (On::Business, 0.8, 8.0),
        "barcode, module 0.8, height 30" => (On::Business, 0.8, 30.0),
        "barcode, module 0.8, height 40" => (On::Business, 0.8, 40.0),
        "barcode, module 0.8, height 80" => (On::Business, 0.8, 80.0),
        "barcode, module 0.8, height 150" => (On::Business, 0.8, 150.0),
        "barcode, module 1.2, height 3" => (On::Business, 1.2, 3.0),
        "barcode, module 1.2, height 5" => (On::Business, 1.2, 5.0),
        "barcode, module 1.2, height 8" => (On::Business, 1.2, 8.0),
        "barcode, module 1.2, height 150" => (On::Business, 1.2, 150.0),
        "barcode beside a photo, module 0.5, height 3" => (On::Photo, 0.5, 3.0),
        "barcode beside a photo, module 0.5, height 5" => (On::Photo, 0.5, 5.0),
        "barcode beside a photo, module 0.5, height 8" => (On::Photo, 0.5, 8.0),
        "barcode beside a photo, module 0.5, height 20" => (On::Photo, 0.5, 20.0),
        "barcode beside a photo, module 0.5, height 30" => (On::Photo, 0.5, 30.0),
        "barcode beside a photo, module 0.5, height 40" => (On::Photo, 0.5, 40.0),
        "barcode beside a photo, module 0.5, height 80" => (On::Photo, 0.5, 80.0),
        "barcode beside a photo, module 0.5, height 150" => (On::Photo, 0.5, 150.0),
        "barcode beside a photo, module 0.8, height 3" => (On::Photo, 0.8, 3.0),
        "barcode beside a photo, module 0.8, height 5" => (On::Photo, 0.8, 5.0),
        "barcode beside a photo, module 0.8, height 8" => (On::Photo, 0.8, 8.0),
        "barcode beside a photo, module 0.8, height 20" => (On::Photo, 0.8, 20.0),
        "barcode beside a photo, module 0.8, height 30" => (On::Photo, 0.8, 30.0),
        "barcode beside a photo, module 0.8, height 40" => (On::Photo, 0.8, 40.0),
        "barcode beside a photo, module 0.8, height 80" => (On::Photo, 0.8, 80.0),
        "barcode beside a photo, module 0.8, height 150" => (On::Photo, 0.8, 150.0),
        "barcode beside a photo, module 1.2, height 3" => (On::Photo, 1.2, 3.0),
        "barcode beside a photo, module 1.2, height 5" => (On::Photo, 1.2, 5.0),
        "barcode beside a photo, module 1.2, height 8" => (On::Photo, 1.2, 8.0),
        "barcode beside a photo, module 1.2, height 20" => (On::Photo, 1.2, 20.0),
        "barcode beside a photo, module 1.2, height 30" => (On::Photo, 1.2, 30.0),
        "barcode beside a photo, module 1.2, height 80" => (On::Photo, 1.2, 80.0),
        "barcode beside a photo, module 1.2, height 150" => (On::Photo, 1.2, 150.0),
    });
    // More codes than a page renders again, all read; a halftone without a code.
    all.extend([
        Shape {
            name: "21 barcodes beside a photo",
            axes: GRID,
            draw: |v| (many_codes(v, 21), None),
        },
        Shape {
            name: "a halftone beside a photo",
            axes: RENDERED,
            draw: |v| (halftone(v), None),
        },
    ]);
    // Single halftone screens over the whole page or a band, no code.
    all.extend(screens! {
        "a scan of dots 2.5 px apart at 0 degrees, crisp" => (Screen::Dots, 2.5, 0.0, 0, false),
        "a scan of dots 2.5 px apart at 0 degrees, crisp, a band" => (Screen::Dots, 2.5, 0.0, 0, true),
        "a scan of dots 2.5 px apart at 0 degrees, blur 1" => (Screen::Dots, 2.5, 0.0, 1, false),
        "a scan of dots 2.5 px apart at 0 degrees, blur 1, a band" => (Screen::Dots, 2.5, 0.0, 1, true),
        "a scan of dots 3.6 px apart at 45 degrees, crisp" => (Screen::Dots, 3.6, 45.0, 0, false),
        "a scan of dots 3.6 px apart at 45 degrees, crisp, a band" => (Screen::Dots, 3.6, 45.0, 0, true),
        "a scan of dots 3.6 px apart at 45 degrees, blur 1" => (Screen::Dots, 3.6, 45.0, 1, false),
        "a scan of dots 3.6 px apart at 45 degrees, blur 1, a band" => (Screen::Dots, 3.6, 45.0, 1, true),
        "a scan of dots 4.2 px apart at 45 degrees, crisp" => (Screen::Dots, 4.2, 45.0, 0, false),
        "a scan of dots 4.2 px apart at 45 degrees, crisp, a band" => (Screen::Dots, 4.2, 45.0, 0, true),
        "a scan of dots 4.2 px apart at 45 degrees, blur 1" => (Screen::Dots, 4.2, 45.0, 1, false),
        "a scan of dots 4.2 px apart at 45 degrees, blur 1, a band" => (Screen::Dots, 4.2, 45.0, 1, true),
        "a scan of dots 5.0 px apart at 15 degrees, crisp" => (Screen::Dots, 5.0, 15.0, 0, false),
        "a scan of dots 5.0 px apart at 15 degrees, crisp, a band" => (Screen::Dots, 5.0, 15.0, 0, true),
        "a scan of dots 5.0 px apart at 15 degrees, blur 1" => (Screen::Dots, 5.0, 15.0, 1, false),
        "a scan of dots 5.0 px apart at 15 degrees, blur 1, a band" => (Screen::Dots, 5.0, 15.0, 1, true),
        "a scan of dots 6.0 px apart at 75 degrees, crisp" => (Screen::Dots, 6.0, 75.0, 0, false),
        "a scan of dots 6.0 px apart at 75 degrees, crisp, a band" => (Screen::Dots, 6.0, 75.0, 0, true),
        "a scan of dots 6.0 px apart at 75 degrees, blur 1" => (Screen::Dots, 6.0, 75.0, 1, false),
        "a scan of dots 6.0 px apart at 75 degrees, blur 1, a band" => (Screen::Dots, 6.0, 75.0, 1, true),
        "a scan of dots 3.0 px apart at 30 degrees, crisp" => (Screen::Dots, 3.0, 30.0, 0, false),
        "a scan of dots 3.0 px apart at 30 degrees, crisp, a band" => (Screen::Dots, 3.0, 30.0, 0, true),
        "a scan of dots 3.0 px apart at 30 degrees, blur 1" => (Screen::Dots, 3.0, 30.0, 1, false),
        "a scan of dots 3.0 px apart at 30 degrees, blur 1, a band" => (Screen::Dots, 3.0, 30.0, 1, true),
        "a scan of lines 2.2 px apart at 0 degrees, crisp" => (Screen::Lines, 2.2, 0.0, 0, false),
        "a scan of lines 2.2 px apart at 0 degrees, crisp, a band" => (Screen::Lines, 2.2, 0.0, 0, true),
        "a scan of lines 2.2 px apart at 0 degrees, blur 1" => (Screen::Lines, 2.2, 0.0, 1, false),
        "a scan of lines 2.2 px apart at 0 degrees, blur 1, a band" => (Screen::Lines, 2.2, 0.0, 1, true),
        "a scan of lines 3.3 px apart at 90 degrees, crisp" => (Screen::Lines, 3.3, 90.0, 0, false),
        "a scan of lines 3.3 px apart at 90 degrees, crisp, a band" => (Screen::Lines, 3.3, 90.0, 0, true),
        "a scan of lines 3.3 px apart at 90 degrees, blur 1" => (Screen::Lines, 3.3, 90.0, 1, false),
        "a scan of lines 3.3 px apart at 90 degrees, blur 1, a band" => (Screen::Lines, 3.3, 90.0, 1, true),
        "a scan of lines 4.0 px apart at 45 degrees, crisp" => (Screen::Lines, 4.0, 45.0, 0, false),
        "a scan of lines 4.0 px apart at 45 degrees, crisp, a band" => (Screen::Lines, 4.0, 45.0, 0, true),
        "a scan of lines 4.0 px apart at 45 degrees, blur 1" => (Screen::Lines, 4.0, 45.0, 1, false),
        "a scan of lines 4.0 px apart at 45 degrees, blur 1, a band" => (Screen::Lines, 4.0, 45.0, 1, true),
        "a scan of lines 5.5 px apart at 90 degrees, crisp" => (Screen::Lines, 5.5, 90.0, 0, false),
        "a scan of lines 5.5 px apart at 90 degrees, crisp, a band" => (Screen::Lines, 5.5, 90.0, 0, true),
        "a scan of lines 5.5 px apart at 90 degrees, blur 1" => (Screen::Lines, 5.5, 90.0, 1, false),
        "a scan of lines 5.5 px apart at 90 degrees, blur 1, a band" => (Screen::Lines, 5.5, 90.0, 1, true),
        "a scan of square dots 3.5 px apart at 0 degrees, crisp" => (Screen::Squares, 3.5, 0.0, 0, false),
        "a scan of square dots 3.5 px apart at 0 degrees, crisp, a band" => (Screen::Squares, 3.5, 0.0, 0, true),
        "a scan of square dots 3.5 px apart at 0 degrees, blur 1" => (Screen::Squares, 3.5, 0.0, 1, false),
        "a scan of square dots 3.5 px apart at 0 degrees, blur 1, a band" => (Screen::Squares, 3.5, 0.0, 1, true),
        "a scan of square dots 5.0 px apart at 30 degrees, crisp" => (Screen::Squares, 5.0, 30.0, 0, false),
        "a scan of square dots 5.0 px apart at 30 degrees, crisp, a band" => (Screen::Squares, 5.0, 30.0, 0, true),
        "a scan of square dots 5.0 px apart at 30 degrees, blur 1" => (Screen::Squares, 5.0, 30.0, 1, false),
        "a scan of square dots 5.0 px apart at 30 degrees, blur 1, a band" => (Screen::Squares, 5.0, 30.0, 1, true),
    });
    // Halftone screens of every pitch and angle, no code.
    all.extend(sweep! {
        "halftone dots, 1 px a point, crisp" => (Screen::Dots, 1, 0, false),
        "halftone dots, 1 px a point, blur 1" => (Screen::Dots, 1, 1, false),
        "halftone dots, 1 px a point, blur 2" => (Screen::Dots, 1, 2, false),
        "halftone dots, 2 px a point, crisp" => (Screen::Dots, 2, 0, false),
        "halftone dots, 2 px a point, blur 1" => (Screen::Dots, 2, 1, false),
        "halftone dots, 2 px a point, blur 2" => (Screen::Dots, 2, 2, false),
        "halftone dot bands, 1 px a point, crisp" => (Screen::Dots, 1, 0, true),
        "halftone dot bands, 1 px a point, blur 1" => (Screen::Dots, 1, 1, true),
        "halftone dot bands, 1 px a point, blur 2" => (Screen::Dots, 1, 2, true),
        "halftone dot bands, 2 px a point, crisp" => (Screen::Dots, 2, 0, true),
        "halftone dot bands, 2 px a point, blur 1" => (Screen::Dots, 2, 1, true),
        "halftone dot bands, 2 px a point, blur 2" => (Screen::Dots, 2, 2, true),
        "halftone square dots, 1 px a point, crisp" => (Screen::Squares, 1, 0, false),
        "halftone square dot bands, 1 px a point, crisp" => (Screen::Squares, 1, 0, true),
        "halftone square dots, 1 px a point, blur 1" => (Screen::Squares, 1, 1, false),
        "halftone square dot bands, 1 px a point, blur 1" => (Screen::Squares, 1, 1, true),
        "halftone square dots, 1 px a point, blur 2" => (Screen::Squares, 1, 2, false),
        "halftone square dot bands, 1 px a point, blur 2" => (Screen::Squares, 1, 2, true),
        "halftone square dots, 2 px a point, crisp" => (Screen::Squares, 2, 0, false),
        "halftone square dot bands, 2 px a point, crisp" => (Screen::Squares, 2, 0, true),
        "halftone square dots, 2 px a point, blur 1" => (Screen::Squares, 2, 1, false),
        "halftone square dot bands, 2 px a point, blur 1" => (Screen::Squares, 2, 1, true),
        "halftone square dots, 2 px a point, blur 2" => (Screen::Squares, 2, 2, false),
        "halftone square dot bands, 2 px a point, blur 2" => (Screen::Squares, 2, 2, true),
        "halftone lines, 1 px a point, crisp" => (Screen::Lines, 1, 0, false),
        "halftone lines, 1 px a point, blur 1" => (Screen::Lines, 1, 1, false),
        "halftone lines, 1 px a point, blur 2" => (Screen::Lines, 1, 2, false),
        "halftone lines, 2 px a point, crisp" => (Screen::Lines, 2, 0, false),
        "halftone lines, 2 px a point, blur 1" => (Screen::Lines, 2, 1, false),
        "halftone lines, 2 px a point, blur 2" => (Screen::Lines, 2, 2, false),
        "halftone line bands, 1 px a point, crisp" => (Screen::Lines, 1, 0, true),
        "halftone line bands, 1 px a point, blur 1" => (Screen::Lines, 1, 1, true),
        "halftone line bands, 1 px a point, blur 2" => (Screen::Lines, 1, 2, true),
        "halftone line bands, 2 px a point, crisp" => (Screen::Lines, 2, 0, true),
        "halftone line bands, 2 px a point, blur 1" => (Screen::Lines, 2, 1, true),
        "halftone line bands, 2 px a point, blur 2" => (Screen::Lines, 2, 2, true),
    });
    // Screens and letters the decoder read false codes from: no code.
    all.extend(screens! {
        "a scan of dots 3.6 px apart at 52 degrees, crisp" => (Screen::Dots, 3.6, 52.0, 0, false),
        "a scan of dots 2.7 px apart at 83 degrees, crisp" => (Screen::Dots, 2.7, 83.0, 0, false),
    });
    all.extend(false_reads! {
        "a scan of lines 3.3 px apart standing up" => |v| false_reads(v, Print::Lines(3.3)),
        "a scan of dots 3.5 px apart, 2 px a point, blur 1" => |v| false_reads(v, Print::Dots(3.5, 2, 1)),
        "a scan of dots 3.6 px apart, 2 px a point, blur 1" => |v| false_reads(v, Print::Dots(3.6, 2, 1)),
        "a scan of dots 3.8 px apart, 2 px a point, blur 1" => |v| false_reads(v, Print::Dots(3.8, 2, 1)),
        "stems and strokes in Times-Roman, twice as large" => |v| letters(v, "FT"),
    });
    // A printed code crossed by a dark line: found once, with words of a doubt.
    all.extend(false_reads! {
        "a printed barcode crossed by a dark line 1 px wide" => |v| ruled_code(v, 1, false),
        "a printed barcode crossed by a dark line 4 px wide" => |v| ruled_code(v, 4, false),
        "a printed barcode lying, crossed by a dark line 2 px wide" => |v| ruled_code(v, 2, true),
    });
    // Two equal codes closer than their quiet zone.
    all.extend(twins! {
        "twin barcodes 1 pt apart" => (On::Business, 20.0, 1.0, true),
        "twin barcodes 3 pt apart" => (On::Business, 20.0, 3.0, true),
        "twin barcodes 4 pt apart" => (On::Business, 20.0, 4.0, true),
        "twin barcodes 5 pt apart" => (On::Business, 20.0, 5.0, true),
        "twin barcodes 1 pt apart, one over the other" => (On::Business, 20.0, 1.0, false),
        "twin barcodes 3 pt apart, one over the other" => (On::Business, 20.0, 3.0, false),
        "twin barcodes 3 pt high 1 pt apart, one over the other" => (On::Business, 3.0, 1.0, false),
        "twin barcodes 5 pt high 1 pt apart, one over the other" => (On::Business, 5.0, 1.0, false),
        "twin barcodes beside a photo 1 pt apart" => (On::Photo, 20.0, 1.0, true),
        "twin barcodes beside a photo 3 pt apart" => (On::Photo, 20.0, 3.0, true),
        "twin barcodes beside a photo 4 pt apart" => (On::Photo, 20.0, 4.0, true),
        "twin barcodes beside a photo 5 pt apart" => (On::Photo, 20.0, 5.0, true),
        "twin barcodes beside a photo 1 pt apart, one over the other" => (On::Photo, 20.0, 1.0, false),
        "twin barcodes beside a photo 3 pt apart, one over the other" => (On::Photo, 20.0, 3.0, false),
        "twin barcodes 3 pt high beside a photo 1 pt apart, one over the other" => (On::Photo, 3.0, 1.0, false),
        "twin barcodes 5 pt high beside a photo 1 pt apart, one over the other" => (On::Photo, 5.0, 1.0, false),
    });
    all
}
