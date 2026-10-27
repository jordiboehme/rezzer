// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The reader of `content.xml`: sheets, cells, notes, validations, names, ranges and pivot tables.

use std::collections::{BTreeMap, HashMap};

use super::budget::{List, Share};
use super::rules::{self, parse_condition, range_text, sheet_and_cell, split_char};
use super::state::{AnnState, ColDef, Msg, Rect, SheetState, ValDef};
use super::styles::StyleBook;
use super::{Annotation, CONTENT, DataPilot, DatabaseRange, OdsSheet};
use crate::a1::a1;
use crate::ooxml::{Frozen, Validation};
use crate::xml::{Elem, Ev};
use crate::{CellPos, DefinedName, Issue, IssueKind, Look, RowSpan};

const MAX_ROWS: u64 = 1_048_576;

const MAX_COLS: u64 = 16_384;

pub(super) struct Content {
    book: StyleBook,
    pub(super) sheets: Vec<OdsSheet>,
    pub(super) names: Vec<DefinedName>,
    pub(super) database_ranges: Vec<DatabaseRange>,
    pub(super) data_pilots: Vec<DataPilot>,
    pub(super) frozen: HashMap<String, Frozen>,
    pub(super) issues: Vec<Issue>,
    cur: Option<SheetState>,
    /// Tables below the top level (inside a cell) and tables of a wrong shape are not read.
    skip_tables: usize,
    shares: [Share; 6],
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
    pub(super) fn new(book: StyleBook, limit: u64) -> Content {
        Content {
            book,
            sheets: Vec::new(),
            names: Vec::new(),
            database_ranges: Vec::new(),
            data_pilots: Vec::new(),
            frozen: HashMap::new(),
            issues: Vec::new(),
            cur: None,
            skip_tables: 0,
            shares: List::ALL.map(|l| Share::new(l, limit)),
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
    pub(super) fn summary(&self, issues: &mut Vec<Issue>) {
        if self.book.chain_too_long() {
            issues.push(Issue::new(
                CONTENT,
                IssueKind::Damaged,
                "a cell style has parent styles more than 64 levels up: what they say about looks and number formats is not inherited",
            ));
        }
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

    fn charge(&mut self, list: List, n: u64) -> bool {
        // Cells with content get the whole total less what the layout lists have charged.
        let layout: u64 = match list {
            List::Cells => self
                .shares
                .iter()
                .skip(1)
                .fold(0, |a, s| a.saturating_add(s.used)),
            _ => 0,
        };
        let share = &mut self.shares[list as usize];
        if share.exhausted {
            return false;
        }
        let limit = share.limit.saturating_sub(layout);
        if share.used.saturating_add(n) > limit {
            // What did not fit is not charged: it was not listed.
            share.exhausted = true;
            self.issue(
                IssueKind::Limit,
                format!("more than {limit} {}: the rest is not listed", list.what()),
            );
            return false;
        }
        share.used += n;
        true
    }

    /// Copied text counts against the budget of the other copied strings (the notes).
    fn charge_text(&mut self, parts: &[&str]) -> bool {
        let bytes: usize = parts.iter().map(|p| p.len()).sum();
        self.charge(List::Cells, bytes as u64 / 16)
    }

    pub(super) fn event(&mut self, ev: &Ev<'_>, depth: usize) {
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
            "database-range" => {
                self.database_range(e);
                return;
            }
            "data-pilot-table" => {
                self.data_pilot(e);
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
            "table-header-rows" if !empty => {
                if let Some(s) = &mut self.cur {
                    s.in_header = true;
                }
            }
            "table-row-group" if !empty => {
                if let Some(s) = &mut self.cur {
                    s.group_depth = s.group_depth.saturating_add(1);
                    s.group_break = true;
                }
            }
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
            "table-header-rows" => {
                if let Some(s) = &mut self.cur {
                    s.in_header = false;
                }
            }
            "table-row-group" => {
                if let Some(s) = &mut self.cur {
                    s.group_depth = s.group_depth.saturating_sub(1);
                }
            }
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
        let frozen = self.frozen.get(&name).copied();
        self.cur = Some(SheetState {
            sheet: OdsSheet {
                name,
                hidden,
                hidden_rows: Vec::new(),
                hidden_cols: Vec::new(),
                annotations: Vec::new(),
                validations: Vec::new(),
                cell_formats: Vec::new(),
                merges: Vec::new(),
                looks: Vec::new(),
                has_looks: self.book.has_looks(),
                header_rows: None,
                outline: Vec::new(),
                frozen,
            },
            row: 0,
            col: 0,
            col_defs: 0,
            cols: Vec::new(),
            in_row: false,
            in_header: false,
            group_depth: 0,
            group_break: false,
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
        let mut pushed = false;
        if s.row < MAX_ROWS {
            let first = s.row as u32;
            let last = (s.row + rep - 1).min(MAX_ROWS - 1) as u32;
            if s.in_header {
                s.sheet.header_rows = Some(match s.sheet.header_rows {
                    Some(h) => RowSpan {
                        first: h.first.min(first),
                        last: h.last.max(last),
                    },
                    None => RowSpan { first, last },
                });
            }
            if s.group_depth > 0 {
                let depth = s.group_depth;
                match s.sheet.outline.last_mut() {
                    Some((prev, d))
                        if !s.group_break
                            && *d == depth
                            && prev.last.saturating_add(1) == first =>
                    {
                        prev.last = last;
                    }
                    _ => {
                        s.sheet.outline.push((RowSpan { first, last }, depth));
                        pushed = true;
                    }
                }
                s.group_break = false;
            }
        }
        if s.row.saturating_add(rep) > MAX_ROWS && !s.rows_clamped {
            s.rows_clamped = true;
            let name = s.sheet.name.clone();
            self.issue(
                IssueKind::Limit,
                format!("sheet {name}: rows past 1,048,576 are ignored"),
            );
        }
        if pushed
            && !self.charge(List::Outline, 1)
            && let Some(s) = &mut self.cur
        {
            s.sheet.outline.pop();
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
        let cells = u64::from(rect.r1 - rect.r0 + 1) * u64::from(rect.c1 - rect.c0 + 1);
        let cols = self.repeat(e, "number-columns-spanned");
        let rows = self.repeat(e, "number-rows-spanned");
        if (cols > 1 || rows > 1) && self.charge(List::Merges, cells) {
            self.list_merges(rect, rows, cols);
        }
        let content = e.attr("value-type").is_some() || e.attr("formula").is_some();
        if content || e.attr("style-name").is_some() {
            let runs = self.style_runs(e, rect);
            if content && self.charge(List::Cells, cells) {
                self.list_formats(&runs, rect);
            }
            self.list_looks(&runs, rect);
        }
    }

    /// One merge per copy of a repeated spanned cell, each clamped to the sheet.
    fn list_merges(&mut self, rect: Rect, rows: u64, cols: u64) {
        let Some(s) = &mut self.cur else { return };
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                let r1 = (u64::from(r) + rows - 1).min(MAX_ROWS - 1) as u32;
                let c1 = (u64::from(c) + cols - 1).min(MAX_COLS - 1) as u32;
                s.sheet
                    .merges
                    .push((CellPos { row: r, col: c }, CellPos { row: r1, col: c1 }));
            }
        }
    }

    /// Which style each run of columns of a cell has: the cell's own, the row default or the
    /// column's. Runs follow the column definitions, so a repeat is never expanded.
    fn style_runs(&self, e: &Elem, rect: Rect) -> Vec<(u32, u32, Option<String>)> {
        let Some(s) = &self.cur else {
            return Vec::new();
        };
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
        runs
    }

    fn list_looks(&mut self, runs: &[(u32, u32, Option<String>)], rect: Rect) {
        for (c0, c1, style) in runs {
            let Some(style) = style else { continue };
            let look = self.book.look(style);
            if look == Look::default() {
                continue;
            }
            let cells = u64::from(rect.r1 - rect.r0 + 1) * u64::from(c1 - c0 + 1);
            if !self.charge(List::Looks, cells) {
                return;
            }
            let Some(s) = &mut self.cur else { return };
            for r in rect.r0..=rect.r1 {
                for c in *c0..=*c1 {
                    s.sheet.looks.push((CellPos { row: r, col: c }, look));
                }
            }
        }
    }

    fn list_formats(&mut self, runs: &[(u32, u32, Option<String>)], rect: Rect) {
        for (c0, c1, style) in runs {
            let Some(style) = style else { continue };
            let Some(code) = self.book.format_of(style, &mut self.issues) else {
                continue;
            };
            let cells = u64::from(rect.r1 - rect.r0 + 1) * u64::from(c1 - c0 + 1);
            if !self.charge(List::Cells, cells.saturating_mul(code.len() as u64 / 32)) {
                return;
            }
            let Some(s) = &mut self.cur else { return };
            for r in rect.r0..=rect.r1 {
                for c in *c0..=*c1 {
                    s.sheet
                        .cell_formats
                        .push((CellPos { row: r, col: c }, code.clone()));
                }
            }
        }
    }

    fn database_range(&mut self, e: &Elem) {
        let name = e.attr("name").unwrap_or("").to_string();
        let Some((sheet, range)) = e.attr("target-range-address").and_then(target) else {
            self.issue(
                IssueKind::Damaged,
                format!("the database range {name} has no readable target range"),
            );
            return;
        };
        if !self.charge(List::Ranges, 1) || !self.charge_text(&[&name, &sheet, &range]) {
            return;
        }
        self.database_ranges.push(DatabaseRange {
            name,
            sheet,
            range,
            header: e.attr("contains-header") != Some("false"),
            footer: e.attr("contains-footer") == Some("true"),
            filter_buttons: e.attr("display-filter-buttons") == Some("true"),
        });
    }

    fn data_pilot(&mut self, e: &Elem) {
        let name = e.attr("name").unwrap_or("").to_string();
        let Some((sheet, range)) = e.attr("target-range-address").and_then(target) else {
            self.issue(
                IssueKind::Damaged,
                format!("the pivot table {name} has no readable target range"),
            );
            return;
        };
        if !self.charge(List::Pilots, 1) || !self.charge_text(&[&name, &sheet, &range]) {
            return;
        }
        self.data_pilots.push(DataPilot { name, sheet, range });
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
                if !self.charge(
                    List::Cells,
                    cells.saturating_mul(1 + text.len() as u64 / 16),
                ) {
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

/// A cell range address (`Sheet1.A1:Sheet1.C10`, `$'My Sheet'.$A$1:.$C$10`) as the sheet of its
/// first cell and an A1 range without `$`. Both ends equal give the one cell.
fn target(address: &str) -> Option<(String, String)> {
    let ends = split_char(address.trim(), ':');
    if ends.len() > 2 {
        return None;
    }
    let (sheet, first) = sheet_and_cell(ends[0])?;
    if let Some(end) = ends.get(1) {
        // A range across two sheets is no rectangle on one sheet.
        let (sheet2, _) = sheet_and_cell(end)?;
        if !sheet2.is_empty() && sheet2 != sheet {
            return None;
        }
    }
    let clean = |c: &str| c.replace('$', "");
    let first = clean(first);
    let last = match ends.get(1) {
        Some(end) => clean(sheet_and_cell(end)?.1),
        None => first.clone(),
    };
    if first.is_empty() || last.is_empty() {
        return None;
    }
    let range = if first == last {
        first
    } else {
        format!("{first}:{last}")
    };
    Some((sheet, range))
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
