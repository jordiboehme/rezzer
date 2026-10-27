// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shapes with pictures and codes (Tasks 14 and 15): template logos in the logo places
//! of a business page, and a Code 128 drawn as bars, its module and its height varied on
//! their own (one shape for each pair). They are read along the axes that apply to them: the
//! scales, the body font, the turns and one or two pages.

use crate::axes::{ALL, Axes, Data, Labels, Look, Next, Shape, Sheet, Size, Variant};
use crate::common::{FormPage, code128_bars, logo, show};
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

/// Task 15: a Code 128 of `module` points a module and `height` points high, drawn as bars at
/// the right of the head of a business page. It is found once, with its text.
fn barcode(v: &Variant, module: f32, height: f32) -> Sheet {
    let t = code128_bars("0000000001", 360.0, 700.0, module, height);
    let mut s = Sheet::new(page_of(v, t));
    s.want.codes.push(("Code 128".into(), "0000000001".into()));
    s
}

/// The shapes with pictures and codes.
pub(crate) fn shapes() -> Vec<Shape> {
    vec![
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
    ]
}
