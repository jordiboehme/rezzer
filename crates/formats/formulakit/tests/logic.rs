// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use formulakit::logic::{FormulaId, Logic, ParamCell, Role, Workbook};
use formulakit::{CellPos, Dialect, analyze};

/// Counts the heap of the thread that allocates, so tests that run side by side do not
/// disturb a measurement.
struct Counting;

thread_local! {
    static CURRENT: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn grow(n: usize) {
    let now = CURRENT.with(|c| {
        c.set(c.get() + n as isize);
        c.get()
    });
    PEAK.with(|p| p.set(p.get().max(now)));
}

fn shrink(n: usize) {
    CURRENT.with(|c| c.set(c.get() - n as isize));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        grow(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        shrink(l.size());
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        shrink(l.size());
        grow(new);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns its result with the peak heap it added on this thread, in bytes.
fn peak_heap<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = CURRENT.with(Cell::get);
    PEAK.with(|p| p.set(base));
    let out = f();
    let peak = PEAK.with(Cell::get);
    (out, usize::try_from(peak - base).unwrap_or(0))
}

/// `A1` style position, 0-based.
fn at(cell: &str) -> CellPos {
    let letters: String = cell.chars().take_while(char::is_ascii_alphabetic).collect();
    let row: u32 = cell[letters.len()..].parse().expect("row");
    let col = letters
        .bytes()
        .fold(0u32, |acc, b| acc * 26 + u32::from(b - b'A' + 1));
    CellPos {
        row: row - 1,
        col: col - 1,
    }
}

fn formula(wb: &mut Workbook, sheet: &str, cell: &str, text: &str) -> FormulaId {
    let pos = at(cell);
    let a = analyze(text, Dialect::Excel, pos).expect("formula parses");
    wb.add_formula(sheet, pos, &a)
}

fn role(logic: &Logic, id: FormulaId) -> Role {
    logic
        .roles
        .iter()
        .find(|(f, _)| *f == id)
        .map(|(_, r)| *r)
        .expect("role")
}

fn param(sheet: &str, cell: &str, read_by: u32) -> ParamCell {
    ParamCell {
        sheet: sheet.to_string(),
        at: at(cell),
        read_by,
    }
}

fn never() -> bool {
    false
}

#[test]
fn roles_from_reads() {
    let mut wb = Workbook::new();
    wb.add_constant("S", at("A1"));
    let b1 = formula(&mut wb, "S", "B1", "=A1*2");
    let c1 = formula(&mut wb, "S", "C1", "=B1+1");
    let d1 = formula(&mut wb, "S", "D1", "=1+2");
    let logic = wb.finish(&never);
    assert!(!logic.stopped);
    assert_eq!(role(&logic, b1), Role::Intermediate);
    assert_eq!(role(&logic, c1), Role::Result);
    assert_eq!(role(&logic, d1), Role::Constant);
    assert_eq!(logic.parameters, vec![param("S", "A1", 1)]);
}

#[test]
fn range_read_marks_members() {
    let mut wb = Workbook::new();
    for c in ["A1", "A2", "A3", "A4"] {
        wb.add_constant("S", at(c));
    }
    formula(&mut wb, "S", "B1", "=SUM(A1:A3)");
    let logic = wb.finish(&never);
    assert_eq!(
        logic.parameters,
        vec![
            param("S", "A1", 1),
            param("S", "A2", 1),
            param("S", "A3", 1)
        ]
    );
}

#[test]
fn read_by_counts_formulas_not_refs() {
    let mut wb = Workbook::new();
    for c in ["A1", "A2", "A3"] {
        wb.add_constant("S", at(c));
    }
    formula(&mut wb, "S", "B1", "=A1+$A$1");
    formula(&mut wb, "S", "B2", "=SUM(A1:A3)+A2");
    let logic = wb.finish(&never);
    assert_eq!(
        logic.parameters,
        vec![
            param("S", "A1", 2),
            param("S", "A2", 1),
            param("S", "A3", 1)
        ]
    );
}

#[test]
fn whole_column_does_not_expand() {
    let start = Instant::now();
    let (logic, peak) = peak_heap(|| {
        let mut wb = Workbook::new();
        for row in 0..10_000 {
            wb.add_constant("S", CellPos { row, col: 0 });
        }
        formula(&mut wb, "S", "B1", "=SUM(A:A)");
        formula(&mut wb, "T", "A1", "=SUM(S!1:1048576)");
        wb.finish(&never)
    });
    let took = start.elapsed();
    eprintln!("whole_column_does_not_expand: {took:?}, peak heap {peak} bytes");
    assert!(took < Duration::from_secs(1), "took {took:?}");
    assert!(peak < 50 * 1024 * 1024, "peak heap {peak} bytes");
    assert_eq!(logic.parameters.len(), 10_000);
    assert!(logic.parameters.iter().all(|p| p.read_by == 2));
}

#[test]
fn percent_of_total_stays_fast() {
    // Every row reads the whole column through an absolute range: 20,000 x 20,000 reads.
    let start = Instant::now();
    let mut wb = Workbook::new();
    for row in 1..=20_000u32 {
        wb.add_constant("S", CellPos { row, col: 1 });
    }
    for row in 1..=20_000u32 {
        let r = row + 1;
        formula(
            &mut wb,
            "S",
            &format!("C{r}"),
            &format!("=B{r}/SUM($B$2:$B$20001)"),
        );
    }
    let logic = wb.finish(&never);
    let took = start.elapsed();
    eprintln!("percent_of_total_stays_fast: {took:?}");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_eq!(logic.groups.len(), 1);
    assert_eq!(logic.parameters.len(), 20_000);
    assert!(logic.parameters.iter().all(|p| p.read_by == 20_000));
}

/// D2:D100 `=B2*C2` with constants in B2:C100.
fn copies() -> (Workbook, Vec<FormulaId>) {
    let mut wb = Workbook::new();
    let mut ids = Vec::new();
    for r in 2..=100 {
        wb.add_constant("S", at(&format!("B{r}")));
        wb.add_constant("S", at(&format!("C{r}")));
        ids.push(formula(
            &mut wb,
            "S",
            &format!("D{r}"),
            &format!("=B{r}*C{r}"),
        ));
    }
    (wb, ids)
}

#[test]
fn copies_fold_into_groups() {
    let (wb, ids) = copies();
    let logic = wb.finish(&never);
    assert_eq!(logic.groups.len(), 1);
    let g = &logic.groups[0];
    assert_eq!(g.id, "p1");
    assert_eq!(g.sheet, "S");
    assert_eq!(g.area, "D2:D100");
    assert_eq!(g.key, "=RC[-2]*RC[-1]");
    assert_eq!(g.members.len(), 99);
    assert_eq!(g.members, ids);
    assert!(ids.iter().all(|id| role(&logic, *id) == Role::Result));
}

#[test]
fn graph_is_group_level() {
    let (wb, _) = copies();
    let logic = wb.finish(&never);
    let g = &logic.graph;
    assert_eq!(
        g.nodes,
        vec![
            (
                "p1".to_string(),
                "S!D2:D100".to_string(),
                "group".to_string()
            ),
            (
                "v1".to_string(),
                "S!B2:C100".to_string(),
                "parameters".to_string()
            ),
        ]
    );
    assert_eq!(g.edges, vec![("v1".to_string(), "p1".to_string())]);
}

#[test]
fn graph_links_groups_formulas_names_and_externals() {
    let mut wb = Workbook::new();
    wb.add_constant("In", at("B1"));
    let rate = analyze("=In!$B$1", Dialect::Excel, at("A1")).expect("name parses");
    wb.add_name("rate", None, Some(&rate));
    wb.add_name(
        "fixed",
        None,
        Some(&analyze("=0.05", Dialect::Excel, at("A1")).expect("ok")),
    );
    for r in 1..=3 {
        formula(&mut wb, "Calc", &format!("A{r}"), &format!("=In!A{r}*rate"));
        wb.add_constant("In", at(&format!("A{r}")));
    }
    let total = formula(&mut wb, "Calc", "B1", "=SUM(A1:A3)+[1]Other!A1+fixed");
    let logic = wb.finish(&never);
    // Nodes: groups and single formulas in cell order, then what each reader brings in
    // (its parameter blocks, then names and externals in order of first use). Edges run
    // from what is read to the reader, in node order.
    assert_eq!(role(&logic, total), Role::Result);
    let kinds: Vec<(&str, &str, &str)> = logic
        .graph
        .nodes
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("p1", "Calc!A1:A3", "group"),
            ("f1", "Calc!B1", "formula"),
            ("v1", "In!A1:A3", "parameters"),
            ("n1", "name:rate", "name"),
            ("x1", "ext:[1]", "external"),
            ("n2", "name:fixed", "name"),
            ("v2", "In!B1", "parameters"),
        ]
    );
    let edges: Vec<(&str, &str)> = logic
        .graph
        .edges
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(
        edges,
        vec![
            ("p1", "f1"),
            ("v1", "p1"),
            ("n1", "p1"),
            ("x1", "f1"),
            ("n2", "f1"),
            ("v2", "n1"),
        ]
    );
    // The constant behind the name is a parameter read by the three formulas.
    assert!(logic.parameters.contains(&param("In", "B1", 3)));
}

#[test]
fn name_scope_and_3d_refs_resolve() {
    let mut wb = Workbook::new();
    for s in ["S1", "S2", "S3"] {
        wb.add_constant(s, at("A1"));
    }
    wb.add_constant("S2", at("B1"));
    let local = analyze("=S2!$B$1", Dialect::Excel, at("A1")).expect("ok");
    wb.add_name("X", Some("S2"), Some(&local));
    formula(&mut wb, "S2", "C1", "=x+SUM(S1:S3!A1)");
    // On another sheet the sheet-scoped name is not visible: no parameter through it.
    formula(&mut wb, "S1", "C1", "=X");
    let logic = wb.finish(&never);
    assert_eq!(
        logic.parameters,
        vec![
            param("S1", "A1", 1),
            param("S2", "A1", 1),
            param("S2", "B1", 1),
            param("S3", "A1", 1),
        ]
    );
}

#[test]
fn stop_callback_ends_early() {
    let (wb, _) = copies();
    let logic = wb.finish(&|| true);
    assert!(logic.stopped);
}
