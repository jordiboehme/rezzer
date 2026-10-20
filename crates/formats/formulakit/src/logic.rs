// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The logic of a whole workbook: which formulas are results and which feed others, which
//! copies fold into one pattern group, which constant cells are parameters, and a
//! dependency graph on group level.
//!
//! Feed a [`Workbook`] with every formula (with its [`Analysis`]), every constant cell and
//! every defined name, then call [`Workbook::finish`]. Ranges are never expanded cell by
//! cell: each sheet keeps its formula and constant cells in one sorted index, and a range
//! visits only the cells that exist inside it, so `A:A` or `1:1048576` costs what the sheet
//! holds, not a million cells. Read counts come from one sweep per sheet over the rows
//! with a Fenwick tree over the columns, so a column of running totals
//! (`=SUM($B$2:B2)` filled down) costs about n log n, not n squared. Chains of defined
//! names are followed without recursion.
//!
//! ```
//! use formulakit::logic::{Role, Workbook};
//! use formulakit::{CellPos, Dialect, analyze};
//!
//! let mut wb = Workbook::new();
//! wb.add_constant("S", CellPos { row: 0, col: 0 });
//! let at = CellPos { row: 0, col: 1 };
//! let id = wb.add_formula("S", at, &analyze("=A1*2", Dialect::Excel, at).unwrap());
//! let logic = wb.finish(&|| false);
//! assert_eq!(logic.roles, vec![(id, Role::Result)]);
//! assert_eq!(logic.parameters.len(), 1);
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use crate::refs::{MAX_COL, TableRows, col_letters};
use crate::{Analysis, CellPos, Ref, anchor_of, quote_sheet};

/// A formula added to a [`Workbook`], numbered from 0 in the order of adding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FormulaId(pub u32);

/// What a formula is in the workbook's logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The formula reads nothing (`=1+2`, `=NOW()`).
    Constant,
    /// No formula reads it (directly or through a name): an output of the workbook.
    Result,
    /// It reads something and other formulas read it.
    Intermediate,
}

/// Copies of one formula on one sheet: two or more formulas with the same pattern key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// `p1`, `p2`, ... in order of the group's first cell (sheets in the order they were
    /// first added, then row, then column). Also the group's node id in the graph.
    pub id: String,
    pub sheet: String,
    /// The shared pattern key ([`Analysis::pattern_key`]).
    pub key: String,
    /// The members in cell order.
    pub members: Vec<FormulaId>,
    /// The bounding range of the members in A1 form without a sheet, e.g. `F62:AK82`. It
    /// can hold cells that are not members when the copies have gaps.
    pub area: String,
}

/// A constant cell that is a parameter: a formula reads it (directly, through a range or
/// through a defined name), or a defined name refers to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamCell {
    pub sheet: String,
    pub at: CellPos,
    /// How many formulas read it (each formula once, however often it names the cell).
    pub read_by: u32,
}

/// The dependency graph on group level.
///
/// Nodes are `(id, label, kind)`. Kinds and ids: `group` (`p1`, ..., the [`Group`] ids,
/// label `Sheet!D2:D100`), `formula` for a formula outside any group (`f1`, ..., label
/// `Sheet!E5`), `parameters` for the constant cells of one sheet that one reader reads
/// (`v1`, ..., label the bounding range `Sheet!B2:C100`), `name` for a defined name or a
/// table reference that does not resolve to cells (`n1`, ..., label `name:rate`,
/// `name:Sheet1!rate` or `table:Table1`) and
/// `external` for another workbook (`x1`, ..., label `ext:[1]`).
///
/// Order: groups and single formulas in cell order, then what each reader brings in (its
/// parameter blocks by sheet, then names, tables and externals in order of first use);
/// names are readers too and bring in what they refer to. Every group and single formula is
/// a node, even when it has no edges.
///
/// An edge `(from, to)` means `to` reads `from`. Edges are listed once each, ordered by the
/// position of `from` and then `to` in `nodes`. A group whose copies read each other (a
/// running total) has an edge to itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupGraph {
    pub nodes: Vec<(String, String, String)>,
    pub edges: Vec<(String, String)>,
}

/// The result of [`Workbook::finish`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Logic {
    /// The role of each formula, in [`FormulaId`] order.
    pub roles: Vec<(FormulaId, Role)>,
    pub groups: Vec<Group>,
    /// In sheet order, then row, then column.
    pub parameters: Vec<ParamCell>,
    pub graph: GroupGraph,
    /// The stop callback ended the work early. `groups` is always complete (it needs no
    /// read work). When the stop came while reads were counted, `roles` holds only the
    /// `Constant` roles and `parameters` and `graph` are empty; when it came while the graph
    /// was built, `roles` and `parameters` are complete and `graph` is empty. The same input
    /// and the same stop point give the same result.
    pub stopped: bool,
}

/// Where a table sits, for [`Workbook::add_table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableArea {
    /// The top left cell, including the header row.
    pub start: CellPos,
    /// The bottom right cell, including the totals row.
    pub end: CellPos,
    /// Header rows at the top (0 or 1 in Excel).
    pub header_rows: u32,
    /// Totals rows at the bottom (0 or 1 in Excel).
    pub totals_rows: u32,
    /// The column names from left to right.
    pub columns: Vec<String>,
}

struct FormulaEntry {
    sheet: u32,
    at: CellPos,
    key: String,
    refs: Vec<Ref>,
}

struct NameEntry {
    name: String,
    /// The scope sheet as given.
    scope: Option<String>,
    /// What the name refers to; `None` when it could not be read.
    refs: Option<Vec<Ref>>,
}

struct TableEntry {
    sheet: u32,
    area: TableArea,
}

/// Collects a workbook's formulas, constants, names and tables. See the
/// [module docs](self).
///
/// Sheets are known by name (case-insensitive) in the order they first appear in
/// [`add_formula`](Self::add_formula), [`add_constant`](Self::add_constant) or
/// [`add_table`](Self::add_table). A 3D reference (`Sheet1:Sheet3!A1`) covers the sheets
/// between its ends in that order, so add the cells sheet by sheet in workbook order.
#[derive(Default)]
pub struct Workbook {
    sheets: Vec<String>,
    sheet_index: HashMap<String, u32>,
    formulas: Vec<FormulaEntry>,
    constants: Vec<(u32, CellPos)>,
    names: Vec<NameEntry>,
    /// Uppercase name and scope to the first name added with them.
    name_index: HashMap<(String, Option<String>), usize>,
    tables: Vec<TableEntry>,
    /// Uppercase table name to the first table added with it.
    table_index: HashMap<String, usize>,
}

/// A rectangle of cells on one sheet, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Rect {
    sheet: u32,
    r1: u32,
    c1: u32,
    r2: u32,
    c2: u32,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.sheet == o.sheet
            && self.r1 <= o.r2
            && o.r1 <= self.r2
            && self.c1 <= o.c2
            && o.c1 <= self.c2
    }

    fn is_cell(&self) -> bool {
        self.r1 == self.r2 && self.c1 == self.c2
    }

    /// `self` without the cells of `o`, as up to four rectangles.
    fn subtract(self, o: &Rect, out: &mut Vec<Rect>) {
        if !self.overlaps(o) {
            out.push(self);
            return;
        }
        if self.r1 < o.r1 {
            out.push(Rect {
                r2: o.r1 - 1,
                ..self
            });
        }
        if self.r2 > o.r2 {
            out.push(Rect {
                r1: o.r2 + 1,
                ..self
            });
        }
        let (r1, r2) = (self.r1.max(o.r1), self.r2.min(o.r2));
        if self.c1 < o.c1 {
            out.push(Rect {
                r1,
                r2,
                c2: o.c1 - 1,
                ..self
            });
        }
        if self.c2 > o.c2 {
            out.push(Rect {
                r1,
                r2,
                c1: o.c2 + 1,
                ..self
            });
        }
    }
}

/// The same cells as `rects` in fewer rectangles: rectangles with the same columns whose
/// rows overlap or touch become one, then the same for rows. Nested ranges (running
/// totals) and copies that read the next cell down collapse this way.
fn merge(mut rects: Vec<Rect>) -> Vec<Rect> {
    rects.sort_unstable_by_key(|r| (r.sheet, r.c1, r.c2, r.r1, r.r2));
    let mut by_cols: Vec<Rect> = Vec::with_capacity(rects.len());
    for r in rects {
        match by_cols.last_mut() {
            Some(l)
                if l.sheet == r.sheet
                    && l.c1 == r.c1
                    && l.c2 == r.c2
                    && r.r1 <= l.r2.saturating_add(1) =>
            {
                l.r2 = l.r2.max(r.r2);
            }
            _ => by_cols.push(r),
        }
    }
    by_cols.sort_unstable_by_key(|r| (r.sheet, r.r1, r.r2, r.c1, r.c2));
    let mut out: Vec<Rect> = Vec::with_capacity(by_cols.len());
    for r in by_cols {
        match out.last_mut() {
            Some(l)
                if l.sheet == r.sheet
                    && l.r1 == r.r1
                    && l.r2 == r.r2
                    && r.c1 <= l.c2.saturating_add(1) =>
            {
                l.c2 = l.c2.max(r.c2);
            }
            _ => out.push(r),
        }
    }
    out
}

/// The same cells as `rects`, each covered once, so a formula counts a cell once. Returns
/// `None` when the stop callback asks to stop.
fn disjoint(rects: Vec<Rect>, stopper: &mut Stopper) -> Option<Vec<Rect>> {
    let rects = merge(rects);
    let (cells, ranges): (Vec<Rect>, Vec<Rect>) = rects.into_iter().partition(Rect::is_cell);
    let mut out: Vec<Rect> = Vec::new();
    let mut pieces = Vec::new();
    let mut next = Vec::new();
    for r in ranges {
        pieces.clear();
        pieces.push(r);
        for o in &out {
            if stopper.tick() {
                return None;
            }
            next.clear();
            for p in pieces.drain(..) {
                p.subtract(o, &mut next);
            }
            std::mem::swap(&mut pieces, &mut next);
            if pieces.is_empty() {
                break;
            }
        }
        out.extend(pieces.iter().copied());
    }
    let range_count = out.len();
    for c in cells {
        if stopper.tick() {
            return None;
        }
        // `merge` leaves no duplicate cells, so only ranges can cover a cell twice.
        if !out[..range_count].iter().any(|o| c.overlaps(o)) {
            out.push(c);
        }
    }
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointKind {
    Formula(u32),
    Constant,
}

/// The formula and constant cells of one sheet, sorted by row, then column.
#[derive(Default)]
struct SheetIndex {
    points: Vec<(CellPos, PointKind)>,
    /// Each row that holds a cell, with the index of its first cell in `points`.
    rows: Vec<(u32, usize)>,
}

impl SheetIndex {
    fn build(mut points: Vec<(CellPos, PointKind)>) -> Self {
        points.sort_by_key(|(p, k)| {
            (
                *p,
                match k {
                    PointKind::Formula(f) => u64::from(*f),
                    PointKind::Constant => u64::MAX,
                },
            )
        });
        points.dedup();
        let mut rows = Vec::new();
        for (i, (p, _)) in points.iter().enumerate() {
            if rows.last().is_none_or(|(r, _)| *r != p.row) {
                rows.push((p.row, i));
            }
        }
        SheetIndex { points, rows }
    }

    /// Calls `visit` with the index of each cell inside the rectangle. Visits only rows
    /// that hold cells; returns false when `visit` asks to stop.
    fn each_in(&self, r: &Rect, mut visit: impl FnMut(usize) -> bool) -> bool {
        let first = self.rows.partition_point(|(row, _)| *row < r.r1);
        for (n, (row, start)) in self.rows.iter().enumerate().skip(first) {
            if *row > r.r2 {
                break;
            }
            let end = self
                .rows
                .get(n + 1)
                .map_or(self.points.len(), |(_, next)| *next);
            let Some(segment) = self.points.get(*start..end) else {
                continue;
            };
            let from = segment.partition_point(|(p, _)| p.col < r.c1);
            for (i, (p, _)) in segment.iter().enumerate().skip(from) {
                if p.col > r.c2 {
                    break;
                }
                if !visit(start + i) {
                    return false;
                }
            }
        }
        true
    }
}

/// A Fenwick tree over the columns of a sheet: add a value to a column range, read the
/// sum at one column.
struct Fenwick {
    tree: Vec<i64>,
}

impl Fenwick {
    fn new() -> Self {
        // Columns 0..=MAX_COL, one past the end for the range end, 1-based inside.
        Fenwick {
            tree: vec![0; MAX_COL as usize + 3],
        }
    }

    fn clear(&mut self) {
        self.tree.iter_mut().for_each(|v| *v = 0);
    }

    fn add_at(&mut self, col: u32, delta: i64) {
        let mut i = col as usize + 1;
        while let Some(v) = self.tree.get_mut(i) {
            *v += delta;
            i += i.isolate_lowest_one();
        }
    }

    /// Adds `delta` to columns `c1..=c2`.
    fn add(&mut self, c1: u32, c2: u32, delta: i64) {
        self.add_at(c1, delta);
        self.add_at(c2.saturating_add(1), -delta);
    }

    fn at(&self, col: u32) -> i64 {
        let mut i = col as usize + 1;
        let mut sum = 0;
        while i > 0 {
            sum += self.tree.get(i).copied().unwrap_or(0);
            i -= i.isolate_lowest_one();
        }
        sum
    }
}

/// A rectangle starts (`delta` +1) or ends (`delta` -1, one row past its end) for the sweep.
#[derive(Clone, Copy)]
struct Event {
    row: u32,
    c1: u32,
    c2: u32,
    delta: i64,
}

fn push_events(events: &mut [Vec<Event>], r: &Rect) {
    if let Some(list) = events.get_mut(r.sheet as usize) {
        list.push(Event {
            row: r.r1,
            c1: r.c1,
            c2: r.c2,
            delta: 1,
        });
        list.push(Event {
            row: r.r2.saturating_add(1),
            c1: r.c1,
            c2: r.c2,
            delta: -1,
        });
    }
}

/// Something a reader reads that is not a cell: a name, a table or another workbook.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Sym {
    /// A defined name added with [`Workbook::add_name`].
    Name(usize),
    /// A name no [`Workbook::add_name`] defined, by its anchor.
    Unknown(String),
    /// A table reference that could not be resolved to cells (unknown table or column).
    Table(String),
    External(String),
}

/// What one reader (a formula, or a defined name) reads.
#[derive(Default)]
struct Reads {
    rects: Vec<Rect>,
    syms: Vec<Sym>,
}

/// Where a reader's references are read from.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    /// The sheet unqualified cell references point into.
    sheet: Option<u32>,
    /// The sheet name lookups start from.
    scope: Option<&'a str>,
    /// The formula cell, for `[@col]` and a table reference without a table name.
    at: Option<CellPos>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Group,
    Formula,
    Parameters,
    Name,
    External,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Group => "group",
            Kind::Formula => "formula",
            Kind::Parameters => "parameters",
            Kind::Name => "name",
            Kind::External => "external",
        }
    }

    fn prefix(self) -> char {
        match self {
            Kind::Group => 'p',
            Kind::Formula => 'f',
            Kind::Parameters => 'v',
            Kind::Name => 'n',
            Kind::External => 'x',
        }
    }

    fn reads(self) -> bool {
        matches!(self, Kind::Group | Kind::Formula | Kind::Name)
    }
}

struct Node {
    kind: Kind,
    label: String,
    /// For readers: the nodes this one brings in, in order of first use.
    brings: Vec<usize>,
}

/// The bounding box of some cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    lo: CellPos,
    hi: CellPos,
}

impl Bounds {
    fn of(p: CellPos) -> Self {
        Bounds { lo: p, hi: p }
    }

    fn add(&mut self, o: Bounds) {
        self.lo.row = self.lo.row.min(o.lo.row);
        self.lo.col = self.lo.col.min(o.lo.col);
        self.hi.row = self.hi.row.max(o.hi.row);
        self.hi.col = self.hi.col.max(o.hi.col);
    }

    fn a1(&self) -> String {
        let cell = |p: CellPos| format!("{}{}", col_letters(p.col), u64::from(p.row) + 1);
        if self.lo == self.hi {
            cell(self.lo)
        } else {
            format!("{}:{}", cell(self.lo), cell(self.hi))
        }
    }
}

fn add_bounds(slot: &mut Option<Bounds>, b: Bounds) {
    match slot {
        Some(x) => x.add(b),
        None => *slot = Some(b),
    }
}

/// What one rectangle holds: the formula nodes and the bounding box of the constants.
struct Hits {
    targets: Vec<usize>,
    constants: Option<Bounds>,
}

/// Rectangles that visit at least this many cells keep their [`Hits`] for later readers.
const MEMO_MIN_CELLS: usize = 256;
/// The most rectangles kept for name closures at one time.
const CLOSURE_MEMO_RECTS: usize = 1 << 20;
/// How often the stop callback is asked inside long loops.
const STOP_EVERY: u32 = 4096;

struct Stopper<'a> {
    stop: &'a dyn Fn() -> bool,
    ticks: u32,
    stopped: bool,
}

impl Stopper<'_> {
    /// Asks the callback now.
    fn check(&mut self) -> bool {
        if !self.stopped && (self.stop)() {
            self.stopped = true;
        }
        self.stopped
    }

    /// Asks the callback every [`STOP_EVERY`] calls.
    fn tick(&mut self) -> bool {
        self.ticks += 1;
        if self.ticks >= STOP_EVERY {
            self.ticks = 0;
            return self.check();
        }
        self.stopped
    }
}

/// The node of each sym, made on first use; a new node is brought in by its reader.
struct SymNodes {
    map: HashMap<Sym, usize>,
    /// Names that got a node, with that node, in order: they are readers too.
    names: Vec<(usize, usize)>,
}

impl Workbook {
    pub fn new() -> Self {
        Self::default()
    }

    fn sheet_id(&mut self, sheet: &str) -> u32 {
        let key = sheet.to_uppercase();
        if let Some(id) = self.sheet_index.get(&key) {
            return *id;
        }
        let id = u32::try_from(self.sheets.len()).unwrap_or(u32::MAX);
        self.sheets.push(sheet.to_string());
        self.sheet_index.insert(key, id);
        id
    }

    fn find_sheet(&self, sheet: &str) -> Option<u32> {
        self.sheet_index.get(&sheet.to_uppercase()).copied()
    }

    /// Adds the formula in cell `at` of `sheet`.
    pub fn add_formula(&mut self, sheet: &str, at: CellPos, analysis: &Analysis) -> FormulaId {
        let sheet = self.sheet_id(sheet);
        let id = FormulaId(u32::try_from(self.formulas.len()).unwrap_or(u32::MAX));
        self.formulas.push(FormulaEntry {
            sheet,
            at,
            key: analysis.pattern_key.clone(),
            refs: analysis.refs.clone(),
        });
        id
    }

    /// Adds a cell that holds a value, not a formula. Only constants that some formula or
    /// name reads become parameters; adding all of a sheet's value cells is fine.
    pub fn add_constant(&mut self, sheet: &str, at: CellPos) {
        let sheet = self.sheet_id(sheet);
        self.constants.push((sheet, at));
    }

    /// Adds a defined name with its scope sheet (`None` for workbook scope) and the analysis
    /// of what it refers to (`None` when that could not be read). References without a
    /// sheet in a sheet-scoped name point into the scope sheet; in a workbook-scoped name
    /// they depend on the active sheet and are not followed.
    pub fn add_name(&mut self, name: &str, scope: Option<&str>, analysis: Option<&Analysis>) {
        let key = (name.to_uppercase(), scope.map(str::to_uppercase));
        self.name_index.entry(key).or_insert(self.names.len());
        self.names.push(NameEntry {
            name: name.to_string(),
            scope: scope.map(str::to_string),
            refs: analysis.map(|a| a.refs.clone()),
        });
    }

    /// Adds a table (a list object), so structured references (`Sales[Price]`,
    /// `[@Qty]`, `Sales[[#Headers],[Price]]`, a bare `Sales`) resolve to its cells. A column
    /// reference reads that column of the data rows unless it names other rows; `[@col]`
    /// reads the formula's own row.
    pub fn add_table(&mut self, name: &str, sheet: &str, area: TableArea) {
        let sheet = self.sheet_id(sheet);
        self.table_index
            .entry(name.to_uppercase())
            .or_insert(self.tables.len());
        self.tables.push(TableEntry { sheet, area });
    }

    /// The defined name `name` as seen from `sheet` (sheet scope first, then workbook).
    fn lookup(&self, name: &str, explicit: Option<&str>, sheet: Option<&str>) -> Option<usize> {
        let find = |scope: Option<&str>| {
            self.name_index
                .get(&(name.to_uppercase(), scope.map(str::to_uppercase)))
                .copied()
        };
        match explicit {
            Some(s) => find(Some(s)),
            None => sheet.and_then(|s| find(Some(s))).or_else(|| find(None)),
        }
    }

    /// The table a reference names; an empty name means the table holding the formula.
    fn find_table(&self, name: &str, ctx: Ctx) -> Option<&TableEntry> {
        if name.is_empty() {
            let (sheet, at) = (ctx.sheet?, ctx.at?);
            return self.tables.iter().find(|t| {
                t.sheet == sheet
                    && (t.area.start.row..=t.area.end.row).contains(&at.row)
                    && (t.area.start.col..=t.area.end.col).contains(&at.col)
            });
        }
        self.table_index
            .get(&name.to_uppercase())
            .and_then(|i| self.tables.get(*i))
    }

    /// The cells a table reference reads, or `None` when it cannot be resolved.
    fn table_rects(
        &self,
        name: &str,
        column: Option<&str>,
        rows: TableRows,
        ctx: Ctx,
    ) -> Option<Vec<Rect>> {
        let t = self.find_table(name, ctx)?;
        let a = &t.area;
        let index = |c: &str| {
            a.columns
                .iter()
                .position(|n| n.to_uppercase() == c.to_uppercase())
                .and_then(|i| u32::try_from(i).ok())
        };
        let (c1, c2) = match column {
            None => (a.start.col, a.end.col),
            Some(c) => {
                let (x, y) = match index(c) {
                    Some(i) => (i, i),
                    None => {
                        let (x, y) = c.split_once(':')?;
                        let (x, y) = (index(x)?, index(y)?);
                        (x.min(y), x.max(y))
                    }
                };
                (a.start.col.saturating_add(x), a.start.col.saturating_add(y))
            }
        };
        let top = a.start.row;
        let data_first = top.saturating_add(a.header_rows);
        let totals_first = a.end.row.saturating_add(1).saturating_sub(a.totals_rows);
        let mut out = Vec::new();
        let mut span = |r1: u32, r2: u32| {
            if r1 <= r2 && r2 <= a.end.row {
                out.push(Rect {
                    sheet: t.sheet,
                    r1,
                    c1,
                    r2,
                    c2: c2.min(a.end.col),
                });
            }
        };
        if rows.this_row {
            let at = ctx.at?;
            if ctx.sheet != Some(t.sheet) || at.row < data_first || at.row >= totals_first {
                return None;
            }
            span(at.row, at.row);
        }
        if rows.headers && a.header_rows > 0 {
            span(top, data_first - 1);
        }
        if rows.data && totals_first > 0 {
            span(data_first, totals_first - 1);
        }
        if rows.totals && a.totals_rows > 0 {
            span(totals_first, a.end.row);
        }
        Some(out)
    }

    /// Resolves the references of a reader.
    fn resolve(&self, refs: &[Ref], ctx: Ctx) -> Reads {
        let mut reads = Reads::default();
        for r in refs {
            match r {
                Ref::Cell { sheet: span, .. } | Ref::Range { sheet: span, .. } => {
                    let (start, end) = match r {
                        Ref::Cell { pos, .. } => (*pos, *pos),
                        Ref::Range { start, end, .. } => (*start, *end),
                        _ => continue,
                    };
                    let sheets: Vec<u32> = match span {
                        None => ctx.sheet.into_iter().collect(),
                        Some(span) => {
                            let first = self.find_sheet(&span.first);
                            match (first, span.last.as_deref().map(|l| self.find_sheet(l))) {
                                (Some(a), None) => vec![a],
                                (Some(a), Some(Some(b))) => (a.min(b)..=a.max(b)).collect(),
                                // A 3D end that holds nothing known: only the known end.
                                (Some(a), Some(None)) => vec![a],
                                (None, Some(Some(b))) => vec![b],
                                (None, _) => Vec::new(),
                            }
                        }
                    };
                    for s in sheets {
                        reads.rects.push(Rect {
                            sheet: s,
                            r1: start.row.min(end.row),
                            c1: start.col.min(end.col),
                            r2: start.row.max(end.row),
                            c2: start.col.max(end.col),
                        });
                    }
                }
                Ref::Name { name, sheet: s } => match self.lookup(name, s.as_deref(), ctx.scope) {
                    Some(i) => reads.syms.push(Sym::Name(i)),
                    None => {
                        // A bare table name reads the table's data rows.
                        let table = s
                            .is_none()
                            .then(|| self.table_rects(name, None, TableRows::DATA, ctx))
                            .flatten();
                        match table {
                            Some(rects) if !name.is_empty() => reads.rects.extend(rects),
                            _ => reads
                                .syms
                                .push(Sym::Unknown(anchor_of(ctx.scope.unwrap_or(""), r))),
                        }
                    }
                },
                Ref::Table { name, column, rows } => {
                    match self.table_rects(name, column.as_deref(), *rows, ctx) {
                        Some(rects) => reads.rects.extend(rects),
                        None => reads.syms.push(Sym::Table(name.clone())),
                    }
                }
                Ref::External { book, .. } => reads.syms.push(Sym::External(book.clone())),
            }
        }
        reads
    }

    fn name_reads(&self, i: usize) -> Reads {
        let Some(n) = self.names.get(i) else {
            return Reads::default();
        };
        let ctx = Ctx {
            sheet: n.scope.as_deref().and_then(|s| self.find_sheet(s)),
            scope: n.scope.as_deref(),
            at: None,
        };
        match &n.refs {
            Some(refs) => self.resolve(refs, ctx),
            None => Reads::default(),
        }
    }

    /// Computes roles, groups, parameters and the graph. `stop` is asked now and then; when
    /// it returns true the work ends early and [`Logic::stopped`] is set.
    pub fn finish(self, stop: &dyn Fn() -> bool) -> Logic {
        let mut stopper = Stopper {
            stop,
            ticks: 0,
            stopped: false,
        };
        let mut logic = Logic::default();

        // Phase 1, no reads: formulas in cell order, folded into groups by sheet and key.
        let count = u32::try_from(self.formulas.len()).unwrap_or(u32::MAX);
        let mut order: Vec<u32> = (0..count).collect();
        let entry = |f: u32| &self.formulas[f as usize];
        order.sort_by_key(|f| (entry(*f).sheet, entry(*f).at));
        let mut by_key: HashMap<(u32, &str), Vec<u32>> = HashMap::new();
        for f in &order {
            let e = entry(*f);
            by_key
                .entry((e.sheet, e.key.as_str()))
                .or_default()
                .push(*f);
        }
        let mut nodes: Vec<Node> = Vec::new();
        let mut formula_node = vec![usize::MAX; self.formulas.len()];
        for f in &order {
            if formula_node[*f as usize] != usize::MAX {
                continue;
            }
            let e = entry(*f);
            let members = by_key
                .remove(&(e.sheet, e.key.as_str()))
                .unwrap_or_default();
            let sheet_name = &self.sheets[e.sheet as usize];
            let n = nodes.len();
            let mut b = Bounds::of(e.at);
            for m in &members {
                b.add(Bounds::of(entry(*m).at));
                formula_node[*m as usize] = n;
            }
            let kind = if members.len() >= 2 {
                Kind::Group
            } else {
                Kind::Formula
            };
            if kind == Kind::Group {
                logic.groups.push(Group {
                    id: format!("p{}", logic.groups.len() + 1),
                    sheet: sheet_name.clone(),
                    key: e.key.clone(),
                    members: members.iter().map(|m| FormulaId(*m)).collect(),
                    area: b.a1(),
                });
            }
            nodes.push(Node {
                kind,
                label: format!("{}!{}", quote_sheet(sheet_name), b.a1()),
                brings: Vec::new(),
            });
        }
        let formula_nodes = nodes.len();
        let constant_roles = |logic: &mut Logic| {
            for (f, e) in self.formulas.iter().enumerate() {
                if e.refs.is_empty() {
                    let id = FormulaId(u32::try_from(f).unwrap_or(u32::MAX));
                    logic.roles.push((id, Role::Constant));
                }
            }
        };
        if stopper.check() {
            constant_roles(&mut logic);
            logic.stopped = true;
            return logic;
        }

        // Phase 2: what each name reads, resolved once.
        let mut name_reads: Vec<Reads> = Vec::with_capacity(self.names.len());
        for i in 0..self.names.len() {
            if stopper.tick() {
                break;
            }
            name_reads.push(self.name_reads(i));
        }

        // Phase 3: what each formula reads. Cells count once per formula, also when it
        // reaches them through names.
        let mut syms = SymNodes {
            map: HashMap::new(),
            names: Vec::new(),
        };
        let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut reader_rects: Vec<Vec<Rect>> = vec![Vec::new(); nodes.len()];
        let mut events: Vec<Vec<Event>> = vec![Vec::new(); self.sheets.len()];
        let mut closures = Closures::new(self.names.len());
        let mut names_read: Vec<usize> = Vec::new();
        for f in &order {
            if stopper.stopped || stopper.tick() {
                break;
            }
            let e = entry(*f);
            let reader = formula_node[*f as usize];
            let ctx = Ctx {
                sheet: Some(e.sheet),
                scope: Some(&self.sheets[e.sheet as usize]),
                at: Some(e.at),
            };
            let reads = self.resolve(&e.refs, ctx);
            names_read.clear();
            for s in &reads.syms {
                self.add_sym(s, reader, &mut nodes, &mut edges, &mut syms);
                if let Sym::Name(i) = s {
                    names_read.push(*i);
                }
            }
            let mut cells = reads.rects.clone();
            if !names_read.is_empty() {
                match closures.get(&names_read, &name_reads, &mut stopper) {
                    Some(c) => cells.extend(c.iter().copied()),
                    None => break,
                }
            }
            let Some(cells) = disjoint(cells, &mut stopper) else {
                break;
            };
            for r in &cells {
                push_events(&mut events, r);
            }
            if let Some(list) = reader_rects.get_mut(reader) {
                list.extend(reads.rects);
            }
        }
        // Names read by formulas are readers too, and bring in what they refer to.
        let mut next = 0;
        while next < syms.names.len() && !stopper.stopped {
            let (i, reader) = syms.names[next];
            next += 1;
            let Some(reads) = name_reads.get(i) else {
                continue;
            };
            for s in &reads.syms {
                self.add_sym(s, reader, &mut nodes, &mut edges, &mut syms);
            }
            if reader_rects.len() <= reader {
                reader_rects.resize(reader + 1, Vec::new());
            }
            reader_rects[reader].extend(reads.rects.iter().copied());
        }
        // Cells a name refers to are parameters even when no formula reads the name.
        let mut named_events: Vec<Vec<Event>> = vec![Vec::new(); self.sheets.len()];
        for reads in &name_reads {
            for r in merge(reads.rects.clone()) {
                push_events(&mut named_events, &r);
            }
        }

        // Phase 4: the cells of each sheet, sorted, and one sweep per sheet for the counts.
        let mut per_sheet: Vec<Vec<(CellPos, PointKind)>> = vec![Vec::new(); self.sheets.len()];
        for (f, e) in self.formulas.iter().enumerate() {
            per_sheet[e.sheet as usize].push((
                e.at,
                PointKind::Formula(u32::try_from(f).unwrap_or(u32::MAX)),
            ));
        }
        for (s, at) in &self.constants {
            per_sheet[*s as usize].push((*at, PointKind::Constant));
        }
        let index: Vec<SheetIndex> = per_sheet.into_iter().map(SheetIndex::build).collect();
        let mut counts: Vec<Vec<u32>> = Vec::with_capacity(index.len());
        let mut named: Vec<Vec<bool>> = Vec::with_capacity(index.len());
        let mut tree = Fenwick::new();
        let mut named_tree = Fenwick::new();
        for (s, idx) in index.iter().enumerate() {
            if stopper.stopped || stopper.check() {
                break;
            }
            let (c, n) = sweep(
                idx,
                &mut events[s],
                &mut named_events[s],
                &mut tree,
                &mut named_tree,
                &mut stopper,
            );
            counts.push(c);
            named.push(n);
        }
        if stopper.stopped {
            constant_roles(&mut logic);
            logic.stopped = true;
            return logic;
        }

        // Roles and parameters are complete now.
        let mut formula_count = vec![0u32; self.formulas.len()];
        for (s, idx) in index.iter().enumerate() {
            for (i, (p, k)) in idx.points.iter().enumerate() {
                let c = counts[s][i];
                match k {
                    PointKind::Formula(f) => {
                        if let Some(slot) = formula_count.get_mut(*f as usize) {
                            *slot = c;
                        }
                    }
                    PointKind::Constant => {
                        if c > 0 || named[s][i] {
                            logic.parameters.push(ParamCell {
                                sheet: self.sheets[s].clone(),
                                at: *p,
                                read_by: c,
                            });
                        }
                    }
                }
            }
        }
        for (f, e) in self.formulas.iter().enumerate() {
            let id = FormulaId(u32::try_from(f).unwrap_or(u32::MAX));
            let role = if e.refs.is_empty() {
                Role::Constant
            } else if formula_count[f] > 0 {
                Role::Intermediate
            } else {
                Role::Result
            };
            logic.roles.push((id, role));
        }

        // Phase 5: edges and parameter blocks, reader by reader.
        let mut memo: HashMap<Rect, Rc<Hits>> = HashMap::new();
        let mut blocks: Vec<(usize, u32, Bounds)> = Vec::new();
        for (reader, rects) in reader_rects.into_iter().enumerate() {
            if stopper.stopped {
                break;
            }
            let mut targets: Vec<usize> = Vec::new();
            let mut constants: BTreeMap<u32, Bounds> = BTreeMap::new();
            for rect in merge(rects) {
                let hits = match memo.get(&rect) {
                    Some(h) => Rc::clone(h),
                    None => {
                        let Some((h, cost)) = hits(&index, &rect, &formula_node, &mut stopper)
                        else {
                            break;
                        };
                        let h = Rc::new(h);
                        if cost >= MEMO_MIN_CELLS {
                            memo.insert(rect, Rc::clone(&h));
                        }
                        h
                    }
                };
                targets.extend(hits.targets.iter().copied());
                if let Some(b) = hits.constants {
                    constants
                        .entry(rect.sheet)
                        .and_modify(|x| x.add(b))
                        .or_insert(b);
                }
            }
            targets.sort_unstable();
            targets.dedup();
            for t in targets {
                edges.insert((t, reader));
            }
            for (sheet, b) in constants {
                blocks.push((reader, sheet, b));
            }
        }
        if stopper.stopped {
            logic.stopped = true;
            return logic;
        }
        for (reader, sheet, b) in blocks {
            let n = nodes.len();
            nodes.push(Node {
                kind: Kind::Parameters,
                label: format!("{}!{}", quote_sheet(&self.sheets[sheet as usize]), b.a1()),
                brings: Vec::new(),
            });
            // Parameter blocks come before the names and externals a reader brings in.
            let at = nodes[reader]
                .brings
                .iter()
                .position(|x| nodes[*x].kind != Kind::Parameters)
                .unwrap_or(nodes[reader].brings.len());
            nodes[reader].brings.insert(at, n);
            edges.insert((n, reader));
        }

        // Final node order: formula nodes, then what each reader brings in, breadth first.
        let mut placed: Vec<usize> = (0..formula_nodes).collect();
        let mut position = vec![usize::MAX; nodes.len()];
        for (i, n) in placed.iter().enumerate() {
            position[*n] = i;
        }
        let mut cursor = 0;
        while cursor < placed.len() {
            let n = placed[cursor];
            cursor += 1;
            if !nodes[n].kind.reads() {
                continue;
            }
            for b in std::mem::take(&mut nodes[n].brings) {
                if position[b] == usize::MAX {
                    position[b] = placed.len();
                    placed.push(b);
                }
            }
        }
        let mut counters: HashMap<char, u32> = HashMap::new();
        let mut ids = vec![String::new(); nodes.len()];
        for n in &placed {
            let kind = nodes[*n].kind;
            let c = counters.entry(kind.prefix()).or_insert(0);
            *c += 1;
            ids[*n] = format!("{}{c}", kind.prefix());
            logic.graph.nodes.push((
                ids[*n].clone(),
                std::mem::take(&mut nodes[*n].label),
                kind.as_str().to_string(),
            ));
        }
        let mut ordered: Vec<(usize, usize)> = edges
            .into_iter()
            .filter(|(a, b)| position[*a] != usize::MAX && position[*b] != usize::MAX)
            .map(|(a, b)| (position[a], position[b]))
            .collect();
        ordered.sort_unstable();
        logic.graph.edges = ordered
            .into_iter()
            .map(|(a, b)| (ids[placed[a]].clone(), ids[placed[b]].clone()))
            .collect();
        logic
    }

    /// The node of `sym`, made and brought in by `reader` on first use, with an edge to
    /// `reader`.
    fn add_sym(
        &self,
        sym: &Sym,
        reader: usize,
        nodes: &mut Vec<Node>,
        edges: &mut BTreeSet<(usize, usize)>,
        syms: &mut SymNodes,
    ) {
        let n = match syms.map.get(sym) {
            Some(n) => *n,
            None => {
                let n = nodes.len();
                let (kind, label) = match sym {
                    Sym::Name(i) => {
                        syms.names.push((*i, n));
                        let e = &self.names[*i];
                        let label = match &e.scope {
                            Some(s) => format!("name:{}!{}", quote_sheet(s), e.name),
                            None => format!("name:{}", e.name),
                        };
                        (Kind::Name, label)
                    }
                    Sym::Unknown(anchor) => (Kind::Name, anchor.clone()),
                    Sym::Table(t) => (Kind::Name, format!("table:{t}")),
                    Sym::External(b) => (Kind::External, format!("ext:{b}")),
                };
                nodes.push(Node {
                    kind,
                    label,
                    brings: Vec::new(),
                });
                syms.map.insert(sym.clone(), n);
                if let Some(r) = nodes.get_mut(reader) {
                    r.brings.push(n);
                }
                n
            }
        };
        edges.insert((n, reader));
    }
}

/// The cells reached through sets of names, following names that refer to names. The walk
/// uses an explicit stack, so a long chain of names cannot overflow the thread stack, and a
/// cycle is walked once (every name in it reaches all of its cells). Results for repeated
/// name sets are kept while they stay small in total.
struct Closures {
    memo: HashMap<Vec<usize>, Rc<[Rect]>>,
    kept: usize,
    /// The walk number that last visited each name.
    seen: Vec<u32>,
    walk: u32,
    stack: Vec<usize>,
}

impl Closures {
    fn new(names: usize) -> Self {
        Closures {
            memo: HashMap::new(),
            kept: 0,
            seen: vec![0; names],
            walk: 0,
            stack: Vec::new(),
        }
    }

    /// The cells reached from `start`, or `None` when the stop callback asks to stop.
    fn get(
        &mut self,
        start: &[usize],
        reads: &[Reads],
        stopper: &mut Stopper,
    ) -> Option<Rc<[Rect]>> {
        let mut key = start.to_vec();
        key.sort_unstable();
        key.dedup();
        if let Some(done) = self.memo.get(&key) {
            return Some(Rc::clone(done));
        }
        self.walk = self.walk.wrapping_add(1);
        if self.walk == 0 {
            self.seen.iter_mut().for_each(|s| *s = 0);
            self.walk = 1;
        }
        let mut cells = Vec::new();
        self.stack.clear();
        self.stack.extend(key.iter().copied());
        while let Some(i) = self.stack.pop() {
            if stopper.tick() {
                return None;
            }
            match self.seen.get_mut(i) {
                Some(s) if *s != self.walk => *s = self.walk,
                _ => continue,
            }
            let Some(r) = reads.get(i) else {
                continue;
            };
            cells.extend(r.rects.iter().copied());
            for s in &r.syms {
                if let Sym::Name(j) = s {
                    self.stack.push(*j);
                }
            }
        }
        let cells: Rc<[Rect]> = merge(cells).into();
        if self.kept + cells.len() <= CLOSURE_MEMO_RECTS {
            self.kept += cells.len();
            self.memo.insert(key, Rc::clone(&cells));
        }
        Some(cells)
    }
}

/// The read counts of each cell of one sheet (formulas that read it) and whether a name
/// refers to it, from one sweep over the rows.
fn sweep(
    idx: &SheetIndex,
    events: &mut [Event],
    named_events: &mut [Event],
    tree: &mut Fenwick,
    named_tree: &mut Fenwick,
    stopper: &mut Stopper,
) -> (Vec<u32>, Vec<bool>) {
    tree.clear();
    named_tree.clear();
    events.sort_unstable_by_key(|e| e.row);
    named_events.sort_unstable_by_key(|e| e.row);
    let mut counts = Vec::with_capacity(idx.points.len());
    let mut named = Vec::with_capacity(idx.points.len());
    let (mut e, mut ne) = (0, 0);
    for (p, _) in &idx.points {
        while let Some(ev) = events.get(e).filter(|ev| ev.row <= p.row) {
            tree.add(ev.c1, ev.c2, ev.delta);
            e += 1;
            if stopper.tick() {
                return (counts, named);
            }
        }
        while let Some(ev) = named_events.get(ne).filter(|ev| ev.row <= p.row) {
            named_tree.add(ev.c1, ev.c2, ev.delta);
            ne += 1;
        }
        counts.push(u32::try_from(tree.at(p.col).max(0)).unwrap_or(u32::MAX));
        named.push(named_tree.at(p.col) > 0);
        if stopper.tick() {
            return (counts, named);
        }
    }
    (counts, named)
}

/// The formula nodes and constants inside one rectangle, with the number of cells visited.
/// `None` when the stop callback asks to stop.
fn hits(
    index: &[SheetIndex],
    rect: &Rect,
    formula_node: &[usize],
    stopper: &mut Stopper,
) -> Option<(Hits, usize)> {
    let mut h = Hits {
        targets: Vec::new(),
        constants: None,
    };
    let Some(sheet) = index.get(rect.sheet as usize) else {
        return Some((h, 0));
    };
    let mut cost = 0;
    let finished = sheet.each_in(rect, |i| {
        cost += 1;
        match sheet.points[i] {
            (_, PointKind::Formula(f)) => {
                if let Some(n) = formula_node.get(f as usize)
                    && h.targets.last() != Some(n)
                {
                    h.targets.push(*n);
                }
            }
            (p, PointKind::Constant) => add_bounds(&mut h.constants, Bounds::of(p)),
        }
        !stopper.tick()
    });
    if !finished {
        return None;
    }
    h.targets.sort_unstable();
    h.targets.dedup();
    Some((h, cost))
}
