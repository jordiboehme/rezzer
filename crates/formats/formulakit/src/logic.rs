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
//! holds, not a million cells.
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

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::refs::col_letters;
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
/// table (`n1`, ..., label `name:rate`, `name:Sheet1!rate` or `table:Table1`) and
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
    /// The stop callback ended the work early. Then the result is incomplete: `roles` holds
    /// only the roles known for certain, and `parameters` and `graph` can be empty.
    pub stopped: bool,
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

/// Collects a workbook's formulas, constants and names. See the [module docs](self).
///
/// Sheets are known by name (case-insensitive) in the order they first appear in
/// [`add_formula`](Self::add_formula) or [`add_constant`](Self::add_constant). A 3D
/// reference (`Sheet1:Sheet3!A1`) covers the sheets between its ends in that order, so add
/// the cells sheet by sheet in workbook order.
#[derive(Default)]
pub struct Workbook {
    sheets: Vec<String>,
    sheet_index: HashMap<String, u32>,
    formulas: Vec<FormulaEntry>,
    constants: Vec<(u32, CellPos)>,
    names: Vec<NameEntry>,
    /// Uppercase name and scope to the first name added with them.
    name_index: HashMap<(String, Option<String>), usize>,
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

/// The same cells as `rects`, each covered once, so a formula counts a cell once.
fn disjoint(rects: &[Rect]) -> Vec<Rect> {
    let mut cells: Vec<Rect> = Vec::new();
    let mut ranges: Vec<Rect> = Vec::new();
    let mut seen = HashSet::new();
    for r in rects {
        if seen.insert(*r) {
            if r.is_cell() {
                cells.push(*r);
            } else {
                ranges.push(*r);
            }
        }
    }
    let mut out: Vec<Rect> = Vec::new();
    let mut pieces = Vec::new();
    let mut next = Vec::new();
    for r in ranges {
        pieces.clear();
        pieces.push(r);
        for o in &out {
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
        if !out[..range_count].iter().any(|o| c.overlaps(o)) {
            out.push(c);
        }
    }
    out
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
        points.sort_by_key(|(p, k)| (*p, matches!(k, PointKind::Constant)));
        points.dedup_by_key(|(p, k)| (*p, *k));
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

/// Something a reader reads that is not a cell: a name, a table or another workbook.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Sym {
    /// A defined name added with [`Workbook::add_name`].
    Name(usize),
    /// A name no [`Workbook::add_name`] defined, by its anchor.
    Unknown(String),
    Table(String),
    External(String),
}

/// What one reader (a formula, or a defined name) reads.
#[derive(Default)]
struct Reads {
    rects: Vec<Rect>,
    syms: Vec<Sym>,
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
#[derive(Clone, Copy)]
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

/// Per distinct rectangle: how many formulas read it (for counts), which readers read it
/// directly (for edges) and whether a defined name refers to it.
#[derive(Default)]
struct Agg {
    mult: u32,
    readers: Vec<usize>,
    named: bool,
}

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

    /// Resolves the references of a reader. `sheet` is the sheet unqualified cell
    /// references point into, `scope` the sheet name lookups start from.
    fn resolve(&self, refs: &[Ref], sheet: Option<u32>, scope: Option<&str>) -> Reads {
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
                        None => sheet.into_iter().collect(),
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
                Ref::Name { name, sheet: s } => {
                    let sym = match self.lookup(name, s.as_deref(), scope) {
                        Some(i) => Sym::Name(i),
                        None => Sym::Unknown(anchor_of(scope.unwrap_or(""), r)),
                    };
                    reads.syms.push(sym);
                }
                Ref::Table { name, .. } => reads.syms.push(Sym::Table(name.clone())),
                Ref::External { book, .. } => reads.syms.push(Sym::External(book.clone())),
            }
        }
        reads
    }

    fn name_reads(&self, i: usize) -> Reads {
        let Some(n) = self.names.get(i) else {
            return Reads::default();
        };
        let sheet = n.scope.as_deref().and_then(|s| self.find_sheet(s));
        match &n.refs {
            Some(refs) => self.resolve(refs, sheet, n.scope.as_deref()),
            None => Reads::default(),
        }
    }

    /// The cells a name reads, following names it refers to; a cycle adds nothing more.
    fn name_cells(
        i: usize,
        reads: &[Reads],
        memo: &mut HashMap<usize, Vec<Rect>>,
        open: &mut HashSet<usize>,
    ) -> Vec<Rect> {
        if let Some(done) = memo.get(&i) {
            return done.clone();
        }
        if !open.insert(i) {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(r) = reads.get(i) {
            out.extend(r.rects.iter().copied());
            for s in &r.syms {
                if let Sym::Name(j) = s {
                    out.extend(Self::name_cells(*j, reads, memo, open));
                }
            }
        }
        open.remove(&i);
        let out = disjoint(&out);
        memo.insert(i, out.clone());
        out
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
        if stopper.check() {
            logic.stopped = true;
            return logic;
        }

        // Formulas in cell order, folded into groups by sheet and pattern key.
        let mut order: Vec<u32> =
            (0..u32::try_from(self.formulas.len()).unwrap_or(u32::MAX)).collect();
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
        let mut group_members: Vec<(usize, Vec<u32>)> = Vec::new();
        for f in &order {
            if formula_node[*f as usize] != usize::MAX {
                continue;
            }
            let e = entry(*f);
            let members = by_key
                .get(&(e.sheet, e.key.as_str()))
                .cloned()
                .unwrap_or_default();
            let sheet = quote_sheet(&self.sheets[e.sheet as usize]);
            let n = nodes.len();
            if members.len() >= 2 {
                let mut b = Bounds::of(e.at);
                for m in &members {
                    b.add(Bounds::of(entry(*m).at));
                    formula_node[*m as usize] = n;
                }
                nodes.push(Node {
                    kind: Kind::Group,
                    label: format!("{sheet}!{}", b.a1()),
                    brings: Vec::new(),
                });
                group_members.push((n, members));
            } else {
                formula_node[*f as usize] = n;
                nodes.push(Node {
                    kind: Kind::Formula,
                    label: format!("{sheet}!{}", Bounds::of(e.at).a1()),
                    brings: Vec::new(),
                });
            }
        }
        let formula_nodes = nodes.len();

        // What each name reads, and the cells it reaches through other names.
        let name_reads: Vec<Reads> = (0..self.names.len()).map(|i| self.name_reads(i)).collect();
        let mut memo = HashMap::new();
        let mut open = HashSet::new();

        // What each formula reads; symbols become nodes in order of first use.
        let mut sym_node: HashMap<Sym, usize> = HashMap::new();
        let mut aggs: HashMap<Rect, Agg> = HashMap::new();
        let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut name_queue: Vec<(usize, usize)> = Vec::new();
        let mut add_sym = |sym: &Sym,
                           reader: usize,
                           nodes: &mut Vec<Node>,
                           edges: &mut BTreeSet<(usize, usize)>,
                           name_queue: &mut Vec<(usize, usize)>| {
            let n = match sym_node.get(sym) {
                Some(n) => *n,
                None => {
                    let n = nodes.len();
                    let (kind, label) = match sym {
                        Sym::Name(i) => {
                            name_queue.push((*i, n));
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
                    sym_node.insert(sym.clone(), n);
                    nodes[reader].brings.push(n);
                    n
                }
            };
            edges.insert((n, reader));
        };
        let mut formula_cells: Vec<Rect> = Vec::new();
        for f in &order {
            if stopper.tick() {
                break;
            }
            let e = entry(*f);
            let reader = formula_node[*f as usize];
            let sheet_name = &self.sheets[e.sheet as usize];
            let reads = self.resolve(&e.refs, Some(e.sheet), Some(sheet_name));
            formula_cells.clear();
            formula_cells.extend(reads.rects.iter().copied());
            for r in &reads.rects {
                let agg = aggs.entry(*r).or_default();
                if agg.readers.last() != Some(&reader) {
                    agg.readers.push(reader);
                }
            }
            for s in &reads.syms {
                add_sym(s, reader, &mut nodes, &mut edges, &mut name_queue);
                if let Sym::Name(i) = s {
                    formula_cells.extend(Self::name_cells(*i, &name_reads, &mut memo, &mut open));
                }
            }
            for r in disjoint(&formula_cells) {
                aggs.entry(r).or_default().mult += 1;
            }
        }
        // Names read by formulas are readers too, and bring in what they refer to.
        let mut next = 0;
        while next < name_queue.len() && !stopper.stopped {
            let (i, reader) = name_queue[next];
            next += 1;
            let Some(reads) = name_reads.get(i) else {
                continue;
            };
            for r in &reads.rects {
                let agg = aggs.entry(*r).or_default();
                if agg.readers.last() != Some(&reader) {
                    agg.readers.push(reader);
                }
            }
            for s in &reads.syms {
                add_sym(s, reader, &mut nodes, &mut edges, &mut name_queue);
            }
        }
        // Cells a name refers to are parameters even when no formula reads the name.
        for reads in &name_reads {
            for r in &reads.rects {
                aggs.entry(*r).or_default().named = true;
            }
        }

        // The cells of each sheet, sorted.
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
        let mut counts: Vec<Vec<u32>> = index.iter().map(|i| vec![0; i.points.len()]).collect();
        let mut named: Vec<Vec<bool>> = index.iter().map(|i| vec![false; i.points.len()]).collect();

        // Visit each distinct rectangle once.
        let mut params: HashMap<(usize, u32), Bounds> = HashMap::new();
        let mut targets: Vec<usize> = Vec::new();
        for (rect, agg) in &mut aggs {
            if stopper.stopped || stopper.check() {
                break;
            }
            let Some(sheet) = index.get(rect.sheet as usize) else {
                continue;
            };
            let s = rect.sheet as usize;
            targets.clear();
            let mut constants: Option<Bounds> = None;
            let finished = sheet.each_in(rect, |i| {
                counts[s][i] += agg.mult;
                named[s][i] |= agg.named;
                match sheet.points[i] {
                    (_, PointKind::Formula(f)) => {
                        let n = formula_node[f as usize];
                        if targets.last() != Some(&n) {
                            targets.push(n);
                        }
                    }
                    (p, PointKind::Constant) => match &mut constants {
                        Some(b) => b.add(Bounds::of(p)),
                        None => constants = Some(Bounds::of(p)),
                    },
                }
                !stopper.tick()
            });
            if !finished {
                break;
            }
            targets.sort_unstable();
            targets.dedup();
            agg.readers.sort_unstable();
            agg.readers.dedup();
            for reader in &agg.readers {
                for t in &targets {
                    edges.insert((*t, *reader));
                }
                if let Some(b) = constants {
                    params
                        .entry((*reader, rect.sheet))
                        .and_modify(|x| x.add(b))
                        .or_insert(b);
                }
            }
        }
        let complete = !stopper.stopped;

        // Roles.
        let read_at = |f: usize| -> bool {
            let e = &self.formulas[f];
            let idx = &index[e.sheet as usize];
            let i = idx
                .points
                .partition_point(|(p, k)| (*p, matches!(k, PointKind::Constant)) < (e.at, false));
            counts[e.sheet as usize].get(i).is_some_and(|c| *c > 0)
        };
        for (f, e) in self.formulas.iter().enumerate() {
            let id = FormulaId(u32::try_from(f).unwrap_or(u32::MAX));
            let role = if e.refs.is_empty() {
                Some(Role::Constant)
            } else if read_at(f) {
                Some(Role::Intermediate)
            } else if complete {
                Some(Role::Result)
            } else {
                None
            };
            if let Some(role) = role {
                logic.roles.push((id, role));
            }
        }
        if !complete {
            logic.stopped = true;
            return logic;
        }

        // Parameter blocks: one node per reader and sheet.
        let mut blocks: Vec<((usize, u32), Bounds)> = params.into_iter().collect();
        blocks.sort_by_key(|((reader, sheet), _)| (*reader, *sheet));
        for ((reader, sheet), b) in blocks {
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
            for b in nodes[n].brings.clone() {
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
                nodes[*n].label.clone(),
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

        // Groups.
        for (n, members) in group_members {
            let first = entry(members[0]);
            let label = &nodes[n].label;
            let area = label.rsplit('!').next().unwrap_or("").to_string();
            logic.groups.push(Group {
                id: ids[n].clone(),
                sheet: self.sheets[first.sheet as usize].clone(),
                key: first.key.clone(),
                members: members.into_iter().map(FormulaId).collect(),
                area,
            });
        }

        // Parameters.
        for (s, idx) in index.iter().enumerate() {
            for (i, (p, k)) in idx.points.iter().enumerate() {
                if *k != PointKind::Constant {
                    continue;
                }
                let c = counts[s][i];
                if c > 0 || named[s][i] {
                    logic.parameters.push(ParamCell {
                        sheet: self.sheets[s].clone(),
                        at: *p,
                        read_by: c,
                    });
                }
            }
        }
        logic
    }
}
