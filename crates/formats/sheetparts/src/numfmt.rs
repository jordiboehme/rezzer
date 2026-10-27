// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Excel number format codes: the built-in table and a formatter.
//!
//! [`format`] turns a number and a format code into the text a person sees, and says what
//! kind of value it is (percent, currency, date, ...) and which unit the format adds
//! (a currency symbol, or literal text such as the `m` in `0.0" m"`). Locale-dependent
//! built-in ids follow en-US. Rounding is half away from zero on the 15 significant digits
//! Excel keeps, so `2.675` with `0.00` gives `2.68` as in Excel.
//!
//! ```
//! use sheetparts::numfmt::{format, NumKind};
//! let f = format("0.0\" m\"", 12.34);
//! assert_eq!(f.text, "12.3 m");
//! assert_eq!(f.unit.as_deref(), Some("m"));
//! assert_eq!(f.kind, NumKind::Number);
//! ```

mod date;
mod render;

use render::{choose, general, render};

/// What a formatted value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumKind {
    General,
    Number,
    Percent,
    Currency,
    Date,
    Time,
    Scientific,
    Fraction,
    Text,
}

/// The result of [`format`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    /// The text as a person would read it in the cell.
    pub text: String,
    pub kind: NumKind,
    /// A currency symbol, or literal unit text such as `m` in `0.0" m"`.
    pub unit: Option<String>,
}

/// The day zero of serial dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSystem {
    /// Serial 1 is 1900-01-01, with Excel's non-existent 1900-02-29.
    Date1900,
    /// Serial 0 is 1904-01-01.
    Date1904,
}

/// The format code of a built-in number format id, or `None` for ids that are not built in
/// or depend on a locale this table does not cover.
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"$\"#,##0_);(\"$\"#,##0)",
        6 => "\"$\"#,##0_);[Red](\"$\"#,##0)",
        7 => "\"$\"#,##0.00_);(\"$\"#,##0.00)",
        8 => "\"$\"#,##0.00_);[Red](\"$\"#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)",
        42 => "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)",
        44 => "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// The kind of value a format code makes of the number 1.
pub fn kind_of(code: &str) -> NumKind {
    format(code, 1.0).kind
}

/// Formats `value` with `code` using the 1900 date system.
pub fn format(code: &str, value: f64) -> Formatted {
    format_with(code, value, DateSystem::Date1900)
}

/// Formats `value` with `code` using the given date system.
pub fn format_with(code: &str, value: f64, system: DateSystem) -> Formatted {
    if !value.is_finite() {
        return plain("#NUM!", NumKind::General);
    }
    // Excel allows 255 characters in a code; anything longer is not a format.
    if code.chars().count() > 255 {
        return plain(&general(value), NumKind::General);
    }
    let sections: Vec<Section> = split_sections(code)
        .into_iter()
        .map(parse_section)
        .collect();
    if sections.is_empty()
        || (sections.len() == 1 && sections[0].toks.is_empty() && code.trim().is_empty())
    {
        return plain(&general(value), NumKind::General);
    }
    let (index, handled_sign) = choose(&sections, value);
    let section = &sections[index];
    render(section, value, handled_sign, system)
}

fn plain(text: &str, kind: NumKind) -> Formatted {
    Formatted {
        text: text.to_string(),
        kind,
        unit: None,
    }
}

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Literal text; the flag says it is a quoted or escaped text or a currency symbol, which
    /// can name a unit.
    Lit(String, bool),
    /// Currency text from `[$...]`.
    Cur(String),
    Digit(char),
    Point,
    Comma,
    Percent,
    Exp(bool),
    Slash,
    At,
    General,
    Date(char, usize),
    AmPm {
        short: bool,
        lower: bool,
    },
    Elapsed(char, usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Default)]
struct Section {
    cond: Option<(Cmp, f64)>,
    toks: Vec<Tok>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Class {
    General,
    Date,
    Text,
    Fraction,
    Scientific,
    Number,
}

impl Section {
    fn class(&self) -> Class {
        let has = |f: &dyn Fn(&Tok) -> bool| self.toks.iter().any(f);
        let digits = has(&|t| matches!(t, Tok::Digit(_)));
        if has(&|t| matches!(t, Tok::Date(..) | Tok::AmPm { .. } | Tok::Elapsed(..))) {
            Class::Date
        } else if has(&|t| matches!(t, Tok::General)) {
            Class::General
        } else if has(&|t| matches!(t, Tok::At)) && !digits {
            Class::Text
        } else if digits && has(&|t| matches!(t, Tok::Slash)) {
            Class::Fraction
        } else if digits && has(&|t| matches!(t, Tok::Exp(_))) {
            Class::Scientific
        } else {
            Class::Number
        }
    }
}

/// Splits a code at the semicolons that are not quoted, escaped or inside brackets.
fn split_sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quote = false;
    let mut bracket = false;
    let mut escape = false;
    for (i, c) in code.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' | '_' | '*' if !quote && !bracket => escape = true,
            '"' if !bracket => quote = !quote,
            '[' if !quote => bracket = true,
            ']' if !quote => bracket = false,
            ';' if !quote && !bracket => {
                out.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&code[start..]);
    out
}

fn is_currency_symbol(c: char) -> bool {
    matches!(
        c,
        '$' | '\u{20ac}'
            | '\u{a3}'
            | '\u{a5}'
            | '\u{a2}'
            | '\u{20b9}'
            | '\u{20bd}'
            | '\u{20a9}'
            | '\u{20aa}'
            | '\u{20ba}'
            | '\u{e3f}'
            | '\u{20ab}'
            | '\u{20b4}'
            | '\u{20a6}'
            | '\u{20a3}'
            | '\u{20b1}'
    )
}

fn starts_with_ci(chars: &[char], at: usize, word: &str) -> bool {
    let w: Vec<char> = word.chars().collect();
    at + w.len() <= chars.len()
        && chars[at..at + w.len()]
            .iter()
            .zip(&w)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn parse_cond(text: &str) -> Option<(Cmp, f64)> {
    let (op, rest) = if let Some(r) = text.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = text.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = text.strip_prefix("<>") {
        (Cmp::Ne, r)
    } else if let Some(r) = text.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = text.strip_prefix('>') {
        (Cmp::Gt, r)
    } else {
        (Cmp::Eq, text.strip_prefix('=')?)
    };
    rest.trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| (op, n))
}

fn parse_section(src: &str) -> Section {
    let chars: Vec<char> = src.chars().collect();
    let mut section = Section::default();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let mut j = i + 1;
                let mut text = String::new();
                while j < chars.len() && chars[j] != '"' {
                    text.push(chars[j]);
                    j += 1;
                }
                section.toks.push(Tok::Lit(text, true));
                i = j + 1;
                continue;
            }
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    section.toks.push(Tok::Lit(next.to_string(), true));
                }
                i += 2;
                continue;
            }
            '_' => {
                if i + 1 < chars.len() {
                    section.toks.push(Tok::Lit(" ".into(), false));
                }
                i += 2;
                continue;
            }
            '*' => {
                i += 2;
                continue;
            }
            '[' => {
                let Some(close) = chars[i + 1..].iter().position(|&c| c == ']') else {
                    break;
                };
                let inner: String = chars[i + 1..i + 1 + close].iter().collect();
                i += close + 2;
                if let Some(rest) = inner.strip_prefix('$') {
                    let symbol = rest.split('-').next().unwrap_or("");
                    if !symbol.is_empty() {
                        section.toks.push(Tok::Cur(symbol.to_string()));
                    }
                } else if let Some(cond) = parse_cond(&inner) {
                    section.cond = Some(cond);
                } else {
                    let lower = inner.to_ascii_lowercase();
                    let first = lower.chars().next();
                    if let Some(f @ ('h' | 'm' | 's')) = first
                        && lower.chars().all(|c| c == f)
                    {
                        section.toks.push(Tok::Elapsed(f, lower.len()));
                    }
                    // Colors, locale and other brackets do not change the text.
                }
                continue;
            }
            '0' | '#' | '?' => section.toks.push(Tok::Digit(c)),
            '.' => section.toks.push(Tok::Point),
            ',' => section.toks.push(Tok::Comma),
            '%' => section.toks.push(Tok::Percent),
            '/' => section.toks.push(Tok::Slash),
            '@' => section.toks.push(Tok::At),
            'e' | 'E' if matches!(chars.get(i + 1), Some('+' | '-')) => {
                section.toks.push(Tok::Exp(chars[i + 1] == '+'));
                i += 2;
                continue;
            }
            'g' | 'G' if starts_with_ci(&chars, i, "general") => {
                section.toks.push(Tok::General);
                i += 7;
                continue;
            }
            'a' | 'A' if starts_with_ci(&chars, i, "am/pm") => {
                section.toks.push(Tok::AmPm {
                    short: false,
                    lower: c == 'a',
                });
                i += 5;
                continue;
            }
            'a' | 'A' if starts_with_ci(&chars, i, "a/p") => {
                section.toks.push(Tok::AmPm {
                    short: true,
                    lower: c == 'a',
                });
                i += 3;
                continue;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'e' | 'E' => {
                let lower = c.to_ascii_lowercase();
                let mut n = 1;
                while chars
                    .get(i + n)
                    .is_some_and(|x| x.eq_ignore_ascii_case(&lower))
                {
                    n += 1;
                }
                section.toks.push(Tok::Date(
                    if lower == 'e' { 'y' } else { lower },
                    if lower == 'e' { 4 } else { n },
                ));
                i += n;
                continue;
            }
            'b' | 'B' | 'g' | 'G' => {}
            other => section
                .toks
                .push(Tok::Lit(other.to_string(), is_currency_symbol(other))),
        }
        i += 1;
    }
    section
}
