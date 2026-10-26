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
use crate::tables::{Rule, RuleKind};
use crate::text::{Gl, PageItems};

/// The short side of a mark is at most this many points.
pub(crate) const MARK_THIN: f32 = 0.5;
/// The long side of a mark is at most this many points.
pub(crate) const MARK_LONG: f32 = 3.0;
/// The long side of a mark is at least this many times its short side.
pub(crate) const MARK_RATIO: f32 = 3.0;
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

/// Whether `c` is one of the noncharacters U+FDD0 to U+FDEF: Rezzer's own placeholders,
/// dropped from the text of a file.
pub(crate) fn is_placeholder(c: char) -> bool {
    ('\u{FDD0}'..='\u{FDEF}').contains(&c)
}

/// Whether a filled axis-aligned box (x0, y0, x1, y1) is a mark: thin, short and long for
/// its width.
pub(crate) fn is_mark(b: [f32; 4]) -> bool {
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let (short, long) = (w.min(h), w.max(h));
    short > 0.0 && short <= MARK_THIN && long <= MARK_LONG && long >= MARK_RATIO * short
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
    /// Amounts with groups of three digits after `,` and after `.`.
    group: [u32; 2],
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
            if let Some(k) = AMOUNT_SEPS.iter().position(|&c| c == a.decimal) {
                self.decimal[k] += 1;
            }
            if let Some(k) = a
                .group
                .and_then(|g| AMOUNT_SEPS.iter().position(|&c| c == g))
            {
                self.group[k] += 1;
            }
        }
    }

    /// Per kind, the character the vote of [`vote`] gives.
    fn style(&self) -> (SeparatorStyle, Votes) {
        let decimal = vote(&self.decimal, &AMOUNT_SEPS);
        let group = vote(&self.group, &AMOUNT_SEPS);
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
    let mut flush = |digits: &mut String,
                     lens: &mut Vec<usize>,
                     seps: &mut Vec<usize>,
                     turned: &mut bool,
                     out: &mut Vec<Option<char>>| {
        if !seps.is_empty() {
            let read = if *turned {
                Err(TURNED)
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
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            if seps.len() == lens.len() {
                lens.push(0);
            }
            digits.push(c);
            *lens.last_mut().expect("a group") += 1;
        } else if c == SEP || c == SEP_TURNED {
            turned |= c == SEP_TURNED;
            if seps.len() == lens.len() {
                *seps.last_mut().expect("a placeholder") += 1;
            } else {
                seps.push(1);
            }
        } else {
            flush(&mut digits, &mut lens, &mut seps, &mut turned, &mut out);
        }
    }
    // A number that ends with placeholders: they cut off an empty group.
    flush(&mut digits, &mut lens, &mut seps, &mut turned, &mut out);
    out
}

/// `text` with each placeholder replaced by what `decided` says, in order.
fn apply(text: &str, decided: &mut dyn Iterator<Item = Option<char>>) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == SEP || c == SEP_TURNED {
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
/// is listed as not read. `marks` are in the frame of the glyphs and come back sorted by
/// their grid cell. False when the marks near the digits were too many to test them all
/// ([`MARK_WORK`]).
pub(crate) fn insert(
    glyphs: &mut Vec<Gl>,
    text: &mut String,
    marks: &mut [[f32; 4]],
    turn: Turn,
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
    let mut after: Vec<(usize, char)> = Vec::new();
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
        // The glyph's own frame: along the baseline and up from it.
        let (d, up) = ((a.dx, a.dy), (-a.dy, a.dx));
        let along = |x: f32, y: f32| (x - a.x) * d.0 + (y - a.y) * d.1;
        let across = |x: f32, y: f32| (x - a.x) * up.0 + (y - a.y) * up.1;
        if across(b.x, b.y).abs() > 0.1 * size {
            continue;
        }
        let (adv_a, adv_b) = (a.adv.unwrap_or(0.5 * size), b.adv.unwrap_or(0.5 * size));
        let (from, to) = (adv_a / 2.0, along(b.x, b.y) + adv_b / 2.0);
        // Neighbours: the second digit follows the first within a font size.
        if to <= from || along(b.x, b.y) - adv_a > size {
            continue;
        }
        // Where a mark's middle may be: between the digit middles, from 0.3 sizes under the
        // baseline to the top of the highest mark that may stand on it.
        let (t0, t1) = (-0.3 * size, 0.3 * size + 0.25 * size.min(2.0 * MARK_LONG));
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
                if s > from && s < to && bottom.abs() <= 0.3 * size && top - bottom <= 0.5 * size {
                    after.push((i, if readable { SEP } else { SEP_TURNED }));
                    continue 'pairs;
                }
            }
        }
    }
    let (Some(&(last, _)), Ok(at)) = (after.last(), u32::try_from(text.len())) else {
        return whole;
    };
    // Each placeholder has its text once at the end of the page's text.
    text.push(SEP);
    text.push(SEP_TURNED);
    let len = SEP.len_utf8() as u32;
    let start = |c: char| if c == SEP { at } else { at + len };
    // Moved up from the back, each placeholder goes in right after its first digit.
    let n = glyphs.len();
    glyphs.resize(n + after.len(), glyphs[last]);
    let (mut src, mut dst) = (n, glyphs.len());
    for &(i, c) in after.iter().rev() {
        while src > i + 1 {
            src -= 1;
            dst -= 1;
            glyphs[dst] = glyphs[src];
        }
        let a = glyphs[i];
        let adv = a.adv.unwrap_or(0.5 * a.size.max(0.1));
        dst -= 1;
        // Inside the first digit with no advance, so it joins the digits without a space.
        glyphs[dst] = Gl {
            start: start(c),
            len,
            x: a.x + 0.75 * adv * a.dx,
            y: a.y + 0.75 * adv * a.dy,
            adv: Some(0.0),
            ..a
        };
    }
    whole
}

/// Lines drawn as rows of marks: dashed or dotted rules of filled dashes, which are marks by
/// shape. Marks of one direction on one line (their middles within 0.1 points across it, the
/// same thickness), each at most its own length after the one before, make one line when
/// they are at least three. `marks` come back sorted by direction, line and start.
pub(crate) fn dashed_rules(marks: &mut [[f32; 4]]) -> Vec<Rule> {
    // Along the long side: (across, start, end, thickness).
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
        fa.0.cmp(&fb.0)
            .then(fa.1.total_cmp(&fb.1))
            .then(fa.2.total_cmp(&fb.2))
    });
    let mut out = Vec::new();
    let mut i = 0;
    while i < marks.len() {
        let (upright, across, start, mut end, thick) = flat(&marks[i]);
        let mut count = 1;
        let mut j = i + 1;
        while let Some(m) = marks.get(j) {
            let (u, c, s, e, t) = flat(m);
            let gap = s - end;
            if u != upright
                || (c - across).abs() > 0.1
                || (t - thick).abs() > 0.1
                || gap < 0.0
                || gap > e - s
            {
                break;
            }
            end = e;
            count += 1;
            j += 1;
        }
        if count >= 3 {
            out.push(if upright {
                Rule {
                    x0: across,
                    y0: start,
                    x1: across,
                    y1: end,
                    kind: RuleKind::Line,
                }
            } else {
                Rule {
                    x0: start,
                    y0: across,
                    x1: end,
                    y1: across,
                    kind: RuleKind::Line,
                }
            });
        }
        i = j;
    }
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
    }

    #[test]
    fn marks_by_shape() {
        assert!(is_mark([10.0, 10.0, 10.24, 12.0]));
        assert!(is_mark([10.0, 10.0, 12.0, 10.24]));
        // Too thick, too long, too square, nothing.
        assert!(!is_mark([10.0, 10.0, 10.6, 12.0]));
        assert!(!is_mark([10.0, 10.0, 10.24, 13.5]));
        assert!(!is_mark([10.0, 10.0, 10.4, 11.0]));
        assert!(!is_mark([10.0, 10.0, 10.0, 12.0]));
    }
}
