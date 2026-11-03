// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the area of a page says against a table there, before its cells are read: a grid over
//! most of the page is its frame, a grid on a picture is a chart's, text much smaller than the
//! body text is a picture of a page, and a grid of many columns is a form or a chart.

use crate::segment::drawn::PAGE_SHARE;

/// Why a grid or region is the page's frame.
pub const PAGE_FRAME: &str = "it covers most of the page";

/// Why a grid or band on a picture is not a table. It is a chart, so its refusal is no loss.
pub const ON_PICTURE: &str = "it lies on a picture";

/// Why a grid or region of tiny text is not a table.
pub const TINY_TEXT: &str =
    "its text is much smaller than the body text, as in a picture of a page";

/// Why a grid of drawn rules of many columns is not a table.
pub const MANY_COLUMNS: &str = "it has more than 15 columns";

/// A grid of drawn rules with more columns than this is a form or a chart, not a table.
pub const MAX_COLUMNS: usize = 15;

/// Text smaller than this share of the body text is a picture of a page, not a table.
pub const SMALL_TEXT: f32 = 0.6;

/// The page a detector looks at: its size and what rules out a table.
#[derive(Debug, Clone, Copy)]
pub struct Area<'a> {
    pub width: f32,
    pub height: f32,
    /// The pictures of the page (x0, y0, x1, y1).
    pub images: &'a [[f32; 4]],
    /// The most common font size of the file's text.
    pub body_size: f32,
}

impl Area<'_> {
    /// Whether a picture lies under most of `b` (x0, y0, x1, y1): a grid or band there is a
    /// chart's.
    pub fn on_picture(&self, b: [f32; 4]) -> bool {
        let size = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
        self.images.iter().any(|i| overlap(*i, b) > 0.5 * size)
    }

    /// Why a table cannot stand in `b` (x0, y0, x1, y1) with text of `sizes`, if it cannot.
    pub fn refuses(&self, b: [f32; 4], sizes: &[f32]) -> Option<&'static str> {
        let size = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
        if size > PAGE_SHARE * self.width * self.height {
            return Some(PAGE_FRAME);
        }
        if self.on_picture(b) {
            return Some(ON_PICTURE);
        }
        let mut sizes = sizes.to_vec();
        sizes.sort_by(f32::total_cmp);
        if let Some(m) = sizes.get(sizes.len() / 2)
            && *m < SMALL_TEXT * self.body_size
        {
            return Some(TINY_TEXT);
        }
        None
    }
}

/// Whether a grid of `rows` by `cols` squares, `texted` of which hold text, is dense: three rows
/// or more with text in at least half of its squares. A grid over most of the page that is
/// dense is a table as large as the page, not the page's frame.
pub fn dense(rows: usize, cols: usize, texted: usize) -> bool {
    rows >= 3 && 2 * texted >= rows * cols
}

/// The area two boxes share.
pub fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let w = a[2].min(b[2]) - a[0].max(b[0]);
    let h = a[3].min(b[3]) - a[1].max(b[1]);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_area_refuses_a_frame_a_picture_and_tiny_text() {
        let images = [[100.0, 100.0, 300.0, 300.0]];
        let area = Area {
            width: 600.0,
            height: 800.0,
            images: &images,
            body_size: 10.0,
        };
        assert_eq!(
            area.refuses([0.0, 0.0, 600.0, 800.0], &[10.0]),
            Some(PAGE_FRAME)
        );
        assert_eq!(
            area.refuses([110.0, 110.0, 290.0, 290.0], &[10.0]),
            Some(ON_PICTURE)
        );
        assert_eq!(
            area.refuses([400.0, 400.0, 500.0, 500.0], &[5.0]),
            Some(TINY_TEXT)
        );
        assert_eq!(area.refuses([400.0, 400.0, 500.0, 500.0], &[10.0]), None);
        assert!(dense(3, 4, 6));
        assert!(!dense(2, 4, 8));
    }
}
