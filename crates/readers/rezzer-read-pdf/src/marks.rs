// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Separators drawn as marks. Boxed number fields of a form often print their decimal and
//! date separators as tiny filled rectangles, not as text; the digits of the value then
//! read without them. A mark between two digits of one baseline puts a placeholder into the
//! page's glyphs; once every page is read, the placeholders become the separators the
//! file's own text uses, or are removed and the number is listed as not read.

use crate::order::Turn;
use crate::scan;
use crate::tables::LABEL_STEP;
use crate::tables::{Rule, RuleKind};
use crate::text::{Gl, PageItems};

/// The long side of a mark is at most this many sizes of its text (3 points at 9.5 point
/// data): a separator is much shorter than the digits it stands between. Relative, so that a
/// form printed larger or smaller has the same marks.
pub(crate) const MARK_LONG: f32 = 0.32;
/// The long side of a mark is at least this many times its short side.
pub(crate) const MARK_RATIO: f32 = 3.0;
/// The short side of a mark is at most this many sizes of its text (0.5 points at 9.5 point
/// data), or [`MARK_PEN`] points: a filled divider of a comb field, a point wide, is no mark.
pub(crate) const MARK_THIN: f32 = 0.053;
/// A mark may always be this many points thick: the pen of a dashed rule keeps its width
/// when the text is set smaller.
pub(crate) const MARK_PEN: f32 = 0.5;
/// Marks one page keeps.
pub(crate) const MAX_MARKS: usize = 100_000;
/// Marks one page may test against its digits; past it, the rest is not looked at.
const MARK_WORK: usize = 4_000_000;
/// Unread numbers of one page named each in a loss; the rest are counted in one more.
const MAX_UNREAD: usize = 20;
/// The year widths of a date written as text.
const YEARS: std::ops::RangeInclusive<usize> = 2..=4;

/// The placeholder a mark puts between two digits: a noncharacter, which the text of a file
/// never brings in (see [`is_placeholder`]).
pub(crate) const SEP: char = '\u{FDD0}';
/// The placeholder a mark puts between two digits that do not read in the direction of the
/// page's text: their separator is not read, and the number is listed as not read.
pub(crate) const SEP_TURNED: char = '\u{FDD1}';
/// The placeholder a bar puts between two digits that has every trait of a separator mark but
/// is too long for one of their size: it is not read as one, and the number is listed as
/// not read, so its digits never stand without a word for what was drawn between them.
pub(crate) const SEP_LONG: char = '\u{FDD2}';

/// Whether `c` is a placeholder [`insert`] puts between two digits.
fn is_sep(c: char) -> bool {
    c == SEP || c == SEP_TURNED || c == SEP_LONG
}

/// Whether `c` is one of the noncharacters U+FDD0 to U+FDEF: Rezzer's own placeholders,
/// dropped from the text of a file.
pub(crate) fn is_placeholder(c: char) -> bool {
    ('\u{FDD0}'..='\u{FDEF}').contains(&c)
}

/// Whether an axis-aligned box (x0, y0, x1, y1) has the shape of a mark: long for its width.
/// Whether it is small enough to be one depends on the text it stands with (see
/// [`is_mark`]), which the content walk does not know yet when the box is drawn.
pub(crate) fn mark_shaped(b: [f32; 4]) -> bool {
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let (short, long) = (w.min(h), w.max(h));
    short > 0.0 && long >= MARK_RATIO * short
}

/// Whether a box is thin enough for a mark with text of `size`: at most [`MARK_THIN`] sizes
/// thick, or [`MARK_PEN`] points, whichever is more.
fn thin_for(b: [f32; 4], size: f32) -> bool {
    let short = (b[2] - b[0]).min(b[3] - b[1]);
    short <= MARK_PEN.max(MARK_THIN * size)
}

/// Whether a box is a mark with text of `size`: of [`mark_shaped`] shape, thin and short for
/// that size.
pub(crate) fn is_mark(b: [f32; 4], size: f32) -> bool {
    let long = (b[2] - b[0]).max(b[3] - b[1]);
    mark_shaped(b) && thin_for(b, size) && long <= MARK_LONG * size
}

/// The text size a page's marks are measured by when they stand by no digits (dashes of a
/// rule, the marks of an empty field): the median size of its visible digits, which are the
/// values a form prints, or of all its visible glyphs when it has no digits. `None` on a page
/// without visible text: there no box is a mark.
pub(crate) fn page_size(glyphs: &[Gl], text: &str) -> Option<f32> {
    let bytes = text.as_bytes();
    let visible = |g: &&Gl| !g.invisible && g.layer.is_none() && g.size > 0.0;
    let digit = |g: &&Gl| g.len == 1 && bytes.get(g.start as usize).is_some_and(u8::is_ascii_digit);
    let mut sizes: Vec<f32> = glyphs
        .iter()
        .filter(visible)
        .filter(digit)
        .map(|g| g.size)
        .collect();
    if sizes.is_empty() {
        sizes = glyphs.iter().filter(visible).map(|g| g.size).collect();
    }
    if sizes.is_empty() {
        return None;
    }
    let mid = sizes.len() / 2;
    let (_, m, _) = sizes.select_nth_unstable_by(mid, f32::total_cmp);
    Some(*m)
}

/// The separator characters the file's own text shows, each set only by clear evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SeparatorStyle {
    pub decimal: Option<char>,
    pub date: Option<char>,
    pub group: Option<char>,
}

/// How many numbers written as text show the character a kind of separator was set by
/// (`won`), of all that show one of its characters (`of`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Vote {
    pub won: u32,
    pub of: u32,
}

/// The votes behind each character of a [`SeparatorStyle`]; a kind not set has none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Votes {
    pub decimal: Vote,
    pub group: Vote,
    pub date: Vote,
}

/// The separator characters `texts` show, and the votes for them.
#[cfg(test)]
pub(crate) fn style_of(texts: &[&str]) -> (SeparatorStyle, Votes) {
    let mut evidence = Evidence::default();
    texts.iter().for_each(|t| evidence.scan(t));
    evidence.style()
}

/// The separator characters the visible text of `pages` shows, and the votes for them, as
/// [`style_of`] gives them for texts: each line of runs is scanned whole (a number may be
/// cut into runs), with no copy of the whole text.
pub(crate) fn style_in(pages: &[PageItems]) -> (SeparatorStyle, Votes) {
    let mut evidence = Evidence::default();
    let mut line = String::new();
    for items in pages {
        let mut last = None;
        for r in &items.runs {
            if last != Some(r.line) {
                evidence.scan(&line);
                line.clear();
            }
            line.push_str(&r.text);
            last = Some(r.line);
        }
        evidence.scan(&line);
        line.clear();
    }
    evidence.style()
}

/// The dates and amounts written as text, counted by their separators, a text at a time.
/// Dates are found first and left out of the amounts, so `01.02.2026` never counts as a
/// point decimal. A number followed by `%` is a rate, not an amount: rates are often written
/// in another style than the amounts of the same file, so they do not vote.
#[derive(Debug, Default)]
struct Evidence {
    /// Amounts with `,` and with `.` before their last two digits.
    decimal: [u32; 2],
    /// Amounts with groups of three digits after `,` and after `.`, by the decimal character
    /// of the amount (`,` then `.`): only the amounts with the decimal character the vote
    /// sets vote for the group character.
    group: [[u32; 2]; 2],
    /// Dates with `.`, `/` and `-`.
    date: [u32; 3],
}

const AMOUNT_SEPS: [char; 2] = [',', '.'];
const DATE_SEPS: [char; 3] = ['.', '/', '-'];

/// Whether `rest` (the text after a number) starts with `%`, after at most one space of any
/// width (`1,5 %` is how German writes a rate).
fn rate_follows(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        Some('%') => true,
        Some(' ' | '\u{a0}' | '\u{2009}' | '\u{202f}') => chars.next() == Some('%'),
        _ => false,
    }
}

impl Evidence {
    fn scan(&mut self, text: &str) {
        let dates = scan::dates(text, YEARS);
        for d in &dates {
            if let Some(k) = DATE_SEPS.iter().position(|&c| c == d.sep) {
                self.date[k] += 1;
            }
        }
        let skip: Vec<(usize, usize)> = dates.iter().map(|d| (d.start, d.end)).collect();
        for a in scan::amounts(text, &skip) {
            if rate_follows(&text[a.end..]) {
                continue;
            }
            let Some(d) = AMOUNT_SEPS.iter().position(|&c| c == a.decimal) else {
                continue;
            };
            self.decimal[d] += 1;
            if let Some(g) = a
                .group
                .and_then(|g| AMOUNT_SEPS.iter().position(|&c| c == g))
            {
                self.group[d][g] += 1;
            }
        }
    }

    /// Per kind, the character the vote of [`vote`] gives.
    /// The group character is voted for only by the amounts with the decimal character that
    /// won, and is never that character: the minority of a mixed decimal vote cannot set it.
    fn style(&self) -> (SeparatorStyle, Votes) {
        let decimal = vote(&self.decimal, &AMOUNT_SEPS);
        let group = decimal
            .and_then(|(d, _)| {
                let k = AMOUNT_SEPS.iter().position(|&c| c == d)?;
                vote(&self.group[k], &AMOUNT_SEPS)
            })
            .filter(|(g, _)| decimal.is_some_and(|(d, _)| d != *g));
        let date = vote(&self.date, &DATE_SEPS);
        let style = SeparatorStyle {
            decimal: decimal.map(|(c, _)| c),
            date: date.map(|(c, _)| c),
            group: group.map(|(c, _)| c),
        };
        let votes = Votes {
            decimal: decimal.map(|(_, v)| v).unwrap_or_default(),
            group: group.map(|(_, v)| v).unwrap_or_default(),
            date: date.map(|(_, v)| v).unwrap_or_default(),
        };
        (style, votes)
    }
}

/// The character of `chars` the numbers vote for with `counts`: the only one shown, at any
/// count; or, when several are shown, one with at least 3 votes and at least 3 times the
/// votes of all the others together. `None` when the vote is not that clear.
fn vote(counts: &[u32], chars: &[char]) -> Option<(char, Vote)> {
    let of: u32 = counts.iter().sum();
    let (&won, &c) = counts.iter().zip(chars).max_by_key(|(n, _)| **n)?;
    let others = of - won;
    let clear = won > 0 && (others == 0 || (won >= 3 && won >= 3 * others));
    clear.then_some((c, Vote { won, of }))
}

/// What the placeholders of one page became.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub decimal: u32,
    pub date: u32,
    pub group: u32,
    /// The numbers whose separators are not read: the number by its printed digits and the
    /// reason.
    pub unread: Vec<String>,
    /// Unread numbers past those `unread` names.
    pub more: u32,
}

/// Puts the separators of `style` in place of the placeholders of the pages' text, or
/// removes them, before the pages are laid out. Returns what each page with placeholders
/// got.
///
/// Placeholders are put only between visible glyphs, so only the visible runs (`runs`) and
/// the quotes taken from them (`quotes`) hold them. The runs of one line are read together,
/// so a number cut into two runs is read as the quote over it reads it. The text nobody sees
/// (`invisible`, `hidden_layers`, `hidden_layer`) is cleaned of them too, so no placeholder
/// can reach the output whatever the glyphs were.
pub(crate) fn resolve(pages: &mut [PageItems], style: &SeparatorStyle) -> Vec<(u32, Resolved)> {
    let mut out = Vec::new();
    let mut line = String::new();
    for page in pages.iter_mut() {
        let mut got = Resolved::default();
        let mut any = false;
        let mut seen = |digits: &str, read: Result<Vec<Kind>, &'static str>| match read {
            Ok(kinds) => {
                for k in kinds {
                    match k {
                        Kind::Decimal => got.decimal += 1,
                        Kind::Group => got.group += 1,
                        Kind::Date => got.date += 1,
                    }
                }
            }
            Err(why) if got.unread.len() < MAX_UNREAD => got
                .unread
                .push(format!("the number {digits} is kept without it, as {why}")),
            Err(_) => got.more += 1,
        };
        let runs = &mut page.runs;
        let mut start = 0;
        while start < runs.len() {
            let at = runs[start].line;
            let end = start + runs[start..].partition_point(|r| r.line == at);
            let held = &mut runs[start..end];
            if held.iter().any(|r| r.text.contains(is_placeholder)) {
                any = true;
                line.clear();
                held.iter().for_each(|r| line.push_str(&r.text));
                let mut decided = decide(&line, style, &mut seen).into_iter();
                for r in held {
                    if r.text.contains(is_placeholder) {
                        r.text = apply(&r.text, &mut decided);
                    }
                }
            }
            start = end;
        }
        for quote in &mut page.quotes {
            if let Some(text) = quote.text.as_mut().filter(|t| t.contains(is_placeholder)) {
                *text = fix(text, style, &mut |_, _| {});
            }
        }
        let hidden = page.invisible.iter_mut().chain(
            page.hidden_layers
                .iter_mut()
                .flat_map(|(_, l)| l.iter_mut()),
        );
        for run in hidden {
            strip(&mut run.text);
        }
        page.hidden_layer.iter_mut().for_each(strip);
        if any {
            out.push((page.page, got));
        }
    }
    out
}

/// Drops every placeholder from `text`.
fn strip(text: &mut String) {
    if text.contains(is_placeholder) {
        text.retain(|c| !is_placeholder(c));
    }
}

/// What a drawn separator is, from the digit groups of the number it cuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Decimal,
    Group,
    Date,
}

/// The kinds of the separators between digit groups of `lens` digits: one before the last
/// two digits, or before the last one, is a decimal separator, and the ones before it cut
/// groups of three after one to three digits; 2-2-2, 2-2-4 and 4-2-2 are dates.
fn kinds(lens: &[usize]) -> Option<Vec<Kind>> {
    if matches!(lens, [2, 2, 2] | [2, 2, 4] | [4, 2, 2]) {
        return Some(vec![Kind::Date; 2]);
    }
    let (&last, rest) = lens.split_last()?;
    let first = *rest.first()?;
    let groups = &rest[1..];
    let fits = (last == 2 || last == 1)
        && first > 0
        && (groups.is_empty() || first <= 3)
        && groups.iter().all(|&n| n == 3);
    fits.then(|| {
        let mut k = vec![Kind::Group; groups.len()];
        k.push(Kind::Decimal);
        k
    })
}

/// What is told of each number with placeholders: its printed digits and the kinds of its
/// separators, or why they are not read.
type Seen<'s> = dyn FnMut(&str, Result<Vec<Kind>, &'static str>) + 's;

/// Why the separators of a number are not read.
const NO_KIND: &str = "its digit groups fit no amount and no date";
const NO_DECIMAL: &str =
    "the amounts written as text in this file do not show one decimal separator";
const NO_GROUP: &str = "the amounts written as text in this file do not show one group separator";
const NO_DATE: &str = "the dates written as text in this file do not show one date separator";
const DATE_ORDER: &str =
    "the dates written as text in this file put day, month and year in another order";
const TURNED: &str = "its digits do not read in the direction of the page's text";
const LONG: &str = "a bar between its digits is too long for a separator of their size";

/// The characters for the separators of a number of digit groups `lens`, with their kinds,
/// or why they are not read. A date with the year first takes only `-`, and one with the
/// year last only `.` or `/`, as the dates written as text set them.
fn read(lens: &[usize], style: &SeparatorStyle) -> Result<(Vec<Kind>, Vec<char>), &'static str> {
    let kinds = kinds(lens).ok_or(NO_KIND)?;
    let year_first = lens[0] == 4;
    let chars = kinds
        .iter()
        .map(|k| match k {
            Kind::Decimal => style.decimal.ok_or(NO_DECIMAL),
            Kind::Group => style.group.ok_or(NO_GROUP),
            Kind::Date => match style.date {
                None => Err(NO_DATE),
                Some(c) if (c == '-') == year_first => Ok(c),
                Some(_) => Err(DATE_ORDER),
            },
        })
        .collect::<Result<Vec<char>, _>>()?;
    Ok((kinds, chars))
}

/// What each placeholder of `text` becomes, in order: a separator, or nothing when the
/// number's separators are not read. `seen` is told each number with placeholders: its
/// printed digits and its kinds, or why it is not read. Placeholders next to each other
/// count as one: a text drawn twice on top of itself (a fake bold) gets one from each copy,
/// and where both copies meet in one text (a quote) they stand together.
fn decide(text: &str, style: &SeparatorStyle, seen: &mut Seen<'_>) -> Vec<Option<char>> {
    let mut out = Vec::new();
    // The number being read: its digits, the length of each digit group, how many
    // placeholders each group ends with, and whether one of them is on turned digits.
    let mut digits = String::new();
    let mut lens: Vec<usize> = vec![0];
    let mut seps: Vec<usize> = Vec::new();
    let mut turned = false;
    let mut long = false;
    let mut flush = |digits: &mut String,
                     lens: &mut Vec<usize>,
                     seps: &mut Vec<usize>,
                     turned: &mut bool,
                     long: &mut bool,
                     out: &mut Vec<Option<char>>| {
        if !seps.is_empty() {
            let read = if *turned {
                Err(TURNED)
            } else if *long {
                Err(LONG)
            } else {
                read(lens, style)
            };
            match read {
                Ok((kinds, chars)) => {
                    for (c, &n) in chars.iter().zip(seps.iter()) {
                        out.push(Some(*c));
                        out.extend(std::iter::repeat_n(None, n - 1));
                    }
                    seen(digits, Ok(kinds));
                }
                Err(why) => {
                    out.extend(std::iter::repeat_n(None, seps.iter().sum()));
                    seen(digits, Err(why));
                }
            }
        }
        digits.clear();
        *lens = vec![0];
        seps.clear();
        *turned = false;
        *long = false;
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            if seps.len() == lens.len() {
                lens.push(0);
            }
            digits.push(c);
            *lens.last_mut().expect("a group") += 1;
        } else if is_sep(c) {
            turned |= c == SEP_TURNED;
            long |= c == SEP_LONG;
            if seps.len() == lens.len() {
                *seps.last_mut().expect("a placeholder") += 1;
            } else {
                seps.push(1);
            }
        } else {
            flush(
                &mut digits,
                &mut lens,
                &mut seps,
                &mut turned,
                &mut long,
                &mut out,
            );
        }
    }
    // A number that ends with placeholders: they cut off an empty group.
    flush(
        &mut digits,
        &mut lens,
        &mut seps,
        &mut turned,
        &mut long,
        &mut out,
    );
    out
}

/// `text` with each placeholder replaced by what `decided` says, in order.
fn apply(text: &str, decided: &mut dyn Iterator<Item = Option<char>>) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_sep(c) {
            out.extend(decided.next().flatten());
        } else if !is_placeholder(c) {
            out.push(c);
        }
    }
    out
}

/// `text` with the placeholders of each number replaced by the separators of `style`, or
/// removed (see [`decide`]).
fn fix(text: &str, style: &SeparatorStyle, seen: &mut Seen<'_>) -> String {
    let decided = decide(text, style, seen);
    apply(text, &mut decided.into_iter())
}

/// The `found-by` note of a page whose separators `got` says, with the `votes` of
/// [`style_in`]; `None` when no separator was written. Only the evidence of the kinds the
/// page used is named, and a vote that was not unanimous says so (`11 of 12 amounts`).
pub(crate) fn note(got: &Resolved, style: &SeparatorStyle, votes: &Votes) -> Option<String> {
    let kinds = [
        (got.decimal, "decimal", style.decimal),
        (got.group, "group", style.group),
        (got.date, "date", style.date),
    ];
    let used: Vec<(u32, &str, char)> = kinds
        .iter()
        .filter(|(n, _, _)| *n > 0)
        .filter_map(|&(n, name, c)| Some((n, name, c?)))
        .collect();
    if used.is_empty() {
        return None;
    }
    let total: u32 = used.iter().map(|u| u.0).sum();
    let counts: Vec<String> = used
        .iter()
        .map(|&(n, name, _)| format!("{n} {name} separator{}", plural(n)))
        .collect();
    let chars: Vec<String> = used.iter().map(|u| format!("\"{}\"", u.2)).collect();
    let (verb, it) = if total == 1 {
        ("is a drawn mark", "it is")
    } else {
        ("are drawn marks", "they are")
    };
    let words = |v: Vote, noun: &str| {
        if v.won == v.of {
            format!("{} {noun}{}", v.won, plural(v.won))
        } else {
            format!("{} of {} {noun}s", v.won, v.of)
        }
    };
    let mut evidence = Vec::new();
    if got.decimal > 0 {
        evidence.push(words(votes.decimal, "amount"));
    } else if got.group > 0 {
        evidence.push(words(votes.group, "amount"));
    }
    if got.date > 0 {
        evidence.push(words(votes.date, "date"));
    }
    Some(format!(
        "Found by: {} on this page {verb}, not text; {it} written as {} as in {} written as \
         text in this file.",
        listed(&counts),
        listed(&chars),
        evidence.join(" and ")
    ))
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `items` joined with commas and a last `and`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The side of a grid cell of mark middles, in points.
const CELL: f32 = 4.0;

/// The grid cell (row, column) of the middle of a mark.
fn cell_of(m: &[f32; 4]) -> (i32, i32) {
    let cell = |v: f32| (v / CELL).floor() as i32;
    (cell((m[1] + m[3]) / 2.0), cell((m[0] + m[2]) / 2.0))
}

/// Puts a placeholder between two neighbouring digit glyphs of one baseline (`glyphs[i]` and
/// `glyphs[i + 1]`, visible, each with the text of one ASCII digit of `text`, in the same
/// direction) when a mark's middle lies between their middles (origin plus half the
/// advance) along the baseline, its bottom within 0.3 font sizes of the baseline and its
/// height at most half the font size, measured across it. Digits that read in `turn`, the
/// direction the page is read in, get [`SEP`]; other digits get [`SEP_TURNED`], whose number
/// is listed as not read. A mark must also be one for the digits' size ([`is_mark`]).
/// `marks` are in the frame of the glyphs and come back sorted by their grid cell; the marks
/// that put a placeholder go into `used`. False when the marks near the digits were too many
/// to test them all ([`MARK_WORK`]).
pub(crate) fn insert(
    glyphs: &mut Vec<Gl>,
    text: &mut String,
    marks: &mut [[f32; 4]],
    turn: Turn,
    used: &mut Vec<[f32; 4]>,
) -> bool {
    if marks.is_empty() || glyphs.len() < 2 {
        return true;
    }
    marks.sort_by_key(cell_of);
    let bytes = text.as_bytes();
    let digit = |g: &Gl| {
        !g.invisible
            && g.layer.is_none()
            && g.actual.is_none()
            && !g.joined
            && g.len == 1
            && bytes.get(g.start as usize).is_some_and(u8::is_ascii_digit)
    };
    let mut after: Vec<(usize, char, (f32, f32))> = Vec::new();
    let mut work = 0usize;
    let mut whole = true;
    'pairs: for i in 0..glyphs.len() - 1 {
        let (a, b) = (&glyphs[i], &glyphs[i + 1]);
        if !digit(a) || !digit(b) {
            continue;
        }
        let size = a.size.max(0.1);
        let same_way = (a.dx - b.dx).abs() <= 0.05 && (a.dy - b.dy).abs() <= 0.05;
        if !same_way || (b.size - a.size).abs() > 0.05 * size {
            continue;
        }
        // The glyph's own frame: along the baseline and up from it. Vertical writing goes on
        // down the column, across its baseline.
        let (d, up) = if a.vertical {
            ((a.dy, -a.dx), (a.dx, a.dy))
        } else {
            ((a.dx, a.dy), (-a.dy, a.dx))
        };
        let along = |x: f32, y: f32| (x - a.x) * d.0 + (y - a.y) * d.1;
        let across = |x: f32, y: f32| (x - a.x) * up.0 + (y - a.y) * up.1;
        if across(b.x, b.y).abs() > 0.1 * size {
            continue;
        }
        let step = along(b.x, b.y);
        let (from, to, t0, t1) = if a.vertical {
            // Where a glyph of vertical writing has its origin depends on the font, so the
            // mark is looked for around the step from one digit to the next, across the
            // column.
            if step <= 0.0 || step > 2.0 * size {
                continue;
            }
            (0.25 * step, 1.25 * step, -0.6 * size, 0.6 * size)
        } else {
            let (adv_a, adv_b) = (a.adv.unwrap_or(0.5 * size), b.adv.unwrap_or(0.5 * size));
            let (from, to) = (adv_a / 2.0, step + adv_b / 2.0);
            // Neighbours: the second digit follows the first within a font size.
            if to <= from || step - adv_a > size {
                continue;
            }
            // Where a mark's middle may be: between the digit middles, from 0.3 sizes under
            // the baseline to the middle of the highest bar that fits (see `fits`).
            (from, to, -0.3 * size, 0.55 * size)
        };
        let corners = [(from, t0), (from, t1), (to, t0), (to, t1)]
            .map(|(s, t)| (a.x + s * d.0 + t * up.0, a.y + s * d.1 + t * up.1));
        let lo = |k: fn(&(f32, f32)) -> f32| corners.iter().map(k).fold(f32::MAX, f32::min);
        let hi = |k: fn(&(f32, f32)) -> f32| corners.iter().map(k).fold(f32::MIN, f32::max);
        let cell = |v: f32| (v / CELL).floor() as i32;
        let (rows, cols) = (
            cell(lo(|c| c.1))..=cell(hi(|c| c.1)),
            cell(lo(|c| c.0))..=cell(hi(|c| c.0)),
        );
        let readable = Turn::of(a.dx, a.dy) == Some(turn) && !a.vertical;
        let ahead = if a.vertical {
            0.1 * step
        } else {
            0.75 * a.adv.unwrap_or(0.5 * size)
        };
        // The placeholder stands just after the first digit, on its way to the second one, so
        // it stays between them in reading order.
        let at = (a.x + ahead * d.0, a.y + ahead * d.1);
        // A bar that is a mark in all but its length, if no mark stands between the two.
        let mut long = false;
        for row in rows {
            work += 1;
            let first = marks.partition_point(|m| cell_of(m) < (row, *cols.start()));
            for m in &marks[first..] {
                work += 1;
                if work > MARK_WORK {
                    whole = false;
                    break 'pairs;
                }
                if cell_of(m) > (row, *cols.end()) {
                    break;
                }
                let s = along((m[0] + m[2]) / 2.0, (m[1] + m[3]) / 2.0);
                let ts = [(m[0], m[1]), (m[0], m[3]), (m[2], m[1]), (m[2], m[3])]
                    .map(|(x, y)| across(x, y));
                let bottom = ts.iter().copied().fold(f32::MAX, f32::min);
                let top = ts.iter().copied().fold(f32::MIN, f32::max);
                let fits = if a.vertical {
                    // Anywhere across the column: such a number is never read anyway.
                    bottom >= t0 && top <= t1
                } else {
                    bottom.abs() <= 0.3 * size && top - bottom <= 0.5 * size
                };
                if s > from && s < to && fits {
                    if is_mark(*m, size) {
                        after.push((i, if readable { SEP } else { SEP_TURNED }, at));
                        used.push(*m);
                        continue 'pairs;
                    }
                    // Only a bar that stands across the baseline, as a separator does: a rule
                    // along it, under the digits, is no bar between them.
                    let along_m = [(m[0], m[1]), (m[0], m[3]), (m[2], m[1]), (m[2], m[3])]
                        .map(|(x, y)| along(x, y));
                    let wide = along_m.iter().copied().fold(f32::MIN, f32::max)
                        - along_m.iter().copied().fold(f32::MAX, f32::min);
                    long |= !a.vertical
                        && MARK_RATIO * wide <= top - bottom
                        && mark_shaped(*m)
                        && thin_for(*m, size);
                }
            }
        }
        if long {
            after.push((i, if readable { SEP_LONG } else { SEP_TURNED }, at));
        }
    }
    let (Some(&(last, ..)), Ok(at)) = (after.last(), u32::try_from(text.len())) else {
        return whole;
    };
    // Each placeholder has its text once at the end of the page's text.
    let all = [SEP, SEP_TURNED, SEP_LONG];
    all.iter().for_each(|&c| text.push(c));
    let len = SEP.len_utf8() as u32;
    let start = |c: char| at + len * all.iter().position(|&p| p == c).unwrap_or(0) as u32;
    // Moved up from the back, each placeholder goes in right after its first digit.
    let n = glyphs.len();
    glyphs.resize(n + after.len(), glyphs[last]);
    let (mut src, mut dst) = (n, glyphs.len());
    for &(i, c, (x, y)) in after.iter().rev() {
        while src > i + 1 {
            src -= 1;
            dst -= 1;
            glyphs[dst] = glyphs[src];
        }
        dst -= 1;
        // With no advance, so it joins the digits without a space.
        glyphs[dst] = Gl {
            start: start(c),
            len,
            x,
            y,
            adv: Some(0.0),
            ..glyphs[i]
        };
    }
    whole
}

/// A divider stands over a line of digits when its foot lies at least this many font sizes
/// over the baseline: over the digits (a digit is at most about three quarters of the size
/// high, so the sides of boxes drawn around each digit, which reach under the baseline, are
/// no dividers).
const DIVIDER_FOOT: f32 = 0.75;

/// The width of a character of a data font, as a share of its size: how near a value starts
/// to its field's edge.
const CHAR: f32 = 0.6;

/// Glyphs the search for divided numbers may look at on one page; past them no more number is
/// cut.
const CUT_WORK: usize = 1_000_000;

/// Puts a word space at a divider of a row of fields inside a number whose drawn separators
/// read as no number (see [`kinds`]): a value that fills its field and touches the value of
/// the next field, its separator drawn as a mark. Each piece of evidence is needed, else the
/// number stays whole and its separators unread:
/// - the divider is one of a frame of fields between rules (see
///   [`crate::tables::field_frames`]) the number stands in, hanging from the frame's top over
///   the digits (see [`DIVIDER_FOOT`]) or rising from its bottom past them, between the
///   middles of two neighbouring digits, and the only divider of the frame in the number;
/// - the frame holds smaller text over the number (its labels);
/// - the digits before the divider, with their separators, read as a number, and the whole
///   does not read as a whole number in groups of three (a thousands value, or a value of three
///   decimals, which only lacks its kind);
/// - the number starts at its field's start, the digits after the divider start within a
///   character of it, and nothing else stands in the next field on the number's line.
///
/// Only the first divider of a number cuts. Runs only in the direction of the page's text,
/// `turn`: on a page read turned a quarter, the digits, the rules and the dividers are looked
/// at in the frame its text reads in, as on an upright page (a page upside down is not cut).
pub(crate) fn cut_at_dividers(glyphs: &mut Vec<Gl>, text: &mut String, rules: &[Rule], turn: Turn) {
    if !text.contains(SEP) || rules.is_empty() {
        return;
    }
    // After which glyph a space goes, and where on the page.
    // A page read upside down has no turned frame here: its digits are not cut.
    if turn == Turn::Half {
        return;
    }
    let spaces: Vec<(usize, (f32, f32))> = if turn == Turn::None {
        divider_spaces(glyphs, text, rules)
            .into_iter()
            .map(|(k, x)| (k, (x, glyphs[k].y)))
            .collect()
    } else {
        let view: Vec<Gl> = glyphs
            .iter()
            .map(|g| {
                let ((x, y), (dx, dy)) = (turn.apply(g.x, g.y), turn.apply(g.dx, g.dy));
                Gl { x, y, dx, dy, ..*g }
            })
            .collect();
        let turned: Vec<Rule> = rules
            .iter()
            .map(|r| {
                let (a, b) = (turn.apply(r.x0, r.y0), turn.apply(r.x1, r.y1));
                Rule {
                    x0: a.0.min(b.0),
                    y0: a.1.min(b.1),
                    x1: a.0.max(b.0),
                    y1: a.1.max(b.1),
                    kind: r.kind,
                }
            })
            .collect();
        // Back on the page: as far along the digit's baseline from it as in the frame.
        divider_spaces(&view, text, &turned)
            .into_iter()
            .map(|(k, x)| {
                let (g, along) = (&glyphs[k], x - view[k].x);
                (k, (g.x + along * g.dx, g.y + along * g.dy))
            })
            .collect()
    };
    if spaces.is_empty() {
        return;
    }
    let Ok(at) = u32::try_from(text.len()) else {
        return;
    };
    text.push(' ');
    // Moved up from the back, each space goes in right after its digit, with no advance.
    let n = glyphs.len();
    glyphs.resize(n + spaces.len(), glyphs[0]);
    let (mut src, mut dst) = (n, glyphs.len());
    for &(k, (x, y)) in spaces.iter().rev() {
        while src > k + 1 {
            src -= 1;
            dst -= 1;
            glyphs[dst] = glyphs[src];
        }
        dst -= 1;
        glyphs[dst] = Gl {
            start: at,
            len: 1,
            x,
            y,
            adv: Some(0.0),
            ..glyphs[k]
        };
    }
}

/// The spaces [`cut_at_dividers`] puts in on a page read upright: after which glyph each goes,
/// and at which x (the divider's).
fn divider_spaces(glyphs: &[Gl], text: &str, rules: &[Rule]) -> Vec<(usize, f32)> {
    let upright = |r: &Rule| r.kind == RuleKind::Line && (r.x1 - r.x0).abs() < (r.y1 - r.y0).abs();
    if !rules.iter().any(upright) {
        return Vec::new();
    }
    let frames = crate::tables::field_frames(rules);
    if frames.iter().all(|f| f.dividers.is_empty()) {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let char_at = |g: &Gl| text[g.start as usize..].chars().next();
    let digit = |g: &Gl| {
        !g.invisible
            && g.layer.is_none()
            && g.len == 1
            && bytes.get(g.start as usize).is_some_and(u8::is_ascii_digit)
    };
    let placeholder = |g: &Gl| char_at(g) == Some(SEP);
    let flat = |g: &Gl| !g.vertical && g.dx > 0.99 && g.dy.abs() < 0.01;
    let ink = |g: &Gl| {
        !g.invisible
            && g.layer.is_none()
            && !placeholder(g)
            && char_at(g).is_some_and(|c| !c.is_whitespace())
    };
    // The glyphs with ink by baseline, to look at a line or a band of the page.
    let mut by_y: Vec<usize> = (0..glyphs.len()).filter(|&k| ink(&glyphs[k])).collect();
    by_y.sort_by(|&a, &b| glyphs[a].y.total_cmp(&glyphs[b].y));
    let work = std::cell::Cell::new(0usize);
    // Whether a glyph of `by_y` with a baseline in `lo..hi` passes `test`; looked at glyphs
    // count as work.
    let any = |lo: f32, hi: f32, test: &dyn Fn(&Gl, usize) -> bool| {
        let from = by_y.partition_point(|&k| glyphs[k].y < lo);
        for &k in &by_y[from..] {
            let g = &glyphs[k];
            if g.y > hi {
                break;
            }
            work.set(work.get() + 1);
            if work.get() > CUT_WORK || test(g, k) {
                return true;
            }
        }
        false
    };
    // After which glyph a space goes, and where.
    let mut spaces: Vec<(usize, f32)> = Vec::new();
    let mut i = 0;
    while i < glyphs.len() {
        if !(digit(&glyphs[i]) && flat(&glyphs[i])) {
            i += 1;
            continue;
        }
        // The number from `i`: digits and placeholders on one baseline, each after the last.
        let mut j = i + 1;
        while j < glyphs.len() {
            let (p, g) = (&glyphs[j - 1], &glyphs[j]);
            let size = p.size.max(0.1);
            let end = p.x + p.adv.unwrap_or(0.5 * size);
            let next = (digit(g) || placeholder(g))
                && flat(g)
                && (g.y - p.y).abs() <= 0.1 * size
                && g.x - end <= 0.5 * size
                && g.x >= p.x - 0.1 * size;
            if !next {
                break;
            }
            j += 1;
        }
        let number = i..j;
        i = j;
        let mut lens: Vec<usize> = vec![0];
        for g in &glyphs[number.clone()] {
            if placeholder(g) {
                lens.push(0);
            } else {
                *lens.last_mut().expect("a group") += 1;
            }
        }
        let thousands =
            (1..=3).contains(&lens[0]) && lens.len() >= 2 && lens[1..].iter().all(|&n| n == 3);
        if lens.len() < 2 || kinds(&lens).is_some() || thousands {
            continue;
        }
        let (head, tail) = (&glyphs[number.start], &glyphs[number.end - 1]);
        let size = head.size.max(0.1);
        let ch = CHAR * size;
        let line = (head.y, tail.x + tail.adv.unwrap_or(0.5 * size));
        // The frames the number stands in, with labels over it.
        let held: Vec<&crate::tables::Frame> = frames
            .iter()
            .filter(|f| f.bottom < line.0 && line.0 < f.top && f.x0 < head.x && line.1 < f.x1 + ch)
            .collect();
        // The first divider between two digits after a separator where the digits before it
        // read as a number.
        let mut prefix: Vec<usize> = vec![0];
        let ids: Vec<usize> = number.clone().collect();
        for w in ids.windows(2) {
            let (a, b) = (&glyphs[w[0]], &glyphs[w[1]]);
            if placeholder(a) {
                prefix.push(0);
                continue;
            }
            *prefix.last_mut().expect("a group") += 1;
            if placeholder(b) || prefix.len() < 2 || kinds(&prefix).is_none() {
                continue;
            }
            let (from, to) = (
                a.x + a.adv.unwrap_or(0.5 * size) / 2.0,
                b.x + b.adv.unwrap_or(0.5 * size) / 2.0,
            );
            // A divider hangs from the frame's top over the digits or rises from its bottom
            // past them; no other divider of the frame stands in the number (the sides of
            // boxes drawn around each digit).
            let at = held.iter().find_map(|f| {
                let k = f
                    .dividers
                    .iter()
                    .zip(&f.spans)
                    .position(|(&x, &(low, high))| {
                        let over = a.y + DIVIDER_FOOT * size;
                        let hangs = (high - f.top).abs() <= DIVIDER_FOOT * size && low >= over;
                        let rises = (low - f.bottom).abs() <= DIVIDER_FOOT * size && high >= over;
                        x > from && x < to && (hangs || rises)
                    })?;
                let inside = f
                    .dividers
                    .iter()
                    .filter(|&&d| d > head.x + ch / 2.0 && d < line.1 - ch / 2.0)
                    .count();
                (inside == 1).then_some((*f, k))
            });
            let Some((f, k)) = at else {
                continue;
            };
            let x = f.dividers[k];
            let start = if k == 0 { f.x0 } else { f.dividers[k - 1] };
            let end = f.dividers.get(k + 1).copied().unwrap_or(f.x1);
            let fits = (head.x - start).abs() <= ch && b.x - x <= ch;
            // Smaller text over the number in the frame: its labels.
            let labelled = fits
                && any(line.0 + 0.5 * size, f.top, &|g, _| {
                    g.size * LABEL_STEP <= size && g.x > f.x0 && g.x < f.x1
                });
            // Nothing else in the next field on the number's line.
            let alone = labelled
                && !any(line.0 - 0.3 * size, line.0 + 0.3 * size, &|g, k| {
                    let mid = g.x + g.adv.unwrap_or(0.5 * g.size) / 2.0;
                    !number.contains(&k) && mid > x && mid < end
                });
            if alone && work.get() <= CUT_WORK {
                spaces.push((w[0], x));
            }
            break;
        }
    }
    spaces
}

/// Marks of one line lie within this share of their thickness of each other across it, and
/// differ in thickness by at most this share (0.1 points for dashes of 0.4 points): relative
/// to the pen, so that a rule drawn thicker or thinner joins the same.
const DASH_SNAP: f32 = 0.25;

/// Lines drawn as rows of marks: dashed or dotted rules of filled dashes, which are marks by
/// shape. Marks of one direction on one line (their middles within [`DASH_SNAP`] of their
/// thickness across it, the same thickness), each at most its own length after the one before
/// (or over it, as a copy), make one line when at least three of them reach past the ones
/// before. The marks of such lines are taken out of `marks`: they are rules, not separators.
pub(crate) fn dashed_rules(marks: &mut Vec<[f32; 4]>) -> Vec<Rule> {
    // Along the long side: (upright, across, start, end, thickness).
    let flat = |m: &[f32; 4]| {
        let (w, h) = (m[2] - m[0], m[3] - m[1]);
        if w >= h {
            (false, (m[1] + m[3]) / 2.0, m[0], m[2], h)
        } else {
            (true, (m[0] + m[2]) / 2.0, m[1], m[3], w)
        }
    };
    marks.sort_by(|a, b| {
        let (fa, fb) = (flat(a), flat(b));
        fa.0.cmp(&fb.0).then(fa.1.total_cmp(&fb.1))
    });
    let mut out = Vec::new();
    let mut dashed = vec![false; marks.len()];
    let mut i = 0;
    while i < marks.len() {
        // One line: marks of one direction whose middles lie within the snap of the first,
        // so a little float noise across it neither loses nor splits it.
        let (upright, across, _, _, thick) = flat(&marks[i]);
        let n = marks[i..]
            .iter()
            .take_while(|m| {
                let (u, c, ..) = flat(m);
                u == upright && c - across <= DASH_SNAP * thick
            })
            .count();
        let line = &mut marks[i..i + n];
        line.sort_by(|a, b| flat(a).2.total_cmp(&flat(b).2));
        let mut k = 0;
        while k < n {
            let (_, _, start, mut end, thick) = flat(&line[k]);
            let mut sum = flat(&line[k]).1;
            let mut j = k + 1;
            // Dashes that reach past the ones before: a copy drawn over a dash (an edge two
            // boxes share, drawn by each) goes on the line without counting.
            let mut ahead = 1;
            while let Some(m) = line.get(j) {
                let (_, c, s, e, t) = flat(m);
                let gap = s - end;
                if (t - thick).abs() > DASH_SNAP * thick.max(t) || gap > e - s {
                    break;
                }
                if e > end {
                    end = e;
                    ahead += 1;
                }
                sum += c;
                j += 1;
            }
            if ahead >= 3 {
                let at = sum / (j - k) as f32;
                dashed[i + k..i + j].iter_mut().for_each(|d| *d = true);
                out.push(if upright {
                    Rule {
                        x0: at,
                        y0: start,
                        x1: at,
                        y1: end,
                        kind: RuleKind::Line,
                    }
                } else {
                    Rule {
                        x0: start,
                        y0: at,
                        x1: end,
                        y1: at,
                        kind: RuleKind::Line,
                    }
                });
            }
            k = j;
        }
        i += n;
    }
    let mut keep = dashed.iter().map(|d| !d);
    marks.retain(|_| keep.next().unwrap_or(true));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vote_of(won: u32, of: u32) -> Vote {
        Vote { won, of }
    }

    #[test]
    fn dotted_dates_are_not_point_decimals() {
        let (style, votes) = style_of(&["01.02.2026", "15.03.2026", "1.234,56", "9,99"]);
        assert_eq!(style.decimal, Some(','));
        assert_eq!(style.date, Some('.'));
        assert_eq!(votes.decimal, vote_of(2, 2));
        assert_eq!(votes.date, vote_of(2, 2));
        // A unanimous vote counts at any count: one amount with groups.
        assert_eq!((style.group, votes.group), (Some('.'), vote_of(1, 1)));
    }

    #[test]
    fn a_mixed_vote_needs_a_clear_majority() {
        // Unanimous, at any count.
        let (style, _) = style_of(&["1,50", "01.02.2026"]);
        assert_eq!((style.decimal, style.date), (Some(','), Some('.')));
        // Two against one, and two against two: not read.
        let (style, _) = style_of(&["1,50 2,50", "3.50"]);
        assert_eq!(style.decimal, None);
        let (style, votes) = style_of(&["1.234,56 und 9.876,54", "1,234.56 und 5,678.90"]);
        assert_eq!((style.decimal, style.group), (None, None));
        assert_eq!(votes, Votes::default());
        // Three against one is the least that counts.
        let (style, votes) = style_of(&["1,50 2,50 3,50", "4.50"]);
        assert_eq!((style.decimal, votes.decimal), (Some(','), vote_of(3, 4)));
        // The same rule for dates.
        let (style, _) = style_of(&["01.02.2026 02.02.2026 03.02.2026", "2026-02-04"]);
        assert_eq!(style.date, Some('.'));
        let (style, _) = style_of(&["01.02.2026 02.02.2026", "2026-02-04"]);
        assert_eq!(style.date, None);
    }

    #[test]
    fn rates_do_not_vote() {
        for rate in ["7.50%", "7.50 %", "7.50\u{a0}%", "7.50\u{202f}%"] {
            let (style, votes) = style_of(&["Summe 1,50", rate]);
            assert_eq!(style.decimal, Some(','), "{rate:?}");
            assert_eq!(votes.decimal, vote_of(1, 1), "{rate:?}");
        }
        // A number further off is no rate.
        let (style, _) = style_of(&["Summe 1,50", "7.50 EUR %"]);
        assert_eq!(style.decimal, None);
    }

    #[test]
    fn groups_need_their_own_evidence() {
        let (style, votes) = style_of(&["1.234,56", "9.876,54", "1,50"]);
        assert_eq!(
            style,
            SeparatorStyle {
                decimal: Some(','),
                date: None,
                group: Some('.'),
            }
        );
        assert_eq!((votes.decimal, votes.group), (vote_of(3, 3), vote_of(2, 2)));
    }

    #[test]
    fn the_note_names_the_evidence_of_the_kinds_used() {
        let style = SeparatorStyle {
            decimal: Some(','),
            date: Some('.'),
            group: None,
        };
        let votes = Votes {
            decimal: vote_of(11, 12),
            group: Vote::default(),
            date: vote_of(2, 2),
        };
        let dates = Resolved {
            date: 2,
            ..Resolved::default()
        };
        assert_eq!(
            note(&dates, &style, &votes).as_deref(),
            Some(
                "Found by: 2 date separators on this page are drawn marks, not text; they are \
                 written as \".\" as in 2 dates written as text in this file."
            )
        );
        let one = Resolved {
            decimal: 1,
            ..Resolved::default()
        };
        assert_eq!(
            note(&one, &style, &votes).as_deref(),
            Some(
                "Found by: 1 decimal separator on this page is a drawn mark, not text; it is \
                 written as \",\" as in 11 of 12 amounts written as text in this file."
            )
        );
        assert_eq!(note(&Resolved::default(), &style, &votes), None);
    }

    #[test]
    fn digit_groups_give_the_kinds() {
        use Kind::{Date, Decimal, Group};
        assert_eq!(kinds(&[4, 2]), Some(vec![Decimal]));
        assert_eq!(kinds(&[1, 3, 3, 2]), Some(vec![Group, Group, Decimal]));
        assert_eq!(kinds(&[3, 3, 2]), Some(vec![Group, Decimal]));
        for date in [[2, 2, 2], [2, 2, 4], [4, 2, 2]] {
            assert_eq!(kinds(&date), Some(vec![Date, Date]));
        }
        // One decimal (`9,0`, `1.234,5`).
        assert_eq!(kinds(&[2, 1]), Some(vec![Decimal]));
        assert_eq!(kinds(&[1, 1]), Some(vec![Decimal]));
        assert_eq!(kinds(&[1, 3, 1]), Some(vec![Group, Decimal]));
        // Groups of four, a first group too long for groups, no last one or two digits, a
        // placeholder at the edge of a number.
        for none in [
            &[1, 4, 2][..],
            &[4, 3, 2],
            &[4, 3, 1],
            &[3, 3],
            &[3, 4],
            &[0, 2],
            &[0, 1],
            &[4, 0],
            &[6],
        ] {
            assert_eq!(kinds(none), None, "{none:?}");
        }
    }

    #[test]
    fn unread_numbers_lose_only_their_placeholders() {
        let style = SeparatorStyle {
            decimal: Some(','),
            ..SeparatorStyle::default()
        };
        let text = format!("Summe 12{SEP}34 am 01{SEP}02{SEP}2026 und 5{SEP_TURNED}50");
        let mut seen = Vec::new();
        let fixed = fix(&text, &style, &mut |n, r| seen.push((n.to_string(), r)));
        assert_eq!(fixed, "Summe 12,34 am 01022026 und 550");
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], ("1234".into(), Ok(vec![Kind::Decimal])));
        assert_eq!(seen[1], ("01022026".into(), Err(NO_DATE)));
        assert_eq!(seen[2], ("550".into(), Err(TURNED)));
        // The placeholders of two copies drawn on top of each other count once.
        let text = format!("1234{SEP}{SEP}56");
        assert_eq!(fix(&text, &style, &mut |_, _| {}), "1234,56");
    }

    #[test]
    fn a_date_takes_the_character_of_its_order() {
        let iso = SeparatorStyle {
            date: Some('-'),
            ..SeparatorStyle::default()
        };
        let dotted = SeparatorStyle {
            date: Some('.'),
            ..SeparatorStyle::default()
        };
        let none = &mut |_: &str, _| {};
        let day_first = format!("01{SEP}02{SEP}2026");
        let year_first = format!("2026{SEP}02{SEP}01");
        assert_eq!(fix(&year_first, &iso, none), "2026-02-01");
        assert_eq!(fix(&day_first, &dotted, none), "01.02.2026");
        let mut why = Vec::new();
        fix(&day_first, &iso, &mut |_, r| why.push(r));
        fix(&year_first, &dotted, &mut |_, r| why.push(r));
        assert_eq!(why, [Err(DATE_ORDER), Err(DATE_ORDER)]);
    }

    #[test]
    fn a_number_cut_into_runs_is_read_whole() {
        let style = SeparatorStyle {
            decimal: Some(','),
            ..SeparatorStyle::default()
        };
        let line = format!("1234{SEP}56");
        let (head, tail) = line.split_at(4 + SEP.len_utf8());
        let mut decided = decide(&line, &style, &mut |_, _| {}).into_iter();
        assert_eq!(
            apply(head, &mut decided) + &apply(tail, &mut decided),
            "1234,56"
        );
    }

    #[test]
    fn rows_of_marks_are_dashed_rules() {
        // A dashed line of five dashes, 1.5 long with gaps of 1, and two separator marks
        // 11.4 points apart, which stay marks.
        let mut marks: Vec<[f32; 4]> = (0..5)
            .map(|k| {
                let x = 100.0 + 2.5 * k as f32;
                [x, 600.0, x + 1.5, 600.3]
            })
            .collect();
        marks.push([200.0, 700.0, 200.24, 702.0]);
        marks.push([211.4, 700.0, 211.64, 702.0]);
        let rules = dashed_rules(&mut marks);
        assert_eq!(rules.len(), 1, "{rules:?}");
        let r = &rules[0];
        assert_eq!((r.x0, r.x1, r.kind), (100.0, 111.5, RuleKind::Line));
        assert!((r.y0 - 600.15).abs() < 0.01 && r.y0 == r.y1);
        // The dashes of the rule are no separators; the two marks stay.
        assert_eq!(marks.len(), 2, "{marks:?}");
    }

    #[test]
    fn a_dashed_line_drawn_twice_is_one_rule() {
        // The edge two dashed boxes share, drawn by each: every dash twice. A separator mark
        // drawn three times over itself stays a mark.
        let mut marks: Vec<[f32; 4]> = (0..10)
            .map(|k| {
                let x = 100.0 + 2.5 * (k / 2) as f32;
                [x, 600.0, x + 1.5, 600.3]
            })
            .collect();
        marks.extend([[200.0, 700.0, 200.24, 702.0]; 3]);
        let rules = dashed_rules(&mut marks);
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert_eq!((rules[0].x0, rules[0].x1), (100.0, 111.5));
        assert_eq!(marks.len(), 3, "{marks:?}");
    }

    #[test]
    fn a_little_noise_across_a_dashed_line_keeps_it_whole() {
        // Middles across the line at 600.15, 600.1499, 600.1501 and 600.16, as a transform or
        // a writer leaves them: still one line from the first dash to the last.
        let jitter = [0.0, -0.0001, 0.0001, 0.01];
        let mut marks: Vec<[f32; 4]> = (0..12)
            .map(|k| {
                let (x, y) = (100.0 + 2.5 * k as f32, 600.0 + jitter[k % 4]);
                [x, y, x + 1.5, y + 0.3]
            })
            .collect();
        let rules = dashed_rules(&mut marks);
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert_eq!((rules[0].x0, rules[0].x1), (100.0, 129.0));
        assert!(marks.is_empty());
    }

    #[test]
    fn a_group_never_comes_from_the_minority_of_the_decimal_vote() {
        // The decimal comma wins 3 to 1; the one grouped amount has a decimal point, so its
        // comma is no group character.
        let (style, votes) = style_of(&["1,50 2,50 3,50", "1,234.56"]);
        assert_eq!((style.decimal, style.group), (Some(','), None));
        assert_eq!(votes.group, Vote::default());
        let mut why = Vec::new();
        let text = format!("1{SEP}234{SEP}56");
        let fixed = fix(&text, &style, &mut |_, r| why.push(r));
        assert_eq!(fixed, "123456");
        assert_eq!(why, [Err(NO_GROUP)]);
    }

    /// One glyph of `text[k]` at (x, y), 9.5 points, reading along +x.
    fn digit(k: u32, x: f32, y: f32, vertical: bool) -> Gl {
        Gl {
            start: k,
            len: 1,
            x,
            y,
            dx: 1.0,
            dy: 0.0,
            size: 9.5,
            adv: Some(5.7),
            bold: false,
            italic: false,
            family: 0,
            mcid: None,
            invisible: false,
            layer: None,
            vertical,
            joined: false,
            actual: None,
            link: None,
        }
    }

    #[test]
    fn a_mark_between_vertical_digits_is_never_read() {
        // Six digits down a column of vertical writing, 9.5 points apart, and a mark across
        // the column between the fourth and the fifth.
        let mut text = String::from("123456");
        let mut glyphs: Vec<Gl> = (0..6)
            .map(|k| digit(k, 100.0, 700.0 - 9.5 * k as f32, true))
            .collect();
        let mut marks = vec![[99.88, 665.0, 100.12, 667.0]];
        assert!(insert(
            &mut glyphs,
            &mut text,
            &mut marks,
            Turn::None,
            &mut Vec::new()
        ));
        assert_eq!(glyphs.len(), 7);
        let placed = &glyphs[4];
        assert_eq!(&text[placed.start as usize..][..3], SEP_TURNED.to_string());
        // Between the fourth digit and the fifth, down the column.
        assert!(
            placed.y < glyphs[3].y && placed.y > glyphs[5].y,
            "{placed:?}"
        );
        let texts: String = glyphs
            .iter()
            .map(|g| &text[g.start as usize..(g.start + g.len) as usize])
            .collect();
        let mut why = Vec::new();
        let style = SeparatorStyle {
            decimal: Some(','),
            ..SeparatorStyle::default()
        };
        assert_eq!(fix(&texts, &style, &mut |_, r| why.push(r)), "123456");
        assert_eq!(why, [Err(TURNED)]);
    }

    #[test]
    fn marks_by_shape_and_size() {
        // With text of 9.5 points: at most 3.04 points long, and a third of that thick.
        assert!(is_mark([10.0, 10.0, 10.24, 12.0], 9.5));
        assert!(is_mark([10.0, 10.0, 12.0, 10.24], 9.5));
        // A dash of a rule, its pen half a point wide.
        assert!(is_mark([10.0, 10.0, 11.6, 10.5], 8.0));
        // Too thick, too long, too square, nothing.
        assert!(!is_mark([10.0, 10.0, 10.8, 12.0], 9.5));
        assert!(!is_mark([10.0, 10.0, 10.24, 13.5], 9.5));
        assert!(!is_mark([10.0, 10.0, 10.4, 11.0], 9.5));
        assert!(!is_mark([10.0, 10.0, 10.0, 12.0], 9.5));
        assert!(!mark_shaped([10.0, 10.0, 10.0, 12.0]));
        // The same marks printed at 1.5 with larger text (15.75 points): 3.15 points long is a
        // mark there, and a mark of 0.75 points thin too.
        assert!(is_mark([10.0, 10.0, 10.24, 13.15], 15.75));
        assert!(is_mark([10.0, 10.0, 10.75, 13.15], 15.75));
        assert!(!is_mark([10.0, 10.0, 10.24, 13.15], 9.5));
        // Printed small (6 points), a bar of 2.5 points is no mark.
        assert!(!is_mark([10.0, 10.0, 10.2, 12.5], 6.0));
        assert!(mark_shaped([10.0, 10.0, 10.2, 12.5]));
    }

    /// Two digits of `size` points on the baseline 700, the second right after the first.
    fn two_digits(size: f32) -> (Vec<Gl>, String) {
        let glyph = |k: u32, x: f32| Gl {
            size,
            adv: Some(0.6 * size),
            ..digit(k, x, 700.0, false)
        };
        (
            vec![glyph(0, 100.0), glyph(1, 100.0 + 0.6 * size)],
            "12".into(),
        )
    }

    #[test]
    fn a_long_mark_is_a_separator_by_large_digits_only() {
        // A mark 3.15 points high between two digits of 15.75 points (a form printed at 1.5
        // with larger text) is a separator; between digits of 9.5 points the same bar is too
        // long for one: it stays what it is, and its number is listed as not read.
        let mark = |size: f32| {
            let x = 100.0 + 0.6 * size;
            [
                x - 0.12,
                700.0 - 0.087 * size,
                x + 0.12,
                700.0 - 0.087 * size + 3.15,
            ]
        };
        let (mut glyphs, mut text) = two_digits(15.75);
        let (mut marks, mut used) = (vec![mark(15.75)], Vec::new());
        assert!(insert(
            &mut glyphs,
            &mut text,
            &mut marks,
            Turn::None,
            &mut used
        ));
        assert_eq!(glyphs.len(), 3);
        assert_eq!(used, [mark(15.75)]);
        let (mut glyphs, mut text) = two_digits(9.5);
        let (mut marks, mut used) = (vec![mark(9.5)], Vec::new());
        assert!(insert(
            &mut glyphs,
            &mut text,
            &mut marks,
            Turn::None,
            &mut used
        ));
        // It is named: a placeholder that lists the number as not read.
        assert_eq!((glyphs.len(), used.len()), (3, 0));
        assert_eq!(&text[glyphs[1].start as usize..][..3], SEP_LONG.to_string());
    }

    #[test]
    fn the_page_size_is_the_median_digit() {
        let (mut glyphs, _) = two_digits(9.5);
        glyphs.push(Gl {
            start: 2,
            ..two_digits(15.0).0[0]
        });
        glyphs.push(Gl {
            start: 3,
            ..two_digits(9.0).0[0]
        });
        // Only the digits count: 9.5 and 9.5, or 15 and 9 (the upper median).
        assert_eq!(page_size(&glyphs, "12ab"), Some(9.5));
        assert_eq!(page_size(&glyphs, "ab12"), Some(15.0));
        // No digits: every glyph.
        assert_eq!(page_size(&glyphs[..2], "ab"), Some(9.5));
        assert_eq!(page_size(&[], ""), None);
    }
}
