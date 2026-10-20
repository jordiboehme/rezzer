// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parts of an ODF spreadsheet (ods) that a spreadsheet crate does not expose: hidden
//! sheets, rows and columns, annotations with author and date, data validations, number
//! formats built from data styles, named ranges and the document metadata.
//!
//! Positions are 0-based like the rest of the crate. `number-rows-repeated` and
//! `number-columns-repeated` move the position by arithmetic and are never expanded into items.
//! A repeated cell that carries content (a value, a formula, a note) is listed per cell, and
//! those cells count against [`Limits::max_items`](crate::Limits). Counts that run past the
//! sheet size (1,048,576 rows, 16,384 columns) are clamped and reported as an [`Issue`].
//!
//! Nothing in the file is executed. The part is XML read through the bounded scanner.

mod rules;
mod styles;

use std::collections::{BTreeMap, HashMap};

use zipguard::Archive;

use crate::a1::a1;
use crate::ooxml::{Ctx, DocProps, Validation};
use crate::xml::{Elem, Ev};
use crate::{CellPos, DefinedName, Issue, IssueKind, Limits, PropValue, RowSpan};
use rules::{parse_condition, range_text};
use styles::StyleBook;

const CONTENT: &str = "content.xml";
const MAX_ROWS: u64 = 1_048_576;
const MAX_COLS: u64 = 16_384;

/// A note on a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// The A1 address of the cell.
    pub cell: String,
    pub author: Option<String>,
    /// The date as written in the file, for example `2024-09-18T18:27:00`.
    pub date: Option<String>,
    /// The paragraphs of the note, joined with newlines.
    pub text: String,
}

/// One sheet of an ODF spreadsheet, in tab order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdsSheet {
    pub name: String,
    /// The table style says `display="false"`. ODF has no "very hidden" state.
    pub hidden: bool,
    /// Rows with `visibility` `collapse` or `filter`, merged into runs.
    pub hidden_rows: Vec<RowSpan>,
    /// Columns with `visibility` `collapse` or `filter`, merged into runs. The `first` and
    /// `last` of a span are column numbers.
    pub hidden_cols: Vec<RowSpan>,
    pub annotations: Vec<Annotation>,
    /// Data validations. `range` is the A1 range of the cells that use the rule, several
    /// ranges separated by spaces. `kind` is `unknown` when the condition is not understood.
    pub validations: Vec<Validation>,
    /// The number format code of each cell with content whose style has a data style.
    pub cell_formats: Vec<(CellPos, String)>,
}

/// What `read` finds in an ODF spreadsheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OdsParts {
    pub sheets: Vec<OdsSheet>,
    /// Global names have no scope, sheet-local ones the sheet name. A named range gives
    /// `refers_to` as a sheet reference (`Sheet1!$A$1:$B$2`); a named expression keeps its
    /// expression as written (`of:=0.19`).
    pub names: Vec<DefinedName>,
    /// Core keys: `creator` (the first author), `lastModifiedBy`, `created`, `modified`,
    /// `title`, `subject`, `description`, `keywords`, `language`. App keys: `Application`,
    /// `EditingDuration`, `EditingCycles`.
    pub props: DocProps,
    pub issues: Vec<Issue>,
}

/// Reads the parts of an ODF spreadsheet. A problem becomes an issue; the other parts are
/// still read.
pub fn read(zip: &mut Archive<'_>, limits: &Limits, stop: &dyn Fn() -> bool) -> OdsParts {
    let mut ctx = Ctx {
        zip,
        limits,
        stop,
        issues: Vec::new(),
    };
    let mut book = StyleBook::default();
    ctx.scan("styles.xml", false, &mut |ev, _| book.event(&ev));

    let mut content = Content::new(book, limits.max_items as u64);
    ctx.scan(CONTENT, true, &mut |ev, depth| content.event(&ev, depth));
    let props = read_meta(&mut ctx);
    let mut issues = ctx.issues;
    issues.append(&mut content.issues);
    content.summary(&mut issues);
    OdsParts {
        sheets: content.sheets,
        names: content.names,
        props,
        issues,
    }
}

/// A rectangle of cells, 0-based and inclusive.
#[derive(Debug, Clone, Copy)]
struct Rect {
    r0: u32,
    r1: u32,
    c0: u32,
    c1: u32,
}

impl Rect {
    fn a1(self) -> String {
        if self.r0 == self.r1 && self.c0 == self.c1 {
            a1(self.r0, self.c0)
        } else {
            format!("{}:{}", a1(self.r0, self.c0), a1(self.r1, self.c1))
        }
    }
}

#[derive(Default)]
struct ValDef {
    condition: String,
    prompt: String,
    error: String,
    rects: Vec<Rect>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Msg {
    Help,
    Error,
}

struct ColDef {
    first: u32,
    last: u32,
    style: Option<String>,
}

#[derive(Default)]
struct AnnState {
    rect: Option<Rect>,
    open: bool,
    author: String,
    date: String,
    field: Option<bool>,
    paras: Vec<String>,
    para: Option<String>,
}

struct SheetState {
    sheet: OdsSheet,
    row: u64,
    col: u64,
    col_defs: u64,
    cols: Vec<ColDef>,
    in_row: bool,
    row_rep: u64,
    row_style: Option<String>,
    rows_clamped: bool,
    cols_clamped: bool,
    cell: Option<Rect>,
    ann: AnnState,
}

struct Content {
    book: StyleBook,
    sheets: Vec<OdsSheet>,
    names: Vec<DefinedName>,
    issues: Vec<Issue>,
    cur: Option<SheetState>,
    /// Tables below the top level (inside a cell) and tables of a wrong shape are not read.
    skip_tables: usize,
    used: u64,
    limit: u64,
    exhausted: bool,
    bad_counts: u64,
    first_bad: String,
    vals: Vec<ValDef>,
    val_index: HashMap<String, usize>,
    val_now: Option<usize>,
    msg: Option<Msg>,
    msg_para: Option<String>,
    unknown_vals: BTreeMap<String, u64>,
}

fn hidden(e: &Elem) -> bool {
    matches!(e.attr("visibility"), Some("collapse" | "filter"))
}

fn push_span(spans: &mut Vec<RowSpan>, first: u32, last: u32) {
    match spans.last_mut() {
        Some(prev) if prev.last.saturating_add(1) >= first => prev.last = prev.last.max(last),
        _ => spans.push(RowSpan { first, last }),
    }
}

impl Content {
    fn new(book: StyleBook, limit: u64) -> Content {
        Content {
            book,
            sheets: Vec::new(),
            names: Vec::new(),
            issues: Vec::new(),
            cur: None,
            skip_tables: 0,
            used: 0,
            limit,
            exhausted: false,
            bad_counts: 0,
            first_bad: String::new(),
            vals: Vec::new(),
            val_index: HashMap::new(),
            val_now: None,
            msg: None,
            msg_para: None,
            unknown_vals: BTreeMap::new(),
        }
    }

    /// Problems that can repeat a million times are said once with a count.
    fn summary(&self, issues: &mut Vec<Issue>) {
        if self.bad_counts > 0 {
            issues.push(Issue::new(
                CONTENT,
                IssueKind::Damaged,
                format!(
                    "{} repeat attributes are not counts, the first is {}: they count as 1",
                    self.bad_counts, self.first_bad
                ),
            ));
        }
        for (name, cells) in self.unknown_vals.iter().take(10) {
            issues.push(Issue::new(
                CONTENT,
                IssueKind::Damaged,
                format!("{cells} cells name the validation {name}, which is not defined"),
            ));
        }
        if self.unknown_vals.len() > 10 {
            issues.push(Issue::new(
                CONTENT,
                IssueKind::Damaged,
                format!(
                    "{} more validation names are not defined",
                    self.unknown_vals.len() - 10
                ),
            ));
        }
    }

    fn issue(&mut self, kind: IssueKind, what: impl Into<String>) {
        self.issues.push(Issue::new(CONTENT, kind, what));
    }

    /// Counts `n` listed cells against the item limit. False once the limit is hit.
    /// A repeat count, at least 1 and at most 4,194,304: more is past the sheet anyway, and the
    /// bound keeps all position arithmetic far from overflow. Bad values are counted and
    /// reported once at the end.
    fn repeat(&mut self, e: &Elem, key: &str) -> u64 {
        match e.attr(key) {
            None => 1,
            Some(v) => match v.trim().parse::<u64>() {
                Ok(n) => n.clamp(1, 1 << 22),
                Err(_) => {
                    self.bad_counts += 1;
                    if self.bad_counts == 1 {
                        self.first_bad = format!("{key}=\"{v}\"");
                    }
                    1
                }
            },
        }
    }

    fn charge(&mut self, n: u64) -> bool {
        if self.exhausted {
            return false;
        }
        self.used = self.used.saturating_add(n);
        if self.used > self.limit {
            self.exhausted = true;
            self.issue(
                IssueKind::Limit,
                format!(
                    "more than {} cells with content in repeated rows or columns: the rest is not listed",
                    self.limit
                ),
            );
            return false;
        }
        true
    }

    fn event(&mut self, ev: &Ev<'_>, depth: usize) {
        self.book.event(ev);
        match ev {
            Ev::Start(e) => self.open(e, depth, false),
            Ev::Empty(e) => self.open(e, depth, true),
            Ev::End(name) => self.close(name),
            Ev::Text(t) => self.text(t),
        }
    }

    fn open(&mut self, e: &Elem, depth: usize, empty: bool) {
        if e.name == "table" {
            if self.cur.is_none() && depth == 4 && self.skip_tables == 0 {
                self.start_sheet(e);
                if empty {
                    self.end_sheet();
                }
            } else if !empty {
                self.skip_tables += 1;
            }
            return;
        }
        if e.name == "named-range" || e.name == "named-expression" {
            self.named(e);
            return;
        }
        if self.skip_tables > 0 {
            return;
        }
        match e.name.as_str() {
            "content-validation" => {
                self.validation(e);
                return;
            }
            "help-message" | "error-message" => {
                self.msg = Some(if e.name == "help-message" {
                    Msg::Help
                } else {
                    Msg::Error
                });
                return;
            }
            "p" if self.msg.is_some() && !empty => {
                self.msg_para = Some(String::new());
                return;
            }
            _ => {}
        }
        if self.cur.is_none() {
            return;
        }
        if self.cur.as_ref().is_some_and(|s| s.ann.open) {
            self.annotation_event(e, empty);
            return;
        }
        match e.name.as_str() {
            "table-column" => self.column(e),
            "table-row" => {
                self.row(e);
                if empty {
                    self.end_row();
                }
            }
            "table-cell" | "covered-table-cell" => self.cell(e, empty),
            "annotation" if !empty => {
                if let Some(s) = &mut self.cur
                    && let Some(rect) = s.cell
                {
                    s.ann = AnnState {
                        rect: Some(rect),
                        open: true,
                        ..AnnState::default()
                    };
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        if name == "table" {
            if self.skip_tables > 0 {
                self.skip_tables -= 1;
            } else if self.cur.is_some() {
                self.end_sheet();
            }
            return;
        }
        if self.skip_tables > 0 {
            return;
        }
        match name {
            "p" if self.msg_para.is_some() => {
                if let (Some(para), Some(i), Some(msg)) =
                    (self.msg_para.take(), self.val_now, self.msg)
                {
                    let target = if msg == Msg::Help {
                        &mut self.vals[i].prompt
                    } else {
                        &mut self.vals[i].error
                    };
                    if !target.is_empty() {
                        target.push('\n');
                    }
                    target.push_str(&para);
                }
                return;
            }
            "help-message" | "error-message" => {
                self.msg = None;
                return;
            }
            "content-validation" => {
                self.val_now = None;
                return;
            }
            _ => {}
        }
        if self.cur.is_none() {
            return;
        }
        if self.cur.as_ref().is_some_and(|s| s.ann.open) {
            self.annotation_close(name);
            return;
        }
        match name {
            "table-row" => self.end_row(),
            "table-cell" | "covered-table-cell" => {
                if let Some(s) = &mut self.cur {
                    s.cell = None;
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if self.skip_tables > 0 {
            return;
        }
        if let Some(p) = &mut self.msg_para {
            p.push_str(t);
            return;
        }
        let Some(s) = &mut self.cur else { return };
        if s.ann.open {
            match (&mut s.ann.para, s.ann.field) {
                (Some(p), _) => p.push_str(t),
                (None, Some(true)) => s.ann.date.push_str(t),
                (None, Some(false)) => s.ann.author.push_str(t),
                _ => {}
            }
        }
    }

    fn start_sheet(&mut self, e: &Elem) {
        let name = e.attr("name").unwrap_or("").to_string();
        if e.attr("name").is_none() {
            self.issue(IssueKind::Damaged, "a sheet has no name");
        }
        let hidden = self.book.sheet_hidden(e.attr("style-name"));
        self.cur = Some(SheetState {
            sheet: OdsSheet {
                name,
                hidden,
                hidden_rows: Vec::new(),
                hidden_cols: Vec::new(),
                annotations: Vec::new(),
                validations: Vec::new(),
                cell_formats: Vec::new(),
            },
            row: 0,
            col: 0,
            col_defs: 0,
            cols: Vec::new(),
            in_row: false,
            row_rep: 1,
            row_style: None,
            rows_clamped: false,
            cols_clamped: false,
            cell: None,
            ann: AnnState::default(),
        });
    }

    fn end_sheet(&mut self) {
        let Some(mut s) = self.cur.take() else { return };
        let mut unknown = Vec::new();
        for v in &mut self.vals {
            if v.rects.is_empty() {
                continue;
            }
            let rects = std::mem::take(&mut v.rects);
            let rule = parse_condition(&v.condition).unwrap_or_else(|| {
                unknown.push(v.condition.clone());
                rules::Rule {
                    kind: "unknown".to_string(),
                    ..rules::Rule::default()
                }
            });
            s.sheet.validations.push(Validation {
                range: rects.iter().map(|r| r.a1()).collect::<Vec<_>>().join(" "),
                kind: rule.kind,
                operator: rule.operator,
                formula1: rule.formula1,
                formula2: rule.formula2,
                list: rule.list,
                prompt: (!v.prompt.is_empty()).then(|| v.prompt.clone()),
                error: (!v.error.is_empty()).then(|| v.error.clone()),
            });
        }
        for c in unknown {
            self.issue(
                IssueKind::Unsupported,
                format!(
                    "sheet {}: the validation condition {c} is not understood",
                    s.sheet.name
                ),
            );
        }
        self.sheets.push(s.sheet);
    }

    fn named(&mut self, e: &Elem) {
        if self.skip_tables > 0 {
            return;
        }
        let Some(name) = e.attr("name") else { return };
        let refers_to = if e.name == "named-range" {
            e.attr("cell-range-address").map(range_text)
        } else {
            e.attr("expression").map(str::to_string)
        };
        let Some(refers_to) = refers_to else {
            self.issue(
                IssueKind::Damaged,
                format!("the name {name} has no reference"),
            );
            return;
        };
        self.names.push(DefinedName {
            name: name.to_string(),
            scope: self.cur.as_ref().map(|s| s.sheet.name.clone()),
            refers_to,
            hidden: false,
            comment: None,
        });
    }

    fn column(&mut self, e: &Elem) {
        let rep = self.repeat(e, "number-columns-repeated");
        let Some(s) = &mut self.cur else { return };
        if s.in_row {
            return;
        }
        let first = s.col_defs;
        s.col_defs = s.col_defs.saturating_add(rep);
        if first >= MAX_COLS {
            if !s.cols_clamped {
                s.cols_clamped = true;
                let name = s.sheet.name.clone();
                self.issue(
                    IssueKind::Limit,
                    format!("sheet {name}: columns past 16,384 are ignored"),
                );
            }
            return;
        }
        let last = (first + rep - 1).min(MAX_COLS - 1);
        if first + rep > MAX_COLS && !s.cols_clamped {
            s.cols_clamped = true;
            let name = s.sheet.name.clone();
            self.issue(
                IssueKind::Limit,
                format!("sheet {name}: columns past 16,384 are ignored"),
            );
        }
        let Some(s) = &mut self.cur else { return };
        if hidden(e) {
            push_span(&mut s.sheet.hidden_cols, first as u32, last as u32);
        }
        s.cols.push(ColDef {
            first: first as u32,
            last: last as u32,
            style: e.attr("default-cell-style-name").map(str::to_string),
        });
    }

    fn row(&mut self, e: &Elem) {
        let rep = self.repeat(e, "number-rows-repeated");
        let Some(s) = &mut self.cur else { return };
        s.in_row = true;
        s.col = 0;
        s.row_rep = rep;
        s.row_style = e.attr("default-cell-style-name").map(str::to_string);
        if s.row < MAX_ROWS && hidden(e) {
            let last = (s.row + rep - 1).min(MAX_ROWS - 1);
            push_span(&mut s.sheet.hidden_rows, s.row as u32, last as u32);
        }
        if s.row.saturating_add(rep) > MAX_ROWS && !s.rows_clamped {
            s.rows_clamped = true;
            let name = s.sheet.name.clone();
            self.issue(
                IssueKind::Limit,
                format!("sheet {name}: rows past 1,048,576 are ignored"),
            );
        }
    }

    fn end_row(&mut self) {
        if let Some(s) = &mut self.cur {
            s.row = s.row.saturating_add(s.row_rep);
            s.in_row = false;
            s.row_style = None;
        }
    }

    fn cell(&mut self, e: &Elem, empty: bool) {
        let rep = self.repeat(e, "number-columns-repeated");
        let covered = e.name == "covered-table-cell";
        let Some(s) = &mut self.cur else { return };
        if !s.in_row {
            return;
        }
        let c0 = s.col;
        s.col = s.col.saturating_add(rep);
        let row = s.row;
        let row_rep = s.row_rep;
        let name = s.sheet.name.clone();
        if c0 >= MAX_COLS || row >= MAX_ROWS {
            let (flag, what) = if c0 >= MAX_COLS {
                (&mut s.cols_clamped, "columns past 16,384")
            } else {
                (&mut s.rows_clamped, "rows past 1,048,576")
            };
            if !std::mem::replace(flag, true) {
                self.issue(
                    IssueKind::Limit,
                    format!("sheet {name}: {what} are ignored"),
                );
            }
            return;
        }
        if c0 + rep > MAX_COLS && !std::mem::replace(&mut s.cols_clamped, true) {
            self.issue(
                IssueKind::Limit,
                format!("sheet {name}: columns past 16,384 are ignored"),
            );
        }
        let rect = Rect {
            r0: row as u32,
            r1: (row + row_rep - 1).min(MAX_ROWS - 1) as u32,
            c0: c0 as u32,
            c1: (c0 + rep - 1).min(MAX_COLS - 1) as u32,
        };
        let Some(s) = &mut self.cur else { return };
        if covered {
            return;
        }
        if !empty {
            s.cell = Some(rect);
        }
        if let Some(name) = e.attr("content-validation-name") {
            match self.val_index.get(name) {
                Some(&i) => add_rect(&mut self.vals[i].rects, rect),
                None => *self.unknown_vals.entry(name.to_string()).or_insert(0) += 1,
            }
        }
        if e.attr("value-type").is_some() || e.attr("formula").is_some() {
            let cells = u64::from(rect.r1 - rect.r0 + 1) * u64::from(rect.c1 - rect.c0 + 1);
            if self.charge(cells) {
                self.list_formats(e, rect);
            }
        }
    }

    fn list_formats(&mut self, e: &Elem, rect: Rect) {
        let Some(s) = &self.cur else { return };
        // Which style each column run has: the cell's own, the row default or the column's.
        let own = e
            .attr("style-name")
            .map(str::to_string)
            .or_else(|| s.row_style.clone());
        let mut runs: Vec<(u32, u32, Option<String>)> = Vec::new();
        if own.is_some() {
            runs.push((rect.c0, rect.c1, own));
        } else {
            let mut c = rect.c0;
            while c <= rect.c1 {
                let idx = s.cols.partition_point(|d| d.last < c);
                match s.cols.get(idx) {
                    Some(d) if d.first <= c => {
                        let end = d.last.min(rect.c1);
                        runs.push((c, end, d.style.clone()));
                        c = end + 1;
                    }
                    Some(d) => {
                        let end = (d.first - 1).min(rect.c1);
                        c = end + 1;
                    }
                    None => break,
                }
            }
        }
        for (c0, c1, style) in runs {
            let Some(style) = style else { continue };
            let Some(code) = self.book.format_of(&style, &mut self.issues) else {
                continue;
            };
            let cells = u64::from(rect.r1 - rect.r0 + 1) * u64::from(c1 - c0 + 1);
            if !self.charge(cells.saturating_mul(code.len() as u64 / 32)) {
                return;
            }
            let Some(s) = &mut self.cur else { return };
            for r in rect.r0..=rect.r1 {
                for c in c0..=c1 {
                    s.sheet
                        .cell_formats
                        .push((CellPos { row: r, col: c }, code.clone()));
                }
            }
        }
    }

    fn validation(&mut self, e: &Elem) {
        let name = e.attr("name").unwrap_or("").to_string();
        self.vals.push(ValDef {
            condition: e.attr("condition").unwrap_or("").to_string(),
            ..ValDef::default()
        });
        self.val_now = Some(self.vals.len() - 1);
        self.val_index.insert(name, self.vals.len() - 1);
    }

    fn annotation_event(&mut self, e: &Elem, empty: bool) {
        let Some(s) = &mut self.cur else { return };
        let a = &mut s.ann;
        if a.para.is_some() {
            match e.name.as_str() {
                "s" => {
                    let n = e
                        .attr("c")
                        .and_then(|c| c.trim().parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1000);
                    if let Some(p) = &mut a.para {
                        p.push_str(&" ".repeat(n));
                    }
                }
                "tab" => {
                    if let Some(p) = &mut a.para {
                        p.push('\t');
                    }
                }
                "line-break" => {
                    if let Some(p) = &mut a.para {
                        p.push('\n');
                    }
                }
                _ => {}
            }
            return;
        }
        match e.name.as_str() {
            "p" if !empty => a.para = Some(String::new()),
            "creator" if !empty => a.field = Some(false),
            "date" | "date-string" if !empty => a.field = Some(true),
            _ => {}
        }
    }

    fn annotation_close(&mut self, name: &str) {
        let Some(s) = &mut self.cur else { return };
        match name {
            "p" if s.ann.para.is_some() => {
                if let Some(p) = s.ann.para.take() {
                    s.ann.paras.push(p);
                }
            }
            "creator" | "date" | "date-string" if s.ann.para.is_none() => s.ann.field = None,
            "annotation" if s.ann.para.is_none() => {
                let ann = std::mem::take(&mut s.ann);
                let Some(rect) = ann.rect else { return };
                let cells = u64::from(rect.r1 - rect.r0 + 1) * u64::from(rect.c1 - rect.c0 + 1);
                let text = ann.paras.join("\n");
                // Every copy of the text counts, so a long note on a wide repeat hits the limit.
                if !self.charge(cells.saturating_mul(1 + text.len() as u64 / 16)) {
                    return;
                }
                let trimmed = |v: String| {
                    let v = v.trim().to_string();
                    (!v.is_empty()).then_some(v)
                };
                let (author, date) = (trimmed(ann.author), trimmed(ann.date));
                let Some(s) = &mut self.cur else { return };
                for r in rect.r0..=rect.r1 {
                    for c in rect.c0..=rect.c1 {
                        s.sheet.annotations.push(Annotation {
                            cell: a1(r, c),
                            author: author.clone(),
                            date: date.clone(),
                            text: text.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Adds a rectangle, extending the previous one when they touch with the same shape.
fn add_rect(rects: &mut Vec<Rect>, new: Rect) {
    if let Some(last) = rects.last_mut() {
        if last.c0 == new.c0 && last.c1 == new.c1 && last.r1 + 1 == new.r0 {
            last.r1 = new.r1;
            return;
        }
        if last.r0 == new.r0 && last.r1 == new.r1 && last.c1 + 1 == new.c0 {
            last.c1 = new.c1;
            return;
        }
    }
    rects.push(new);
}

fn read_meta(ctx: &mut Ctx<'_, '_>) -> DocProps {
    let mut props = DocProps::default();
    let mut key: Option<(String, Option<String>, Option<String>)> = None;
    let mut text = String::new();
    let mut keywords: Vec<String> = Vec::new();
    ctx.scan("meta.xml", false, &mut |ev, depth| match ev {
        Ev::Start(e) if depth == 3 => {
            key = Some((
                e.name.clone(),
                e.attr("name").map(str::to_string),
                e.attr("value-type").map(str::to_string),
            ));
            text.clear();
        }
        Ev::Empty(e) if depth == 3 && e.name == "user-defined" => {
            if let Some(n) = e.attr("name") {
                props
                    .custom
                    .push((n.to_string(), custom_value(e.attr("value-type"), "")));
            }
        }
        Ev::Text(t) if key.is_some() => text.push_str(t),
        Ev::End(_) if depth == 3 => {
            let Some((name, prop, ty)) = key.take() else {
                return;
            };
            let value = text.trim().to_string();
            if name == "user-defined" {
                if let Some(n) = prop {
                    props.custom.push((n, custom_value(ty.as_deref(), &value)));
                }
            } else if name == "keyword" {
                if !value.is_empty() {
                    keywords.push(value);
                }
            } else if !value.is_empty() {
                let (map, k) = match name.as_str() {
                    "initial-creator" => (true, "creator"),
                    "creator" => (true, "lastModifiedBy"),
                    "creation-date" => (true, "created"),
                    "date" => (true, "modified"),
                    "title" => (true, "title"),
                    "subject" => (true, "subject"),
                    "description" => (true, "description"),
                    "language" => (true, "language"),
                    "generator" => (false, "Application"),
                    "editing-duration" => (false, "EditingDuration"),
                    "editing-cycles" => (false, "EditingCycles"),
                    _ => return,
                };
                let target = if map { &mut props.core } else { &mut props.app };
                target.insert(k.to_string(), value);
            }
        }
        _ => {}
    });
    if !keywords.is_empty() {
        props
            .core
            .insert("keywords".to_string(), keywords.join(", "));
    }
    props
}

fn custom_value(ty: Option<&str>, value: &str) -> PropValue {
    let other = |t: &str| PropValue::Other {
        ty: t.to_string(),
        value: value.to_string(),
    };
    match ty {
        None | Some("string") => PropValue::Text(value.to_string()),
        Some("float") => value
            .parse()
            .map_or_else(|_| other("float"), PropValue::Number),
        Some("boolean") => match value {
            "true" => PropValue::Bool(true),
            "false" => PropValue::Bool(false),
            _ => other("boolean"),
        },
        Some("date") => PropValue::DateTime(value.to_string()),
        Some(t) => other(t),
    }
}
