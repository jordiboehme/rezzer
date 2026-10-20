// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! R1C1 text of references, so copies of one formula share a pattern key.

use crate::refs::{CellPos, Ref, Whole, book_prefix, sheet_prefix};

/// One axis: absolute `R5` (1-based), relative `R[-2]`, or `R` for the same row.
fn axis(letter: char, value: u32, origin: u32, abs: bool) -> String {
    if abs {
        return format!("{letter}{}", u64::from(value) + 1);
    }
    let offset = i64::from(value) - i64::from(origin);
    if offset == 0 {
        letter.to_string()
    } else {
        format!("{letter}[{offset}]")
    }
}

fn cell(pos: CellPos, abs_row: bool, abs_col: bool, at: CellPos) -> String {
    format!(
        "{}{}",
        axis('R', pos.row, at.row, abs_row),
        axis('C', pos.col, at.col, abs_col)
    )
}

/// The address part in R1C1, relative to the formula cell `at`.
fn address(r: &Ref, at: CellPos) -> Option<String> {
    match r {
        Ref::Cell {
            pos,
            abs_row,
            abs_col,
            ..
        } => Some(cell(*pos, *abs_row, *abs_col, at)),
        Ref::Range {
            start,
            end,
            abs,
            whole,
            ..
        } => Some(match whole {
            Whole::None => format!(
                "{}:{}",
                cell(*start, abs[0], abs[1], at),
                cell(*end, abs[2], abs[3], at)
            ),
            Whole::Cols => format!(
                "{}:{}",
                axis('C', start.col, at.col, abs[1]),
                axis('C', end.col, at.col, abs[3])
            ),
            Whole::Rows => format!(
                "{}:{}",
                axis('R', start.row, at.row, abs[0]),
                axis('R', end.row, at.row, abs[2])
            ),
        }),
        _ => None,
    }
}

/// The R1C1 text of a cell or range reference, with its sheet or book prefix. `None` for
/// names and tables, which stay as written.
pub(crate) fn r1c1(r: &Ref, at: CellPos) -> Option<String> {
    match r {
        Ref::Cell { sheet, .. } | Ref::Range { sheet, .. } => {
            let addr = address(r, at)?;
            Some(match sheet {
                Some(span) => format!("{}!{addr}", sheet_prefix(span)),
                None => addr,
            })
        }
        Ref::External { book, inner } => {
            let sheet = match inner.as_ref() {
                Ref::Cell { sheet, .. } | Ref::Range { sheet, .. } => sheet.as_ref(),
                _ => return None,
            };
            let prefix = book_prefix(book, sheet.map(|s| s.first.as_str()));
            Some(format!("{prefix}!{}", address(inner, at)?))
        }
        Ref::Name { .. } | Ref::Table { .. } => None,
    }
}
