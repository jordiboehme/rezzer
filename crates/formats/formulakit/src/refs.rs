// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! References a formula reads, and their anchors.

/// The last row of a sheet (0-based), used for whole-column ranges.
pub const MAX_ROW: u32 = 1_048_575;
/// The last column of a sheet (0-based, `XFD`), used for whole-row ranges.
pub const MAX_COL: u32 = 16_383;

/// A cell position, 0-based (`A1` is row 0, column 0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellPos {
    pub row: u32,
    pub col: u32,
}

/// The sheet part of a reference: one sheet, or a 3D span from `first` to `last`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SheetSpan {
    pub first: String,
    /// The last sheet of a 3D reference (`Sheet1:Sheet3!A1`).
    pub last: Option<String>,
}

/// Whether a range covers whole rows or whole columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whole {
    /// A plain range like `A1:B9`.
    None,
    /// Whole rows like `2:3`: the columns run from 0 to [`MAX_COL`].
    Rows,
    /// Whole columns like `B:D`: the rows run from 0 to [`MAX_ROW`].
    Cols,
}

/// Something a formula reads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ref {
    /// One cell. `sheet` is `None` for the formula's own sheet.
    Cell {
        sheet: Option<SheetSpan>,
        pos: CellPos,
        abs_row: bool,
        abs_col: bool,
    },
    /// A range. `abs` holds the `$` flags: start row, start column, end row, end column.
    Range {
        sheet: Option<SheetSpan>,
        start: CellPos,
        end: CellPos,
        abs: [bool; 4],
        whole: Whole,
    },
    /// A defined name. `sheet` is set when the formula names the sheet (`Sheet1!gg`); a
    /// name without one may still be sheet scoped, which only the workbook can tell.
    Name { name: String, sheet: Option<String> },
    /// A structured table reference. `name` is empty when a formula inside the table leaves
    /// it out (`[@c]`); the workbook reader knows which table that is. `column` is the
    /// column (or `first:last` for a column span) when the reference names one.
    Table {
        name: String,
        column: Option<String>,
    },
    /// A reference into another workbook. `book` is written as in the formula: `[1]` (an
    /// index into the workbook's external links) or a path with the file name in brackets.
    External { book: String, inner: Box<Ref> },
}

/// The Excel column letters of a 0-based column (`0` is `A`, `27` is `AB`).
pub fn col_letters(col: u32) -> String {
    let mut n = u64::from(col) + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        // rem < 26, so the cast cannot truncate.
        out.push(char::from(b'A' + rem as u8));
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}

fn a1(pos: CellPos) -> String {
    format!("{}{}", col_letters(pos.col), u64::from(pos.row) + 1)
}

/// The address part of a cell or range, without `$` and without a sheet.
pub(crate) fn address(r: &Ref) -> Option<String> {
    match r {
        Ref::Cell { pos, .. } => Some(a1(*pos)),
        Ref::Range {
            start, end, whole, ..
        } => Some(match whole {
            Whole::None => format!("{}:{}", a1(*start), a1(*end)),
            Whole::Cols => format!("{}:{}", col_letters(start.col), col_letters(end.col)),
            Whole::Rows => format!("{}:{}", u64::from(start.row) + 1, u64::from(end.row) + 1),
        }),
        _ => None,
    }
}

/// Whether a sheet name can stand without quotes in a formula.
fn plain_sheet(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if first.is_ascii_digit() {
        return false;
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return false;
    }
    !looks_like_a1(name) && !looks_like_r1c1(name)
}

/// `A1`, `xfd1048576`: one to three letters, then digits.
fn looks_like_a1(name: &str) -> bool {
    let letters = name.chars().take_while(char::is_ascii_alphabetic).count();
    let rest = name.get(letters..).unwrap_or("");
    (1..=3).contains(&letters) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// `R`, `C`, `R1`, `C2`, `R1C1`, `RC`.
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

fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A sheet name as a formula writes it: plain when possible, else in single quotes with
/// each `'` doubled (`'My Sheet'`, `'Q1 ''24'`).
pub fn quote_sheet(name: &str) -> String {
    if plain_sheet(name) {
        name.to_string()
    } else {
        quoted(name)
    }
}

/// The sheet prefix of a reference, without the `!`. A 3D span is quoted as a whole.
pub(crate) fn sheet_prefix(span: &SheetSpan) -> String {
    match &span.last {
        None => quote_sheet(&span.first),
        Some(last) if plain_sheet(&span.first) && plain_sheet(last) => {
            format!("{}:{last}", span.first)
        }
        Some(last) => quoted(&format!("{}:{last}", span.first)),
    }
}

/// The book and sheet prefix of an external reference, without the `!`.
pub(crate) fn book_prefix(book: &str, sheet: Option<&str>) -> String {
    let inner = book
        .strip_prefix('[')
        .and_then(|b| b.strip_suffix(']'))
        .unwrap_or("");
    let plain_book = !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    match sheet {
        None => book.to_string(),
        Some(s) if plain_book && plain_sheet(s) => format!("{book}{s}"),
        Some(s) => quoted(&format!("{book}{s}")),
    }
}

/// The anchor of a reference read by a formula on `sheet`: `Sheet!A1`, `Sheet!A1:B9`,
/// `name:gg`, `name:Sheet1!gg`, `table:Table1` or `ext:[1]Sheet1!A1`. Cell anchors carry no
/// `$`: they name a place, not how the formula wrote it.
pub fn anchor_of(sheet: &str, r: &Ref) -> String {
    match r {
        Ref::Cell { sheet: span, .. } | Ref::Range { sheet: span, .. } => {
            let prefix = match span {
                Some(span) => sheet_prefix(span),
                None => quote_sheet(sheet),
            };
            format!("{prefix}!{}", address(r).unwrap_or_default())
        }
        Ref::Name { name, sheet: None } => format!("name:{name}"),
        Ref::Name {
            name,
            sheet: Some(s),
        } => format!("name:{}!{name}", quote_sheet(s)),
        Ref::Table { name, .. } => format!("table:{name}"),
        Ref::External { book, inner } => match inner.as_ref() {
            Ref::Cell { sheet: span, .. } | Ref::Range { sheet: span, .. } => {
                let s = span.as_ref().map(|s| s.first.as_str());
                format!(
                    "ext:{}!{}",
                    book_prefix(book, s.or(Some(sheet))),
                    address(inner).unwrap_or_default()
                )
            }
            Ref::Name { name, sheet: s } => {
                format!("ext:{}!{name}", book_prefix(book, s.as_deref()))
            }
            other => format!("ext:{book}{}", anchor_of(sheet, other)),
        },
    }
}

/// A sheet name as written in a formula, without its quotes: `'My S'` becomes `My S`.
fn unquote(text: &str) -> String {
    match text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => text.to_string(),
    }
}

/// Turns a name as the parser returns it (`gg`, `Sheet1!gg`, `'My S'!gg`, `[1]!gg`,
/// `[Book.xlsx]Sheet1!gg`) into a reference.
pub(crate) fn name_ref(text: &str) -> Ref {
    let (qualifier, name) = match text.rfind('!') {
        Some(i) => (text.get(..i).unwrap_or(""), text.get(i + 1..).unwrap_or("")),
        None => ("", text),
    };
    if qualifier.is_empty() {
        return Ref::Name {
            name: name.to_string(),
            sheet: None,
        };
    }
    let qualifier = unquote(qualifier);
    // Sheet names cannot hold `]`, so one marks a book: `[1]`, `[1]Sheet1`,
    // `C:\dir\[Book.xlsx]Sheet1`. The book ends at the last `]`.
    if let Some(end) = qualifier.rfind(']') {
        let book = qualifier.get(..=end).unwrap_or("").to_string();
        let sheet = qualifier.get(end + 1..).unwrap_or("");
        return Ref::External {
            book,
            inner: Box::new(Ref::Name {
                name: name.to_string(),
                sheet: (!sheet.is_empty()).then(|| sheet.to_string()),
            }),
        };
    }
    Ref::Name {
        name: name.to_string(),
        sheet: Some(qualifier),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters() {
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(MAX_COL), "XFD");
        assert_eq!(col_letters(u32::MAX), "MWLQKWV");
    }

    #[test]
    fn names_split_into_scope_and_book() {
        assert_eq!(
            name_ref("'[2]My S'!gg"),
            Ref::External {
                book: "[2]".into(),
                inner: Box::new(Ref::Name {
                    name: "gg".into(),
                    sheet: Some("My S".into())
                })
            }
        );
        assert_eq!(
            name_ref("[Book.xlsx]Sheet1!gg"),
            Ref::External {
                book: "[Book.xlsx]".into(),
                inner: Box::new(Ref::Name {
                    name: "gg".into(),
                    sheet: Some("Sheet1".into())
                })
            }
        );
        assert_eq!(
            name_ref("'it''s'!gg"),
            Ref::Name {
                name: "gg".into(),
                sheet: Some("it's".into())
            }
        );
    }

    #[test]
    fn book_prefixes_quote_paths() {
        assert_eq!(book_prefix("[1]", Some("Sheet1")), "[1]Sheet1");
        assert_eq!(book_prefix("[1]", Some("My S")), "'[1]My S'");
        assert_eq!(
            book_prefix("C:\\x\\[Book.xlsx]", Some("S")),
            "'C:\\x\\[Book.xlsx]S'"
        );
        assert_eq!(book_prefix("[1]", None), "[1]");
    }
}
