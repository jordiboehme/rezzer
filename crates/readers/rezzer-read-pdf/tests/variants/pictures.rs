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
