// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Markdown reader: CommonMark with GFM tables, alerts and task lists, footnotes, math,
//! wikilinks and YAML or TOML frontmatter, read into the document model.
//!
//! The parser events are consumed by an explicit stack, never by recursion, so nesting of any
//! depth cannot overflow the call stack. Containers (block quote, list, list item, footnote
//! definition) are charged to the depth budget; a subtree over the limit is skipped and
//! reported as a loss.

mod builder;
mod frontmatter;

use rezzer_limits::Budget;
use rezzer_model::{
    Completeness, Document, FormatId, Loss, LossReason, ReadOptions, ReadOutcome, Reader,
    ReaderInfo, Severity, Source, source_info,
};

use builder::Builder;

/// Reads Markdown. The encoding is detected; existing frontmatter becomes properties, GFM
/// alerts become callouts and links and wikilinks form a link graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownReader;

impl Reader for MarkdownReader {
    fn id(&self) -> &'static str {
        "markdown"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Markdown]
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

        if !decoded.confident {
            doc.losses.push(Loss {
                location: "document".into(),
                what: "some bytes could not be decoded and were replaced with U+FFFD".into(),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }

        let mut builder = Builder::new(budget);
        builder.run(&decoded.text);
        let first_h1 = builder.first_h1.take();
        builder.finish_into(&mut doc);
        if doc.properties.title.is_none() {
            doc.properties.title = first_h1;
        }
        builder.add_graph(&mut doc, &src.name);

        let completeness = if doc
            .losses
            .iter()
            .any(|l| matches!(l.why, LossReason::Limit | LossReason::Damaged))
        {
            Completeness::Partial
        } else {
            Completeness::Complete
        };
        ReadOutcome {
            blobs: Vec::new(),
            document: doc,
            completeness,
        }
    }
}
