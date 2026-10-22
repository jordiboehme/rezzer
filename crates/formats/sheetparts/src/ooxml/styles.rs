// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use super::Ctx;
use crate::Look;
use crate::numfmt;
use crate::xml::{Elem, Ev};

/// No id written: the cell style (or the default) decides.
const NONE: u32 = u32::MAX;

/// One `xf` entry, reduced to what the looks need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Xf {
    font: u32,
    fill: u32,
    border: u32,
    xf_id: u32,
    indent: u8,
    center_across: bool,
}

impl Xf {
    fn from_elem(e: &Elem) -> Xf {
        let id = |key: &str| e.attr(key).and_then(|v| v.parse().ok()).unwrap_or(NONE);
        Xf {
            font: id("fontId"),
            fill: id("fillId"),
            border: id("borderId"),
            xf_id: e.attr("xfId").and_then(|v| v.parse().ok()).unwrap_or(0),
            indent: 0,
            center_across: false,
        }
    }
}

/// A border edge counts as a rule when it is a thin or thicker line.
fn is_rule(style: Option<&str>) -> bool {
    matches!(style, Some("thin" | "medium" | "thick" | "double"))
}

/// White (also by palette index and theme color 0) and automatic colors are no fill. White
/// with a tint below -0.05 is a visible grey (for example "White, Background 1, Darker 15%"),
/// so it counts as filled.
fn is_visible_color(e: &Elem) -> bool {
    if e.flag("auto") {
        return false;
    }
    let darkened = e
        .attr("tint")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .is_some_and(|t| t < -0.05);
    if let Some(rgb) = e.attr("rgb") {
        return !rgb.to_ascii_uppercase().ends_with("FFFFFF") || darkened;
    }
    if let Some(i) = e.attr("indexed") {
        return match i {
            "64" => false,
            "1" | "9" | "65" => darkened,
            _ => true,
        };
    }
    if let Some(t) = e.attr("theme") {
        return t != "0" || darkened;
    }
    false
}

/// The cell formats of a workbook: which number format each cell style index uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Styles {
    /// The number format id of each `cellXfs` entry, in order.
    xf_ids: Vec<u32>,
    custom: BTreeMap<u32, String>,
    /// The `cellXfs` entries, parallel to `xf_ids`.
    xfs: Vec<Xf>,
    /// The `cellStyleXfs` entries (named cell styles).
    style_xfs: Vec<Xf>,
    /// Per font: bold.
    fonts: Vec<bool>,
    /// Per fill: a visible solid fill.
    fills: Vec<bool>,
    /// Per border: (rule above, rule below).
    borders: Vec<(bool, bool)>,
    /// Which of fonts, fills, borders and cellXfs the part had.
    seen: [bool; 4],
}

impl Styles {
    /// The format code of the cell style with this index (the `s` attribute of a cell).
    /// Custom formats come first, then the built-in ids (en-US). `None` when the index is
    /// out of range or the id is neither defined nor built in.
    pub fn number_format(&self, style_index: u32) -> Option<&str> {
        let id = *self.xf_ids.get(style_index as usize)?;
        match self.custom.get(&id) {
            Some(code) => Some(code.as_str()),
            None => numfmt::builtin(id),
        }
    }

    /// The number format id of the cell style with this index, `None` when the index is out
    /// of range. Tells a style without a known code (a locale-dependent built-in id) from a
    /// missing style.
    pub fn format_id(&self, style_index: u32) -> Option<u32> {
        self.xf_ids.get(style_index as usize).copied()
    }

    /// The look of the cell style with this index. `None` when the index is out of range.
    /// Bold, fill and borders come from the entry itself, else from its named cell style
    /// (`xfId` in `cellStyleXfs`), else from font, fill or border 0. Only the result counts.
    pub fn look(&self, style_index: u32) -> Option<Look> {
        let xf = self.xfs.get(style_index as usize)?;
        let named = self.style_xfs.get(xf.xf_id as usize);
        let pick = |own: u32, from_named: Option<u32>| {
            if own != NONE {
                own
            } else {
                from_named.filter(|v| *v != NONE).unwrap_or(0)
            }
        };
        let font = pick(xf.font, named.map(|n| n.font));
        let fill = pick(xf.fill, named.map(|n| n.fill));
        let border = pick(xf.border, named.map(|n| n.border));
        let (rule_above, rule_below) = self
            .borders
            .get(border as usize)
            .copied()
            .unwrap_or_default();
        Some(Look {
            bold: self.fonts.get(font as usize).copied().unwrap_or(false),
            filled: self.fills.get(fill as usize).copied().unwrap_or(false),
            indent: xf.indent,
            center_across: xf.center_across,
            rule_above,
            rule_below,
        })
    }

    /// Whether the styles part had fonts, fills, borders and cellXfs. When it did not, the
    /// looks are missing, which is not the same as plain.
    pub fn has_looks(&self) -> bool {
        self.seen.iter().all(|s| *s)
    }

    /// The number of cell styles.
    pub fn len(&self) -> usize {
        self.xf_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.xf_ids.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    NumFmts,
    Fonts,
    Fills,
    Borders,
    StyleXfs,
    CellXfs,
    /// Anything else, for example `dxfs`, whose fonts, fills and borders are not cell styles.
    Other,
}

/// Reads the styles part in one streaming pass. Every list grows by one small entry per
/// element, so the part reader's item limit bounds them all.
pub(super) fn read_styles(ctx: &mut Ctx<'_, '_>, part: &str) -> Styles {
    let mut styles = Styles::default();
    let mut section = Section::None;
    // Inside a `border` or `patternFill`, to place the edge and color children.
    let mut in_pattern_solid = false;
    ctx.scan(part, true, &mut |ev, depth| match ev {
        Ev::Start(e) | Ev::Empty(e) => {
            let is_start = matches!(ev, Ev::Start(_));
            let name = e.name.as_str();
            // Sections are direct children of the root.
            if depth == 2 {
                section = match name {
                    "numFmts" => Section::NumFmts,
                    "fonts" => {
                        styles.seen[0] = true;
                        Section::Fonts
                    }
                    "fills" => {
                        styles.seen[1] = true;
                        Section::Fills
                    }
                    "borders" => {
                        styles.seen[2] = true;
                        Section::Borders
                    }
                    "cellStyleXfs" => Section::StyleXfs,
                    "cellXfs" => {
                        styles.seen[3] = true;
                        Section::CellXfs
                    }
                    _ => Section::Other,
                };
                if !is_start {
                    section = Section::None;
                }
                return;
            }
            match (section, name) {
                (Section::NumFmts, "numFmt") => {
                    if let (Some(id), Some(code)) = (
                        e.attr("numFmtId").and_then(|v| v.parse().ok()),
                        e.attr("formatCode"),
                    ) {
                        styles.custom.insert(id, code.to_string());
                    }
                }
                (Section::CellXfs, "xf") => {
                    styles
                        .xf_ids
                        .push(e.attr("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0));
                    styles.xfs.push(Xf::from_elem(e));
                }
                (Section::StyleXfs, "xf") => styles.style_xfs.push(Xf::from_elem(e)),
                (Section::CellXfs | Section::StyleXfs, "alignment") => {
                    let list = if section == Section::CellXfs {
                        &mut styles.xfs
                    } else {
                        &mut styles.style_xfs
                    };
                    if let Some(xf) = list.last_mut() {
                        xf.indent = e
                            .attr("indent")
                            .and_then(|v| v.parse::<u64>().ok())
                            .map_or(0, |v| v.min(255) as u8);
                        xf.center_across = e.attr("horizontal") == Some("centerContinuous");
                    }
                }
                (Section::Fonts, "font") => styles.fonts.push(false),
                (Section::Fonts, "b") => {
                    let on = !matches!(e.attr("val"), Some("0" | "false"));
                    if let Some(b) = styles.fonts.last_mut() {
                        *b = on;
                    }
                }
                (Section::Fills, "fill") => {
                    styles.fills.push(false);
                    in_pattern_solid = false;
                }
                (Section::Fills, "patternFill") => {
                    in_pattern_solid = e.attr("patternType") == Some("solid");
                }
                (Section::Fills, "fgColor") if in_pattern_solid => {
                    if let Some(f) = styles.fills.last_mut() {
                        *f = is_visible_color(e);
                    }
                }
                (Section::Borders, "border") => styles.borders.push((false, false)),
                (Section::Borders, "top") => {
                    if let Some(b) = styles.borders.last_mut() {
                        b.0 = is_rule(e.attr("style"));
                    }
                }
                (Section::Borders, "bottom") => {
                    if let Some(b) = styles.borders.last_mut() {
                        b.1 = is_rule(e.attr("style"));
                    }
                }
                _ => {}
            }
        }
        Ev::End(_) if depth == 2 => section = Section::None,
        _ => {}
    });
    styles
}
