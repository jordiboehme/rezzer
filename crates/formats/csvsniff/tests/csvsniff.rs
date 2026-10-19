// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use csvsniff::{ColumnKind, Derived, DerivedOp, RaggedRow, analyze, sniff};

fn full(text: &str) -> csvsniff::Analysis {
    let d = sniff(text).expect("dialect");
    analyze(text, &d, 100_000)
}

#[test]
fn sniffs_comma() {
    let d = sniff("a,b,c\n1,2,3\n4,5,6\n").unwrap();
    assert_eq!(d.delimiter, b',');
    assert_eq!(d.quote, b'"');
    assert!(d.consistent_rows >= 0.99);
}

#[test]
fn sniffs_semicolon_with_trailing_delimiter() {
    let d = sniff("a;b;c;\n1,5;2;3;\n4;5,5;6;\n").unwrap();
    assert_eq!(d.delimiter, b';');
}

#[test]
fn sniffs_tab() {
    let d = sniff("a\tb\tc\n1\t2\t3\n4\t5\t6\n").unwrap();
    assert_eq!(d.delimiter, b'\t');
}

#[test]
fn prose_is_not_csv() {
    let one = "It was a dark, stormy night, and the rain fell in torrents, except at occasional intervals, when it was checked by a violent gust of wind.";
    assert_eq!(sniff(one), None);
    let many = "Well, I think so.\nHowever, as he said, it was late, and the road, which was long, ended.\nNo.\nShe came, saw, and left, as always, quietly.\n";
    assert_eq!(sniff(many), None);
}

#[test]
fn detects_header() {
    let d = sniff("name,age\nAnn,30\nBob,25\nCid,41\n").unwrap();
    assert!(d.has_header);
    let d = sniff("name,city\nAnn,Berlin\nBob,Berlin\nCid,Bonn\n").unwrap();
    assert!(d.has_header);
}

#[test]
fn headerless_detected() {
    let text = "AT.01\tBurgenland\tBurgenland\t2781194\nAT.02\tKaernten\tCarinthia\t2774686\nAT.03\tNiederoesterreich\tLower Austria\t2770542\n";
    let d = sniff(text).unwrap();
    assert_eq!(d.delimiter, b'\t');
    assert!(!d.has_header);
    let a = analyze(text, &d, 1000);
    assert_eq!(a.rows, 3);
    assert_eq!(a.columns[0].name, "Column 1");
}

#[test]
fn column_kinds_and_places() {
    let a = full(
        "i,d,t,day,b,e\n1,1.25,x,2024-01-31,true,\n2,3,y,2024-02-01,no,\n3,0.5,z,2024-03-05,yes,\n",
    );
    assert_eq!(a.columns[0].kind, ColumnKind::Integer);
    assert_eq!(a.columns[1].kind, ColumnKind::Decimal { places: 2 });
    assert_eq!(a.columns[2].kind, ColumnKind::Text);
    assert_eq!(a.columns[3].kind, ColumnKind::Date);
    assert_eq!(a.columns[4].kind, ColumnKind::Bool);
    assert_eq!(a.columns[5].kind, ColumnKind::Empty);
    assert_eq!(a.columns[5].missing, 3);
}

#[test]
fn date_styles() {
    let a = full("a,b,c,d\n2024-01-31T10:00:00Z,x,y,z\n2024-02-01 10:00,x,y,z\n");
    assert_eq!(a.columns[0].kind, ColumnKind::Date);
    let a = full("a,b,c\n31.12.2020,12/31/2020,31-May-13\n01.02.2021,01/02/2021,1-Jun-13\n");
    for c in &a.columns {
        assert_eq!(c.kind, ColumnKind::Date, "{}", c.name);
    }
}

#[test]
fn unit_from_header() {
    let a = full("Mass (kg),Length [m],Rate in %,Plain\n1,2,3,4\n5,6,7,8\n");
    assert_eq!(a.columns[0].unit.as_deref(), Some("kg"));
    assert_eq!(a.columns[1].unit.as_deref(), Some("m"));
    assert_eq!(a.columns[2].unit.as_deref(), Some("%"));
    assert_eq!(a.columns[3].unit, None);
    let a = full("Institution in Country,x\nA,1\nB,2\n");
    assert_eq!(a.columns[0].unit, None);
}

#[test]
fn key_like_column() {
    let a = full("id,code,grp,val\n1,A1,x,1.5\n2,B2,x,1.5\n3,C3,y,2.5\n4,D4,y,2.5\n");
    assert!(a.columns[0].key_like);
    assert!(a.columns[1].key_like);
    assert!(!a.columns[2].key_like);
    assert!(!a.columns[2].unique);
    assert!(!a.columns[3].key_like);
}

#[test]
fn ragged_rows_reported() {
    let text = "a,b,c\n1,2,3\n4,5,6,7\n8,9,10\n1,2,3\n4,5,6\n7,8,9\n";
    let a = full(text);
    assert_eq!(
        a.ragged_rows,
        vec![RaggedRow {
            line: 3,
            fields: 4,
            expected: 3
        }]
    );
    assert_eq!(a.rows, 6);
}

#[test]
fn ragged_line_counts_embedded_newlines() {
    let text = "a,b,c\n\"x\ny\",2,3\n4,5\n6,7,8\n1,2,3\n4,5,6\n7,8,9\n";
    let a = full(text);
    assert_eq!(
        a.ragged_rows,
        vec![RaggedRow {
            line: 4,
            fields: 2,
            expected: 3
        }]
    );
}

#[test]
fn unterminated_quote_flagged() {
    let a = full("a,b\n1,2\n3,\"oops\n4,5\n6,7\n");
    assert!(a.unterminated_quote);
    let a = full("a,b\n1,\"fine\"\n3,\"he said \"\"hi\"\"\"\n");
    assert!(!a.unterminated_quote);
}

#[test]
fn derived_sum_found() {
    let mut t = String::from("A,B,C,Total\n");
    for i in 1..=12 {
        let (a, b, c) = (i, i * 3 % 7 + 1, i * i);
        t.push_str(&format!("{a},{b},{c},{}\n", a + b + c));
    }
    let a = full(&t);
    assert_eq!(
        a.derived,
        vec![Derived {
            target: 3,
            parts: vec![0, 1, 2],
            op: DerivedOp::Sum,
            holds: 12,
            checked: 12
        }]
    );
}

#[test]
fn derived_with_decimals_and_missing() {
    let mut t = String::from("a,b,total\n");
    for i in 1..=14 {
        let a = i as f64 * 1.37;
        let b = (i * i) as f64 * 0.51;
        let sum = a + b;
        if i == 5 {
            t.push_str(&format!("{a:.2},,{sum:.2}\n"));
        } else {
            t.push_str(&format!("{a:.2},{b:.2},{sum:.2}\n"));
        }
    }
    let a = full(&t);
    assert_eq!(a.derived.len(), 1);
    let d = &a.derived[0];
    assert_eq!((d.target, d.parts.clone()), (2, vec![0, 1]));
    assert_eq!((d.holds, d.checked), (13, 13));
}

#[test]
fn no_derived_below_10_rows() {
    let a = full("a,b,c\n1,2,3\n4,5,9\n");
    assert!(a.derived.is_empty());
}

#[test]
fn no_derived_over_max_rows() {
    let mut t = String::from("A,B,Total\n");
    for i in 1..=20 {
        t.push_str(&format!("{i},{},{}\n", i + 1, 2 * i + 1));
    }
    let d = sniff(&t).unwrap();
    assert!(analyze(&t, &d, 10).derived.is_empty());
    assert_eq!(analyze(&t, &d, 10).rows, 20);
    assert!(!analyze(&t, &d, 100).derived.is_empty());
}
