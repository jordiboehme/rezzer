// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the text pass of a page read sees, in default code so it is tested without the models:
//! the image, and the extra text region `read_page` adds for content outside every drawing on a
//! page without a text region.

use image::RgbImage;

use crate::prepare::{blank_boxes, content_box, keep_boxes};
use crate::regions::grid_box;
use crate::types::{Region, RegionKind};

/// The note on a text region that `read_page` adds for content outside every drawing on a page
/// without a text region.
pub const POSSIBLE_TEXT_NOTE: &str = "possible missed text";

/// The certainty of such a region: the detectors saw no text there, so it is low; the note says
/// why.
pub const POSSIBLE_TEXT_CERTAINTY: f32 = 0.5;

/// What the text pass sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextScope {
    /// Only the given text regions (`read` with caller regions).
    Regions,
    /// The whole page with the drawings blanked (`read_page`).
    Page,
}

/// The text pass of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct TextPass {
    /// The page image the text pass reads (before `prepare_page`).
    pub image: RgbImage,
    /// A text region to add (scope Page, no text region on the page, content outside every
    /// drawing): its pixel box.
    pub new_region_px: Option<[u32; 4]>,
}

/// The text pass for a page with these text and drawn regions (pixel boxes), or None when there
/// is nothing to read:
/// - Regions: a white page with only the text boxes (plus 16 px), then the drawn boxes (plus
///   16 px) blanked on top; None without a text region.
/// - Page: the whole page with the drawn boxes (plus 16 px) blanked. Without a text region, the
///   content of that image ([`content_box`]) becomes a new region; None when there is none.
pub fn text_pass(
    page: &RgbImage,
    scope: TextScope,
    text_px: &[[u32; 4]],
    drawn_px: &[[u32; 4]],
) -> Option<TextPass> {
    match scope {
        TextScope::Regions => (!text_px.is_empty()).then(|| TextPass {
            image: blank_boxes(&keep_boxes(page, text_px), drawn_px),
            new_region_px: None,
        }),
        TextScope::Page => {
            let image = blank_boxes(page, drawn_px);
            if !text_px.is_empty() {
                return Some(TextPass {
                    image,
                    new_region_px: None,
                });
            }
            let b = content_box(&image)?;
            Some(TextPass {
                image,
                new_region_px: Some(b),
            })
        }
    }
}

/// The "possible missed text" region for a pixel box of a `width` x `height` page.
pub fn possible_text_region(b: [u32; 4], width: u32, height: u32) -> Region {
    Region {
        bbox: grid_box(b.map(i64::from), width, height),
        kind: RegionKind::Text,
        certainty: POSSIBLE_TEXT_CERTAINTY,
        note: Some(POSSIBLE_TEXT_NOTE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::split_text;

    const W: u32 = 400;
    const H: u32 = 600;

    /// A page with black squares (20 x 20 px) at the given top-left corners.
    fn page(marks: &[(u32, u32)]) -> RgbImage {
        let mut img = RgbImage::from_pixel(W, H, image::Rgb([255, 255, 255]));
        for &(x0, y0) in marks {
            for y in y0..y0 + 20 {
                for x in x0..x0 + 20 {
                    img.put_pixel(x, y, image::Rgb([0, 0, 0]));
                }
            }
        }
        img
    }

    fn ink(img: &RgbImage, x: u32, y: u32) -> bool {
        img.get_pixel(x + 10, y + 10)[0] == 0
    }

    // Marks: a text line inside the text box, a stray line outside every region, a mark inside
    // the drawing, and one in the overlap of the text box and the drawing.
    const IN_TEXT: (u32, u32) = (60, 60);
    const STRAY: (u32, u32) = (60, 400);
    const IN_DRAWING: (u32, u32) = (300, 250);
    const OVERLAP: (u32, u32) = (200, 150);
    const TEXT_BOX: [u32; 4] = [50, 50, 230, 180];
    const DRAWN_BOX: [u32; 4] = [190, 140, 380, 300];

    /// `read` with caller regions: only the given text regions are read, drawings blanked even
    /// where they overlap a text box.
    #[test]
    fn regions_scope_shows_only_the_text_regions() {
        let p = page(&[IN_TEXT, STRAY, IN_DRAWING, OVERLAP]);
        let pass = text_pass(&p, TextScope::Regions, &[TEXT_BOX], &[DRAWN_BOX]).unwrap();
        assert!(ink(&pass.image, IN_TEXT.0, IN_TEXT.1));
        assert!(
            !ink(&pass.image, STRAY.0, STRAY.1),
            "stray text outside the regions"
        );
        assert!(!ink(&pass.image, IN_DRAWING.0, IN_DRAWING.1));
        assert!(
            !ink(&pass.image, OVERLAP.0, OVERLAP.1),
            "drawing ink in a text box"
        );
        assert_eq!(pass.new_region_px, None);
        // No text region: no text pass.
        assert_eq!(text_pass(&p, TextScope::Regions, &[], &[DRAWN_BOX]), None);
    }

    /// `read_page`: the whole page with only the drawings blanked, so stray text is kept.
    #[test]
    fn page_scope_blanks_only_the_drawings() {
        let p = page(&[IN_TEXT, STRAY, IN_DRAWING, OVERLAP]);
        let pass = text_pass(&p, TextScope::Page, &[TEXT_BOX], &[DRAWN_BOX]).unwrap();
        assert!(ink(&pass.image, IN_TEXT.0, IN_TEXT.1));
        assert!(
            ink(&pass.image, STRAY.0, STRAY.1),
            "stray text must be read"
        );
        assert!(!ink(&pass.image, IN_DRAWING.0, IN_DRAWING.1));
        assert!(!ink(&pass.image, OVERLAP.0, OVERLAP.1));
        // With a text region, no extra region.
        assert_eq!(pass.new_region_px, None);
    }

    /// `read_page` on a page without a text region: content outside the drawings becomes a new
    /// region; a page whose ink is all in drawings gets no text pass.
    #[test]
    fn page_scope_without_text_region_adds_possible_missed_text() {
        let p = page(&[STRAY, IN_DRAWING]);
        let pass = text_pass(&p, TextScope::Page, &[], &[DRAWN_BOX]).unwrap();
        assert_eq!(pass.new_region_px, Some([60, 400, 80, 420]));
        assert!(ink(&pass.image, STRAY.0, STRAY.1));
        assert!(!ink(&pass.image, IN_DRAWING.0, IN_DRAWING.1));
        let only_drawing = page(&[IN_DRAWING]);
        assert_eq!(
            text_pass(&only_drawing, TextScope::Page, &[], &[DRAWN_BOX]),
            None
        );
        // The region: text, the note, certainty 0.5, the box on the grid.
        let r = possible_text_region([60, 400, 80, 420], W, H);
        assert_eq!(r.kind, RegionKind::Text);
        assert_eq!(r.note.as_deref(), Some(POSSIBLE_TEXT_NOTE));
        assert_eq!(r.certainty, POSSIBLE_TEXT_CERTAINTY);
        assert_eq!(
            (r.bbox.x0, r.bbox.y0, r.bbox.x1, r.bbox.y1),
            (150, 667, 200, 700)
        );
    }

    /// The text of a page read goes to the text regions with no line dropped, also the lines of
    /// text that lies outside every region (they go to a region next to them in reading order).
    #[test]
    fn page_text_with_stray_lines_keeps_every_line() {
        // A page read: two lines from the text region, a stray line below it.
        let answer = "# Plan\nfirst line\n\nstray line far below";
        for heights in [&[130u32][..], &[130, 20], &[10, 10, 10, 10]] {
            let parts = split_text(answer, heights);
            let lines: Vec<&str> = parts
                .iter()
                .flat_map(|p| p.lines())
                .filter(|l| !l.trim().is_empty())
                .collect();
            assert_eq!(
                lines,
                ["# Plan", "first line", "stray line far below"],
                "{heights:?}"
            );
        }
    }
}
