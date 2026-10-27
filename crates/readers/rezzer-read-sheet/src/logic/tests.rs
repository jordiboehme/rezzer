// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the logic pass.

#[test]
fn unit_symbols_make_a_unit_column() {
    // Plain symbols without a digit or a sign.
    assert!(is_unit_column_texts(&["m", "m", "-", "kPa", "kPa"]));
    assert!(is_unit_column_texts(&["m2", "m"]));
    assert!(is_unit_column_texts(&["l/s", "l/s", "l/s"]));
    assert!(is_unit_column_texts(&["m/s", "m/s"]));
    // Quotients of symbols, beside a text that is not a unit.
    assert!(is_unit_column_texts(&[
        "veh/hr", "veh/hr", "g/s", "g/m3", "m3/s", "m/s"
    ]));
    // Count words beside a real symbol (the tunnel jet fan block).
    assert!(is_unit_column_texts(&["Pa", "N", "nos"]));
    assert!(is_unit_column_texts(&["kg", "pcs"]));
    assert!(is_unit_column_texts(&["m/s", "ea"]));
    // A quotient of two lower case symbols is a unit: metres per day.
    assert!(is_unit_column_texts(&["m/d", "m/d"]));
    // Words, codes and a single symbol are not a unit column.
    assert!(!is_unit_column_texts(&["m"]));
    assert!(!is_unit_column_texts(&["OK", "N/A", "yes"]));
    assert!(!is_unit_column_texts(&["N/A", "N/A"]));
    assert!(!is_unit_column_texts(&["A/C", "S/N"]));
    assert!(!is_unit_column_texts(&["A", "B", "C"]));
    assert!(!is_unit_column_texts(&["red", "blue", "green"]));
    // A class column, a Y/N column, a gender column (m and d) and a yes/no column are
    // data, not units.
    assert!(!is_unit_column_texts(&["A", "A", "A"]));
    assert!(!is_unit_column_texts(&["N", "N", "N"]));
    assert!(!is_unit_column_texts(&["m", "d", "m", "d"]));
    assert!(!is_unit_column_texts(&["no", "no"]));
    assert!(!is_unit_column_texts(&["yes", "no", "no"]));
    // A one-letter symbol beside count words is a flag column.
    assert!(!is_unit_column_texts(&["N", "no", "no"]));
    assert!(!is_unit_column_texts(&["m", "ea"]));
}

use rezzer_limits::Limits;
use rezzer_model::{Confidence, Detection, Document, FormatId, SourceInfo};

use super::*;

fn out() -> Out {
    Out::new(Document::new(SourceInfo {
        path: None,
        name: "t.xlsx".into(),
        sha256: String::new(),
        size: 0,
        format: FormatId::Xlsx,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "test".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
        encoding: None,
        reader: None,
    }))
}

/// A1:A3 values and B1:B3 copies of `=A1*2`.
fn copies(out: &mut Out) -> Formulas {
    let mut formulas = Formulas::new(&[], u64::MAX);
    for row in 0..3 {
        formulas.add_constant("S", row, 0);
        let index = out.doc.facets.formulas.len();
        let (f, problem) = formulas.cell(
            "S",
            row,
            1,
            &format!("A{}*2", row + 1),
            FormulaKind::Cell,
            None,
            Some(Value::number(2.0, None)),
            index,
        );
        assert!(problem.is_none());
        out.doc.facets.formulas.push(f);
    }
    formulas
}

#[test]
fn stopped_logic_keeps_patterns_and_says_so() {
    let mut out = out();
    let formulas = copies(&mut out);
    let budget = Budget::new(Limits::default());
    budget.cancel();
    let state = ReadState {
        calc: None,
        formats: &|_| None,
        incomplete: false,
    };
    workbook_logic(&mut out, formulas, Vec::new(), state, &budget);
    let doc = &out.doc;
    for f in &doc.facets.formulas {
        assert_eq!(f.pattern.as_deref(), Some("p1"));
        assert_eq!(f.range.as_ref().map(|a| a.0.as_str()), Some("S!B1:B3"));
        assert_eq!(f.role, None);
    }
    assert!(doc.facets.parameters.is_empty());
    assert!(doc.facets.graphs.is_empty());
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Limit
            && l.what
                .starts_with("time limit reached while the workbook logic")),
        "{:#?}",
        doc.losses
    );
    assert!(out.partial);
}

#[test]
fn label_callback_is_the_label_rule() {
    assert!(is_label("Span"));
    assert!(is_label("  Span  "));
    assert!(!is_label("   "));
    assert!(!is_label(""));
    assert!(is_label(&"a".repeat(MAX_LABEL)));
    assert!(!is_label(&"a".repeat(MAX_LABEL + 1)));
}

#[test]
fn unit_column_callback_reads_units() {
    assert!(is_unit_column_texts(&["Unit", "whatever"]));
    assert!(is_unit_column_texts(&[" kN/m3 ", "m2", "mm"]));
    assert!(!is_unit_column_texts(&["Alice", "Bob", "Carol"]));
    assert!(!is_unit_column_texts(&[]));
}

#[test]
fn layout_facts_name_functions_and_same_sheet_reads() {
    let mut out = out();
    let mut formulas = Formulas::new(&[], u64::MAX);
    let texts = [
        (4, "SUBTOTAL(9,B2:B4)"),
        (5, "SUM(B2:B5)+Data!B2+'S'!C1"),
        (6, "SUM(Data!B2:B5)"),
        (7, "1+"),
    ];
    for (row, text) in texts {
        let index = out.doc.facets.formulas.len();
        let (f, _) = formulas.cell("S", row, 1, text, FormulaKind::Cell, None, None, index);
        out.doc.facets.formulas.push(f);
    }
    let (f, _) = formulas.cell("Data", 0, 0, "SUM(A2:A3)", FormulaKind::Cell, None, None, 9);
    out.doc.facets.formulas.push(f);
    let rect = |top, left, bottom, right| sheetlayout::Rect {
        top,
        left,
        bottom,
        right,
    };
    let facts = formulas.layout_facts("S");
    assert_eq!(facts.len(), 4, "{facts:?}");
    assert_eq!(facts[&(4, 1)].functions, ["SUBTOTAL"]);
    assert_eq!(facts[&(4, 1)].reads, [rect(1, 1, 3, 1)]);
    assert_eq!(facts[&(5, 1)].functions, ["SUM"]);
    assert_eq!(facts[&(5, 1)].reads, [rect(1, 1, 4, 1), rect(0, 2, 0, 2)]);
    // A read on another sheet never marks a row of this one.
    assert!(facts[&(6, 1)].reads.is_empty());
    // A formula that does not parse still counts as a formula.
    assert_eq!(facts[&(7, 1)], sheetlayout::FormulaIn::default());
    // Handed out once per sheet.
    assert!(formulas.layout_facts("S").is_empty());
    assert_eq!(formulas.layout_facts("data").len(), 1);
}

#[test]
fn convert_calls_and_literals() {
    let calls = calls_of(
        "=A1+convert(B1,\"m\",\"ft\")*CONVERT(C1,\"g\",\"kg\")",
        "CONVERT",
    );
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], ["B1", "\"m\"", "\"ft\""]);
    assert!(calls_of("=\"CONVERT(B1,1,2)\"&MYCONVERT(B1,1,2)", "CONVERT").is_empty());
    assert_eq!(text_literal("\"a\"\"b\""), Some("a\"b".into()));
    assert_eq!(text_literal("B1"), None);
    assert_eq!(literal_number("0.05"), Some(Value::number(0.05, None)));
    assert_eq!(literal_number("=-3"), Some(Value::number(-3.0, None)));
    assert_eq!(literal_number("5%"), Some(Value::percent(0.05)));
    assert_eq!(literal_number("1.5E-3"), Some(Value::number(0.0015, None)));
    for t in ["inf", "NaN", "A1", "1+2", "\"5\"", ""] {
        assert_eq!(literal_number(t), None, "{t}");
    }
}
