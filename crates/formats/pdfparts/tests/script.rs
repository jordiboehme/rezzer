// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::{Duration, Instant};

use pdfparts::script::{FieldRefs, describe, field_refs};

fn words(script: &str) -> Option<String> {
    describe(script)
}

#[test]
fn number_formats_in_words() {
    let cases = [
        (
            r#"AFNumber_Format(2, 0, 0, 0, "$", true);"#,
            r#"number, 2 decimals, "$" before the number"#,
        ),
        (
            r#"AFNumber_Format(2, 1, 0, 0, "", true);"#,
            "number, 2 decimals, no thousands separator",
        ),
        (
            r#"AFNumber_Format(2, 2, 1, 0, "€", false)"#,
            r#"number, 2 decimals, separator 1.234,56, negative in red, "€" after the number"#,
        ),
        (
            r#"AFNumber_Format(1, 3, 2, 0, "", true);"#,
            "number, 1 decimal, separator 1234,56, negative in parentheses",
        ),
        (
            r#"AFNumber_Format(0, 0, 3, 0, "", true);"#,
            "number, 0 decimals, negative in red parentheses",
        ),
        ("AFPercent_Format(1, 0)", "percent, 1 decimal"),
        (
            "AFPercent_Format(2, 1);",
            "percent, 2 decimals, no thousands separator",
        ),
    ];
    for (script, want) in cases {
        assert_eq!(words(script).as_deref(), Some(want), "{script}");
    }
}

#[test]
fn date_time_and_special_formats_in_words() {
    let cases = [
        (r#"AFDate_FormatEx("mm/dd/yyyy");"#, "date mm/dd/yyyy"),
        (r#"AFDate_KeystrokeEx("mm/dd/yyyy");"#, "date mm/dd/yyyy"),
        ("AFDate_Format(0);", "date m/d"),
        ("AFDate_Format(13);", "date m/d/yy HH:MM"),
        ("AFTime_Format(2);", "time HH:MM:ss"),
        ("AFTime_Format(1)", "time h:MM tt"),
        ("AFSpecial_Format(0);", "ZIP code"),
        ("AFSpecial_Format(1);", "ZIP+4 code"),
        ("AFSpecial_Format(2);", "phone number"),
        ("AFSpecial_Format(3);", "social security number"),
        (r#"AFSpecial_KeystrokeEx("999-9999");"#, "mask 999-9999"),
    ];
    for (script, want) in cases {
        assert_eq!(words(script).as_deref(), Some(want), "{script}");
    }
}

#[test]
fn simple_calculate_reads_fields() {
    let script = r#"AFSimple_Calculate("SUM", new Array("a", "b"));"#;
    assert_eq!(words(script).as_deref(), Some("sum of a, b"));
    assert_eq!(
        field_refs(script),
        FieldRefs {
            names: vec!["a".into(), "b".into()],
            dynamic: false
        }
    );
    for (f, word) in [
        ("PRD", "product"),
        ("AVG", "average"),
        ("MIN", "minimum"),
        ("MAX", "maximum"),
    ] {
        let s = format!(r#"AFSimple_Calculate("{f}", ["x.1", 'y'])"#);
        assert_eq!(words(&s), Some(format!("{word} of x.1, y")));
    }
    // A list in one string, as Acrobat allows.
    let s = r#"AFSimple_Calculate("SUM", "Line 1, Line 2");"#;
    assert_eq!(words(s).as_deref(), Some("sum of Line 1, Line 2"));
    assert_eq!(field_refs(s).names, ["Line 1", "Line 2"]);
    // A name built at run time.
    let s = r#"AFSimple_Calculate("SUM", new Array("a", prefix + "b"));"#;
    assert_eq!(words(s), None);
    assert!(field_refs(s).dynamic);
    assert_eq!(words(r#"AFSimple_Calculate("MEDIAN", ["a"])"#), None);
}

#[test]
fn range_validate_words() {
    let cases = [
        (
            "AFRange_Validate(true, 0, true, 100);",
            Some("between 0 and 100"),
        ),
        ("AFRange_Validate(true, 0, false, 100);", Some("at least 0")),
        (
            "AFRange_Validate(false, 0, true, 100);",
            Some("at most 100"),
        ),
        (
            "AFRange_Validate(true, -1.5, true, 2.25)",
            Some("between -1.5 and 2.25"),
        ),
        ("AFRange_Validate(false, 0, false, 0);", None),
        ("AFRange_Validate(1, 0, 1, 100);", None),
    ];
    for (script, want) in cases {
        assert_eq!(words(script).as_deref(), want, "{script}");
    }
}

#[test]
fn odd_af_arguments_keep_call_text() {
    for script in [
        "AFNumber_Format(2)",
        r#"AFNumber_Format("x", 0, 0, 0, "$", true)"#,
        r#"AFNumber_Format(2, 4, 0, 0, "", true)"#,
        r#"AFNumber_Format(2, 0, 7, 0, "", true)"#,
        r#"AFNumber_Format(-1, 0, 0, 0, "", true)"#,
        r#"AFNumber_Format(2, 0, 0, 0, "$", true); doMore();"#,
        r#"AFNumber_Keystroke(2, 0, 0, 0, "$", true);"#,
        r#"AFTime_Keystroke("HH:MM:ss");"#,
        "AFDate_Format(14)",
        "AFSpecial_Format(4)",
        r#"AFDate_FormatEx("")"#,
        r#"AFDate_FormatEx(fmt)"#,
        "AFPercent_Format(1, 0, 3)",
        "event.value = 1;",
        "",
        "AFNumber_Format(2, 0, 0, 0, \"$\", true",
        "AFSimple_Calculate(\"SUM\", [[[[[[[[[[[[\"a\"]]]]]]]]]]]])",
        r#"AFNumber_Format(9223372036854775807, 0, 0, 0, "", true)"#,
        "AFNumber_Format(16, 0, 0, 0, \"\", true)",
        "AFPercent_Format(16, 0)",
    ] {
        assert_eq!(words(script), None, "{script}");
    }
}

#[test]
fn field_refs_from_get_field() {
    let refs = field_refs(r#"event.value = this.getField("Source").value;"#);
    assert_eq!(refs.names, ["Source"]);
    assert!(!refs.dynamic);
    let refs = field_refs(
        "var a = getField('x').value; var b = this.getField( \"a.b\" ).value;\n\
         var c = getField(\"x\").value; var d = getField(\"q\\\"r\\u0041\");",
    );
    assert_eq!(refs.names, ["x", "a.b", "q\"rA"]);
    assert!(!refs.dynamic);
    // A concatenation outside the call is not a dynamic name.
    let refs = field_refs(
        "function callback(x)\r\n{\r\nthis.getField(\"Field1\").value = event.target.name + \"=\" + event.target.value + \"::\" + x;\r\n}",
    );
    assert_eq!(refs.names, ["Field1"]);
    assert!(!refs.dynamic);
    // Words that only contain the name are not calls.
    assert_eq!(
        field_refs("mygetField(\"a\"); getFields(\"b\")"),
        FieldRefs::default()
    );
}

#[test]
fn absurd_numbers_are_odd() {
    let long = "1".repeat(31);
    assert_eq!(
        words(&format!("AFRange_Validate(true, {long}, false, 0)")),
        None
    );
    let ok = "1".repeat(30);
    assert_eq!(
        words(&format!("AFRange_Validate(true, {ok}, false, 0)")),
        Some(format!("at least {ok}"))
    );
    assert_eq!(
        words("AFNumber_Format(15, 0, 0, 0, \"\", true)").as_deref(),
        Some("number, 15 decimals")
    );
    // A name listed twice is named once.
    assert_eq!(
        words(r#"AFSimple_Calculate("SUM", ["a", "b", "a"])"#).as_deref(),
        Some("sum of a, b")
    );
}

#[test]
fn comments_and_strings_are_not_calls() {
    let refs = field_refs(
        "// getField(\"c1\")\n/* getField(\"c2\") getField(n) */\n\
         var s = \"getField('c3')\"; var t = 'getField(\"c4\")';\n\
         var u = getField(\"real\").value; // getField(x)",
    );
    assert_eq!(refs.names, ["real"]);
    assert!(!refs.dynamic);
    // An unclosed comment or string ends the scan.
    assert_eq!(field_refs("/* getField(\"a\")"), FieldRefs::default());
    assert_eq!(field_refs("'getField(\"a\")"), FieldRefs::default());
}

#[test]
fn dynamic_field_name_is_unknown() {
    for script in [
        "this.getField(name).value",
        "getField(\"row\" + i).value",
        "getField(\"a\" + \"b\")",
        "getField()",
        "getField(\"unclosed",
    ] {
        let refs = field_refs(script);
        assert!(refs.dynamic, "{script}");
        assert!(refs.names.is_empty(), "{script}");
    }
    let refs = field_refs("getField(\"a\").value + getField(n).value");
    assert_eq!(refs.names, ["a"]);
    assert!(refs.dynamic);
}

#[test]
fn long_scripts_are_scanned_once() {
    // 1 MB of unclosed calls and 1 MB of nested lists stay fast.
    let unclosed = "getField(\"".repeat(100_000);
    let nested = format!("AFSimple_Calculate(\"SUM\", {})", "[".repeat(1 << 20));
    let calls = "getField(\"a\");".repeat(100_000);
    let start = Instant::now();
    assert!(field_refs(&unclosed).dynamic);
    assert!(field_refs(&nested).dynamic);
    assert_eq!(describe(&nested), None);
    assert_eq!(field_refs(&calls).names, ["a"]);
    let took = start.elapsed();
    assert!(took < Duration::from_secs(1), "{took:?}");
}
