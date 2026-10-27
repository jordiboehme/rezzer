// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Optional page geometry: where a paged source puts its blocks, rules and pictures.
//! Deliberately no serde derives: it never reaches the model JSON, the schema or the cache.

use crate::inline::Anchor;

/// The most boxes on one page.
pub const MAX_BOXES_PER_PAGE: usize = 4096;
/// The most boxes in one [`Geometry`].
pub const MAX_BOXES: usize = 200_000;

/// What a [`PlacedBox`] frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    /// A run of text.
    Text,
    /// A heading, with its depth.
    Heading(u8),
    /// A table.
    Table,
    /// One cell of a table.
    Cell,
    /// A ruled line.
    Rule,
    /// A drawn shape that is not a line or a picture.
    Drawn,
    /// A picture.
    Picture,
}

/// One framed thing on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedBox {
    pub kind: BoxKind,
    /// x0, y0, x1, y1 in points, y upward, in the frame of the page's text (as
    /// `Barcode.bounds`).
    pub bounds: [f32; 4],
    pub anchor: Option<Anchor>,
}

/// One page with its size and boxes.
#[derive(Debug, Clone, PartialEq)]
pub struct PageGeometry {
    /// 1-based page number.
    pub page: u32,
    /// Width and height in points.
    pub size: [f32; 2],
    pub boxes: Vec<PlacedBox>,
}

/// The pages of a paged source with their boxes.
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    pub pages: Vec<PageGeometry>,
    /// The boxes added through [`Geometry::push`], so the total cap needs no sum over pages.
    pushed: usize,
}

impl PartialEq for Geometry {
    fn eq(&self, other: &Geometry) -> bool {
        self.pages == other.pages
    }
}

impl Geometry {
    /// Adds an empty page. False for a size that is not finite and positive, or a page
    /// added before.
    pub fn add_page(&mut self, page: u32, size: [f32; 2]) -> bool {
        let valid = size.iter().all(|v| v.is_finite() && *v > 0.0);
        // Pages come in order: the last one answers the usual case, a scan the rest.
        let known = match self.pages.last() {
            Some(last) if last.page < page => false,
            _ => self.pages.iter().any(|p| p.page == page),
        };
        if !valid || known {
            return false;
        }
        self.pages.push(PageGeometry {
            page,
            size,
            boxes: Vec::new(),
        });
        true
    }

    /// Adds a box to a page. False when the page was not added, a bound is not finite,
    /// `x1 < x0` or `y1 < y0`, or a cap is reached.
    pub fn push(&mut self, page: u32, b: PlacedBox) -> bool {
        let [x0, y0, x1, y1] = b.bounds;
        if !b.bounds.iter().all(|v| v.is_finite()) || x1 < x0 || y1 < y0 {
            return false;
        }
        if self.pushed >= MAX_BOXES {
            return false;
        }
        let Some(p) = self.pages.iter_mut().rev().find(|p| p.page == page) else {
            return false;
        };
        if p.boxes.len() >= MAX_BOXES_PER_PAGE {
            return false;
        }
        p.boxes.push(b);
        self.pushed += 1;
        true
    }

    /// The first box carrying this anchor.
    pub fn box_of(&self, anchor: &Anchor) -> Option<&PlacedBox> {
        self.pages
            .iter()
            .flat_map(|p| p.boxes.iter())
            .find(|b| b.anchor.as_ref() == Some(anchor))
    }

    /// The number of boxes on all pages.
    pub fn len(&self) -> usize {
        self.pages.iter().map(|p| p.boxes.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
