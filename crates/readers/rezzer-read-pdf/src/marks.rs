// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Separators drawn as marks. Boxed number fields of a form often print their decimal and
//! date separators as tiny filled rectangles, not as text; the digits of the value then
//! read without them. A mark between two digits of one baseline puts a placeholder into the
//! page's glyphs; once every page is read, the placeholders become the separators the
//! file's own text uses, or are removed and the number is listed as not read.

use crate::scan;
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

/// The separator characters `texts` show, and the words that name the evidence (for example
/// `3 amounts and 2 dates`).
#[cfg(test)]
pub(crate) fn style_of(texts: &[&str]) -> (SeparatorStyle, String) {
    let mut evidence = Evidence::default();
    texts.iter().for_each(|t| evidence.scan(t));
    evidence.style()
}

/// The separator characters the visible text of `pages` shows, and the words that name the
/// evidence, as [`style_of`] gives them for texts: each line of runs is scanned whole (a
/// number may be cut into runs), with no copy of the whole text.
pub(crate) fn style_in(pages: &[PageItems]) -> (SeparatorStyle, String) {
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
/// point decimal.
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

    /// Per kind, the one character with at least two hits while the others have none, and
    /// the words for the evidence of the kinds set.
    fn style(&self) -> (SeparatorStyle, String) {
        let decimal = clear(&self.decimal, &AMOUNT_SEPS);
        let group = clear(&self.group, &AMOUNT_SEPS);
        let date = clear(&self.date, &DATE_SEPS);
        let amounts = decimal.or(group).map(|(_, n)| n);
        let words = match (amounts, date) {
            (Some(a), Some((_, d))) => format!("{a} amounts and {d} dates"),
            (Some(a), None) => format!("{a} amounts"),
            (None, Some((_, d))) => format!("{d} dates"),
            (None, None) => String::new(),
        };
        let style = SeparatorStyle {
            decimal: decimal.map(|(c, _)| c),
            date: date.map(|(c, _)| c),
            group: group.map(|(c, _)| c),
        };
        (style, words)
    }
}

/// The character of `chars` whose count is at least 2 while every other one is 0, with its
/// count.
fn clear(counts: &[u32], chars: &[char]) -> Option<(char, u32)> {
    let mut hit = counts.iter().zip(chars).filter(|(n, _)| **n > 0);
    let (&n, &c) = hit.next()?;
    (n >= 2 && hit.next().is_none()).then_some((c, n))
}

/// What the placeholders of one page became.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub decimal: u32,
    pub date: u32,
    pub group: u32,
    /// The numbers whose separators are not read: the place, the digit count and the
    /// reason, never the value.
    pub unread: Vec<String>,
    /// Unread numbers past those `unread` names.
    pub more: u32,
}

/// Puts the separators of `style` in place of the placeholders of the pages' text, or
/// removes them, before the pages are laid out. Returns what each page with placeholders
/// got.
///
/// Placeholders are put only between visible glyphs, so only the visible runs (`runs`) and
/// the quotes taken from them (`quotes`, which get the same characters) hold them. The text
/// nobody sees (`invisible`, `hidden_layers`, `hidden_layer`) is cleaned of them too, so no
/// placeholder can reach the output whatever the glyphs were.
pub(crate) fn resolve(pages: &mut [PageItems], style: &SeparatorStyle) -> Vec<(u32, Resolved)> {
    let mut out = Vec::new();
    for page in pages.iter_mut() {
        let mut got = Resolved::default();
        let mut any = false;
        for run in &mut page.runs {
            if !run.text.contains(SEP) {
                continue;
            }
            any = true;
            let line = run.line + 1;
            run.text = fix(&run.text, style, &mut |digits, read| match read {
                Ok(kinds) => {
                    for k in kinds {
                        match k {
                            Kind::Decimal => got.decimal += 1,
                            Kind::Group => got.group += 1,
                            Kind::Date => got.date += 1,
                        }
                    }
                }
                Err(why) if got.unread.len() < MAX_UNREAD => got.unread.push(format!(
                    "the number in line {line} ({digits} digits) is kept without it, as {why}"
                )),
                Err(_) => got.more += 1,
            });
        }
        for quote in &mut page.quotes {
            if let Some(text) = quote.text.as_mut().filter(|t| t.contains(SEP)) {
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
/// two digits is a decimal separator, and the ones before it cut groups of three after one
/// to three digits; 2-2-2, 2-2-4 and 4-2-2 are dates.
fn kinds(lens: &[usize]) -> Option<Vec<Kind>> {
    if matches!(lens, [2, 2, 2] | [2, 2, 4] | [4, 2, 2]) {
        return Some(vec![Kind::Date; 2]);
    }
    let (&last, rest) = lens.split_last()?;
    let first = *rest.first()?;
    let groups = &rest[1..];
    let fits = last == 2
        && first > 0
        && (groups.is_empty() || first <= 3)
        && groups.iter().all(|&n| n == 3);
    fits.then(|| {
        let mut k = vec![Kind::Group; groups.len()];
        k.push(Kind::Decimal);
        k
    })
}

/// `text` with the placeholders of each number replaced by the separators of `style`, or
/// removed when a character is not known or the digit groups fit no kind. `seen` is told
/// each number with placeholders: its digit count and its kinds, or why it is not read.
fn fix(
    text: &str,
    style: &SeparatorStyle,
    seen: &mut dyn FnMut(usize, Result<Vec<Kind>, &'static str>),
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut number = String::new();
    let mut flush = |number: &mut String, out: &mut String| {
        if !number.contains(SEP) {
            out.push_str(number);
            number.clear();
            return;
        }
        let lens: Vec<usize> = number.split(SEP).map(str::len).collect();
        let digits = lens.iter().sum();
        let read = kinds(&lens)
            .ok_or("its digit groups fit no amount and no date")
            .and_then(|k| {
                let chars: Result<Vec<char>, &'static str> = k
                    .iter()
                    .map(|k| match k {
                        Kind::Decimal => style.decimal.ok_or(
                            "the amounts written as text in this file do not show one decimal \
                             separator",
                        ),
                        Kind::Group => style.group.ok_or(
                            "the amounts written as text in this file do not show one group \
                             separator",
                        ),
                        Kind::Date => style.date.ok_or(
                            "the dates written as text in this file do not show one date \
                             separator",
                        ),
                    })
                    .collect();
                chars.map(|c| (k, c))
            });
        match read {
            Ok((k, chars)) => {
                let mut chars = chars.into_iter();
                // One character a placeholder: `kinds` gives one kind each.
                for c in number.chars() {
                    if c != SEP {
                        out.push(c);
                    } else if let Some(sep) = chars.next() {
                        out.push(sep);
                    }
                }
                seen(digits, Ok(k));
            }
            Err(why) => {
                out.extend(number.chars().filter(|&c| c != SEP));
                seen(digits, Err(why));
            }
        }
        number.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || c == SEP {
            number.push(c);
        } else {
            flush(&mut number, &mut out);
            out.push(c);
        }
    }
    flush(&mut number, &mut out);
    out
}

/// The `found-by` note of a page whose separators `got` says, with the `evidence` words of
/// [`style_in`]; `None` when no separator was written.
pub(crate) fn note(got: &Resolved, style: &SeparatorStyle, evidence: &str) -> Option<String> {
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
        .map(|&(n, name, _)| {
            let s = if n == 1 { "" } else { "s" };
            format!("{n} {name} separator{s}")
        })
        .collect();
    let chars: Vec<String> = used.iter().map(|u| format!("\"{}\"", u.2)).collect();
    let (verb, it) = if total == 1 {
        ("is a drawn mark", "it is")
    } else {
        ("are drawn marks", "they are")
    };
    Some(format!(
        "Found by: {} on this page {verb}, not text; {it} written as {} as in {evidence} \
         written as text in this file.",
        listed(&counts),
        listed(&chars)
    ))
}

/// `items` joined with commas and a last `and`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The middle of a mark along x.
fn middle(m: &[f32; 4]) -> f32 {
    (m[0] + m[2]) / 2.0
}

/// Puts a placeholder between two neighbouring digit glyphs of one baseline (`glyphs[i]` and
/// `glyphs[i + 1]`, visible, each with the text of one ASCII digit of `text`, reading along
/// +x) when a mark's middle lies between their middles (origin plus half the advance), its
/// bottom within 0.3 font sizes of the baseline and its height at most half the font size.
/// `marks` are in the frame of the glyphs and come back sorted. False when the marks near
/// the digits were too many to test them all ([`MARK_WORK`]).
pub(crate) fn insert(glyphs: &mut Vec<Gl>, text: &mut String, marks: &mut [[f32; 4]]) -> bool {
    if marks.is_empty() || glyphs.len() < 2 {
        return true;
    }
    marks.sort_by(|a, b| middle(a).total_cmp(&middle(b)));
    let bytes = text.as_bytes();
    let digit = |g: &Gl| {
        !g.invisible
            && g.layer.is_none()
            && g.actual.is_none()
            && !g.joined
            && !g.vertical
            && g.dx >= 0.985
            && g.len == 1
            && bytes.get(g.start as usize).is_some_and(u8::is_ascii_digit)
    };
    let mut after: Vec<usize> = Vec::new();
    let mut work = 0usize;
    let mut whole = true;
    'pairs: for i in 0..glyphs.len() - 1 {
        let (a, b) = (&glyphs[i], &glyphs[i + 1]);
        if !digit(a) || !digit(b) {
            continue;
        }
        let size = a.size.max(0.1);
        if (b.size - a.size).abs() > 0.05 * size || (b.y - a.y).abs() > 0.1 * size {
            continue;
        }
        let (adv_a, adv_b) = (a.adv.unwrap_or(0.5 * size), b.adv.unwrap_or(0.5 * size));
        let (from, to) = (a.x + adv_a / 2.0, b.x + adv_b / 2.0);
        // Neighbours: the second digit follows the first within a font size.
        if to <= from || b.x - (a.x + adv_a) > size {
            continue;
        }
        let first = marks.partition_point(|m| middle(m) <= from);
        for m in &marks[first..] {
            work += 1;
            if work > MARK_WORK {
                whole = false;
                break 'pairs;
            }
            if middle(m) >= to {
                break;
            }
            if (m[1] - a.y).abs() <= 0.3 * size && m[3] - m[1] <= 0.5 * size {
                after.push(i);
                break;
            }
        }
    }
    let (Some(&last), Ok(start)) = (after.last(), u32::try_from(text.len())) else {
        return whole;
    };
    text.push(SEP);
    let len = SEP.len_utf8() as u32;
    // Moved up from the back, each placeholder goes in right after its first digit.
    let n = glyphs.len();
    glyphs.resize(n + after.len(), glyphs[last]);
    let (mut src, mut dst) = (n, glyphs.len());
    for &i in after.iter().rev() {
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
            start,
            len,
            x: a.x + 0.75 * adv * a.dx,
            y: a.y + 0.75 * adv * a.dy,
            adv: Some(0.0),
            ..a
        };
    }
    whole
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_dates_are_not_point_decimals() {
        let (style, evidence) = style_of(&["01.02.2026", "15.03.2026", "1.234,56", "9,99"]);
        assert_eq!(style.decimal, Some(','));
        assert_eq!(style.date, Some('.'));
        // One amount with groups is no evidence for the group character.
        assert_eq!(style.group, None);
        assert_eq!(evidence, "2 amounts and 2 dates");
    }

    #[test]
    fn one_hit_or_two_styles_set_nothing() {
        let (style, _) = style_of(&["1,50", "01.02.2026"]);
        assert_eq!(style, SeparatorStyle::default());
        let (style, _) = style_of(&["1,50 2,50", "3.50"]);
        assert_eq!(style.decimal, None);
        let (style, evidence) = style_of(&["1.234,56 und 9.876,54", "1,234.56"]);
        assert_eq!((style.decimal, style.group), (None, None));
        assert_eq!(evidence, "");
    }

    #[test]
    fn groups_need_their_own_evidence() {
        let (style, evidence) = style_of(&["1.234,56", "9.876,54"]);
        assert_eq!(
            style,
            SeparatorStyle {
                decimal: Some(','),
                date: None,
                group: Some('.'),
            }
        );
        assert_eq!(evidence, "2 amounts");
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
        // Groups of four, a first group too long for groups, no last two digits, a
        // placeholder at the edge of a number.
        for none in [
            &[1, 4, 2][..],
            &[4, 3, 2],
            &[3, 3],
            &[3, 4],
            &[0, 2],
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
        let text = format!("Summe 12{SEP}34 am 01{SEP}02{SEP}2026");
        let mut seen = Vec::new();
        let fixed = fix(&text, &style, &mut |n, r| seen.push((n, r)));
        assert_eq!(fixed, "Summe 12,34 am 01022026");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], (4, Ok(vec![Kind::Decimal])));
        assert_eq!(seen[1].0, 8);
        assert!(seen[1].1.is_err());
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
