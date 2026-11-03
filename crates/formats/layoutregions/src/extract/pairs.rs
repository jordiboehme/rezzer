// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and their values: the text of a drawn box or of a cell between rules read as fields,
//! each a label and its value (none: an empty field), or why it is not clear.
//!
//! Finding where fields stand stays with the reader: the boxes a page draws and which runs
//! each holds, the frames of rules and their dividers, the groups of boxes and of rows, the
//! text beyond a group, titles, claims and every loss it writes. Reading their text is here:
//! [`parse`] reads a box or a cell as fields, [`box_fields`] the boxes of a page, and
//! [`strips`] the rows of fields between rules (their cells, the cuts at field edges, labels
//! under the rule their values stand on). [`blocks`] holds the rules for lines without rules:
//! a block of labels and values, and blocks of facts side by side (an address beside a bank).
//! A pair whose value is not clear is never guessed: the reader gets the reason (one of the
//! constants here) and the runs, and says it.
//!
//! Every function works on the reader's own runs (see [`LineRun`]), borrowed, and takes and
//! returns indexes into the slice it is given: the reader cuts runs at field edges, and the
//! pieces share one primitive, so an index names a piece where a primitive id cannot.

use std::collections::HashSet;

use crate::extract::text::address_shaped;
use crate::line::{LABEL_STEP, LineRun};

pub mod blocks;
pub mod strips;

/// Why a box beside fields is not read as a field.
pub const THREE_STYLES: &str = "it holds text in three styles or more";

pub const LABEL_BELOW: &str = "its label stands below its value";

/// Why a box beside fields is not read as a field: its text is of one style, so no label is
/// set apart from a value (a value with no label, a name over an address at the values' size).
pub const NO_LABEL: &str = "its text is of one style, with no label set apart from a value";

/// Why a cell of a row of fields is left out of it: its lines are an address.
pub const ADDRESS: &str = "its lines read as an address, not as a label over its value";

/// Why a cell of a row of fields is left out of it: its lines read as an address under a
/// first line without a number and without a colon at its end.
pub const NAMED_ADDRESS: &str = "its lines read as an address under a name, and nothing \
     shows its first line to be a label";

pub const WEIGHT_ONLY: &str = "its label and its value differ in weight, slant or font only, so \
     which is the label is not clear";

/// A label is at most this many characters and words, a value of a field between rules at
/// most this many characters.
pub const LABEL_CHARS: usize = 40;

pub const LABEL_WORDS: usize = 6;

pub(crate) const VALUE_CHARS: usize = 80;

/// A box's label is at most this many lines.
pub const LABEL_LINES: usize = 2;

/// One field: the runs of its label and of its value (none: the field is empty).
#[derive(Debug, Default)]
pub struct Field {
    pub label: Vec<usize>,
    pub value: Vec<usize>,
}

/// What the text of a box or of a cell between rules is.
#[derive(Debug)]
pub enum Parsed {
    Fields(Vec<Field>),
    /// No label and value: the text stays as it is.
    No,
    /// Not paired, for this reason.
    Unsure(&'static str),
}

/// A run's style: its size in half points, bold, italic, and its font family (see
/// [`LineRun::font`]; 0 when not known). A bold or italic face of a family is that
/// family: an address in Courier beside labels in Helvetica of one size is not in their style.
pub type Style = (i32, bool, bool, u32);

pub fn style_of<R: LineRun>(r: &R) -> Style {
    (
        (r.size() * 2.0).round() as i32,
        r.bold(),
        r.italic(),
        r.font(),
    )
}

/// Whether two runs are in one style: one [`style_of`], or one face whose sizes are less than
/// a size step apart (see [`LABEL_STEP`]): sizes on both sides of a half-point edge of the key
/// by float noise are one size.
pub fn same_style<R: LineRun>(a: &R, b: &R) -> bool {
    let (sa, sb) = (style_of(a), style_of(b));
    sa == sb
        || ((sa.1, sa.2, sa.3) == (sb.1, sb.2, sb.3)
            && a.size().max(b.size()) < LABEL_STEP * a.size().min(b.size()))
}

/// The mean exact size of the runs `ids`, raised markers `marker` aside (0 for none): the
/// half-point key of [`style_of`] parts sizes that differ by float noise only (8.749 and 8.751
/// points), so a size step (see [`LABEL_STEP`]) is told by the exact sizes.
pub(crate) fn mean_size<R: LineRun>(
    runs: &[R],
    ids: impl Iterator<Item = usize>,
    marker: &HashSet<usize>,
) -> f32 {
    let (mut sum, mut n) = (0.0f32, 0usize);
    for i in ids.filter(|i| !marker.contains(i)) {
        sum += runs[i].size();
        n += 1;
    }
    sum / n.max(1) as f32
}

/// Whether the text `ids` is lines of one face whose first line is smaller than the lines
/// under it, but by less than a size step (see [`LABEL_STEP`]; the half-point keys may differ
/// or not): a first line that may be meant as labels, though no label style is set apart from
/// the values.
pub(crate) fn under_a_step<R: LineRun>(runs: &[R], ids: &[usize]) -> bool {
    let ids: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| inked(&runs[i]) && !runs[i].off_axis())
        .collect();
    let lines = lines_of(runs, &ids);
    let marker = markers(runs, &lines);
    let Some((first, rest)) = lines.split_first() else {
        return false;
    };
    let face = |i: &usize| {
        let s = style_of(&runs[*i]);
        (s.1, s.2, s.3)
    };
    let one_face = lines
        .iter()
        .flatten()
        .filter(|i| !marker.contains(i))
        .all(|i| face(i) == face(&first[0]));
    let (a, b) = (
        mean_size(runs, first.iter().copied(), &marker),
        mean_size(runs, rest.iter().flatten().copied(), &marker),
    );
    one_face && !rest.is_empty() && a > 0.0 && b > a && b < LABEL_STEP * a
}

pub fn inked<R: LineRun>(r: &R) -> bool {
    r.text().chars().any(|c| !c.is_whitespace())
}

/// Whether `r` is a raised marker of the run `left` before it on its line (a footnote
/// reference of one or two characters, set smaller and higher): it goes with that run and
/// counts for no style of its own.
pub(crate) fn raised<R: LineRun>(r: &R, left: &R) -> bool {
    let t = r.text().trim();
    r.note()
        || (!t.is_empty()
            && t.chars().count() <= 2
            && t.chars().all(|c| c.is_alphanumeric() || c == '*')
            && r.size() <= 0.8 * left.size()
            && r.y() - left.y() >= 0.2 * left.size())
}

/// The raised markers of `lines` (see [`raised`]): each goes with the run before it.
pub fn markers<R: LineRun>(runs: &[R], lines: &[Vec<usize>]) -> HashSet<usize> {
    let mut marker: HashSet<usize> = HashSet::new();
    for line in lines {
        for w in line.windows(2) {
            if raised(&runs[w[1]], &runs[w[0]]) {
                marker.insert(w[1]);
            }
        }
    }
    marker
}

/// The runs `ids` in lines from the top, each line left to right: a run joins a line its
/// height overlaps by at least half of the smaller of the two.
pub fn lines_of<R: LineRun>(runs: &[R], ids: &[usize]) -> Vec<Vec<usize>> {
    let extent = |r: &R| (r.y() - 0.25 * r.size(), r.y() + 0.75 * r.size());
    let mut order = ids.to_vec();
    order.sort_by(|&a, &b| runs[b].y().total_cmp(&runs[a].y()));
    let mut lines: Vec<(Vec<usize>, (f32, f32))> = Vec::new();
    for i in order {
        let (lo, hi) = extent(&runs[i]);
        if let Some((line, core)) = lines.last_mut() {
            let overlap = hi.min(core.1) - lo.max(core.0);
            let smaller = (hi - lo).min(core.1 - core.0).max(0.01);
            if overlap / smaller >= 0.5 {
                line.push(i);
                continue;
            }
        }
        lines.push((vec![i], (lo, hi)));
    }
    lines
        .into_iter()
        .map(|(mut l, _)| {
            l.sort_by(|&a, &b| runs[a].x().total_cmp(&runs[b].x()));
            l
        })
        .collect()
}

/// How a box or a cell is read.
#[derive(Clone, Copy)]
pub struct Reading<'a> {
    /// A box: its text is one field, its label at most [`LABEL_LINES`] lines (lines of the
    /// label style one under the other are one label).
    pub one_field: bool,
    /// The label must be a size step smaller than the value (see [`LABEL_STEP`]); else
    /// another weight or slant also sets it apart.
    pub smaller: bool,
    /// Text of one style under this size (half points) is labels only: empty fields.
    pub small: i32,
    /// Styles known as labels' on the page (the text alone in an empty boxed field): a label
    /// and a value of one size that differ in weight, slant or font only pair when the label's
    /// style is one of them, else which one is the label is not clear.
    pub labels: &'a [Style],
}

/// The fields the runs `ids` are: text of two styles, the first one met (at the top left)
/// the label's, smaller than the value or in another weight or slant; each label starts a
/// field and the value beside it on its line or on the lines under it is its value. Text of
/// one small style is labels of empty fields. Three styles or more, or a label after a value
/// in a box, is unsure.
pub fn parse<R: LineRun>(runs: &[R], ids: &[usize], how: Reading) -> Parsed {
    let ids: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| inked(&runs[i]) && !runs[i].off_axis())
        .collect();
    let lines = lines_of(runs, &ids);
    let marker = markers(runs, &lines);
    let mut styles: Vec<Style> = Vec::new();
    for &i in lines.iter().flatten() {
        let s = style_of(&runs[i]);
        if !marker.contains(&i) && !styles.contains(&s) {
            styles.push(s);
        }
    }
    match styles.len() {
        0 => return Parsed::No,
        1 => {
            // Bold text alone in a box or between rules is a title, not the label of an empty
            // field: the text after it is its own.
            if styles[0].0 >= how.small || styles[0].1 {
                return Parsed::No;
            }
            // Labels only: each line an empty field, or in a box one label of a line or two.
            if how.one_field {
                if lines.len() > LABEL_LINES {
                    return Parsed::No;
                }
                return Parsed::Fields(vec![Field {
                    label: lines.into_iter().flatten().collect(),
                    value: Vec::new(),
                }]);
            }
            return Parsed::Fields(
                lines
                    .into_iter()
                    .map(|label| Field {
                        label,
                        value: Vec::new(),
                    })
                    .collect(),
            );
        }
        2 => {}
        _ => return Parsed::Unsure(THREE_STYLES),
    }
    // The exact size of a style's text (see [`mean_size`]).
    let exact = |s: Style| {
        let of = lines
            .iter()
            .flatten()
            .copied()
            .filter(|&i| style_of(&runs[i]) == s);
        mean_size(runs, of, &marker)
    };
    let one_size = |a: Style, b: Style| {
        let (x, y) = (exact(a), exact(b));
        x.max(y) < LABEL_STEP * x.min(y)
    };
    let apart = |label: Style, value: Style| {
        (value.0 - label.0 >= 1 && exact(value) >= LABEL_STEP * exact(label))
            || (!how.smaller
                && (label.1, label.2, label.3) != (value.1, value.2, value.3)
                && (label.0 <= value.0 || one_size(label, value)))
    };
    let (label, value) = (styles[0], styles[1]);
    // Lines that read as an address are no label over its value: a first line with a number
    // (a street over its postcode and town) at any size; a first line without one over address
    // lines (a name, a company, or a label) is a label only on evidence (see below). Such a
    // cell is left out of its row, its text read as it stands, and a loss says so.
    match address_cell(runs, &lines, &marker) {
        Some(Address::Numbered) => return Parsed::Unsure(ADDRESS),
        Some(Address::Named) => {
            // The first line is a label only when it ends with a colon: a style shared with
            // other labels is no evidence here, as such a style may itself come from a pair
            // that size alone decided.
            let head: Vec<usize> = lines[0]
                .iter()
                .copied()
                .filter(|i| !marker.contains(i))
                .collect();
            if !words(runs, &head).ends_with(':') {
                return Parsed::Unsure(NAMED_ADDRESS);
            }
        }
        None => {}
    }
    if !apart(label, value) {
        // The label's style first met under the value's: the label stands below it.
        return if apart(value, label) {
            Parsed::Unsure(LABEL_BELOW)
        } else {
            Parsed::No
        };
    }
    // Of one size, weight, slant or font alone does not say which line is the label (a bold
    // value over a regular label reads like a bold label over a regular value): only a style
    // known as a label's does.
    if label.0 == value.0 || one_size(label, value) {
        if how.labels.contains(&value) {
            return Parsed::Unsure(LABEL_BELOW);
        }
        if !how.labels.contains(&label) {
            return Parsed::Unsure(WEIGHT_ONLY);
        }
    }
    let is_label = |i: usize, prev: Option<bool>| {
        if marker.contains(&i) {
            prev.unwrap_or(false)
        } else {
            style_of(&runs[i]) == label
        }
    };
    let mut fields: Vec<Field> = Vec::new();
    // The field being read, and whether its value has begun.
    let mut open: Option<(usize, bool)> = None;
    let mut label_lines = 0;
    for line in &lines {
        let mut kinds: Vec<bool> = Vec::with_capacity(line.len());
        for &i in line {
            let prev = kinds.last().copied();
            kinds.push(is_label(i, prev));
        }
        // A line is labels, then values: a label right of a value is no field.
        let first_value = kinds.iter().position(|k| !k).unwrap_or(kinds.len());
        if kinds[first_value..].iter().any(|k| *k) {
            return Parsed::Unsure(LABEL_BELOW);
        }
        let (labels, values) = line.split_at(first_value);
        if !labels.is_empty() {
            match open {
                // In a box, a second label line under the first, before any value, goes on it.
                Some((f, false)) if how.one_field && label_lines < LABEL_LINES => {
                    fields[f].label.extend_from_slice(labels);
                    label_lines += 1;
                }
                Some(_) if how.one_field => return Parsed::Unsure(LABEL_BELOW),
                _ => {
                    fields.push(Field {
                        label: labels.to_vec(),
                        value: Vec::new(),
                    });
                    label_lines = 1;
                }
            }
            if how.one_field && label_lines > LABEL_LINES {
                return Parsed::No;
            }
            open = Some((fields.len() - 1, false));
        }
        if !values.is_empty() {
            let Some((f, _)) = open else {
                // A value before any label: a label under it is unsure, none is no field.
                let later = lines
                    .iter()
                    .flatten()
                    .any(|&i| !marker.contains(&i) && style_of(&runs[i]) == label);
                return if later {
                    Parsed::Unsure(LABEL_BELOW)
                } else {
                    Parsed::No
                };
            };
            fields[f].value.extend_from_slice(values);
            open = Some((f, true));
        }
    }
    Parsed::Fields(fields)
}

/// A cell of an address holds at most this many lines: a name, a line more, a street, a
/// postcode and town, a country.
pub(crate) const ADDRESS_LINES: usize = 5;

/// What [`address_cell`] finds a cell's lines to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Address {
    /// An address whose first line holds a number, or starts with `c/o`: a street over its
    /// postcode and town, a postcode and town over the country.
    Numbered,
    /// A first line without a number (a name, a company, or a label) over address lines.
    Named,
}

/// Whether `lines` (raised markers aside) read as an address (see [`Address`]): two address
/// lines (see [`blocks::address_lines`]) whose second holds a word without a digit (a
/// town: a line of codes and numbers alone is a value); three lines or more
/// of an address's shape (see [`address_shaped`]) whose first line holds a
/// letter and a number; a first line that starts with `c/o`, or with a postcode (a number of
/// four digits or more) and a town over the country (words only); or a first line without a
/// number over lines that are an address by one of these. Two to [`ADDRESS_LINES`] lines, none
/// with an amount, a decimal number or a date.
pub(crate) fn address_cell<R: LineRun>(
    runs: &[R],
    lines: &[Vec<usize>],
    marker: &HashSet<usize>,
) -> Option<Address> {
    let kept: Vec<Vec<usize>> = lines
        .iter()
        .map(|l| l.iter().copied().filter(|i| !marker.contains(i)).collect())
        .filter(|l: &Vec<usize>| !l.is_empty())
        .collect();
    if !(2..=ADDRESS_LINES).contains(&kept.len()) {
        return None;
    }
    let texts: Vec<String> = kept.iter().map(|l| words(runs, l)).collect();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    // An address holds no amount, no decimal number, no date: digits on both sides of a
    // point or a comma.
    let decimal = |t: &str| {
        let c: Vec<char> = t.chars().collect();
        c.windows(3)
            .any(|w| w[0].is_ascii_digit() && matches!(w[1], '.' | ',') && w[2].is_ascii_digit())
    };
    if texts.iter().any(|t| decimal(t)) {
        return None;
    }
    let numbered = |t: &[&str]| {
        let head = t[0];
        let letters = |l: &str| l.chars().any(char::is_alphabetic);
        let digits = |l: &str| l.chars().any(|c| c.is_ascii_digit());
        let words = |l: &str| letters(l) && !digits(l);
        let postcode = head
            .split_whitespace()
            .next()
            .is_some_and(|w| w.len() >= 4 && w.chars().all(|c| c.is_ascii_digit()))
            && letters(head);
        // The line under a street holds a word (a town, a country): a line of codes and numbers
        // alone under a numbered label (`Konto 1` over `DE00 1234 5678`) is its value.
        let word_under = t[1..].iter().any(|l| l.split_whitespace().any(&words));
        (blocks::address_lines(t) && word_under)
            || (letters(head) && digits(head) && address_shaped(t))
            || (head.to_lowercase().starts_with("c/o ") && t[1..].iter().all(|l| letters(l)))
            || (postcode && t[1..].iter().all(|l| words(l)) && t.len() == 2)
    };
    if numbered(&texts) {
        Some(Address::Numbered)
    } else if !texts[0].chars().any(|c| c.is_ascii_digit())
        && texts.len() >= 3
        && (numbered(&texts[1..]) || address_shaped(&texts[1..]))
    {
        Some(Address::Named)
    } else {
        None
    }
}

/// The text of runs, words joined by one space.
pub fn words<R: LineRun>(runs: &[R], ids: &[usize]) -> String {
    let mut out = String::new();
    for &i in ids {
        for w in runs[i].text().split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(w);
        }
    }
    out
}

/// Whether a field's label and value are as short as a form's.
pub fn short<R: LineRun>(runs: &[R], f: &Field) -> bool {
    let label = words(runs, &f.label);
    label.chars().count() <= LABEL_CHARS
        && label.split_whitespace().count() <= LABEL_WORDS
        && words(runs, &f.value).chars().count() <= VALUE_CHARS
}

/// The size under which text of one style is labels only (half points): half a point under
/// the largest of the page's text sizes; none on a page without text sizes.
pub fn small_size(text_sizes: &[f32]) -> i32 {
    text_sizes
        .last()
        .map_or(i32::MIN, |s| (s * 2.0).round() as i32)
}

/// The fields of drawn boxes, each box's runs `in_box` read as one field (see [`parse`]; a
/// box `consumed` by a framed title is not read): a box whose text is a label at its top or
/// left in one style and a value in another, or a label alone smaller than the largest of the
/// page's `text_sizes` (an empty field). Boxes whose label and value differ in weight, slant
/// or font only are read again knowing the styles of the labels alone in empty fields. Gives
/// each box's field, and the boxes not clear with why.
pub fn box_fields<R: LineRun>(
    runs: &[R],
    in_box: &[Vec<usize>],
    consumed: &[bool],
    text_sizes: &[f32],
) -> (Vec<Option<Field>>, Vec<(usize, &'static str)>) {
    let mut how = Reading {
        one_field: true,
        smaller: false,
        small: small_size(text_sizes).saturating_sub(1),
        labels: &[],
    };
    let mut fields: Vec<Option<Field>> = (0..in_box.len()).map(|_| None).collect();
    let mut unsure: Vec<(usize, &'static str)> = Vec::new();
    for (b, ids) in in_box.iter().enumerate() {
        if ids.is_empty() || consumed[b] {
            continue;
        }
        match parse(runs, ids, how) {
            Parsed::Fields(mut f) if f.len() == 1 => fields[b] = f.pop(),
            Parsed::Unsure(why) => unsure.push((b, why)),
            _ => {}
        }
    }
    // The style of the text alone in an empty boxed field is a label's: boxes whose label and
    // value differ in weight, slant or font only are read again knowing it.
    let mut known: Vec<Style> = fields
        .iter()
        .flatten()
        .filter(|f| f.value.is_empty())
        .filter_map(|f| f.label.first().map(|&i| style_of(&runs[i])))
        .collect();
    known.sort_unstable();
    known.dedup();
    if !known.is_empty() && unsure.iter().any(|u| u.1 == WEIGHT_ONLY) {
        how.labels = &known;
        for (b, why) in &mut unsure {
            if *why != WEIGHT_ONLY {
                continue;
            }
            match parse(runs, &in_box[*b], how) {
                Parsed::Fields(mut f) if f.len() == 1 => {
                    fields[*b] = f.pop();
                    *why = "";
                }
                Parsed::Unsure(again) => *why = again,
                _ => *why = "",
            }
        }
        unsure.retain(|u| !u.1.is_empty());
    }
    (fields, unsure)
}

#[cfg(test)]
mod tests;
