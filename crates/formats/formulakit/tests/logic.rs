// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use formulakit::logic::{FormulaId, Logic, ParamCell, Role, TableArea, TableError, Workbook};
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

/// The wall-clock stop of the work-bound tests: far above any machine, so only a hang fails
/// it. The work itself is bounded by counted steps.
const SAFETY: Duration = Duration::from_secs(60);

/// Runs `f` and gives its result, the steps of workbook logic it did
/// ([`formulakit::logic::work_steps`], `None` in a release build) and its wall time.
fn counted<T>(f: impl FnOnce() -> T) -> (T, Option<u64>, Duration) {
    let before = formulakit::logic::work_steps();
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    let steps = before
        .zip(formulakit::logic::work_steps())
        .map(|(b, a)| a - b);
    (out, steps, took)
}

/// Checks a work bound: at most `max` steps (in a debug build, which counts them) and the
/// wall time under [`SAFETY`]. Each bound is the count measured when it was set (2026-10-27,
/// debug build, `--no-capture`), with half of it again as margin: a growth past that is a
/// change in the work, not a slow machine.
fn assert_work(what: &str, steps: Option<u64>, max: u64, took: Duration) {
    eprintln!("{what}: {steps:?} steps (bound {max}) in {took:?}");
    if cfg!(debug_assertions) {
        assert!(
            steps.is_some_and(|n| n <= max),
            "{what}: {steps:?} steps of work, bound {max}"
        );
    } else {
        assert_eq!(steps, None);
    }
    assert!(took < SAFETY, "{what}: took {took:?}");
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
    let ((logic, peak), steps, took) = counted(|| {
        peak_heap(|| {
            let mut wb = Workbook::new();
            for row in 0..10_000 {
                wb.add_constant("S", CellPos { row, col: 0 });
            }
            formula(&mut wb, "S", "B1", "=SUM(A:A)");
            formula(&mut wb, "T", "A1", "=SUM(S!1:1048576)");
            wb.finish(&never)
        })
    });
    eprintln!("whole_column_does_not_expand: peak heap {peak} bytes");
    // Measured once (2026-10-27): 30,017 steps, about three a constant cell.
    const STEPS: u64 = 30_017;
    assert_work(
        "whole_column_does_not_expand",
        steps,
        STEPS + STEPS / 2,
        took,
    );
    assert!(peak < 50 * 1024 * 1024, "peak heap {peak} bytes");
    assert_eq!(logic.parameters.len(), 10_000);
    assert!(logic.parameters.iter().all(|p| p.read_by == 2));
}

#[test]
fn percent_of_total_stays_fast() {
    // Every row reads the whole column through an absolute range: 20,000 x 20,000 reads.
    let (logic, steps, took) = counted(|| {
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
        wb.finish(&never)
    });
    // Measured once (2026-10-27): 80,022 steps, four a formula.
    const STEPS: u64 = 80_022;
    assert_work(
        "percent_of_total_stays_fast",
        steps,
        STEPS + STEPS / 2,
        took,
    );
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

#[test]
fn many_names_resolve_fast() {
    let (logic, steps, took) = counted(|| {
        let mut wb = Workbook::new();
        for i in 0..5_000u32 {
            wb.add_constant("In", CellPos { row: i, col: 0 });
            let a = analyze(
                &format!("=In!$A${}", i + 1),
                Dialect::Excel,
                CellPos::default(),
            )
            .expect("name parses");
            wb.add_name(&format!("Größe{i}"), None, Some(&a));
        }
        for row in 0..20_000u32 {
            let at = CellPos { row, col: 1 };
            let a = analyze(&format!("=GRÖSSE{}*2", row % 5_000), Dialect::Excel, at)
                .expect("formula parses");
            wb.add_formula("Calc", at, &a);
        }
        wb.finish(&never)
    });
    // Measured once (2026-10-27): 90,023 steps, about four a formula.
    const STEPS: u64 = 90_023;
    assert_work("many_names_resolve_fast", steps, STEPS + STEPS / 2, took);
    // Names match in any letter case, also beyond ASCII.
    assert_eq!(logic.parameters.len(), 5_000);
    assert!(logic.parameters.iter().all(|p| p.read_by == 4));
}

/// 20,000 running totals `=SUM($B$2:Bn)` with constants in column B.
fn running_totals(rows: u32) -> (Workbook, Vec<FormulaId>) {
    let mut wb = Workbook::new();
    let mut ids = Vec::new();
    for row in 1..=rows {
        wb.add_constant("S", CellPos { row, col: 1 });
        let at = CellPos { row, col: 2 };
        let a = analyze(&format!("=SUM($B$2:B{})", row + 1), Dialect::Excel, at)
            .expect("formula parses");
        ids.push(wb.add_formula("S", at, &a));
    }
    (wb, ids)
}

#[test]
fn stop_in_the_middle_keeps_only_certain_roles() {
    let run = || {
        let (wb, _) = running_totals(20_000);
        let calls = Cell::new(0u32);
        wb.finish(&|| {
            calls.set(calls.get() + 1);
            calls.get() > 5
        })
    };
    let logic = run();
    assert!(logic.stopped);
    assert!(logic.roles.iter().all(|(_, r)| *r != Role::Result));
    // Groups need no read work and survive a stop.
    assert_eq!(logic.groups.len(), 1);
    assert_eq!(logic.groups[0].area, "C2:C20001");
    // What survives a stop does not depend on hash order.
    assert_eq!(logic, run());
}

#[test]
fn running_totals_stay_near_linear() {
    let ((logic, ids), steps, took) = counted(|| {
        let (wb, ids) = running_totals(100_000);
        (wb.finish(&never), ids)
    });
    // Measured once (2026-10-27): 600,148 steps, six a formula.
    const STEPS: u64 = 600_148;
    assert_work(
        "running_totals_stay_near_linear",
        steps,
        STEPS + STEPS / 2,
        took,
    );
    assert!(!logic.stopped);
    assert_eq!(logic.groups.len(), 1);
    assert_eq!(logic.parameters.len(), 100_000);
    assert_eq!(logic.parameters[0].read_by, 100_000);
    assert_eq!(logic.parameters[99_999].read_by, 1);
    assert!(ids.iter().all(|id| role(&logic, *id) == Role::Result));
    assert_eq!(logic.graph.nodes.len(), 2);
    assert_eq!(
        logic.graph.edges,
        vec![("v1".to_string(), "p1".to_string())]
    );
}

#[test]
fn deep_name_chain_on_a_small_stack() {
    let worker = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut wb = Workbook::new();
            wb.add_constant("In", at("A1"));
            let first = analyze("=In!$A$1", Dialect::Excel, at("A1")).expect("ok");
            wb.add_name("nm_0", None, Some(&first));
            for k in 1..100_000 {
                let a = analyze(&format!("=nm_{}", k - 1), Dialect::Excel, at("A1")).expect("ok");
                wb.add_name(&format!("nm_{k}"), None, Some(&a));
            }
            formula(&mut wb, "Calc", "A1", "=nm_99999*2");
            wb.finish(&never)
        })
        .expect("thread starts");
    let logic = worker.join().expect("no stack overflow");
    assert_eq!(logic.parameters, vec![param("In", "A1", 1)]);
}

#[test]
fn chained_names_memory_is_bounded() {
    let (logic, peak) = peak_heap(|| {
        let mut wb = Workbook::new();
        for k in 0..6_000u32 {
            wb.add_constant("In", CellPos { row: k, col: 0 });
            let text = if k == 0 {
                "=In!$A$1".to_string()
            } else {
                format!("=nm_{}+In!$A${}", k - 1, k + 1)
            };
            let a = analyze(&text, Dialect::Excel, at("A1")).expect("ok");
            wb.add_name(&format!("nm_{k}"), None, Some(&a));
        }
        formula(&mut wb, "Calc", "A1", "=nm_5999");
        wb.finish(&never)
    });
    eprintln!("chained_names_memory_is_bounded: peak heap {peak} bytes");
    assert!(peak < 64 * 1024 * 1024, "peak heap {peak} bytes");
    assert_eq!(logic.parameters.len(), 6_000);
    assert!(logic.parameters.iter().all(|p| p.read_by == 1));
}

#[test]
fn name_cycle_counts_every_cell() {
    let mut wb = Workbook::new();
    wb.add_constant("In", at("A1"));
    wb.add_constant("In", at("A2"));
    let a = analyze("=B_+In!$A$1", Dialect::Excel, at("A1")).expect("ok");
    let b = analyze("=A_+In!$A$2", Dialect::Excel, at("A1")).expect("ok");
    wb.add_name("A_", None, Some(&a));
    wb.add_name("B_", None, Some(&b));
    formula(&mut wb, "Calc", "A1", "=A_");
    formula(&mut wb, "Calc", "A2", "=B_");
    let logic = wb.finish(&never);
    assert_eq!(
        logic.parameters,
        vec![param("In", "A1", 2), param("In", "A2", 2)]
    );
}

#[test]
fn tables_resolve_to_cells() {
    let mut wb = Workbook::new();
    for c in ["A1", "B1", "C1", "A2", "B2", "A3", "B3", "A4", "B4"] {
        wb.add_constant("S", at(c));
    }
    wb.add_table(
        "Sales",
        "S",
        TableArea {
            start: at("A1"),
            end: at("C4"),
            header_rows: 1,
            totals_rows: 0,
            columns: vec!["Price".into(), "Qty".into(), "Total".into()],
        },
    )
    .expect("table area is fine");
    let mut rows = Vec::new();
    for r in 2..=4 {
        // A formula inside the table may leave out the table name.
        rows.push(formula(
            &mut wb,
            "S",
            &format!("C{r}"),
            "=[@Price]*Sales[@qty]",
        ));
    }
    let total = formula(&mut wb, "S", "E1", "=SUM(Sales[Total])");
    formula(&mut wb, "S", "E2", "=COUNTA(Sales[[#Headers],[Price]])");
    formula(&mut wb, "S", "E3", "=SUM(Sales[Nope])");
    let logic = wb.finish(&never);
    assert!(
        rows.iter()
            .all(|id| role(&logic, *id) == Role::Intermediate)
    );
    assert_eq!(role(&logic, total), Role::Result);
    assert_eq!(
        logic.parameters,
        vec![
            param("S", "A1", 1),
            param("S", "A2", 1),
            param("S", "B2", 1),
            param("S", "A3", 1),
            param("S", "B3", 1),
            param("S", "A4", 1),
            param("S", "B4", 1),
        ]
    );
    let nodes: Vec<(&str, &str, &str)> = logic
        .graph
        .nodes
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    assert_eq!(
        nodes,
        vec![
            ("f1", "S!E1", "formula"),
            ("p1", "S!C2:C4", "group"),
            ("f2", "S!E2", "formula"),
            ("f3", "S!E3", "formula"),
            ("v1", "S!A2:B4", "parameters"),
            ("v2", "S!A1", "parameters"),
            ("n1", "table:Sales", "name"),
        ]
    );
    assert!(
        logic
            .graph
            .edges
            .contains(&("p1".to_string(), "f1".to_string()))
    );
}

#[test]
fn bare_table_name_reads_its_data_rows() {
    let mut wb = Workbook::new();
    for c in ["A1", "A2", "A3", "A4"] {
        wb.add_constant("S", at(c));
    }
    wb.add_table(
        "T",
        "S",
        TableArea {
            start: at("A1"),
            end: at("A4"),
            header_rows: 1,
            totals_rows: 1,
            columns: vec!["v".into()],
        },
    )
    .expect("table area is fine");
    formula(&mut wb, "S", "C1", "=SUM(t)");
    formula(&mut wb, "S", "C2", "=T[[#Totals],[v]]");
    let logic = wb.finish(&never);
    assert_eq!(
        logic.parameters,
        vec![
            param("S", "A2", 1),
            param("S", "A3", 1),
            param("S", "A4", 1)
        ]
    );
}

#[test]
fn damaged_table_area_is_refused() {
    let area = |end: &str, totals_rows: u32, columns: usize| TableArea {
        start: at("A1"),
        end: at(end),
        header_rows: 1,
        totals_rows,
        columns: (0..columns).map(|i| format!("c{i}")).collect(),
    };
    let mut wb = Workbook::new();
    wb.add_constant("S", at("E2"));
    wb.add_constant("S", at("A2"));
    let refused: Vec<TableError> = [
        ("T", area("B3", 0, 6)),
        ("U", area("B3", 5, 2)),
        (
            "V",
            TableArea {
                start: at("C1"),
                ..area("A3", 0, 1)
            },
        ),
    ]
    .into_iter()
    .filter_map(|(name, a)| wb.add_table(name, "S", a).err())
    .collect();
    assert_eq!(refused.len(), 3);
    assert!(refused[0].message.contains("6 column names for 2 columns"));
    assert!(refused[1].message.contains("totals"));
    assert!(refused[2].message.contains("after"));
    formula(&mut wb, "S", "G1", "=SUM(T[c5])+SUM(U[[#Totals],[c0]])");
    formula(&mut wb, "S", "G2", "=E2");
    let logic = wb.finish(&never);
    // The refused tables read nothing and do not disturb other counts.
    assert_eq!(logic.parameters, vec![param("S", "E2", 1)]);
    let tables: Vec<&str> = logic
        .graph
        .nodes
        .iter()
        .filter(|(_, l, _)| l.starts_with("table:"))
        .map(|(_, l, _)| l.as_str())
        .collect();
    assert_eq!(tables, vec!["table:T", "table:U"]);
}

#[test]
fn many_area_name_read_by_many_formulas_stays_small() {
    // A name with 700 scattered cells, read by 30,000 formulas.
    let cells: Vec<String> = (0..700).map(|i| format!("In!$A${}", 2 * i + 1)).collect();
    let (logic, peak) = peak_heap(|| {
        let mut wb = Workbook::new();
        for i in 0..700u32 {
            wb.add_constant("In", CellPos { row: 2 * i, col: 0 });
        }
        let pick = analyze(
            &format!("=SUM({})", cells.join(",")),
            Dialect::Excel,
            at("A1"),
        )
        .expect("name parses");
        wb.add_name("pick", None, Some(&pick));
        for row in 0..30_000u32 {
            let at = CellPos { row, col: 4 };
            let a = analyze(&format!("=pick*D{}", row + 1), Dialect::Excel, at)
                .expect("formula parses");
            wb.add_formula("Calc", at, &a);
        }
        wb.finish(&never)
    });
    eprintln!("many_area_name_read_by_many_formulas_stays_small: peak heap {peak} bytes");
    assert!(peak < 64 * 1024 * 1024, "peak heap {peak} bytes");
    assert_eq!(logic.parameters.len(), 700);
    assert!(logic.parameters.iter().all(|p| p.read_by == 30_000));
}
