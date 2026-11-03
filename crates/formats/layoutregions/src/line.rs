// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Visual lines of text runs and the text of a line: the one place where runs become a line,
//! for the engine and for a reader that keeps runs of its own (see [`LineRun`]).

use crate::input::{GapIn, TextRun};

/// A gap wider than this many font sizes splits a line into pieces, which may stand in
/// different columns.
pub const PIECE_GAP: f32 = 0.8;

/// A label or a title differs in size from the text beside it when the larger is at least
/// this many times the smaller: half a point at 12.5 points, and less than half a point at
/// the sizes of forms (8 over 8.5 points differ). A ratio, so that it holds at every scale.
pub const LABEL_STEP: f32 = 1.04;

/// A new paragraph starts at a gap over this many times the line height.
pub const GAP_RATIO: f32 = 1.5;

/// Font sizes closer than this are the same size.
pub const SIZE_SLACK: f32 = 0.5;

/// Line ends this close, in points, are one edge.
pub const EDGE: f32 = 1.0;

/// A run that starts less than this many sizes right of where the ink of the run before it
/// ends touches it (see [`apart`]): less than a space.
const TOUCH: f32 = 0.2;

/// A run that starts this many of its sizes or more left of where the ink of the run before it
/// on its line ends is printed over that run: far more than a kerning pair takes back.
const OVERPRINT: f32 = 0.5;

/// A run of text as lines are made of it: the engine's [`TextRun`], or a reader's own run.
pub trait LineRun {
    fn text(&self) -> &str;
    /// Left edge.
    fn x(&self) -> f32;
    /// Baseline.
    fn y(&self) -> f32;
    /// Advance width.
    fn w(&self) -> f32;
    /// Rendered height.
    fn h(&self) -> f32;
    fn size(&self) -> f32;
    fn bold(&self) -> bool;
    fn italic(&self) -> bool;
    /// The family key of its font, 0 when not known.
    fn font(&self) -> u32;
    /// The run's line number in reading order; the runs of one line are next to each other.
    fn line(&self) -> u32;
    /// The run reads in another direction than the page's text (a margin note, a stamp).
    fn off_axis(&self) -> bool;
    /// The run is a reference to a footnote of its page (its text is the footnote's label):
    /// a raised marker that goes with the text before it. False when the reader does not know.
    fn note(&self) -> bool {
        false
    }
    /// The page of the run, for a list that holds runs of several pages; 0 for one page.
    fn page(&self) -> u32 {
        0
    }
    /// The white space between its words, from the glyphs, in order (see [`GapIn`]).
    fn gaps(&self) -> impl Iterator<Item = GapIn> + '_;
}

impl LineRun for TextRun {
    fn text(&self) -> &str {
        &self.text
    }
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
    }
    fn w(&self) -> f32 {
        self.w
    }
    fn h(&self) -> f32 {
        self.h
    }
    fn size(&self) -> f32 {
        self.size
    }
    fn bold(&self) -> bool {
        self.bold
    }
    fn italic(&self) -> bool {
        self.italic
    }
    fn font(&self) -> u32 {
        self.font
    }
    fn line(&self) -> u32 {
        self.line
    }
    fn off_axis(&self) -> bool {
        self.off_axis
    }
    fn gaps(&self) -> impl Iterator<Item = GapIn> + '_ {
        self.gaps.iter().copied()
    }
}

/// A borrowed run is a run: the lines of some runs of a page (a segment's) borrow them.
impl<R: LineRun> LineRun for &R {
    fn text(&self) -> &str {
        (**self).text()
    }
    fn x(&self) -> f32 {
        (**self).x()
    }
    fn y(&self) -> f32 {
        (**self).y()
    }
    fn w(&self) -> f32 {
        (**self).w()
    }
    fn h(&self) -> f32 {
        (**self).h()
    }
    fn size(&self) -> f32 {
        (**self).size()
    }
    fn bold(&self) -> bool {
        (**self).bold()
    }
    fn italic(&self) -> bool {
        (**self).italic()
    }
    fn font(&self) -> u32 {
        (**self).font()
    }
    fn line(&self) -> u32 {
        (**self).line()
    }
    fn off_axis(&self) -> bool {
        (**self).off_axis()
    }
    fn note(&self) -> bool {
        (**self).note()
    }
    fn page(&self) -> u32 {
        (**self).page()
    }
    fn gaps(&self) -> impl Iterator<Item = GapIn> + '_ {
        (**self).gaps()
    }
}

/// One visual line.
#[derive(Debug)]
pub struct TextLine<'a, R> {
    pub runs: &'a [R],
    /// The text without leading and trailing white space.
    pub text: String,
    /// The size of the run with the most characters.
    pub size: f32,
    /// The left edge of the first run.
    pub x: f32,
    /// The baseline of the run with the most characters.
    pub y: f32,
    /// The largest height of the runs.
    pub h: f32,
}

impl<R> TextLine<'_, R> {
    /// The vertical middle of the line.
    pub fn middle(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// The lines of `runs`, in order: runs of one line number (and page) next to each other.
pub fn text_lines<R: LineRun>(runs: &[R]) -> Vec<TextLine<'_, R>> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < runs.len() {
        let mut end = start + 1;
        while end < runs.len()
            && runs[end].line() == runs[start].line()
            && runs[end].page() == runs[start].page()
        {
            end += 1;
        }
        out.push(text_line(&runs[start..end]));
        start = end;
    }
    out
}

/// The line of `runs`, which must not be empty.
pub fn text_line<R: LineRun>(runs: &[R]) -> TextLine<'_, R> {
    // A run printed over the run before it, or touching it in another size or weight, is
    // another text (see [`apart`]).
    let mut text = String::new();
    let mut ink_end = f32::MIN;
    let mut before: Option<&R> = None;
    for r in runs {
        if apart(before, r, ink_end)
            && !r.text().starts_with(char::is_whitespace)
            && !text.ends_with(char::is_whitespace)
        {
            text.push(' ');
        }
        text.push_str(r.text());
        if !r.text().trim().is_empty() {
            ink_end = ink_end.max(r.x() + r.w());
            before = Some(r);
        }
    }
    // The run with the most characters sets the size and the baseline.
    let main = runs
        .iter()
        .max_by_key(|r| r.text().chars().count())
        .unwrap_or(&runs[0]);
    TextLine {
        runs,
        text: text.trim().to_string(),
        size: main.size(),
        x: runs[0].x(),
        y: main.y(),
        h: runs.iter().map(LineRun::h).fold(0.0, f32::max),
    }
}

/// Whether `c` is one of the noncharacters U+FDD0 to U+FDEF: Rezzer's own placeholders (a
/// reader puts them where it read a mark between two digits), dropped from the text of a file.
pub fn is_placeholder(c: char) -> bool {
    ('\u{FDD0}'..='\u{FDEF}').contains(&c)
}

/// Whether the text of a run may be printed over another: two letters or digits or more. A
/// lone mark (an accent drawn as a run of its own over its letter) belongs to its word, as
/// the reading order keeps it there.
fn overprints(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= 2
}

/// Whether the run `r` is another text than the run `before` it on its line, whose ink ends
/// at `ink_end`: it is printed over it (it starts [`OVERPRINT`] sizes or more left of that
/// end, with text of its own, see [`overprints`]), or it touches it (it starts less than
/// [`TOUCH`] sizes right of that end, or over it) in a larger size by a label step or in
/// another weight, the words that meet each of two letters or digits or more and one with a
/// letter, the second opening with a capital or a digit or the first ending in a colon: a label
/// and its value set against it (`Name` and `Hinterhaus`). A word cut by a change of style goes
/// on in a small letter (`Rechnungs` and `nummer`, `Muster` and `mann`): one word. A smaller run after
/// a word is a raised part of it (`21st`), a word in pieces of one style is one word, and a
/// number in pieces (a change of font inside `1234,56`) is one number.
pub fn apart<R: LineRun>(before: Option<&R>, r: &R, ink_end: f32) -> bool {
    if r.x() + OVERPRINT * r.size() < ink_end && overprints(r.text()) {
        return true;
    }
    let Some(b) = before else {
        return false;
    };
    let (last, first) = (
        b.text().split_whitespace().next_back(),
        r.text().split_whitespace().next(),
    );
    let letters = |w: Option<&str>| w.is_some_and(|w| w.chars().any(char::is_alphabetic));
    // A new word opens with a capital or a digit (`Hinterhaus`, `4711`), or follows a label's
    // colon; the rest of a word cut by a change of style opens with a small letter (`nummer`).
    let opens = first
        .is_some_and(|w| w.starts_with(|c: char| c.is_uppercase() || c.is_ascii_digit()))
        || last.is_some_and(|w| w.ends_with(':'));
    opens
        && r.x() < ink_end + TOUCH * r.size().max(b.size())
        && (r.size() >= LABEL_STEP * b.size() || r.bold() != b.bold())
        && last.is_some_and(overprints)
        && first.is_some_and(overprints)
        && (letters(last) || letters(first))
}

/// The ink characters of `text`: white space is not counted.
pub fn ink(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

/// Rounds a font size to half points.
pub fn size_key(size: f32) -> i32 {
    (size * 2.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, Shift};

    fn run(text: &str, x: f32, w: f32, size: f32, bold: bool) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y: 700.0,
            w,
            h: size,
            size,
            font: 0,
            font_kind: FontKind::Unknown,
            bold,
            italic: false,
            color: None,
            shift: Shift::Base,
            line: 0,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    /// A label and its value set against it in another weight are two texts; a word cut by a
    /// change of style goes on as one word.
    #[test]
    fn a_line_parts_texts_that_touch() {
        let label = [
            run("Name", 40.0, 20.0, 9.0, true),
            run("Hinterhaus", 60.0, 45.0, 9.0, false),
        ];
        assert_eq!(text_line(&label).text, "Name Hinterhaus");
        let word = [
            run("Rechnungs", 40.0, 40.0, 9.0, true),
            run("nummer", 80.0, 30.0, 9.0, false),
        ];
        assert_eq!(text_line(&word).text, "Rechnungsnummer");
        let l = text_line(&label);
        assert_eq!((l.size, l.x, l.y, l.h), (9.0, 40.0, 700.0, 9.0));
        assert_eq!(l.middle(), 704.5);
    }

    #[test]
    fn lines_follow_the_line_numbers() {
        let mut runs = vec![
            run("Eins", 40.0, 20.0, 9.0, false),
            run(" zwei", 60.0, 20.0, 9.0, false),
            run("Drei", 40.0, 20.0, 9.0, false),
        ];
        runs[2].line = 1;
        let lines = text_lines(&runs);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Eins zwei", "Drei"]);
        assert_eq!(ink(" a b "), 2);
        assert_eq!(size_key(9.26), 19);
    }
}
