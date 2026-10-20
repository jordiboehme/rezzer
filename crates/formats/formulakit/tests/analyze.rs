// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use formulakit::{
    Analysis, CellPos, Dialect, Ref, SheetSpan, Whole, analyze, anchor_of, quote_sheet,
};

fn pos(row: u32, col: u32) -> CellPos {
    CellPos { row, col }
}

fn excel(text: &str) -> Analysis {
    analyze(text, Dialect::Excel, pos(0, 0)).expect("formula parses")
}

fn sheet(name: &str) -> Option<SheetSpan> {
    Some(SheetSpan {
        first: name.to_string(),
        last: None,
    })
}

fn cell(sheet_name: Option<&str>, row: u32, col: u32) -> Ref {
    Ref::Cell {
        sheet: sheet_name.and_then(sheet),
        pos: pos(row, col),
        abs_row: false,
        abs_col: false,
    }
}

fn anchors(a: &Analysis, on: &str) -> Vec<String> {
    a.refs.iter().map(|r| anchor_of(on, r)).collect()
}

#[test]
fn cross_sheet_and_3d_refs() {
    let a = excel("=SUM(Sheet1:Sheet3!A1)");
    assert_eq!(
        a.refs,
        vec![Ref::Cell {
            sheet: Some(SheetSpan {
                first: "Sheet1".into(),
                last: Some("Sheet3".into()),
            }),
            pos: pos(0, 0),
            abs_row: false,
            abs_col: false,
        }]
    );
    assert_eq!(anchors(&a, "Main"), vec!["Sheet1:Sheet3!A1"]);

    let a = excel("='Q1 ''24'!B2*2");
    assert_eq!(a.refs, vec![cell(Some("Q1 '24"), 1, 1)]);
    assert_eq!(anchors(&a, "Main"), vec!["'Q1 ''24'!B2"]);
    assert_eq!(a.display, "='Q1 ''24'!B2*2");

    let a = excel("=SUM('Jan 24:Mar 24'!B5:C6)");
    assert_eq!(anchors(&a, "Main"), vec!["'Jan 24:Mar 24'!B5:C6"]);

    // An unqualified reference gets the sheet of the formula in its anchor.
    let a = excel("=A1+Data!$B$2");
    assert_eq!(anchors(&a, "My Sheet"), vec!["'My Sheet'!A1", "Data!B2"]);
}

#[test]
fn quoted_sheet_names_roundtrip() {
    for name in [
        "Sheet1",
        "My Sheet",
        "Q1 '24",
        "2024",
        "A1",
        "R1C1",
        "Übersicht",
        "Лист1",
        "a-b",
        "it's",
        "x.y",
    ] {
        let r = cell(Some(name), 4, 2);
        let anchor = anchor_of("Other", &r);
        let back = excel(&format!("={anchor}"));
        assert_eq!(back.refs, vec![r], "anchor {anchor} for sheet {name:?}");
    }
    assert_eq!(quote_sheet("Sheet1"), "Sheet1");
    assert_eq!(quote_sheet("Übersicht"), "Übersicht");
    assert_eq!(quote_sheet("My Sheet"), "'My Sheet'");
    assert_eq!(quote_sheet("Q1 '24"), "'Q1 ''24'");
    assert_eq!(quote_sheet("2024"), "'2024'");
    assert_eq!(quote_sheet("A1"), "'A1'");
    assert_eq!(quote_sheet("R1C1"), "'R1C1'");
    assert_eq!(quote_sheet("R"), "'R'");

    let range = Ref::Range {
        sheet: sheet("My Sheet"),
        start: pos(0, 0),
        end: pos(8, 1),
        abs: [false; 4],
        whole: Whole::None,
    };
    assert_eq!(anchor_of("x", &range), "'My Sheet'!A1:B9");
    let back = excel(&format!("={}", anchor_of("x", &range)));
    assert_eq!(back.refs, vec![range]);
}

#[test]
fn names_tables_and_externals() {
    let a = excel("=gg*zcor");
    assert_eq!(
        a.refs,
        vec![
            Ref::Name {
                name: "gg".into(),
                sheet: None
            },
            Ref::Name {
                name: "zcor".into(),
                sheet: None
            },
        ]
    );
    assert_eq!(anchors(&a, "z"), vec!["name:gg", "name:zcor"]);

    let a = excel("=Sheet1!gg+'My S'!hh");
    assert_eq!(
        a.refs,
        vec![
            Ref::Name {
                name: "gg".into(),
                sheet: Some("Sheet1".into())
            },
            Ref::Name {
                name: "hh".into(),
                sheet: Some("My S".into())
            },
        ]
    );
    assert_eq!(anchors(&a, "z"), vec!["name:Sheet1!gg", "name:'My S'!hh"]);

    let a = excel("=SUM(Table1[c])");
    assert_eq!(
        a.refs,
        vec![Ref::Table {
            name: "Table1".into(),
            column: Some("c".into())
        }]
    );
    assert_eq!(anchors(&a, "z"), vec!["table:Table1"]);
    assert_eq!(a.functions, vec!["SUM"]);

    let a = excel("=[1]Sheet1!A1");
    assert_eq!(
        a.refs,
        vec![Ref::External {
            book: "[1]".into(),
            inner: Box::new(cell(Some("Sheet1"), 0, 0)),
        }]
    );
    assert_eq!(anchors(&a, "z"), vec!["ext:[1]Sheet1!A1"]);

    let a = excel("=[1]!gg+'[2]My S'!B2");
    assert_eq!(anchors(&a, "z"), vec!["ext:[1]!gg", "ext:'[2]My S'!B2"]);
}

#[test]
fn copies_share_pattern_key() {
    let d2 = analyze("=B2*C2", Dialect::Excel, pos(1, 3)).unwrap();
    let d3 = analyze("=B3*C3", Dialect::Excel, pos(2, 3)).unwrap();
    assert_eq!(d2.pattern_key, d3.pattern_key);
    assert_eq!(d2.pattern_key, "=RC[-2]*RC[-1]");

    let a = analyze("=$B$2*C2", Dialect::Excel, pos(1, 3)).unwrap();
    let b = analyze("=$B$2*C3", Dialect::Excel, pos(2, 3)).unwrap();
    assert_eq!(a.pattern_key, b.pattern_key);
    assert_eq!(a.pattern_key, "=R2C2*RC[-1]");

    let times = analyze("=B2*C2", Dialect::Excel, pos(1, 3)).unwrap();
    let plus = analyze("=B2+C2", Dialect::Excel, pos(1, 3)).unwrap();
    assert_ne!(times.pattern_key, plus.pattern_key);

    // Sheets, whole columns and mixed absolute flags are part of the key.
    let a = analyze("=SUM(Data!$A:$A)+A$1", Dialect::Excel, pos(4, 1)).unwrap();
    assert_eq!(a.pattern_key, "=SUM(Data!C1:C1)+R1C[-1]");
}

#[test]
fn let_and_lambda_params_are_not_names() {
    let a = excel("=LET(x,A1*2,x+1)");
    assert_eq!(a.refs, vec![cell(None, 0, 0)]);
    assert_eq!(a.lambda_params, vec!["x"]);

    let a = excel("=_xlfn.LET(_xlpm.x,A1*2,_xlpm.x+1)");
    assert_eq!(a.refs, vec![cell(None, 0, 0)]);
    assert_eq!(a.display, "=LET(x,A1*2,x+1)");
    assert_eq!(a.lambda_params, vec!["x"]);

    let a = excel("=_xlfn.LAMBDA(_xlpm.a,_xlpm.a*2)(B1)");
    assert_eq!(a.refs, vec![cell(None, 0, 1)]);
    assert_eq!(a.display, "=LAMBDA(a,a*2)(B1)");

    // A LET-bound lambda called by its name is not a user function.
    let a = excel("=LET(f,LAMBDA(v,v+1),f(gg))");
    assert!(a.user_functions.is_empty(), "{:?}", a.user_functions);
    assert_eq!(
        a.refs,
        vec![Ref::Name {
            name: "gg".into(),
            sheet: None
        }]
    );
    assert_eq!(a.lambda_params, vec!["f", "v"]);

    // A name used before its binding is a workbook name.
    let a = excel("=LET(x,x+1,x)");
    assert_eq!(
        a.refs,
        vec![Ref::Name {
            name: "x".into(),
            sheet: None
        }]
    );
}

#[test]
fn xlfn_prefixes_removed_for_display() {
    let a = excel("=_xlfn.XLOOKUP(A1,B:B,C:C)");
    assert_eq!(a.display, "=XLOOKUP(A1,B:B,C:C)");
    assert_eq!(a.functions, vec!["XLOOKUP"]);
    assert!(a.user_functions.is_empty());

    let a = excel("=_xlfn._xlws.SORT(A1:A3)");
    assert_eq!(a.display, "=SORT(A1:A3)");

    // Text in a string literal stays as written; spans are bytes, not characters.
    let a = excel("=Übersicht!A1&\"_xlfn.\"&_xlfn.CONCAT(B1)");
    assert_eq!(a.display, "=Übersicht!A1&\"_xlfn.\"&CONCAT(B1)");

    // A formula without the leading equals sign, as some readers return it.
    let a = excel("_xlfn.XLOOKUP(A1,B:B,C:C)");
    assert_eq!(a.display, "=XLOOKUP(A1,B:B,C:C)");
    assert_eq!(a.refs.len(), 3);
}

#[test]
fn openformula_to_a1() {
    let a = analyze("of:=[.B4]+[$Data.A3]", Dialect::OpenFormula, pos(0, 0)).unwrap();
    assert_eq!(a.display, "=B4+Data!A3");
    assert_eq!(a.refs, vec![cell(None, 3, 1), cell(Some("Data"), 2, 0)]);

    let a = analyze(
        "of:=([.B4]-[.$G$1])/[.$G$2]",
        Dialect::OpenFormula,
        pos(3, 2),
    )
    .unwrap();
    assert_eq!(a.display, "=(B4-$G$1)/$G$2");
    let b = analyze(
        "of:=([.C4]-[.$G$1])/[.$G$2]",
        Dialect::OpenFormula,
        pos(3, 3),
    )
    .unwrap();
    assert_eq!(a.pattern_key, b.pattern_key);

    let a = analyze(
        "of:=SUM([.A1:.B2];[$'Q1 ''24'.$C$3])",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(a.display, "=SUM(A1:B2,'Q1 ''24'!$C$3)");

    // 3D, external, union, intersection and array constants.
    let a = analyze(
        "of:=SUM([$Sheet1.A1:$Sheet3.B2])",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(a.display, "=SUM(Sheet1:Sheet3!A1:B2)");
    let a = analyze(
        "of:=['file:///x.ods'#$Sheet1.A1]",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(
        a.refs,
        vec![Ref::External {
            book: "[file:///x.ods]".into(),
            inner: Box::new(cell(Some("Sheet1"), 0, 0)),
        }]
    );
    let a = analyze(
        "of:=SUM([.A1]~[.B1];[.C1])",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(a.display, "=SUM((A1,B1),C1)");
    let a = analyze(
        "of:=SUM([.A1:.A3]![.A2:.B2])",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(a.display, "=SUM(A1:A3 A2:B2)");
    let a = analyze("of:=SUM({1;2|3;4})", Dialect::OpenFormula, pos(0, 0)).unwrap();
    assert_eq!(a.display, "=SUM({1,2;3,4})");
    let a = analyze(
        "of:=COM.MICROSOFT.XLOOKUP([.A1];[.B:.B];[.C:.C])",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(a.display, "=XLOOKUP(A1,B:B,C:C)");
    assert!(a.user_functions.is_empty());
    let a = analyze("of:=L_nb/(1+Xnb)", Dialect::OpenFormula, pos(0, 0)).unwrap();
    assert_eq!(anchors(&a, "s"), vec!["name:L_nb", "name:Xnb"]);
    let a = analyze("of:=[.#REF!]+1", Dialect::OpenFormula, pos(0, 0)).unwrap();
    assert_eq!(a.display, "=#REF!+1");
    assert!(a.refs.is_empty());

    // An Excel formula stored in an ODF file.
    let a = analyze(
        "msoxl:=_xlfn.XLOOKUP(A1,B:B,C:C)",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert_eq!(a.display, "=XLOOKUP(A1,B:B,C:C)");
}

#[test]
fn udf_and_external_function_are_listed() {
    let a = excel("=[1]!unf_pvt_dZdt(A1,gg)");
    assert_eq!(a.user_functions, vec!["[1]!unf_pvt_dZdt"]);
    assert_eq!(a.functions, vec!["[1]!unf_pvt_dZdt"]);
    assert_eq!(anchors(&a, "s"), vec!["s!A1", "name:gg"]);
    assert_eq!(a.display, "=[1]!unf_pvt_dZdt(A1,gg)");

    let a = excel("=MyVbaFn(A1)+SUM(B1)");
    assert_eq!(a.user_functions, vec!["MyVbaFn"]);
    assert_eq!(a.functions, vec!["MyVbaFn", "SUM"]);

    let a = excel("=_xludf.Foo(1)");
    assert_eq!(a.user_functions, vec!["Foo"]);
    assert_eq!(a.display, "=Foo(1)");

    let a = analyze(
        "of:=ORG.OPENOFFICE.WEEKS([.A1];[.A2];0)",
        Dialect::OpenFormula,
        pos(0, 0),
    )
    .unwrap();
    assert!(a.user_functions.is_empty(), "{:?}", a.user_functions);
}

#[test]
fn volatile_is_flagged() {
    for text in [
        "=NOW()",
        "=TODAY()+1",
        "=RAND()",
        "=RANDBETWEEN(1,6)",
        "=_xlfn.RANDARRAY(3)",
        "=SUM(OFFSET(A1,1,1,2,2))",
        "=INDIRECT(\"A1\")",
        "=CELL(\"row\",A1)",
        "=INFO(\"osversion\")",
    ] {
        assert!(excel(text).volatile, "{text}");
    }
    assert!(!excel("=SUM(A1:A3)").volatile);
    assert!(!excel("=MyNow()").volatile);
}

#[test]
fn unparsable_formula_keeps_text_and_note() {
    for text in [
        "=SUM(",
        "=1+",
        "=)",
        "=\"open",
        "of:=[.A1",
        "of:=[nonsense]",
        "xyz:=A1",
    ] {
        let dialect = if text.starts_with("of:") {
            Dialect::OpenFormula
        } else {
            Dialect::Excel
        };
        let err = analyze(text, dialect, pos(0, 0)).expect_err(text);
        assert!(!err.message.is_empty(), "{text}");
    }

    // Deep and long inputs fail cleanly instead of overflowing the stack.
    let deep = format!("={}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(analyze(&deep, Dialect::Excel, pos(0, 0)).is_err());
    // 300 chained operands pass Excel's limits; the parser may refuse them, but never panics.
    let long = format!("={}", vec!["A1"; 300].join("+"));
    let _ = analyze(&long, Dialect::Excel, pos(0, 0));
    let huge = format!("={}", "1+".repeat(100_000));
    assert!(analyze(&huge, Dialect::Excel, pos(0, 0)).is_err());
}

#[test]
fn ranges_rows_and_columns() {
    let a = excel("=SUM(B:D)+SUM(2:3)+SUM(Sheet1!$A$1:B$9)");
    assert_eq!(
        a.refs,
        vec![
            Ref::Range {
                sheet: None,
                start: pos(0, 1),
                end: pos(1_048_575, 3),
                abs: [false; 4],
                whole: Whole::Cols,
            },
            Ref::Range {
                sheet: None,
                start: pos(1, 0),
                end: pos(2, 16_383),
                abs: [false; 4],
                whole: Whole::Rows,
            },
            Ref::Range {
                sheet: sheet("Sheet1"),
                start: pos(0, 0),
                end: pos(8, 1),
                abs: [true, true, true, false],
                whole: Whole::None,
            },
        ]
    );
    assert_eq!(anchors(&a, "s"), vec!["s!B:D", "s!2:3", "Sheet1!A1:B9"]);

    // A reference read twice is listed once.
    assert_eq!(excel("=A1+A1*A1").refs.len(), 1);
}
