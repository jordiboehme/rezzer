// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout of a sheet: its cells, merges, facts and signals as `sheetlayout` input, the
//! detection, and the regions mapped into blocks by `rezzer_layout`. Past the input builders
//! this is the same for every format.

use std::collections::HashMap;

use rezzer_layout::{Caps, CellText, ColumnNames, Mapped, NameCaption};
use rezzer_limits::Budget;
use rezzer_model::{Block, CellArea, Loss, LossReason, Severity, locate_cells};
use sheetlayout::{
    CellIn, Class, Fact, FactKind, FormulaIn, Input, Kind, NameIn, Pos, Rect, Style,
};
use sheetparts::{DefinedName, Look};

use crate::grid::{self, RawCell, Val};
use crate::logic;

/// Empty cells with center across selection passed to the detector per sheet, at most. They
/// only tell how far a text to their left spreads.
const MAX_CENTER_CELLS: usize = 100_000;

/// What a sheet says about its layout beyond its cells.
#[derive(Debug, Default)]
pub(crate) struct Signals {
    pub facts: Vec<Fact>,
    pub names: Vec<NameIn>,
    pub outline: Vec<(u32, u32, u8)>,
    pub frozen: Option<(u32, u32)>,
    pub print_titles: Option<(u32, u32)>,
    /// Whether the style flags were read at all.
    pub styles_read: bool,
}

/// One cell as the detector and the tables see it.
pub(crate) struct Shown {
    pub text: String,
    pub class: Class,
    pub number: Option<f64>,
}

/// The layout of one sheet, mapped into blocks, the number of merged areas ignored because
/// they overlap one before them, and what names the cells of its regions.
pub(crate) struct SheetLayout {
    pub mapped: Mapped,
    pub merges_ignored: u64,
    pub paths: SheetPaths,
}

impl SheetLayout {
    /// Puts the blocks of the sheet after `blocks` (a note when the sheet is empty) and its
    /// losses into `out`, with one for merged areas that were ignored. Gives the parameter
    /// paths and whether the element limit was reached while the tables were built.
    pub(crate) fn place(
        self,
        q: &str,
        blocks: &mut Vec<Block>,
        out: &mut crate::Out,
    ) -> (SheetPaths, bool) {
        if self.mapped.blocks.is_empty() {
            blocks.push(grid::note("The sheet is empty."));
        }
        blocks.extend(self.mapped.blocks);
        for l in self.mapped.losses {
            out.loss(l.location, l.what, l.why, l.severity);
        }
        if self.merges_ignored > 0 {
            out.loss(
                q,
                format!(
                    "{} merged areas overlap another one or are too large; shown as single cells",
                    self.merges_ignored
                ),
                LossReason::Damaged,
                Severity::Info,
            );
        }
        (self.paths, self.mapped.elements_out)
    }
}

/// What names the data cells of one region (spec 8.7): the author's heading texts above
/// them, the header path of their column and the label of their row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PathRegion {
    /// The whole region, with its caption, header and note rows.
    pub rect: Rect,
    /// The data cells: the rows of the data area in the table's columns.
    pub area: Rect,
    /// The author headings the region's table stands under, wide to narrow: caption
    /// headings, defined names standing as captions, section headings. Never a generated
    /// heading.
    pub base: Vec<String>,
    /// The header path of each column from `area.left`, parts joined with " / ".
    pub header: Vec<String>,
    /// The left columns of `area` that name the rows.
    pub label_cols: u32,
    /// The texts of the label columns in the data rows, by row, then column.
    pub labels: Vec<(u32, u32, String)>,
}

/// The regions of one sheet that can name a parameter.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SheetPaths {
    pub regions: Vec<PathRegion>,
}

impl SheetPaths {
    /// The paths of the regions of `layout`, mapped into `blocks`; `texts` are the sheet's
    /// shown texts, sorted. A region's base is what its Markdown stands under: the author
    /// headings (caption headings, defined names standing as captions, section headings)
    /// open above its table. A generated heading closes the deeper ones and adds no part.
    pub fn of(layout: &sheetlayout::Layout, texts: &[CellText], blocks: &[Block]) -> Self {
        let mut out = Vec::new();
        let mut by_range: HashMap<String, usize> = HashMap::new();
        for r in &layout.regions {
            let Some(data) = r.data else {
                continue;
            };
            let (left, right, label_cols, header) = match r.kind {
                Kind::KeyValue => (data.left, data.right, 1, Vec::new()),
                Kind::Table => {
                    let cols = r.body.unwrap_or(data);
                    (cols.left, cols.right, r.label_cols, r.header_paths.clone())
                }
                Kind::Grid | Kind::Text => (data.left, data.right, 0, Vec::new()),
                Kind::Title => continue,
            };
            by_range.insert(r.rect.a1(), out.len());
            out.push(PathRegion {
                rect: r.rect,
                area: Rect {
                    top: data.top,
                    left,
                    bottom: data.bottom,
                    right,
                },
                base: Vec::new(),
                header,
                label_cols,
                labels: Vec::new(),
            });
        }
        for (range, headings) in rezzer_model::table_headings(blocks) {
            if let Some(&k) = by_range.get(range) {
                out[k].base = headings
                    .into_iter()
                    .filter(|t| logic::is_label(t))
                    .collect();
            }
        }
        // The label texts, each cell located once.
        let points: Vec<(u32, u32)> = texts.iter().map(|t| (t.row, t.col)).collect();
        let mut paths = SheetPaths { regions: out };
        for (t, at) in texts.iter().zip(paths.locate(&points, false)) {
            let Some(k) = at else {
                continue;
            };
            let region = &mut paths.regions[k];
            if t.col < region.area.left.saturating_add(region.label_cols) {
                region.labels.push((t.row, t.col, t.text.clone()));
            }
        }
        paths
    }

    /// The region whose data area (`whole`: whole rectangle) holds each `(row, column)`
    /// point. Regions never overlap.
    pub fn locate(&self, points: &[(u32, u32)], whole: bool) -> Vec<Option<usize>> {
        let areas: Vec<CellArea> = self
            .regions
            .iter()
            .map(|r| {
                let a = if whole { r.rect } else { r.area };
                CellArea {
                    top: a.top,
                    left: a.left,
                    bottom: a.bottom,
                    right: a.right,
                }
            })
            .collect();
        locate_cells(&areas, points)
    }

    /// The narrow end of the path of the data cell at `row`, `col` of region `k`, without
    /// the region's base: the parts of its column's header path, each a label without its
    /// unit (the label rule of the parameters), and the texts its row has in the label
    /// columns left of it, as they stand. Both empty when neither names it.
    pub fn narrow_parts(&self, k: usize, row: u32, col: u32) -> (Vec<String>, Vec<&str>) {
        let r = &self.regions[k];
        let mut header = Vec::new();
        let i = (col - r.area.left) as usize;
        if let Some(path) = r.header.get(i).filter(|p| !p.is_empty()) {
            header.extend(path.split(" / ").filter_map(label_part));
        }
        let end = col.min(r.area.left.saturating_add(r.label_cols));
        let from = r.labels.partition_point(|l| l.0 < row);
        let labels = r.labels[from..]
            .iter()
            .take_while(|l| l.0 == row)
            .filter(|l| l.1 < end)
            .map(|l| l.2.trim())
            .filter(|t| logic::is_label(t))
            .collect();
        (header, labels)
    }

    /// Whether `col` is one of the label columns of region `k`.
    pub fn in_label_cols(&self, k: usize, col: u32) -> bool {
        let r = &self.regions[k];
        col < r.area.left.saturating_add(r.label_cols)
    }
}

/// A text as one path part: a label (the label rule) without the unit it names. A text that
/// is only a unit gives no part.
fn label_part(text: &str) -> Option<String> {
    logic::is_label(text)
        .then(|| crate::units::split_label(text).0)
        .flatten()
}

/// The style flags of a look; every flag is known.
pub(crate) fn style_of(look: Look) -> Style {
    Style {
        bold: Some(look.bold),
        filled: Some(look.filled),
        indent: Some(look.indent),
        center_across: Some(look.center_across),
        rule_above: Some(look.rule_above),
        rule_below: Some(look.rule_below),
    }
}

/// Detects the layout of one sheet and maps it into blocks. `cells` are sorted and each
/// position is there once; `shown` gives a cell's shown text and class, `style` its style
/// flags; `centered` lists empty positions with center across selection; `formulas` are
/// the formula facts of the sheet.
#[allow(clippy::too_many_arguments)]
pub(crate) fn map_sheet(
    sheet_q: &str,
    cells: &[RawCell],
    shown: &dyn Fn(&RawCell) -> Shown,
    style: &dyn Fn(u32, u32) -> Style,
    centered: &[(u32, u32)],
    merges: &[Rect],
    mut formulas: HashMap<(u32, u32), FormulaIn>,
    signals: Signals,
    caption_names: &[NameCaption],
    budget: &Budget,
) -> SheetLayout {
    let (merges, merges_ignored) = rezzer_layout::clean_merges(merges);
    let at: Vec<(u32, u32)> = cells.iter().map(|c| (c.row, c.col)).collect();
    let hidden = rezzer_layout::covered(&at, &merges);
    let mut texts = Vec::with_capacity(cells.len());
    let mut input_cells = Vec::with_capacity(cells.len());
    for (c, covered) in cells.iter().zip(hidden) {
        let s = shown(c);
        // A formula cell, even when its text was not kept or its result is empty.
        let has_formula = c.formula.is_some()
            || c.shared
            || c.shared_unknown
            || c.text_dropped
            || c.val == Val::Empty;
        if !covered {
            input_cells.push(CellIn {
                pos: Pos {
                    row: c.row,
                    col: c.col,
                },
                class: s.class,
                text: s.text.clone(),
                number: s.number,
                style: style(c.row, c.col),
                formula: has_formula.then(|| formulas.remove(&(c.row, c.col)).unwrap_or_default()),
            });
        }
        texts.push(CellText {
            row: c.row,
            col: c.col,
            text: s.text,
        });
    }
    // Empty cells with center across selection, where no cell and no merge is.
    let free = rezzer_layout::covered(centered, &merges);
    let mut extra: Vec<CellIn> = centered
        .iter()
        .zip(free)
        .filter(|(p, under)| !under && at.binary_search(p).is_err())
        .map(|(p, _)| p)
        .take(MAX_CENTER_CELLS + 1)
        .map(|&(row, col)| CellIn {
            pos: Pos { row, col },
            class: Class::Text,
            text: String::new(),
            number: None,
            style: style(row, col),
            formula: None,
        })
        .collect();
    let centered_cut = extra.len() > MAX_CENTER_CELLS;
    extra.truncate(MAX_CENTER_CELLS);
    if !extra.is_empty() {
        input_cells.append(&mut extra);
        input_cells.sort_by_key(|c| c.pos);
    }
    let input = Input {
        cells: input_cells,
        merges: merges.clone(),
        facts: signals.facts,
        names: signals.names,
        outline: signals.outline,
        frozen: signals.frozen,
        print_titles: signals.print_titles,
        header_guess: None,
        styles_read: signals.styles_read,
        is_label: &logic::is_label,
        is_unit_column: &logic::is_unit_column_texts,
    };
    let stop = || budget.check_time().is_err();
    let layout = sheetlayout::detect(&input, &rezzer_layout::limits(budget), &stop);
    drop(input);
    let mut caps = Caps::default();
    let mut mapped = rezzer_layout::blocks(
        sheet_q,
        &layout,
        &texts,
        &merges,
        ColumnNames::Letters,
        caption_names,
        &mut caps,
        budget,
    );
    if centered_cut {
        mapped.losses.push(Loss {
            location: sheet_q.to_string(),
            what: format!(
                "more than {MAX_CENTER_CELLS} empty cells centered across a selection; past them a text does not spread over its neighbours"
            ),
            why: LossReason::Limit,
            severity: Severity::Info,
        });
    }
    let paths = SheetPaths::of(&layout, &texts, &mapped.blocks);
    SheetLayout {
        mapped,
        merges_ignored,
        paths,
    }
}

/// A 0-based area as a rectangle.
pub(crate) fn rect_of(area: ((u32, u32), (u32, u32))) -> Rect {
    let ((top, left), (bottom, right)) = area;
    Rect {
        top,
        left,
        bottom,
        right,
    }
}

/// The rectangle a defined name refers to when its whole text is one area on `sheet`
/// (`Sheet1!$A$4:$F$62`, `'My sheet'!B2:C9`).
pub(crate) fn name_rect(refers_to: &str, sheet: &str) -> Option<Rect> {
    let text = refers_to.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    // The reference follows the last `!` outside quotes; a comma outside quotes makes a
    // list of areas.
    let mut quoted = false;
    let mut at = None;
    for (i, c) in text.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '!' if !quoted => at = Some(i),
            ',' | ' ' | '(' if !quoted => return None,
            _ => {}
        }
    }
    let at = at?;
    let (name, area) = (&text[..at], &text[at + 1..]);
    let name = match name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => name.to_string(),
    };
    if name.contains(['[', ':']) || name.to_lowercase() != sheet.to_lowercase() {
        return None;
    }
    grid::parse_area(area).map(rect_of)
}

/// The defined names that are one area on `sheet`, and the ones among them that may stand
/// as a caption (not hidden, not written by the application for itself).
/// A caption name carries the anchor the names facet gives it (`name:Prices`,
/// `name:'My sheet'!Local`).
pub(crate) fn sheet_names(names: &[DefinedName], sheet: &str) -> (Vec<NameIn>, Vec<NameCaption>) {
    let mut list = Vec::new();
    let mut captions = Vec::new();
    for n in names {
        let Some(rect) = name_rect(&n.refers_to, sheet) else {
            continue;
        };
        let builtin = n
            .name
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("_xlnm."));
        if !n.hidden && !builtin {
            let anchor = match &n.scope {
                Some(s) => format!("name:{}!{}", formulakit::quote_sheet(s), n.name),
                None => format!("name:{}", n.name),
            };
            captions.push(NameCaption {
                name: n.name.clone(),
                anchor,
            });
        }
        list.push(NameIn {
            name: n.name.clone(),
            rect,
            hidden: n.hidden,
            builtin,
        });
    }
    (list, captions)
}

/// Runs of rows with one outline level, from `(row, level)` pairs.
pub(crate) fn outline_runs(rows: &[(u32, u8)]) -> Vec<(u32, u32, u8)> {
    let mut sorted = rows.to_vec();
    sorted.sort_unstable();
    let mut out: Vec<(u32, u32, u8)> = Vec::new();
    for (row, level) in sorted {
        match out.last_mut() {
            Some(last) if last.2 == level && last.1.checked_add(1) == Some(row) => last.1 = row,
            _ => out.push((row, row, level)),
        }
    }
    out
}

/// An Excel table as a fact.
pub(crate) fn table_fact(
    area: ((u32, u32), (u32, u32)),
    name: &str,
    header_rows: u32,
    totals_rows: u32,
    columns: &[String],
) -> Fact {
    Fact {
        rect: rect_of(area),
        kind: FactKind::ListObject {
            name: name.to_string(),
            header_rows,
            totals_rows,
            columns: columns.to_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_on_this_sheet_only() {
        let r = |t: &str| name_rect(t, "NST-EST2023-POP");
        assert_eq!(
            r("'NST-EST2023-POP'!$A$4:$F$62"),
            Some(Rect {
                top: 3,
                left: 0,
                bottom: 61,
                right: 5
            })
        );
        assert_eq!(r("=nst-est2023-pop!A1").map(|x| x.a1()), Some("A1".into()));
        assert_eq!(r("Other!$A$1:$B$2"), None);
        assert_eq!(r("'NST-EST2023-POP'!A1:B2,'NST-EST2023-POP'!C1"), None);
        assert_eq!(r("[1]'NST-EST2023-POP'!A1"), None);
        assert_eq!(r("{1,2}"), None);
        assert_eq!(
            name_rect("'it''s'!B2:C3", "it's").map(|x| x.a1()),
            Some("B2:C3".into())
        );
    }

    #[test]
    fn caption_names_are_anchored_like_the_names_facet() {
        let name = |name: &str, scope: Option<&str>, refers_to: &str, hidden: bool| DefinedName {
            name: name.into(),
            scope: scope.map(str::to_string),
            refers_to: refers_to.into(),
            hidden,
            comment: None,
        };
        let names = [
            name("Prices", None, "'My sheet'!$A$1:$B$4", false),
            name("Local", Some("My sheet"), "'My sheet'!$C$1:$D$4", false),
            name("Secret", None, "'My sheet'!$E$1:$F$4", true),
            name(
                "_xlnm._FilterDatabase",
                Some("My sheet"),
                "'My sheet'!$A$1:$B$4",
                true,
            ),
        ];
        let (list, captions) = sheet_names(&names, "My sheet");
        assert_eq!(list.len(), 4);
        let shown: Vec<(&str, &str)> = captions
            .iter()
            .map(|c| (c.name.as_str(), c.anchor.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Prices", "name:Prices"),
                ("Local", "name:'My sheet'!Local")
            ]
        );
    }

    #[test]
    fn outline_rows_fold_into_runs() {
        assert_eq!(
            outline_runs(&[(5, 1), (2, 1), (3, 1), (4, 2), (7, 1)]),
            [(2, 3, 1), (4, 4, 2), (5, 5, 1), (7, 7, 1)]
        );
    }
}
