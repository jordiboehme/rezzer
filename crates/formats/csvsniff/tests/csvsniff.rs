// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use csvsniff::{CellClass, ColumnKind, Derived, DerivedOp, RaggedRow, analyze, cell_class, sniff};

fn full(text: &str) -> csvsniff::Analysis {
    let d = sniff(text).expect("dialect");
    analyze(text, &d, 100_000)
}

fn fixed(text: &str) -> csvsniff::Analysis {
    let d = csvsniff::Dialect {
        delimiter: b',',
        quote: b'"',
        lead_records: 0,
        has_header: true,
        consistent_rows: 1.0,
    };
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
fn quote_problem_is_located() {
    // Open at the end of the text.
    let text = "a,b\n1,2\n3,\"oops\n4,5\n";
    let a = fixed(text);
    let p = a.quote_problems[0];
    assert!(p.at_end);
    assert_eq!(&text[p.offset..p.offset + 5], "\"oops");
    // Closed by a later quote that is not at the end of a field.
    let text = "a,b\n1,\"open\n2,3\n4,\"x y\",5\n6,7\n";
    let a = fixed(text);
    let p = a.quote_problems[0];
    assert!(!p.at_end);
    assert!(!a.unterminated_quote);
    assert_eq!(&text[p.offset..p.offset + 5], "\"open");
    // Proper multi-line and escaped quotes are fine.
    let a = fixed("a,b\n1,\"ha \n\"\"ha\"\" \nha\"\n3,4\n");
    assert!(a.quote_problems.is_empty());
}

#[test]
fn mid_file_and_end_problems_are_both_reported() {
    let text = "a,b\n1,2\n2,\"open\n3,4\n4,\"x y\",5\n9,\"oops,1\n7,8\n7,8\n";
    let a = fixed(text);
    assert_eq!(a.quote_problems.len(), 2);
    assert!(!a.quote_problems[0].at_end);
    assert!(a.quote_problems[1].at_end);
    assert!(a.unterminated_quote);
    assert_eq!(&text[a.quote_problems[1].offset..][..6], "\"oops,");
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
        let j = i;
        let (a, b, c) = (
            j * 371 % 900 + 100,
            j * 533 % 700 + 200,
            j * 179 % 600 + 3150,
        );
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
    assert_eq!((d.holds, d.checked), (13, 14));
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
        let j = i;
        let (a, b) = (j * 371 % 900 + 100, j * 533 % 700 + 200);
        t.push_str(&format!("{a},{b},{}\n", a + b));
    }
    let d = sniff(&t).unwrap();
    assert!(analyze(&t, &d, 5).derived.is_empty());
    assert_eq!(analyze(&t, &d, 5).rows, 20);
    assert!(!analyze(&t, &d, 100).derived.is_empty());
}

#[test]
fn derived_counts_missing_part_as_zero() {
    let mut t = String::from("Solid,Gas,Flaring,Total\n");
    for i in 1..=20 {
        let (a, b) = (i % 4 * 300 + 1000, i % 3 * 500 + 600);
        let f = if i % 5 == 0 {
            Some(i * 71 % 400 + 300)
        } else {
            None
        };
        let total = a + b + f.unwrap_or(0);
        let f = f.map_or(String::new(), |v| v.to_string());
        t.push_str(&format!("{a},{b},{f},{total}\n"));
    }
    let a = full(&t);
    let d = a.derived.iter().find(|d| d.target == 3).expect("derived");
    assert_eq!(d.parts, vec![0, 1, 2]);
    assert_eq!((d.holds, d.checked), (20, 20));
}

#[test]
fn year_like_columns_are_never_parts() {
    let mut t = String::from("Year,A,B,Total\n");
    for i in 1..=12 {
        let (y, a, b) = (2000 + i % 3, i * 371 % 900 + 100, i * 533 % 700 + 200);
        t.push_str(&format!("{y},{a},{b},{}\n", y + a + b));
    }
    assert!(full(&t).derived.is_empty());
}

#[test]
fn ragged_file_is_accepted() {
    let mut t = String::from("a,b,c\n");
    for i in 0..7 {
        t.push_str(&format!("{i},x,y\n"));
    }
    t.push_str("1,2,3,4\n1,2\n1,2,3\nx,\"open,z\n");
    let d = sniff(&t).expect("accepted");
    assert_eq!(d.delimiter, b',');
    assert!(d.consistent_rows < 0.8 && d.consistent_rows >= 0.6);
    let a = analyze(&t, &d, 1000);
    assert!(a.unterminated_quote);
    assert!(!a.ragged_rows.is_empty());
    let prose = "Well, I think so.\nHowever, as he said, it was late, and the road, which was long, ended.\nNo.\nShe came, saw, and left, as always, quietly.\n";
    assert_eq!(sniff(prose), None);
}

#[test]
fn consecutive_id_column_is_never_a_part() {
    let mut t = String::from("n,A,B,Total\n");
    for i in 0..12 {
        let (n, a, b) = (100 + i, i * 371 % 900 + 100, i * 533 % 700 + 200);
        t.push_str(&format!("{n},{a},{b},{}\n", n + a + b));
    }
    assert!(full(&t).derived.is_empty());
}

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

#[test]
fn likert_files_have_no_derived_columns() {
    let mut s = 12345u64;
    for trial in 0..20 {
        let mut t = (0..12)
            .map(|i| format!("q{i}"))
            .collect::<Vec<_>>()
            .join(",");
        t.push('\n');
        for _ in 0..30 {
            let row: Vec<String> = (0..12)
                .map(|_| (xorshift(&mut s) % 5 + 1).to_string())
                .collect();
            t.push_str(&row.join(","));
            t.push('\n');
        }
        let a = full(&t);
        assert!(a.derived.is_empty(), "trial {trial}: {:?}", a.derived);
    }
}

#[test]
fn co2_corpus_total_is_still_found() {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../spike/corpus/web-text/csv-datahub-co2-fossil-global.csv"
    );
    let Ok(text) = std::fs::read_to_string(p) else {
        return;
    };
    let a = full(&text);
    assert_eq!(a.derived.len(), 1, "{:?}", a.derived);
    let d = &a.derived[0];
    assert_eq!((d.target, d.parts.clone()), (1, vec![2, 3, 4, 5, 6]));
    assert_eq!((d.holds, d.checked), (210, 275));
}

#[test]
fn non_csv_text_is_rejected() {
    assert_eq!(sniff("Dear Anna, thanks.\nBest regards, Jordi\n"), None);
    assert_eq!(
        sniff("Hello, Anna\nThanks, Bob\nI went home, ate, slept, and then, later, woke.\n"),
        None
    );
    let mut json = String::from("{\n");
    for i in 0..20 {
        json.push_str(&format!("  \"key{i}\": {i},\n"));
    }
    json.push_str("  \"last\": 0\n}\n");
    assert_eq!(sniff(&json), None);
    assert_eq!(sniff("[\n  1, 2, 3,\n  4, 5, 6,\n  7, 8, 9\n]\n"), None);
    let g: String = (0..50)
        .map(|i| format!("G1 X{i} Y{i} ; move {i}\n"))
        .collect();
    assert_eq!(sniff(&g), None);
    let g: String = (0..50)
        .map(|i| format!("; step {i}, part\n; note {i}, more\n"))
        .collect();
    assert_eq!(sniff(&g), None);
    let c: String = (0..30).map(|i| format!("    foo(a{i}, b{i});\n")).collect();
    assert_eq!(sniff(&c), None);
    let c: String = (0..30).map(|i| format!("if (a{i}, b{i}) {{\n")).collect();
    assert_eq!(sniff(&c), None);
    let long: String = (0..10)
        .map(|i| {
            format!(
                "this is a rather long sentence number {i},and here is another long one for you\n"
            )
        })
        .collect();
    assert_eq!(sniff(&long), None);
}

#[test]
fn two_lines_need_three_fields() {
    assert!(sniff("a,b,c\n1,2,3\n").is_some());
    assert_eq!(sniff("a,b\n1,2\n"), None);
}

#[test]
fn tie_prefers_numeric_cells() {
    let t = "01.01.2020;1,5;2,5\n02.01.2020;1,7;2,8\n03.01.2020;1,9;3,1\n04.01.2020;2,0;3,3\n";
    assert_eq!(sniff(t).unwrap().delimiter, b';');
}

#[test]
fn year_headers_count_as_header() {
    let d = sniff("Country,2019,2020,2021\nAT,1,2,3\nDE,4,5,6\nFR,7,8,9\n").unwrap();
    assert!(d.has_header);
    let d = sniff("AT,2019,2020\nDE,2019,2020\nFR,2019,2021\n").unwrap();
    assert!(!d.has_header);
}

#[test]
fn extra_cells_do_not_add_columns() {
    let mut t = String::from("a,b,c\n");
    for _ in 0..9 {
        t.push_str("1,2,3\n");
    }
    t.push_str("1,2,3,4,5,6,7\n");
    let a = full(&t);
    assert_eq!(a.columns.len(), 3);
    assert_eq!(a.ragged_rows.len(), 1);
}

#[test]
fn ragged_rows_are_capped() {
    let mut t = String::from("a,b,c\n1,2,3\n");
    for _ in 0..2500 {
        t.push_str("1,2,3\n");
    }
    for _ in 0..1500 {
        t.push_str("1,2\n");
    }
    let a = full(&t);
    assert_eq!(a.ragged_rows.len(), 1000);
    assert_eq!(a.ragged_total, 1500);
}

#[test]
fn units_from_known_tokens_only() {
    let a = full("Born in Rome,Mass in kg,Rate in %,Temp in mm\nx,1,2,3\ny,2,3,4\n");
    assert_eq!(a.columns[0].unit, None);
    assert_eq!(a.columns[1].unit.as_deref(), Some("kg"));
    assert_eq!(a.columns[2].unit.as_deref(), Some("%"));
    assert_eq!(a.columns[3].unit.as_deref(), Some("mm"));
}

#[test]
fn number_of_cases_is_a_measure() {
    let mut t = String::from("Number of cases,Deaths,Recovered,Case number,Total\n");
    for i in 1..=12 {
        let (a, b, c) = (i * 371 % 900 + 100, i * 533 % 700 + 200, i * 17 % 60 + 15);
        t.push_str(&format!("{a},{b},{c},{},{}\n", i * 7 % 5 + 3, a + b + c));
    }
    let a = full(&t);
    assert_eq!(a.derived.len(), 1, "{:?}", a.derived);
    assert_eq!(a.derived[0].parts, vec![0, 1, 2]);
    let t = t.replace("Number of cases", "Case no");
    assert!(full(&t).derived.is_empty());
}

#[test]
fn real_csv_with_codes_and_quoted_text_is_accepted() {
    let mut pc = String::new();
    for i in 0..20 {
        pc.push_str(&format!(
            "M{} {}AB,{} Main Street,Manchester\n",
            i % 9 + 1,
            i % 9 + 1,
            i + 1
        ));
    }
    assert_eq!(sniff(&pc).unwrap().delimiter, b',');
    let mut sku = String::from("sku,name,price\n");
    for i in 0..20 {
        sku.push_str(&format!("M{}00,Widget {i},{}.99\n", i + 1, i + 3));
    }
    assert_eq!(sniff(&sku).unwrap().delimiter, b',');
    let mut semi = String::from("sku;name;price\n");
    for i in 0..20 {
        semi.push_str(&format!("G{}0;Widget {i};{}.99\n", i + 1, i + 3));
    }
    assert_eq!(sniff(&semi).unwrap().delimiter, b';');
    let mut tr = String::from("english,german\n");
    for i in 0..20 {
        tr.push_str(&format!(
            "\"I would like to buy {i} apples please\",\"Ich moechte bitte {i} Aepfel kaufen\"\n"
        ));
    }
    assert_eq!(sniff(&tr).unwrap().delimiter, b',');
    let mut qa = String::from("id,question,answer\n");
    for i in 0..20 {
        qa.push_str(&format!(
            "{i},\"What is the capital of country {i}?\",\"The capital of country {i} is city {i}\"\n"
        ));
    }
    assert_eq!(sniff(&qa).unwrap().delimiter, b',');
}

#[test]
fn consecutive_ids_are_judged_over_all_rows() {
    let mut t = String::from("n,A,B,Total\n");
    for i in 0..30u64 {
        let n = (i * 7) % 30 + 100;
        let (a, b) = (i * 371 % 900 + 1000, i * 533 % 700 + 2000);
        t.push_str(&format!("{n},{a},{b},{}\n", n + a + b));
    }
    let d = sniff(&t).unwrap();
    assert!(analyze(&t, &d, 15).derived.is_empty());
}

#[test]
fn identifier_columns_do_not_use_up_numeric_slots() {
    let mut s = 99u64;
    let mut t = String::from("k");
    for i in 1..=11 {
        t.push_str(&format!(",m{i}"));
    }
    t.push_str(",Total\n");
    for i in 0..40u64 {
        let k = (i * 7) % 40 + 1;
        let m: Vec<u64> = (0..11).map(|_| xorshift(&mut s) % 4000 + 1000).collect();
        let total = m[0] + m[1] + m[2];
        let cells: Vec<String> = m.iter().map(u64::to_string).collect();
        t.push_str(&format!("{k},{},{total}\n", cells.join(",")));
    }
    let a = full(&t);
    assert!(
        a.derived
            .iter()
            .any(|d| d.target == 12 && d.parts == vec![1, 2, 3]),
        "{:?}",
        a.derived
    );
}

#[test]
fn two_column_semicolon_exports_with_codes_are_accepted() {
    let mut t = String::from("sku;name\n");
    for i in 0..20 {
        t.push_str(&format!("M{}00;Widget {i}\n", i + 1));
    }
    assert_eq!(sniff(&t).unwrap().delimiter, b';');
    let mut t = String::from("name;value\n");
    for i in 0..20 {
        t.push_str(&format!(";{i}\n"));
    }
    assert_eq!(sniff(&t).unwrap().delimiter, b';');
    // Real G-code with a comment header is still rejected.
    let mut g = String::from("; generated\n");
    for i in 0..30 {
        g.push_str(&format!("G1 X{i} ; move\n"));
    }
    assert_eq!(sniff(&g), None);
}

#[test]
fn long_increasing_id_column_is_not_a_part() {
    let mut t = String::from("n,A,B,Total\n");
    for i in 1..=250_000u64 {
        let (a, b) = (i * 371 % 900 + 5000, i * 533 % 700 + 6000);
        t.push_str(&format!("{i},{a},{b},{}\n", i + a + b));
    }
    let a = full(&t);
    assert!(a.columns[0].unique_exhausted && !a.columns[0].unique);
    assert!(a.derived.is_empty(), "{:?}", a.derived);
}

#[test]
fn analyze_until_stops_early_and_says_so() {
    let mut text = String::from("id,name\n");
    for i in 0..50_000 {
        text.push_str(&format!("{i},n{i}\n"));
    }
    let d = sniff(&text).unwrap();
    let calls = std::cell::Cell::new(0usize);
    let stop = || {
        calls.set(calls.get() + 1);
        true
    };
    let a = csvsniff::analyze_until(&text, &d, 1000, &stop);
    assert!(a.truncated);
    assert!(a.rows < 50_000, "rows {}", a.rows);
    assert!(a.quote_problems.is_empty() && a.derived.is_empty());
    let whole = analyze(&text, &d, 1000);
    assert!(!whole.truncated);
    assert_eq!(whole.rows, 50_000);
}

#[test]
fn sniff_looks_only_at_the_first_mebibyte() {
    // A normal start, then a record of 64 MiB: sniffing must not scan it.
    let mut text = String::from("a,b,c\n1,2,3\n4,5,6\n7,8,9\n");
    text.push_str(&"x".repeat(64 * 1024 * 1024));
    text.push('\n');
    let start = std::time::Instant::now();
    let d = sniff(&text).expect("dialect from the start of the text");
    assert_eq!(d.delimiter, b',');
    assert!(
        start.elapsed() < std::time::Duration::from_millis(150),
        "{:?}",
        start.elapsed()
    );
    // Multi-byte text cut at the cap must not panic.
    let wide = "é,ö\n".repeat(400_000);
    assert!(sniff(&wide).is_some());
}

#[test]
fn cell_class_matches_header_rule() {
    let cases = [
        ("", CellClass::Empty),
        ("  ", CellClass::Empty),
        ("12", CellClass::Number),
        (" -1.5 ", CellClass::Number),
        ("+3", CellClass::Number),
        ("2024-01-31", CellClass::Date),
        ("31-May-13", CellClass::Date),
        ("2024-01-31T10:00:00Z", CellClass::Date),
        ("yes", CellClass::Bool),
        ("TRUE", CellClass::Bool),
        ("abc", CellClass::Text),
        ("1,5", CellClass::Text),
        ("1.", CellClass::Text),
    ];
    for (cell, class) in cases {
        assert_eq!(cell_class(cell), class, "{cell:?}");
    }
    // The header rule counts a bool cell as text: `yes` heads a column of numbers.
    assert!(sniff("flag,yes\n1,2\n3,4\n5,6\n").unwrap().has_header);
    // A number in the first row is no header, a date neither.
    assert!(!sniff("a,1\nb,2\nc,3\n").unwrap().has_header);
    assert!(
        !sniff("x,2024-01-31\ny,2024-02-01\nz,2024-03-01\n")
            .unwrap()
            .has_header
    );
}

#[test]
fn sniff_skips_title_lines() {
    let text = "Station report 2024\nSource: test data\nid,name,value\n1,alpha,10\n2,beta,20\n3,gamma,30\n";
    let d = sniff(text).expect("dialect");
    assert_eq!(d.delimiter, b',');
    assert_eq!(d.lead_records, 2);
    assert!(d.has_header);
    let a = analyze(text, &d, 100_000);
    let names: Vec<&str> = a.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "value"]);
    assert_eq!(a.rows, 3);
    assert!(a.ragged_rows.is_empty());
    assert_eq!(a.columns[0].kind, ColumnKind::Integer);
}

#[test]
fn plain_and_ragged_files_have_no_lead() {
    assert_eq!(sniff("a,b,c\n1,2,3\n4,5,6\n").unwrap().lead_records, 0);
    // A short header over wider rows passes as it is: no title line is taken off.
    let mut text = String::from("a,b\n1,2,3\n");
    for i in 0..12 {
        text.push_str(&format!("{i},{i},{i}\n"));
    }
    let d = sniff(&text).unwrap();
    assert_eq!(d.lead_records, 0);
    // More than ten short lines are no title lines.
    let mut text = String::new();
    for i in 0..11 {
        text.push_str(&format!("note {i}\n"));
    }
    text.push_str("a,b,c\n1,2,3\n4,5,6\n");
    assert_eq!(sniff(&text), None);
}

#[test]
fn prose_with_a_title_line_is_no_csv() {
    let letter = "Dear Anna\nThank you for the letter, it was nice\nThe kids are fine, school is ok\nSee you soon, Jordi\n";
    assert_eq!(sniff(letter), None);
    let list = "Shopping list\nmilk, eggs\nbread, butter\njam, tea\n";
    assert_eq!(sniff(list), None);
    // Two columns under a title are a table when a header names a column of numbers.
    let d =
        sniff("Prices 2024\nSource: shop\nitem,price\nmilk,1.20\nbread,2.50\njam,3.10\n").unwrap();
    assert_eq!(d.lead_records, 2);
    assert!(d.has_header);
}
