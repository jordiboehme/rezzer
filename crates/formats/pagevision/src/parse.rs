// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Model answers to block content: the class of a drawing, text, diagrams and sketches, and the
//! Markdown of a block.

use crate::types::{Content, Table};

/// What a drawn region shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum DrawingClass {
    /// Boxes, shapes or words connected by arrows or lines: read as Mermaid.
    Diagram,
    /// Anything else: read as a description only.
    Sketch,
}

/// The reason of a Content::Text for an answer with nothing in it.
pub const EMPTY_ANSWER: &str = "empty answer";

/// The class from the classifier's answer: its first word, lower case, punctuation trimmed.
/// `diagram` is a Diagram. Anything else is a Sketch; the reason names the answer unless the
/// word was `sketch`.
pub fn parse_class(answer: &str) -> (DrawingClass, Option<String>) {
    let word = answer
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    match word.as_str() {
        "diagram" => (DrawingClass::Diagram, None),
        "sketch" => (DrawingClass::Sketch, None),
        _ => {
            let head: String = answer.trim().chars().take(20).collect();
            (
                DrawingClass::Sketch,
                Some(format!("classifier answered \"{head}\"")),
            )
        }
    }
}

fn empty() -> Content {
    Content::Text {
        text: String::new(),
        reason: EMPTY_ANSWER.to_string(),
    }
}

/// The answer without an enclosing ```` ```markdown ```` or ```` ```md ```` fence (or a bare
/// ```` ``` ```` fence around everything), trimmed.
fn unwrap_fence(md: &str) -> &str {
    let t = md.trim();
    let Some(first_nl) = t.find('\n') else {
        return t;
    };
    let opener = t[..first_nl].trim();
    if !matches!(opener, "```markdown" | "```md" | "```") || !t.ends_with("```") {
        return t;
    }
    let inner = &t[first_nl + 1..t.len() - 3];
    // A bare fence only when no other fence is inside.
    if opener == "```" && inner.contains("```") {
        return t;
    }
    inner.trim()
}

/// A list item's text: `- x`, `* x`, `+ x` or `1. x`.
fn list_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &t[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(r.trim());
        }
    }
    None
}

/// A text region's answer as content:
/// - a single `#` heading line is a Heading;
/// - a text of only list items is a List;
/// - anything else is a Paragraph with the Markdown as it is;
/// - an empty answer is `Content::Text` with the reason [`EMPTY_ANSWER`].
pub fn parse_text(md: &str) -> Content {
    let t = unwrap_fence(md);
    if t.is_empty() {
        return empty();
    }
    let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
    if let [line] = lines[..] {
        let level = line.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) && line[level..].starts_with(' ') {
            return Content::Heading {
                level: level as u8,
                text: line[level..].trim().to_string(),
            };
        }
    }
    let items: Vec<&str> = lines.iter().filter_map(|l| list_item(l)).collect();
    if items.len() == lines.len() {
        return Content::List {
            items: items.into_iter().map(str::to_string).collect(),
        };
    }
    Content::Paragraph {
        text: t.to_string(),
    }
}

/// The fenced blocks of an answer as (info string, body), and the text outside them.
fn split_fences(md: &str) -> (Vec<(String, String)>, String) {
    let mut blocks = Vec::new();
    let mut outside = String::new();
    let mut current: Option<(String, String)> = None;
    for line in md.lines() {
        let t = line.trim();
        match &mut current {
            None => {
                if let Some(info) = t.strip_prefix("```") {
                    current = Some((info.trim().to_lowercase(), String::new()));
                } else {
                    outside.push_str(line);
                    outside.push('\n');
                }
            }
            Some((_, body)) => {
                if t.starts_with("```") {
                    blocks.push(current.take().expect("inside a block"));
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    // A block the model did not close runs to the end.
    if let Some(open) = current {
        blocks.push(open);
    }
    (blocks, outside)
}

/// Text outside fences as one description: quote marks and a leading `Sketch:` or `Diagram:`
/// removed, lines trimmed, empty lines dropped.
fn description(outside: &str) -> String {
    let lines: Vec<&str> = outside
        .lines()
        .map(|l| l.trim().trim_start_matches('>').trim())
        .filter(|l| !l.is_empty())
        .collect();
    let text = lines.join("\n");
    for prefix in ["Sketch:", "Diagram:"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    text
}

/// A diagram's answer as content, with the reason when it is not a checked diagram with a
/// description:
/// - one ```` ```mermaid ```` block that passes [`crate::mermaid::check`] is a Diagram, its
///   description the text outside the block (the `>` quote); without one the reason is
///   "no description" and the Mermaid is kept;
/// - otherwise a Picture with that description, and the reason beside it.
pub fn parse_diagram(md: &str) -> (Content, Option<String>) {
    let t = unwrap_fence(md);
    if t.is_empty() {
        return (empty(), Some(EMPTY_ANSWER.to_string()));
    }
    let (blocks, outside) = split_fences(t);
    let desc = description(&outside);
    let mermaid: Vec<&String> = blocks
        .iter()
        .filter(|(info, _)| info == "mermaid")
        .map(|(_, body)| body)
        .collect();
    let failed = |reason: String| {
        (
            Content::Picture {
                description: desc.clone(),
            },
            Some(reason),
        )
    };
    match mermaid[..] {
        [] => failed("no mermaid block".to_string()),
        [src] => {
            let src = src.trim_end().to_string();
            match crate::mermaid::check(&src) {
                Ok(()) => {
                    let reason = desc.is_empty().then(|| "no description".to_string());
                    (
                        Content::Diagram {
                            mermaid: src,
                            description: desc,
                        },
                        reason,
                    )
                }
                Err(e) => failed(e),
            }
        }
        _ => failed(format!("{} mermaid blocks", mermaid.len())),
    }
}

/// A sketch's answer as content: a Picture whose description is the answer without any fenced
/// block, so a sketch never carries Mermaid.
pub fn parse_sketch(md: &str) -> Content {
    let (_, outside) = split_fences(unwrap_fence(md));
    let desc = description(&outside);
    if desc.is_empty() {
        return empty();
    }
    Content::Picture { description: desc }
}

/// A quote of one or more lines, the first one starting with `head`.
fn quote(head: &str, text: &str) -> String {
    let mut lines = text.lines();
    let mut out = format!("> {head}{}", lines.next().unwrap_or(""));
    for l in lines {
        out.push_str("\n> ");
        out.push_str(l);
    }
    out
}

fn table_markdown(t: &Table) -> String {
    let row = |cells: &Vec<crate::types::Cell>| {
        let texts: Vec<String> = cells.iter().map(|c| c.text.replace('|', "\\|")).collect();
        format!("| {} |", texts.join(" | "))
    };
    let mut out = Vec::new();
    for (i, cells) in t.rows.iter().enumerate() {
        out.push(row(cells));
        if i + 1 == usize::from(t.header_rows.max(1)) {
            out.push(format!("|{}", "---|".repeat(cells.len().max(1))));
        }
    }
    out.join("\n")
}

/// The Markdown of a block. `class` is the class of a drawn region, None for a text region.
/// - Diagram: the mermaid fence, a blank line, the description as a `> ` quote;
/// - Picture: `> Sketch: ` and the description, or `> Diagram: ` for a diagram that did not
///   parse;
/// - the other kinds: their Markdown.
pub fn block_markdown(content: &Content, class: Option<DrawingClass>) -> String {
    match content {
        Content::Diagram {
            mermaid,
            description,
        } => {
            let fence = format!("```mermaid\n{}\n```", mermaid.trim_end());
            if description.is_empty() {
                fence
            } else {
                format!("{fence}\n\n{}", quote("", description))
            }
        }
        Content::Picture { description } => match class {
            Some(DrawingClass::Diagram) => quote("Diagram: ", description),
            _ => quote("Sketch: ", description),
        },
        Content::Heading { level, text } => {
            format!("{} {text}", "#".repeat(usize::from((*level).clamp(1, 6))))
        }
        Content::Paragraph { text } => text.clone(),
        Content::List { items } => items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
        Content::Table(t) => table_markdown(t),
        Content::Text { text, .. } => text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_junk_counts_as_sketch() {
        for junk in ["I think", "", "Flowchart", "  ", "diagrams"] {
            let (class, reason) = parse_class(junk);
            assert_eq!(class, DrawingClass::Sketch, "{junk:?}");
            let reason = reason.unwrap_or_else(|| panic!("no reason for {junk:?}"));
            assert!(reason.starts_with("classifier answered \""), "{reason}");
        }
        assert_eq!(
            parse_class("Flowchart").1.as_deref(),
            Some("classifier answered \"Flowchart\"")
        );
        // Only the first 20 characters of the answer.
        assert_eq!(
            parse_class("It looks like a flowchart of steps")
                .1
                .as_deref(),
            Some("classifier answered \"It looks like a flow\"")
        );
        assert_eq!(parse_class("Diagram."), (DrawingClass::Diagram, None));
        assert_eq!(parse_class("diagram"), (DrawingClass::Diagram, None));
        assert_eq!(parse_class(" \"Diagram\"\n"), (DrawingClass::Diagram, None));
        assert_eq!(parse_class("Sketch."), (DrawingClass::Sketch, None));
        assert_eq!(
            parse_class("sketch of a teapot"),
            (DrawingClass::Sketch, None)
        );
    }

    #[test]
    fn sketch_never_parses_mermaid() {
        let answer = "A teapot with a lid.\n\n```mermaid\ngraph TD\n  A --> B\n```\n";
        match parse_sketch(answer) {
            Content::Picture { description } => {
                assert_eq!(description, "A teapot with a lid.");
            }
            other => panic!("{other:?}"),
        }
        let c = parse_sketch("> Sketch: a garden chair");
        assert_eq!(
            c,
            Content::Picture {
                description: "a garden chair".into()
            }
        );
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Sketch)),
            "> Sketch: a garden chair"
        );
        assert!(!block_markdown(&c, Some(DrawingClass::Sketch)).contains("```"));
    }

    #[test]
    fn diagram_with_good_mermaid_is_a_diagram() {
        let answer =
            "```mermaid\ngraph LR\n    App --> DB\n    DB <--> MCP\n```\n\n> App points to DB.";
        let (c, reason) = parse_diagram(answer);
        assert_eq!(reason, None);
        assert_eq!(
            c,
            Content::Diagram {
                mermaid: "graph LR\n    App --> DB\n    DB <--> MCP".into(),
                description: "App points to DB.".into()
            }
        );
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Diagram)),
            "```mermaid\ngraph LR\n    App --> DB\n    DB <--> MCP\n```\n\n> App points to DB."
        );
        // A whole answer in a markdown fence is unwrapped first.
        let wrapped = format!("```markdown\n{answer}\n```");
        assert_eq!(parse_diagram(&wrapped), (c, None));
    }

    #[test]
    fn diagram_with_bad_mermaid_becomes_picture_with_reason() {
        let answer = "```mermaid\ngraph TD\n    A[Start --> B\n```\n\n> A start node.";
        let (c, reason) = parse_diagram(answer);
        assert_eq!(
            c,
            Content::Picture {
                description: "A start node.".into()
            }
        );
        let reason = reason.unwrap();
        assert!(reason.contains("unclosed"), "{reason}");
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Diagram)),
            "> Diagram: A start node."
        );
        // No mermaid block at all.
        let (c, reason) = parse_diagram("Two boxes and an arrow.");
        assert_eq!(
            c,
            Content::Picture {
                description: "Two boxes and an arrow.".into()
            }
        );
        assert_eq!(reason.as_deref(), Some("no mermaid block"));
        // Two blocks.
        let two = "```mermaid\ngraph TD\nA-->B\n```\n```mermaid\ngraph TD\nC-->D\n```\n> d";
        let (c, reason) = parse_diagram(two);
        assert!(matches!(c, Content::Picture { .. }));
        assert_eq!(reason.as_deref(), Some("2 mermaid blocks"));
    }

    #[test]
    fn diagram_without_description_keeps_the_mermaid() {
        let (c, reason) = parse_diagram("```mermaid\ngraph TD\n  A --> B\n```");
        assert_eq!(
            c,
            Content::Diagram {
                mermaid: "graph TD\n  A --> B".into(),
                description: String::new()
            }
        );
        assert_eq!(reason.as_deref(), Some("no description"));
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Diagram)),
            "```mermaid\ngraph TD\n  A --> B\n```"
        );
        // A description without the quote mark is taken as it is.
        let (c, reason) = parse_diagram("```mermaid\ngraph TD\n  A --> B\n```\nA to B.");
        assert_eq!(reason, None);
        assert!(matches!(c, Content::Diagram { description, .. } if description == "A to B."));
    }

    #[test]
    fn heading_list_paragraph() {
        assert_eq!(
            parse_text("## Plan\n"),
            Content::Heading {
                level: 2,
                text: "Plan".into()
            }
        );
        assert_eq!(
            parse_text("- one\n- two\n* three\n1. four"),
            Content::List {
                items: vec!["one".into(), "two".into(), "three".into(), "four".into()]
            }
        );
        let mixed = "# Title\n\nSome text.\n\n- a\n- b";
        assert_eq!(parse_text(mixed), Content::Paragraph { text: mixed.into() });
        assert_eq!(
            parse_text("```markdown\nJust words.\n```"),
            Content::Paragraph {
                text: "Just words.".into()
            }
        );
        assert_eq!(
            parse_text("  \n"),
            Content::Text {
                text: String::new(),
                reason: EMPTY_ANSWER.into()
            }
        );
        // The Markdown of each kind.
        assert_eq!(block_markdown(&parse_text("## Plan"), None), "## Plan");
        assert_eq!(block_markdown(&parse_text("* a\n* b"), None), "- a\n- b");
        assert_eq!(block_markdown(&parse_text(mixed), None), mixed);
    }

    #[test]
    fn sketch_markdown_prefix() {
        let c = Content::Picture {
            description: "Three teapots.".into(),
        };
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Sketch)),
            "> Sketch: Three teapots."
        );
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Diagram)),
            "> Diagram: Three teapots."
        );
        assert_eq!(block_markdown(&c, None), "> Sketch: Three teapots.");
        // A sketch description of several lines stays one quote.
        let c = Content::Picture {
            description: "A teapot.\nA cup.".into(),
        };
        assert_eq!(
            block_markdown(&c, Some(DrawingClass::Sketch)),
            "> Sketch: A teapot.\n> A cup."
        );
    }

    #[test]
    fn empty_answers_say_so() {
        for c in [parse_sketch(""), parse_diagram(" ").0, parse_text("")] {
            assert_eq!(
                c,
                Content::Text {
                    text: String::new(),
                    reason: EMPTY_ANSWER.into()
                }
            );
        }
        assert_eq!(parse_diagram("").1.as_deref(), Some(EMPTY_ANSWER));
    }
}
