// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Labels and values and blocks of facts in lines without rules: a block of lines of a label
//! and its value ([`key_values`]), and blocks of lines side by side that are blocks of their
//! own, an address beside a bank or a register entry, not rows ([`blocks_of`],
//! [`address_columns`]). The rules of an address's lines ([`address_lines`], [`postal`],
//! [`address_block`]) are here too: the reader's headings and lists read them as well.

use super::ADDRESS_LINES;
use crate::extract::table::grid::{Piece, columns_of, text_of};
use crate::extract::table::{MAX_SQUARES, RunRange};
use crate::extract::text::{ADDRESS_WORDS, address_shaped, word_count};
use crate::file::data_only;
use crate::line::LineRun;
use crate::segment::cut::SIZE_STEP;

/// Whether two lines one under the other are an address: each a few words (at most
/// [`ADDRESS_WORDS`]) with letters and a number, the number in another place on each (see
/// [`word_shape`]): a street and its number over a postcode and its town. A label over its
/// value has a number on one line at most (`Nr` over `4711`, `Strasse` over `Musterweg 1`), or
/// in the same place on both (`Zimmer 2` over `Raum 101`). Their left edge is the caller's to
/// check.
pub(crate) fn address_lines(lines: &[&str]) -> bool {
    let line = |l: &&str| {
        word_count(l) <= ADDRESS_WORDS
            && l.chars().any(char::is_alphabetic)
            && l.chars().any(|c| c.is_ascii_digit())
    };
    matches!(lines, [a, b] if line(a) && line(b) && word_shape(a) != word_shape(b))
}

/// Whether lines are a block of an address, a bank or a register entry: [`address_shaped`],
/// and not all of one shape (see [`word_shape`]), as a name, a street and a postcode with its
/// town are not. The labels or values of one form in a column of a table (`Feld 1`,
/// `Feld 2`, or `Musterweg 1`, `Musterweg 2`) have one shape.
pub fn address_block(lines: &[&str]) -> bool {
    let Some(first) = lines.first() else {
        return false;
    };
    let shape = word_shape(first);
    lines.iter().any(|l| word_shape(l) != shape) && address_shaped(lines)
}

/// A line of a block in [`address_columns`]: its text, whether all of it is bold, and its
/// size.
#[derive(Debug, Clone)]
pub struct ColumnLine {
    pub text: String,
    pub bold: bool,
    pub size: f32,
}

/// What [`address_columns`] makes of blocks of lines side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Columns {
    /// Rows of one block, a table or labels and values: read as they are.
    Rows,
    /// Blocks of their own, read one after the other. Rows were possible too (their lines
    /// share baselines), so that is said.
    Blocks,
    /// Blocks of their own under a header row of their headings (`Anschrift`,
    /// `Bankverbindung`): read as the table they make, with the doubt said.
    Headed,
    /// Rows that hold a postal address in a column but are no blocks by their evidence: read
    /// as rows, with the doubt said (they may be blocks side by side).
    Doubt,
}

/// How well the lines of a column are a block of facts of its own (see [`fact_block`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fact {
    /// No block of facts: too long, an amount, a line without a letter, no digit.
    No,
    /// A block of facts whose lines are of one kind by their words or shape (two `Telefon`
    /// lines, two `IBAN` lines, two lines that end with the town): a block beside an address,
    /// and a column of a table as well.
    Kinds,
    /// A block of facts whose lines are each of another kind.
    Facts,
}

/// Whether the lines of a column are a block of facts of its own (an address, a bank, a
/// register entry, a contact). A first line that ends with a colon over two lines or more is
/// its heading (`Bankverbindung:`), and the block is the lines under it. The block holds two
/// to [`ADDRESS_LINES`] lines, each short (at most [`ADDRESS_WORDS`] words) with a letter, no
/// colon at its end (a label's) and no amount alone (see [`data_only`]), and a digit on some
/// line (a street number, a postcode, an account, a phone number): else [`Fact::No`]. Each line
/// a fact of another kind (no two lines start or end with the same word, and not all have one
/// shape, see [`word_shape`]) is [`Fact::Facts`]; else, when two lines start or end with the
/// same word but the lines under its first do not all start with one word (`Musterweg 1`,
/// `Musterweg 2` are records), [`Fact::Kinds`]. The
/// lines of a column of a table are records of one kind: they share words (`Halle 2`, `Halle
/// 1`; `12 Stueck`, `8 Stueck`) or a shape (`3 Kartons`, `12 Paletten`).
fn fact_block(block: &[&ColumnLine]) -> Fact {
    let headed = block.len() >= 3
        && block[0].text.ends_with(':')
        && !block[0].text.chars().any(|c| c.is_ascii_digit());
    let block = if headed { &block[1..] } else { block };
    if !(2..=ADDRESS_LINES).contains(&block.len()) {
        return Fact::No;
    }
    let short = |l: &&ColumnLine| {
        word_count(&l.text) <= ADDRESS_WORDS
            && l.text.chars().any(char::is_alphabetic)
            && !l.text.ends_with(':')
            && !data_only(&l.text)
    };
    if !block.iter().all(short)
        || !block
            .iter()
            .any(|l| l.text.chars().any(|c| c.is_ascii_digit()))
    {
        return Fact::No;
    }
    let distinct = |word: &dyn Fn(&str) -> Option<String>| {
        let mut seen: Vec<String> = block.iter().filter_map(|l| word(&l.text)).collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        seen.len() == n
    };
    let first = |t: &str| t.split_whitespace().next().map(str::to_lowercase);
    let last = |t: &str| t.split_whitespace().last().map(str::to_lowercase);
    let shapes = block
        .iter()
        .any(|l| word_shape(&l.text) != word_shape(&block[0].text));
    if distinct(&first) && distinct(&last) && shapes {
        return Fact::Facts;
    }
    // Lines of one kind repeat a word; lines of one shape alone (`Kundennr. 1`, `Vertrag 7`)
    // are labels or records, no block.
    let rest: Vec<Option<String>> = block[1..].iter().map(|l| first(&l.text)).collect();
    let repeated = !distinct(&first) || !distinct(&last);
    if !repeated || rest.windows(2).all(|w| w[0] == w[1]) {
        Fact::No
    } else {
        Fact::Kinds
    }
}

/// A label's index holds at most this many characters (`1`, `12`, `A2`).
const INDEX_CHARS: usize = 3;

/// Whether `text` is a numbered label: words without a digit, then one word with a digit of at
/// most [`INDEX_CHARS`] characters (`Kundennr. 1`, `Vertrag 7`, `Tarif A2`).
fn indexed(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some((last, head)) = words.split_last() else {
        return false;
    };
    !head.is_empty()
        && head
            .iter()
            .all(|w| w.chars().any(char::is_alphabetic) && !w.chars().any(|c| c.is_ascii_digit()))
        && last.chars().any(|c| c.is_ascii_digit())
        && last.chars().count() <= INDEX_CHARS
}

/// Whether lines are a postal address: a line of a postcode (a number of four digits or more)
/// and a town, and a line of a street or a PO box with its number (words, then a number of
/// at most four characters: `Musterweg 1`, `Hauptstr. 2a`, `Postfach 12 34`).
pub(crate) fn postal(lines: &[&str]) -> bool {
    let town = lines.iter().any(|l| {
        let mut w = l.split_whitespace();
        w.next()
            .is_some_and(|n| n.len() >= 4 && n.chars().all(|c| c.is_ascii_digit()))
            && w.next().is_some_and(|t| t.chars().any(char::is_alphabetic))
    });
    let street = lines.iter().any(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        w.len() >= 2
            && w[0]
                .chars()
                .all(|c| c.is_alphabetic() || c == '.' || c == '-')
            && w.last().is_some_and(|n| {
                n.len() <= 4 && n.chars().next().is_some_and(|c| c.is_ascii_digit())
            })
    });
    town && street
}

/// Whether blocks of lines side by side on shared baselines are blocks of their own (an
/// address, a bank, a register entry, a contact beside each other) and not the columns of a
/// table or of labels and values. `columns` holds each column's lines by row, `None` where
/// the column has no line on a row. They are blocks when all of these hold:
/// - two columns or more and three rows or more;
/// - each column a block: its lines one under the other, with no row left empty between
///   them (a block may start a row lower or end higher than the others);
/// - each column a block of facts of its own (see [`fact_block`]): the evidence is each
///   block's, never one column's. One column whose lines are of one kind ([`Fact::Kinds`]:
///   two phone numbers, two banks) counts too, when a column of the region is a [`postal`]
///   address;
/// - one column an [`address_block`].
///
/// Their lines share baselines, so rows were possible too: the caller says the doubt. A first
/// row that is a header (bold over lines that are not, or a size step larger) over such blocks
/// may be their headings or a table's titles: [`Columns::Headed`]. Lines that are no such
/// blocks but where a column holds a postal address (and no column of labels without a digit
/// stands left of it, and no column of amounts beside it) are read as rows with the doubt
/// said: [`Columns::Doubt`].
pub fn address_columns(columns: &[Vec<Option<ColumnLine>>]) -> Columns {
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    if columns.len() < 2 || rows < 3 {
        return Columns::Rows;
    }
    let text_of = |c: &[Option<ColumnLine>]| -> Vec<String> {
        c.iter().flatten().map(|l| l.text.clone()).collect()
    };
    let postal_at = columns
        .iter()
        .position(|c| postal(&text_of(c).iter().map(String::as_str).collect::<Vec<_>>()));
    let has_postal = postal_at.is_some();
    // A column without a digit left of the address is its labels (`Name`, `Strasse`, `Ort`):
    // rows, no doubt. So is one whose every line is words and a short index after them
    // (`Kundennr. 1`, `Vertrag 7`, `Tarif A2`): numbered labels. A phone or an account line has
    // a long number (`Telefon 0123 4567`), and stays a doubt.
    let labels = postal_at.is_some_and(|p| {
        columns[..p].iter().any(|c| {
            let lines = text_of(c);
            !lines
                .iter()
                .any(|t| t.chars().any(|ch| ch.is_ascii_digit()))
                || (!lines.is_empty() && lines.iter().all(|t| indexed(t)))
        })
    });
    // A column of amounts beside it makes rows of a table: no doubt either.
    let amounts = columns
        .iter()
        .any(|c| c.iter().flatten().all(|l| data_only(&l.text)));
    let rows_or_doubt = if has_postal && !labels && !amounts {
        Columns::Doubt
    } else {
        Columns::Rows
    };
    // Each column's lines, one under the other without a gap (a block may start a row
    // lower than the others, or end higher).
    let mut blocks: Vec<Vec<&ColumnLine>> = Vec::with_capacity(columns.len());
    let mut kinds = 0;
    for c in columns {
        let start = c.iter().take_while(|l| l.is_none()).count();
        let block: Vec<&ColumnLine> = c[start..].iter().map_while(Option::as_ref).collect();
        if c[start + block.len()..].iter().any(Option::is_some) {
            return rows_or_doubt;
        }
        match fact_block(&block) {
            Fact::Facts => {}
            Fact::Kinds => kinds += 1,
            Fact::No => return rows_or_doubt,
        }
        blocks.push(block);
    }
    if kinds > 1 || (kinds == 1 && !has_postal) {
        return rows_or_doubt;
    }
    let addressed = blocks
        .iter()
        .any(|b| address_block(&b.iter().map(|l| l.text.as_str()).collect::<Vec<_>>()));
    if !addressed {
        return rows_or_doubt;
    }
    // A header: the first row, a line in every column, bold over lines that are not, or a size
    // step larger.
    let Some(first) = columns
        .iter()
        .map(|c| c.first().and_then(Option::as_ref))
        .collect::<Option<Vec<&ColumnLine>>>()
    else {
        return Columns::Blocks;
    };
    let rest: Vec<&ColumnLine> = blocks.iter().flat_map(|b| b[1..].iter().copied()).collect();
    let bold_head = first.iter().all(|l| l.bold) && rest.iter().any(|l| !l.bold);
    let smallest_head = first.iter().map(|l| l.size).fold(f32::MAX, f32::min);
    let largest_rest = rest.iter().map(|l| l.size).fold(0.0f32, f32::max);
    if bold_head || smallest_head >= SIZE_STEP * largest_rest {
        Columns::Headed
    } else {
        Columns::Blocks
    }
}

/// The shape of a line by its words: for each run of words with a digit and each run of words
/// without one, whether it holds a digit. `Musterweg 1` and `Feld 2` have one shape, `Muster
/// GmbH` and `12345 Musterstadt` each another.
fn word_shape(text: &str) -> Vec<bool> {
    let mut out: Vec<bool> = Vec::new();
    for w in text.split_whitespace() {
        let digit = w.chars().any(|c| c.is_ascii_digit());
        if out.last() != Some(&digit) {
            out.push(digit);
        }
    }
    out
}

/// Lines of labels and values one table may have at most.
pub(crate) const MAX_KEY_VALUES: usize = MAX_SQUARES / 2;

/// Labels and values line up within this many points.
const KV_ALIGN: f32 = 1.0;

/// A label is at most this many characters and words; a value at most this many characters.
const KV_LABEL: usize = 40;
const KV_LABEL_WORDS: usize = 6;
const KV_VALUE: usize = 80;

/// One row of a block of labels and values: its line, and the flow ranges of its label and
/// value; a row without a value range has both in one run, cut after the label's colon.
pub struct KeyValue {
    pub line: usize,
    pub label: (usize, usize),
    pub value: Option<(usize, usize)>,
}

/// What [`key_values`] makes of lines of a region.
pub enum KeyValues {
    No,
    /// A block of this many lines: more than one table may have.
    TooMany(usize),
    Rows(Vec<KeyValue>),
}

/// The block of labels and values the lines `a..z` of a region are, when they are one: at
/// least three lines in a row of exactly two pieces, the labels flush left, the values flush
/// left or right or the labels ending in a colon, no label or value wrapping, short pieces,
/// and no bold first line over plain ones (a header). A line right after the block whose one
/// piece starts with a short label and a colon joins it, cut after the colon.
pub fn key_values<R: LineRun>(
    runs: &[R],
    lines: &[RunRange],
    a: usize,
    z: usize,
    pieces: &[Vec<Piece>],
    off: &[bool],
) -> KeyValues {
    if pieces.iter().any(|p| p.len() != 2) || z - a < 2 {
        return KeyValues::No;
    }
    if z - a > MAX_KEY_VALUES {
        return KeyValues::TooMany(z - a);
    }
    let label = |p: &Piece| text_of(runs, p.start, p.end);
    let labels: Vec<String> = pieces.iter().map(|p| label(&p[0])).collect();
    let colons = labels.iter().all(|l| l.ends_with(':'));
    // Lines after the region whose one run starts with a short label and a colon.
    let mut extra: Vec<usize> = Vec::new();
    let mut end = z;
    while colons && end < lines.len() && !off[end] && extra.len() < MAX_KEY_VALUES {
        let (prev, cur) = (&lines[end - 1], &lines[end]);
        let gap = prev.y - cur.y;
        if gap <= 0.0 || gap > 2.0 * prev.size.max(cur.size) || cur.end - cur.start != 1 {
            break;
        }
        let text = runs[cur.start].text().trim();
        let Some(cut) = text.find(": ") else { break };
        let head = &text[..=cut];
        if head.chars().count() > KV_LABEL || head.split_whitespace().count() > KV_LABEL_WORDS {
            break;
        }
        if (runs[cur.start].x() - pieces[0][0].x0).abs() > KV_ALIGN {
            break;
        }
        extra.push(end);
        end += 1;
    }
    let n = z - a + extra.len();
    if n < 3 {
        return KeyValues::No;
    }
    let x0 = pieces[0][0].x0;
    let lefts = pieces.iter().all(|p| (p[0].x0 - x0).abs() <= KV_ALIGN);
    let (v0, v1) = (pieces[0][1].x0, pieces[0][1].x1);
    let flush_left = pieces.iter().all(|p| (p[1].x0 - v0).abs() <= KV_ALIGN);
    let flush_right = pieces.iter().all(|p| (p[1].x1 - v1).abs() <= KV_ALIGN);
    if !lefts || !(flush_left || flush_right || colons) {
        return KeyValues::No;
    }
    let short = labels
        .iter()
        .all(|l| l.chars().count() <= KV_LABEL && l.split_whitespace().count() <= KV_LABEL_WORDS)
        && pieces
            .iter()
            .all(|p| text_of(runs, p[1].start, p[1].end).chars().count() <= KV_VALUE);
    if !short {
        return KeyValues::No;
    }
    // Lines of the block follow each other at most two line heights apart.
    let spaced = (a + 1..z).all(|k| {
        let gap = lines[k - 1].y - lines[k].y;
        gap > 0.0 && gap <= 2.0 * lines[k - 1].size.max(lines[k].size)
    });
    // A line right after the block that starts under the values goes on a value.
    let wraps = lines.get(end).is_some_and(|next| {
        let last = &lines[end - 1];
        let gap = last.y - next.y;
        gap > 0.0
            && gap <= 1.6 * last.size.max(next.size)
            && (runs[next.start].x() - v0).abs() <= KV_ALIGN
    });
    let bold = |l: &RunRange| runs[l.start..l.end].iter().all(|r| r.bold());
    let header = bold(&lines[a]) && (a + 1..z).any(|k| !bold(&lines[k]));
    if !spaced || wraps || header {
        return KeyValues::No;
    }
    let mut rows: Vec<KeyValue> = pieces
        .iter()
        .enumerate()
        .map(|(k, p)| KeyValue {
            line: a + k,
            label: (p[0].start, p[0].end),
            value: Some((p[1].start, p[1].end)),
        })
        .collect();
    rows.extend(extra.into_iter().map(|k| KeyValue {
        line: k,
        label: (lines[k].start, lines[k].end),
        value: None,
    }));
    KeyValues::Rows(rows)
}

/// The lines of a region by column (see [`columns_of`]), each column's line on each row, and
/// the region's runs column by column, each column top down. A piece over two columns, two
/// pieces of a row in one column or a piece in no column gives no columns.
pub fn blocks_of<R: LineRun>(
    runs: &[R],
    pieces: &[Vec<Piece>],
) -> (Vec<Vec<Option<ColumnLine>>>, Vec<usize>) {
    let columns = columns_of(pieces);
    let none = || (Vec::new(), Vec::new());
    let mut lines: Vec<Vec<Option<ColumnLine>>> = vec![vec![None; pieces.len()]; columns.len()];
    let mut by_column: Vec<Vec<usize>> = vec![Vec::new(); columns.len()];
    for (row, line) in pieces.iter().enumerate() {
        for p in line {
            let hit: Vec<usize> = (0..columns.len())
                .filter(|&c| p.x1 > columns[c].0 && p.x0 < columns[c].1)
                .collect();
            let [c] = hit[..] else {
                return none();
            };
            if lines[c][row].is_some() {
                return none();
            }
            let inked: Vec<&R> = runs[p.start..p.end]
                .iter()
                .filter(|r| !r.text().trim().is_empty())
                .collect();
            if inked.is_empty() {
                return none();
            }
            lines[c][row] = Some(ColumnLine {
                text: text_of(runs, p.start, p.end),
                bold: inked.iter().all(|r| r.bold()),
                size: inked.iter().map(|r| r.size()).fold(0.0f32, f32::max),
            });
            by_column[c].extend(p.start..p.end);
        }
    }
    (lines, by_column.into_iter().flatten().collect())
}
