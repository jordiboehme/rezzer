// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PP-DocLayoutV3 on Candle (CPU). The model files in this folder are adapted from
//! oar-ocr-vl 0.10.3 `src/models/pp_doclayout/` (Apache-2.0); each names its changes.
//! PP-DocLayoutV3 is an RT-DETR detector (HGNetV2-L backbone, hybrid encoder,
//! deformable-attention decoder) with a reading-order head inside the decoder.

mod config;
mod decoder;
mod encoder;
mod err;
mod hgnetv2;
mod layers;
mod mask_head;
mod model;
mod order;
mod processing;

use std::path::Path;

use image::RgbImage;

use crate::PageError;
use crate::layoutpost::{DRAWN_CLASSES, LayoutBox, v3_filter};
use crate::rules::Rect;

/// The PP-DocLayoutV3 layout detector (CPU, F32).
#[derive(Debug)]
pub struct LayoutDetector {
    model: model::PpDocLayout,
}

impl LayoutDetector {
    /// Loads the safetensors file (`model::LAYOUT_V3`) with the embedded `config.json`.
    pub fn load(path: &Path) -> Result<Self, PageError> {
        if !path.exists() {
            return Err(PageError::ModelMissing {
                path: path.to_path_buf(),
            });
        }
        let config = config::PpDocLayoutConfig::from_json(super::V3_CONFIG_JSON)?;
        Ok(Self {
            model: model::PpDocLayout::load(path, config)?,
        })
    }

    /// Every query of the model with its best class, score and box in page pixels, in
    /// reading order (no threshold).
    pub fn raw(&self, rgb: &RgbImage) -> Result<Vec<LayoutBox>, PageError> {
        self.model.raw(rgb)
    }

    /// The drawn regions of a page: the PaddleX filter, then the drawn classes.
    pub fn drawn(&self, rgb: &RgbImage) -> Result<Vec<Rect>, PageError> {
        let raw = self.raw(rgb)?;
        Ok(v3_filter(&raw, rgb.width(), rgb.height())
            .into_iter()
            .filter(|b| DRAWN_CLASSES.contains(&b.class))
            .map(|b| b.bbox)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_file_is_named() {
        let path = Path::new("/nonexistent/pp-doclayoutv3.safetensors");
        match LayoutDetector::load(path) {
            Err(PageError::ModelMissing { path: p }) => assert_eq!(p, path),
            other => panic!("{other:?}"),
        }
    }
}
