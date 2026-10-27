// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Form scripts read as text, never run: the fields a script reads, and the Acrobat form
//! calls (`AFNumber_Format` and the like) in words.
//!
//! ```
//! use pdfparts::script::{describe, field_refs};
//! let refs = field_refs(r#"event.value = this.getField("Source").value;"#);
//! assert_eq!(refs.names, ["Source"]);
//! assert!(!refs.dynamic);
//! assert_eq!(
//!     describe(r#"AFNumber_Format(2, 0, 0, 0, "$", true);"#).as_deref(),
//!     Some(r#"number, 2 decimals, "$" before the number"#)
//! );
//! ```

use std::collections::HashSet;

/// The fields a script reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldRefs {
    /// Names given as string literals, each once, in the order they appear.
    pub names: Vec<String>,
    /// The script also builds a field name at run time (`getField(name)`, a concatenation),
    /// so the list may be incomplete.
    pub dynamic: bool,
}

/// The fields `script` reads through `getField(...)` and `AFSimple_Calculate`. The scan is one
/// pass over the text.
pub fn field_refs(script: &str) -> FieldRefs {
    let mut refs = FieldRefs::default();
    let mut seen = HashSet::new();
    let mut add = |name: String, refs: &mut FieldRefs| {
        if seen.insert(name.clone()) {
            refs.names.push(name);
        }
    };
    // Each call is parsed from where the one before ended, so overlapping or unclosed calls
    // do not make the scan quadratic.
    let mut cursor = 0;
    for (at, word) in calls(script) {
        if at < cursor {
            continue;
        }
        let mut p = Parser {
            s: script,
            i: at + word.len(),
            depth: 0,
        };
        p.ws();
        if !p.eat('(') {
            cursor = p.i;
            continue;
        }
        if word == "getField" {
            p.ws();
            match p.string() {
                Some(name) => {
                    p.ws();
                    if p.eat(')') {
                        add(name, &mut refs);
                    } else {
                        refs.dynamic = true;
                    }
                }
                None => refs.dynamic = true,
            }
        } else {
            match p.args() {
                Some(args) => match args.get(1).map(field_list) {
                    Some(Some(names)) => names.into_iter().for_each(|n| add(n, &mut refs)),
                    _ => refs.dynamic = true,
                },
                None => refs.dynamic = true,
            }
        }
        cursor = p.i;
    }
    refs
}

/// Where `getField` and `AFSimple_Calculate` are named as functions, in text order. Comments
/// and string literals are skipped; an unclosed one ends the scan.
fn calls(script: &str) -> impl Iterator<Item = (usize, &'static str)> + '_ {
    let bytes = script.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < bytes.len() {
            let at = i;
            // One step for the byte here; a comment or string skipped counts its bytes.
            crate::work::add(1);
            match bytes[at] {
                b'/' if bytes.get(at + 1) == Some(&b'/') => {
                    i = bytes[at..]
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(bytes.len(), |k| at + k + 1);
                    crate::work::add((i - at) as u64);
                    continue;
                }
                b'/' if bytes.get(at + 1) == Some(&b'*') => {
                    i = bytes[at + 2..]
                        .windows(2)
                        .position(|w| w == b"*/")
                        .map_or(bytes.len(), |k| at + 2 + k + 2);
                    crate::work::add((i - at) as u64);
                    continue;
                }
                quote @ (b'"' | b'\'' | b'`') => {
                    i = string_end(bytes, at + 1, quote);
                    crate::work::add((i - at) as u64);
                    continue;
                }
                _ => {}
            }
            i += 1;
            for word in ["getField", "AFSimple_Calculate"] {
                if bytes[at..].starts_with(word.as_bytes())
                    && (at == 0 || !ident_byte(bytes[at - 1]))
                    && !bytes.get(at + word.len()).is_some_and(|b| ident_byte(*b))
                {
                    i = at + word.len();
                    return Some((at, word));
                }
            }
        }
        None
    })
}

/// Where a string literal that starts at `from` ends: after its closing `quote`, or at the
/// end of the text.
fn string_end(bytes: &[u8], from: usize, quote: u8) -> usize {
    let mut k = from;
    while k < bytes.len() {
        match bytes[k] {
            b'\\' => k += 2,
            b if b == quote => return k + 1,
            _ => k += 1,
        }
    }
    bytes.len()
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The names of an `AFSimple_Calculate` field list: an array of strings, or one string
/// with names separated by commas. `None` when an entry is not a literal.
fn field_list(arg: &Arg) -> Option<Vec<String>> {
    match arg {
        Arg::List(items) => items
            .iter()
            .map(|a| match a {
                Arg::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Arg::Str(s) => Some(
            s.split(',')
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect(),
        ),
        _ => None,
    }
}

/// The Acrobat date formats of `AFDate_Format(n)`.
const DATE_FORMATS: [&str; 14] = [
    "m/d",
    "m/d/yy",
    "mm/dd/yy",
    "mm/yy",
    "d-mmm",
    "d-mmm-yy",
    "dd-mmm-yy",
    "yy-mm-dd",
    "mmm-yy",
    "mmmm-yy",
    "mmm d, yyyy",
    "mmmm d, yyyy",
    "m/d/yy h:MM tt",
    "m/d/yy HH:MM",
];

/// The Acrobat time formats of `AFTime_Format(n)`.
const TIME_FORMATS: [&str; 4] = ["HH:MM", "h:MM tt", "HH:MM:ss", "h:MM:ss tt"];

/// `script` in words when it is one Acrobat form call with the arguments that call takes:
/// a number, percent, date, time or special format, a simple calculation or a range check.
/// Anything else, or odd or missing arguments, gives `None`; the caller then shows the text.
pub fn describe(script: &str) -> Option<String> {
    let body = script.trim();
    let body = body.strip_suffix(';').unwrap_or(body).trim_end();
    let mut p = Parser {
        s: body,
        i: 0,
        depth: 0,
    };
    let name = p.ident()?;
    p.ws();
    if !p.eat('(') {
        return None;
    }
    let args = p.args()?;
    if p.i != body.len() {
        return None;
    }
    use Arg::{Bool, List, Str};
    match (name, args.as_slice()) {
        ("AFNumber_Format", [dec, sep, neg, curr_style, Str(curr), Bool(before)]) => {
            curr_style.int()?;
            let mut parts = vec!["number".to_string(), decimals(dec.int()?)?];
            parts.extend(separator(sep.int()?)?);
            parts.extend(
                match neg.int()? {
                    0 => None,
                    1 => Some("negative in red"),
                    2 => Some("negative in parentheses"),
                    3 => Some("negative in red parentheses"),
                    _ => return None,
                }
                .map(str::to_string),
            );
            if !curr.is_empty() {
                let side = if *before { "before" } else { "after" };
                parts.push(format!("\"{curr}\" {side} the number"));
            }
            Some(parts.join(", "))
        }
        ("AFPercent_Format", [dec, sep, rest @ ..]) => {
            let before = match rest {
                [] => false,
                [Bool(b)] => *b,
                _ => return None,
            };
            let mut parts = vec!["percent".to_string(), decimals(dec.int()?)?];
            parts.extend(separator(sep.int()?)?);
            if before {
                parts.push("\"%\" before the number".into());
            }
            Some(parts.join(", "))
        }
        ("AFDate_FormatEx" | "AFDate_KeystrokeEx", [Str(f)]) if !f.is_empty() => {
            Some(format!("date {f}"))
        }
        ("AFDate_Format", [n]) => Some(format!("date {}", DATE_FORMATS.get(n.index()?)?)),
        ("AFTime_Format", [n]) => Some(format!("time {}", TIME_FORMATS.get(n.index()?)?)),
        ("AFSpecial_Format", [n]) => Some(
            match n.int()? {
                0 => "ZIP code",
                1 => "ZIP+4 code",
                2 => "phone number",
                3 => "social security number",
                _ => return None,
            }
            .to_string(),
        ),
        ("AFSpecial_KeystrokeEx", [Str(mask)]) if !mask.is_empty() => Some(format!("mask {mask}")),
        ("AFSimple_Calculate", [Str(f), fields @ (List(_) | Str(_))]) => {
            let word = match f.as_str() {
                "SUM" => "sum",
                "PRD" => "product",
                "AVG" => "average",
                "MIN" => "minimum",
                "MAX" => "maximum",
                _ => return None,
            };
            let mut seen = HashSet::new();
            let names: Vec<String> = field_list(fields)?
                .into_iter()
                .filter(|n| seen.insert(n.clone()))
                .collect();
            if names.is_empty() {
                return None;
            }
            Some(format!("{word} of {}", names.join(", ")))
        }
        ("AFRange_Validate", [Bool(low), lo, Bool(high), hi]) => {
            let (lo, hi) = (lo.number()?, hi.number()?);
            match (low, high) {
                (true, true) => Some(format!("between {lo} and {hi}")),
                (true, false) => Some(format!("at least {lo}")),
                (false, true) => Some(format!("at most {hi}")),
                (false, false) => None,
            }
        }
        _ => None,
    }
}

/// Most characters of a number argument; a longer one is an odd argument.
const NUMBER_MAX: usize = 30;

/// Most decimals a format may ask for; more is an odd argument.
const DECIMALS_MAX: i64 = 15;

/// The decimals of a format in words, `None` past [`DECIMALS_MAX`].
fn decimals(n: i64) -> Option<String> {
    if n > DECIMALS_MAX {
        return None;
    }
    Some(decimals_words(n))
}

fn decimals_words(n: i64) -> String {
    if n == 1 {
        "1 decimal".into()
    } else {
        format!("{n} decimals")
    }
}

/// The words for a separator style; `Some(None)` for the default `1,234.56`.
fn separator(style: i64) -> Option<Option<String>> {
    Some(
        match style {
            0 => None,
            1 => Some("no thousands separator"),
            2 => Some("separator 1.234,56"),
            3 => Some("separator 1234,56"),
            _ => return None,
        }
        .map(str::to_string),
    )
}

/// One argument of a call, as far as a literal can be read.
#[derive(Debug, Clone, PartialEq)]
enum Arg {
    /// A number as written.
    Num(String),
    Str(String),
    Bool(bool),
    /// `new Array(...)` or `[...]`.
    List(Vec<Arg>),
}

impl Arg {
    /// A whole number, at least 0.
    fn int(&self) -> Option<i64> {
        match self {
            Arg::Num(n) => n.parse().ok().filter(|v| *v >= 0),
            _ => None,
        }
    }

    fn index(&self) -> Option<usize> {
        usize::try_from(self.int()?).ok()
    }

    /// Any number, as written.
    fn number(&self) -> Option<&str> {
        match self {
            Arg::Num(n) => Some(n),
            _ => None,
        }
    }
}

/// A small reader for JavaScript literals.
struct Parser<'a> {
    s: &'a str,
    i: usize,
    /// Lists open around the position, bounded by [`NESTING_MAX`].
    depth: usize,
}

/// Deepest nesting of lists in one argument. Form calls nest one level.
const NESTING_MAX: usize = 8;

impl<'a> Parser<'a> {
    fn rest(&self) -> &str {
        &self.s[self.i..]
    }

    fn peek(&self) -> Option<char> {
        crate::work::add(1);
        self.rest().chars().next()
    }

    fn ws(&mut self) {
        let rest = self.rest();
        self.i += rest.len() - rest.trim_start().len();
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        let rest = self.rest().as_bytes();
        if rest.starts_with(w.as_bytes()) && !rest.get(w.len()).is_some_and(|b| ident_byte(*b)) {
            self.i += w.len();
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Option<&'a str> {
        let start = self.i;
        let n = self.rest().bytes().take_while(|b| ident_byte(*b)).count();
        if n == 0 || self.rest().as_bytes()[0].is_ascii_digit() {
            return None;
        }
        self.i += n;
        let s: &'a str = self.s;
        Some(&s[start..self.i])
    }

    /// A string literal in single or double quotes, escapes decoded. `None` when there is
    /// none here or it does not end; the position then stays after what was read.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek().filter(|c| *c == '"' || *c == '\'')?;
        self.i += 1;
        let mut out = String::new();
        let mut chars = self.rest().char_indices();
        let start = self.i;
        while let Some((k, c)) = chars.next() {
            crate::work::add(1);
            if c == quote {
                self.i = start + k + 1;
                return Some(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let Some((_, e)) = chars.next() else {
                break;
            };
            match e {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'v' => out.push('\u{b}'),
                '0' => out.push('\0'),
                // A line continuation.
                '\n' => {}
                'u' | 'x' => {
                    let len = if e == 'u' { 4 } else { 2 };
                    let hex: String = chars.clone().take(len).map(|(_, c)| c).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(ch) if hex.len() == len => {
                            out.push(ch);
                            for _ in 0..len {
                                chars.next();
                            }
                        }
                        _ => out.push(e),
                    }
                }
                other => out.push(other),
            }
        }
        self.i = self.s.len();
        None
    }

    /// The arguments after `(` up to and including `)`. `None` when one is not a literal.
    fn args(&mut self) -> Option<Vec<Arg>> {
        self.list(')')
    }

    fn list(&mut self, close: char) -> Option<Vec<Arg>> {
        if self.depth >= NESTING_MAX {
            return None;
        }
        self.depth += 1;
        let out = self.items(close);
        self.depth -= 1;
        out
    }

    fn items(&mut self, close: char) -> Option<Vec<Arg>> {
        let mut out = Vec::new();
        self.ws();
        if self.eat(close) {
            return Some(out);
        }
        loop {
            self.ws();
            out.push(self.arg()?);
            self.ws();
            if self.eat(close) {
                return Some(out);
            }
            if !self.eat(',') {
                return None;
            }
        }
    }

    fn arg(&mut self) -> Option<Arg> {
        match self.peek()? {
            '"' | '\'' => self.string().map(Arg::Str),
            '[' => {
                self.i += 1;
                self.list(']').map(Arg::List)
            }
            c if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() => {
                let start = self.i;
                let mut n = 0;
                for (k, ch) in self.rest().char_indices() {
                    let sign = k == 0 && (ch == '-' || ch == '+');
                    if !(sign || ch.is_ascii_digit() || ch == '.') {
                        break;
                    }
                    n = k + 1;
                }
                self.i += n;
                let text = &self.s[start..self.i];
                let digits = text.trim_start_matches(['-', '+']);
                let valid = digits.bytes().any(|b| b.is_ascii_digit())
                    && digits.matches('.').count() <= 1
                    && text.len() <= NUMBER_MAX;
                valid.then(|| Arg::Num(text.trim_start_matches('+').to_string()))
            }
            _ => {
                if self.eat_word("true") {
                    Some(Arg::Bool(true))
                } else if self.eat_word("false") {
                    Some(Arg::Bool(false))
                } else if self.eat_word("new") {
                    self.ws();
                    if !self.eat_word("Array") {
                        return None;
                    }
                    self.ws();
                    if !self.eat('(') {
                        return None;
                    }
                    self.list(')').map(Arg::List)
                } else {
                    None
                }
            }
        }
    }
}
