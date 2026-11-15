// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether a fact holds in a document. A fact never holds on a read that lost one of its
//! texts, and a pair holds only in the body.
//!
//! - `row`: one row of a table has a cell for each text, left to right in the order of the
//!   fact. A text is the whole text of its cell; with `@N` it is the N-th line of the cell.
//! - `same_row`: one row of a table has a cell of its own for each text, in any order.
//! - `not_row`: both texts stand in the document, and no row of any table holds both, in two
//!   cells or glued in one. A text that is lost fails the fact.
//! - `cell_under`: a cell of a header row holds the header and a cell in a later row of the
//!   same table, whose columns all lie within the columns of the header cell, holds the
//!   value. A table that marks no header row has its first row as one.
//! - `pair`: a table row with two cells that hold text, the label then the value; or, in a
//!   table read as label and value pairs or as totals, two such cells side by side; or one
//!   line of text `LABEL: VALUE`, where the value is the text after the colon of that label,
//!   up to the next label with a colon or the end of the line; an amount is the first amount
//!   there and a contained text stands at its start (see [`line_pair`]). A colon at
//!   the end of a label does not count, and a colon between two digits parts nothing. Form
//!   fields listed beside the body are no pair: the body must show it.
//! - `heading`: a heading block with exactly this text, and this level when one is given.
//! - `order`: both texts are in the text of the document, and the first place of the first is
//!   before the first place of the second.
//! - `text`: one block or cell holds the text.
//! - `link`: a link has exactly this target.

use rezzer_model::RegionKind;

use crate::doc::{Placed, Table, Unit, View};
use crate::facts::{Fact, Kind, Mode, Term};
use crate::text::{Decimal, find_words, first_numbers, has_number, has_words, starts_with_words};

/// What the check of one fact found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub pass: bool,
    /// Where the first text of the fact that the document holds stands.
    pub place: Place,
}

/// Where a fact's text stands in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// In a block of a region of this kind.
    Region(RegionKindName),
    /// In a block the document lists no region for.
    NoRegion,
    /// None of the fact's texts is in the document.
    NotFound,
}

/// A region kind by its name in the model JSON, so that it sorts and prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionKindName(pub &'static str);

/// The name of `kind` as the model JSON writes it.
pub fn kind_name(kind: RegionKind) -> &'static str {
    match kind {
        RegionKind::HeaderBand => "header_band",
        RegionKind::FooterBand => "footer_band",
        RegionKind::TitleBlock => "title_block",
        RegionKind::Heading => "heading",
        RegionKind::Paragraph => "paragraph",
        RegionKind::List => "list",
        RegionKind::Table => "table",
        RegionKind::LabelValue => "label_value",
        RegionKind::Totals => "totals",
        RegionKind::Figure => "figure",
        RegionKind::Code => "code",
        RegionKind::FootnoteLegend => "footnote_legend",
        RegionKind::Spatial => "spatial",
    }
}

impl Place {
    /// The name in the regions table of the scorecard.
    pub fn name(self) -> &'static str {
        match self {
            Place::Region(kind) => kind.0,
            Place::NoRegion => "(no region)",
            Place::NotFound => "(not found)",
        }
    }
}

/// The text a term is compared with: the unit's whole text, or the line the term names.
fn target<'a>(term: &Term, unit: &'a Unit) -> Option<&'a str> {
    match term.line {
        None => Some(unit.text.as_str()),
        Some(n) => unit.lines.get(n - 1).map(String::as_str),
    }
}

/// Whether `text` is what `term` asks for, in a document that parts decimals with `decimal`.
fn fits(term: &Term, text: &str, decimal: Decimal) -> bool {
    match &term.mode {
        Mode::Exact => text == term.text,
        Mode::Contains => has_words(text, &term.text),
        Mode::Number(n) => has_number(text, n, decimal),
        // Any label: text with a letter and no digit, so that an amount beside an amount is
        // no pair.
        Mode::Any => {
            text.chars().any(char::is_alphabetic) && !text.chars().any(|c| c.is_ascii_digit())
        }
    }
}

/// Whether the unit is what `term` asks for.
fn matches(view: &View, term: &Term, unit: &Unit) -> bool {
    target(term, unit).is_some_and(|text| fits(term, text, view.decimal))
}

/// Whether the unit holds the text of `term` at all, among other text.
fn holds(view: &View, term: &Term, unit: &Unit) -> bool {
    match &term.mode {
        Mode::Exact | Mode::Contains => has_words(&unit.text, &term.text),
        Mode::Number(n) => has_number(&unit.text, n, view.decimal),
        Mode::Any => false,
    }
}

/// Whether the document holds the text of `term` in some block or cell.
fn stands(view: &View, term: &Term) -> bool {
    view.units.iter().any(|u| holds(view, term, u))
}

/// `text` without a colon at its end.
fn label_text(text: &str) -> &str {
    text.strip_suffix(':').unwrap_or(text).trim_end()
}

/// Whether `text` is the label `term` asks for: a colon at the end of either does not count.
fn fits_label(term: &Term, text: &str, decimal: Decimal) -> bool {
    match &term.mode {
        Mode::Exact => label_text(text) == label_text(&term.text),
        _ => fits(term, text, decimal),
    }
}

/// The cells of `row` that hold text, left to right.
fn filled<'a>(view: &'a View, row: &[Placed]) -> Vec<&'a Unit> {
    row.iter()
        .map(|p| &view.units[p.unit])
        .filter(|u| !u.text.is_empty())
        .collect()
}

/// Whether the terms can each take a cell of their own among `cells`: `taken` marks the cells
/// in use.
fn assign(view: &View, terms: &[Term], cells: &[&Unit], taken: &mut Vec<bool>) -> bool {
    let Some((first, rest)) = terms.split_first() else {
        return true;
    };
    for (k, cell) in cells.iter().enumerate() {
        if !taken[k] && matches(view, first, cell) {
            taken[k] = true;
            if assign(view, rest, cells, taken) {
                return true;
            }
            taken[k] = false;
        }
    }
    false
}

fn row(view: &View, terms: &[Term]) -> bool {
    view.tables.iter().flat_map(|t| &t.rows).any(|row| {
        let mut cells = row.iter().map(|p| &view.units[p.unit]);
        // The leftmost cell for each text in turn: if any choice fits, this one does.
        terms
            .iter()
            .all(|t| cells.any(|cell| matches(view, t, cell)))
    })
}

fn same_row(view: &View, terms: &[Term]) -> bool {
    view.tables.iter().flat_map(|t| &t.rows).any(|row| {
        let cells: Vec<&Unit> = row.iter().map(|p| &view.units[p.unit]).collect();
        assign(view, terms, &cells, &mut vec![false; cells.len()])
    })
}

fn shares_a_row(view: &View, a: &Term, b: &Term) -> bool {
    view.tables.iter().flat_map(|t| &t.rows).any(|row| {
        let mut cells = row.iter().map(|p| &view.units[p.unit]);
        let mut others = cells.clone();
        cells.any(|c| holds(view, a, c)) && others.any(|c| holds(view, b, c))
    })
}

fn cell_under(view: &View, header: &Term, value: &Term) -> bool {
    let in_table = |table: &Table| {
        let head = table.header_rows.max(1).min(table.rows.len());
        table.rows[..head].iter().enumerate().any(|(h, row)| {
            row.iter()
                .filter(|p| fits_label(header, &view.units[p.unit].text, view.decimal))
                .any(|top| {
                    // The value's columns lie within the header's: a wide cell over several
                    // headers stands under none of them.
                    table.rows[h + 1..].iter().flatten().any(|p| {
                        p.unit != top.unit
                            && top.first <= p.first
                            && p.end <= top.end
                            && matches(view, value, &view.units[p.unit])
                    })
                })
        })
    };
    view.tables.iter().any(in_table)
}

/// Where the colons of `line` part a label from a value: every colon but one between two
/// digits (a time, a ratio).
fn colons(line: &str) -> Vec<usize> {
    let digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
    line.match_indices(':')
        .map(|(at, _)| at)
        .filter(|&at| {
            !(digit(line[..at].chars().next_back()) && digit(line[at + 1..].chars().next()))
        })
        .collect()
}

/// A segment between two colons of a line, parted in the value of the label before it and
/// the next label: the next label is the last words that hold no digit (at least the last
/// word), the value is the words before them. Where a value ends and a label starts is not
/// stated by the line, so a value of words alone before another label is not read: the fact
/// fails there, which is the safe side.
fn value_and_label(segment: &str) -> (String, String) {
    let words: Vec<&str> = segment.split(' ').filter(|w| !w.is_empty()).collect();
    let digitless = |w: &&&str| !w.chars().any(|c| c.is_ascii_digit());
    let label = words
        .iter()
        .rev()
        .take_while(digitless)
        .count()
        .max(1)
        .min(words.len());
    let (value, label) = words.split_at(words.len() - label);
    (value.join(" "), label.join(" "))
}

/// Whether one line reads `LABEL: VALUE`. The colons of the line part it into segments. The
/// label is the text before its colon: at the start of the line the whole segment, after an
/// earlier colon the label part of the segment (see [`value_and_label`]). The value is the
/// text after that colon: after the last colon the whole rest of the line, before another
/// colon the value part of the segment. An amount (`#`) is the first amount of that span and a
/// contained text (`~`) stands at its start (see [`fits_line_value`]). So `Netto: 100,00
/// Brutto: 119,00` pairs `Netto` with 100,00 and `Brutto` with 119,00, and never `Netto` with
/// 119,00, with or without a colon after `Brutto`.
fn line_pair(line: &str, label: &Term, value: &Term, decimal: Decimal) -> bool {
    let at = colons(line);
    (0..at.len()).any(|k| {
        let before = if k == 0 {
            line[..at[0]].trim().to_string()
        } else {
            value_and_label(&line[at[k - 1] + 1..at[k]]).1
        };
        let after = match at.get(k + 1) {
            Some(&next) => value_and_label(&line[at[k] + 1..next]).0,
            None => line[at[k] + 1..].trim().to_string(),
        };
        !before.is_empty()
            && !after.is_empty()
            && fits_label(label, &before, decimal)
            && fits_line_value(value, &after, decimal)
    })
}

/// Whether the value span of a line (see [`line_pair`]) is what `term` asks for. A line does
/// not always mark where a value ends: a next label with no colon (`Netto: 100,00 Brutto
/// 119,00`) is still in the span. So an amount (`#`) is only the first amount of the span,
/// and a contained text (`~`) stands only at its start. An exact text is the whole span.
fn fits_line_value(term: &Term, span: &str, decimal: Decimal) -> bool {
    match &term.mode {
        Mode::Number(n) => first_numbers(span, decimal).contains(n),
        Mode::Contains => starts_with_words(span, &term.text),
        Mode::Exact | Mode::Any => fits(term, span, decimal),
    }
}

fn pair(view: &View, label: &Term, value: &Term) -> bool {
    let d = view.decimal;
    let in_row = |table: &Table, row: &Vec<Placed>| {
        let cells = filled(view, row);
        let is = |l: &Unit, v: &Unit| {
            target(label, l).is_some_and(|t| fits_label(label, t, d)) && matches(view, value, v)
        };
        if cells.len() == 2 {
            return is(cells[0], cells[1]);
        }
        let pairs = matches!(
            table.region,
            Some(RegionKind::LabelValue | RegionKind::Totals)
        );
        // Side by side by place, so that an empty value does not shift the pairs after it.
        pairs
            && row.as_chunks::<2>().0.iter().any(|c| {
                let (l, v) = (&view.units[c[0].unit], &view.units[c[1].unit]);
                !l.text.is_empty() && !v.text.is_empty() && is(l, v)
            })
    };
    let in_tables = view
        .tables
        .iter()
        .any(|t| t.rows.iter().any(|row| in_row(t, row)));
    in_tables
        || view
            .units
            .iter()
            .flat_map(|u| &u.lines)
            .any(|line| line_pair(line, label, value, d))
}

fn heading(view: &View, text: &Term, level: Option<u8>) -> bool {
    view.units.iter().any(|u| {
        u.heading
            .is_some_and(|l| level.is_none_or(|want| want == l))
            && fits(text, &u.text, view.decimal)
    })
}

/// Where the text of `term` stands first in the document's text.
fn first_place(view: &View, term: &Term) -> Option<usize> {
    match &term.mode {
        Mode::Exact | Mode::Contains => find_words(&view.stream, &term.text, 0),
        // An amount stands where the first unit that holds it starts.
        Mode::Number(_) => {
            let mut at = 0;
            for unit in view.units.iter().filter(|u| !u.text.is_empty()) {
                if holds(view, term, unit) {
                    return Some(at);
                }
                at += unit.text.len() + 1;
            }
            None
        }
        Mode::Any => None,
    }
}

fn order(view: &View, a: &Term, b: &Term) -> bool {
    match (first_place(view, a), first_place(view, b)) {
        (Some(first), Some(second)) => first < second,
        _ => false,
    }
}

/// The unit that holds a text of the fact, the fact's first text first.
fn unit_of(view: &View, fact: &Fact) -> Option<usize> {
    if let Kind::Link(target) = &fact.kind {
        return view.links.iter().find(|(t, _)| t == target).map(|l| l.1);
    }
    fact.terms()
        .into_iter()
        .find_map(|t| view.units.iter().position(|u| holds(view, t, u)))
}

/// Checks `fact` against the document.
pub fn check(view: &View, fact: &Fact) -> Outcome {
    let pass = match &fact.kind {
        Kind::Row(terms) => row(view, terms),
        Kind::SameRow(terms) => same_row(view, terms),
        Kind::NotRow(a, b) => stands(view, a) && stands(view, b) && !shares_a_row(view, a, b),
        Kind::CellUnder { header, value } => cell_under(view, header, value),
        Kind::Pair { label, value } => pair(view, label, value),
        Kind::Heading { text, level } => heading(view, text, *level),
        Kind::Order(a, b) => order(view, a, b),
        Kind::Text(text) => stands(view, text),
        Kind::Link(target) => view.links.iter().any(|(t, _)| t == target),
    };
    let place = match unit_of(view, fact) {
        None => Place::NotFound,
        Some(unit) => match view.region_of(unit) {
            Some(region) => Place::Region(RegionKindName(kind_name(region.kind))),
            None => Place::NoRegion,
        },
    };
    Outcome { pass, place }
}
