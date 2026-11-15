// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The pre-pass reading of a page (the readers that read a page before the region engine
//! did), run once per page before it is segmented, as given-region pre-passes. Each class of
//! reader whose reading by area loses content stays a pre-pass until it moves into the
//! engine: its tables are given regions of the reader's kind, certainty `Rule`, with the
//! reader's own signal and found-by note. Every class of the untagged readers is such a
//! pre-pass, and every family keeps its refusals with it (text it refused stays where it
//! stands, with its notes, and no typer reads it); a class that moves into the engine has
//! its call taken out of [`read`].
//!
//! The readers run in a fixed order on one copy of the page's flow, so they see the lines they
//! saw before and veto each other as before: the running headers and footers and the footnote
//! legend are taken out first, then the underlined titles of a form page, the box pairs, and
//! the tables without tags (the columns of totals beside ruled sections, ruled grids, header
//! bands, rows of fields and their stacks, then the readers without rules), with the page's
//! segments made once on its drawn structure, as today.

use layoutregions::PageIn;
use layoutregions::file::Band;
use rezzer_model::LossReason;

use super::{Context, Drawn, Flow, Run, Slot, footnotes, lines};
use crate::tables::{self, BoxTitle, Planned, Unsure};

/// The families of the pre-pass readers by the call that runs them: their refusals (the text they
/// refused, with why, and their notes) cannot be told apart below a family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    /// The underlined titles of a form page.
    Titles,
    /// The box pairs.
    Boxes,
    /// The readers on drawn rules: side totals, ruled grids, header bands, rows of fields and
    /// their stacks.
    Ruled,
    /// The readers without rules.
    Borderless,
}

/// The signal of the text the family `f` refused and keeps as it stands.
pub(super) fn refused_id(f: Family) -> &'static str {
    match f {
        Family::Titles => "pdf.underlined_title.refused",
        Family::Boxes => "pdf.box_pairs.refused",
        Family::Ruled => "pdf.ruled.refused",
        Family::Borderless => "pdf.borderless.refused",
    }
}

/// A part of the pre-pass reading given as a region: a table or a title (placed as its reader made
/// it, taken once), or text a family refused, with why.
pub(super) enum Part {
    Table(Option<Planned>),
    Title(Option<BoxTitle>),
    Refused(Family, &'static str),
}

/// What the pre-pass readers found on a page: its tables in the order the readers made them, the
/// titles of boxes and bars, and the copy of the flow they left (a run cut at a field edge or
/// a column line may have left a piece in it). Each refusal and note with its family.
#[derive(Default)]
pub(super) struct Reading {
    pub tables: Vec<Planned>,
    pub titles: Vec<(Family, BoxTitle)>,
    pub left: Flow,
    /// The notes of the readers, with why: what they refused, left out or did not look at.
    pub notes: Vec<(Family, String, LossReason)>,
    pub unsure: Vec<(Family, Unsure)>,
    /// The text the readers refused and keep as it stands: the ids of its runs and why.
    pub doubts: Vec<(Family, Vec<u32>, &'static str)>,
    /// What a bound or the stop cut short in the segments the readers read by.
    pub issues: Vec<layoutregions::Issue>,
    /// The cells of rows of fields the ruled pass read apart: the ids of each cell's runs
    /// (see `tables::Found::cells`).
    pub cells: Vec<Vec<u32>>,
    /// The runs whose lines stay lines of their own, by their ids (see
    /// `tables::Found::own_lines`).
    pub own_lines: Vec<u32>,
}

/// Today's reading of the page whose flow (the tables of its tag tree taken out) is `flow`:
/// `geometry` is false on a tagged page whose tag tree holds, where no pre-pass reader ran on
/// the drawn structure. `page_in` is the page as the region engine takes it.
#[allow(clippy::too_many_arguments)]
pub(super) fn read(
    page: u32,
    height: f32,
    flow: &Flow,
    page_in: &PageIn,
    drawn: &Drawn<'_>,
    cx: &Context<'_>,
    geometry: bool,
    stop: &dyn Fn() -> bool,
) -> Reading {
    let mut out = Reading::default();
    if !geometry || flow.runs.is_empty() {
        return out;
    }
    let mut copy = flow.copied();
    // The running headers and footers: one band of each per page; text the tag tree places is
    // content, wherever it stands.
    let footers: Vec<Run> = {
        let all = lines(&copy.runs);
        let content: Vec<bool> = if cx.tree.is_some() {
            let mut at = 0;
            all.iter()
                .map(|l| {
                    let s = &copy.slots[at..at + l.runs.len()];
                    at += l.runs.len();
                    s.iter().any(|s| *s != Slot::Free)
                })
                .collect()
        } else {
            vec![false; all.len()]
        };
        let by_line = cx.style.furniture(&all, height, &content);
        let mut bands = Vec::with_capacity(copy.runs.len());
        for (l, b) in all.iter().zip(by_line) {
            bands.extend(std::iter::repeat_n(b, l.runs.len()));
        }
        drop(all);
        copy.take(|i, _| bands[i].is_some())
            .into_iter()
            .zip(bands.iter().flatten())
            .filter(|(_, b)| **b == Band::Bottom)
            .map(|(t, _)| t.run)
            .collect()
    };
    // The footnote legend of an untagged page: a legend set in columns is no table.
    if cx.tree.is_none() {
        let _ = footnotes::take(page, &mut copy, cx.style.legends.get(&page));
    }
    let text_sizes = {
        let two_page_lines = lines(&footers);
        let two_page = two_page_lines
            .iter()
            .filter(|l| cx.style.two_page_line(l, height))
            .flat_map(|l| l.runs.iter());
        layoutregions::file::text_sizes(copy.runs.iter().chain(two_page))
    };
    // A bold title alone over a rule no grid takes, and the lines under it, on a page drawn as
    // a form without form fields.
    if drawn.widgets.is_empty() && tables::form_drawn(drawn.grids(), drawn.boxes) {
        let (titles, halted) =
            tables::underlined_titles(&mut copy, (drawn.rules, drawn.grids()), stop);
        out.titles
            .extend(titles.into_iter().map(|t| (Family::Titles, t)));
        if let Some(why) = halted {
            out.notes.push((
                Family::Titles,
                format!(
                    "underlined titles on page {page} are looked for only in part: {why}; the \
                     text past that is read as it stands"
                ),
                LossReason::Limit,
            ));
        }
    }
    // Drawn boxes of a label over its value, before the tables: a row of them is no header
    // band. A page with form fields is the form's.
    if drawn.widgets.is_empty() {
        let (found, titles, (unsure, doubts), skipped) = tables::box_pairs(
            page,
            &mut copy,
            drawn.boxes,
            drawn.grids(),
            &text_sizes,
            stop,
        );
        out.tables.extend(found);
        let b = Family::Boxes;
        out.titles.extend(titles.into_iter().map(|t| (b, t)));
        out.notes
            .extend(unsure.into_iter().map(|w| (b, w, LossReason::Uncertain)));
        out.doubts
            .extend(doubts.into_iter().map(|(ids, why)| (b, ids, why)));
        out.notes.extend(skipped.map(|w| (b, w, LossReason::Limit)));
    }
    // On a page with form fields, boxes and aligned labels are the form: the pre-pass table
    // readers read no table there (as before the switch); the fields are the form layer's.
    if !drawn.widgets.is_empty() {
        out.left = copy;
        return out;
    }
    // The tables without tags, on the segments of the page's drawn structure.
    let segments = layoutregions::segment::segment_drawn(
        page_in,
        drawn.structure,
        &rezzer_layout::pdf::engine_limits(cx.budget),
        stop,
    );
    let (found, mark) = tables::untagged_marked(page, &mut copy, drawn, &segments, height, stop);
    out.issues = segments.issues;
    let by = |k: usize, ruled: usize| {
        if k < ruled {
            Family::Ruled
        } else {
            Family::Borderless
        }
    };
    out.tables.extend(found.tables);
    out.cells = found.cells;
    out.own_lines = found.own_lines;
    out.unsure.extend(
        found
            .unsure
            .into_iter()
            .enumerate()
            .map(|(k, u)| (by(k, mark.unsure), u)),
    );
    out.doubts.extend(
        found
            .doubts
            .into_iter()
            .enumerate()
            .map(|(k, (ids, why))| (by(k, mark.doubts), ids, why)),
    );
    for (k, w) in found.skipped.into_iter().enumerate() {
        out.notes.push((by(k, mark.skipped), w, LossReason::Limit));
    }
    for (k, w) in found.uncertain.into_iter().enumerate() {
        out.notes
            .push((by(k, mark.uncertain), w, LossReason::Uncertain));
    }
    // The grids, bands and regions refused without a note: one loss per kind and family.
    let (ruled_quiet, borderless_quiet): (Vec<_>, Vec<_>) = found
        .quiet
        .into_iter()
        .enumerate()
        .partition(|(k, _)| *k < mark.quiet);
    for (family, quiet) in [
        (Family::Ruled, ruled_quiet),
        (Family::Borderless, borderless_quiet),
    ] {
        let quiet = quiet.into_iter().map(|(_, q)| q).collect();
        for w in super::refused_quietly(page, quiet) {
            out.notes.push((family, w, LossReason::Uncertain));
        }
    }
    out.left = copy;
    out
}
