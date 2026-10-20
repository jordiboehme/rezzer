// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A1 addresses and sheet name quoting shared by the readers.

/// A sheet name as a formula anchor writes it. The rules are those of formulakit's
/// `quote_sheet`, so anchors from both crates agree: plain only when the name is letters,
/// digits and underscores, does not start with a digit and does not look like `A1` or `R1C1`.
pub(crate) fn quote_sheet(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| !c.is_ascii_digit())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !looks_like_a1(name)
        && !looks_like_r1c1(name);
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn looks_like_a1(name: &str) -> bool {
    let letters = name.chars().take_while(char::is_ascii_alphabetic).count();
    let rest = name.get(letters..).unwrap_or("");
    (1..=3).contains(&letters) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn looks_like_r1c1(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if let Some(rest) = upper.strip_prefix('R') {
        match rest.split_once('C') {
            Some((r, c)) => digits(r) && digits(c),
            None => digits(rest),
        }
    } else if let Some(rest) = upper.strip_prefix('C') {
        digits(rest)
    } else {
        false
    }
}

/// Column letters for a 0-based column: 0 is `A`, 16383 is `XFD`.
pub(crate) fn col_letters(col: u32) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The A1 address of a 0-based position.
pub(crate) fn a1(row: u32, col: u32) -> String {
    format!("{}{}", col_letters(col), row + 1)
}
