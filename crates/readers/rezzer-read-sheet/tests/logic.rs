// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The workbook logic: parameters with labels and units, roles, pattern groups, the
//! dependency graph and the stored-results warning.

mod common;

use common::*;
use rezzer_model::{Graph, Parameter, Role, Severity};

/// A text cell.
fn text(at: &str, s: &str) -> String {
    format!("<c r=\"{at}\" t=\"inlineStr\"><is><t>{s}</t></is></c>")
}

/// A number cell with an optional style.
fn num(at: &str, v: f64, style: Option<u32>) -> String {
    let s = style.map(|s| format!(" s=\"{s}\"")).unwrap_or_default();
    format!("<c r=\"{at}\"{s}><v>{v}</v></c>")
}

/// A formula cell with an optional cached number.
fn formula_cell(at: &str, f: &str, cached: Option<f64>) -> String {
    let v = cached.map(|v| format!("<v>{v}</v>")).unwrap_or_default();
    format!("<c r=\"{at}\"><f>{f}</f>{v}</c>")
}

fn row(n: u32, cells: &[String]) -> String {
    format!("<row r=\"{n}\">{}</row>", cells.concat())
}

fn param<'a>(doc: &'a Document, anchor: &str) -> &'a Parameter {
    doc.facets
        .parameters
        .iter()
        .find(|p| p.anchor.0 == anchor)
        .unwrap_or_else(|| panic!("no parameter {anchor}: {:#?}", doc.facets.parameters))
}

fn unit(p: &Parameter) -> (Option<&str>, Option<&str>) {
    (p.unit.as_deref(), p.unit_source.as_deref())
}

fn number(v: f64) -> Option<Value> {
    Some(Value::Number {
        value: v,
        unit: None,
    })
}

fn graph(doc: &Document) -> &Graph {
    doc.facets
        .graphs
        .iter()
        .find(|g| g.id == "dependencies")
        .unwrap_or_else(|| panic!("no dependency graph: {:#?}", doc.facets.graphs))
}

fn stale_losses(doc: &Document) -> Vec<&str> {
    doc.losses
        .iter()
        .filter(|l| l.what.starts_with("stored results may be old"))
        .map(|l| l.what.as_str())
        .collect()
}

#[test]
fn parameters_with_label_and_unit() {
    let rows = [
        row(
            1,
            &[
                text("A1", "Excavated diameter (m)"),
                num("B1", 10.0, None),
                formula_cell("C1", "B1*2", Some(20.0)),
                formula_cell("D1", "SUM(B2:B5)", Some(309.0)),
            ],
        ),
        row(2, &[text("A2", "Load [kN]"), num("B2", 5.0, None)]),
        row(3, &[text("A3", "Span in mm"), num("B3", 300.0, None)]),
        // A one-letter symbol that starts the last word is a variable, not a unit.
        row(4, &[text("A4", "Height (h)"), num("B4", 3.0, None)]),
        // A bare trailing word is never a unit.
        row(5, &[text("A5", "Type A"), num("B5", 1.0, None)]),
        row(7, &[text("E7", "t (m)")]),
        row(
            8,
            &[num("E8", 0.1, None), formula_cell("F8", "E8*2", Some(0.2))],
        ),
        row(
            9,
            &[num("E9", 0.2, None), formula_cell("F9", "E9*2", Some(0.4))],
        ),
    ];
    let out = read(&Book::default().sheet("S", &ws("", &rows.concat(), "")));
    let doc = &out.document;

    let d = param(doc, "S!B1");
    assert_eq!(d.label.as_deref(), Some("Excavated diameter"));
    assert_eq!(d.value, number(10.0));
    assert_eq!(unit(d), (Some("m"), Some("label")));
    assert_eq!(d.read_by, 1);
    assert_eq!(d.name, None);

    let load = param(doc, "S!B2");
    assert_eq!(load.label.as_deref(), Some("Load"));
    assert_eq!(unit(load), (Some("kN"), Some("label")));
    let span = param(doc, "S!B3");
    assert_eq!(span.label.as_deref(), Some("Span"));
    assert_eq!(unit(span), (Some("mm"), Some("label")));
    let height = param(doc, "S!B4");
    assert_eq!(height.label.as_deref(), Some("Height (h)"));
    assert_eq!(unit(height), (None, None));
    let kind = param(doc, "S!B5");
    assert_eq!(kind.label.as_deref(), Some("Type A"));
    assert_eq!(unit(kind), (None, None));

    // The column header above a block of values labels each of them.
    for anchor in ["S!E8", "S!E9"] {
        let t = param(doc, anchor);
        assert_eq!(t.label.as_deref(), Some("t"), "{anchor}");
        assert_eq!(unit(t), (Some("m"), Some("header")), "{anchor}");
    }
    // Formula cells and text cells nobody reads are no parameters.
    assert!(!doc.facets.parameters.iter().any(|p| p.anchor.0 == "S!C1"));
    assert!(!doc.facets.parameters.iter().any(|p| p.anchor.0 == "S!A1"));
}

#[test]
fn unit_from_number_format() {
    let rows = [
        row(
            1,
            &[
                text("A1", "Length"),
                num("B1", 10.0, Some(4)),
                formula_cell("C1", "SUM(B1:B6)", Some(1.0)),
            ],
        ),
        row(2, &[text("A2", "Price"), num("B2", 2.5, Some(5))]),
        row(3, &[text("A3", "Cost"), num("B3", 100.0, Some(6))]),
        row(4, &[text("A4", "Unit weight"), num("B4", 22.0, Some(7))]),
        row(5, &[text("A5", "Start"), num("B5", 3.0, Some(8))]),
        row(6, &[text("A6", "Rate"), num("B6", 0.05, Some(1))]),
    ];
    let out = read(&Book::default().sheet("S", &ws("", &rows.concat(), "")));
    let doc = &out.document;
    assert_eq!(unit(param(doc, "S!B1")), (Some("m"), Some("number-format")));
    assert_eq!(unit(param(doc, "S!B2")), (Some("€"), Some("number-format")));
    assert_eq!(unit(param(doc, "S!B3")), (Some("$"), Some("number-format")));
    assert_eq!(
        unit(param(doc, "S!B4")),
        (Some("kN/m3"), Some("number-format"))
    );
    // A literal that is no unit gives no unit.
    assert_eq!(unit(param(doc, "S!B5")), (None, None));
    // A percent format makes the value a percent.
    let rate = param(doc, "S!B6");
    assert_eq!(rate.value, Some(Value::Percent { value: 0.05 }));
    assert_eq!(rate.label.as_deref(), Some("Rate"));
}

#[test]
fn unit_from_convert_argument() {
    let rows = [
        row(
            1,
            &[
                text("A1", "Distance"),
                num("B1", 5.0, None),
                formula_cell("C1", "CONVERT(B1,\"mi\",\"km\")", Some(8.04672)),
            ],
        ),
        row(
            2,
            &[
                text("A2", "Mass"),
                num("B2", 2.0, None),
                formula_cell("C2", "CONVERT(B2*1,\"kg\",\"lbm\")", Some(4.4)),
            ],
        ),
        row(
            3,
            &[
                text("A3", "Speed"),
                num("B3", 2.0, None),
                formula_cell("C3", "CONVERT(B3,\"mph\",\"m/s\")", Some(0.9)),
            ],
        ),
    ];
    let out = read(&Book::default().sheet("S", &ws("", &rows.concat(), "")));
    let doc = &out.document;
    let d = param(doc, "S!B1");
    assert_eq!(d.label.as_deref(), Some("Distance"));
    assert_eq!(unit(d), (Some("mi"), Some("convert")));
    // The first argument is no plain cell: nothing is said about B2.
    assert_eq!(unit(param(doc, "S!B2")), (None, None));
    assert_eq!(unit(param(doc, "S!B3")), (Some("mph"), Some("convert")));
}

#[test]
fn name_used_as_variable_is_a_parameter() {
    let out = corpus("ex012.PVT_z_factor.xlsm");
    let doc = &out.document;
    let gg = param(doc, "z!L3");
    assert_eq!(gg.name.as_deref(), Some("gg"));
    assert_eq!(gg.value, number(0.75));
    assert!(gg.read_by > 0, "{gg:?}");
    let zcor = param(doc, "z!L4");
    assert_eq!(zcor.name.as_deref(), Some("zcor"));
    assert_eq!(zcor.value, number(2.0));

    // A name on a cell and a name with a literal value, both read as variables.
    let rows = [
        row(
            1,
            &[
                text("A1", "Rate"),
                num("B1", 7.0, None),
                formula_cell("C1", "B1*fixed", Some(0.35)),
            ],
        ),
        row(2, &[formula_cell("C2", "rate+FIXED", Some(7.05))]),
    ];
    let book = Book::default()
        .sheet("S", &ws("", &rows.concat(), ""))
        .names(
            "<definedName name=\"fixed\">0.05</definedName><definedName name=\"rate\">S!$B$1</definedName>",
        );
    let out = read(&book);
    let doc = &out.document;
    let rate = param(doc, "S!B1");
    assert_eq!(rate.name.as_deref(), Some("rate"));
    assert_eq!(rate.label.as_deref(), Some("Rate"));
    assert_eq!(rate.read_by, 2);
    let fixed = param(doc, "name:fixed");
    assert_eq!(fixed.name.as_deref(), Some("fixed"));
    assert_eq!(fixed.value, number(0.05));
    assert_eq!(fixed.read_by, 2);
    assert_eq!(unit(fixed), (None, None));
}

/// A1:A3 values, B1:B3 copies of `=A1*2`, C1 the sum, D1 a constant formula.
fn small_model() -> Book {
    let rows = [
        row(
            1,
            &[
                num("A1", 1.0, None),
                formula_cell("B1", "A1*2", Some(2.0)),
                formula_cell("C1", "SUM(B1:B3)", Some(12.0)),
                formula_cell("D1", "1+2", Some(3.0)),
            ],
        ),
        row(
            2,
            &[num("A2", 2.0, None), formula_cell("B2", "A2*2", Some(4.0))],
        ),
        row(
            3,
            &[num("A3", 3.0, None), formula_cell("B3", "A3*2", Some(6.0))],
        ),
    ];
    Book::default().sheet("S", &ws("", &rows.concat(), ""))
}

#[test]
fn roles_and_patterns_set() {
    let out = read(&small_model());
    let doc = &out.document;
    for anchor in ["S!B1", "S!B2", "S!B3"] {
        let f = formula(doc, anchor);
        assert_eq!(f.role, Some(Role::Intermediate), "{anchor}");
        assert_eq!(f.pattern.as_deref(), Some("p1"), "{anchor}");
        assert_eq!(f.range.as_ref().map(|a| a.0.as_str()), Some("S!B1:B3"));
    }
    let sum = formula(doc, "S!C1");
    assert_eq!(sum.role, Some(Role::Result));
    assert_eq!(sum.pattern, None);
    assert_eq!(sum.range, None);
    assert_eq!(formula(doc, "S!D1").role, Some(Role::Constant));

    // 2,133 formulas fold into a few patterns.
    let out = corpus("ex012.PVT_z_factor.xlsm");
    let doc = &out.document;
    let mut patterns: Vec<&str> = doc
        .facets
        .formulas
        .iter()
        .filter_map(|f| f.pattern.as_deref())
        .collect();
    let members = patterns.len();
    patterns.sort_unstable();
    patterns.dedup();
    assert!(members > 2_000, "{members} formulas in patterns");
    assert!(patterns.len() <= 12, "{} patterns", patterns.len());
    assert!(
        doc.facets
            .formulas
            .iter()
            .filter(|f| f.kind != Some(FormulaKind::Name))
            .all(|f| f.role.is_some())
    );
}

#[test]
fn dependency_graph_added() {
    let out = read(&small_model());
    let doc = &out.document;
    let g = graph(doc);
    assert_eq!(g.kind, "dependency");
    let node = |label: &str| {
        g.nodes
            .iter()
            .find(|n| n.label == label)
            .unwrap_or_else(|| panic!("no node {label}: {:#?}", g.nodes))
    };
    let group = node("S!B1:B3");
    assert_eq!(group.id, "p1");
    assert_eq!(group.kind.as_deref(), Some("group"));
    assert_eq!(group.anchor.as_ref().map(|a| a.0.as_str()), Some("S!B1:B3"));
    let sum = node("S!C1");
    assert_eq!(sum.kind.as_deref(), Some("formula"));
    let inputs = node("S!A1:A3");
    assert_eq!(inputs.kind.as_deref(), Some("parameters"));
    assert_eq!(node("S!D1").kind.as_deref(), Some("formula"));
    assert_eq!(g.nodes.len(), 4, "{:#?}", g.nodes);
    let edges: Vec<(&str, &str, &str)> = g
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str(), e.kind.as_str()))
        .collect();
    assert_eq!(edges.len(), 2, "{edges:?}");
    assert!(
        edges.contains(&(inputs.id.as_str(), "p1", "feeds")),
        "{edges:?}"
    );
    assert!(
        edges.contains(&("p1", sum.id.as_str(), "feeds")),
        "{edges:?}"
    );
}

#[test]
fn manual_calc_warns_results_may_be_old() {
    let mut book = small_model();
    book.calc_pr = "<calcPr calcMode=\"manual\" iterate=\"1\"/>".into();
    let out = read(&book);
    let doc = &out.document;
    let stale: Vec<_> = doc
        .losses
        .iter()
        .filter(|l| l.what.starts_with("stored results may be old"))
        .collect();
    assert_eq!(stale.len(), 1, "{:#?}", doc.losses);
    assert_eq!(stale[0].why, LossReason::Uncertain);
    assert_eq!(stale[0].severity, Severity::Warning);
    assert_eq!(stale[0].location, "document");
    assert_eq!(
        stale[0].what,
        "stored results may be old: calculation is set to manual; iterative calculation is on"
    );
    assert_eq!(out.completeness, Completeness::Complete);

    // A formula without a stored result.
    let rows = row(
        1,
        &[
            num("A1", 1.0, None),
            formula_cell("B1", "A1*2", None),
            formula_cell("C1", "A1*3", Some(3.0)),
        ],
    );
    let out = read(&Book::default().sheet("S", &ws("", &rows, "")));
    assert_eq!(
        stale_losses(&out.document),
        ["stored results may be old: 1 formula has no stored result"]
    );

    // Automatic calculation with every result stored: no warning.
    let out = read(&small_model());
    assert!(stale_losses(&out.document).is_empty());
}

#[test]
fn unread_formula_leaves_results_open() {
    // C1 cannot be parsed, so nobody can tell whether it reads B1.
    let rows = row(
        1,
        &[
            num("A1", 1.0, None),
            formula_cell("B1", "A1*2", Some(2.0)),
            formula_cell("C1", "SUM(B1", Some(2.0)),
        ],
    );
    let out = read(&Book::default().sheet("S", &ws("", &rows, "")));
    let doc = &out.document;
    let b1 = formula(doc, "S!B1");
    assert_eq!(b1.role, None);
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Uncertain
            && l.location == "document"
            && l.what.contains("1 formula could not be analyzed")),
        "{:#?}",
        doc.losses
    );
    // What is certain stays: A1 is read by B1.
    assert_eq!(param(doc, "S!A1").read_by, 1);
}

#[test]
fn tunnel_parameters_have_labels() {
    let out = corpus("tunnel_design_calculations.xlsx");
    let doc = &out.document;
    // `Excavated diameter, D | 10 | m`: the unit right after the value.
    let d = param(doc, "Structural_Lining!B5");
    assert_eq!(d.label.as_deref(), Some("Excavated diameter, D"));
    assert_eq!(d.value, number(10.0));
    assert_eq!(unit(d), (Some("m"), Some("unit-cell")));
    let gamma = param(doc, "Structural_Lining!B8");
    assert_eq!(unit(gamma), (Some("kN/m3"), Some("unit-cell")));
    // The input units of all three calculation sheets stay.
    for (anchor, u) in [
        ("Structural_Lining!B9", "deg"),
        ("Structural_Lining!B11", "MPa"),
        ("Ventilation!B5", "m"),
        ("Ventilation!B15", "K"),
        ("Ventilation!B17", "N"),
        ("Drainage!B9", "mm/hr"),
        ("Drainage!B13", "min"),
    ] {
        assert_eq!(
            unit(param(doc, anchor)),
            (Some(u), Some("unit-cell")),
            "{anchor}"
        );
    }
    // `Target Factor of Safety | 2 | -`: a dash is no unit.
    assert_eq!(unit(param(doc, "Structural_Lining!B13")), (None, None));
    let t = param(doc, "Structural_Lining!E49");
    assert_eq!(t.label.as_deref(), Some("t"));
    assert_eq!(unit(t), (Some("m"), Some("header")));
    assert!(graph(doc).nodes.len() > 1);
}

#[test]
fn empty_text_result_is_stored() {
    // `=IF(A1="","",A1)` stores an empty text: that is a result, not a missing one.
    let rows = row(
        1,
        &[
            text("A1", ""),
            "<c r=\"B1\" t=\"str\"><f>IF(A1=\"\",\"\",A1)</f><v></v></c>".to_string(),
        ],
    );
    let out = read(&Book::default().sheet("S", &ws("", &rows, "")));
    let doc = &out.document;
    assert_eq!(
        formula(doc, "S!B1").cached,
        Some(Value::Text {
            value: String::new()
        })
    );
    assert!(stale_losses(doc).is_empty(), "{:#?}", doc.losses);
}

#[test]
fn full_calc_on_load_and_tables_warn_results_may_be_old() {
    let mut book = small_model();
    book.calc_pr = "<calcPr fullCalcOnLoad=\"1\"/>".into();
    assert_eq!(
        stale_losses(&read(&book).document),
        ["stored results may be old: the file asks for a full recalculation when opened"]
    );

    // Automatic except tables, with a data table in the workbook.
    let rows = "<row r=\"2\"><c r=\"A2\"><v>1</v></c></row><row r=\"5\"><c r=\"B5\"><f t=\"dataTable\" ref=\"B5:B7\" dt2D=\"0\" dtr=\"0\" r1=\"A2\"/><v>1</v></c></row>";
    let mut book = Book::default().sheet("S", &ws("", rows, ""));
    book.calc_pr = "<calcPr calcMode=\"autoNoTable\"/>".into();
    assert_eq!(
        stale_losses(&read(&book).document),
        ["stored results may be old: data tables are not recalculated automatically"]
    );

    // Automatic except tables, but no data table: nothing to say.
    let mut book = small_model();
    book.calc_pr = "<calcPr calcMode=\"autoNoTable\"/>".into();
    assert!(stale_losses(&read(&book).document).is_empty());
}

/// Values in B, read by a SUM in D1, with `codes` in column C and the labels in A.
fn coded_values(header: Option<[&str; 3]>, codes: &[&str]) -> ReadOutcome {
    let mut rows = Vec::new();
    let first = if let Some(h) = header {
        rows.push(row(
            1,
            &[text("A1", h[0]), text("B1", h[1]), text("C1", h[2])],
        ));
        2
    } else {
        1
    };
    let last = first + codes.len() as u32 - 1;
    for (i, code) in codes.iter().enumerate() {
        let r = first + i as u32;
        let mut cells = vec![
            text(&format!("A{r}"), &format!("Item {r}")),
            num(&format!("B{r}"), 10.0 + r as f64, None),
            text(&format!("C{r}"), code),
        ];
        if i == 0 {
            cells.push(formula_cell(
                &format!("D{r}"),
                &format!("SUM(B{first}:B{last})"),
                Some(1.0),
            ));
        }
        rows.push(row(r, &cells));
    }
    read(&Book::default().sheet("S", &ws("", &rows.concat(), "")))
}

#[test]
fn data_columns_beside_values_are_no_units() {
    let cases: [(Option<[&str; 3]>, &[&str]); 5] = [
        (Some(["Name", "Score", "Grade"]), &["A", "C", "F", "B", "T"]),
        (None, &["A", "C", "F", "B", "T"]),
        (None, &["N", "S", "W", "N"]),
        (
            Some(["Store", "Sales", "Region"]),
            &["MN", "GB", "cd", "var", "h"],
        ),
        (None, &["MN", "GB", "cd", "var", "h"]),
    ];
    for (header, codes) in cases {
        let out = coded_values(header, codes);
        let doc = &out.document;
        assert!(!doc.facets.parameters.is_empty());
        for p in &doc.facets.parameters {
            assert_eq!(unit(p), (None, None), "{codes:?}: {p:?}");
        }
    }
    // A column whose header says it holds units is a unit column.
    let out = coded_values(Some(["Item", "Qty", "Unit"]), &["kg", "m", "pcs"]);
    let doc = &out.document;
    assert_eq!(unit(param(doc, "S!B2")), (Some("kg"), Some("unit-cell")));
    assert_eq!(unit(param(doc, "S!B3")), (Some("m"), Some("unit-cell")));
    assert_eq!(unit(param(doc, "S!B4")), (None, None));
}

#[test]
fn codes_and_variables_are_no_units() {
    let labels = [
        ("Revenue Spain (ES)", None),
        ("Shear force (V)", None),
        ("Number of piles (N)", None),
        ("Height of wall (h)", None),
        ("Shear force [V]", Some("V")),
        ("Number of piles [N]", Some("N")),
        ("Excavated diameter (m)", Some("m")),
        ("Code PA", None),
    ];
    let mut rows = Vec::new();
    for (i, (label, _)) in labels.iter().enumerate() {
        let r = i as u32 + 1;
        let mut cells = vec![
            text(&format!("A{r}"), label),
            num(&format!("B{r}"), 1.0, None),
        ];
        if i == 0 {
            cells.push(formula_cell("C1", "SUM(B1:B8)", Some(8.0)));
        }
        rows.push(row(r, &cells));
    }
    let out = read(&Book::default().sheet("S", &ws("", &rows.concat(), "")));
    let doc = &out.document;
    for (i, (label, want)) in labels.iter().enumerate() {
        let p = param(doc, &format!("S!B{}", i + 1));
        assert_eq!(p.unit.as_deref(), *want, "{label}");
    }
}

#[test]
fn disagreeing_units_give_none_with_a_note() {
    // The label says mm, the number format writes m.
    let rows = row(
        1,
        &[
            text("A1", "Length (mm)"),
            num("B1", 10.0, Some(4)),
            formula_cell("C1", "B1*2", Some(20.0)),
        ],
    );
    let out = read(&Book::default().sheet("S", &ws("", &rows, "")));
    let doc = &out.document;
    let p = param(doc, "S!B1");
    assert_eq!(p.label.as_deref(), Some("Length"));
    assert_eq!(unit(p), (None, None));
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Uncertain
            && l.location == "S!B1"
            && l.what.contains("mm (label)")
            && l.what.contains("m (number-format)")),
        "{:#?}",
        doc.losses
    );
}

#[test]
fn headers_are_near_and_never_a_value() {
    // `Length (m)` heads G42:G46; `see note` in G47 is a value inside the column.
    let mut rows = vec![row(41, &[text("G41", "Length (m)")])];
    for r in 42..=50 {
        let cell = if r == 47 {
            text("G47", "see note")
        } else {
            num(&format!("G{r}"), f64::from(r), None)
        };
        let mut cells = vec![cell];
        if r == 42 {
            cells.push(formula_cell("H42", "SUM(G42:G50)", Some(1.0)));
        }
        rows.push(row(r, &cells));
    }
    let out = read(&Book::default().sheet("S", &ws("", &rows.concat(), "")));
    let doc = &out.document;
    for anchor in ["S!G42", "S!G44"] {
        let p = param(doc, anchor);
        assert_eq!(p.label.as_deref(), Some("Length"), "{anchor}");
        assert_eq!(unit(p), (Some("m"), Some("header")), "{anchor}");
    }
    // More than 3 cells below the header, and below the text value: no header.
    for anchor in ["S!G45", "S!G48", "S!G50"] {
        let p = param(doc, anchor);
        assert_eq!(p.label, None, "{anchor}");
        assert_eq!(unit(p), (None, None), "{anchor}");
    }
}

#[test]
fn sheet_part_cut_at_its_item_limit_leaves_results_open() {
    let mut cells = vec![num("A1", 1.0, None), formula_cell("B1", "A1*2", Some(2.0))];
    for c in ["C", "D", "E", "F", "G", "H"] {
        cells.push(num(&format!("{c}1"), 1.0, None));
    }
    let book = Book::default().sheet("S", &ws("", &row(1, &cells), ""));
    let limits = Limits {
        max_elements: 60,
        ..Limits::default()
    };
    let out = read_with(book.bytes(), limits);
    let doc = &out.document;
    assert!(
        doc.losses
            .iter()
            .any(|l| l.location.ends_with("sheet1.xml") && l.what.contains("items")),
        "{:#?}",
        doc.losses
    );
    assert_eq!(formula(doc, "S!B1").role, None);
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Uncertain
            && l.what.starts_with("the workbook logic is incomplete")),
        "{:#?}",
        doc.losses
    );
}
