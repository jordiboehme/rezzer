// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The prompts of a region read. Every change changes the prompt hash of the fingerprint, so a
//! changed prompt needs a new evaluation run.

/// Asks whether a drawn region is a diagram or a sketch, in one word.
pub const CLASSIFY: &str = "Look at this hand-drawn picture. Answer with one word. Answer \"diagram\" if it is made of boxes, shapes or words connected by arrows or lines, like a flowchart, a graph or a sequence of steps. Answer \"sketch\" for anything else, like a drawing of an object, a machine part, a person or a scene.";

/// Reads a text region as Markdown.
pub const TEXT: &str = "Transcribe the handwritten text in this image into Markdown. Copy the handwritten words exactly; do not invent or correct text. Keep headings, lists and line breaks. Output only the Markdown.";

/// Reads a diagram as Mermaid plus a one-sentence description. The wording of the arrow sentence
/// is the one that won the content test; a stricter "never translate" made the model invent
/// arrow labels.
pub const DIAGRAM: &str = "This image shows one hand-drawn diagram. Output it as a ```mermaid code block with every node label and arrow exactly as drawn (arrow direction matters; use <--> for arrows with heads on both ends), followed by a one-sentence description as a > quote. Output only the Markdown.";

/// Describes a sketch. It never asks for Mermaid.
pub const SKETCH: &str = "This image shows one hand-drawn sketch. Describe in one or two sentences what it shows. Copy any handwritten words in it exactly. Output only the description.";

/// The four prompts in the order the fingerprint hashes them.
pub const ALL: [&str; 4] = [CLASSIFY, TEXT, DIAGRAM, SKETCH];

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
        assert!(DIAGRAM.contains("```mermaid"));
        assert!(!DIAGRAM.contains("never translate"));
    }

    #[test]
    fn joined_prompts_are_separated() {
        let j = joined();
        assert!(j.starts_with(CLASSIFY));
        assert!(j.ends_with(SKETCH));
        assert_eq!(j.matches("\n\0\n").count(), 3);
    }
}
