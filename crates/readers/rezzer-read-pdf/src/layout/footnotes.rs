// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Footnotes of an untagged page: the legend at the bottom of the page, and the references to
//! it in the text above (a star after a word, a raised number after a label).
//!
//! Each footnote becomes a footnote section `fn:p<page>-<marker>`, so its label is unique in
//! the file, and each reference whose marker the legend holds becomes a footnote reference
//! to it. A reference whose marker the legend does not hold stays text, set apart from the
//! word before it.

use std::collections::{HashMap, HashSet};

use rezzer_model::{Block, Inline, SpatialLine, SpatialPiece, SpatialSpan};

/// The legend of a page as the file pass finds it, by the page's runs as that pass saw them.
pub(super) use layoutregions::file::Legend as Found;
use layoutregions::file::{NoteMark, RefPlace, Split, word_marks};

use super::{EDGE, Flow, Run};

/// Legend entries whose markers stand this many legend sizes apart or more are in other
/// columns.
const COLUMN_GAP: f32 = 2.0;

/// The legend of a page taken out of its flow.
#[derive(Default)]
pub(super) struct Legend {
    /// Each footnote, in reading order (column by column, top down): its label, its runs
    /// and the bytes its marker takes of their trimmed text.
    pub notes: Vec<(String, Vec<Run>, usize)>,
    /// The label of each marker the legend holds.
    labels: HashMap<NoteMark, String>,
    /// Why a legend the style pass found is not used, as a loss.
    pub refused: Option<String>,
    /// The ids of the legend's runs (see [`Flow::ids`]).
    pub ids: Vec<u32>,
}

impl Legend {
    /// The marker each label of the legend stands for, as printed: `2`, `*`, `3)`.
    pub(super) fn markers(&self) -> HashMap<String, String> {
        self.labels
            .iter()
            .map(|(m, l)| (l.clone(), printed(*m)))
            .collect()
    }
}

/// A marker as printed.
fn printed(m: NoteMark) -> String {
    match m {
        NoteMark::Stars(c) => "*".repeat(usize::from(c)),
        NoteMark::Paren(n) => format!("{n})"),
        NoteMark::Raised(n) => n.to_string(),
    }
}

/// `run` cut at `split`: the words before it and the words after it as a run that starts a
/// new word.
fn cut(run: &Run, split: &Split) -> Option<(Run, Run)> {
    if let Some(k) = split.gap {
        return run.cut_at(k);
    }
    let end = run.x + run.w;
    let mut before = run.clone();
    before.text = run.text.get(..split.byte)?.trim_end().to_string();
    before.w = (split.from - run.x).max(0.0);
    let mut after = run.clone();
    after.text = format!(" {}", run.text.get(split.byte..)?);
    after.x = split.to;
    after.w = (end - split.to).max(0.0);
    Some((before, after))
}

/// `found` with every run moved to where `place` puts it; `None` when a run is not there.
fn placed_in(found: &Found, place: impl Fn(usize) -> Option<usize>) -> Option<Found> {
    let place = |i: u32| place(i as usize).map(|p| p as u32);
    let mut found = found.clone();
    for e in &mut found.entries {
        for i in &mut e.runs {
            *i = place(*i)?;
        }
    }
    for r in &mut found.raised {
        r.run = place(r.run)?;
        match &mut r.place {
            RefPlace::End => {}
            RefPlace::Inside { word, .. } | RefPlace::Above { word } => *word = place(*word)?,
        }
    }
    Some(found)
}

/// Marks the raised references of the untagged page `page` to the legend `found` (by the
/// style pass, which saw the page's runs `flow` keeps by their place) in `flow`, and takes
/// the legend's runs out of `flow`.
pub(super) fn take(page: u32, flow: &mut Flow, found: Option<&Found>) -> Legend {
    let at = std::mem::take(&mut flow.at);
    let placed = found.and_then(|f| placed_in(f, |i| at.binary_search(&i).ok()));
    flow.at = at;
    let Some(found) = placed else {
        // A legend whose lines are no longer all in the page's text is not used, and the
        // read says so.
        return Legend {
            refused: found.map(|_| {
                format!(
                    "footnotes on page {page} are not linked to their markers: the lines of \
                     their legend are not all in the page's text; the legend stays text"
                )
            }),
            ..Legend::default()
        };
    };
    let mut entries = found.entries;
    // Reading order: column by column, top down.
    let size = entries.first().map_or(1.0, |e| e.h.max(1.0));
    let mut xs: Vec<f32> = entries.iter().map(|e| e.x).collect();
    xs.sort_by(f32::total_cmp);
    let mut columns: Vec<f32> = Vec::new();
    for x in xs {
        if columns.last().is_none_or(|c| x - c >= COLUMN_GAP * size) {
            columns.push(x);
        }
    }
    let column = |x: f32| columns.iter().rposition(|c| *c <= x + EDGE).unwrap_or(0);
    entries.sort_by(|a, b| column(a.x).cmp(&column(b.x)).then(b.top.total_cmp(&a.top)));
    let mut labels: HashMap<NoteMark, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut named = Vec::with_capacity(entries.len());
    for e in &entries {
        let base = format!("p{page}-{}", e.mark.name());
        let mut label = base.clone();
        let mut k = 2;
        while !used.insert(label.clone()) {
            label = format!("{base}-{k}");
            k += 1;
        }
        labels.entry(e.mark).or_insert_with(|| label.clone());
        named.push(label);
    }
    // A raised reference to a footnote of the legend becomes a reference run after its word;
    // one to no footnote stays text where it is, set apart from its word.
    let mut inside = Vec::new();
    for r in &found.raised {
        let reference = r.run as usize;
        let Some(label) = labels.get(&r.mark) else {
            let run = &mut flow.runs[reference];
            if !run.text.starts_with(char::is_whitespace) {
                run.text.insert(0, ' ');
            }
            continue;
        };
        match r.place {
            RefPlace::End => {}
            RefPlace::Above { word } => {
                // The number goes to the end of the line above, which its runs follow.
                let word = word as usize;
                let (line, y) = (flow.runs[word].line, flow.runs[word].y);
                let run = &mut flow.runs[reference];
                run.line = line;
                run.y = y;
            }
            RefPlace::Inside { word, split } => inside.push((word as usize, split, reference)),
        }
        let run = &mut flow.runs[reference];
        run.text.clone_from(label);
        run.note = true;
    }
    let moved = place_inside(flow, inside);
    let mut owner: Vec<Option<usize>> = vec![None; flow.runs.len()];
    for (n, e) in entries.iter().enumerate() {
        for &i in &e.runs {
            owner[moved[i as usize]] = Some(n);
        }
    }
    let mut runs: Vec<Vec<Run>> = vec![Vec::new(); entries.len()];
    let taken = flow.take(|i, _| owner[i].is_some());
    let mut ids = Vec::with_capacity(taken.len());
    for (t, n) in taken.into_iter().zip(owner.into_iter().flatten()) {
        runs[n].push(t.run);
        ids.push(t.id);
    }
    Legend {
        notes: named
            .into_iter()
            .zip(runs)
            .zip(&entries)
            .map(|((label, runs), e)| (label, runs, e.cut))
            .collect(),
        labels,
        refused: None,
        ids,
    }
}

/// Puts each reference run of `inside` (word run, gap, reference run) into the white space
/// of its word run: the word run is cut at the gap and the reference stands between the two
/// parts. Returns where each run of the flow went. A word run that no longer has the gap
/// keeps the reference after it.
fn place_inside(flow: &mut Flow, mut inside: Vec<(usize, Split, usize)>) -> Vec<usize> {
    let n = flow.runs.len();
    if inside.is_empty() {
        return (0..n).collect();
    }
    inside.sort_by_key(|c| (c.0, c.1.byte));
    let refs: HashSet<usize> = inside.iter().map(|c| c.2).collect();
    let all = std::mem::take(flow);
    let mut runs: Vec<Option<Run>> = all.runs.into_iter().map(Some).collect();
    let mut moved = vec![0; n];
    let mut out = Flow::default();
    // A run goes with its slot and place, and with its own id (`r`, its index in `all`).
    let push = |out: &mut Flow, run: Run, slot, at, r: usize| {
        out.runs.push(run);
        out.slots.push(slot);
        out.at.push(at);
        out.ids.push(all.ids[r]);
        out.runs.len() - 1
    };
    let mut next = 0;
    for i in 0..n {
        if refs.contains(&i) {
            continue;
        }
        let (slot, at) = (all.slots[i], all.at[i]);
        let Some(run) = runs[i].take() else {
            continue;
        };
        moved[i] = out.runs.len();
        // The cuts of this run, in order, each in the part after the cut before: its gaps
        // and bytes count from that cut.
        let mut rest = Some(run);
        let mut last: Option<Split> = None;
        while next < inside.len() && inside[next].0 == i {
            let (_, split, r) = inside[next];
            next += 1;
            let reference = runs[r].take().expect("a reference is placed once");
            // The part after a cut starts with one space before its first word, and its gaps
            // count from the cut.
            let here = match last {
                // Two references after one word: the second follows the first.
                Some(l) if l.byte == split.byte => {
                    moved[r] = push(&mut out, reference, slot, at, r);
                    continue;
                }
                None => Some(split),
                Some(l) => (split.byte + 1).checked_sub(l.byte).map(|byte| Split {
                    byte,
                    gap: split.gap.zip(l.gap).and_then(|(g, k)| g.checked_sub(k + 1)),
                    ..split
                }),
            };
            let parts = rest.as_ref().zip(here).and_then(|(x, here)| cut(x, &here));
            match parts {
                Some((before, after)) => {
                    push(&mut out, before, slot, at, i);
                    rest = Some(after);
                    last = Some(split);
                }
                // Without the gap the reference goes after what is left of the run.
                None => {
                    if let Some(x) = rest.take() {
                        push(&mut out, x, slot, at, i);
                    }
                }
            }
            moved[r] = push(&mut out, reference, slot, at, r);
        }
        if let Some(x) = rest {
            push(&mut out, x, slot, at, i);
        }
    }
    *flow = out;
    moved
}

/// Writes the references of `blocks` to the footnotes `unwritten` (by label) as the text of
/// their markers: a stop that cut the legend leaves no reference without its footnote.
pub(super) fn unlink(blocks: &mut [Block], legend: &Legend, unwritten: &[&str]) {
    if unwritten.is_empty() {
        return;
    }
    let marks: HashMap<&str, NoteMark> = legend
        .labels
        .iter()
        .map(|(m, l)| (l.as_str(), *m))
        .collect();
    let text = |id: &str| match marks.get(id) {
        Some(NoteMark::Stars(c)) => "*".repeat(usize::from(*c)),
        Some(NoteMark::Paren(n)) => format!("{n})"),
        Some(NoteMark::Raised(n)) => format!(" {n}"),
        None => String::new(),
    };
    let fix = |inlines: &mut Vec<Inline>| {
        for i in inlines.iter_mut() {
            if let Inline::FootnoteRef { id } = i
                && unwritten.contains(&id.as_str())
            {
                *i = Inline::plain(text(id));
            }
        }
    };
    walk_inlines(blocks, &mut |i| fix(i));
    // In text kept as laid out the marker stays as printed; only the reference goes.
    walk_spatial(blocks, &mut |lines| {
        for p in lines.iter_mut().flat_map(|l| &mut l.pieces) {
            for span in &mut p.spans {
                if span
                    .footnote
                    .as_deref()
                    .is_some_and(|id| unwritten.contains(&id))
                {
                    span.footnote = None;
                }
            }
            p.spans
                .retain(|s| s.link.is_some() || s.footnote.is_some() || s.strong || s.emphasis);
        }
    });
}

/// Calls `f` with the lines of every block of `blocks` kept as laid out, also inside
/// sections, quotes, callouts and list items.
fn walk_spatial(blocks: &mut [Block], f: &mut dyn FnMut(&mut Vec<SpatialLine>)) {
    for b in blocks {
        match b {
            Block::Spatial { lines, .. } => f(lines),
            Block::List { items, .. } => items.iter_mut().for_each(|i| walk_spatial(i, f)),
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => walk_spatial(blocks, f),
            _ => {}
        }
    }
}

/// Calls `f` with every list of inlines of `blocks` outside links: paragraphs, headings, list
/// items, table cells and the blocks inside sections, quotes and callouts.
fn walk_inlines(blocks: &mut [Block], f: &mut dyn FnMut(&mut Vec<Inline>)) {
    for b in blocks {
        match b {
            Block::Heading { inlines, .. } | Block::Paragraph { inlines, .. } => f(inlines),
            Block::List { items, .. } => items.iter_mut().for_each(|i| walk_inlines(i, f)),
            Block::Table { rows, .. } => rows.iter_mut().flatten().for_each(|c| f(&mut c.inlines)),
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => walk_inlines(blocks, f),
            _ => {}
        }
    }
}

/// The `found-by` note of a page with `n` footnotes written, `linked` of them with their
/// references.
pub(super) fn note(page: u32, n: usize, linked: usize) -> Option<String> {
    let how = crate::found::FOOTNOTE_FOUND_BY;
    let what = match n {
        0 => return None,
        1 => format!("1 footnote at the bottom of page {page} is read from its legend, {how}"),
        _ => {
            format!("{n} footnotes at the bottom of page {page} are read from their legend, {how}")
        }
    };
    let refs = match (n, linked) {
        (1, 1) => "; the marker above is linked to it".to_string(),
        (1, 0) => "; no marker above is linked to it".to_string(),
        (_, 0) => "; no marker above is linked to them".to_string(),
        (n, l) if n == l => "; the markers above are linked to them".to_string(),
        (_, l) => format!("; {l} of them are linked to their markers above"),
    };
    Some(what + &refs)
}

/// The labels of the footnote references of `blocks`.
pub(super) fn referenced(blocks: &mut [Block]) -> HashSet<String> {
    let mut found = HashSet::new();
    walk_inlines(blocks, &mut |inlines| {
        for i in inlines.iter() {
            if let Inline::FootnoteRef { id } = i {
                found.insert(id.clone());
            }
        }
    });
    walk_spatial(blocks, &mut |lines| {
        let spans = lines.iter().flat_map(|l| &l.pieces).flat_map(|p| &p.spans);
        found.extend(spans.filter_map(|s| s.footnote.clone()));
    });
    found
}

/// Turns the markers that end a word in the text of `blocks` into references to the
/// footnotes of `legend`: `Betrag*` becomes `Betrag` and a reference.
pub(super) fn link_words(blocks: &mut [Block], legend: &Legend) {
    if legend.labels.is_empty() {
        return;
    }
    walk_inlines(blocks, &mut |i| link_inlines(i, legend));
    walk_spatial(blocks, &mut |lines| {
        for p in lines.iter_mut().flat_map(|l| &mut l.pieces) {
            link_piece(p, legend);
        }
    });
}

/// [`link_words`] for one piece of text kept as laid out: the marker stays as printed and
/// becomes a span that refers to its footnote. A marker already a reference stays one.
fn link_piece(piece: &mut SpatialPiece, legend: &Legend) {
    let found: Vec<(usize, usize, String)> = word_marks(&piece.text)
        .into_iter()
        .filter_map(|(start, word, mark, len)| {
            let label = legend.labels.get(&mark)?;
            let end = start + word.trim_end_matches([':', ',', '.', ';']).len();
            Some((end - len, end, label.clone()))
        })
        .collect();
    if found.is_empty() {
        return;
    }
    let chars = |b: usize| u32::try_from(piece.text[..b].chars().count()).unwrap_or(u32::MAX);
    for (from, to, label) in found {
        let (start, end) = (chars(from), chars(to));
        mark_span(&mut piece.spans, start, end, label);
    }
}

/// Marks the characters `start..end` of a piece as a reference to `label`: the spans there
/// are cut so the marker is a span of its own, with the link and style of the first span it
/// was part of. A marker that is already a reference stays as it is.
fn mark_span(spans: &mut Vec<SpatialSpan>, start: u32, end: u32, label: String) {
    let over = |s: &SpatialSpan| s.start < end && start < s.end;
    if spans.iter().any(|s| over(s) && s.footnote.is_some()) {
        return;
    }
    let mut style: Option<SpatialSpan> = None;
    let mut out = Vec::with_capacity(spans.len() + 2);
    for s in spans.drain(..) {
        if !over(&s) {
            out.push(s);
            continue;
        }
        if style.is_none() {
            style = Some(s.clone());
        }
        if s.start < start {
            out.push(SpatialSpan {
                end: start,
                ..s.clone()
            });
        }
        if s.end > end {
            out.push(SpatialSpan { start: end, ..s });
        }
    }
    out.push(SpatialSpan {
        start,
        end,
        footnote: Some(label),
        ..style.unwrap_or_default()
    });
    out.sort_by_key(|s| s.start);
    *spans = out;
}

/// [`link_words`] for one list of inlines. Text inside links stays as it is.
fn link_inlines(inlines: &mut Vec<Inline>, legend: &Legend) {
    if !inlines.iter().any(|i| matches!(i, Inline::Text { .. })) {
        return;
    }
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        let Inline::Text {
            text,
            emphasis,
            strong,
            strike,
        } = inline
        else {
            out.push(inline);
            continue;
        };
        let piece = |t: &str| Inline::Text {
            text: t.to_string(),
            emphasis,
            strong,
            strike,
        };
        let mut from = 0;
        for (start, word, mark, len) in word_marks(&text) {
            let Some(label) = legend.labels.get(&mark) else {
                continue;
            };
            let kept = word.trim_end_matches([':', ',', '.', ';']).len();
            let end = start + kept;
            if end - len > from {
                out.push(piece(&text[from..end - len]));
            }
            out.push(Inline::FootnoteRef { id: label.clone() });
            from = end;
        }
        if from == 0 {
            out.push(piece(&text));
        } else if from < text.len() {
            out.push(piece(&text[from..]));
        }
    }
    *inlines = out;
}

#[cfg(test)]
mod tests {
    use super::super::Gaps;
    use super::*;

    /// The legend the file pass finds on the page of `runs`, by the runs' places.
    fn find(runs: &[Run]) -> Option<Found> {
        let mut style = super::super::Style::new(&[super::super::PageText {
            page: 1,
            height: 842.0,
            runs,
        }]);
        style.legends.remove(&1)
    }

    fn run(text: &str, line: u32, x: f32, y: f32, size: f32) -> Run {
        Run {
            page: 1,
            text: text.into(),
            x,
            y,
            w: text.len() as f32 * size * 0.5,
            h: size,
            size,
            bold: false,
            italic: false,
            family: 0,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
            note: false,
            gaps: Gaps::default(),
        }
    }

    /// A legend the style pass found whose runs the page's text no longer holds is not used,
    /// and says so (Task 7 carry: no silent refusal).
    #[test]
    fn a_legend_not_found_again_is_refused_with_a_note() {
        let runs = vec![
            run("Betrag*", 0, 40.0, 700.0, 9.0),
            run("die Werte werden monatlich gemeldet", 1, 40.0, 690.0, 9.0),
            run("* steuerfrei", 2, 40.0, 60.0, 6.0),
        ];
        let found = find(&runs);
        assert!(found.is_some());
        // The page's text without its last line: the legend's run is gone.
        let mut flow = Flow::new(runs[..2].to_vec(), vec![super::super::Slot::Free; 2]);
        let legend = take(1, &mut flow, found.as_ref());
        assert!(legend.notes.is_empty());
        let why = legend.refused.expect("a note");
        assert!(why.contains("page 1"), "{why}");
        assert_eq!(flow.runs.len(), 2);
        // Without a legend found, nothing is said.
        let mut flow = Flow::new(runs[..2].to_vec(), vec![super::super::Slot::Free; 2]);
        assert!(take(1, &mut flow, None).refused.is_none());
    }

    #[test]
    fn a_marker_put_on_the_next_line_goes_to_its_word() {
        // The raised 1 after `Musterwert` opens the next line in the line order; the 2 after
        // the run of two labels `Xa Xb` stands after `Xa`, the 3 after `Xb`.
        let mut xa = run(" Xa Xb", 2, 40.0, 700.0, 6.6);
        xa.w = 15.0;
        let runs = vec![
            run("Musterwert", 0, 40.0, 720.0, 6.6),
            run("1", 1, 73.5, 721.9, 4.7),
            run(" Kl Muster", 1, 40.0, 712.0, 6.6),
            xa,
            run("2", 2, 47.0, 701.9, 4.7),
            run("3", 2, 55.5, 701.9, 4.7),
            run("die Werte werden monatlich gemeldet", 3, 40.0, 690.0, 9.0),
            run("1 A = Muster", 4, 40.0, 70.0, 6.0),
            run("2 B = Beispiel", 5, 40.0, 62.0, 6.0),
            run("3 C = Chor", 6, 40.0, 54.0, 6.0),
        ];
        let n = runs.len();
        let found = find(&runs);
        let mut flow = Flow::new(runs, vec![super::super::Slot::Free; n]);
        let legend = take(1, &mut flow, found.as_ref());
        assert_eq!(legend.notes.len(), 3);
        let texts: Vec<(&str, u32, bool)> = flow
            .runs
            .iter()
            .map(|r| (r.text.as_str(), r.line, r.note))
            .collect();
        assert_eq!(
            texts[..6],
            [
                ("Musterwert", 0, false),
                ("p1-1", 0, true),
                (" Kl Muster", 1, false),
                (" Xa", 2, false),
                ("p1-2", 2, true),
                (" Xb", 2, false),
            ]
        );
        assert_eq!(texts[6], ("p1-3", 2, true));
    }

    #[test]
    fn references_to_unwritten_footnotes_become_text() {
        let mut legend = Legend::default();
        legend.labels.insert(NoteMark::Stars(1), "p1-star".into());
        legend.labels.insert(NoteMark::Raised(2), "p1-2".into());
        let reference = |id: &str| Inline::FootnoteRef { id: id.into() };
        let mut blocks = vec![Block::Paragraph {
            inlines: vec![
                Inline::plain("Betrag"),
                reference("p1-star"),
                Inline::plain(" Einheit"),
                reference("p1-2"),
            ],
            role: None,
            anchor: None,
        }];
        // A stop wrote the first footnote only.
        unlink(&mut blocks, &legend, &["p1-2"]);
        let Block::Paragraph { inlines, .. } = &blocks[0] else {
            unreachable!()
        };
        assert_eq!(
            inlines,
            &[
                Inline::plain("Betrag"),
                reference("p1-star"),
                Inline::plain(" Einheit"),
                Inline::plain(" 2"),
            ]
        );
    }

    #[test]
    fn two_markers_after_one_word_follow_each_other() {
        let mut word = run(" Xa Xb", 0, 40.0, 700.0, 6.6);
        word.w = 15.0;
        let runs = vec![
            word,
            run("2", 0, 47.0, 701.9, 4.7),
            run("3", 0, 49.0, 701.9, 4.7),
            run("die Werte werden monatlich gemeldet", 1, 40.0, 690.0, 9.0),
            run("2 B = Beispiel", 2, 40.0, 62.0, 6.0),
            run("3 C = Chor", 3, 40.0, 54.0, 6.0),
        ];
        let n = runs.len();
        let found = find(&runs);
        let mut flow = Flow::new(runs, vec![super::super::Slot::Free; n]);
        take(1, &mut flow, found.as_ref());
        let texts: Vec<&str> = flow.runs.iter().map(|r| r.text.as_str()).take(4).collect();
        assert_eq!(texts, [" Xa", "p1-2", "p1-3", " Xb"]);
    }

    #[test]
    fn markers_in_words_become_references() {
        let mut legend = Legend::default();
        legend.labels.insert(NoteMark::Stars(1), "p1-star".into());
        let mut inlines = vec![Inline::plain("Betrag* 1.234,56 und Summe**")];
        link_inlines(&mut inlines, &legend);
        assert_eq!(
            inlines,
            [
                Inline::plain("Betrag"),
                Inline::FootnoteRef {
                    id: "p1-star".into()
                },
                Inline::plain(" 1.234,56 und Summe**"),
            ]
        );
    }
}
