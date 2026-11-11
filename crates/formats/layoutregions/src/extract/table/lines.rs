// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The lines of the cells of one table row (Task 29, vision class A). A cell's lines stay lines
//! of their own unless there is positive evidence that a line wraps into the next: then they are
//! joined, as one text. A timetable's stop cell lists the departure's stop over the arrival's,
//! beside a time cell with the departure's time over the arrival's: read with spaces, the two
//! stops make one long name and the times lose their stops; kept as lines, each stop stays on
//! the line of its time. A line break inside a wrapped sentence costs a reader nothing; a space
//! between two values loses which value is which. So only a wrap joins.
//!
//! A writer breaks a line early only to start a new value, and text that runs on goes on in a
//! small letter or after a hyphen. So a line wraps when it is full (the next line's first word
//! would not have fit after it within its column) and the text goes on (a hyphen at its end, a
//! small letter at the start of the next). A link or a URL that goes on across the line end
//! wraps too: one target is one text.
//!
//! A cell whose first line stands where a line of another cell of its row starts after a kept
//! break keeps that place: a value on the second line of a row of two-line cells stands after an
//! empty first line, so it stays with the second line of the cells beside it (a platform of the
//! arrival, never shifted onto the departure).
//!
//! One rule for every table of a page, given by the tag tree, read from drawn rules or from lines
//! without rules: the reader asks [`cell_lines`] for each row before it writes it. No row is
//! parted into rows; no value moves to another row.

use crate::line::LineRun;

/// A line of one cell stands on a line of another when their baselines are at most this share
/// of the larger size apart: half a line, so no line can stand on two.
const PAIR_NEAR: f32 = 0.5;

/// A word space is about this share of the size of its text.
const SPACE: f32 = 0.25;

/// Two runs of a line with at most this share of their size between them are one word.
const TOUCH: f32 = 0.15;

/// How a link goes on from the last run of a line to the first run of the next, as the reader
/// knows it (the engine's lines know no links).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunsOn {
    /// No link holds both runs.
    Apart,
    /// One link holds both, its text in words: the lines join with a space.
    Words,
    /// One link holds both and its text is its address going on: the lines join with nothing
    /// between them.
    Address,
}

/// How a line of a cell meets the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meets {
    /// The next line stays a line of its own.
    Break,
    /// The line wraps: the next goes on after a space (or right after a hyphen of a word).
    Wrap,
    /// A link holds both lines, its text in words: the next goes on after a space.
    Tied,
    /// A word goes on: the line is full and ends in a hyphen after a letter, and the next opens
    /// with a letter. Half a word is no value, whatever the lines beside it do.
    Word,
    /// An address goes on: the next goes on with nothing between them.
    Glue,
}

/// One line of a cell: its baseline, its larger size, where its ink ends, its text, the width
/// of its first word, and its first and last run with text (by their place in the cell).
#[derive(Debug, Clone)]
struct CellLine {
    y: f32,
    size: f32,
    x1: f32,
    text: String,
    first_word: f32,
    first: usize,
    last: usize,
}

/// The lines of the runs of one cell, in the order of the runs, and for each run the index of
/// its line. A line is a stretch of runs on one line of the page (`page` and `line`), as the
/// reader joins them: a run of white space goes with the line before it.
fn lines_of<R: LineRun>(runs: &[R]) -> (Vec<CellLine>, Vec<usize>) {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut of_run = Vec::with_capacity(runs.len());
    let mut at: Option<(u32, u32)> = None;
    for (i, r) in runs.iter().enumerate() {
        if !r.text().trim().is_empty() {
            if at != Some((r.page(), r.line())) {
                groups.push(Vec::new());
                at = Some((r.page(), r.line()));
            }
            if let Some(g) = groups.last_mut() {
                g.push(i);
            }
        }
        of_run.push(groups.len().saturating_sub(1));
    }
    let lines = groups
        .iter()
        .map(|g| {
            let mut by_x = g.clone();
            by_x.sort_by(|&a, &b| runs[a].x().total_cmp(&runs[b].x()));
            let size = g.iter().map(|&i| runs[i].size()).fold(1.0, f32::max);
            // The text of the line, a space between runs that stand apart; the width of its
            // first word, which may go on over runs that touch.
            let mut text = String::new();
            let mut first_word = 0.0f32;
            let mut in_word = true;
            let mut end = f32::MIN;
            for &i in &by_x {
                let r = &runs[i];
                let t = r.text();
                let apart = end > f32::MIN && r.x() - end > TOUCH * size;
                if apart && !text.ends_with(char::is_whitespace) {
                    text.push(' ');
                }
                if in_word {
                    let lead = if text.is_empty() { t.trim_start() } else { t };
                    if apart || lead.starts_with(char::is_whitespace) {
                        in_word = false;
                    } else {
                        let chars = lead.chars().count().max(1) as f32;
                        let word = lead.chars().take_while(|c| !c.is_whitespace()).count() as f32;
                        if text.is_empty() {
                            first_word = r.w() * word / chars;
                        } else {
                            first_word = r.x() - runs[by_x[0]].x() + r.w() * word / chars;
                        }
                        in_word = word == chars;
                    }
                }
                text.push_str(if text.is_empty() { t.trim_start() } else { t });
                end = end.max(r.x() + r.w());
            }
            CellLine {
                y: runs[g[0]].y(),
                size,
                x1: end,
                text: text.trim_end().to_string(),
                first_word,
                first: g[0],
                last: g[g.len() - 1],
            }
        })
        .collect();
    (lines, of_run)
}

/// Whether a word is an address with its scheme (`https://...`).
fn address(word: &str) -> bool {
    word.split_once("://")
        .is_some_and(|(scheme, rest)| !rest.is_empty() && scheme.chars().all(char::is_alphanumeric))
}

/// The marks of the path and the query of an address.
const ADDRESS_MARKS: [char; 9] = ['/', '-', '_', '?', '&', '=', '#', '%', '.'];

/// Whether `word`, the first word of a line under an address, goes on as the path of that
/// address: only characters an address may hold, no `@` (an e-mail address is a value of its
/// own) and no scheme (another address), a mark of a path at its start or inside it, and a
/// letter: a date, a phone number or an amount (`12.03.2020`, `030-123456`) is a value. A dot
/// or a comma at its end is the sentence's (`Tel.`), so a plain word is no path.
fn path_part(word: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-._~:/?#[]!$&'()*+,;=%".contains(c);
    let core = word.trim_end_matches(['.', ',', ';', ':', ')']);
    !word.is_empty()
        && word.chars().all(allowed)
        && !word.contains("://")
        && core.contains(ADDRESS_MARKS)
        && core.chars().any(|c| c.is_ascii_alphabetic())
}

/// How line `a` of a cell meets the next line `b`, by positive evidence only:
/// - a link holds the end of `a` and the start of `b` (`link`): one target is one text;
/// - `a` is full (the first word of `b` would not have fit after it within `limit`, where its
///   column ends) and the text goes on: `a` ends in an address and the first word of `b` goes on
///   as its path (see [`path_part`]; only that word is glued, the rest of `b` follows as it
///   stands), `a` ends in a hyphen, or `b` opens with a small letter.
///
/// A line with room left for the next word was ended by its writer: the next line is a value of
/// its own (a stop under a stop), whatever letter it opens with. A full line before a capital
/// stays a line too: that costs a reader a line break in a sentence, never a value.
fn meets(a: &CellLine, b: &CellLine, limit: f32, link: RunsOn) -> Meets {
    match link {
        RunsOn::Address => return Meets::Glue,
        RunsOn::Words => return Meets::Tied,
        RunsOn::Apart => {}
    }
    let full = a.x1 + SPACE * a.size + b.first_word > limit;
    if !full {
        return Meets::Break;
    }
    let last = a.text.split_whitespace().next_back().unwrap_or("");
    let next = b.text.split_whitespace().next().unwrap_or("");
    if address(last) && path_part(next) {
        return Meets::Glue;
    }
    let word = a
        .text
        .strip_suffix('-')
        .is_some_and(|rest| rest.ends_with(char::is_alphabetic))
        && b.text.starts_with(char::is_alphabetic);
    if word {
        return Meets::Word;
    }
    if a.text.ends_with('-') || b.text.starts_with(char::is_lowercase) {
        return Meets::Wrap;
    }
    Meets::Break
}

/// The empty lines before a cell `c` whose first line is `first`: the shown line its baseline
/// stands on in the other cells of its row (`several`: the cells of two lines or more), when
/// they say so clearly.
///
/// Each other cell with a later line on that baseline has a say: the number of breaks it keeps
/// over that line when the line starts after a kept break, or none when its lines are joined
/// across the baseline (it shows no line there). The value takes a place only when more than
/// half of those cells name the same one. When they disagree (a note at a closer line step, a
/// join in one cell over the line) there is no clear place, and the value stands in its cell
/// without one: empty lines that follow one cell would put it off the line of the others.
fn lead_of(
    first: &CellLine,
    c: usize,
    several: &[usize],
    lined: &[(Vec<CellLine>, Vec<usize>)],
    ends: &[Vec<Meets>],
) -> usize {
    let mut says: Vec<Option<usize>> = Vec::new();
    for &g in several {
        if g == c {
            continue;
        }
        let lines = &lined[g].0;
        let on = lines
            .iter()
            .position(|l| (l.y - first.y).abs() <= PAIR_NEAR * first.size.max(l.size));
        match on {
            Some(k) if k > 0 && ends[g][k - 1] == Meets::Break => says.push(Some(
                ends[g][..k].iter().filter(|m| **m == Meets::Break).count(),
            )),
            Some(k) if k > 0 => says.push(None),
            _ => {}
        }
    }
    says.iter()
        .flatten()
        .copied()
        .find(|n| 2 * says.iter().filter(|s| **s == Some(*n)).count() > says.len())
        .unwrap_or(0)
}

/// How one cell of a row is written (see [`cell_lines`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellLines {
    /// Empty lines before its first line: the kept lines of the row over it.
    pub lead: usize,
    /// The runs (by their place in the cell's runs) that start a line after a line break; every
    /// other line goes on the line before it.
    pub broken: Vec<usize>,
    /// The runs that start a line going on right after the line before it, with no space: an
    /// address that runs on.
    pub glued: Vec<usize>,
}

/// How the cells of one table row (each its runs, in reading order) are written: each cell's
/// lines stay lines of their own unless one wraps into the next (see [`meets`]; `limits` holds
/// where each cell's column ends, `link` how a link goes on from one run of a cell to another).
///
/// A line that starts where another cell of the row starts a kept line stays a line, so the
/// lines of the row's cells stay side by side. A link or an address that goes on is never cut.
///
/// A cell whose first line stands on a later line of other cells of its row begins with an
/// empty line for each break they keep over it, when they agree on that number (see
/// [`lead_of`]): a one-line value on the second line of a row of two-line cells keeps its line.
/// The lines counted are the ones shown, so a wrap over the value's line moves nothing; and a
/// value beside a line in the middle of a wrapped text has no line to keep.
pub fn cell_lines<R: LineRun>(
    cells: &[&[R]],
    limits: &[f32],
    link: &dyn Fn(&R, &R) -> RunsOn,
) -> Vec<CellLines> {
    let lined: Vec<(Vec<CellLine>, Vec<usize>)> = cells.iter().map(|c| lines_of(c)).collect();
    // For each cell, how each of its lines meets the next, by the cell's own evidence.
    let mut ends: Vec<Vec<Meets>> = lined
        .iter()
        .enumerate()
        .map(|(c, (lines, _))| {
            let limit = limits.get(c).copied().unwrap_or(f32::MAX);
            lines
                .windows(2)
                .map(|w| {
                    let on = link(&cells[c][w[0].last], &cells[c][w[1].first]);
                    meets(&w[0], &w[1], limit, on)
                })
                .collect()
        })
        .collect();
    // The row's lines: the baselines a cell starts a kept line on, from the top. A line of
    // another cell that starts on one stays a line too, whatever its own evidence: in a narrow
    // column of values each line is full (`ab 08:15` over `an 09:40` beside a stop over a
    // stop), and a line kept beside a line joined would lose which value stands with which.
    let mut row_lines: Vec<(f32, f32)> = (0..cells.len())
        .flat_map(|c| {
            let lines = &lined[c].0;
            ends[c]
                .iter()
                .enumerate()
                .filter(|(_, m)| **m == Meets::Break)
                .map(move |(k, _)| (lines[k + 1].y, lines[k + 1].size))
        })
        .collect();
    row_lines.sort_by(|a, b| b.0.total_cmp(&a.0));
    if !row_lines.is_empty() {
        for (c, (lines, _)) in lined.iter().enumerate() {
            for k in 0..ends[c].len() {
                if ends[c][k] != Meets::Wrap {
                    continue;
                }
                let l = &lines[k + 1];
                let at = row_lines.partition_point(|r| r.0 > l.y);
                let on = |r: Option<&(f32, f32)>| {
                    r.is_some_and(|r| (r.0 - l.y).abs() <= PAIR_NEAR * r.1.max(l.size))
                };
                if on(row_lines.get(at)) || on(at.checked_sub(1).and_then(|i| row_lines.get(i))) {
                    ends[c][k] = Meets::Break;
                }
            }
        }
    }
    let several: Vec<usize> = (0..cells.len())
        .filter(|&c| lined[c].0.len() >= 2)
        .collect();
    (0..cells.len())
        .map(|c| {
            let (lines, of_run) = &lined[c];
            let Some(first) = lines.first() else {
                return CellLines::default();
            };
            let lead = lead_of(first, c, &several, &lined, &ends);
            let mut seen = vec![false; lines.len()];
            let mut broken = Vec::new();
            let mut glued = Vec::new();
            for (k, &l) in of_run.iter().enumerate() {
                if l > 0 && !seen[l] && lines[l].first == k {
                    match ends[c][l - 1] {
                        Meets::Break => broken.push(k),
                        Meets::Glue => glued.push(k),
                        Meets::Wrap | Meets::Tied | Meets::Word => {}
                    }
                    seen[l] = true;
                }
            }
            CellLines {
                lead,
                broken,
                glued,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, Shift, TextRun};

    /// A run of 5 points a letter at size 10, its line told by its baseline.
    fn run(text: &str, x: f32, y: f32) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y,
            w: 5.0 * text.chars().count() as f32,
            h: 10.0,
            size: 10.0,
            bold: false,
            italic: false,
            font: 0,
            font_kind: FontKind::Proportional,
            color: None,
            shift: Shift::Base,
            line: (1000.0 - y) as u32,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    fn apart(_: &TextRun, _: &TextRun) -> RunsOn {
        RunsOn::Apart
    }

    fn of(cells: &[&[TextRun]], limits: &[f32]) -> Vec<(usize, Vec<usize>)> {
        cell_lines(cells, limits, &apart)
            .into_iter()
            .map(|c| (c.lead, c.broken))
            .collect()
    }

    /// A timetable row: the stops and the times keep their lines; a platform on the second line
    /// stands after an empty first line.
    #[test]
    fn lines_of_values_stay_lines_and_a_value_keeps_its_line() {
        let stops = [
            run("Musterstadt Hbf", 40.0, 100.0),
            run("Beispielheim", 40.0, 88.0),
        ];
        let times = [run("ab 08:15", 210.0, 100.0), run("an 09:40", 210.0, 88.0)];
        let platform = [run("4", 270.0, 88.0)];
        let train = [run("RE 4711", 290.0, 100.0)];
        let out = of(
            &[&stops[..], &times[..], &platform[..], &train[..]],
            &[210.0, 270.0, 290.0, 330.0],
        );
        assert_eq!(out, [(0, vec![1]), (0, vec![1]), (1, vec![]), (0, vec![])]);
    }

    /// Values that open with a small letter stay apart while their lines have room left: a
    /// named stop over a named stop, a named time with its platform over the next.
    #[test]
    fn named_values_with_room_left_stay_apart() {
        let stops = [
            run("ab Musterstadt Hbf", 40.0, 100.0),
            run("an Beispielheim", 40.0, 88.0),
        ];
        let times = [
            run("ab 08:15 Gleis 4", 210.0, 100.0),
            run("an 09:40 Gleis 5", 210.0, 88.0),
        ];
        let spans = [
            run("ca. 3 Std.", 330.0, 100.0),
            run("ca. 5 Std.", 330.0, 88.0),
        ];
        let out = of(
            &[&stops[..], &times[..], &spans[..]],
            &[210.0, 330.0, 420.0],
        );
        assert_eq!(out, [(0, vec![1]), (0, vec![1]), (0, vec![1])]);
    }

    /// A full line joins the next when the text goes on: in a small letter, or after a hyphen.
    #[test]
    fn a_full_line_that_goes_on_joins() {
        let small = [
            run("Wartung der Anlage im ganzen", 40.0, 100.0),
            run("und im Keller", 40.0, 88.0),
        ];
        let hyphen = [
            run("Wartung der Heizungs-", 200.0, 100.0),
            run("Anlage", 200.0, 88.0),
        ];
        let out = of(&[&small[..], &hyphen[..]], &[190.0, 310.0]);
        assert_eq!(out, [(0, vec![]), (0, vec![])]);
    }

    /// In a narrow column each line of a list of values is full, and a value may open with a
    /// small letter: beside a cell that keeps its lines, a line on the same baseline stays a
    /// line. Alone, it has only the evidence of a wrap.
    #[test]
    fn a_line_beside_a_kept_line_stays_a_line() {
        let stops = [
            run("Musterstadt Hbf", 40.0, 100.0),
            run("Beispielheim", 40.0, 88.0),
        ];
        let times = [run("ab 08:15", 210.0, 100.0), run("an 09:40", 210.0, 88.0)];
        let platform = [run("4", 255.0, 88.0)];
        let out = of(
            &[&stops[..], &times[..], &platform[..]],
            &[210.0, 255.0, 280.0],
        );
        assert_eq!(out, [(0, vec![1]), (0, vec![1]), (1, vec![])]);
        assert_eq!(of(&[&times[..]], &[255.0]), [(0, vec![])]);
        // A word that goes on after a hyphen is no value of its own: it joins beside kept lines.
        let title = [run("Leistungs-", 300.0, 100.0), run("datum", 300.0, 88.0)];
        let unit = [run("Steuer", 360.0, 100.0), run("Euro", 360.0, 88.0)];
        let out = of(&[&title[..], &unit[..]], &[355.0, 480.0]);
        assert_eq!(out, [(0, vec![]), (0, vec![1])]);
    }

    /// One cue alone joins nothing: a full line before a capital stays a line (a break in a
    /// sentence costs no value), and so does a short line before a small letter or after a
    /// hyphen (the writer ended it).
    #[test]
    fn a_full_line_or_a_cue_alone_keeps_the_break() {
        let full = [
            run("Bitte Nummer bereithalten, Ref.-Nr.", 340.0, 100.0),
            run("123456789012", 340.0, 88.0),
        ];
        let capital = [
            run("Leitung der Abteilung", 40.0, 100.0),
            run("Musterstadt", 40.0, 88.0),
        ];
        let small = [run("Schraube M8", 40.0, 76.0), run("verzinkt", 40.0, 64.0)];
        let hyphen = [run("Nord-", 40.0, 52.0), run("Seite", 40.0, 40.0)];
        assert_eq!(of(&[&full[..]], &[520.0]), [(0, vec![1])]);
        assert_eq!(of(&[&capital[..]], &[150.0]), [(0, vec![1])]);
        assert_eq!(of(&[&small[..]], &[200.0]), [(0, vec![1])]);
        assert_eq!(of(&[&hyphen[..]], &[200.0]), [(0, vec![1])]);
    }

    /// A link that goes on across a line end is one text: its address is glued, its words are
    /// joined; an address without a link goes on when its line is full and broken at a mark.
    #[test]
    fn a_link_or_an_address_that_goes_on_joins() {
        let linked = [
            run("https://example.org/muster-", 40.0, 100.0),
            run("Seite.pdf", 40.0, 88.0),
        ];
        let out = cell_lines(&[&linked[..]], &[400.0], &|_, _| RunsOn::Address);
        assert_eq!(
            (out[0].broken.clone(), out[0].glued.clone()),
            (vec![], vec![1])
        );
        let out = cell_lines(&[&linked[..]], &[400.0], &|_, _| RunsOn::Words);
        assert_eq!(
            (out[0].broken.clone(), out[0].glued.clone()),
            (vec![], vec![])
        );
        let plain = [
            run("Siehe https://example.org/muster/", 40.0, 100.0),
            run("Seite.pdf", 40.0, 88.0),
        ];
        let out = cell_lines(&[&plain[..]], &[210.0], &apart);
        assert_eq!(
            (out[0].broken.clone(), out[0].glued.clone()),
            (vec![], vec![1])
        );
        // With room left the address ended: the next line is a line of its own.
        let out = cell_lines(&[&plain[..]], &[400.0], &apart);
        assert_eq!(
            (out[0].broken.clone(), out[0].glued.clone()),
            (vec![1], vec![])
        );
    }

    /// The empty lines before a value are the kept lines over it, not the lines as set: a stop
    /// that wraps over the arrival's line does not move the platform down.
    #[test]
    fn a_wrap_over_a_paired_line_moves_nothing() {
        let stops = [
            run("Musterstadt Hbf, Nord-", 40.0, 100.0),
            run("seite", 40.0, 88.0),
            run("Beispielheim", 40.0, 76.0),
        ];
        let platform = [run("4", 260.0, 76.0)];
        let out = of(&[&stops[..], &platform[..]], &[150.0, 280.0]);
        assert_eq!(out, [(0, vec![2]), (1, vec![])]);
    }

    /// The other cells do not agree on the value's line: no empty lines. A note that keeps
    /// more lines over the arrival than the stops and the times is outvoted; with the stops
    /// alone against it there is no clear place.
    #[test]
    fn a_value_takes_the_line_most_cells_name() {
        // A join in the stop cell where the note keeps a break.
        let stops = [
            run("Musterstadt Hbf, Nord-", 40.0, 100.0),
            run("seite", 40.0, 88.0),
            run("Beispielheim", 40.0, 76.0),
        ];
        let times = [run("ab 08:15", 170.0, 100.0), run("an 09:40", 170.0, 76.0)];
        let note = [
            run("Fahrrad", 300.0, 100.0),
            run("Mitnahme", 300.0, 88.0),
            run("Gleiswechsel", 300.0, 76.0),
        ];
        let platform = [run("4", 260.0, 76.0)];
        let limits = [150.0, 260.0, 300.0, 420.0];
        let out = of(&[&stops[..], &times[..], &platform[..], &note[..]], &limits);
        assert_eq!(out[2], (1, vec![]));
        let out = of(
            &[&stops[..], &platform[..], &note[..]],
            &[150.0, 300.0, 420.0],
        );
        assert_eq!(out[1], (0, vec![]));
        // A note at a closer line step: its third line stands near the arrival's baseline.
        let stops = [
            run("Musterstadt Hbf", 40.0, 100.0),
            run("Beispielheim", 40.0, 84.0),
        ];
        let times = [run("ab 08:15", 170.0, 100.0), run("an 09:40", 170.0, 84.0)];
        let small = [
            run("Fahrrad", 300.0, 100.0),
            run("Mitnahme", 300.0, 92.0),
            run("Gleiswechsel", 300.0, 84.0),
        ];
        let platform = [run("4", 260.0, 84.0)];
        let out = of(
            &[&stops[..], &times[..], &platform[..], &small[..]],
            &limits,
        );
        assert_eq!(out[2], (1, vec![]));
        let out = of(
            &[&stops[..], &platform[..], &small[..]],
            &[150.0, 300.0, 420.0],
        );
        assert_eq!(out[1], (0, vec![]));
        // The mirror: a note with a join over the arrival shows fewer lines than three stops
        // over three times.
        let stops = [
            run("Musterstadt Hbf", 40.0, 100.0),
            run("Probedorf", 40.0, 88.0),
            run("Beispielheim", 40.0, 76.0),
        ];
        let times = [
            run("ab 08:15", 170.0, 100.0),
            run("an 09:05", 170.0, 88.0),
            run("an 09:40", 170.0, 76.0),
        ];
        let note = [
            run("Fahrrad nur an der Mitnah-", 300.0, 100.0),
            run("me", 300.0, 88.0),
            run("Gleiswechsel", 300.0, 76.0),
        ];
        let platform = [run("4", 260.0, 76.0)];
        let out = of(
            &[&stops[..], &times[..], &platform[..], &note[..]],
            &[150.0, 260.0, 300.0, 430.0],
        );
        assert_eq!(out[2], (2, vec![]));
        assert_eq!(out[3], (0, vec![2]));
    }

    /// An address without a link goes on only when the next line's first word is a part of its
    /// path. A value that opens the next line is no path: a named number, an e-mail address, a
    /// plain word. Only the path's word is glued.
    #[test]
    fn an_address_goes_on_only_with_its_path() {
        let read = |second: &str| {
            let cell = [
                run("Web: https://www.example.org/", 40.0, 100.0),
                run(second, 40.0, 88.0),
            ];
            let out = cell_lines(&[&cell[..]], &[190.0], &apart);
            (out[0].broken.clone(), out[0].glued.clone())
        };
        assert_eq!(read("Tel. 0123 456"), (vec![1], vec![]));
        // An e-mail address is never glued; it opens in a small letter under a full line, so
        // the wrap test joins it with a space, as any text that goes on.
        assert_eq!(read("info@example.org"), (vec![], vec![]));
        assert_eq!(read("Info@example.org"), (vec![1], vec![]));
        assert_eq!(read("Musterstadt"), (vec![1], vec![]));
        // A date, a phone number or an amount has no letter: it is a value, never a path.
        assert_eq!(read("12.03.2020"), (vec![1], vec![]));
        assert_eq!(read("030-123456"), (vec![1], vec![]));
        assert_eq!(read("1.677,21"), (vec![1], vec![]));
        assert_eq!(read("muster/seite.pdf und mehr"), (vec![], vec![1]));
    }

    /// A value beside wrapped text has no line to keep: beside a paragraph that wraps (centred
    /// on its second line), and beside a line in the middle of a wrap of a text that keeps a
    /// break elsewhere.
    #[test]
    fn a_value_beside_wrapped_text_takes_no_empty_line() {
        let text = [
            run("Wartung der Heizungsanlage und", 40.0, 100.0),
            run("im Haus Musterweg 1", 40.0, 88.0),
        ];
        let amount = [run("250,00", 300.0, 88.0)];
        assert_eq!(
            of(&[&text[..], &amount[..]], &[190.0, 330.0]),
            [(0, vec![]), (0, vec![])]
        );
        let text = [
            run("Leitung der Abteilung in der", 40.0, 100.0),
            run("Musterstadt mit ihren vielen", 40.0, 88.0),
            run("und guten Leuten", 40.0, 76.0),
        ];
        let date = [run("12-10-2020", 300.0, 76.0)];
        assert_eq!(
            of(&[&text[..], &date[..]], &[190.0, 360.0]),
            [(0, vec![1]), (0, vec![])]
        );
    }
}
