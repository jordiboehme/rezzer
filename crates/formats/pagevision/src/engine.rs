// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page handle (feature `engine`): the detectors for the regions, Qwen for their content.

use image::RgbImage;

use crate::assemble::{RegionAnswer, assemble};
use crate::detect::Detectors;
use crate::error::PageError;
use crate::llama::{Generation, RunOptions, VisionModel};
use crate::model::{ModelFiles, PREPROCESS_VERSION, QWEN35_4B, prompt_sha256};
use crate::parse::{DrawingClass, parse_class};
use crate::prepare::{decode_on_white, grid_to_px, prepare_crop};
use crate::prompts;
use crate::regions::regions_fingerprint;
use crate::types::{Fingerprint, PageResult, Region, RegionKind, Regions, Stats};

/// The output budget of the classifier: one word.
const CLASSIFY_MAX_TOKENS: u32 = 4;

/// The two detectors and the vision model, loaded once and used for many pages.
#[derive(Debug)]
pub struct PageVision {
    detectors: Detectors,
    model: VisionModel,
}

fn add_stats(sum: &mut Stats, s: &Stats) {
    sum.prompt_tokens += s.prompt_tokens;
    sum.image_tokens += s.image_tokens;
    sum.output_tokens += s.output_tokens;
    sum.encode_ms += s.encode_ms;
    sum.decode_ms += s.decode_ms;
}

impl PageVision {
    /// Loads the four model files. A missing file is `PageError::ModelMissing` with its path.
    pub fn load(files: &ModelFiles, opts: RunOptions) -> Result<Self, PageError> {
        for path in [&files.model, &files.mmproj, &files.layout, &files.text_det] {
            if !path.is_file() {
                return Err(PageError::ModelMissing { path: path.clone() });
            }
        }
        Ok(Self {
            detectors: Detectors::load(&files.layout, &files.text_det)?,
            model: VisionModel::load_paths(&QWEN35_4B, &files.model, &files.mmproj, opts)?,
        })
    }

    /// The regions of a PNG page, in reading order.
    pub fn regions(&self, png: &[u8]) -> Result<Regions, PageError> {
        self.detectors.regions(png)
    }

    /// Reads the given regions of a PNG page, in their order, from crops of the colour page:
    /// - a text region: one `TEXT` call;
    /// - a drawn region: a `CLASSIFY` call of at most 4 tokens, then `DIAGRAM` or `SKETCH`.
    ///
    /// A region whose call fails gives no block and marks the page truncated. The classifier's
    /// own token limit never marks the page.
    pub fn read(&mut self, png: &[u8], regions: &[Region]) -> Result<PageResult, PageError> {
        let (raw, width, height) = decode_on_white(png)?;
        let page = RgbImage::from_raw(width, height, raw)
            .expect("decode_on_white returns width * height * 3 bytes");
        let mut stats = Stats::default();
        let mut answers = Vec::with_capacity(regions.len());
        for region in regions {
            answers.push(self.read_region(&page, region, &mut stats));
        }
        let mut result = assemble(width, height, answers, self.fingerprint(), stats)?;
        if !self.model.options().keep_raw {
            result.raw = None;
        }
        Ok(result)
    }

    fn read_region(&mut self, page: &RgbImage, region: &Region, stats: &mut Stats) -> RegionAnswer {
        let (w, h) = page.dimensions();
        let crop = prepare_crop(page, grid_to_px(region.bbox, w, h), &QWEN35_4B.image);
        let max_tokens = self.model.options().max_tokens;
        let mut answer = RegionAnswer {
            region: region.clone(),
            class: None,
            class_answer: None,
            text: String::new(),
            truncated: false,
            failed: None,
        };
        let mut call = |model: &mut VisionModel, prompt: &str, max: u32| {
            let g: Result<Generation, PageError> = model.generate_with(&crop, prompt, max);
            if let Ok(g) = &g {
                add_stats(stats, &g.stats);
            }
            g
        };
        let prompt = if region.kind == RegionKind::Drawing {
            match call(&mut self.model, prompts::CLASSIFY, CLASSIFY_MAX_TOKENS) {
                Ok(g) => {
                    let (class, _) = parse_class(&g.text);
                    answer.class = Some(class);
                    answer.class_answer = Some(g.text);
                    match class {
                        DrawingClass::Diagram => prompts::DIAGRAM,
                        DrawingClass::Sketch => prompts::SKETCH,
                    }
                }
                Err(e) => {
                    answer.failed = Some(format!("classifier: {e}"));
                    return answer;
                }
            }
        } else {
            prompts::TEXT
        };
        match call(&mut self.model, prompt, max_tokens) {
            Ok(g) => {
                answer.text = g.text;
                answer.truncated = g.truncated;
            }
            Err(e) => answer.failed = Some(e.to_string()),
        }
        answer
    }

    /// The regions of a PNG page, then [`read`](Self::read) over them.
    pub fn read_page(&mut self, png: &[u8]) -> Result<PageResult, PageError> {
        let regions = self.regions(png)?;
        self.read(png, &regions.regions)
    }

    /// The versions and hashes a result depends on: the Qwen files, the four prompts (joined by
    /// "\n\0\n"), the preprocessing, the detectors, the rules and the ink.
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            model_sha256: vec![
                QWEN35_4B.model.sha256.to_string(),
                QWEN35_4B.mmproj.sha256.to_string(),
            ],
            prompt_sha256: prompt_sha256(&prompts::joined()),
            preprocess: PREPROCESS_VERSION,
            ..regions_fingerprint()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn load_names_the_first_missing_file() {
        let files = ModelFiles::in_dir(Path::new("/nonexistent"));
        match PageVision::load(&files, RunOptions::default()) {
            Err(PageError::ModelMissing { path }) => assert_eq!(path, files.model),
            other => panic!("{other:?}"),
        }
    }
}
