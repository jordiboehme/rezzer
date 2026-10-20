// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Units of parameters: what counts as a unit, and where a label, a number format or a
//! `CONVERT` argument names one. A short list decides; anything not on it is no unit, so
//! Rezzer never guesses one.

/// SI prefixes, longest first so `da` wins over `d`.
const PREFIXES: &[&str] = &[
    "da", "Y", "Z", "E", "P", "T", "G", "M", "k", "h", "d", "c", "m", "µ", "μ", "u", "n", "p", "f",
    "a",
];

/// Units that take an SI prefix (`mm`, `kN`, `MPa`, `kWh`).
const PREFIXED: &[&str] = &[
    "m", "g", "s", "A", "K", "mol", "cd", "Hz", "N", "Pa", "J", "W", "C", "V", "F", "Ω", "ohm",
    "S", "Wb", "T", "H", "lm", "lx", "Bq", "Gy", "Sv", "kat", "L", "l", "t", "bar", "eV", "Wh",
    "VA", "var", "Nm", "B", "bit",
];

/// Units without a prefix: time, angle, temperature, ratios, imperial and US units.
const PLAIN: &[&str] = &[
    "min", "h", "hr", "hrs", "sec", "day", "days", "week", "weeks", "month", "months", "year",
    "years", "yr", "hour", "hours", "minutes", "seconds", "deg", "°", "°C", "°F", "°R", "rad",
    "sr", "ha", "ppm", "ppb", "%", "‰", "dB", "dBA", "rpm", "mph", "kph", "km/h", "kn", "knots",
    "in", "inch", "ft", "yd", "mi", "nmi", "lb", "lbs", "lbf", "lbm", "oz", "kip", "kips", "ksi",
    "psi", "psf", "psia", "psig", "gal", "qt", "bbl", "hp", "BTU", "Btu", "acre", "acres", "cfm",
    "gpm", "atm", "mmHg", "Torr", "cal", "kcal",
];

/// ISO 4217 currency codes in common use.
const CURRENCY_CODES: &[&str] = &[
    "USD", "EUR", "GBP", "JPY", "CHF", "CNY", "INR", "AUD", "CAD", "SEK", "NOK", "DKK", "PLN",
    "CZK", "HUF", "RUB", "BRL", "MXN", "ZAR", "KRW", "SGD", "HKD", "NZD", "TRY", "ILS", "AED",
    "SAR", "THB", "IDR", "MYR", "PHP", "VND", "NGN", "EGP", "ARS", "CLP", "COP", "UAH", "RON",
    "BGN", "ISK",
];

/// Currency signs.
const CURRENCY_SIGNS: &[&str] = &[
    "$", "€", "£", "¥", "₹", "₩", "₽", "₺", "₪", "₫", "฿", "₦", "₴", "₱", "zł", "Kč", "kr", "Fr.",
];

/// Units of Excel's `CONVERT` that are not in the lists above; metric ones take a prefix.
const CONVERT_METRIC: &[&str] = &["e", "ang", "ar", "ly", "pc", "parsec", "p", "at", "ga", "c"];
const CONVERT_PLAIN: &[&str] = &[
    "sg",
    "u",
    "ozm",
    "grain",
    "cwt",
    "shweight",
    "uk_cwt",
    "lcwt",
    "stone",
    "ton",
    "uk_ton",
    "LTON",
    "brton",
    "Nmi",
    "ell",
    "Pica",
    "Picapt",
    "pica",
    "survey_mi",
    "mn",
    "d",
    "dyn",
    "dy",
    "pond",
    "HPh",
    "hh",
    "wh",
    "flb",
    "btu",
    "HP",
    "PS",
    "w",
    "cel",
    "fah",
    "kel",
    "Rank",
    "Reau",
    "tsp",
    "tspm",
    "tbs",
    "cup",
    "pt",
    "us_pt",
    "uk_pt",
    "uk_qt",
    "uk_gal",
    "lt",
    "ang3",
    "barrel",
    "bushel",
    "ft3",
    "in3",
    "ly3",
    "m3",
    "mi3",
    "yd3",
    "Nmi3",
    "Pica3",
    "GRT",
    "regton",
    "MTON",
    "uk_acre",
    "us_acre",
    "ang2",
    "ft2",
    "in2",
    "ly2",
    "m2",
    "Morgen",
    "mi2",
    "Nmi2",
    "Pica2",
    "yd2",
    "byte",
    "admkn",
    "m/h",
    "m/hr",
    "m/s",
    "m/sec",
    "Wh",
    "ev",
];

/// Texts that read as a prefixed unit but mean something else (`pH`, `am`, `pm`).
const NOT_UNITS: &[&str] = &["pH", "am", "pm", "AM", "PM", "as", "Ms", "Pt"];

/// Bases an exponent may follow (`m3`, `ft2`, `s^-1`); `B2` is a cell, not bytes squared.
const WITH_EXPONENT: &[&str] = &[
    "m", "s", "ft", "in", "yd", "mi", "L", "l", "h", "min", "rad",
];

/// Longest unit text looked at.
const MAX_UNIT: usize = 24;

/// One unit symbol, with an optional exponent (`m`, `kN`, `m3`, `m²`, `s^-1`).
fn atom(s: &str) -> bool {
    if NOT_UNITS.contains(&s) {
        return false;
    }
    let base = strip_exponent(s);
    if base.is_empty() {
        return false;
    }
    let raised = base.len() != s.len();
    let plain = |b: &str| {
        (PLAIN.contains(&b) || PREFIXED.contains(&b)) && (!raised || WITH_EXPONENT.contains(&b))
    };
    if plain(base) {
        return true;
    }
    PREFIXES.iter().any(|p| {
        base.strip_prefix(p).is_some_and(|rest| {
            PREFIXED.contains(&rest) && (!raised || WITH_EXPONENT.contains(&rest))
        })
    })
}

/// `s` without a trailing exponent: `2`, `3`, `²`, `³`, `^2`, `^-1`, `⁻¹`.
fn strip_exponent(s: &str) -> &str {
    if let Some((base, exp)) = s.split_once('^') {
        let digits = exp.strip_prefix('-').unwrap_or(exp);
        return if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            base
        } else {
            ""
        };
    }
    let trimmed = s.trim_end_matches(['²', '³', '¹', '⁻']);
    if trimmed.len() != s.len() {
        return trimmed;
    }
    match s.char_indices().last() {
        Some((i, c)) if i > 0 && matches!(c, '2' | '3' | '4') => &s[..i],
        _ => s,
    }
}

/// Whether `s` is a currency code or sign.
fn currency(s: &str) -> bool {
    CURRENCY_CODES.contains(&s) || CURRENCY_SIGNS.contains(&s)
}

/// Whether `s` is a known unit: one unit, a currency, or units joined by `/`, `*`, `·`,
/// `⋅` or `.` (`kN/m3`, `kN·m`, `EUR/kWh`). A one-letter unit standing alone counts only
/// when `single` is true: alone it is just as often a variable name.
pub(crate) fn is_unit(s: &str, single: bool) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_UNIT || s.chars().any(char::is_whitespace) {
        return false;
    }
    if s.chars().count() == 1 && !single {
        return false;
    }
    if currency(s) || PLAIN.contains(&s) {
        return true;
    }
    let mut parts = 0;
    for part in s.split('/') {
        for factor in part.split(['*', '·', '⋅', '.']) {
            parts += 1;
            if !(atom(factor) || currency(factor)) {
                return false;
            }
        }
    }
    parts > 0
}

/// A `CONVERT` unit text: a known unit or one of Excel's own unit names.
pub(crate) fn is_convert_unit(s: &str) -> bool {
    if is_unit(s, true) || CONVERT_PLAIN.contains(&s) || CONVERT_METRIC.contains(&s) {
        return true;
    }
    PREFIXES.iter().any(|p| {
        s.strip_prefix(p)
            .is_some_and(|rest| !rest.is_empty() && CONVERT_METRIC.contains(&rest))
    })
}

/// A label without its unit, and the unit it names: `Excavated diameter (m)`, `Load [kN]`,
/// `Span in mm` or a value with its unit at the end (`Diameter 10.0 mm`). A one-letter unit
/// in parentheses that is the first letter of the word before it (`Height (h)`) is a
/// variable, not a unit; a bare trailing word (`Type A`) is never a unit.
pub(crate) fn split_label(text: &str) -> (Option<String>, Option<String>) {
    let t = text.trim();
    let clean = |s: &str| {
        let s = s.trim().trim_end_matches([',', ':', ';']).trim();
        (!s.is_empty()).then(|| s.to_string())
    };
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(inner) = t.strip_suffix(close)
            && let Some(start) = inner.rfind(open)
        {
            let unit = inner[start + open.len_utf8()..].trim();
            let before = &inner[..start];
            let single_ok = close == ']' || !initial_of_last_word(before, unit);
            if is_unit(unit, single_ok) {
                return (clean(before), Some(unit.to_string()));
            }
        }
    }
    if let Some((before, unit)) = t.rsplit_once(' ') {
        let unit = unit.trim();
        if let Some(label) = before.trim_end().strip_suffix(" in")
            && is_unit(unit, false)
        {
            return (clean(label), Some(unit.to_string()));
        }
        // A number and its unit at the end. One letter after a number in a label is
        // usually an identifier (`Phase 2 A`), not a unit.
        let before = before.trim_end();
        let (head, last) = before.rsplit_once(' ').unwrap_or(("", before));
        if is_number(last) && is_unit(unit, false) {
            return (clean(head), Some(unit.to_string()));
        }
    }
    if t.chars().count() > 1 && is_unit(t, false) {
        return (None, Some(t.to_string()));
    }
    (clean(t), None)
}

/// Whether a one-letter `unit` is the first letter of the last word of `before`.
fn initial_of_last_word(before: &str, unit: &str) -> bool {
    let mut u = unit.chars();
    let (Some(letter), None) = (u.next(), u.next()) else {
        return false;
    };
    before
        .split_whitespace()
        .last()
        .and_then(|w| w.chars().next())
        .is_some_and(|c| c.to_lowercase().eq(letter.to_lowercase()))
}

fn is_number(s: &str) -> bool {
    let s = s.replace(',', "");
    !s.is_empty() && s.parse::<f64>().is_ok_and(f64::is_finite)
}

/// The unit a number format writes next to the number: its quoted or escaped literal text
/// (`0.0" m"`, `0\ \k\N`), a currency (`[$€-407]`, `"$"#,##0`) or a currency sign Excel
/// shows without quotes. Only the first section (positive numbers) is read.
pub(crate) fn from_number_format(code: &str) -> Option<String> {
    let mut literal = String::new();
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ';' => break,
            '"' => {
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                    literal.push(d);
                }
            }
            '\\' => {
                if let Some(d) = chars.next() {
                    literal.push(d);
                }
            }
            '[' => {
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d);
                }
                // `[$€-407]` or `[$USD]`: the text between `$` and `-`. Colors,
                // conditions and `[$-409]` name no unit.
                if let Some(rest) = inner.strip_prefix('$') {
                    let sign = rest.split('-').next().unwrap_or("");
                    if !sign.is_empty() {
                        literal.push(' ');
                        literal.push_str(sign);
                        literal.push(' ');
                    }
                }
            }
            '_' | '*' => {
                // Padding: the next character only sets a width or a fill.
                chars.next();
                literal.push(' ');
            }
            c if CURRENCY_SIGNS
                .iter()
                .any(|s| s.starts_with(c) && s.len() == c.len_utf8()) =>
            {
                literal.push(' ');
                literal.push(c);
                literal.push(' ');
            }
            _ => literal.push(' '),
        }
    }
    let unit = literal.split_whitespace().collect::<Vec<_>>();
    match unit.as_slice() {
        [one] if is_unit(one, true) => Some((*one).to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_on_the_list() {
        for u in [
            "m", "mm", "kN", "MPa", "kN/m3", "kNm/m", "m/s", "m²", "s^-1", "kWh", "EUR", "€", "%",
            "°C", "psi", "kN·m", "EUR/kWh", "deg", "mol/L",
        ] {
            assert!(is_unit(u, true), "{u}");
        }
        for u in [
            "D", "gamma", "veh/hr", "-", "Week", "x", "t (m)", "", "B2", "pH", "am",
        ] {
            assert!(!is_unit(u, true), "{u}");
        }
        assert!(!is_unit("A", false));
        assert!(is_unit("A", true));
    }

    #[test]
    fn labels_split_their_unit() {
        let s = |t: &str| split_label(t);
        assert_eq!(
            s("Excavated diameter (m)"),
            (Some("Excavated diameter".into()), Some("m".into()))
        );
        assert_eq!(s("Load [kN]"), (Some("Load".into()), Some("kN".into())));
        assert_eq!(s("Span in mm"), (Some("Span".into()), Some("mm".into())));
        assert_eq!(
            s("Diameter 10.0 mm"),
            (Some("Diameter".into()), Some("mm".into()))
        );
        assert_eq!(s("Phase 2 A"), (Some("Phase 2 A".into()), None));
        assert_eq!(s("Height (h)"), (Some("Height (h)".into()), None));
        assert_eq!(s("Type A"), (Some("Type A".into()), None));
        assert_eq!(s("Radius, R"), (Some("Radius, R".into()), None));
        assert_eq!(s("Area [A]"), (Some("Area".into()), Some("A".into())));
        assert_eq!(s("t (m)"), (Some("t".into()), Some("m".into())));
        assert_eq!(s("kPa"), (None, Some("kPa".into())));
        assert_eq!(s("m"), (Some("m".into()), None));
        assert_eq!(s("Rate:"), (Some("Rate".into()), None));
        assert_eq!(
            s("Excavated (bored) diameter"),
            (Some("Excavated (bored) diameter".into()), None)
        );
    }

    #[test]
    fn number_formats_name_their_unit() {
        let f = from_number_format;
        assert_eq!(f("0.0\" m\""), Some("m".into()));
        assert_eq!(f("#,##0.00 [$€-407]"), Some("€".into()));
        assert_eq!(f("\"$\"#,##0"), Some("$".into()));
        assert_eq!(f("$#,##0.00;[Red]-$#,##0.00"), Some("$".into()));
        assert_eq!(f("0.00\\ \\k\\N"), Some("kN".into()));
        assert_eq!(f("[$-409]mm/dd/yy"), None);
        assert_eq!(f("0%"), None);
        assert_eq!(f("\"Week \"0"), None);
        assert_eq!(f("General"), None);
        assert_eq!(f("_($* #,##0_)"), Some("$".into()));
    }

    #[test]
    fn convert_units() {
        for u in [
            "mi", "km", "lbm", "kg", "mph", "m/s", "ozm", "cel", "kpc", "Nmi",
        ] {
            assert!(is_convert_unit(u), "{u}");
        }
        assert!(!is_convert_unit("banana"));
    }
}
