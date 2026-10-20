// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use formulakit::{CellPos, Dialect, analyze, latex, worth_math};

fn tex(text: &str) -> String {
    latex(text, Dialect::Excel).expect("formula renders")
}

fn worth(text: &str) -> bool {
    worth_math(&analyze(text, Dialect::Excel, CellPos::default()).expect("formula parses"))
}

#[test]
fn latex_of_division_and_power() {
    assert_eq!(
        tex("=B2*C2/1000"),
        r"\frac{\mathrm{B2} \cdot \mathrm{C2}}{1000}"
    );
    assert_eq!(
        tex("=SQRT(A1^2+B1^2)"),
        r"\sqrt{\mathrm{A1}^{2}+\mathrm{B1}^{2}}"
    );
}

#[test]
fn latex_adds_parentheses_by_precedence() {
    assert_eq!(
        tex("=(A1+B1)*C1"),
        r"(\mathrm{A1}+\mathrm{B1}) \cdot \mathrm{C1}"
    );
    assert_eq!(tex("=A1-(B1+C1)"), r"\mathrm{A1}-(\mathrm{B1}+\mathrm{C1})");
    assert_eq!(tex("=A1-B1+C1"), r"\mathrm{A1}-\mathrm{B1}+\mathrm{C1}");
    // Excel binds the minus sign tighter than the power: -2^2 is 4.
    assert_eq!(tex("=-2^2"), r"(-2)^{2}");
    assert_eq!(tex("=(A1+1)^(1/3)"), r"(\mathrm{A1}+1)^{\frac{1}{3}}");
    assert_eq!(tex("=A1*-B1"), r"\mathrm{A1} \cdot (-\mathrm{B1})");
    assert_eq!(tex("=(A1/B1)^2"), r"(\frac{\mathrm{A1}}{\mathrm{B1}})^{2}");
    assert_eq!(tex("=A1*10%"), r"\mathrm{A1} \cdot 10\%");
}

#[test]
fn latex_functions_constants_and_references() {
    assert_eq!(tex("=PI()*$A$1^2"), r"\pi \cdot \mathrm{A1}^{2}");
    assert_eq!(tex("=POWER(A1+1,3)"), r"(\mathrm{A1}+1)^{3}");
    assert_eq!(
        tex("=ROUND(A1/3,2)"),
        r"\operatorname{ROUND}(\frac{\mathrm{A1}}{3}, 2)"
    );
    assert_eq!(
        tex("=_xlfn.STDEV.S(A1:A9)"),
        r"\operatorname{STDEV.S}(\mathrm{A1{:}A9})"
    );
    assert_eq!(
        tex("=Data!B2*rate"),
        r"\mathrm{Data!B2} \cdot \mathrm{rate}"
    );
    assert_eq!(tex("=my_rate*2"), r"\mathrm{my\_rate} \cdot 2");
    assert_eq!(
        tex("=IF(A1>=B1,\"ok\",1.5E+3)"),
        r#"\operatorname{IF}(\mathrm{A1} \geq \mathrm{B1}, \text{"ok"}, 1.5E+3)"#
    );
    assert_eq!(
        latex("of:=[.B2]*[.C2]/1000", Dialect::OpenFormula).as_deref(),
        Some(r"\frac{\mathrm{B2} \cdot \mathrm{C2}}{1000}")
    );
    assert_eq!(latex("=SUM(", Dialect::Excel), None);
    assert_eq!(
        tex("=my_fn(A1)*2"),
        r"\operatorname{my\_fn}(\mathrm{A1}) \cdot 2"
    );
    assert_eq!(tex("=Größe(A1)*2"), r"\text{Größe}(\mathrm{A1}) \cdot 2");
}

#[test]
fn latex_escapes_what_a_file_writes() {
    // A sheet name or a string can hold TeX commands; they must come out as text.
    let out = tex(r#"='a{\href{x}}%$#'!A1*2&"\url{y}^~""#);
    assert!(!out.contains(r"\href"), "{out}");
    assert!(!out.contains(r"\url"), "{out}");
    assert_eq!(
        out,
        r#"\text{'a\{\textbackslash{}href\{x\}\}\%\$\#'!A1} \cdot 2 \mathbin{\&} \text{"\textbackslash{}url\{y\}\textasciicircum{}\textasciitilde{}"}"#
    );
    let out = tex("='Übersicht'!A1/2");
    assert_eq!(out, r"\frac{\text{Übersicht!A1}}{2}");
}

#[test]
fn simple_sum_is_not_worth_math() {
    for text in [
        "=SUM(A1:A9)",
        "=A1+B1-C1",
        "=A1",
        "=5",
        "=-A1",
        "=Sheet1!A1:Sheet1!B2",
    ] {
        assert!(!worth(text), "{text}");
    }
    for text in [
        "=A1*2",
        "=A1/B1",
        "=SQRT(A1)",
        "=A1^2",
        "=ROUND(A1,2)",
        "=A1%",
    ] {
        assert!(worth(text), "{text}");
    }
}
