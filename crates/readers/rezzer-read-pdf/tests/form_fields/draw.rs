// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Drawing the pieces of a generated form: rules, marks and text, in PDF content operators.

/// A rule drawn as a filled bar of thickness `t` from `x0` to `x1`, its top at `y`.
pub(crate) fn bar_h(x0: f32, x1: f32, y: f32, t: f32) -> String {
    format!("{x0} {} {} {t} re f\n", y - t, x1 - x0)
}

/// The same rule stroked as a line of width `t`, its middle at `y - t / 2`.
pub(crate) fn line_h(x0: f32, x1: f32, y: f32, t: f32) -> String {
    let m = y - t / 2.0;
    format!("q {t} w {x0} {m} m {x1} {m} l S Q\n")
}

/// A rule drawn as a filled bar of thickness `t` from `y0` to `y1`, centred on `x`.
pub(crate) fn bar_v(x: f32, y0: f32, y1: f32, t: f32) -> String {
    format!("{} {y0} {t} {} re f\n", x - t / 2.0, y1 - y0)
}

/// A separator mark of `h` points from `y0`, centred on `x`, filled and stroked with a
/// hairline as print runs draw it.
pub(crate) fn mark_b(x: f32, y0: f32, h: f32) -> String {
    let (l, r, top) = (x - 0.12, x + 0.12, y0 + h);
    format!("q 0 w {r} {top} m {l} {top} l {l} {y0} l {r} {y0} l h B* Q\n")
}

/// Text in Helvetica-Bold (`/FB`).
pub(crate) fn bold(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FB {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// Text in Helvetica (`/FL`) without the scaling of a template label.
pub(crate) fn plain(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FL {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// Text in Courier (`/FD`), whose every character is 0.6 of the size wide.
pub(crate) fn courier(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /FD {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// How the thin rules of a form are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Stroke {
    /// Filled bars.
    Bar,
    /// Rows of short filled dashes, a dash and a gap of a fifth and a tenth of the data size.
    Dashed,
    /// Stroked lines with a pattern of dots and round caps.
    Dotted,
}

/// A horizontal rule of thickness `t` from `x0` to `x1`, its top at `y`, drawn as `stroke`;
/// `size` (the data size) sets the dashes.
pub(crate) fn rule_h(stroke: Stroke, x0: f32, x1: f32, y: f32, t: f32, size: f32) -> String {
    match stroke {
        Stroke::Bar => bar_h(x0, x1, y, t),
        Stroke::Dashed => {
            let (dash, step) = (0.2 * size, 0.3 * size);
            let mut out = String::new();
            let mut x = x0;
            while x < x1 {
                out += &format!("{x} {} {} {t} re f\n", y - t, dash.min(x1 - x));
                x += step;
            }
            out
        }
        Stroke::Dotted => {
            let m = y - t / 2.0;
            format!(
                "q {t} w 1 J [0 {}] 0 d {x0} {m} m {x1} {m} l S Q\n",
                0.15 * size
            )
        }
    }
}

/// A vertical rule of thickness `t` from `y0` to `y1`, centred on `x`, drawn as `stroke`.
pub(crate) fn rule_v(stroke: Stroke, x: f32, y0: f32, y1: f32, t: f32, size: f32) -> String {
    match stroke {
        Stroke::Bar => bar_v(x, y0, y1, t),
        Stroke::Dashed => {
            let (dash, step) = (0.2 * size, 0.3 * size);
            let mut out = String::new();
            let mut y = y0;
            while y < y1 {
                out += &format!("{} {y} {t} {} re f\n", x - t / 2.0, dash.min(y1 - y));
                y += step;
            }
            out
        }
        Stroke::Dotted => format!(
            "q {t} w 1 J [0 {}] 0 d {x} {y0} m {x} {y1} l S Q\n",
            0.15 * size
        ),
    }
}
