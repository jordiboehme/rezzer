// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sheetparts::numfmt::{DateSystem, NumKind, builtin, format, format_with, kind_of};

fn text(code: &str, value: f64) -> String {
    format(code, value).text
}

#[test]
fn percent_0_09_is_9_percent() {
    let f = format("0%", 0.09);
    assert_eq!(f.text, "9%");
    assert_eq!(f.kind, NumKind::Percent);
    assert_eq!(text("0.0 %", 0.09), "9.0 %");
    assert_eq!(text("0.00%", 0.12345), "12.35%");
    assert_eq!(text("0%", -0.5), "-50%");
}

#[test]
fn thousands_and_decimals() {
    assert_eq!(text("#,##0", 1234567.0), "1,234,567");
    assert_eq!(text("#,##0.00", 1234.5), "1,234.50");
    assert_eq!(text("0.00", 3.14259), "3.14");
    assert_eq!(text("0", 2.5), "3");
    assert_eq!(text("0.00", 2.675), "2.68");
    assert_eq!(text("0.00", 0.125), "0.13");
    assert_eq!(text("000", 7.0), "007");
    assert_eq!(text("#,##0", 0.0), "0");
    assert_eq!(text("#.##", 5.0), "5.");
    assert_eq!(text("0.0#", 1.5), "1.5");
    assert_eq!(text("0.0#", 1.25), "1.25");
    assert_eq!(text("?.??", 1.5), "1.5 ");
    assert_eq!(text("#,##0,", 1234567.0), "1,235");
    assert_eq!(text("0.0,,", 2_500_000.0), "2.5");
    assert_eq!(text("#,##0", 999.5), "1,000");
    assert_eq!(text("00-00", 1234.0), "12-34");
    assert_eq!(format("#,##0.00", 1.0).kind, NumKind::Number);
}

#[test]
fn currency_symbol_is_unit() {
    let f = format("\"$\"#,##0.00", 1234.5);
    assert_eq!(f.text, "$1,234.50");
    assert_eq!(f.kind, NumKind::Currency);
    assert_eq!(f.unit.as_deref(), Some("$"));

    let euro = format("#,##0.00\\ [$\u{20ac}-407]", 12.5);
    assert_eq!(euro.text, "12.50 \u{20ac}");
    assert_eq!(euro.unit.as_deref(), Some("\u{20ac}"));
    assert_eq!(euro.kind, NumKind::Currency);

    let pound = format("\"\u{a3}\"#,##0.00", 3.0);
    assert_eq!(pound.unit.as_deref(), Some("\u{a3}"));

    let builtin7 = format(builtin(7).unwrap(), -1234.5);
    assert_eq!(builtin7.text, "($1,234.50)");
    assert_eq!(builtin7.kind, NumKind::Currency);
    assert_eq!(format("[$$-409]#,##0", 5.0).text, "$5");
}

#[test]
fn literal_unit_text() {
    let f = format("0.0\" m\"", 12.34);
    assert_eq!(f.text, "12.3 m");
    assert_eq!(f.unit.as_deref(), Some("m"));
    assert_eq!(f.kind, NumKind::Number);
    let kg = format("0.00 \"kg\"", 2.0);
    assert_eq!(kg.text, "2.00 kg");
    assert_eq!(kg.unit.as_deref(), Some("kg"));
    assert_eq!(format("0.0", 1.0).unit, None);
}

#[test]
fn dates_1900_and_1904() {
    let d = format("yyyy-mm-dd", 45000.0);
    assert_eq!(d.text, "2023-03-15");
    assert_eq!(d.kind, NumKind::Date);
    assert_eq!(text("yyyy-mm-dd", 1.0), "1900-01-01");
    assert_eq!(text("yyyy-mm-dd", 59.0), "1900-02-28");
    assert_eq!(text("yyyy-mm-dd", 60.0), "1900-02-29");
    assert_eq!(text("yyyy-mm-dd", 61.0), "1900-03-01");
    assert_eq!(text("m/d/yyyy", 36526.0), "1/1/2000");
    assert_eq!(
        format_with("yyyy-mm-dd", 0.0, DateSystem::Date1904).text,
        "1904-01-01"
    );
    assert_eq!(
        format_with("yyyy-mm-dd", 43000.0, DateSystem::Date1904).text,
        "2021-09-23"
    );
    assert_eq!(
        text("dddd, mmmm d, yyyy", 45000.0),
        "Wednesday, March 15, 2023"
    );
    assert_eq!(text("ddd mmm yy", 45000.0), "Wed Mar 23");
    assert_eq!(text("d-mmm-yy", 45000.0), "15-Mar-23");

    let t = format("h:mm:ss", 0.75);
    assert_eq!(t.text, "18:00:00");
    assert_eq!(t.kind, NumKind::Time);
    assert_eq!(text("h:mm AM/PM", 0.75), "6:00 PM");
    assert_eq!(text("hh:mm", 45000.5104166667), "12:15");
    assert_eq!(text("yyyy-mm-dd hh:mm", 45000.5), "2023-03-15 12:00");
    assert_eq!(format("yyyy-mm-dd hh:mm", 45000.5).kind, NumKind::Date);
    assert_eq!(text("[h]:mm:ss", 1.5), "36:00:00");
    assert_eq!(text("mm:ss", 0.000694444), "01:00");
    assert_eq!(text("h:mm:ss.00", 0.5000011574), "12:00:00.10");
    assert_eq!(text("yyyy-mm-dd", -1.0), "####");
    assert_eq!(kind_of("d/m/yy;@"), NumKind::Date);
    assert_eq!(kind_of("0.00"), NumKind::Number);
}

#[test]
fn negative_section() {
    assert_eq!(text("0.00;(0.00)", -3.5), "(3.50)");
    assert_eq!(text("0.00;(0.00)", 3.5), "3.50");
    assert_eq!(text("0.00;[Red]-0.00", -3.5), "-3.50");
    assert_eq!(text("0;-0;\"zero\"", 0.0), "zero");
    assert_eq!(text("0;-0;\"zero\"", 5.0), "5");
    assert_eq!(text("#,##0.00", -1234.5), "-1,234.50");
    assert_eq!(text("[>100]\"big\";0", 500.0), "big");
    assert_eq!(text("[>100]\"big\";0", 5.0), "5");
    assert_eq!(text(";;;", 5.0), "");
    assert_eq!(text("#,##0_);(#,##0)", -5.0), "(5)");
}

#[test]
fn general_scientific_fraction_text() {
    assert_eq!(text("General", 5.0), "5");
    assert_eq!(text("General", 0.1 + 0.2), "0.3");
    assert_eq!(text("General", 1234.5678), "1234.5678");
    assert_eq!(text("General", -2.5), "-2.5");
    assert_eq!(text("General", 123456789012.0), "1.23457E+11");
    assert_eq!(format("General", 1.0).kind, NumKind::General);
    assert_eq!(text("0.00E+00", 12345.0), "1.23E+04");
    assert_eq!(text("0.00E+00", 0.00012), "1.20E-04");
    assert_eq!(text("##0.0E+0", 12345.0), "12.3E+3");
    assert_eq!(format("0.00E+00", 1.0).kind, NumKind::Scientific);
    assert_eq!(text("# ?/?", 1.5), "1 1/2");
    assert_eq!(text("# ??/??", 0.3333333), "  1/3 ");
    assert_eq!(text("?/?", 0.5), "1/2");
    assert_eq!(text("# ?/8", 0.5), " 4/8");
    assert_eq!(format("# ?/?", 1.5).kind, NumKind::Fraction);
    assert_eq!(format("@", 5.0).kind, NumKind::Text);
    assert_eq!(text("@", 5.0), "5");
    // Broken codes never panic and fall back to General.
    assert_eq!(text("\"open", 5.0), "open");
    assert_eq!(text("[", 5.0), "");
}

#[test]
fn builtin_table() {
    assert_eq!(builtin(0), Some("General"));
    assert_eq!(builtin(1), Some("0"));
    assert_eq!(builtin(9), Some("0%"));
    assert_eq!(builtin(10), Some("0.00%"));
    assert_eq!(builtin(14), Some("mm-dd-yy"));
    assert_eq!(builtin(49), Some("@"));
    assert_eq!(builtin(23), None);
    assert_eq!(builtin(164), None);
    for id in 0..50 {
        if let Some(code) = builtin(id) {
            // Every built-in code formats without panic.
            let _ = format(code, 1234.5);
            let _ = format(code, -0.5);
        }
    }
}

#[test]
fn hostile_inputs_do_not_panic() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e300,
        -1e300,
        5e-324,
        0.0,
        -0.0,
    ] {
        for code in [
            "General",
            "0.00",
            "#,##0",
            "0%",
            "yyyy-mm-dd",
            "h:mm",
            "0.00E+00",
            "# ?/?",
            "@",
            "[h]:mm",
            "0;0;0;@",
        ] {
            let _ = format(code, value);
        }
    }
    let long = "0".repeat(10_000);
    let _ = format(&long, 1.0);
    let _ = format(&"[".repeat(100), 1.0);
}

#[test]
fn more_cases_checked_by_hand() {
    assert_eq!(text("# ?/?", 2.0), "2    ");
    assert_eq!(text("# ?/?", 0.0), "0    ");
    assert_eq!(text("h:mm AM/PM", 0.5), "12:00 PM");
    assert_eq!(text("h:mm AM/PM", 0.0), "12:00 AM");
    assert_eq!(text("mmm-yy", 45000.0), "Mar-23");
    assert_eq!(text("m/d/yy h:mm", 45000.75), "3/15/23 18:00");
    assert_eq!(text("General", 1e-5), "1E-05");
    assert_eq!(text("General", 0.00012), "0.00012");
    assert_eq!(text("0.000", -0.0004), "-0.000");
    assert_eq!(text("yyyy-mm-dd", 1e9), "####");
    assert_eq!(text("#,##0.0,,\"M\"", 12_345_678.0), "12.3M");
    assert_eq!(text("00.00", 5.5), "05.50");
    assert_eq!(format("\"x\"0", 3.0).unit.as_deref(), Some("x"));
}
