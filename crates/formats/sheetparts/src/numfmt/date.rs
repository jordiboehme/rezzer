// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dates and times: serial numbers to calendar fields and the date and time sections.

use super::{DateSystem, Formatted, NumKind, Section, Tok, plain};

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

pub(super) fn render_date(section: &Section, value: f64, system: DateSystem) -> Formatted {
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
