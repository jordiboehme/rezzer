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

/// The reason of a Diagram whose answer has no description.
pub const NO_DESCRIPTION: &str = "no description";

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
    let mut text = lines.join("\n");
    for prefix in ["Sketch:", "Diagram:"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim().to_string();
        }
    }
    // A description the model put in quotes.
    match text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        Some(inner) if !inner.contains('"') => inner.trim().to_string(),
        _ => text,
    }
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
            // A `>` line is never Mermaid: a quote the model wrote inside the fence is the
            // description.
            let (quoted, kept): (Vec<&str>, Vec<&str>) =
                src.lines().partition(|l| l.trim_start().starts_with('>'));
            let desc = if desc.is_empty() {
                description(&quoted.join("\n"))
            } else {
                desc.clone()
            };
            let src = kept.join("\n").trim_end().to_string();
            match crate::mermaid::check(&src) {
                Ok(()) => {
                    let reason = desc.is_empty().then(|| NO_DESCRIPTION.to_string());
                    (
                        Content::Diagram {
                            mermaid: src,
                            description: desc,
                        },
                        reason,
                    )
                }
                Err(e) => (Content::Picture { description: desc }, Some(e)),
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

/// The Mermaid blocks of a whole-page answer, in order, each with the `>` quote lines right after
/// it. Everything else (the page text) is left out. A block the model did not close runs to the
/// end.
pub fn mermaid_sections(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    // After a closed block: the quote lines that follow it.
    let mut after_block = false;
    for line in md.lines() {
        let t = line.trim();
        if let Some(lines) = &mut current {
            lines.push(line);
            if t.starts_with("```") {
                after_block = true;
                out.push(current.take().expect("inside a block"));
            }
            continue;
        }
        if t.starts_with("```mermaid") {
            current = Some(vec![line]);
            after_block = false;
            continue;
        }
        if after_block {
            if t.starts_with('>') || t.is_empty() {
                if let Some(last) = out.last_mut() {
                    last.push(line);
                }
                continue;
            }
            after_block = false;
        }
    }
    if let Some(open) = current {
        out.push(open);
    }
    out.into_iter()
        .map(|lines| lines.join("\n").trim().to_string())
        .collect()
}

/// Splits the text of a page into one part per text region, in reading order. `weights` are the
/// region heights. The non-empty lines go to the regions in order, each region getting a share of
/// the lines in proportion to its weight (largest remainder, ties to the earlier region), and at
/// least one line while lines are left. Blank lines stay with the line before them. No line is
/// lost or doubled: the parts joined in order hold every line of the text.
pub fn split_text(md: &str, weights: &[u32]) -> Vec<String> {
    let k = weights.len();
    if k == 0 {
        return Vec::new();
    }
    let text = unwrap_fence(md);
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.iter().filter(|l| !l.trim().is_empty()).count();
    // Shares by largest remainder; a weight of 0 counts as 1.
    let w: Vec<u64> = weights.iter().map(|&x| u64::from(x.max(1))).collect();
    let total: u64 = w.iter().sum();
    let mut share: Vec<usize> = w.iter().map(|x| (n as u64 * x / total) as usize).collect();
    let mut order: Vec<usize> = (0..k).collect();
    // Larger remainder first, then the earlier region.
    order.sort_by_key(|&i| (std::cmp::Reverse(n as u64 * w[i] % total), i));
    let mut left = n - share.iter().sum::<usize>();
    for &i in order.iter().cycle() {
        if left == 0 {
            break;
        }
        share[i] += 1;
        left -= 1;
    }
    // At least one line per region while lines are left: take from the largest share.
    // Only from a region that keeps a line itself, or from one past the first n regions (those
    // get no line anyway when there are fewer lines than regions).
    let m = k.min(n);
    for i in 0..m {
        if share[i] == 0 {
            let j = (0..k)
                .filter(|&j| share[j] >= 2 || (j >= m && share[j] >= 1))
                .max_by_key(|&j| (share[j], j))
                .expect("a region with a line to spare");
            share[j] -= 1;
            share[i] += 1;
        }
    }
    let mut parts = vec![String::new(); k];
    let (mut region, mut taken, mut blanks) = (0usize, 0usize, 0usize);
    for line in lines {
        if line.trim().is_empty() {
            blanks += 1;
            continue;
        }
        while region + 1 < k && taken == share[region] {
            region += 1;
            taken = 0;
        }
        if !parts[region].is_empty() {
            parts[region].push_str(&"\n".repeat(1 + blanks));
        }
        blanks = 0;
        parts[region].push_str(line);
        taken += 1;
    }
    parts.into_iter().map(|p| p.trim().to_string()).collect()
}

/// A Mermaid section given to a diagram region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assigned {
    pub section: String,
    /// True when pagevision removed the section's description (a part of a split block); the
    /// missing description then does not lower the certainty.
    pub description_removed: bool,
}

/// The Mermaid sections of a page read for `regions` diagram regions, in reading order:
/// - with at least as many sections as regions, the first `regions` (extra ones are dropped);
/// - with fewer, the first section whose flowchart has exactly as many unconnected parts as
///   regions are left (the regions without a section, plus its own) is split into those parts in
///   place, without the section's quote (it describes the joined block, not one part);
/// - otherwise the sections as they are; the regions after them get none.
pub fn assign_blocks(sections: Vec<String>, regions: usize) -> Vec<Assigned> {
    let whole = |section: String| Assigned {
        section,
        description_removed: false,
    };
    let mut sections = sections;
    if sections.len() >= regions {
        sections.truncate(regions);
        return sections.into_iter().map(whole).collect();
    }
    let want = regions - sections.len() + 1;
    let mut out: Vec<Assigned> = Vec::with_capacity(regions);
    let mut split_done = false;
    for section in sections {
        if split_done {
            out.push(whole(section));
            continue;
        }
        let (blocks, outside) = split_fences(&section);
        let Some((_, src)) = blocks.iter().find(|(info, _)| info == "mermaid") else {
            out.push(whole(section));
            continue;
        };
        // A `>` line inside the fence is the description, as in parse_diagram; it is not
        // Mermaid.
        let (quoted, kept): (Vec<&str>, Vec<&str>) =
            src.lines().partition(|l| l.trim_start().starts_with('>'));
        let parts = match crate::mermaid::flowchart_components(kept.join("\n").trim_end()) {
            Some(p) if p.len() == want => p,
            _ => {
                out.push(whole(section));
                continue;
            }
        };
        // The block's description was written for the joined block and fits no part on its
        // own, so no part gets it.
        let had_description = !description(&format!("{}\n{outside}", quoted.join("\n"))).is_empty();
        out.extend(parts.into_iter().map(|p| Assigned {
            section: format!("```mermaid\n{p}\n```"),
            description_removed: had_description,
        }));
        split_done = true;
    }
    out
}

/// A description shorter than this (letters and digits) is never taken for page text: a word
/// or two can match the page by chance.
pub const PAGE_TEXT_MIN_CHARS: usize = 12;

/// A diagram section without its `>` description when the description only repeats the page's
/// own text: its letters and digits, lower case, are contained in those of `page_text` (the text
/// pass), and there are at least [`PAGE_TEXT_MIN_CHARS`] of them. Line breaks and punctuation do
/// not count, so a sentence the text pass broke over two lines still matches. Otherwise the
/// section as it is.
pub fn drop_page_text_description(section: &str, page_text: &str) -> String {
    let squash = |t: &str| -> String {
        t.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let (blocks, outside) = split_fences(section);
    let Some((_, src)) = blocks.iter().find(|(info, _)| info == "mermaid") else {
        return section.to_string();
    };
    let (quoted, kept): (Vec<&str>, Vec<&str>) =
        src.lines().partition(|l| l.trim_start().starts_with('>'));
    let desc = description(&format!("{}\n{outside}", quoted.join("\n")));
    let d = squash(&desc);
    if d.chars().count() < PAGE_TEXT_MIN_CHARS || !squash(page_text).contains(&d) {
        return section.to_string();
    }
    format!("```mermaid\n{}\n```", kept.join("\n").trim_end())
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

    /// Qwen sometimes writes the `>` description inside the fence, as on test2.
    #[test]
    fn quote_inside_the_fence_is_the_description() {
        let answer = "```mermaid\ngraph TD\n    A(\"search\") --> B(\"teach\")\n    B --> A\n\n    > \"A cycle of steps.\"\n```";
        let (c, reason) = parse_diagram(answer);
        assert_eq!(reason, None);
        assert_eq!(
            c,
            Content::Diagram {
                mermaid: "graph TD\n    A(\"search\") --> B(\"teach\")\n    B --> A".into(),
                description: "A cycle of steps.".into()
            }
        );
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

    /// A whole-page answer as the page prompt gives it: text, then each diagram with its quote.
    const PAGE_ANSWER: &str = "This note is without\nreal-time recognition.\n\n```mermaid\ngraph TD\n    A[search] --> B[Learn]\n```\n\n> A cycle.\n\nSome text.\n\n```mermaid\ngraph LR\n    App --> DB\n```\n> App and DB.\n> Second line.\n\nMore text.";

    #[test]
    fn mermaid_sections_keep_blocks_and_quotes_only() {
        let s = mermaid_sections(PAGE_ANSWER);
        assert_eq!(
            s,
            [
                "```mermaid\ngraph TD\n    A[search] --> B[Learn]\n```\n\n> A cycle.",
                "```mermaid\ngraph LR\n    App --> DB\n```\n> App and DB.\n> Second line.",
            ]
        );
        // Each section parses as a diagram.
        for x in &s {
            assert!(
                matches!(parse_diagram(x), (Content::Diagram { .. }, None)),
                "{x}"
            );
        }
        // An unclosed block runs to the end; no block gives nothing.
        assert_eq!(
            mermaid_sections("text\n```mermaid\ngraph TD\n  A --> B"),
            ["```mermaid\ngraph TD\n  A --> B"]
        );
        assert!(mermaid_sections("Only text.\n\n```python\nx = 1\n```").is_empty());
    }

    fn lines(parts: &[String]) -> Vec<String> {
        parts
            .iter()
            .flat_map(|p| p.lines())
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn split_text_by_region_height() {
        let md = "# Title\n\nline 2\nline 3\n\n- a\n- b\n- c";
        // One region takes everything.
        assert_eq!(split_text(md, &[40]), [md]);
        // Heights 1 and 2: 2 and 4 of the 6 lines.
        let parts = split_text(md, &[10, 20]);
        assert_eq!(parts, ["# Title\n\nline 2", "line 3\n\n- a\n- b\n- c"]);
        // A tie goes to the earlier region: 5 lines, 3 and 2.
        let parts = split_text("a\nb\nc\nd\ne", &[10, 10]);
        assert_eq!(parts, ["a\nb\nc", "d\ne"]);
        // Every region gets a line while lines are left, even a small one.
        let parts = split_text("a\nb\nc\nd", &[100, 1]);
        assert_eq!(parts, ["a\nb\nc", "d"]);
        // More regions than lines.
        assert_eq!(split_text("a\nb", &[1, 1, 1]), ["a", "b", ""]);
        // Empty text, no regions.
        assert_eq!(split_text("  \n", &[1, 2]), ["", ""]);
        assert!(split_text("a", &[]).is_empty());
        // A fenced answer is unwrapped first.
        assert_eq!(split_text("```markdown\nx\ny\n```", &[1, 1]), ["x", "y"]);
    }

    #[test]
    fn split_text_loses_and_doubles_nothing() {
        let md = "one\ntwo\n\nthree\nfour\nfive\n\n\nsix\nseven";
        let want: Vec<String> = md
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        for weights in [&[1u32][..], &[3, 1], &[1, 1, 1], &[5, 1, 9, 2], &[1; 9]] {
            let parts = split_text(md, weights);
            assert_eq!(parts.len(), weights.len());
            assert_eq!(lines(&parts), want, "{weights:?}");
        }
    }

    /// test2 as the page read wrote it: both diagrams in one block, with one quote.
    const TEST2_PAGE_ANSWER: &str = "This note is without\nreal-time recognition.\n\n```mermaid\ngraph TD\n    A[search] --> B[Learn]\n    B --> C[teach]\n    C --> A\n    D[App] --> E[DB]\n    E <--> F[MCP]\n```\n\n> This note is without real-time recognition.";

    #[test]
    fn one_block_with_two_parts_fills_two_regions() {
        // The block's one description covers both parts, so no part gets it, and each part says
        // that pagevision removed it.
        let blocks = assign_blocks(mermaid_sections(TEST2_PAGE_ANSWER), 2);
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().all(|b| b.description_removed));
        let (first, _) = parse_diagram(&blocks[0].section);
        let (second, _) = parse_diagram(&blocks[1].section);
        assert_eq!(
            first,
            Content::Diagram {
                mermaid: "graph TD\n    A[search] --> B[Learn]\n    B --> C[teach]\n    C --> A"
                    .into(),
                description: String::new()
            }
        );
        assert_eq!(
            second,
            Content::Diagram {
                mermaid: "graph TD\n    D[App] --> E[DB]\n    E <--> F[MCP]".into(),
                description: String::new()
            }
        );
        let plain = |v: Vec<Assigned>| -> Vec<String> {
            assert!(v.iter().all(|a| !a.description_removed));
            v.into_iter().map(|a| a.section).collect()
        };
        // One region: the block stays whole, with its description.
        let one = plain(assign_blocks(mermaid_sections(TEST2_PAGE_ANSWER), 1));
        assert_eq!(one, mermaid_sections(TEST2_PAGE_ANSWER));
        // Three regions: two parts do not fill three, so nothing is split.
        let three = plain(assign_blocks(mermaid_sections(TEST2_PAGE_ANSWER), 3));
        assert_eq!(three, mermaid_sections(TEST2_PAGE_ANSWER));
        // Extra blocks are dropped.
        let two = mermaid_sections(PAGE_ANSWER);
        assert_eq!(plain(assign_blocks(two.clone(), 1)), [two[0].clone()]);
        // A split block that had no description: nothing was removed.
        let bare = "```mermaid\ngraph TD\n    A --> B\n    C --> D\n```".to_string();
        let parts = assign_blocks(vec![bare], 2);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|a| !a.description_removed));
    }

    #[test]
    fn split_text_gives_early_regions_their_line() {
        // Two lines, a tall last region: the first two regions still get a line each.
        assert_eq!(split_text("a\nb", &[1, 1, 100]), ["a", "b", ""]);
        assert_eq!(split_text("a\nb\nc", &[1, 1, 100]), ["a", "b", "c"]);
    }

    #[test]
    fn page_text_description_is_dropped() {
        // test2: the text pass broke the sentence over two lines with a blank line.
        let page_text = "This note is without\n\nreal-time recognition.";
        let section = "```mermaid\ngraph LR\n    App --> DB\n```\n\n> This note is without real-time recognition.";
        let out = drop_page_text_description(section, page_text);
        assert_eq!(out, "```mermaid\ngraph LR\n    App --> DB\n```");
        assert!(matches!(
            parse_diagram(&out),
            (Content::Diagram { description, .. }, _) if description.is_empty()
        ));
        // The same quote inside the fence.
        let inside = "```mermaid\ngraph LR\n    App --> DB\n    > \"This note is without real-time recognition.\"\n```";
        assert_eq!(
            drop_page_text_description(inside, page_text),
            "```mermaid\ngraph LR\n    App --> DB\n```"
        );
        // A real description stays.
        let real = "```mermaid\ngraph LR\n    App --> DB\n```\n\n> App writes to the database.";
        assert_eq!(drop_page_text_description(real, page_text), real);
        // A short one stays even when the page holds it.
        let short = "```mermaid\ngraph LR\n    A --> B\n```\n> Real time.";
        assert_eq!(drop_page_text_description(short, page_text), short);
        // No page text: nothing to compare.
        assert_eq!(drop_page_text_description(section, ""), section);
    }

    /// The minimum length on both sides: 11 letters and digits stay, 12 are dropped.
    #[test]
    fn page_text_threshold_is_twelve_characters() {
        assert_eq!(PAGE_TEXT_MIN_CHARS, 12);
        let page_text = "Notes: abcdefghijkl and more.";
        let eleven = "```mermaid\ngraph LR\n    A --> B\n```\n> abcde fghijk";
        let twelve = "```mermaid\ngraph LR\n    A --> B\n```\n> abcde fghijkl";
        assert_eq!(drop_page_text_description(eleven, page_text), eleven);
        assert_eq!(
            drop_page_text_description(twelve, page_text),
            "```mermaid\ngraph LR\n    A --> B\n```"
        );
    }
}
