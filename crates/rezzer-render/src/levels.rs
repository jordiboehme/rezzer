// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which notes a [`NoteLevel`] shows, and the line under `## Not read` that says what was
//! left out.
//!
//! - `full` shows every note.
//! - `brief` leaves out the notes that only say how something was read: a region of a page
//!   kept as laid out because no region type is clear for it (see [`laid_out_doubt`]), the
//!   found-by note on a table, on pairs or on footnotes of a page, the
//!   doubt about the header of a sheet region that its layout line already says, and the
//!   rule words of a sheet region's layout line on the method that found it (see
//!   [`rule_shown`]). The facts of the file stay at every level: an Excel table, a pivot
//!   table, a defined name, a database range, a filter range, print titles and frozen panes.
//!   Every other note stays, where it stands: a note that is unclear stays, because a note
//!   left out that was needed is a silent gap.
//! - `none` shows no notes in the content and no `## Not read` rows: no found-by note, no
//!   method words and no note on merged cells. What the content holds stays: pictures with
//!   their need of vision, the place, header and facts of a sheet region, the rows of
//!   subtotals and totals, and the marks of what is not shown (rows and columns past a limit,
//!   a picture not extracted).
//!
//! In `brief` the line under `## Not read` counts only the rows of `## Not read` (losses) it
//! leaves out. An explanation that is no loss (a found-by note, method words) is no gap and
//! goes without a word. With no row left out there is no line, and no `## Not read` heading
//! is written for it. At `none` the line names no number and is written whenever a note is
//! dropped: a row, a found-by note or the note on merged cells. The method words of a sheet
//! layout line (see [`rule_shown`]) are explanation, not a note: they are not counted, and
//! they go at `none` with no line, as found-by words go at `brief`.

use std::borrow::Cow;
use std::collections::HashSet;

use rezzer_model::{
    BOX_FOUND_BY, Block, Document, FOOTNOTE_FOUND_BY, FOOTNOTE_LINKED, FOOTNOTES_LINKED,
    FOUND_BY_ROLE, KEPT_AS_LAID_OUT, LAID_OUT_ABOUT_EQUALLY, LAID_OUT_NO_EVIDENCE,
    LAID_OUT_NOT_CLEARLY, LayoutKind, Loss, LossReason, MARKED_IN_JSON_ONLY, SIDE_FOUND_BY,
    STRIP_FOUND_BY, SectionKind, Severity,
};

use crate::blocks::MAX_DEPTH;
use crate::{NoteLevel, OutputSection, RenderOptions, Selection};

/// Whether `level` shows the words of a rule that found a sheet region (rule `id`, words
/// `detail`) in its layout line. `brief` and `none` leave out the rules that only say the
/// method:
/// - `G` with gap words (`empty-row gaps`, `empty-column gaps`, `empty-row and empty-column
///   gaps`, `no gaps`, also with `, apart from a block it touches only at a corner` or `, cut
///   around a table range`); a `G` that a limit made (`rest of the sheet after ...`, `a piece
///   past the cut depth limit`) stays;
/// - `H0` (`no header found`: the line then says it itself), `H1` (`header by type change`),
///   `H2` (`bold header`), `H3` (`all-text header ...`): the header rows are in the line;
/// - `L` (`label column A`): the row labels are in the line;
/// - `K` (`column C is not a unit column`): why the region is no list of pairs.
///
/// The rules that say where a part of the region is stay, and so does any rule not named
/// here: `T` (`caption rows 1 to 2`), `T2` (`title in row 27`, `title rows only`), `R1`
/// (`section heading in row 5`), `C` (`continued in rows 12 to 40`), `N` (`notes in row 9`).
pub(crate) fn rule_shown(id: &str, detail: &str, level: NoteLevel) -> bool {
    if level == NoteLevel::Full {
        return true;
    }
    let gap = ["empty-row", "empty-column", "no gaps"]
        .iter()
        .any(|w| detail.trim().starts_with(w));
    let method = match id {
        "G" => gap,
        "H0" | "H1" | "H2" | "H3" | "L" | "K" => true,
        _ => false,
    };
    !method
}

/// Whether `level` shows a found-by paragraph of a paged reader with `text`. `brief` leaves
/// out the ones that say why a table or pairs were read so (pairs from drawn boxes or from
/// fields between rules, a column of totals beside a table) and how a footnote legend was
/// found. It keeps the one on separators drawn as marks: it says that Rezzer wrote a
/// character into values, and any found-by note it does not know.
pub(crate) fn found_by_shown(text: &str, level: NoteLevel) -> bool {
    match level {
        NoteLevel::Full => true,
        NoteLevel::Brief => !says_how_found(text),
        NoteLevel::None => false,
    }
}

/// The found-by notes `brief` leaves out: pairs from drawn boxes or from fields between rules,
/// a column of totals beside a table, and the footnote note of a page whose footnotes are all
/// linked to their markers. A footnote note that says a marker is not linked stays: a link is
/// missing.
fn says_how_found(text: &str) -> bool {
    [BOX_FOUND_BY, STRIP_FOUND_BY, SIDE_FOUND_BY].contains(&text)
        || (text.contains(FOOTNOTE_FOUND_BY)
            && (text.ends_with(FOOTNOTE_LINKED) || text.ends_with(FOOTNOTES_LINKED)))
}

/// Whether `level` shows the note under a table that names its merged cells. Every level
/// but `none` does: it says that the cells a merged cell covers are empty.
pub(crate) fn merge_note_shown(level: NoteLevel) -> bool {
    level != NoteLevel::None
}

/// Whether `what` is the note of a region of a page kept as laid out because no region type
/// is clear for it: no type has clear evidence, two read about equally, or one reads, but
/// not clearly enough. Such a note only says how the text was read. A region that reads as a
/// kind its reader built nothing from (`it reads as table, but most of its cells are
/// empty`) may have lost the ties of its values, and its note stays; so does a note that says
/// the bold or italic text of the region is marked in the JSON only.
fn laid_out_doubt(what: &str) -> bool {
    let Some(at) = what
        .strip_prefix("the region at ")
        .and_then(|rest| rest.find(KEPT_AS_LAID_OUT).map(|k| &rest[k..]))
    else {
        return false;
    };
    if at.contains(MARKED_IN_JSON_ONLY) {
        return false;
    }
    at.ends_with(LAID_OUT_NO_EVIDENCE)
        || at.contains(LAID_OUT_ABOUT_EQUALLY)
        || at.ends_with(LAID_OUT_NOT_CLEARLY)
}

/// Which losses a level shows as rows of `## Not read`, for one output of a document.
pub(crate) struct LossFilter {
    level: NoteLevel,
    /// The unsure grid regions the content writes, as their loss names them: the location
    /// `<sheet>!<range>` and the reasons joined by `; `. Their layout line says the same
    /// reasons (`Grid A1:F2, header unsure: no header found`).
    grids: HashSet<(String, String)>,
}

impl LossFilter {
    /// The filter for `doc` written with `opts`; `selection` names the sheets written.
    pub(crate) fn new(
        doc: &Document,
        opts: &RenderOptions,
        selection: Option<&Selection>,
    ) -> LossFilter {
        let mut filter = LossFilter {
            level: opts.notes,
            grids: HashSet::new(),
        };
        let want = |s: OutputSection| opts.sections.as_ref().is_none_or(|l| l.contains(&s));
        if opts.notes != NoteLevel::Brief || !want(OutputSection::Content) {
            return filter;
        }
        for b in doc
            .body
            .iter()
            .filter(|b| selection.is_none_or(|s| s.keeps_block(b)))
        {
            if let Block::Section {
                kind: SectionKind::Sheet,
                anchor: Some(sheet),
                blocks,
                ..
            } = b
            {
                filter.grids_in(&sheet.0, blocks, 0);
            }
        }
        filter
    }

    fn grids_in(&mut self, sheet: &str, list: &[Block], depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for b in list {
            match b {
                Block::Section { blocks, .. } => self.grids_in(sheet, blocks, depth + 1),
                Block::Table {
                    layout: Some(layout),
                    ..
                } if layout.kind == LayoutKind::Grid => {
                    let reasons = if layout.unsure.is_empty() {
                        "region unsure".to_string()
                    } else {
                        layout.unsure.join("; ")
                    };
                    self.grids
                        .insert((format!("{sheet}!{}", layout.range), reasons));
                }
                _ => {}
            }
        }
    }

    /// Whether the level shows `loss` as a row of `## Not read`.
    pub(crate) fn shows(&self, loss: &Loss) -> bool {
        match self.level {
            NoteLevel::Full => true,
            NoteLevel::Brief => !self.says_how_read(loss),
            NoteLevel::None => false,
        }
    }

    /// Whether `loss` only says how something was read, so `brief` leaves it out. Both kinds
    /// are uncertain and of severity info:
    /// - a region of a page kept as laid out because no region type is clear for it (or two
    ///   about equally); its text stays in the content as it stands;
    /// - the doubt about the header of a sheet region written as a grid, whose layout line
    ///   says the same reasons.
    ///
    /// Every other loss is shown: a warning, an error, a limit, anything not read, and every
    /// uncertain note that touches a value or says that something may be missing.
    fn says_how_read(&self, loss: &Loss) -> bool {
        if loss.why != LossReason::Uncertain || loss.severity != Severity::Info {
            return false;
        }
        laid_out_doubt(&loss.what)
            || self
                .grids
                .contains(&(loss.location.clone(), loss.what.clone()))
    }

    /// The losses of `list` the level shows, in their order.
    pub(crate) fn shown<'l>(&self, list: &'l [Loss]) -> Cow<'l, [Loss]> {
        if self.level == NoteLevel::Full {
            return Cow::Borrowed(list);
        }
        Cow::Owned(list.iter().filter(|l| self.shows(l)).cloned().collect())
    }
}

/// How many rows of `## Not read` (losses of `doc`) `filter` leaves out. At `none` one more
/// when the written content drops a note: a found-by note or the note on merged cells. The
/// line at `none` names no number; it is written when this is not 0.
pub(crate) fn left_out(
    doc: &Document,
    opts: &RenderOptions,
    selection: Option<&Selection>,
    filter: &LossFilter,
) -> usize {
    let rows = doc.losses.iter().filter(|l| !filter.shows(l)).count();
    let want = |s: OutputSection| opts.sections.as_ref().is_none_or(|l| l.contains(&s));
    if opts.notes != NoteLevel::None || !want(OutputSection::Content) {
        return rows;
    }
    let dropped = doc
        .body
        .iter()
        .filter(|b| selection.is_none_or(|s| s.keeps_block(b)))
        .any(|b| drops_a_note(b, opts.table_style, 0));
    rows + usize::from(dropped)
}

/// Whether `none` drops a note from `b` or the blocks inside it: a found-by paragraph or the
/// note on merged cells under a table.
fn drops_a_note(b: &Block, style: crate::TableStyle, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let any = |list: &[Block]| list.iter().any(|b| drops_a_note(b, style, depth + 1));
    match b {
        Block::Section { blocks, .. }
        | Block::Quote { blocks, .. }
        | Block::Callout { blocks, .. } => any(blocks),
        Block::List { items, .. } => items.iter().any(|item| any(item)),
        Block::Paragraph {
            role: Some(role), ..
        } => role == FOUND_BY_ROLE,
        Block::Table {
            header_rows,
            rows,
            layout,
            ..
        } => {
            rows.iter()
                .flatten()
                .any(|c| c.row_span > 1 || c.col_span > 1)
                && match layout {
                    Some(layout) => {
                        crate::layout::layout_merge_note(layout, *header_rows, rows, style)
                    }
                    None => crate::blocks::model_merge_note(*header_rows, rows),
                }
        }
        _ => false,
    }
}

/// The line under `## Not read` that says how many notes `level` left out, and how to see
/// them. `None` at `full` and when nothing was left out.
pub(crate) fn left_out_line(level: NoteLevel, count: usize) -> Option<String> {
    let notes = if count == 1 {
        "1 note on how the file was read is not shown".to_string()
    } else {
        format!("{count} notes on how the file was read are not shown")
    };
    match level {
        NoteLevel::Full => None,
        _ if count == 0 => None,
        NoteLevel::Brief => Some(format!(
            "{notes}. `--notes full` shows {}.",
            if count == 1 { "it" } else { "them" }
        )),
        NoteLevel::None => {
            Some("Notes are not shown. `--notes brief` or `--notes full` shows them.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_left_out_line_counts_and_names_the_option() {
        assert_eq!(left_out_line(NoteLevel::Full, 3), None);
        assert_eq!(left_out_line(NoteLevel::Brief, 0), None);
        assert_eq!(
            left_out_line(NoteLevel::Brief, 1).as_deref(),
            Some("1 note on how the file was read is not shown. `--notes full` shows it.")
        );
        assert_eq!(
            left_out_line(NoteLevel::Brief, 4).as_deref(),
            Some("4 notes on how the file was read are not shown. `--notes full` shows them.")
        );
        assert_eq!(
            left_out_line(NoteLevel::None, 2).as_deref(),
            Some("Notes are not shown. `--notes brief` or `--notes full` shows them.")
        );
        assert_eq!(left_out_line(NoteLevel::None, 0), None);
    }

    /// The rules a limit made stay in brief, also when they start like gap words would;
    /// gap words and the method rules go; a rule of an unknown id stays.
    #[test]
    fn rule_shown_keeps_the_limit_rules() {
        for detail in [
            "rest of the sheet after the cut depth limit",
            "rest of the sheet after the element limit",
            "a piece past the cut depth limit",
        ] {
            assert!(rule_shown("G", detail, NoteLevel::Brief), "{detail}");
            assert!(rule_shown("G", detail, NoteLevel::None), "{detail}");
        }
        for detail in [
            "empty-row gaps",
            "empty-column gaps",
            "empty-row and empty-column gaps, cut around a table range",
            "no gaps, apart from a block it touches only at a corner",
        ] {
            assert!(!rule_shown("G", detail, NoteLevel::Brief), "{detail}");
            assert!(rule_shown("G", detail, NoteLevel::Full), "{detail}");
        }
        for id in ["H0", "H1", "H2", "H3", "L", "K"] {
            assert!(!rule_shown(id, "words", NoteLevel::Brief), "{id}");
        }
        for id in ["T", "T2", "R1", "C", "N", "Q9"] {
            assert!(rule_shown(id, "words", NoteLevel::Brief), "{id}");
        }
    }

    #[test]
    fn note_levels_parse_by_name() {
        for level in NoteLevel::ALL {
            assert_eq!(level.name().parse::<NoteLevel>(), Ok(level));
        }
        assert_eq!(" Brief ".parse::<NoteLevel>(), Ok(NoteLevel::Brief));
        let err = "some".parse::<NoteLevel>().unwrap_err();
        assert!(err.contains("full, brief or none"), "{err}");
        assert_eq!(NoteLevel::default(), NoteLevel::Full);
    }
}
