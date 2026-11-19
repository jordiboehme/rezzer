// Adapted from oar-ocr-vl 0.10.3 src/models/pp_doclayout/err.rs (Apache-2.0, Copyright the
// oar-ocr authors, https://github.com/GreatV/oar-ocr). Changes: the helpers return PageError
// instead of the oar-ocr-vl error type. Licensed under the Apache License, Version 2.0; see
// LICENSE-APACHE in this crate.

//! Error helpers shared by the PP-DocLayout modules.

use crate::PageError;

pub(super) const MODEL_NAME: &str = "PP-DocLayout";

/// Wraps a Candle failure that happened while loading or running the model.
pub(super) fn infer_err(context: &'static str, error: candle_core::Error) -> PageError {
    PageError::Engine(format!("{MODEL_NAME}: {context}: {error}"))
}

/// Wraps a Candle failure in tensor pre/post-processing.
pub(super) fn proc_err(context: &'static str, error: candle_core::Error) -> PageError {
    PageError::Engine(format!("{MODEL_NAME}: {context}: {error}"))
}

/// A configuration or shape the port does not support.
pub(super) fn config_err(message: String) -> PageError {
    PageError::Engine(message)
}
