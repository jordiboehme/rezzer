// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Plain text reader: any encoding in, one section of paragraphs out.

use rezzer_limits::Budget;
use rezzer_model::{
    Block, Completeness, Document, FormatId, Inline, Loss, LossReason, ReadOptions, ReadOutcome,
    Reader, ReaderInfo, SectionKind, Severity, Source, source_info,
};

/// How many paragraphs are read between two time checks.
const CHECK_EVERY: usize = 1000;

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

        let text = decoded.text.replace("\r\n", "\n").replace('\r', "\n");
        let mut blocks = Vec::new();
        let paragraphs = split_paragraphs(&text);
        for paragraph in paragraphs {
            let done = blocks.len();
            if done > 0 && done % CHECK_EVERY == 0 && budget.check_time().is_err() {
                completeness = Completeness::Partial;
                doc.losses.push(Loss {
                    location: format!("paragraph {}", done + 1),
                    what: "rest of the text not read".into(),
                    why: LossReason::Limit,
                    severity: Severity::Warning,
                });
                break;
            }
            blocks.push(Block::Paragraph {
                inlines: vec![Inline::Text {
                    text: paragraph,
                    emphasis: false,
                    strong: false,
                }],
                role: None,
                anchor: None,
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

/// Splits LF-normalized text into paragraphs. Blank lines (empty or only whitespace) separate
/// paragraphs; the lines of one paragraph are joined with a newline.
fn split_paragraphs(text: &str) -> impl Iterator<Item = String> + '_ {
    let mut lines = text.split('\n');
    std::iter::from_fn(move || {
        let mut current: Vec<&str> = Vec::new();
        for line in lines.by_ref() {
            if line.trim().is_empty() {
                if !current.is_empty() {
                    break;
                }
            } else {
                current.push(line);
            }
        }
        if current.is_empty() {
            None
        } else {
            Some(current.join("\n"))
        }
    })
}
