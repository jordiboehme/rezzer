// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout and text line detectors on Candle (feature `detect`, CPU only).

use std::path::Path;

use crate::PageError;
use crate::ink::{Gray, flatten, gray_to_rgb};
use crate::regions::regions_from;
use crate::rules::Rect;
use crate::types::Regions;

/// The `config.json` of PP-DocLayoutV3 at the pinned revision of
/// [`crate::model::LAYOUT_V3`], embedded so the model folder needs no fifth file.
pub const V3_CONFIG_JSON: &[u8] = include_bytes!("v3/config.json");

pub mod v3;
pub mod v6;

pub use v3::LayoutDetector;
pub use v6::TextLineDetector;

/// Both detectors, loaded once and used for many pages.
#[derive(Debug)]
pub struct Detectors {
    layout: LayoutDetector,
    text: TextLineDetector,
}

impl Detectors {
    /// Loads the layout detector (`model::LAYOUT_V3`) and the text line detector
    /// (`model::TEXT_DET_V6`). A missing file is `PageError::ModelMissing`.
    pub fn load(layout: &Path, text_det: &Path) -> Result<Self, PageError> {
        Ok(Self {
            layout: LayoutDetector::load(layout)?,
            text: TextLineDetector::load(text_det)?,
        })
    }

    /// The drawn regions and the text lines of the grey detector page, in page pixels.
    pub fn boxes(&self, gray: &Gray) -> Result<(Vec<Rect>, Vec<Rect>), PageError> {
        let rgb = gray_to_rgb(gray);
        Ok((self.layout.drawn(&rgb)?, self.text.lines(&rgb)?))
    }

    /// The regions of a PNG page: the detectors on the grey page, then
    /// [`regions_from`](crate::regions::regions_from).
    pub fn regions(&self, png: &[u8]) -> Result<Regions, PageError> {
        let gray = flatten(png)?;
        let (drawn, lines) = self.boxes(&gray)?;
        Ok(regions_from(&gray, &drawn, &lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_detector_file_is_named() {
        let layout = Path::new("/nonexistent/pp-doclayoutv3.safetensors");
        let text = Path::new("/nonexistent/pp-ocrv6-small-det.onnx");
        match Detectors::load(layout, text) {
            Err(PageError::ModelMissing { path }) => assert_eq!(path, layout),
            other => panic!("{other:?}"),
        }
    }
}
