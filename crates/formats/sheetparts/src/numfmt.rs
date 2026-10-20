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

// ---------------------------------------------------------------------------------------
// Choosing the section
// ---------------------------------------------------------------------------------------

fn cmp_matches((op, n): (Cmp, f64), v: f64) -> bool {
    match op {
        Cmp::Lt => v < n,
        Cmp::Le => v <= n,
        Cmp::Gt => v > n,
        Cmp::Ge => v >= n,
        Cmp::Eq => v == n,
        Cmp::Ne => v != n,
    }
}

/// Returns the section index and whether that section already stands for a negative number
/// (so the sign is not written again).
fn choose(sections: &[Section], v: f64) -> (usize, bool) {
    let mut count = sections.len();
    // A last section that is only text is the text section, not the negative one.
    if count >= 2 && sections[count - 1].class() == Class::Text {
        count -= 1;
    }
    if sections[..count.min(2)].iter().any(|s| s.cond.is_some()) {
        for (i, s) in sections[..count.min(2)].iter().enumerate() {
            if s.cond.is_some_and(|c| cmp_matches(c, v)) {
                return (i, false);
            }
        }
        if count >= 3 {
            return (2, false);
        }
        if count == 2 && sections[1].cond.is_none() {
            return (1, false);
        }
        return (0, false);
    }
    match count {
        1 => (0, false),
        2 => {
            if v < 0.0 {
                (1, true)
            } else {
                (0, false)
            }
        }
        _ => {
            if v > 0.0 {
                (0, false)
            } else if v < 0.0 {
                (1, true)
            } else {
                (2, false)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Digits
// ---------------------------------------------------------------------------------------

/// The 15 significant digits of `x` (x >= 0) and the count of digits before the decimal
/// point (zero or negative for numbers below 1).
fn digits_of(x: f64) -> (Vec<u8>, i32) {
    if x == 0.0 {
        return (vec![0; 15], 1);
    }
    let s = format!("{x:.14e}");
    let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exp.parse::<i32>().unwrap_or(0) + 1)
}

/// Rounds half up at `nfrac` decimals and returns the integer digits (no leading zeros)
/// and the fraction digits.
fn round_at(digits: &[u8], p: i32, nfrac: usize) -> (String, String) {
    let mut v: Vec<u8> = digits.to_vec();
    let mut p = p;
    if p < 0 {
        let mut padded = vec![0; (-p) as usize];
        padded.extend_from_slice(&v);
        v = padded;
        p = 0;
    }
    let mut p = p as usize;
    let keep = p + nfrac;
    while v.len() <= keep {
        v.push(0);
    }
    let up = v[keep] >= 5;
    v.truncate(keep);
    if up {
        let mut i = keep;
        loop {
            if i == 0 {
                v.insert(0, 1);
                p += 1;
                break;
            }
            i -= 1;
            if v[i] == 9 {
                v[i] = 0;
            } else {
                v[i] += 1;
                break;
            }
        }
    }
    let int: String = v[..p].iter().map(|d| (b'0' + d) as char).collect();
    let frac: String = v[p..].iter().map(|d| (b'0' + d) as char).collect();
    (int.trim_start_matches('0').to_string(), frac)
}

/// General format: up to 11 characters, scientific for very large and very small numbers.
fn general(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let (digits, p) = digits_of(a);
    if !(1e-4..1e11).contains(&a) {
        let (mant, exp) = scientific_parts(&digits, p, 1, 5);
        let mant = mant.trim_end_matches('0').trim_end_matches('.').to_string();
        let exp_sign = if exp < 0 { '-' } else { '+' };
        return format!("{sign}{mant}E{exp_sign}{:02}", exp.abs());
    }
    let decimals = (10 - p).max(0) as usize;
    let (int, frac) = round_at(&digits, p, decimals);
    let frac = frac.trim_end_matches('0');
    let int = if int.is_empty() { "0" } else { &int };
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

/// Mantissa text ("d.ddd") and exponent for scientific notation where the exponent is a
/// multiple of `group` (1 for plain, 3 for engineering style `##0.0E+0`).
fn scientific_parts(digits: &[u8], p: i32, group: i32, nfrac: usize) -> (String, i32) {
    let zero = digits.iter().all(|&d| d == 0);
    let mut exp = if zero {
        0
    } else {
        (p - 1).div_euclid(group) * group
    };
    loop {
        let int_len = p - exp;
        let (int, frac) = round_at(digits, int_len, nfrac);
        if (int.len() as i32) > group.max(1) && !zero {
            exp += group;
            continue;
        }
        let int = if int.is_empty() { "0".to_string() } else { int };
        let text = if nfrac == 0 {
            int
        } else {
            format!("{int}.{frac}")
        };
        return (text, exp);
    }
}

// ---------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------

fn render(section: &Section, value: f64, handled_sign: bool, system: DateSystem) -> Formatted {
    let negative = value < 0.0;
    let abs = value.abs();
    let sign = if negative && !handled_sign { "-" } else { "" };
    match section.class() {
        Class::General => {
            let g = general(abs);
            let mut text = String::new();
            for t in &section.toks {
                match t {
                    Tok::General => text.push_str(&format!("{sign}{g}")),
                    Tok::Lit(s, _) | Tok::Cur(s) => text.push_str(s),
                    _ => {}
                }
            }
            plain(&text, NumKind::General)
        }
        Class::Text => {
            let g = general(value);
            let mut text = String::new();
            for t in &section.toks {
                match t {
                    Tok::At => text.push_str(&g),
                    Tok::Lit(s, _) | Tok::Cur(s) => text.push_str(s),
                    _ => {}
                }
            }
            plain(&text, NumKind::Text)
        }
        Class::Date => render_date(section, value, system),
        Class::Fraction => render_fraction(section, abs, sign),
        Class::Scientific => render_scientific(section, abs, sign),
        Class::Number => render_number(section, abs, sign),
    }
}

fn unit_of(toks: &[Tok]) -> (Option<String>, bool) {
    let first = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let last = toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let (prefix, suffix): (&[Tok], &[Tok]) = match (first, last) {
        (Some(f), Some(l)) => (&toks[..f], &toks[l + 1..]),
        _ => (&toks[..0], &toks[..0]),
    };
    let gather = |part: &[Tok]| -> (String, bool) {
        let mut text = String::new();
        let mut cur = false;
        for t in part {
            match t {
                Tok::Lit(s, true) => text.push_str(s),
                Tok::Cur(s) => {
                    text.push_str(s);
                    cur = true;
                }
                // Plain separators between the pieces of a unit keep their place.
                Tok::Lit(s, false) if !text.is_empty() && s.trim().is_empty() => text.push_str(s),
                _ => {}
            }
        }
        (text.trim().to_string(), cur)
    };
    for part in [suffix, prefix] {
        let (text, cur) = gather(part);
        let meaningful = text
            .chars()
            .any(|c| c.is_alphabetic() || is_currency_symbol(c) || c == '\u{b0}');
        if meaningful {
            let currency = cur || text.chars().all(is_currency_symbol);
            return (Some(text), currency);
        }
    }
    (None, false)
}

fn kind_and_unit(toks: &[Tok], base: NumKind) -> (NumKind, Option<String>) {
    let (unit, currency) = unit_of(toks);
    let kind = if base != NumKind::Number {
        base
    } else if toks.iter().any(|t| matches!(t, Tok::Percent)) {
        NumKind::Percent
    } else if currency {
        NumKind::Currency
    } else {
        NumKind::Number
    };
    (kind, unit)
}

/// Writes the integer and fraction digits into the placeholders of `toks`.
fn emit_digits(toks: &[Tok], int: &str, frac: &str, thousands: bool, out: &mut String) {
    let point = toks.iter().position(|t| matches!(t, Tok::Point));
    let first_digit = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let int_slots = toks
        .iter()
        .enumerate()
        .filter(|(i, t)| matches!(t, Tok::Digit(_)) && point.is_none_or(|p| *i < p))
        .count();
    let int_chars: Vec<char> = int.chars().collect();
    let frac_chars: Vec<char> = frac.chars().collect();
    let mut slot = 0usize;
    let mut frac_i = 0usize;
    let mut seen_digit = false;
    for (i, t) in toks.iter().enumerate() {
        let in_frac = point.is_some_and(|p| i > p);
        match t {
            Tok::Digit(kind) if !in_frac => {
                // The digits that belong to this slot; the first slot also takes the overflow.
                let upto = int_chars.len() as i64 - (int_slots - slot) as i64;
                let from = if slot == 0 { 0 } else { upto };
                let mut piece: Vec<char> = Vec::new();
                if upto >= 0 {
                    piece.extend(&int_chars[from.max(0) as usize..=upto as usize]);
                } else {
                    match kind {
                        '0' => piece.push('0'),
                        '?' => piece.push(' '),
                        _ => {}
                    }
                }
                let n = piece.len();
                for (j, ch) in piece.into_iter().enumerate() {
                    let right = (int_slots - 1 - slot) + (n - 1 - j);
                    if ch.is_ascii_digit() {
                        if thousands && seen_digit && right % 3 == 2 {
                            out.push(',');
                        }
                        seen_digit = true;
                    }
                    out.push(ch);
                }
                slot += 1;
            }
            Tok::Digit(kind) => {
                let digit = frac_chars.get(frac_i).copied().unwrap_or('0');
                let trailing_zero = frac_chars
                    .get(frac_i..)
                    .is_none_or(|rest| rest.iter().all(|&c| c == '0'));
                frac_i += 1;
                match kind {
                    '0' => out.push(digit),
                    '#' if !trailing_zero => out.push(digit),
                    '?' if trailing_zero => out.push(' '),
                    '?' => out.push(digit),
                    _ => {}
                }
            }
            Tok::Point => out.push('.'),
            Tok::Comma => {
                if first_digit.is_none_or(|f| i < f) {
                    out.push(',');
                }
            }
            Tok::Percent => out.push('%'),
            Tok::Slash => out.push('/'),
            Tok::Lit(s, _) | Tok::Cur(s) => out.push_str(s),
            Tok::At | Tok::General => {}
            _ => {}
        }
    }
}

fn render_number(section: &Section, abs: f64, sign: &str) -> Formatted {
    let toks = &section.toks;
    let first = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let last = toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let (kind, unit) = kind_and_unit(toks, NumKind::Number);
    let Some((first, last)) = first.zip(last) else {
        let mut text = String::new();
        emit_digits(toks, "", "", false, &mut text);
        return Formatted { text, kind, unit };
    };
    let thousands = toks[first..=last].iter().any(|t| matches!(t, Tok::Comma));
    let scale = toks[last + 1..]
        .iter()
        .take_while(|t| matches!(t, Tok::Comma | Tok::Percent | Tok::Lit(..)))
        .filter(|t| matches!(t, Tok::Comma))
        .count();
    let percents = toks.iter().filter(|t| matches!(t, Tok::Percent)).count();
    let mut v = abs;
    for _ in 0..percents {
        v *= 100.0;
    }
    for _ in 0..scale {
        v /= 1000.0;
    }
    if !v.is_finite() {
        return plain("#NUM!", NumKind::General);
    }
    let point = toks.iter().position(|t| matches!(t, Tok::Point));
    let nfrac = toks
        .iter()
        .enumerate()
        .filter(|(i, t)| matches!(t, Tok::Digit(_)) && point.is_some_and(|p| *i > p))
        .count();
    let (digits, p) = digits_of(v);
    let (int, frac) = round_at(&digits, p, nfrac);
    let mut text = String::from(sign);
    emit_digits(toks, &int, &frac, thousands, &mut text);
    Formatted { text, kind, unit }
}

fn render_scientific(section: &Section, abs: f64, sign: &str) -> Formatted {
    let toks = &section.toks;
    let exp_at = toks
        .iter()
        .position(|t| matches!(t, Tok::Exp(_)))
        .unwrap_or(toks.len());
    let (mant_toks, rest) = toks.split_at(exp_at);
    let plus = matches!(rest.first(), Some(Tok::Exp(true)));
    let point = mant_toks.iter().position(|t| matches!(t, Tok::Point));
    let digit_at = |i: usize, t: &Tok| matches!(t, Tok::Digit(_)) && point.is_none_or(|p| i < p);
    let int_slots = mant_toks
        .iter()
        .enumerate()
        .filter(|(i, t)| digit_at(*i, t))
        .count()
        .max(1) as i32;
    let nfrac = mant_toks
        .iter()
        .enumerate()
        .filter(|(i, t)| matches!(t, Tok::Digit(_)) && point.is_some_and(|p| *i > p))
        .count();
    let (digits, p) = digits_of(abs);
    let (mant, exp) = scientific_parts(&digits, p, int_slots, nfrac);
    let (int, frac) = mant
        .split_once('.')
        .map_or((mant.as_str(), ""), |(a, b)| (a, b));
    let int = if int == "0" { "" } else { int };
    let mut text = String::from(sign);
    emit_digits(mant_toks, int, frac, false, &mut text);
    let exp_digits = rest
        .iter()
        .skip(1)
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count()
        .max(1);
    let exp_sign = if exp < 0 {
        "-"
    } else if plus {
        "+"
    } else {
        ""
    };
    text.push('E');
    text.push_str(exp_sign);
    text.push_str(&format!("{:0width$}", exp.abs(), width = exp_digits));
    for t in rest.iter().skip(1) {
        match t {
            Tok::Lit(s, _) | Tok::Cur(s) => text.push_str(s),
            Tok::Percent => text.push('%'),
            _ => {}
        }
    }
    Formatted {
        text,
        kind: NumKind::Scientific,
        unit: unit_of(toks).0,
    }
}

fn place(slots: &[char], digits: &str, left: bool) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let n = slots.len();
    let mut out: Vec<String> = vec![String::new(); n];
    if left {
        for (i, kind) in slots.iter().enumerate() {
            out[i] = match chars.get(i) {
                Some(c) => c.to_string(),
                None => match kind {
                    '0' => "0".into(),
                    '?' => " ".into(),
                    _ => String::new(),
                },
            };
        }
        if chars.len() > n
            && let Some(last) = out.last_mut()
        {
            last.push_str(&chars[n..].iter().collect::<String>());
        }
    } else {
        for (i, kind) in slots.iter().enumerate() {
            let from_right = n - 1 - i;
            out[i] = if chars.len() > from_right {
                let c = chars[chars.len() - 1 - from_right];
                if i == 0 && chars.len() > n {
                    chars[..=chars.len() - n].iter().collect()
                } else {
                    c.to_string()
                }
            } else {
                match kind {
                    '0' => "0".into(),
                    '?' => " ".into(),
                    _ => String::new(),
                }
            };
        }
    }
    out.concat()
}

fn render_fraction(section: &Section, abs: f64, sign: &str) -> Formatted {
    let toks = &section.toks;
    let slash = toks
        .iter()
        .position(|t| matches!(t, Tok::Slash))
        .unwrap_or(0);
    let mut numer_start = slash;
    while numer_start > 0 && matches!(toks[numer_start - 1], Tok::Digit(_)) {
        numer_start -= 1;
    }
    let numer_slots: Vec<char> = toks[numer_start..slash]
        .iter()
        .filter_map(|t| {
            if let Tok::Digit(c) = t {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    let mut denom_end = slash + 1;
    let mut fixed = String::new();
    let mut denom_slots: Vec<char> = Vec::new();
    while denom_end < toks.len() {
        match &toks[denom_end] {
            Tok::Digit(c) => denom_slots.push(*c),
            Tok::Lit(s, false)
                if denom_slots.is_empty() && s.len() == 1 && s.as_bytes()[0].is_ascii_digit() =>
            {
                fixed.push_str(s)
            }
            _ => break,
        }
        denom_end += 1;
    }
    let whole_slots: Vec<char> = toks[..numer_start]
        .iter()
        .filter_map(|t| {
            if let Tok::Digit(c) = t {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    let has_whole = !whole_slots.is_empty();
    let max_den: u64 = if let Ok(d) = fixed.parse::<u64>() {
        d.max(1)
    } else {
        10u64.saturating_pow(denom_slots.len().clamp(1, 6) as u32) - 1
    };
    let fixed_den = fixed.parse::<u64>().ok().filter(|d| *d > 0);
    let mut whole = if has_whole { abs.floor() } else { 0.0 };
    let frac = abs - whole;
    let (mut n, mut d) = (0.0f64, 1u64);
    let mut best = f64::MAX;
    let range: Box<dyn Iterator<Item = u64>> = match fixed_den {
        Some(f) => Box::new(std::iter::once(f)),
        None => Box::new(1..=max_den),
    };
    for den in range {
        let num = (frac * den as f64).round();
        let err = (frac - num / den as f64).abs();
        if err < best - 1e-12 {
            best = err;
            n = num;
            d = den;
        }
    }
    if has_whole && n >= d as f64 {
        whole += 1.0;
        n = 0.0;
    }
    let mut text = String::from(sign);
    let mut whole_done = false;
    for t in &toks[..numer_start] {
        match t {
            Tok::Digit(_) if !whole_done => {
                let digits = if whole == 0.0 && n == 0.0 {
                    "0".to_string()
                } else {
                    format!("{whole:.0}").trim_start_matches('0').to_string()
                };
                text.push_str(&place(&whole_slots, &digits, false));
                whole_done = true;
            }
            Tok::Digit(_) => {}
            Tok::Lit(s, _) | Tok::Cur(s) => text.push_str(s),
            _ => {}
        }
    }
    if n == 0.0 && has_whole {
        let width = numer_slots.len()
            + 1
            + if fixed.is_empty() {
                denom_slots.len()
            } else {
                fixed.len()
            };
        text.push_str(&" ".repeat(width));
    } else {
        let n_text = format!("{n:.0}");
        text.push_str(&place(&numer_slots, &n_text, false));
        text.push('/');
        if fixed.is_empty() {
            text.push_str(&place(&denom_slots, &d.to_string(), true));
        } else {
            text.push_str(&fixed);
        }
    }
    for t in &toks[denom_end..] {
        match t {
            Tok::Lit(s, _) | Tok::Cur(s) => text.push_str(s),
            Tok::Percent => text.push('%'),
            _ => {}
        }
    }
    Formatted {
        text,
        kind: NumKind::Fraction,
        unit: unit_of(toks).0,
    }
}

// ---------------------------------------------------------------------------------------
// Dates and times
// ---------------------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Year, month, day and weekday (0 = Sunday) of a serial date.
fn civil(serial: i64, system: DateSystem) -> (i64, u32, u32, usize) {
    let unix = match system {
        DateSystem::Date1904 => serial - 24107,
        DateSystem::Date1900 => {
            if serial == 0 {
                return (1900, 1, 0, 6);
            }
            if serial == 60 {
                return (1900, 2, 29, 3);
            }
            if serial < 60 {
                let (y, m, d, _) = from_unix(serial - 25568);
                return (y, m, d, ((serial + 6) % 7) as usize);
            }
            serial - 25569
        }
    };
    from_unix(unix)
}

fn from_unix(days: i64) -> (i64, u32, u32, usize) {
    let weekday = (days + 4).rem_euclid(7) as usize;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d, weekday)
}

fn render_date(section: &Section, value: f64, system: DateSystem) -> Formatted {
    let toks = &section.toks;
    if value < 0.0 {
        return plain("####", NumKind::Date);
    }
    let has_ampm = toks.iter().any(|t| matches!(t, Tok::AmPm { .. }));
    let elapsed_h = toks.iter().any(|t| matches!(t, Tok::Elapsed('h', _)));
    let elapsed_m = toks.iter().any(|t| matches!(t, Tok::Elapsed('m', _)));
    // Fractional seconds: a point and zeros right after the seconds.
    let mut frac_digits = 0usize;
    for (i, t) in toks.iter().enumerate() {
        if matches!(t, Tok::Date('s', _) | Tok::Elapsed('s', _))
            && matches!(toks.get(i + 1), Some(Tok::Point))
        {
            let n = toks[i + 2..]
                .iter()
                .take_while(|t| matches!(t, Tok::Digit('0')))
                .count();
            frac_digits = frac_digits.max(n.min(3));
        }
    }
    let unit_ms = 10i64.pow(3 - frac_digits as u32);
    let round_to_unit = |ms: f64| -> i64 {
        let ms = ms.round() as i64;
        (ms + unit_ms / 2) / unit_ms * unit_ms
    };
    let mut days = value.floor();
    if days > 2_958_465.0 {
        return plain("####", NumKind::Date);
    }
    let mut day_ms = round_to_unit((value - days) * 86_400_000.0);
    let total_ms = round_to_unit(value * 86_400_000.0);
    if day_ms >= 86_400_000 {
        day_ms -= 86_400_000;
        days += 1.0;
    }
    let (year, month, day, weekday) = civil(days as i64, system);
    if year > 9999 {
        return plain("####", NumKind::Date);
    }
    let (hours, minutes, seconds, millis) = if elapsed_h {
        (
            total_ms / 3_600_000,
            (total_ms / 60_000) % 60,
            (total_ms / 1000) % 60,
            total_ms % 1000,
        )
    } else if elapsed_m {
        (
            0,
            total_ms / 60_000,
            (total_ms / 1000) % 60,
            total_ms % 1000,
        )
    } else if toks.iter().any(|t| matches!(t, Tok::Elapsed('s', _))) {
        (0, 0, total_ms / 1000, total_ms % 1000)
    } else {
        (
            day_ms / 3_600_000,
            (day_ms / 60_000) % 60,
            (day_ms / 1000) % 60,
            day_ms % 1000,
        )
    };

    // A month token next to hours or seconds is minutes.
    let significant = |range: &mut dyn Iterator<Item = usize>| -> Option<&Tok> {
        range.map(|i| &toks[i]).find(|t| !matches!(t, Tok::Lit(..)))
    };
    let is_minute = |i: usize| -> bool {
        let before = significant(&mut (0..i).rev());
        let after = significant(&mut (i + 1..toks.len()));
        matches!(before, Some(Tok::Date('h', _) | Tok::Elapsed('h', _)))
            || matches!(after, Some(Tok::Date('s', _) | Tok::Elapsed('s', _)))
    };

    let mut text = String::new();
    let mut has_date = false;
    let mut skip_to = 0;
    for (i, t) in toks.iter().enumerate() {
        if i < skip_to {
            continue;
        }
        match t {
            Tok::Date('y', n) => {
                has_date = true;
                if *n <= 2 {
                    text.push_str(&format!("{:02}", year.rem_euclid(100)));
                } else {
                    text.push_str(&format!("{year:04}"));
                }
            }
            Tok::Date('m', n) if is_minute(i) => {
                if *n == 1 {
                    text.push_str(&minutes.to_string());
                } else {
                    text.push_str(&format!("{minutes:02}"));
                }
            }
            Tok::Date('m', n) => {
                has_date = true;
                let name = MONTHS[(month.clamp(1, 12) - 1) as usize];
                match n {
                    1 => text.push_str(&month.to_string()),
                    2 => text.push_str(&format!("{month:02}")),
                    3 => text.push_str(&name[..3]),
                    4 => text.push_str(name),
                    _ => text.push_str(&name[..1]),
                }
            }
            Tok::Date('d', n) => {
                has_date = true;
                match n {
                    1 => text.push_str(&day.to_string()),
                    2 => text.push_str(&format!("{day:02}")),
                    3 => text.push_str(&DAYS[weekday][..3]),
                    _ => text.push_str(DAYS[weekday]),
                }
            }
            Tok::Date('h', n) => {
                let h = if has_ampm {
                    match hours % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    hours
                };
                if *n == 1 {
                    text.push_str(&h.to_string());
                } else {
                    text.push_str(&format!("{h:02}"));
                }
            }
            Tok::Date('s', n) => {
                if *n == 1 {
                    text.push_str(&seconds.to_string());
                } else {
                    text.push_str(&format!("{seconds:02}"));
                }
                if frac_digits > 0 && matches!(toks.get(i + 1), Some(Tok::Point)) {
                    let ms = format!("{millis:03}");
                    text.push('.');
                    text.push_str(&ms[..frac_digits]);
                    skip_to = i + 2 + frac_digits;
                }
            }
            Tok::Elapsed(c, n) => {
                let v = match c {
                    'h' => hours,
                    'm' => minutes,
                    _ => seconds,
                };
                text.push_str(&format!("{v:0width$}", width = *n));
                if *c == 's' && frac_digits > 0 && matches!(toks.get(i + 1), Some(Tok::Point)) {
                    let ms = format!("{millis:03}");
                    text.push('.');
                    text.push_str(&ms[..frac_digits]);
                    skip_to = i + 2 + frac_digits;
                }
            }
            Tok::AmPm { short, lower } => {
                let pm = hours % 24 >= 12;
                let s = match (short, pm) {
                    (false, false) => "AM",
                    (false, true) => "PM",
                    (true, false) => "A",
                    (true, true) => "P",
                };
                text.push_str(&if *lower {
                    s.to_lowercase()
                } else {
                    s.to_string()
                });
            }
            Tok::Lit(s, _) | Tok::Cur(s) => text.push_str(s),
            Tok::Slash => text.push('/'),
            Tok::Point => text.push('.'),
            Tok::Comma => text.push(','),
            Tok::Percent => text.push('%'),
            Tok::Digit(c) => text.push(*c),
            _ => {}
        }
    }
    Formatted {
        text,
        kind: if has_date {
            NumKind::Date
        } else {
            NumKind::Time
        },
        unit: None,
    }
}
