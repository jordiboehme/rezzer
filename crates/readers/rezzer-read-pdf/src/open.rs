// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opening the file with pdfparts under the read budget, and what a failed open tells.

use std::panic::AssertUnwindSafe;

use pdfparts::{OpenError, Pdf};
use rezzer_limits::{Budget, contain};
use rezzer_model::{Block, Inline, LossReason, ReadOutcome, Severity};

use crate::Out;

/// The pdfparts limits for a read budget. One stream may unpack to the unpacked limit, all
/// streams together too; parsed objects and the canonical copy may take half the memory
/// limit.
pub(crate) fn limits(budget: &Budget) -> pdfparts::Limits {
    let lim = budget.limits();
    pdfparts::Limits {
        max_depth: usize::try_from(lim.max_depth).unwrap_or(usize::MAX),
        max_items: usize::try_from(lim.max_elements).unwrap_or(usize::MAX),
        max_stream_bytes: lim.max_unpacked_bytes,
        max_total_bytes: lim.max_unpacked_bytes,
        max_ratio: u64::from(lim.max_ratio),
        max_object_bytes: lim.max_memory_bytes / 2,
        ..pdfparts::Limits::default()
    }
}

/// Opens the file. On failure the outcome to return is ready.
pub(crate) fn open(
    bytes: &[u8],
    password: Option<&str>,
    budget: &Budget,
    mut out: Out,
) -> Result<(Pdf, Out), Box<ReadOutcome>> {
    let stop = || budget.check_time().is_err();
    let opened = contain(AssertUnwindSafe(|| {
        Pdf::open(bytes, password, &limits(budget), &stop)
    }));
    let pdf = match opened {
        Ok(Ok(pdf)) => pdf,
        Ok(Err(e)) => return Err(Box::new(refused(e, out))),
        Err(panic) => {
            out.loss(
                "document",
                format!("the whole file: opening it stopped with an internal error: {panic}"),
                LossReason::Internal,
                Severity::Error,
            );
            return Err(Box::new(out.failed()));
        }
    };
    if let Err(hit) = budget.add_unpacked(pdf.unpacked()) {
        out.loss(
            "document",
            format!("the whole file: {hit}"),
            LossReason::Limit,
            Severity::Error,
        );
        return Err(Box::new(out.failed()));
    }
    Ok((pdf, out))
}

/// The outcome of an open that failed.
fn refused(e: OpenError, mut out: Out) -> ReadOutcome {
    let (said, why, what) = match &e {
        OpenError::NeedsPassword => (
            Some("This PDF is protected with a password and is not read."),
            LossReason::Encrypted,
            "the whole file: it is encrypted and needs a password".to_string(),
        ),
        OpenError::WrongPassword => (
            Some("The given password does not open this PDF."),
            LossReason::Encrypted,
            "the whole file: the given password does not open it".to_string(),
        ),
        OpenError::UnsupportedEncryption(_) => {
            (None, LossReason::Encrypted, format!("the whole file: {e}"))
        }
        OpenError::Limit(_) => (None, LossReason::Limit, format!("the whole file: {e}")),
        OpenError::NotPdf | OpenError::Damaged(_) => return unreadable(&e, out),
    };
    if let Some(said) = said {
        out.doc
            .body
            .push(Block::paragraph(vec![Inline::plain(said)]));
    }
    out.loss("document", what, why, Severity::Error);
    out.failed()
}

/// A file pdfparts cannot read as a PDF: not read at all.
fn unreadable(e: &OpenError, mut out: Out) -> ReadOutcome {
    out.loss(
        "document",
        format!("the whole file: {e}"),
        LossReason::Damaged,
        Severity::Error,
    );
    out.failed()
}
