// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Validation conditions and cell range addresses in ODF notation.

use crate::a1::quote_sheet;

/// A validation condition taken apart.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Rule {
    pub kind: String,
    pub operator: Option<String>,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
    pub list: Option<Vec<String>>,
}

/// Splits `text` at `sep` where it is outside quotes and brackets.
fn split_top(text: &str, sep: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quote, mut sq, mut depth) = (false, false, 0i32);
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().unwrap_or(' ');
        if !quote && !sq && depth == 0 && rest.starts_with(sep) {
            out.push(std::mem::take(&mut cur));
            i += sep.len();
            continue;
        }
        match c {
            '"' if !sq => quote = !quote,
            '\'' if !quote => sq = !sq,
            '(' | '[' | '{' if !quote && !sq => depth += 1,
            ')' | ']' | '}' if !quote && !sq => depth -= 1,
            _ => {}
        }
        cur.push(c);
        i += c.len_utf8();
    }
    out.push(cur);
    out
}

/// `"a";"b"` as `a` and `b`, or `None` when an argument is no quoted literal.
fn literals(args: &[String]) -> Option<Vec<String>> {
    args.iter()
        .map(|a| {
            let a = a.trim();
            let inner = a.strip_prefix('"')?.strip_suffix('"')?;
            Some(inner.replace("\"\"", "\""))
        })
        .collect()
}

fn operator(name: &str) -> Option<&'static str> {
    Some(match name {
        "between" => "between",
        "not-between" => "notBetween",
        "equal-to" => "equal",
        "not-equal-to" => "notEqual",
        "greater-than" => "greaterThan",
        "less-than" => "lessThan",
        "greater-than-or-equal-to" => "greaterThanOrEqual",
        "less-than-or-equal-to" => "lessThanOrEqual",
        _ => return None,
    })
}

fn symbol_operator(sym: &str) -> Option<&'static str> {
    Some(match sym {
        "=" => "equal",
        "<>" | "!=" => "notEqual",
        ">" => "greaterThan",
        "<" => "lessThan",
        ">=" => "greaterThanOrEqual",
        "<=" => "lessThanOrEqual",
        _ => return None,
    })
}

/// Reads a condition like `of:cell-content-is-whole-number() and of:cell-content-is-between(1;10)`.
/// `None` when a clause is not understood.
pub(super) fn parse_condition(condition: &str) -> Option<Rule> {
    let mut rule = Rule::default();
    let text = condition.trim();
    if text.is_empty() {
        rule.kind = "none".to_string();
        return Some(rule);
    }
    for clause in split_top(text, " and ") {
        let clause = clause.trim();
        let clause = clause.strip_prefix("of:").unwrap_or(clause);
        let open = clause.find('(')?;
        let name = &clause[..open];
        let close = clause.rfind(')')?;
        if close < open {
            return None;
        }
        let inner = &clause[open + 1..close];
        let tail = clause[close + 1..].trim();
        let args: Vec<String> = if inner.trim().is_empty() {
            Vec::new()
        } else {
            split_top(inner, ";")
        };
        let arg = |i: usize| args.get(i).map(|a| a.trim().to_string());
        match name {
            "cell-content-is-whole-number" => rule.kind = "whole".to_string(),
            "cell-content-is-decimal-number" => rule.kind = "decimal".to_string(),
            "cell-content-is-date" => rule.kind = "date".to_string(),
            "cell-content-is-time" => rule.kind = "time".to_string(),
            "is-true-formula" => {
                rule.kind = "custom".to_string();
                rule.formula1 = Some(inner.trim().to_string());
            }
            "cell-content-is-in-list" => {
                rule.kind = "list".to_string();
                rule.list = literals(&args);
                rule.formula1 = Some(args.iter().map(|a| a.trim()).collect::<Vec<_>>().join(";"));
            }
            "cell-content-text-length" if !tail.is_empty() => {
                let sym_len = tail.chars().take_while(|c| "<>=!".contains(*c)).count();
                let (sym, value) = tail.split_at(sym_len);
                rule.kind = "textLength".to_string();
                rule.operator = Some(symbol_operator(sym)?.to_string());
                rule.formula1 = Some(value.trim().to_string());
            }
            _ => {
                if let Some(op) = name.strip_prefix("cell-content-text-length-is-") {
                    rule.kind = "textLength".to_string();
                    rule.operator = Some(operator(op)?.to_string());
                } else {
                    let op = name.strip_prefix("cell-content-is-")?;
                    rule.operator = Some(operator(op)?.to_string());
                    if rule.kind.is_empty() {
                        rule.kind = "decimal".to_string();
                    }
                }
                rule.formula1 = arg(0);
                rule.formula2 = arg(1);
            }
        }
    }
    Some(rule)
}

/// Quote-aware split at a character.
pub(super) fn split_char(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quote = false;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '\'' {
            quote = !quote;
        } else if c == sep && !quote {
            out.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&text[start..]);
    out
}

/// `$'My Sheet'.$A$1` as (`My Sheet`, `$A$1`). A part without a sheet gives an empty name.
pub(super) fn sheet_and_cell(part: &str) -> Option<(String, &str)> {
    let dot = {
        let mut quote = false;
        let mut found = None;
        for (i, c) in part.char_indices() {
            if c == '\'' {
                quote = !quote;
            } else if c == '.' && !quote {
                found = Some(i);
            }
        }
        found?
    };
    let sheet = part[..dot].trim_start_matches('$');
    let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => sheet.to_string(),
    };
    Some((sheet, &part[dot + 1..]))
}

/// A cell range address of a named range as the formula text a sheet reference would have:
/// `$Sheet1.$A$1:.$B$2` becomes `Sheet1!$A$1:$B$2`. Several ranges are joined with commas. What
/// is not an address is returned as written.
pub(super) fn range_text(address: &str) -> String {
    let mut parts = Vec::new();
    for range in split_char(address.trim(), ' ') {
        if range.is_empty() {
            continue;
        }
        let ends = split_char(range, ':');
        let first = sheet_and_cell(ends[0]);
        let Some((sheet, cell)) = first else {
            return address.to_string();
        };
        let mut text = format!("{}!{cell}", quote_sheet(&sheet));
        if ends.len() == 2 {
            let Some((sheet2, cell2)) = sheet_and_cell(ends[1]) else {
                return address.to_string();
            };
            if sheet2.is_empty() || sheet2 == sheet {
                text.push(':');
            } else {
                text.push_str(&format!(":{}!", quote_sheet(&sheet2)));
            }
            text.push_str(cell2);
        } else if ends.len() > 2 {
            return address.to_string();
        }
        parts.push(text);
    }
    if parts.is_empty() {
        address.to_string()
    } else {
        parts.join(",")
    }
}
