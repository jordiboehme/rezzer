// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typer of figures: a picture, a drawn graphic with a figure caption next to it, or
//! text the table rules refuse as a chart's or a picture's.
//!
//! The segmentation gives each picture a leaf of its own (see
//! [`crate::segment::segment_page`]), and its caption stands in a text segment beside it, so
//! the caption is looked for on the page next to the segment's box, as the table rules look
//! for one next to a grid ([`captions_near`]).

use crate::extract::table::caption::captions_near;
use crate::input::{PageIn, PrimId};
use crate::region::{Bounds, Kind};
use crate::segment::drawn::PAGE_SHARE;
use crate::segment::{Segment, Source};
use crate::typers::table::refusal;
use crate::typers::{PageView, Typed, signal};

/// `figure.picture`: the segment holds a picture and no text with ink or code.
pub const FIGURE_PICTURE: u16 = 550;
/// `figure.caption`: a figure caption line ("Figure 3", "Fig. 3", "Abbildung 3", "Abb. 3")
/// stands next to a segment that holds a picture.
pub const FIGURE_CAPTION: u16 = 200;
/// `figure.drawn_caption`: a figure caption line stands next to a segment of drawn rules or
/// boxes alone (a drawn chart).
pub const FIGURE_DRAWN_CAPTION: u16 = 450;
/// `figure.on_picture`, `figure.page_picture` and `figure.caption` as refusals of the table
/// rules on a drawn grid (see [`refusal`]): a grid on a picture, of text much smaller than the
/// body text, or with a figure caption next to it or in it.
pub const FIGURE_REFUSED: u16 = 900;

/// Whether a run of `seg` has ink: text other than white space.
pub(crate) fn has_ink(page: &PageIn, seg: &Segment) -> bool {
    seg.prims.iter().any(|p| match *p {
        PrimId::Text(i) => page
            .texts
            .get(i as usize)
            .is_some_and(|r| !r.text.trim().is_empty()),
        _ => false,
    })
}

/// Whether `b` covers more than [`PAGE_SHARE`] of the page: the page's frame.
pub(crate) fn page_frame(page: &PageIn, b: &Bounds) -> bool {
    let size = (b.x1 - b.x0).max(0.0) * (b.y1 - b.y0).max(0.0);
    size > PAGE_SHARE * page.size[0] * page.size[1]
}

/// Whether a figure caption stands next to `b` (x0, y0, x1, y1) on the page of `view`: the
/// caption search of the table rules, sized by the file's body text, as the segment has none
/// of its own.
pub(crate) fn caption_next_to(view: &PageView<'_>, b: &Bounds) -> bool {
    let sizes = [view.file.body_size];
    let (_, figure) = captions_near(
        &view.page.texts,
        view.lines(),
        view.by_y(),
        [b.x0, b.y0, b.x1, b.y1],
        &sizes,
        &|_| false,
    );
    figure
}

/// Figure: the segment holds a picture and no text with ink or code (`figure.picture`; a
/// segment of a picture and a code is the parent of both, each a leaf of its own), with a figure
/// caption next to it (`figure.caption`); a segment of drawn rules or boxes alone with a
/// figure caption next to it (`figure.drawn_caption`); or the table rules refuse a drawn grid
/// as a chart or a picture of a page (`figure.on_picture`, `figure.page_picture`,
/// `figure.caption`, see [`refusal`]). A caption next to the page's frame counts for nothing:
/// such a box is next to every line.
pub fn type_figure(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.figure_of(seg)
}

/// [`type_figure`], worked out.
pub(crate) fn figure(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    crate::work::add(1 + seg.prims.len() as u64);
    let mut signals = Vec::new();
    if !has_ink(view.page, seg) {
        let pictures = seg
            .prims
            .iter()
            .filter(|p| matches!(p, PrimId::Picture(_)))
            .count();
        let codes = seg.prims.iter().any(|p| matches!(p, PrimId::Code(_)));
        let drawn = seg
            .prims
            .iter()
            .any(|p| matches!(p, PrimId::Rule(_) | PrimId::Box(_)));
        let caption = !page_frame(view.page, &seg.bounds) && caption_next_to(view, &seg.bounds);
        if pictures > 0 && !codes {
            signals.push(signal(
                "figure.picture",
                format!("{pictures} pictures"),
                FIGURE_PICTURE,
            ));
            if caption {
                signals.push(signal(
                    "figure.caption",
                    "a figure caption stands next to it".into(),
                    FIGURE_CAPTION,
                ));
            }
        } else if drawn && !codes && caption {
            signals.push(signal(
                "figure.drawn_caption",
                "a figure caption stands next to its drawing".into(),
                FIGURE_DRAWN_CAPTION,
            ));
        }
    }
    // A refusal speaks for a figure on a drawn grid only: a grid on a picture, next to a figure
    // caption or of tiny text is a chart or a picture of a page. Text alone that the table rules
    // refuse next to a caption, holding one, or on a picture is what its own lines read as: a
    // caption line in a block of text, or text over a background picture, makes no figure of
    // it. There the refusal only names why no table was read.
    if matches!(seg.source, Source::Grid { .. })
        && let Some(r) = refusal(view, seg)
        && r.kind == Kind::Figure
    {
        signals.push(signal(r.signal.id, r.signal.detail, FIGURE_REFUSED));
    }
    Typed::of(Kind::Figure, signals)
}
