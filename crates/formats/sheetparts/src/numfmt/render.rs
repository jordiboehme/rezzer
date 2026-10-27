// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Choosing the section of a format code and writing a number with it.

use super::date::render_date;
use super::{Class, Cmp, DateSystem, Formatted, NumKind, Section, Tok, is_currency_symbol, plain};

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
pub(super) fn choose(sections: &[Section], v: f64) -> (usize, bool) {
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
pub(super) fn general(v: f64) -> String {
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

pub(super) fn render(
    section: &Section,
    value: f64,
    handled_sign: bool,
    system: DateSystem,
) -> Formatted {
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
