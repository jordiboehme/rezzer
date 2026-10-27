// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parsing of call arguments and literals.

use super::*;

/// The arguments of a call, given the text after its `(`. `None` when the closing `)` is not
/// the end of the text.
pub(super) fn top_level_args(text: &str) -> Option<Vec<String>> {
    let (args, rest) = call_args(text)?;
    rest.trim().is_empty().then_some(args)
}

/// The arguments of a call, given the text after its `(`, and the text after its `)`.
/// `None` when the call is not closed.
fn call_args(text: &str) -> Option<(Vec<String>, &str)> {
    let mut depth = 0usize;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' | '\'' => {
                current.push(c);
                // Quoted text or sheet name; a doubled quote stays inside.
                for (_, d) in chars.by_ref() {
                    current.push(d);
                    if d == c {
                        break;
                    }
                }
            }
            '(' | '{' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | '}' | ']' if depth > 0 => {
                depth -= 1;
                current.push(c);
            }
            ')' => {
                args.push(std::mem::take(&mut current));
                return Some((args, &text[i + 1..]));
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    None
}

/// The arguments of every call of `function` in a formula text, outside text literals and
/// quoted sheet names.
pub(super) fn calls_of(text: &str, function: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut prev: Option<char> = None;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            prev = Some(c);
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
        } else if !prev.is_some_and(|p| p.is_alphanumeric() || p == '_' || p == '.')
            && let Some(head) = text.get(i..i + function.len() + 1)
            && head
                .get(..function.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(function))
            && head.ends_with('(')
            && let Some((args, _)) = call_args(&text[i + function.len() + 1..])
        {
            out.push(args);
        }
        prev = Some(c);
    }
    out
}

/// The text of a string literal argument (`"mi"`), with doubled quotes undone.
pub(super) fn text_literal(arg: &str) -> Option<String> {
    let inner = arg.trim().strip_prefix('"')?.strip_suffix('"')?;
    let text = inner.replace("\"\"", "\"");
    (!text.is_empty()).then_some(text)
}

/// The one cell a formula consists of (`=B5`, `=$L$3`, `=z!$L$3`), with its sheet. A
/// reference without a sheet points into `sheet`; `None` when there is none to point to.
pub(super) fn plain_cell(a: &Analysis, sheet: Option<&str>) -> Option<(String, CellPos)> {
    let [
        Ref::Cell {
            sheet: span, pos, ..
        },
    ] = a.refs.as_slice()
    else {
        return None;
    };
    if !a.functions.is_empty() || span.as_ref().is_some_and(|s| s.last.is_some()) {
        return None;
    }
    let target = match span {
        Some(s) => s.first.clone(),
        None => sheet?.to_string(),
    };
    let cell = crate::grid::a1(pos.row, pos.col);
    let shown = a.display.trim_start_matches('=').replace('$', "");
    let alone = shown == cell || shown == format!("{}!{cell}", quote_sheet(&target));
    alone.then_some((target, *pos))
}

/// The value of a name whose whole text is one number (`0.05`, `=-3`, `1.5E-3`, `5%`).
pub(super) fn literal_number(text: &str) -> Option<Value> {
    let t = text.trim().trim_start_matches('=').trim();
    let (digits, percent) = match t.strip_suffix('%') {
        Some(d) => (d.trim_end(), true),
        None => (t, false),
    };
    let body = digits.strip_prefix(['-', '+']).unwrap_or(digits);
    if !body.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'E' | 'e' | '+' | '-'))
    {
        return None;
    }
    let v: f64 = digits.parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(if percent {
        Value::percent(v / 100.0)
    } else {
        Value::number(v, None)
    })
}
