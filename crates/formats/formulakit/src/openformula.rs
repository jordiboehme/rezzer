// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OpenFormula (ODF) text to Excel A1 text.
//!
//! ODF writes references in brackets (`[.B4]`, `[$Data.A3]`, `[$Sheet1.A1:$Sheet3.B2]`,
//! `['file:///x.ods'#$Sheet1.A1]`), separates arguments with `;`, writes a union as `~`, an
//! intersection as `!` and array rows with `|`. This module rewrites all of that into the Excel
//! form, so one Excel parse serves both dialects.

use crate::ParseError;
use crate::refs::{SheetSpan, quote_sheet, sheet_prefix};

/// One open bracket level: where its current argument starts in the output.
struct Frame {
    start: usize,
    /// Plain grouping brackets, not a call: a union alone inside them needs no extra brackets.
    bare: bool,
    union: bool,
    args: usize,
}

fn error(message: impl Into<String>) -> ParseError {
    ParseError {
        message: message.into(),
    }
}

/// An Excel union inside an argument list needs its own brackets: `SUM((A1,B1),C1)`.
fn close_arg(frame: &mut Frame, out: &mut String, closing_group: bool) {
    if frame.union && !(frame.bare && closing_group && frame.args == 1) {
        out.insert(frame.start, '(');
        out.push(')');
    }
    frame.union = false;
}

/// Rewrites an OpenFormula formula (`=...`, namespace prefix already removed) into Excel text.
pub(crate) fn to_excel(text: &str) -> Result<String, ParseError> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    if let Some((_, '=')) = chars.peek() {
        out.push('=');
        chars.next();
    }
    let mut frames = vec![Frame {
        start: out.len(),
        bare: false,
        union: false,
        args: 1,
    }];
    let mut array_depth = 0usize;
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some((_, s)) = chars.next() {
                    out.push(s);
                    if s == '"' {
                        if let Some((_, '"')) = chars.peek() {
                            out.push('"');
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
            }
            '#' => {
                // An error literal such as #DIV/0!, #NAME? or #N/A.
                out.push('#');
                while let Some(&(_, s)) = chars.peek() {
                    if s.is_ascii_alphanumeric() || s == '/' || s == '_' {
                        out.push(s);
                        chars.next();
                    } else {
                        if s == '!' || s == '?' {
                            out.push(s);
                            chars.next();
                        }
                        break;
                    }
                }
            }
            '[' => {
                let body_start = i + 1;
                let mut in_quote = false;
                let mut end = None;
                for (j, s) in chars.by_ref() {
                    match s {
                        '\'' => in_quote = !in_quote,
                        ']' if !in_quote => {
                            end = Some(j);
                            break;
                        }
                        _ => {}
                    }
                }
                let end = end.ok_or_else(|| error("OpenFormula reference without closing ]"))?;
                let body = text
                    .get(body_start..end)
                    .ok_or_else(|| error("OpenFormula reference is not valid text"))?;
                out.push_str(&reference(body)?);
            }
            '{' => {
                array_depth += 1;
                out.push('{');
            }
            '}' => {
                array_depth = array_depth.saturating_sub(1);
                out.push('}');
            }
            '|' if array_depth > 0 => out.push(';'),
            ';' if array_depth > 0 => out.push(','),
            ';' => {
                if let Some(frame) = frames.last_mut() {
                    close_arg(frame, &mut out, false);
                    out.push(',');
                    frame.start = out.len();
                    frame.args += 1;
                }
            }
            '~' => {
                if let Some(frame) = frames.last_mut() {
                    frame.union = true;
                }
                out.push(',');
            }
            '!' if array_depth == 0 => out.push(' '),
            '(' => {
                let call = out
                    .chars()
                    .next_back()
                    .is_some_and(|p| p.is_alphanumeric() || p == '_' || p == '.');
                out.push('(');
                frames.push(Frame {
                    start: out.len(),
                    bare: !call,
                    union: false,
                    args: 1,
                });
            }
            ')' => {
                if frames.len() > 1
                    && let Some(mut frame) = frames.pop()
                {
                    close_arg(&mut frame, &mut out, true);
                }
                out.push(')');
            }
            other => out.push(other),
        }
    }
    if let Some(frame) = frames.first_mut() {
        close_arg(frame, &mut out, false);
    }
    Ok(out)
}

/// Index of the first `ch` outside single quotes.
fn find_unquoted(text: &str, ch: char) -> Option<usize> {
    let mut in_quote = false;
    for (i, c) in text.char_indices() {
        if c == '\'' {
            in_quote = !in_quote;
        } else if c == ch && !in_quote {
            return Some(i);
        }
    }
    None
}

fn unquote(text: &str) -> Result<String, ParseError> {
    match text.strip_prefix('\'') {
        Some(rest) => rest
            .strip_suffix('\'')
            .map(|inner| inner.replace("''", "'"))
            .ok_or_else(|| error(format!("unclosed quote in OpenFormula reference {text}"))),
        None => Ok(text.to_string()),
    }
}

/// `$?[A-Z]{1,3}$?[0-9]+`, `$?[A-Z]{1,3}` (whole column) or `$?[0-9]+` (whole row).
fn coordinate(text: &str) -> Result<String, ParseError> {
    let bad = || error(format!("unsupported OpenFormula coordinate {text:?}"));
    let rest = text.strip_prefix('$').unwrap_or(text);
    let letters = rest.chars().take_while(char::is_ascii_alphabetic).count();
    let rest = rest.get(letters..).ok_or_else(bad)?;
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    let ok = letters <= 3
        && rest.len() <= 7
        && rest.chars().all(|c| c.is_ascii_digit())
        && (letters > 0 || !rest.is_empty())
        && !text.ends_with('$');
    if ok {
        Ok(text.to_ascii_uppercase())
    } else {
        Err(bad())
    }
}

/// One side of a reference: an optional sheet and a coordinate (`$Data.A3`, `.B4`).
fn part(text: &str) -> Result<(Option<String>, String), ParseError> {
    let text = text.trim();
    if let Some(coord) = text.strip_prefix('.') {
        return Ok((None, coordinate(coord)?));
    }
    let sheet_part = text.strip_prefix('$').unwrap_or(text);
    // The coordinate holds no dot, so the sheet ends at the last dot outside quotes.
    let mut dot = None;
    let mut in_quote = false;
    for (i, c) in sheet_part.char_indices() {
        match c {
            '\'' => in_quote = !in_quote,
            '.' if !in_quote => dot = Some(i),
            _ => {}
        }
    }
    match dot {
        Some(i) => {
            let sheet = unquote(sheet_part.get(..i).unwrap_or(""))?;
            if sheet.is_empty() {
                return Err(error(format!(
                    "empty sheet in OpenFormula reference {text}"
                )));
            }
            Ok((
                Some(sheet),
                coordinate(sheet_part.get(i + 1..).unwrap_or(""))?,
            ))
        }
        None => Ok((None, coordinate(text)?)),
    }
}

/// The Excel text of one bracketed OpenFormula reference (the text between `[` and `]`).
fn reference(body: &str) -> Result<String, ParseError> {
    if body.contains("#REF!") {
        return Ok("#REF!".to_string());
    }
    let (book, address) = match find_unquoted(body, '#') {
        Some(i) => (
            Some(unquote(body.get(..i).unwrap_or(""))?),
            body.get(i + 1..).unwrap_or(""),
        ),
        None => (None, body),
    };
    let (first, second) = match find_unquoted(address, ':') {
        Some(i) => (
            address.get(..i).unwrap_or(""),
            Some(address.get(i + 1..).unwrap_or("")),
        ),
        None => (address, None),
    };
    let (sheet1, coord1) = part(first)?;
    let (sheet2, coord2) = match second {
        Some(s) => {
            let (sheet, coord) = part(s)?;
            (sheet, Some(coord))
        }
        None => (None, None),
    };
    let coords = match &coord2 {
        Some(c2) if *c2 != coord1 || sheet2.is_none() => format!("{coord1}:{c2}"),
        Some(_) => coord1.clone(),
        None => coord1.clone(),
    };
    match (book, sheet1, sheet2) {
        (Some(book), Some(s1), s2) if s2.as_ref().is_none_or(|s| *s == s1) => {
            let prefix = format!("[{book}]{s1}").replace('\'', "''");
            Ok(format!("'{prefix}'!{coords}"))
        }
        (Some(_), _, _) => Err(error(format!(
            "unsupported external OpenFormula reference {body}"
        ))),
        (None, Some(s1), Some(s2)) if s1 != s2 => {
            let span = SheetSpan {
                first: s1,
                last: Some(s2),
            };
            Ok(format!("{}!{coords}", sheet_prefix(&span)))
        }
        (None, Some(s1), _) => Ok(format!("{}!{coords}", quote_sheet(&s1))),
        (None, None, None) => Ok(coords),
        (None, None, Some(_)) => Err(error(format!(
            "OpenFormula reference with a sheet only on its end: {body}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_references_and_separators() {
        let cases = [
            ("=[.B4]+[$Data.A3]", "=B4+Data!A3"),
            ("=SUM([.A1:.B2];[.C1])", "=SUM(A1:B2,C1)"),
            ("=[$'it''s'.a1]", "='it''s'!A1"),
            ("=[$Sheet1.A1:.B2]", "=Sheet1!A1:B2"),
            ("=[$Sheet1.A1:$Sheet1.B2]", "=Sheet1!A1:B2"),
            ("=[$Sheet1.A1:$Sheet3.A1]", "=Sheet1:Sheet3!A1"),
            ("=[$'S 1'.A1:$'S 3'.A1]", "='S 1:S 3'!A1"),
            ("=[.A:.C]+[.1:.3]", "=A:C+1:3"),
            ("=[.A1]~[.B1]", "=(A1,B1)"),
            ("=([.A1]~[.B1])", "=(A1,B1)"),
            ("=INDEX([.A1]~[.B1];1)", "=INDEX((A1,B1),1)"),
            ("=\"a;b~c[.A1]\"&[.A1]", "=\"a;b~c[.A1]\"&A1"),
            ("=IF([.A1]=#N/A;#DIV/0!;1)", "=IF(A1=#N/A,#DIV/0!,1)"),
            ("=['file:///a b.ods'#$S.A1]", "='[file:///a b.ods]S'!A1"),
        ];
        for (input, expected) in cases {
            assert_eq!(to_excel(input).unwrap(), expected, "{input}");
        }
        assert!(to_excel("=[.A1").is_err());
        assert!(to_excel("=[.A1x]").is_err());
        assert!(to_excel("=[.]").is_err());
    }
}
