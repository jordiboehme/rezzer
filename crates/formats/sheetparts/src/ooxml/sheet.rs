// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use zipguard::Archive;

use super::comments::{read_notes, read_threads};
use super::objects::{PivotSources, read_charts, read_pivot, read_table};
use super::workbook::{SheetInfo, WorkbookParts};
use super::{Ctx, Rel};
use crate::xml::{Elem, Ev};
use crate::{CellPos, Issue, IssueKind, Limits, RowSpan};

/// What a formula element says beyond its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrKind {
    /// Part of a shared formula group. The master cell carries the text and the range.
    Shared { index: u32 },
    /// An array formula (also the cells inside the range that carry an empty `f`).
    Array,
    /// An array formula whose cell metadata (`cm`) marks it as a dynamic array.
    DynamicArray,
    /// A what-if data table.
    DataTable {
        r1: Option<String>,
        r2: Option<String>,
        two_d: bool,
        /// The one input is a row input cell (`dtr`).
        row_input: bool,
    },
}

/// A formula that is more than a plain formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaAttr {
    pub at: CellPos,
    pub kind: AttrKind,
    /// The `ref` attribute as written.
    pub range: Option<String>,
    /// The text of the `f` element, `Some("")` when the element is empty.
    pub text: Option<String>,
}

/// A data validation rule. `operator` is as written; `None` means the default (between).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    /// The `sqref` as written, possibly several ranges separated by spaces.
    pub range: String,
    /// `whole`, `decimal`, `list`, `date`, `time`, `textLength`, `custom` or `none`.
    pub kind: String,
    pub operator: Option<String>,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
    /// The entries of a list that is written out in the rule (`"a,b,c"`).
    pub list: Option<Vec<String>>,
    pub prompt: Option<String>,
    pub error: Option<String>,
}

/// One rule of a conditional format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CondFormat {
    pub range: String,
    pub kind: String,
    pub operator: Option<String>,
    pub formulas: Vec<String>,
    pub priority: u32,
}

/// A legacy note (the yellow sticky kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub cell: String,
    pub author: Option<String>,
    pub text: String,
}

/// One entry of a threaded comment. A reply has a `parent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub cell: String,
    pub id: String,
    pub parent: Option<String>,
    /// The display name from the person list.
    pub person: Option<String>,
    pub date: Option<String>,
    pub text: String,
    pub done: bool,
}

/// An Excel table (list object).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub range: String,
    pub columns: Vec<String>,
    /// Header rows at the top (`headerRowCount`, 1 when not written).
    pub header_rows: u32,
    /// Totals rows at the bottom (`totalsRowCount`, 0 when not written).
    pub totals_rows: u32,
}

/// A pivot table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pivot {
    pub name: String,
    /// The cell range of the pivot table on its sheet.
    pub location: String,
    /// Where the data comes from, for example `Sheet1!A1:J11` or a table name.
    pub source: Option<String>,
    /// Offsets inside `location` (0-based): the first row of the column headers, the first
    /// row of data values and the first column of data values. 0 when not written.
    pub first_header_row: u32,
    pub first_data_row: u32,
    pub first_data_col: u32,
    /// `rowGrandTotals`: grand totals for the rows. The band drawn is the grand total COLUMN
    /// at the right edge of the table (each row gets a total). Defaults to true.
    pub row_grand_totals: bool,
    /// `colGrandTotals`: grand totals for the columns. The band drawn is the grand total ROW
    /// at the bottom edge of the table (each column gets a total). Defaults to true.
    pub col_grand_totals: bool,
    /// The number of fields on the row axis and on the column axis (the `field` children of
    /// `rowFields` and `colFields`, 0 when absent; the `count` attribute is not trusted). A column axis may include the values
    /// pseudo field.
    pub row_fields: u32,
    pub col_fields: u32,
}

/// Rows and columns fixed at the top and left of a sheet by a frozen pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frozen {
    pub rows: u32,
    pub cols: u32,
}

/// A chart, one per plot element (a combination chart gives several).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chart {
    /// `bar`, `line`, `pie`, `scatter`, `area` and so on.
    pub kind: String,
    pub title: Option<String>,
    /// For each series: its name and its values range. The name is the cell reference when the
    /// series name comes from a cell, otherwise the literal text.
    pub series: Vec<(Option<String>, String)>,
}

/// What the sheet part and its related parts say beyond the cell values.
#[derive(Debug, Clone, Default)]
pub struct SheetParts {
    pub hidden_rows: Vec<RowSpan>,
    pub hidden_cols: Vec<RowSpan>,
    /// Rows with an outline level above 0: (row, level).
    pub outline_rows: Vec<(u32, u8)>,
    /// Cells with a style other than the default.
    pub cell_styles: Vec<(CellPos, u32)>,
    pub formula_attrs: Vec<FormulaAttr>,
    pub validations: Vec<Validation>,
    pub cond_formats: Vec<CondFormat>,
    pub notes: Vec<Note>,
    pub threads: Vec<Thread>,
    pub tables: Vec<TableInfo>,
    pub pivots: Vec<Pivot>,
    pub charts: Vec<Chart>,
    /// The `ref` of the sheet's autoFilter.
    pub auto_filter: Option<String>,
    /// The frozen pane of the first view, `None` when the pane is split, not frozen or absent.
    pub frozen: Option<Frozen>,
    pub issues: Vec<Issue>,
}

/// Parses an A1 reference (`B5`, `$B$5`) into a 0-based position.
pub(crate) fn parse_a1(text: &str) -> Option<CellPos> {
    let text = text.replace('$', "");
    let letters = text.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let (l, d) = text.split_at(letters);
    if d.is_empty() || d.len() > 7 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut col = 0u32;
    for c in l.bytes() {
        col = col * 26 + u32::from(c.to_ascii_uppercase() - b'A' + 1);
    }
    let row: u32 = d.parse().ok()?;
    if !(1..=16384).contains(&col) || !(1..=1_048_576).contains(&row) {
        return None;
    }
    Some(CellPos {
        row: row - 1,
        col: col - 1,
    })
}

fn push_span(spans: &mut Vec<RowSpan>, first: u32, last: u32) {
    if let Some(prev) = spans.last_mut()
        && first <= prev.last.saturating_add(1)
        && last >= prev.first
    {
        prev.last = prev.last.max(last);
        return;
    }
    spans.push(RowSpan { first, last });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    None,
    Formula,
    Formula1,
    Formula2,
    Cf,
}

struct PendingF {
    kind: Option<(AttrKind, Option<String>)>,
    at: CellPos,
    cm: Option<u32>,
}

struct Scan<'p> {
    part: &'p str,
    out: SheetParts,
    extra: Vec<Issue>,
    in_sheet_data: bool,
    ext_depth: usize,
    next_row: u32,
    next_col: u32,
    at: Option<CellPos>,
    cm: Option<u32>,
    pending_f: Option<PendingF>,
    dynamic_fixups: Vec<(usize, u32)>,
    target: Target,
    buf: String,
    dv: Option<Validation>,
    cf: Option<(CondFormat, String)>,
    cf_range: String,
    warned_ext: [bool; 2],
    in_custom_views: bool,
    views: u32,
}

impl Scan<'_> {
    fn event(&mut self, ev: Ev<'_>) {
        match ev {
            Ev::Start(e) => self.start(e),
            Ev::Empty(e) => {
                self.start(e);
                self.end(&e.name);
            }
            Ev::End(name) => self.end(name),
            Ev::Text(t) => {
                if self.target != Target::None {
                    self.buf.push_str(t);
                }
            }
        }
    }

    fn start(&mut self, e: &Elem) {
        let name = e.name.as_str();
        if name == "extLst" {
            self.ext_depth += 1;
        }
        if self.ext_depth > 0 {
            let which = match name {
                "conditionalFormatting" => Some((0, "conditional formats")),
                "dataValidation" => Some((1, "data validations")),
                _ => None,
            };
            if let Some((i, what)) = which
                && !self.warned_ext[i]
            {
                self.warned_ext[i] = true;
                self.extra.push(Issue::new(
                    self.part,
                    IssueKind::Unsupported,
                    format!("{what} in the extension list are not read"),
                ));
            }
            return;
        }
        match name {
            "sheetData" => self.in_sheet_data = true,
            "customSheetViews" => self.in_custom_views = true,
            "autoFilter" if !self.in_custom_views && self.out.auto_filter.is_none() => {
                self.out.auto_filter = e.attr("ref").map(str::to_string);
            }
            "sheetView" if !self.in_custom_views => self.views += 1,
            "pane" if !self.in_custom_views && self.views == 1 => {
                // `frozenSplit` is a split pane that was then frozen (ECMA-376: frozen split).
                if matches!(e.attr("state"), Some("frozen" | "frozenSplit")) {
                    let split = |key: &str, max: u32| {
                        e.attr(key)
                            .and_then(|v| v.trim().parse::<f64>().ok())
                            .filter(|v| *v >= 0.0)
                            .map_or(0, |v| (v as u32).min(max))
                    };
                    let rows = split("ySplit", 1_048_576);
                    let cols = split("xSplit", 16_384);
                    if rows > 0 || cols > 0 {
                        self.out.frozen = Some(Frozen { rows, cols });
                    }
                }
            }
            "col" => {
                if e.flag("hidden") {
                    let min = e.attr("min").and_then(|v| v.parse::<u32>().ok());
                    let max = e.attr("max").and_then(|v| v.parse::<u32>().ok());
                    match (min, max) {
                        (Some(min), Some(max)) if (1..=16384).contains(&min) && max >= min => {
                            push_span(&mut self.out.hidden_cols, min - 1, max.min(16384) - 1)
                        }
                        _ => self.bad_ref("a hidden column span with a column number out of range"),
                    }
                }
            }
            "row" if self.in_sheet_data => {
                let valid = match e.attr("r") {
                    None => Some(self.next_row).filter(|r| *r < 1_048_576),
                    Some(v) => v
                        .parse::<u32>()
                        .ok()
                        .filter(|r| (1..=1_048_576).contains(r))
                        .map(|r| r - 1),
                };
                let row = valid.unwrap_or(self.next_row);
                self.next_row = row.saturating_add(1);
                self.next_col = 0;
                if valid.is_none() {
                    self.bad_ref("a row with a row number out of range");
                } else {
                    if e.flag("hidden") {
                        push_span(&mut self.out.hidden_rows, row, row);
                    }
                    if let Some(level) = e.attr("outlineLevel").and_then(|v| v.parse::<u8>().ok())
                        && level > 0
                    {
                        self.out.outline_rows.push((row, level));
                    }
                }
            }
            "c" if self.in_sheet_data => {
                let row = self.next_row.saturating_sub(1);
                let at = match e.attr("r") {
                    Some(r) => parse_a1(r),
                    None => Some(CellPos {
                        row,
                        col: self.next_col,
                    }),
                };
                match at {
                    Some(at) => {
                        self.next_col = at.col.saturating_add(1);
                        self.at = Some(at);
                        if let Some(s) = e.attr("s").and_then(|v| v.parse::<u32>().ok())
                            && s != 0
                        {
                            self.out.cell_styles.push((at, s));
                        }
                    }
                    None => {
                        self.at = None;
                        self.bad_ref("a cell with a bad reference");
                    }
                }
                self.cm = e.attr("cm").and_then(|v| v.parse().ok());
            }
            "f" if self.in_sheet_data => {
                if let Some(at) = self.at {
                    let range = e.attr("ref").map(str::to_string);
                    let kind = match e.attr("t") {
                        Some("shared") => Some((
                            AttrKind::Shared {
                                index: e.attr("si").and_then(|v| v.parse().ok()).unwrap_or(0),
                            },
                            range,
                        )),
                        Some("array") => Some((AttrKind::Array, range)),
                        Some("dataTable") => Some((
                            AttrKind::DataTable {
                                r1: e.attr("r1").map(str::to_string),
                                r2: e.attr("r2").map(str::to_string),
                                two_d: e.flag("dt2D"),
                                row_input: e.flag("dtr"),
                            },
                            range,
                        )),
                        _ => Some((AttrKind::Array, None)),
                    };
                    let untyped = !matches!(e.attr("t"), Some("shared" | "array" | "dataTable"));
                    self.pending_f = Some(PendingF {
                        kind: if untyped { None } else { kind },
                        at,
                        cm: self.cm,
                    });
                    self.target = Target::Formula;
                    self.buf.clear();
                }
            }
            "dataValidation" => {
                self.dv = Some(Validation {
                    range: e.attr("sqref").unwrap_or("").to_string(),
                    kind: e.attr("type").unwrap_or("none").to_string(),
                    operator: e.attr("operator").map(str::to_string),
                    formula1: None,
                    formula2: None,
                    list: None,
                    prompt: e.attr("prompt").map(str::to_string),
                    error: e.attr("error").map(str::to_string),
                });
            }
            "formula1" | "formula2" if self.dv.is_some() => {
                self.target = if name == "formula1" {
                    Target::Formula1
                } else {
                    Target::Formula2
                };
                self.buf.clear();
            }
            "conditionalFormatting" => {
                self.cf_range = e.attr("sqref").unwrap_or("").to_string();
            }
            "cfRule" => {
                self.cf = Some((
                    CondFormat {
                        range: self.cf_range.clone(),
                        kind: e.attr("type").unwrap_or("").to_string(),
                        operator: e.attr("operator").map(str::to_string),
                        formulas: Vec::new(),
                        priority: e.attr("priority").and_then(|v| v.parse().ok()).unwrap_or(0),
                    },
                    String::new(),
                ));
            }
            "formula" if self.cf.is_some() => {
                self.target = Target::Cf;
                self.buf.clear();
            }
            _ => {}
        }
    }

    fn end(&mut self, name: &str) {
        if name == "extLst" {
            self.ext_depth = self.ext_depth.saturating_sub(1);
            return;
        }
        if self.ext_depth > 0 {
            return;
        }
        match name {
            "sheetData" => self.in_sheet_data = false,
            "customSheetViews" => self.in_custom_views = false,
            "c" => {
                self.at = None;
                self.cm = None;
            }
            "f" => {
                if let Some(p) = self.pending_f.take() {
                    let text = std::mem::take(&mut self.buf);
                    self.target = Target::None;
                    let kind = match p.kind {
                        Some(k) => Some(k),
                        // An `f` without a type and without text is an array cell whose
                        // formula sits in the first cell of its range.
                        None if text.trim().is_empty() => Some((AttrKind::Array, None)),
                        None => None,
                    };
                    if let Some((kind, range)) = kind {
                        if let (AttrKind::Array, Some(cm)) = (&kind, p.cm) {
                            self.dynamic_fixups.push((self.out.formula_attrs.len(), cm));
                        }
                        self.out.formula_attrs.push(FormulaAttr {
                            at: p.at,
                            kind,
                            range,
                            text: Some(text),
                        });
                    }
                }
            }
            "formula1" | "formula2" => {
                let text = std::mem::take(&mut self.buf);
                self.target = Target::None;
                if let Some(dv) = self.dv.as_mut() {
                    if name == "formula1" {
                        dv.formula1 = Some(text);
                    } else {
                        dv.formula2 = Some(text);
                    }
                }
            }
            "dataValidation" => {
                if let Some(mut dv) = self.dv.take() {
                    if dv.kind == "list"
                        && let Some(f) = dv.formula1.as_deref()
                    {
                        dv.list = literal_list(f);
                    }
                    self.out.validations.push(dv);
                }
            }
            "formula" if self.target == Target::Cf => {
                let text = std::mem::take(&mut self.buf);
                self.target = Target::None;
                if let Some((cf, _)) = self.cf.as_mut() {
                    cf.formulas.push(text);
                }
            }
            "cfRule" => {
                if let Some((cf, _)) = self.cf.take() {
                    self.out.cond_formats.push(cf);
                }
            }
            _ => {}
        }
    }

    fn bad_ref(&mut self, what: &str) {
        if !self.extra.iter().any(|i| i.what == what) {
            self.extra
                .push(Issue::new(self.part, IssueKind::Damaged, what));
        }
    }
}

/// `"a,b,c"` as a list of entries. Anything that is not one quoted literal is a reference or
/// a formula and gives `None`.
fn literal_list(formula: &str) -> Option<Vec<String>> {
    let inner = formula.strip_prefix('"')?.strip_suffix('"')?;
    if inner.replace("\"\"", "").contains('"') {
        return None;
    }
    let inner = inner.replace("\"\"", "\"");
    Some(inner.split(',').map(str::to_string).collect())
}

fn dynamic_cells(ctx: &mut Ctx<'_, '_>, part: &str) -> Vec<bool> {
    #[derive(PartialEq)]
    enum Section {
        None,
        Types,
        Future,
        Cells,
    }
    let mut section = Section::None;
    let mut types: Vec<String> = Vec::new();
    let mut futures: Vec<(String, Vec<bool>)> = Vec::new();
    let mut cells: Vec<Vec<(usize, usize)>> = Vec::new();
    ctx.scan(part, true, &mut |ev, _| match ev {
        Ev::Start(e) | Ev::Empty(e) => match e.name.as_str() {
            "metadataTypes" => section = Section::Types,
            "futureMetadata" => {
                section = Section::Future;
                futures.push((e.attr("name").unwrap_or("").to_string(), Vec::new()));
            }
            "cellMetadata" => section = Section::Cells,
            "metadataType" if section == Section::Types => {
                types.push(e.attr("name").unwrap_or("").to_string())
            }
            "bk" if section == Section::Future => {
                if let Some(f) = futures.last_mut() {
                    f.1.push(false);
                }
            }
            "bk" if section == Section::Cells => cells.push(Vec::new()),
            "dynamicArrayProperties" if section == Section::Future => {
                if let Some(last) = futures.last_mut().and_then(|f| f.1.last_mut()) {
                    *last = e.flag("fDynamic");
                }
            }
            "rc" if section == Section::Cells => {
                let t = e.attr("t").and_then(|v| v.parse().ok()).unwrap_or(0);
                let v = e
                    .attr("v")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(usize::MAX);
                if let Some(c) = cells.last_mut() {
                    c.push((t, v));
                }
            }
            _ => {}
        },
        Ev::End("metadataTypes" | "futureMetadata" | "cellMetadata") => section = Section::None,
        _ => {}
    });
    cells
        .iter()
        .map(|rcs| {
            rcs.iter().any(|&(t, v)| {
                t.checked_sub(1)
                    .and_then(|i| types.get(i))
                    .is_some_and(|name| {
                        futures
                            .iter()
                            .filter(|f| &f.0 == name)
                            .any(|f| f.1.get(v).copied().unwrap_or(false))
                    })
            })
        })
        .collect()
}

fn workbook_rels(ctx: &mut Ctx<'_, '_>) -> Vec<Rel> {
    let root = ctx.rels("");
    let part = root
        .iter()
        .find(|r| r.is("/officeDocument") && !r.external)
        .map(|r| r.target.clone())
        .unwrap_or_else(|| "xl/workbook.xml".to_string());
    ctx.rels(&part)
}

/// Reads the logic of one sheet: hidden rows and columns, formula attributes, validations,
/// conditional formats, notes, threaded comments, tables, pivot tables and charts.
///
/// The sheet XML is streamed once. `sheetData` cells are skipped except for their formula
/// attributes and non-default styles. Problems end up in `issues`.
pub fn read_sheet(
    zip: &mut Archive,
    sheet: &SheetInfo,
    wb: &WorkbookParts,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> SheetParts {
    let mut ctx = Ctx {
        zip,
        limits,
        stop,
        issues: Vec::new(),
    };
    let part = sheet.part.clone();
    if part.is_empty() {
        ctx.issues.push(Issue::new(
            "xl/workbook.xml",
            IssueKind::Damaged,
            format!("sheet {} has no part", sheet.name),
        ));
        return SheetParts {
            issues: ctx.issues,
            ..SheetParts::default()
        };
    }

    let mut scan = Scan {
        part: &part,
        out: SheetParts::default(),
        extra: Vec::new(),
        in_sheet_data: false,
        ext_depth: 0,
        next_row: 0,
        next_col: 0,
        at: None,
        cm: None,
        pending_f: None,
        dynamic_fixups: Vec::new(),
        target: Target::None,
        buf: String::new(),
        dv: None,
        cf: None,
        cf_range: String::new(),
        warned_ext: [false; 2],
        in_custom_views: false,
        views: 0,
    };
    ctx.scan(&part, true, &mut |ev, _| scan.event(ev));
    let Scan {
        mut out,
        extra,
        dynamic_fixups,
        ..
    } = scan;
    ctx.issues.extend(extra);

    if !dynamic_fixups.is_empty() {
        let rels = workbook_rels(&mut ctx);
        let meta = rels
            .iter()
            .find(|r| r.is("/sheetMetadata") && !r.external)
            .map(|r| r.target.clone())
            .unwrap_or_else(|| "xl/metadata.xml".to_string());
        let dynamic = dynamic_cells(&mut ctx, &meta);
        for (index, cm) in dynamic_fixups {
            let is_dynamic = cm
                .checked_sub(1)
                .and_then(|i| dynamic.get(i as usize))
                .copied()
                .unwrap_or(false);
            if is_dynamic && let Some(a) = out.formula_attrs.get_mut(index) {
                a.kind = AttrKind::DynamicArray;
            }
        }
    }

    let rels = ctx.rels(&part);
    let has_threads = rels.iter().any(|r| r.is("/threadedComment") && !r.external);
    let mut pivots = PivotSources::default();
    for rel in rels.iter().filter(|r| !r.external) {
        if rel.is("/comments") {
            out.notes
                .extend(read_notes(&mut ctx, &rel.target, has_threads));
        } else if rel.is("/threadedComment") {
            out.threads
                .extend(read_threads(&mut ctx, &rel.target, &wb.persons));
        } else if rel.is("/table") {
            out.tables.extend(read_table(&mut ctx, &rel.target));
        } else if rel.is("/pivotTable") {
            out.pivots
                .extend(read_pivot(&mut ctx, &rel.target, &mut pivots));
        } else if rel.is("/drawing") {
            out.charts.extend(read_charts(&mut ctx, &rel.target));
        }
    }
    out.issues = ctx.issues;
    out
}
