// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rules H1, H2 and H3, print titles and rule L (spec 6.5).

use crate::detect::{Line, Lines, Tick};
use crate::input::{CellIn, Class, Rect, column_letters};
use crate::{Certainty, Evidence};

/// Header rows the rules may give.
pub(crate) const RULE_BAND_MAX: u64 = 4;
/// Header rows a fact (print titles here) may give.
pub(crate) const FACT_BAND_MAX: u64 = 10;
/// Left columns rule L may name.
const LABEL_COLS_MAX: u64 = 3;

/// What the header rules decided for a region.
pub(crate) enum Band {
    /// No band: no rule found one.
    None,
    /// The rules disagree, with the reason in words.
    Unsure(String, Vec<Evidence>),
    /// The band, first and last sheet row.
    Rows {
        first: u32,
        end: u32,
        certainty: Certainty,
        evidence: Vec<Evidence>,
    },
}

/// What a row of a region holds, for the header rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    /// One text cell and at most the merge that holds it (spec 6.3).
    Single,
    /// Only text cells and years, no formula.
    Header,
    /// A number, date, bool or formula cell, and not a header row.
    Data,
    /// Neither: text with an error value, for example.
    Other,
}

/// An integer from 1900 to 2100 (the year rule of H1).
fn is_year(c: &CellIn) -> bool {
    if c.class != Class::Number {
        return false;
    }
    let n = c.number.or_else(|| c.text.trim().parse::<f64>().ok());
    n.is_some_and(|n| n.fract() == 0.0 && (1900.0..=2100.0).contains(&n))
}

fn sort(lines: &Lines, line: &Line) -> Sort {
    if lines.single_text(line).is_some() {
        return Sort::Single;
    }
    let mut header = true;
    let mut data = false;
    let mut any = false;
    for c in lines.cells_of(line) {
        any = true;
        if c.formula.is_some() {
            header = false;
            data = true;
            continue;
        }
        match c.class {
            Class::Text => {}
            Class::Number if is_year(c) => {}
            Class::Number | Class::Date | Class::Bool => {
                header = false;
                data = true;
            }
            Class::Error => header = false,
        }
    }
    if !any {
        Sort::Other
    } else if header {
        Sort::Header
    } else if data {
        Sort::Data
    } else {
        Sort::Other
    }
}

/// One-based row words: `row 3` or `rows 3 to 4`.
pub(crate) fn row_words(first: u32, end: u32) -> String {
    if first == end {
        format!("row {}", u64::from(first) + 1)
    } else {
        format!("rows {} to {}", u64::from(first) + 1, u64::from(end) + 1)
    }
}

/// Rule H1's band: its last row and the row where the values change type.
struct H1 {
    end: u32,
    change: u32,
}

/// The rows of `lines` from `row` on, down to `bottom`.
fn lines_from<'l>(lines: &'l Lines, row: u32, bottom: u32) -> impl Iterator<Item = &'l Line> {
    lines.lines[lines.first_at(row)..]
        .iter()
        .take_while(move |l| l.row <= bottom)
}

/// Rule H1 (type change): the leading rows of only text and years, at most 4, then (past
/// single-text rows) a row with a number, date, bool or formula. A vertical merge that starts
/// in the band extends it to its last row; a band past 4 rows is none.
fn h1(lines: &Lines, top: u32, bottom: u32, tick: &Tick) -> Option<H1> {
    let mut end = None;
    for (expect, l) in (u64::from(top)..).zip(lines_from(lines, top, bottom)) {
        if u64::from(l.row) != expect || sort(lines, l) != Sort::Header {
            break;
        }
        end = Some(l.row);
        if u64::from(l.row) - u64::from(top) + 1 > RULE_BAND_MAX {
            return None;
        }
    }
    let mut end = end?;
    let tops: Vec<u32> = lines
        .merges
        .iter()
        .map(|&m| lines.prep.merges[m as usize].top)
        .collect();
    loop {
        let lo = tops.partition_point(|&t| t < top);
        let hi = tops.partition_point(|&t| t <= end);
        let ext = lines.merges[lo..hi]
            .iter()
            .map(|&m| lines.prep.merges[m as usize].bottom.min(bottom))
            .filter(|&b| b > end)
            .max();
        let Some(ext) = ext else {
            break;
        };
        if u64::from(ext) - u64::from(top) + 1 > RULE_BAND_MAX {
            return None;
        }
        if lines_from(lines, end + 1, ext).any(|l| sort(lines, l) != Sort::Header) {
            return None;
        }
        end = ext;
    }
    if end == u32::MAX {
        return None;
    }
    for l in lines_from(lines, end + 1, bottom) {
        if tick.step() {
            return None;
        }
        match sort(lines, l) {
            Sort::Single => continue,
            Sort::Data => {
                return Some(H1 { end, change: l.row });
            }
            Sort::Header | Sort::Other => return None,
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Bold,
    Fill,
    No,
}

/// A row is marked when every non-empty cell is bold, or every one is filled. `None` when a
/// flag it needs is unknown.
fn mark(lines: &Lines, line: &Line) -> Option<Mark> {
    let all = |flag: fn(&CellIn) -> Option<bool>| -> Option<bool> {
        let mut unknown = false;
        for c in lines.cells_of(line).filter(|c| !c.text.is_empty()) {
            match flag(c) {
                Some(false) => return Some(false),
                None => unknown = true,
                Some(true) => {}
            }
        }
        (!unknown).then_some(true)
    };
    let bold = all(|c| c.style.bold);
    let fill = all(|c| c.style.filled);
    match (bold, fill) {
        (Some(true), _) => Some(Mark::Bold),
        (_, Some(true)) => Some(Mark::Fill),
        (Some(false), Some(false)) => Some(Mark::No),
        _ => None,
    }
}

/// Rule H2's answer.
enum H2 {
    /// Styles are not read.
    NotRead,
    /// No band of 1 to 4 marked rows followed by an unmarked one, or a flag is unknown.
    NoAnswer,
    /// The last marked row, and whether the marks were all bold.
    Band { end: u32, bold: bool },
}

impl H2 {
    fn words(bold: bool) -> &'static str {
        if bold { "bold" } else { "fill" }
    }
}

/// Rule H2 (style change): the leading marked rows, at most 4, single-text rows skipped,
/// when the next row is not marked.
fn h2(lines: &Lines, top: u32, bottom: u32, tick: &Tick) -> H2 {
    if !lines.prep.input.styles_read {
        return H2::NotRead;
    }
    let mut count = 0u64;
    let mut last = None;
    let mut bold = true;
    for l in lines_from(lines, top, bottom) {
        if tick.step() {
            return H2::NoAnswer;
        }
        if sort(lines, l) == Sort::Single {
            continue;
        }
        match mark(lines, l) {
            None => return H2::NoAnswer,
            Some(Mark::No) => {
                return match last {
                    Some(end) => H2::Band { end, bold },
                    None => H2::NoAnswer,
                };
            }
            Some(m) => {
                count += 1;
                if count > RULE_BAND_MAX {
                    return H2::NoAnswer;
                }
                last = Some(l.row);
                bold &= m == Mark::Bold;
            }
        }
    }
    H2::NoAnswer
}

/// Rule H3 (all-text header): the first row, when a positive signal names it and its cells
/// fill every column, are all text, all different and none repeats below in its column.
/// Gives the signal's words.
fn h3(lines: &Lines, data: Rect, h2: &H2, tick: &Tick) -> Option<String> {
    let top = data.top;
    let signal = match h2 {
        H2::Band { end, bold } if *end == top => format!("{} first row", H2::words(*bold)),
        _ if lines.prep.input.header_guess == Some((top, true)) => "header guess".to_string(),
        _ => return None,
    };
    let line = lines.at(top)?;
    let mut texts: Vec<&str> = Vec::new();
    for c in lines.cells_of(line) {
        if c.text.is_empty() || c.class != Class::Text || c.formula.is_some() {
            return None;
        }
        texts.push(&c.text);
    }
    if texts.len() as u64 != lines.rect.width() {
        return None;
    }
    let mut sorted = texts.clone();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0] == w[1]) {
        return None;
    }
    let mut below = false;
    for l in lines_from(lines, top.saturating_add(1), data.bottom).filter(|l| l.row > top) {
        below = true;
        for c in lines.cells_of(l) {
            if tick.step() {
                return None;
            }
            let k = (c.pos.col - lines.rect.left) as usize;
            if texts.get(k) == Some(&c.text.as_str()) {
                return None;
            }
        }
    }
    below.then_some(signal)
}

/// The last row of the print titles, when they start at or above the first row after the
/// captions, reach it, leave data rows and give at most 10 header rows.
fn print_band(lines: &Lines, top: u32, bottom: u32) -> Option<u32> {
    let (a, b) = lines.prep.input.print_titles?;
    let (a, b) = (a.min(b), a.max(b));
    (a <= top && top <= b && b < bottom && u64::from(b) - u64::from(top) < FACT_BAND_MAX)
        .then_some(b)
}

/// Step 6a: the header band of a rule region's data area (the rows after its captions).
/// `had_print` says the print titles lie in the region.
pub(crate) fn band(lines: &Lines, data: Rect, had_print: bool, tick: &Tick) -> Band {
    let (top, bottom) = (data.top, data.bottom);
    let one = h1(lines, top, bottom, tick);
    let two = h2(lines, top, bottom, tick);
    let h1_words = |h: &H1| format!("header {} by type change", row_words(top, h.end));
    let h2_words = |end: u32, bold: bool| {
        let how = if bold { "bold" } else { "filled" };
        format!("{how} header {}", row_words(top, end))
    };
    if had_print && let Some(end) = print_band(lines, top, bottom) {
        let win = format!("; the print titles name {} and win", row_words(top, end));
        let mut evidence = vec![Evidence::PrintTitles];
        if let Some(h) = &one {
            let mut detail = h1_words(h);
            if h.end != end {
                detail.push_str(&win);
            }
            evidence.push(Evidence::Rule { id: "H1", detail });
        }
        if let H2::Band { end: e, bold } = two {
            let mut detail = h2_words(e, bold);
            if e != end {
                detail.push_str(&win);
            }
            evidence.push(Evidence::Rule { id: "H2", detail });
        }
        return Band::Rows {
            first: top,
            end,
            certainty: Certainty::Exact,
            evidence,
        };
    }
    match one {
        Some(h) => {
            let mut evidence = vec![Evidence::Rule {
                id: "H1",
                detail: h1_words(&h),
            }];
            if let H2::Band { end, bold } = two {
                if end != h.end {
                    return Band::Unsure(
                        format!(
                            "values change type at row {} but {} ends at row {}",
                            u64::from(h.change) + 1,
                            H2::words(bold),
                            u64::from(end) + 1
                        ),
                        Vec::new(),
                    );
                }
                evidence.push(Evidence::Rule {
                    id: "H2",
                    detail: h2_words(end, bold),
                });
            }
            Band::Rows {
                first: top,
                end: h.end,
                certainty: Certainty::Rule,
                evidence,
            }
        }
        None => match h3(lines, data, &two, tick) {
            Some(signal) => Band::Rows {
                first: top,
                end: top,
                certainty: Certainty::Rule,
                evidence: vec![Evidence::Rule {
                    id: "H3",
                    detail: format!("all-text header {}, {signal}", row_words(top, top)),
                }],
            },
            None => Band::None,
        },
    }
}

/// Rule L: the left columns, at most 3 and never every column, whose data rows hold text in
/// at least 80 % of the rows and whose header cells are empty or text. Gives the count and the
/// words.
pub(crate) fn label_cols(
    lines: &Lines,
    header_rows: &[u32],
    data: Rect,
    tick: &Tick,
) -> (u32, Option<String>) {
    let rect = lines.rect;
    let max = LABEL_COLS_MAX.min(rect.width() - 1) as usize;
    if max == 0 {
        return (0, None);
    }
    let left = rect.left;
    let reach = u64::from(left) + max as u64;
    let mut ok = [true; LABEL_COLS_MAX as usize];
    for &r in header_rows {
        if let Some(l) = lines.at(r) {
            for c in lines
                .cells_of(l)
                .take_while(|c| u64::from(c.pos.col) < reach)
            {
                if c.class != Class::Text {
                    ok[(c.pos.col - left) as usize] = false;
                }
            }
        }
    }
    let mut counts = [0u64; LABEL_COLS_MAX as usize];
    let mut rows = 0u64;
    for l in lines_from(lines, data.top, data.bottom) {
        if tick.step() {
            return (0, None);
        }
        rows += 1;
        for c in lines
            .cells_of(l)
            .take_while(|c| u64::from(c.pos.col) < reach)
        {
            if c.class == Class::Text && !c.text.is_empty() {
                counts[(c.pos.col - left) as usize] += 1;
            }
        }
    }
    let mut n = 0usize;
    while rows > 0 && n < max && ok[n] && counts[n] * 5 >= rows * 4 {
        n += 1;
    }
    let words = match n {
        0 => None,
        1 => Some(format!("label column {}", column_letters(left))),
        _ => Some(format!(
            "label columns {} to {}",
            column_letters(left),
            column_letters(left + n as u32 - 1)
        )),
    };
    (n as u32, words)
}
