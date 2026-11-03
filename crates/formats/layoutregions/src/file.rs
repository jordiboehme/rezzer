// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a file says as a whole, learned from every page before any page is laid out: the
//! body size, the heading sizes, the running headers and footers, the footer of a file of two
//! pages and the footnote legend of each page.
//!
//! [`learn`] reads the pages three times over (the running lines first, then the body size
//! without them and without the legends, then the heading sizes against that body size). It
//! borrows the runs of each page as the reader keeps them (see [`FilePage`], [`LineRun`]):
//! nothing of the file's text is copied.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::input::{PageIn, TextRun};
use crate::line::{LineRun, TextLine, ink, size_key, text_lines};

mod legend;

pub use legend::{Legend, LegendEntry, NoteMark, RaisedRef, RefPlace, Split, word_marks};

/// Share of the page height at the top and the bottom where running headers and footers sit.
/// pdflatex sets page numbers about 11 % above the bottom edge of an A4 page.
pub(crate) const BAND: f32 = 0.18;

/// Pages a header or footer line must repeat on. In a file of two pages, a footer line repeats
/// on both (see [`learn`]).
pub(crate) const REPEATS: usize = 3;

/// Lines of one header or footer sit at the same height on every page, within this many
/// points.
pub(crate) const Y_STEP: f32 = 4.0;

/// A line at least this many times the body size is a heading in an untagged file.
pub const HEADING_RATIO: f32 = 1.2;

/// A line smaller than this many times the body size is a heading only when it is led by
/// bold ink (see [`bold_led`]): data lines and labels set a little larger than the body are
/// no headings.
pub const HEADING_PLAIN_RATIO: f32 = 1.5;

/// A heading line is shorter than this many characters.
pub(crate) const HEADING_CHARS: usize = 120;

/// Currency words and signs that may stand after an amount.
pub const CURRENCIES: [&str; 7] = ["EUR", "USD", "CHF", "GBP", "\u{20ac}", "$", "\u{a3}"];

/// Memory the mid-page line texts the file pass keeps may take, to tell running headers from
/// content: the text and about 64 bytes of set overhead per line. Past it, no more are kept:
/// a header text that also stands mid-page later in a huge file may then keep its header
/// role.
#[cfg(not(test))]
const BODY_KEY_BYTES: usize = 32 << 20;
#[cfg(test)]
const BODY_KEY_BYTES: usize = 1_000;

/// Set overhead of one kept line text.
const BODY_KEY_COST: usize = 64;

#[cfg(test)]
thread_local! {
    /// The mid-page line texts the last file pass kept.
    static BODY_KEPT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The band of the page a running line stands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Band {
    Top,
    Bottom,
}

/// A running header or footer: a line text (see [`furniture_key`]) in a band of the page that
/// repeats on at least [`REPEATS`] pages at one height (or on both pages of a file of two, at
/// the bottom). One group of such heights is one `Running`; a text at two heights is two.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub key: String,
    pub band: Band,
    /// The pages of the group, in order.
    pub pages: Vec<u32>,
    /// The lowest and the highest middle of its lines (see [`TextLine::middle`]).
    pub at: [f32; 2],
}

/// What the layout learns from the whole file before it lays out a page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileFacts {
    /// The most common font size, by ink character count, of the text outside running
    /// headers and footers and outside the footnote legends of an untagged file: the mean
    /// exact size of the winning half point.
    pub body_size: f32,
    /// The distinct sizes of the lines that may head (see [`may_head`]), largest first: the
    /// place of a size is its heading level.
    pub heading_sizes: Vec<f32>,
    /// The running headers and footers, by band, key and height.
    pub running: Vec<Running>,
    /// The keys of the running footers that are running by the two-page rule only, sorted.
    /// They stand in the bottom band, as the rule asks. They are running footers by their role,
    /// but their ink still counts for the body size and the text sizes of a page: an invoice
    /// of two pages keeps the body size and so the headings it had.
    pub two_page: Vec<String>,
    /// The footnote legend of each page of an untagged file that has one.
    pub legends: BTreeMap<u32, Legend>,
}

/// The text of a line with every digit replaced, white space collapsed: the same running line
/// on every page, whatever page number it holds.
pub(crate) fn furniture_key(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The band of a page of `height` a line stands in, if any.
pub(crate) fn band_of<R>(line: &TextLine<'_, R>, height: f32) -> Option<Band> {
    if height <= 0.0 || height.is_nan() {
        return None;
    }
    let middle = line.middle();
    if middle >= height * (1.0 - BAND) {
        Some(Band::Top)
    } else if middle <= height * BAND {
        Some(Band::Bottom)
    } else {
        None
    }
}

/// The band and the key of a line that may be a running line on a page of `height`: it stands
/// in a band and has text.
pub(crate) fn running_key<R>(line: &TextLine<'_, R>, height: f32) -> Option<(Band, String)> {
    let b = band_of(line, height)?;
    let key = furniture_key(&line.text);
    (!key.is_empty()).then_some((b, key))
}

/// Whether a line on a page of `height` is a footer of the two-page rule: in the bottom band,
/// with a key of `two_page`.
pub fn two_page_line<R>(line: &TextLine<'_, R>, height: f32, two_page: &HashSet<String>) -> bool {
    running_key(line, height).is_some_and(|(b, k)| b == Band::Bottom && two_page.contains(&k))
}

/// The running lines by band and key, for the check of a page's lines.
#[derive(Debug, Clone, Default)]
pub struct RunningIndex {
    at: HashMap<(Band, String), Vec<[f32; 2]>>,
}

impl RunningIndex {
    pub fn new(running: &[Running]) -> RunningIndex {
        let mut at: HashMap<(Band, String), Vec<[f32; 2]>> = HashMap::new();
        for r in running {
            at.entry((r.band, r.key.clone())).or_default().push(r.at);
        }
        RunningIndex { at }
    }

    /// The band of each of `lines`, a page of `height`, that is a running header or footer on
    /// it. A header stands above all text of the page and a footer below it: a repeated line
    /// inside the text stays text. Lines of white space only are no text of the page.
    /// A line marked in `content` is never a header or footer; an empty `content` marks none.
    pub fn bands<R>(
        &self,
        lines: &[TextLine<'_, R>],
        height: f32,
        content: &[bool],
    ) -> Vec<Option<Band>> {
        let found: Vec<Option<Band>> = lines
            .iter()
            .enumerate()
            .map(|(k, l)| {
                if content.get(k).copied().unwrap_or(false) {
                    return None;
                }
                let key = running_key(l, height)?;
                let m = l.middle();
                self.at
                    .get(&key)?
                    .iter()
                    .any(|[low, high]| m >= *low && m <= *high)
                    .then_some(key.0)
            })
            .collect();
        // Lines of white space only are no text to stand above or below: a print run blanks
        // the empty lines of a footer column with spaces.
        let body = || {
            lines
                .iter()
                .zip(&found)
                .filter(|(l, b)| b.is_none() && !l.text.is_empty())
        };
        let top = body().map(|(l, _)| l.middle()).fold(f32::MIN, f32::max);
        let bottom = body().map(|(l, _)| l.middle()).fold(f32::MAX, f32::min);
        lines
            .iter()
            .zip(found)
            .map(|(l, b)| match b {
                Some(Band::Top) if l.middle() > top => Some(Band::Top),
                Some(Band::Bottom) if l.middle() < bottom => Some(Band::Bottom),
                _ => None,
            })
            .collect()
    }
}

/// Whether a line is large enough and short enough to be a heading against `body_size`: at
/// least [`HEADING_RATIO`] times it, under [`HEADING_CHARS`] characters, with a letter or digit.
pub(crate) fn is_heading_size<R>(line: &TextLine<'_, R>, body_size: f32) -> bool {
    body_size > 0.0
        && line.size >= body_size * HEADING_RATIO
        && line.text.chars().count() < HEADING_CHARS
        && line.text.chars().any(char::is_alphanumeric)
}

/// Whether an untagged line may be a heading by its own text and style: large enough, short,
/// not ending in an amount (`ends_in_amount`, the reader's own reading of amounts and dates),
/// and, when it is smaller than [`HEADING_PLAIN_RATIO`] times the body size, led by bold ink
/// (see [`bold_led`]) and not only data.
pub fn may_head<R: LineRun>(
    line: &TextLine<'_, R>,
    body_size: f32,
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> bool {
    // A number set large (a form or file number) stays a heading.
    let large = line.size >= HEADING_PLAIN_RATIO * body_size;
    is_heading_size(line, body_size)
        && !ends_in_amount(&line.text)
        && (large || (!data_only(&line.text) && bold_led(line)))
}

/// Whether a line is bold where it matters for a heading: all its ink is bold, or it opens
/// with bold ink and goes on in plain words that are no data (`SECTION 1 (TO BE COMPLETED BY
/// PAYEE)`). A bold label that ends in a colon before plain text is a label and its value
/// (`Kundennummer: 00000`), and so is a bold label before a plain number or code.
pub(crate) fn bold_led<R: LineRun>(line: &TextLine<'_, R>) -> bool {
    let inked: Vec<&R> = line.runs.iter().filter(|r| ink(r.text()) > 0).collect();
    let lead = inked.iter().take_while(|r| r.bold()).count();
    if lead == 0 {
        return false;
    }
    if lead == inked.len() {
        return true;
    }
    let label: String = inked[..lead].iter().map(|r| r.text()).collect();
    let rest: String = inked[lead..].iter().map(|r| r.text()).collect();
    !label.trim_end().ends_with(':') && !data_only(&rest)
}

/// Whether a line holds only numbers, codes, dates and amounts: each of its words holds a
/// digit or is a currency word or sign, and one holds a digit (`1234567 8901 2 30`,
/// `00123 45B`). A title holds a word (`Bericht 2025`).
pub fn data_only(text: &str) -> bool {
    let mut digit = false;
    for w in text.split_whitespace() {
        if w.chars().any(|c| c.is_ascii_digit()) {
            digit = true;
        } else if !CURRENCIES.contains(&w) {
            return false;
        }
    }
    digit
}

/// One page as [`learn`] reads it: its number, the height of the frame its runs are in, and
/// its runs in line order, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct FilePage<'a, R> {
    pub page: u32,
    pub height: f32,
    pub runs: &'a [R],
}

impl<'a> FilePage<'a, TextRun> {
    /// The text of an engine page.
    pub fn of(page: &'a PageIn) -> FilePage<'a, TextRun> {
        FilePage {
            page: page.page,
            height: page.size[1],
            runs: &page.texts,
        }
    }
}

/// Learns body size, heading sizes, running headers and footers and, for an untagged file
/// (`tagged` false), the footnote legends from every page of `pages`, in order.
/// `ends_in_amount` tells a line that ends in an amount, which is no heading.
///
/// A line at the foot of both pages of a file of two is a footer: a letter or an invoice of two
/// pages has no third page to repeat it on. A letterhead at the top of both is content: the
/// title and fields of an invoice repeat on its second page.
pub fn learn<R: LineRun>(
    pages: &[FilePage<'_, R>],
    tagged: bool,
    ends_in_amount: &dyn Fn(&str) -> bool,
) -> FileFacts {
    let mut seen: HashMap<(Band, String), Vec<(f32, u32)>> = HashMap::new();
    // Texts that stand in the middle of some page are content, wherever else they stand.
    // A bare page number is not held against: numbers fill tables too.
    let mut body: HashSet<String> = HashSet::new();
    let mut body_bytes = 0usize;
    for p in pages {
        for l in text_lines(p.runs) {
            if let Some(key) = running_key(&l, p.height) {
                seen.entry(key).or_default().push((l.middle(), p.page));
            } else {
                let key = furniture_key(&l.text);
                let cost = key.len() + BODY_KEY_COST;
                if body_bytes + cost <= BODY_KEY_BYTES && body.insert(key) {
                    body_bytes += cost;
                }
            }
        }
    }
    #[cfg(test)]
    BODY_KEPT.with(|k| k.set(body.len()));
    let mut running: Vec<Running> = Vec::new();
    let mut two_page: BTreeSet<String> = BTreeSet::new();
    for ((band, key), mut at) in seen {
        if key.chars().any(char::is_alphabetic) && body.contains(&key) {
            continue;
        }
        at.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Heights within Y_STEP of the lowest one of a group are one height.
        let mut i = 0;
        while i < at.len() {
            let low = at[i].0;
            let mut j = i;
            let mut pages_at = BTreeSet::new();
            while j < at.len() && at[j].0 - low <= Y_STEP {
                pages_at.insert(at[j].1);
                j += 1;
            }
            let both = pages.len() == 2 && band == Band::Bottom && pages_at.len() == 2;
            if both && pages_at.len() < REPEATS {
                two_page.insert(key.clone());
            }
            if pages_at.len() >= REPEATS || both {
                running.push(Running {
                    key: key.clone(),
                    band,
                    pages: pages_at.into_iter().collect(),
                    at: [low, at[j - 1].0],
                });
            }
            i = j;
        }
    }
    running.sort_by(|a, b| {
        (a.band, &a.key)
            .cmp(&(b.band, &b.key))
            .then(a.at[0].total_cmp(&b.at[0]))
    });
    let index = RunningIndex::new(&running);
    let two_page_keys: HashSet<String> = two_page.iter().cloned().collect();
    let mut legends: BTreeMap<u32, Legend> = BTreeMap::new();
    // Ink characters by size (half points), and the sum of their exact sizes.
    let mut chars: HashMap<i32, (usize, f64)> = HashMap::new();
    let count_run = |chars: &mut HashMap<i32, (usize, f64)>, text: &str, size: f32| {
        let n = ink(text);
        let e = chars.entry(size_key(size)).or_default();
        e.0 += n;
        e.1 += n as f64 * f64::from(size);
    };
    for p in pages {
        let height = p.height;
        let all = text_lines(p.runs);
        let bands = index.bands(&all, height, &[]);
        for (l, b) in all.iter().zip(&bands) {
            if b.is_some() && two_page_line(l, height, &two_page_keys) {
                for r in l.runs {
                    count_run(&mut chars, r.text(), r.size());
                }
            }
        }
        // The lines without running headers and footers, as the layout of the page sees
        // them, and where each starts among the page's runs.
        let mut lines = Vec::with_capacity(all.len());
        let mut starts = Vec::with_capacity(all.len());
        let mut at = 0;
        for (l, b) in all.into_iter().zip(&bands) {
            if b.is_none() {
                starts.push(at);
            }
            at += l.runs.len();
            if b.is_none() {
                lines.push(l);
            }
        }
        // A legend of footnotes is no body text: a dense legend in a tiny size would make
        // every line a heading. A tagged file says its notes.
        let legend = if tagged {
            None
        } else {
            legend::find_lines(&lines, &starts)
        };
        let in_legend: HashSet<u32> = legend
            .as_ref()
            .map(|f| {
                f.entries
                    .iter()
                    .flat_map(|e| e.runs.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        for (l, first) in lines.iter().zip(&starts) {
            for (k, r) in l.runs.iter().enumerate() {
                if !in_legend.contains(&((first + k) as u32)) {
                    count_run(&mut chars, r.text(), r.size());
                }
            }
        }
        if let Some(found) = legend {
            legends.insert(p.page, found);
        }
    }
    // The most ink characters wins; on a tie the smaller size. Padding is no text: a data
    // line padded into its place would count its spaces. The body size is the mean exact
    // size of the winning half point, not the half point: the ratios to it then hold for a
    // page printed at any scale.
    let body_size = chars
        .iter()
        .max_by_key(|(size, (n, _))| (*n, -**size))
        .map(|(size, (n, sum))| match *n {
            0 => *size as f32 / 2.0,
            n => (sum / n as f64) as f32,
        })
        .unwrap_or(0.0);
    let mut sizes: Vec<i32> = Vec::new();
    for p in pages {
        let all = text_lines(p.runs);
        let bands = index.bands(&all, p.height, &[]);
        for (l, b) in all.iter().zip(&bands) {
            if b.is_none() && may_head(l, body_size, ends_in_amount) {
                sizes.push(size_key(l.size));
            }
        }
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes.dedup();
    FileFacts {
        body_size,
        heading_sizes: sizes.into_iter().map(|s| s as f32 / 2.0).collect(),
        running,
        two_page: two_page.into_iter().collect(),
        legends,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, Shift, TextRun};
    use crate::line::text_line;

    fn run(text: &str, line: u32, y: f32, size: f32) -> TextRun {
        TextRun {
            text: text.into(),
            x: 72.0,
            y,
            w: 100.0,
            h: size,
            size,
            font: 0,
            font_kind: FontKind::Unknown,
            bold: false,
            italic: false,
            color: None,
            shift: Shift::Base,
            line,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    fn page(page: u32, texts: Vec<TextRun>) -> PageIn {
        PageIn {
            page,
            size: [595.0, 842.0],
            turn: 0,
            texts,
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        }
    }

    fn no_amount(_: &str) -> bool {
        false
    }

    fn learned(pages: &[PageIn], tagged: bool) -> FileFacts {
        let pages: Vec<FilePage<'_, TextRun>> = pages.iter().map(FilePage::of).collect();
        learn(&pages, tagged, &no_amount)
    }

    #[test]
    fn lines_of_data_only() {
        assert!(data_only("1234567 8901 2 30"));
        assert!(data_only("01.03.2026"));
        assert!(data_only("00000000000"));
        assert!(data_only("1.234,56 EUR"));
        assert!(!data_only("Bericht 2025"));
        assert!(!data_only("EUR"));
        assert!(!data_only("Rechnung"));
    }

    #[test]
    fn bold_leads_a_heading_line() {
        let bold = |text: &str| TextRun {
            bold: true,
            ..run(text, 0, 700.0, 9.0)
        };
        let plain = |text: &str| run(text, 0, 700.0, 9.0);
        let all = [bold("Rechnung")];
        assert!(bold_led(&text_line(&all)));
        let section = [bold("SECTION 1"), plain(" (TO BE COMPLETED BY PAYEE)")];
        assert!(bold_led(&text_line(&section)));
        let label = [bold("Kundennummer:"), plain(" 00000")];
        assert!(!bold_led(&text_line(&label)));
        let value = [bold("Rechnungsnummer"), plain(" RE-2026-001")];
        assert!(!bold_led(&text_line(&value)));
        let plain_first = [plain("Seite"), bold(" 1")];
        assert!(!bold_led(&text_line(&plain_first)));
    }

    #[test]
    fn an_empty_file_has_no_facts() {
        assert_eq!(learned(&[], false), FileFacts::default());
        assert_eq!(learned(&[], true), FileFacts::default());
        assert_eq!(learned(&[page(1, Vec::new())], false), FileFacts::default());
    }

    /// A tagged file learns its body and heading sizes as an untagged one does; only the
    /// legends are left to its tags.
    #[test]
    fn a_tagged_file_has_heading_sizes() {
        let mut runs = vec![run("Kapitel Eins", 0, 650.0, 18.0)];
        runs.extend((1..=4).map(|k| {
            run(
                "die Firma liefert die bestellte Ware",
                k,
                600.0 - 14.0 * k as f32,
                10.0,
            )
        }));
        let facts = learned(&[page(1, runs)], true);
        assert!((facts.body_size - 10.0).abs() < 1e-4, "{}", facts.body_size);
        assert_eq!(facts.heading_sizes, [18.0]);
        assert!(facts.legends.is_empty());
    }

    #[test]
    fn kept_body_lines_are_bounded() {
        // Distinct mid-page lines of 13 bytes each cost 77 bytes with the set: 1,000 bytes
        // keep 12 of them.
        let letter = |k: u32| char::from(b'a' + (k % 26) as u8);
        let runs: Vec<TextRun> = (0..200)
            .map(|k| {
                let word: String = [letter(k / 676), letter(k / 26), letter(k)]
                    .iter()
                    .collect();
                run(&format!("body line {word}"), k, 500.0, 10.0)
            })
            .collect();
        learned(&[page(1, runs)], false);
        assert_eq!(BODY_KEPT.with(|k| k.get()), 1_000 / (13 + BODY_KEY_COST));
    }

    /// The legend leaves the body size only in an untagged file, where the layout takes it
    /// too; a tagged file counts it, as the text sizes of its pages do.
    #[test]
    fn only_an_untagged_legend_leaves_the_body_size() {
        let mut runs = vec![run("Wert1) und Wert2) gemeldet", 0, 700.0, 9.0)];
        runs.extend((1..=2).map(|k| {
            run(
                &format!("{k}) Muster der Erklaerung zu diesem Wert mit vielen Worten"),
                k,
                140.0 - 8.0 * k as f32,
                6.0,
            )
        }));
        let pages = [page(1, runs)];
        let untagged = learned(&pages, false);
        assert_eq!(untagged.body_size, 9.0);
        assert_eq!(
            untagged.legends.get(&1).map(|l| l.lines.clone()),
            Some(vec![1, 2])
        );
        let tagged = learned(&pages, true);
        assert_eq!(tagged.body_size, 6.0);
        assert!(tagged.legends.is_empty());
    }

    /// The body size is the exact size of the body text, not its half point: a letter printed
    /// at 0.675 sets its body of 7 points at 4.725, which a half point would make 4.5, and a
    /// name of 6.75 points (10 at full size, under 1.5 times the body there) would then be
    /// 1.5 times the body.
    #[test]
    fn the_body_size_is_exact() {
        let runs: Vec<TextRun> = (0..4)
            .map(|k| {
                run(
                    "die Firma liefert die bestellte Ware",
                    k,
                    700.0 - 6.0 * k as f32,
                    4.725,
                )
            })
            .collect();
        let facts = learned(&[page(1, runs)], false);
        assert!(
            (facts.body_size - 4.725).abs() < 1e-4,
            "{}",
            facts.body_size
        );
        assert!(6.75 < HEADING_PLAIN_RATIO * facts.body_size);
    }

    /// A page of a file of `n` pages at `scale`: a running header, a page number in the
    /// footer, a heading in 18 pt and one in 16 pt, body lines in 10 pt with a raised
    /// reference, and a legend of one footnote in 7 pt.
    fn scaled_page(k: u32, scale: f32) -> PageIn {
        let s = |v: f32| v * scale;
        let mut texts = Vec::new();
        let mut at = |text: &str, x: f32, y: f32, size: f32| {
            let line = texts.len() as u32;
            texts.push(TextRun {
                x: s(x),
                w: s(text.len() as f32 * size * 0.5),
                ..run(text, line, s(y), s(size))
            });
        };
        at("Muster GmbH Jahresbericht", 72.0, 800.0, 9.0);
        at("Kapitel Eins", 72.0, 650.0, 18.0);
        at("Abschnitt Zwei", 72.0, 620.0, 16.0);
        for i in 0..6 {
            at(
                "die Firma liefert die bestellte Ware an Erika Mustermann",
                72.0,
                590.0 - 14.0 * i as f32,
                10.0,
            );
        }
        // A raised reference after `Mustermann`, on the line of the last body line.
        let last = texts.len() as u32 - 1;
        texts.push(TextRun {
            x: s(72.0 + 56.0 * 5.0),
            w: s(3.0),
            line: last,
            ..run("1", last, s(520.0 + 3.0), s(6.0))
        });
        let mut at = |text: &str, x: f32, y: f32, size: f32| {
            let line = texts.len() as u32;
            texts.push(TextRun {
                x: s(x),
                w: s(text.len() as f32 * size * 0.5),
                ..run(text, line, s(y), s(size))
            });
        };
        at("1 Muster der Erklaerung", 72.0, 200.0, 7.0);
        at(&format!("Seite {k}"), 280.0, 40.0, 9.0);
        PageIn {
            size: [s(595.0), s(842.0)],
            ..page(k, texts)
        }
    }

    /// The same file at 0.75x and 1.5x: the facts are equal but for sizes and places, which
    /// scale.
    #[test]
    fn facts_are_the_same_at_every_scale() {
        let at = |scale: f32| {
            let pages: Vec<PageIn> = (1..=3).map(|k| scaled_page(k, scale)).collect();
            learned(&pages, false)
        };
        let one = at(1.0);
        assert!((one.body_size - 10.0).abs() < 1e-4, "{}", one.body_size);
        assert_eq!(one.heading_sizes, [18.0, 16.0]);
        let keys: Vec<(&str, Band, &[u32])> = one
            .running
            .iter()
            .map(|r| (r.key.as_str(), r.band, r.pages.as_slice()))
            .collect();
        assert_eq!(
            keys,
            [
                ("Muster GmbH Jahresbericht", Band::Top, &[1, 2, 3][..]),
                ("Seite #", Band::Bottom, &[1, 2, 3][..]),
            ]
        );
        assert!(one.two_page.is_empty());
        assert_eq!(one.legends.keys().copied().collect::<Vec<_>>(), [1, 2, 3]);
        for scale in [0.75, 1.5] {
            let other = at(scale);
            let close = |a: f32, b: f32| (a * scale - b).abs() < 1e-3 * scale.max(1.0);
            assert!(close(one.body_size, other.body_size), "{scale}");
            assert_eq!(one.heading_sizes.len(), other.heading_sizes.len());
            for (a, b) in one.heading_sizes.iter().zip(&other.heading_sizes) {
                assert!(close(*a, *b), "{scale}: {a} {b}");
            }
            assert_eq!(one.running.len(), other.running.len());
            for (a, b) in one.running.iter().zip(&other.running) {
                assert_eq!((&a.key, a.band, &a.pages), (&b.key, b.band, &b.pages));
                assert!(
                    close(a.at[0], b.at[0]) && close(a.at[1], b.at[1]),
                    "{scale}"
                );
            }
            assert_eq!(one.two_page, other.two_page);
            assert_eq!(one.legends.len(), other.legends.len());
            for (p, a) in &one.legends {
                let b = &other.legends[p];
                assert_eq!(a.lines, b.lines);
                assert_eq!(a.raised, b.raised);
                assert_eq!(a.entries.len(), b.entries.len());
                for (x, y) in a.entries.iter().zip(&b.entries) {
                    assert_eq!((x.mark, x.cut, &x.runs), (y.mark, y.cut, &y.runs));
                    assert!(close(x.x, y.x) && close(x.top, y.top) && close(x.h, y.h));
                }
            }
        }
    }

    /// A line at the foot of both pages of a file of two is a footer, a letterhead at the top
    /// of both is content.
    #[test]
    fn a_footer_on_both_pages_of_two_is_running() {
        let pages: Vec<PageIn> = (1..=2)
            .map(|k| {
                page(
                    k,
                    vec![
                        run("Muster GmbH Rechnung", 0, 800.0, 13.0),
                        run("die Ware", 1, 600.0, 10.0),
                        run("Bank DE00 0000 0000 0000 0000 00", 2, 40.0, 8.0),
                    ],
                )
            })
            .collect();
        let facts = learned(&pages, false);
        assert_eq!(facts.two_page, ["Bank DE## #### #### #### #### ##"]);
        let keys: Vec<(&str, Band)> = facts
            .running
            .iter()
            .map(|r| (r.key.as_str(), r.band))
            .collect();
        assert_eq!(keys, [("Bank DE## #### #### #### #### ##", Band::Bottom)]);
        // Its ink still counts for the body size (8 pt, not 10 pt): the plain 13 pt title is
        // a heading, as it is 1.5 times that body.
        assert!((facts.body_size - 8.0).abs() < 1e-4, "{}", facts.body_size);
        assert_eq!(facts.heading_sizes, [13.0]);
    }

    /// A header repeated at two heights is two groups, each with its own range.
    #[test]
    fn running_lines_keep_the_height_of_each_group() {
        let pages: Vec<PageIn> = (1..=6)
            .map(|k| {
                let y = if k <= 3 { 800.0 } else { 780.0 };
                page(
                    k,
                    vec![
                        run("Muster GmbH", 0, y + (k % 2) as f32, 9.0),
                        run("die Firma liefert die bestellte Ware", 1, 600.0, 10.0),
                    ],
                )
            })
            .collect();
        let facts = learned(&pages, false);
        let groups: Vec<([f32; 2], &[u32])> = facts
            .running
            .iter()
            .map(|r| (r.at, r.pages.as_slice()))
            .collect();
        assert_eq!(
            groups,
            [
                ([784.5, 785.5], &[4, 5, 6][..]),
                ([804.5, 805.5], &[1, 2, 3][..]),
            ]
        );
        // A line of the key between the two ranges is no running header.
        let index = RunningIndex::new(&facts.running);
        let between = [run("Muster GmbH", 0, 790.0, 9.0)];
        assert_eq!(index.bands(&text_lines(&between), 842.0, &[]), [None]);
        let inside = [run("Muster GmbH", 0, 800.0, 9.0)];
        assert_eq!(
            index.bands(&text_lines(&inside), 842.0, &[]),
            [Some(Band::Top)]
        );
    }
}
