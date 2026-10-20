// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Formulas as LaTeX math, limited to what KaTeX renders.
//!
//! Division becomes `\frac`, powers `^{}`, `SQRT` `\sqrt{}`, `PI()` `\pi`, multiplication
//! `\cdot`, other functions `\operatorname{NAME}(...)` and references `\mathrm{B2}`. The tree
//! has no parentheses, so they are put back where precedence needs them. Text from the file
//! (sheet names, defined names, strings) is escaped, so it can never become a TeX command.

use crate::engine::{self, BinOp, Lit, Node, Target, UnOp};
use crate::refs::{Ref, address, sheet_prefix};
use crate::{Analysis, Dialect, excel_text, functions};

/// The formula as LaTeX math (without `$` delimiters), or `None` when it cannot be read.
/// `analysis_text` is the formula text, for example [`Analysis::display`].
pub fn latex(analysis_text: &str, dialect: Dialect) -> Option<String> {
    let excel = excel_text(analysis_text, dialect).ok()?;
    let tree = engine::parse(&excel).ok()?;
    if has_unknown(&tree) {
        return None;
    }
    Some(render(&tree, &excel).text)
}

/// Whether the tree holds an operator this code does not know, which it cannot write.
fn has_unknown(node: &Node) -> bool {
    match node {
        Node::Ref { .. } | Node::Area { .. } | Node::Literal(_) | Node::Empty => false,
        Node::Func { args, .. } => args.iter().any(has_unknown),
        Node::Call { callee, args } => has_unknown(callee) || args.iter().any(has_unknown),
        Node::Unary { op, expr } => *op == UnOp::Other || has_unknown(expr),
        Node::Binary { op, left, right } => {
            *op == BinOp::Other || has_unknown(left) || has_unknown(right)
        }
        Node::Array(rows) => rows.iter().flatten().any(has_unknown),
    }
}

/// Whether the formula is worth showing as math: it has an operator or a function beyond
/// `+`, `-` and `SUM`. A single reference, a constant, a sum or a plain addition is not.
/// Reference operators (`:`, union, intersection, `#`, `@`) do not count.
pub fn worth_math(analysis: &Analysis) -> bool {
    engine::parse(&analysis.display).is_ok_and(|tree| has_math(&tree))
}

fn has_math(node: &Node) -> bool {
    match node {
        Node::Ref { .. } | Node::Area { .. } | Node::Literal(_) | Node::Empty => false,
        Node::Func { name, args, .. } => {
            !functions::strip_prefixes(name)
                .0
                .eq_ignore_ascii_case("SUM")
                || args.iter().any(has_math)
        }
        Node::Call { .. } => true,
        Node::Unary { op, expr } => match op {
            UnOp::Plus | UnOp::Minus | UnOp::Spill | UnOp::At => has_math(expr),
            UnOp::Percent | UnOp::Other => true,
        },
        Node::Binary { op, left, right } => match op {
            BinOp::Add | BinOp::Sub | BinOp::Range | BinOp::Union | BinOp::Intersect => {
                has_math(left) || has_math(right)
            }
            _ => true,
        },
        Node::Array(rows) => rows.iter().flatten().any(has_math),
    }
}

/// Binding strength, higher binds tighter.
const CMP: u8 = 1;
const CONCAT: u8 = 2;
const ADD: u8 = 3;
const MUL: u8 = 4;
const NEG: u8 = 5;
const PERCENT: u8 = 6;
const POW: u8 = 7;
/// A fraction: needs no parentheses except as the base of a power.
const FRAC: u8 = 8;
const ATOM: u8 = 10;

struct Tex {
    text: String,
    prec: u8,
    /// A leading sign: wrapped when it follows another operator.
    signed: bool,
}

fn tex(text: String, prec: u8) -> Tex {
    Tex {
        text,
        prec,
        signed: false,
    }
}

fn wrap(t: Tex) -> String {
    format!("({})", t.text)
}

/// The operand text, in parentheses when it binds weaker than `min`.
fn operand(t: Tex, min: u8) -> String {
    if t.prec < min { wrap(t) } else { t.text }
}

/// A right operand: also wrapped when it starts with a sign (`a \cdot (-b)`).
fn right(t: Tex, min: u8) -> String {
    if t.prec < min || t.signed {
        wrap(t)
    } else {
        t.text
    }
}

/// Escapes text for `\text{}`.
fn text_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\textbackslash{}"),
            '^' => out.push_str(r"\textasciicircum{}"),
            '~' => out.push_str(r"\textasciitilde{}"),
            '{' | '}' | '#' | '$' | '%' | '&' | '_' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// A name or reference: `\mathrm{}` when it holds only safe ASCII, else `\text{}`.
fn word(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '!' | ':'));
    if safe {
        let inner = s.replace('_', r"\_").replace(':', "{:}");
        format!(r"\mathrm{{{inner}}}")
    } else {
        format!(r"\text{{{}}}", text_escape(s))
    }
}

/// A reference as a reader sees it: no `$`, sheets quoted only when needed.
fn reference(r: &Ref, written: &str) -> String {
    let plain = match r {
        Ref::Cell { sheet, .. } | Ref::Range { sheet, .. } => {
            let addr = address(r).unwrap_or_default();
            match sheet {
                Some(span) => format!("{}!{addr}", sheet_prefix(span)),
                None => addr,
            }
        }
        _ => written.replace('$', ""),
    };
    word(&plain)
}

fn args(list: &[Node], src: &str) -> String {
    list.iter()
        .map(|a| render(a, src).text)
        .collect::<Vec<_>>()
        .join(", ")
}

fn power(base: Tex, exp: Tex) -> Tex {
    tex(format!("{}^{{{}}}", operand(base, ATOM), exp.text), POW)
}

// Recursion depth is bounded by the parser's tree height limit.
fn render(node: &Node, src: &str) -> Tex {
    match node {
        Node::Empty => tex(String::new(), ATOM),
        Node::Literal(lit) => match lit {
            Lit::Number(n) => tex(n.clone(), ATOM),
            Lit::Text(t) => tex(format!(r#"\text{{"{}"}}"#, text_escape(t)), ATOM),
            Lit::Bool(b) => tex(word(if *b { "TRUE" } else { "FALSE" }), ATOM),
            Lit::Error(e) => tex(format!(r"\text{{{}}}", text_escape(e)), ATOM),
        },
        Node::Ref { span, target } => {
            let written = src.get(span.clone()).unwrap_or("");
            let text = match target {
                Target::Ref(r) => reference(r, written),
                Target::Name(raw) => word(functions::strip_prefixes(raw).0),
            };
            tex(text, ATOM)
        }
        Node::Area { folded, .. } => tex(reference(folded, ""), ATOM),
        Node::Func {
            name, args: list, ..
        } => {
            let bare = functions::strip_prefixes(name).0;
            let upper = bare.to_uppercase();
            match (upper.as_str(), list.as_slice()) {
                ("SQRT", [x]) => tex(format!(r"\sqrt{{{}}}", render(x, src).text), ATOM),
                ("PI", []) => tex(r"\pi".to_string(), ATOM),
                ("POWER", [b, e]) => power(render(b, src), render(e, src)),
                _ => {
                    let shown = if functions::is_builtin(&upper) {
                        upper.as_str()
                    } else {
                        bare
                    };
                    let safe = shown
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'));
                    let name = if safe {
                        format!(r"\operatorname{{{}}}", shown.replace('_', r"\_"))
                    } else {
                        format!(r"\text{{{}}}", text_escape(shown))
                    };
                    tex(format!("{name}({})", args(list, src)), ATOM)
                }
            }
        }
        Node::Call { callee, args: list } => {
            let callee = operand(render(callee, src), ATOM);
            tex(format!("{callee}({})", args(list, src)), ATOM)
        }
        Node::Array(rows) => {
            let rows: Vec<String> = rows.iter().map(|r| args(r, src)).collect();
            tex(format!(r"\{{{}\}}", rows.join("; ")), ATOM)
        }
        Node::Unary { op, expr } => {
            let inner = render(expr, src);
            match op {
                UnOp::Minus | UnOp::Plus => {
                    let sign = if *op == UnOp::Minus { "-" } else { "+" };
                    Tex {
                        text: format!("{sign}{}", operand(inner, NEG)),
                        prec: NEG,
                        signed: true,
                    }
                }
                UnOp::Percent => tex(format!(r"{}\%", operand(inner, PERCENT)), PERCENT),
                UnOp::Spill => tex(format!(r"{}\#", operand(inner, ATOM)), ATOM),
                UnOp::At => tex(format!("@{}", operand(inner, ATOM)), ATOM),
                // `latex` refuses trees with unknown operators before rendering.
                UnOp::Other => tex(String::new(), ATOM),
            }
        }
        Node::Binary { op, left, right: r } => {
            let (l, r) = (render(left, src), render(r, src));
            let infix = |sym: &str, prec: u8, l: Tex, r: Tex, right_min: u8| {
                let signed = l.signed && l.prec >= prec;
                Tex {
                    text: format!("{}{sym}{}", operand(l, prec), right(r, right_min)),
                    prec,
                    signed,
                }
            };
            match op {
                BinOp::Div => tex(format!(r"\frac{{{}}}{{{}}}", l.text, r.text), FRAC),
                BinOp::Pow => power(l, r),
                BinOp::Mul => infix(r" \cdot ", MUL, l, r, MUL),
                BinOp::Add => infix("+", ADD, l, r, ADD),
                BinOp::Sub => infix("-", ADD, l, r, ADD + 1),
                BinOp::Concat => infix(r" \mathbin{\&} ", CONCAT, l, r, CONCAT),
                BinOp::Eq => infix("=", CMP, l, r, CMP + 1),
                BinOp::Ne => infix(r" \neq ", CMP, l, r, CMP + 1),
                BinOp::Lt => infix("<", CMP, l, r, CMP + 1),
                BinOp::Gt => infix(">", CMP, l, r, CMP + 1),
                BinOp::Le => infix(r" \leq ", CMP, l, r, CMP + 1),
                BinOp::Ge => infix(r" \geq ", CMP, l, r, CMP + 1),
                BinOp::Range => tex(
                    format!("{}{{:}}{}", operand(l, ATOM), operand(r, ATOM)),
                    ATOM,
                ),
                BinOp::Union => tex(format!("({}, {})", l.text, r.text), ATOM),
                BinOp::Intersect => infix(r" \cap ", ATOM - 1, l, r, ATOM - 1),
                BinOp::Other => tex(String::new(), ATOM),
            }
        }
    }
}
