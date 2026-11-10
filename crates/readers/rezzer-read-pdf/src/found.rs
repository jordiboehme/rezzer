// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The "Found by:" notes of a page: what a region was read by, said in words before the first
//! block of its kind on the page (ruling P6-W: one note per kind and page). The words come from
//! the evidence of the region's decision (see [`found_by_text`]); the notes of footnotes and of
//! decimal separators drawn as marks count what they found and are written where they are
//! counted (`layout::footnotes::note`, `marks`), with the fixed words of the footnote note here.

use layoutregions::{Decision, Kind};

/// The note before the first table of drawn boxes of a page.
pub(crate) const BOX_FOUND_BY: &str = "Found by: label and value pairs from drawn boxes; the label \
     is the smaller text at the top of each box.";

/// The note before the first table of fields between rules of a page.
pub(crate) const STRIP_FOUND_BY: &str = "Found by: label and value pairs from rows of fields \
     between drawn rules; each label is the smaller text and pairs with the value beside it or \
     under it.";

/// The note before the first table on a page the column of totals adds to.
pub(crate) const SIDE_FOUND_BY: &str = "Found by: a column of amounts beside a table, drawn with \
     rules that line up with the table's rules; its amounts are read as one more column of the \
     table, and each total label in it pairs with the amount under it.";

/// How a footnote legend is found: the words of the footnote note after its count (see
/// `layout::footnotes::note`).
pub(crate) const FOOTNOTE_FOUND_BY: &str = "set in one size below all other text, each opening \
     with a note marker, the first of which also stands after a word above";

/// The "Found by:" words of a decision, from its signals: pairs from drawn boxes (a box or a
/// grid of boxes that each hold a label and its value), pairs from rows of fields between drawn
/// rules (fields in strips, or fields between the dividers of a ruled row), or a footnote legend
/// (the fixed words of its note). `None` for every other region: it is said by its block alone.
/// A region of today's reading (a column of totals beside a ruled table, a stack of strips, see
/// `layout::today`) keeps the note of its reader.
pub(crate) fn found_by_text(decision: &Decision) -> Option<&'static str> {
    let has = |id: &str| decision.signals.iter().any(|s| s.id == id);
    match decision.kind {
        Kind::LabelValue if has("label_value.strip_fields") || has("label_value.dividers") => {
            Some(STRIP_FOUND_BY)
        }
        Kind::LabelValue if has("label_value.fields") || has("label_value.field_cells") => {
            Some(BOX_FOUND_BY)
        }
        Kind::FootnoteLegend if decision.certainty != layoutregions::Certainty::Unsure => {
            Some(FOOTNOTE_FOUND_BY)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layoutregions::segment::SegId;
    use layoutregions::{Certainty, Score, Signal};

    fn decision(kind: Kind, ids: &[&'static str]) -> Decision {
        Decision {
            seg: SegId(1),
            kind,
            certainty: Certainty::Rule,
            score: Score(900),
            signals: ids
                .iter()
                .map(|&id| Signal {
                    id,
                    detail: String::new(),
                    weight: 300,
                })
                .collect(),
            alternatives: Vec::new(),
        }
    }

    /// Each note kind comes from the evidence of its region: boxes, strips and a footnote
    /// legend; a table, a pair of lines or a paragraph says nothing, nor does a region of
    /// today's reading (its reader's note stands, see `layout::today`).
    #[test]
    fn found_by_notes_come_from_evidence() {
        let box_cell = decision(
            Kind::LabelValue,
            &["label_value.fields", "label_value.label_style"],
        );
        assert_eq!(found_by_text(&box_cell), Some(BOX_FOUND_BY));
        let grid = decision(Kind::LabelValue, &["label_value.field_cells"]);
        assert_eq!(found_by_text(&grid), Some(BOX_FOUND_BY));
        let strip = decision(Kind::LabelValue, &["label_value.strip_fields"]);
        assert_eq!(found_by_text(&strip), Some(STRIP_FOUND_BY));
        let row = decision(
            Kind::LabelValue,
            &["label_value.fields", "label_value.dividers"],
        );
        assert_eq!(found_by_text(&row), Some(STRIP_FOUND_BY));
        let legend = decision(Kind::FootnoteLegend, &["footnote_legend.entries"]);
        assert_eq!(found_by_text(&legend), Some(FOOTNOTE_FOUND_BY));
        for (kind, ids) in [
            (Kind::LabelValue, &["label_value.key_values"][..]),
            (Kind::LabelValue, &["label_value.line_pairs"][..]),
            (Kind::Table, &["table.columns"][..]),
            (Kind::Totals, &["totals.under_money_columns"][..]),
            (Kind::Totals, &["pdf.side_totals"][..]),
            (Kind::LabelValue, &["pdf.strip_stack"][..]),
            (Kind::Paragraph, &["paragraph.joins"][..]),
        ] {
            assert_eq!(
                found_by_text(&decision(kind, ids)),
                None,
                "{kind:?} {ids:?}"
            );
        }
    }
}
