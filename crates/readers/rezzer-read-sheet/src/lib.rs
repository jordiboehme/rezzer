// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Spreadsheet reader: XLSX and XLSM workbooks with their values and the logic behind them.
//!
//! calamine gives the values, cached results and formula texts (through its sparse cell
//! reader, never a dense range), merged cells and VBA module names. sheetparts gives what
//! calamine does not expose: sheet states, names with scope, styles, comments, tables,
//! pivots, charts, validations and data tables. formulakit reads every formula. zipguard
//! measures every entry before calamine unpacks anything. VBA, add-ins and linked files are listed,
//! never run or opened.

mod grid;
mod logic;
mod package;
mod units;
mod xlsx;

use rezzer_limits::Budget;
use rezzer_model::{
    Block, Completeness, Document, FormatId, Inline, Loss, LossReason, ReadOptions, ReadOutcome,
    Reader, ReaderInfo, Severity, Source, source_info,
};

/// Reads XLSX and XLSM workbooks. XLSB and ODS are claimed too; their readers come later and
/// until then the read fails with a `not_implemented` loss.
#[derive(Debug, Clone, Copy, Default)]
pub struct SheetReader;

impl Reader for SheetReader {
    fn id(&self) -> &'static str {
        "sheet"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        &[
            FormatId::Xlsx,
            FormatId::Xlsm,
            FormatId::Xlsb,
            FormatId::Ods,
        ]
    }

    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        let mut info = source_info(src);
        info.reader = Some(ReaderInfo {
            id: self.id().to_string(),
            version: self.version().to_string(),
        });
        let out = Out::new(Document::new(info));
        match src.format {
            FormatId::Xlsx | FormatId::Xlsm => xlsx::read(src.bytes(), budget, out),
            other => not_implemented(out, other),
        }
    }
}

/// A failed outcome for a format whose reader is not built yet.
fn not_implemented(mut out: Out, format: FormatId) -> ReadOutcome {
    let name = match format {
        FormatId::Xlsb => "XLSB",
        FormatId::Ods => "ODS",
        _ => "these",
    };
    out.doc
        .body
        .push(Block::paragraph(vec![Inline::plain(format!(
            "{name} workbooks are not read yet."
        ))]));
    out.loss(
        "document",
        format!("the whole file: {name} workbooks are not read yet"),
        LossReason::NotImplemented,
        Severity::Error,
    );
    out.failed()
}

/// The document being built, with whether something was not read.
pub(crate) struct Out {
    pub doc: Document,
    pub partial: bool,
}

impl Out {
    fn new(doc: Document) -> Out {
        Out {
            doc,
            partial: false,
        }
    }

    /// Adds a loss. Damaged parts, limits and parts not read make the read partial; an
    /// unsupported detail or an uncertain fact does not.
    pub fn loss(
        &mut self,
        location: impl Into<String>,
        what: impl Into<String>,
        why: LossReason,
        severity: Severity,
    ) {
        if matches!(
            why,
            LossReason::Damaged
                | LossReason::Limit
                | LossReason::Encrypted
                | LossReason::NotImplemented
                | LossReason::Internal
        ) {
            self.partial = true;
        }
        self.doc.losses.push(Loss {
            location: location.into(),
            what: what.into(),
            why,
            severity,
        });
    }

    pub fn finish(self) -> ReadOutcome {
        ReadOutcome {
            document: self.doc,
            completeness: if self.partial {
                Completeness::Partial
            } else {
                Completeness::Complete
            },
        }
    }

    pub fn failed(self) -> ReadOutcome {
        ReadOutcome {
            document: self.doc,
            completeness: Completeness::Failed,
        }
    }
}

/// Inlines of a text with line breaks: each line break becomes [`Inline::LineBreak`].
pub(crate) fn text_inlines(text: &str) -> Vec<Inline> {
    if !text.contains(['\n', '\r']) {
        return vec![Inline::plain(text)];
    }
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push(Inline::LineBreak);
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.is_empty() {
            out.push(Inline::plain(line));
        }
    }
    out
}
