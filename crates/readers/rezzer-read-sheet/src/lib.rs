// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Spreadsheet reader: XLSX, XLSM, XLSB and ODS workbooks with their values and the logic
//! behind them.
//!
//! calamine gives the values, cached results and formula texts (through its sparse cell
//! readers for XLSX and XLSB; for ODS only after the size of its dense sheets is bounded),
//! merged cells and VBA module names. sheetparts gives what calamine does not expose: sheet
//! states, names with scope, styles, comments, tables, pivots, charts, validations and data
//! tables, and the ODF parts. formulakit reads every formula. zipguard measures every entry
//! before calamine unpacks anything. VBA, add-ins and linked files are listed, never run or
//! opened. Encrypted workbooks are reported, never decrypted.

mod biff12;
mod grid;
mod logic;
mod ods;
mod odscan;
mod package;
mod units;
mod xlsb;
mod xlsx;

use rezzer_limits::Budget;
use rezzer_model::{
    Block, Completeness, Document, FormatId, Inline, Loss, LossReason, ReadOptions, ReadOutcome,
    Reader, ReaderInfo, Severity, Source, source_info,
};

/// Reads XLSX, XLSM and XLSB workbooks (feature `xlsx`) and ODS workbooks (feature `ods`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SheetReader;

/// The formats this build reads.
const FORMATS: &[FormatId] = match (cfg!(feature = "xlsx"), cfg!(feature = "ods")) {
    (true, true) => &[
        FormatId::Xlsx,
        FormatId::Xlsm,
        FormatId::Xlsb,
        FormatId::Ods,
    ],
    (true, false) => &[FormatId::Xlsx, FormatId::Xlsm, FormatId::Xlsb],
    (false, true) => &[FormatId::Ods],
    (false, false) => &[],
};

/// The first bytes of an OLE compound file.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

impl Reader for SheetReader {
    fn id(&self) -> &'static str {
        "sheet"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        FORMATS
    }

    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        let mut info = source_info(src);
        info.reader = Some(ReaderInfo {
            id: self.id().to_string(),
            version: self.version().to_string(),
        });
        let mut out = Out::new(Document::new(info));
        let bytes = src.bytes();
        // A password-protected OOXML file is an OLE compound file around an encrypted
        // package, whatever the extension. It is not decrypted.
        if bytes.starts_with(&OLE_MAGIC) {
            return ole(bytes, out);
        }
        match src.format {
            FormatId::Xlsx | FormatId::Xlsm if cfg!(feature = "xlsx") => {
                xlsx::read(bytes, budget, out)
            }
            FormatId::Xlsb if cfg!(feature = "xlsx") => xlsb::read(bytes, budget, out),
            FormatId::Ods if cfg!(feature = "ods") => ods::read(bytes, budget, out),
            other => {
                out.loss(
                    "document",
                    format!("the whole file: {other:?} is not read by this build"),
                    LossReason::NotImplemented,
                    Severity::Error,
                );
                out.failed()
            }
        }
    }
}

/// An OLE compound file given as a workbook: encrypted (an `EncryptedPackage` stream) or not
/// a workbook package at all.
fn ole(bytes: &[u8], mut out: Out) -> ReadOutcome {
    let encrypted = rezzer_limits::contain(|| {
        cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map(|c| {
            c.read_root_storage()
                .any(|e| e.name().eq_ignore_ascii_case("EncryptedPackage"))
        })
    });
    let (what, why) = match encrypted {
        Ok(Ok(true)) => (
            "the whole file: the workbook is protected with a password (an encrypted package); it is not decrypted".to_string(),
            LossReason::Encrypted,
        ),
        Ok(Ok(false)) => (
            "the whole file: an OLE compound file without an encrypted package, not a workbook package".to_string(),
            LossReason::Damaged,
        ),
        Ok(Err(e)) => (
            format!("the whole file: a broken OLE compound file: {e}"),
            LossReason::Damaged,
        ),
        Err(panic) => (
            format!("the whole file: listing the OLE compound file failed: {panic}"),
            LossReason::Internal,
        ),
    };
    if why == LossReason::Encrypted {
        out.doc.body.push(Block::paragraph(vec![Inline::plain(
            "This workbook is protected with a password and is not read.",
        )]));
    }
    out.loss("document", what, why, Severity::Error);
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
