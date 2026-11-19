// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The prompts of a page read. Every change changes the prompt hash of the fingerprint, so a
//! changed prompt needs a new evaluation run.

/// Asks whether a drawn region is a diagram or a sketch, in one word.
pub const CLASSIFY: &str = "Look at this hand-drawn picture. Answer with one word. Answer \"diagram\" if it is made of boxes, shapes or words connected by arrows or lines, like a flowchart, a graph or a sequence of steps. Answer \"sketch\" for anything else, like a drawing of an object, a machine part, a person or a scene.";

/// Reads the text of a whole page whose drawings are blanked white: the whole-page prompt of
/// the content test without its diagram sentence.
pub const PAGE_TEXT: &str = "Transcribe this handwritten note page into Markdown. Copy the handwritten words exactly; do not invent or correct text. Keep headings, lists and line breaks. Output only the Markdown.";

/// Reads a whole page with its diagrams: the whole-page prompt of the content test. Only its
/// Mermaid blocks are used. The wording of the arrow sentence won the content test; a stricter
/// "never translate" made the model invent arrow labels.
pub const PAGE: &str = "Transcribe this handwritten note page into Markdown. Copy the handwritten words exactly; do not invent or correct text. Keep headings, lists and line breaks. If the page contains a diagram, output each diagram as a ```mermaid code block with every node label and arrow exactly as drawn (arrow direction matters; use <--> for arrows with heads on both ends), followed by a one-sentence description as a > quote. Output only the Markdown.";

/// Describes a sketch from its crop. It never asks for Mermaid.
pub const SKETCH: &str = "This image shows one hand-drawn sketch. Describe in one or two sentences what it shows. Copy any handwritten words in it exactly. Output only the description.";

/// The four prompts in the order the fingerprint hashes them.
pub const ALL: [&str; 4] = [CLASSIFY, PAGE_TEXT, PAGE, SKETCH];

/// The text the prompt hash of the fingerprint covers: the four prompts joined by "\n\0\n".
pub fn joined() -> String {
    ALL.join("\n\0\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sketch_prompt_never_asks_for_mermaid() {
        assert!(!SKETCH.to_lowercase().contains("mermaid"));
        assert!(!CLASSIFY.to_lowercase().contains("mermaid"));
        assert!(!PAGE_TEXT.to_lowercase().contains("mermaid"));
        assert!(!PAGE_TEXT.contains("diagram"));
        assert!(PAGE.contains("```mermaid"));
        assert!(!PAGE.contains("never translate"));
        // The text prompt is the page prompt without its diagram sentence.
        let cut = PAGE.find(" If the page contains a diagram").unwrap();
        let end = PAGE.find(" Output only the Markdown.").unwrap();
        assert_eq!(format!("{}{}", &PAGE[..cut], &PAGE[end..]), PAGE_TEXT);
    }

    #[test]
    fn joined_prompts_are_separated() {
        let j = joined();
        assert!(j.starts_with(CLASSIFY));
        assert!(j.ends_with(SKETCH));
        assert_eq!(j.matches("\n\0\n").count(), 3);
    }
}
