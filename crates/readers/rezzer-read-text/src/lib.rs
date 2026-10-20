// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Plain text reader: any encoding in, one section of paragraphs out.

use std::borrow::Cow;

use rezzer_limits::Budget;
use rezzer_model::{
    Block, Completeness, Document, FormatId, Inline, Loss, LossReason, ReadOptions, ReadOutcome,
    Reader, ReaderInfo, SectionKind, Severity, Source, source_info,
};
use textenc::LineEnding;

/// How many paragraphs are read between two time checks.
const CHECK_EVERY: usize = 1000;

/// How many bytes of text are read between two time checks, so one huge paragraph is covered.
const CHECK_BYTES: usize = 1024 * 1024;

/// Model elements one line adds to its paragraph: the block (first line) or a line break
/// (further lines), and the text.
const LINE_ELEMENTS: u64 = 2;

const TIME_STOP: &str = "rest of the text not read";
const TEXT_STOP: &str = "text limit reached, rest of the paragraph and the text not read";
const ELEMENT_STOP: &str = "element limit reached, rest of the text not read";

/// Reads plain text. The encoding is detected, line endings are normalized to LF and the text
/// is split into paragraphs at blank lines.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextReader;

impl Reader for TextReader {
    fn id(&self) -> &'static str {
        "text"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }

    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        let decoded = textenc::decode(src.bytes());
        let mut info = source_info(src);
        info.encoding = Some(decoded.encoding.to_string());
        info.reader = Some(ReaderInfo {
            id: self.id().to_string(),
            version: self.version().to_string(),
        });
        let mut doc = Document::new(info);
        let mut completeness = Completeness::Complete;

        if !decoded.confident {
            completeness = Completeness::Partial;
            doc.losses.push(Loss {
                location: "document".into(),
                what: "some bytes could not be decoded and were replaced with U+FFFD".into(),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }

        let text = normalize_newlines(&decoded.text, decoded.line_ending);
        let max_text = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
        let mut blocks: Vec<Block> = Vec::new();
        let mut current: Vec<&str> = Vec::new();
        // Bytes of the paragraph in `current`, line breaks included.
        let mut current_bytes = 0usize;
        let mut since_check = 0usize;
        let mut stopped_at: Option<(usize, &str)> = None;
        for line in text.split('\n') {
            if line.trim().is_empty() {
                if !current.is_empty() {
                    blocks.push(paragraph(&current));
                    current.clear();
                    current_bytes = 0;
                }
                continue;
            }
            if current.is_empty() && !blocks.is_empty() && blocks.len().is_multiple_of(CHECK_EVERY)
            {
                // Checked lazily, only when another paragraph follows.
                if budget.check_time().is_err() {
                    stopped_at = Some((blocks.len() + 1, TIME_STOP));
                    break;
                }
            }
            // Every line is charged before it is kept: the block and its text for the first
            // line, a line break and a text for each further one.
            if budget.add_elements(LINE_ELEMENTS).is_err() {
                // What was read of the paragraph is kept.
                let kept = !current.is_empty();
                if kept {
                    blocks.push(paragraph(&current));
                    current.clear();
                }
                stopped_at = Some((blocks.len() + usize::from(!kept), ELEMENT_STOP));
                break;
            }
            let separator = usize::from(!current.is_empty());
            if budget
                .charge_text(current_bytes + separator + line.len())
                .is_err()
            {
                // Keep what fits of the paragraph, cut on a character boundary.
                let room = max_text.saturating_sub(current_bytes + separator);
                let mut cut = room.min(line.len());
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                if cut > 0 {
                    current.push(&line[..cut]);
                }
                let kept = !current.is_empty();
                if kept {
                    blocks.push(paragraph(&current));
                    current.clear();
                }
                stopped_at = Some((blocks.len() + usize::from(!kept), TEXT_STOP));
                break;
            }
            since_check += line.len() + 1;
            if since_check >= CHECK_BYTES {
                since_check = 0;
                if budget.check_time().is_err() {
                    // Keep what is read of the current paragraph.
                    current.push(line);
                    blocks.push(paragraph(&current));
                    stopped_at = Some((blocks.len(), TIME_STOP));
                    current.clear();
                    break;
                }
            }
            current_bytes += separator + line.len();
            current.push(line);
        }
        if stopped_at.is_none() && !current.is_empty() {
            blocks.push(paragraph(&current));
        }
        if let Some((n, what)) = stopped_at {
            completeness = Completeness::Partial;
            doc.losses.push(Loss {
                location: format!("paragraph {n}"),
                what: what.into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        }

        doc.body.push(Block::Section {
            kind: SectionKind::Other,
            title: None,
            anchor: None,
            blocks,
        });
        ReadOutcome {
            document: doc,
            completeness,
        }
    }
}

/// One paragraph: the lines as text inlines with a line break between them.
fn paragraph(lines: &[&str]) -> Block {
    let mut inlines = Vec::with_capacity(lines.len() * 2);
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            inlines.push(Inline::LineBreak);
        }
        inlines.push(Inline::plain(*line));
    }
    Block::paragraph(inlines)
}

/// Turns CRLF and lone CR into LF in one pass. LF-only text and text without line breaks are
/// borrowed unchanged.
fn normalize_newlines(text: &str, ending: LineEnding) -> Cow<'_, str> {
    if matches!(ending, LineEnding::Lf | LineEnding::None) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}
