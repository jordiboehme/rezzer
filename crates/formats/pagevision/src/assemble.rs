// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A page result from the answers of its regions: blocks, certainty, truncation and the page
//! Markdown. Default code, so it is tested without the models.

use crate::error::PageError;
use crate::parse::{
    DrawingClass, EMPTY_ANSWER, NO_DESCRIPTION, block_markdown, parse_diagram, parse_sketch,
    parse_text,
};
use crate::types::{Block, Content, Fingerprint, PageResult, Region, RegionKind, Stats};

/// The description of a diagram region that the page read gave no Mermaid block for.
pub const NO_DIAGRAM: &str = "A diagram that the page reading did not return.";

/// The certainty factor of a block whose content did not parse into its kind.
const UNPARSED_FACTOR: f32 = 0.5;
/// The certainty factor of a block whose answer hit the token limit.
const TRUNCATED_FACTOR: f32 = 0.3;
/// Model output is never certain.
const MAX_CERTAINTY: f32 = 0.95;

/// What the model answered for one region.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionAnswer {
    pub region: Region,
    /// The class of a drawn region; None for a text region.
    pub class: Option<DrawingClass>,
    /// The classifier's answer for a drawn region, as it came (debug output only).
    pub class_answer: Option<String>,
    /// The content answer.
    pub text: String,
    /// The content answer hit the token limit.
    pub truncated: bool,
    /// pagevision removed the diagram's description on purpose (a split block, or a description
    /// that only repeated the page text); the missing description then costs no certainty.
    pub description_removed: bool,
    /// Why the region could not be read; it then gives no block.
    pub failed: Option<String>,
}

/// The page result from the answers in region order:
/// - one block per region, with the region's box;
/// - the certainty is the region's, times 0.5 when the content did not parse, times 0.3 when
///   the answer was truncated, at most 0.95;
/// - a failed region gives no block and marks the page truncated; an empty answer gives no
///   block;
/// - a page without any block is `PageError::Unreadable`;
/// - the page Markdown is the block Markdown joined by blank lines;
/// - `raw` holds every answer with its region, class and reasons.
pub fn assemble(
    width_px: u32,
    height_px: u32,
    answers: Vec<RegionAnswer>,
    fingerprint: Fingerprint,
    stats: Stats,
) -> Result<PageResult, PageError> {
    let mut blocks = Vec::new();
    let mut raw = String::new();
    let (mut truncated, mut empty, mut failed) = (false, 0usize, 0usize);
    for (i, a) in answers.iter().enumerate() {
        let b = a.region.bbox;
        raw.push_str(&format!(
            "--- region {i}: {:?} [{}, {}, {}, {}]\n",
            a.region.kind, b.x0, b.y0, b.x1, b.y1
        ));
        if let Some(class) = a.class {
            raw.push_str(&format!(
                "class: {class:?}; classifier answered {:?}\n",
                a.class_answer.as_deref().unwrap_or("")
            ));
        }
        if let Some(e) = &a.failed {
            raw.push_str(&format!("failed: {e}\n"));
            truncated = true;
            failed += 1;
            continue;
        }
        if a.truncated {
            raw.push_str("truncated\n");
            truncated = true;
        }
        let (content, reason) = match (a.region.kind, a.class) {
            // A diagram region the page read gave no block for: say so.
            (RegionKind::Drawing, Some(DrawingClass::Diagram)) if a.text.trim().is_empty() => (
                Content::Picture {
                    description: NO_DIAGRAM.to_string(),
                },
                Some("no mermaid block for this region".to_string()),
            ),
            (RegionKind::Drawing, Some(DrawingClass::Diagram)) => parse_diagram(&a.text),
            (RegionKind::Drawing, _) => (parse_sketch(&a.text), None),
            _ => (parse_text(&a.text), None),
        };
        if let Content::Text { reason: r, .. } = &content
            && r == EMPTY_ANSWER
        {
            raw.push_str(&format!("{EMPTY_ANSWER}: no block\n"));
            empty += 1;
            continue;
        }
        // A description pagevision removed on purpose is not a parse failure.
        let excused = a.description_removed && reason.as_deref() == Some(NO_DESCRIPTION);
        let parsed = (reason.is_none() || excused) && !matches!(content, Content::Text { .. });
        if let Some(r) = &reason {
            raw.push_str(&format!("reason: {r}\n"));
        }
        raw.push_str(&a.text);
        raw.push('\n');
        let mut certainty = a.region.certainty;
        if !parsed {
            certainty *= UNPARSED_FACTOR;
        }
        if a.truncated {
            certainty *= TRUNCATED_FACTOR;
        }
        blocks.push(Block {
            bbox: a.region.bbox,
            certainty: certainty.min(MAX_CERTAINTY),
            markdown: block_markdown(&content, a.class),
            content,
        });
    }
    if blocks.is_empty() {
        let reason = if answers.is_empty() {
            "no region on the page".to_string()
        } else {
            format!("no region gave a block: {empty} empty, {failed} failed")
        };
        return Err(PageError::Unreadable { reason });
    }
    let markdown = blocks
        .iter()
        .map(|b| b.markdown.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(PageResult {
        width_px,
        height_px,
        blocks,
        markdown,
        truncated,
        stats,
        fingerprint,
        raw: Some(raw),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BBox;

    fn region(y0: u16, kind: RegionKind, certainty: f32) -> Region {
        Region {
            bbox: BBox {
                x0: 100,
                y0,
                x1: 900,
                y1: y0 + 50,
            },
            kind,
            certainty,
            note: None,
        }
    }

    fn text(y0: u16, answer: &str) -> RegionAnswer {
        RegionAnswer {
            region: region(y0, RegionKind::Text, 1.0),
            class: None,
            class_answer: None,
            text: answer.to_string(),
            truncated: false,
            description_removed: false,
            failed: None,
        }
    }

    fn drawing(y0: u16, class: DrawingClass, answer: &str) -> RegionAnswer {
        RegionAnswer {
            region: region(y0, RegionKind::Drawing, 1.0),
            class: Some(class),
            class_answer: Some(format!("{class:?}")),
            text: answer.to_string(),
            truncated: false,
            description_removed: false,
            failed: None,
        }
    }

    fn run(answers: Vec<RegionAnswer>) -> Result<PageResult, PageError> {
        assemble(
            1404,
            1872,
            answers,
            Fingerprint::default(),
            Stats::default(),
        )
    }

    const DIAGRAM: &str = "```mermaid\ngraph TD\n  A --> B\n```\n\n> A to B.";

    /// Review Focus 2: a page with ink but nothing readable is Unreadable, never an empty
    /// success.
    #[test]
    fn page_without_regions_is_unreadable() {
        match run(Vec::new()) {
            Err(PageError::Unreadable { reason }) => assert!(reason.contains("no region")),
            other => panic!("{other:?}"),
        }
        // Regions whose answers are all empty or failed.
        let mut failed = text(100, "");
        failed.failed = Some("engine error".into());
        match run(vec![text(10, "  "), failed]) {
            Err(PageError::Unreadable { reason }) => {
                assert!(reason.contains("1 empty"), "{reason}");
                assert!(reason.contains("1 failed"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn failed_region_marks_truncated_and_keeps_others() {
        let mut failed = drawing(300, DrawingClass::Diagram, "");
        failed.failed = Some("engine error: decoding".into());
        let page = run(vec![text(100, "Hello."), failed, text(500, "Bye.")]).unwrap();
        assert!(page.truncated);
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(page.markdown, "Hello.\n\nBye.");
        // An empty answer drops its block, but does not mark the page.
        let page = run(vec![text(100, "Hello."), text(200, "")]).unwrap();
        assert!(!page.truncated);
        assert_eq!(page.blocks.len(), 1);
        // A truncated answer keeps its block and marks the page.
        let mut cut = text(100, "Hello");
        cut.truncated = true;
        let page = run(vec![cut]).unwrap();
        assert!(page.truncated);
        assert_eq!(page.blocks.len(), 1);
    }

    #[test]
    fn certainty_is_capped_below_one() {
        let page = run(vec![text(100, "Hello.")]).unwrap();
        assert_eq!(page.blocks[0].certainty, 0.95);
        // 0.8 for the region.
        let mut a = text(100, "Hello.");
        a.region.certainty = 0.8;
        assert_eq!(run(vec![a.clone()]).unwrap().blocks[0].certainty, 0.8);
        // Truncated: 0.8 * 0.3.
        a.truncated = true;
        let c = run(vec![a]).unwrap().blocks[0].certainty;
        assert!((c - 0.24).abs() < 1e-6, "{c}");
        // A diagram that did not parse: 1.0 * 0.5.
        let bad = drawing(
            100,
            DrawingClass::Diagram,
            "```mermaid\ngraph TD\n  A[x --> B\n```",
        );
        let page = run(vec![bad]).unwrap();
        assert_eq!(page.blocks[0].certainty, 0.5);
        assert!(matches!(page.blocks[0].content, Content::Picture { .. }));
        assert!(page.blocks[0].markdown.starts_with("> Diagram: "));
        // Both: 0.5 * 0.3.
        let mut both = drawing(100, DrawingClass::Diagram, "no mermaid here");
        both.truncated = true;
        let c = run(vec![both]).unwrap().blocks[0].certainty;
        assert!((c - 0.15).abs() < 1e-6, "{c}");
    }

    #[test]
    fn blocks_follow_region_order() {
        let page = run(vec![
            text(100, "# Title"),
            drawing(300, DrawingClass::Diagram, DIAGRAM),
            drawing(600, DrawingClass::Sketch, "A teapot."),
            text(800, "- a\n- b"),
        ])
        .unwrap();
        let ys: Vec<u16> = page.blocks.iter().map(|b| b.bbox.y0).collect();
        assert_eq!(ys, [100, 300, 600, 800]);
        assert!(matches!(page.blocks[0].content, Content::Heading { .. }));
        assert!(matches!(page.blocks[1].content, Content::Diagram { .. }));
        assert!(matches!(page.blocks[2].content, Content::Picture { .. }));
        assert!(matches!(page.blocks[3].content, Content::List { .. }));
        assert_eq!(
            page.markdown,
            "# Title\n\n```mermaid\ngraph TD\n  A --> B\n```\n\n> A to B.\n\n> Sketch: A teapot.\n\n- a\n- b"
        );
        assert_eq!((page.width_px, page.height_px), (1404, 1872));
        assert_eq!(
            page.blocks[1].bbox,
            region(300, RegionKind::Drawing, 1.0).bbox
        );
        // The raw answers name each region and the classifier's answer.
        let raw = page.raw.unwrap();
        assert!(raw.contains("Diagram"), "{raw}");
        assert!(raw.contains("A teapot."), "{raw}");
    }

    /// Review Focus 3 at the page level: a sketch whose answer holds a Mermaid block gives a
    /// picture without it.
    #[test]
    fn sketch_with_mermaid_answer_has_no_mermaid() {
        let page = run(vec![drawing(100, DrawingClass::Sketch, DIAGRAM)]).unwrap();
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.markdown, "> Sketch: A to B.");
        assert!(!page.markdown.contains("mermaid"));
    }

    #[test]
    fn empty_answer_reason_is_in_raw() {
        let page = run(vec![text(100, "Hello."), text(200, "")]).unwrap();
        assert!(page.raw.unwrap().contains(EMPTY_ANSWER));
    }

    /// A diagram region without a Mermaid block from the page read is a picture that says so,
    /// never silently empty.
    #[test]
    fn diagram_without_block_becomes_picture() {
        let page = run(vec![
            text(100, "Hello."),
            drawing(300, DrawingClass::Diagram, ""),
        ])
        .unwrap();
        assert_eq!(page.blocks.len(), 2);
        let b = &page.blocks[1];
        assert_eq!(
            b.content,
            Content::Picture {
                description: NO_DIAGRAM.to_string()
            }
        );
        assert_eq!(b.markdown, format!("> Diagram: {NO_DIAGRAM}"));
        assert_eq!(b.certainty, 0.5);
        assert!(!page.truncated);
        // A sketch with an empty answer still gives no block.
        let page = run(vec![
            text(100, "Hello."),
            drawing(300, DrawingClass::Sketch, ""),
        ])
        .unwrap();
        assert_eq!(page.blocks.len(), 1);
    }

    /// The certainty is halved for a diagram the model gave no description, not for one whose
    /// description pagevision removed.
    #[test]
    fn removed_description_keeps_the_certainty() {
        let bare = "```mermaid\ngraph TD\n  A --> B\n```";
        let mut removed = drawing(100, DrawingClass::Diagram, bare);
        removed.description_removed = true;
        let page = run(vec![removed]).unwrap();
        assert!(matches!(page.blocks[0].content, Content::Diagram { .. }));
        assert_eq!(page.blocks[0].certainty, 0.95);
        let page = run(vec![drawing(100, DrawingClass::Diagram, bare)]).unwrap();
        assert_eq!(page.blocks[0].certainty, 0.5);
        // A removed description does not excuse a diagram that failed the check.
        let mut broken = drawing(
            100,
            DrawingClass::Diagram,
            "```mermaid\ngraph TD\n  A[x --> B\n```",
        );
        broken.description_removed = true;
        assert_eq!(run(vec![broken]).unwrap().blocks[0].certainty, 0.5);
    }
}
