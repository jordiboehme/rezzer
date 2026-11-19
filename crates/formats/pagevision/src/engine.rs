// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page handle (feature `engine`): the detectors for the regions, Qwen for their content.
//! Qwen reads the text and the diagrams from the whole page and describes sketches from their
//! crops.

use image::RgbImage;

use crate::assemble::{RegionAnswer, assemble};
use crate::detect::Detectors;
use crate::error::PageError;
use crate::llama::{Generation, RunOptions, VisionModel};
use crate::model::{ModelFiles, PREPROCESS_VERSION, QWEN35_4B, prompt_sha256};
use crate::parse::{DrawingClass, mermaid_sections, parse_class, split_text};
use crate::prepare::{
    Prepared, blank_boxes, decode_on_white, grid_to_px, prepare_crop, prepare_page,
};
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

    /// Reads the given regions of a PNG page:
    /// - each drawn region is classified from its crop (one call of at most 4 tokens); a sketch
    ///   is described from its crop (`SKETCH`);
    /// - if any region is a diagram, one pass over the whole page (`PAGE`) gives the Mermaid
    ///   blocks, which go to the diagram regions in reading order; its text and any extra block
    ///   are dropped, and a diagram region without a block becomes a picture that says so;
    /// - if any region is text, one pass over the whole page with every drawn region blanked
    ///   white (`PAGE_TEXT`) gives the text, split over the text regions in reading order by
    ///   their height ([`split_text`]).
    ///
    /// A failed call gives no block for its regions and marks the page truncated. The
    /// classifier's own token limit never marks the page.
    pub fn read(&mut self, png: &[u8], regions: &[Region]) -> Result<PageResult, PageError> {
        let (raw, width, height) = decode_on_white(png)?;
        let page = RgbImage::from_raw(width, height, raw)
            .expect("decode_on_white returns width * height * 3 bytes");
        let max_tokens = self.model.options().max_tokens;
        let mut stats = Stats::default();
        let mut log = String::new();
        let mut answers: Vec<RegionAnswer> = regions
            .iter()
            .map(|r| RegionAnswer {
                region: r.clone(),
                class: None,
                class_answer: None,
                text: String::new(),
                truncated: false,
                failed: None,
            })
            .collect();

        // Drawn regions: the class from the crop, and the description of a sketch.
        let mut drawn_px = Vec::new();
        for a in answers.iter_mut() {
            if a.region.kind != RegionKind::Drawing {
                continue;
            }
            let px = grid_to_px(a.region.bbox, width, height);
            drawn_px.push(px);
            let crop = prepare_crop(&page, px, &QWEN35_4B.image);
            match self.call(&crop, prompts::CLASSIFY, CLASSIFY_MAX_TOKENS, &mut stats) {
                Ok(g) => {
                    let (class, _) = parse_class(&g.text);
                    a.class = Some(class);
                    a.class_answer = Some(g.text);
                }
                Err(e) => {
                    a.failed = Some(format!("classifier: {e}"));
                    continue;
                }
            }
            if a.class == Some(DrawingClass::Sketch) {
                match self.call(&crop, prompts::SKETCH, max_tokens, &mut stats) {
                    Ok(g) => {
                        a.text = g.text;
                        a.truncated = g.truncated;
                    }
                    Err(e) => a.failed = Some(e.to_string()),
                }
            }
        }

        // Diagrams: the Mermaid blocks of one whole-page read, in reading order.
        let diagrams: Vec<usize> = (0..answers.len())
            .filter(|&i| answers[i].class == Some(DrawingClass::Diagram))
            .collect();
        if !diagrams.is_empty() {
            let image = prepare_page(&page, &QWEN35_4B.image);
            match self.call(&image, prompts::PAGE, max_tokens, &mut stats) {
                Ok(g) => {
                    let sections = mermaid_sections(&g.text);
                    log.push_str(&format!(
                        "--- page read: {} mermaid blocks for {} diagram regions\n{}\n",
                        sections.len(),
                        diagrams.len(),
                        g.text
                    ));
                    let mut sections = sections.into_iter();
                    for &i in &diagrams {
                        answers[i].text = sections.next().unwrap_or_default();
                        answers[i].truncated = g.truncated;
                    }
                }
                Err(e) => {
                    for &i in &diagrams {
                        answers[i].failed = Some(format!("page read: {e}"));
                    }
                }
            }
        }

        // Text: one read of the page with the drawings blanked, split over the text regions.
        let texts: Vec<usize> = (0..answers.len())
            .filter(|&i| answers[i].region.kind == RegionKind::Text)
            .collect();
        if !texts.is_empty() {
            let image = prepare_page(&blank_boxes(&page, &drawn_px), &QWEN35_4B.image);
            match self.call(&image, prompts::PAGE_TEXT, max_tokens, &mut stats) {
                Ok(g) => {
                    log.push_str(&format!("--- text read\n{}\n", g.text));
                    let heights: Vec<u32> = texts
                        .iter()
                        .map(|&i| {
                            let b = answers[i].region.bbox;
                            u32::from(b.y1.saturating_sub(b.y0))
                        })
                        .collect();
                    for (&i, part) in texts.iter().zip(split_text(&g.text, &heights)) {
                        answers[i].text = part;
                        answers[i].truncated = g.truncated;
                    }
                }
                Err(e) => {
                    for &i in &texts {
                        answers[i].failed = Some(format!("text read: {e}"));
                    }
                }
            }
        }

        let mut result = assemble(width, height, answers, self.fingerprint(), stats)?;
        if self.model.options().keep_raw {
            result.raw = Some(format!("{}{log}", result.raw.unwrap_or_default()));
        } else {
            result.raw = None;
        }
        Ok(result)
    }

    fn call(
        &mut self,
        image: &Prepared,
        prompt: &str,
        max_tokens: u32,
        stats: &mut Stats,
    ) -> Result<Generation, PageError> {
        let g = self.model.generate_with(image, prompt, max_tokens)?;
        add_stats(stats, &g.stats);
        Ok(g)
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
