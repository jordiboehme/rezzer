// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The error type of the crate.

use std::path::PathBuf;

/// Why reading a page failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PageError {
    /// No readable block on the page.
    #[error("no readable block on the page: {reason}")]
    Unreadable { reason: String },
    #[error("model file missing: {}", path.display())]
    ModelMissing { path: PathBuf },
    /// The image size has no entry in the token table.
    #[error("the image size {width}x{height} has no entry in the token table")]
    UnknownImageSize { width: u32, height: u32 },
    /// llama.cpp produced a different image token count than the table says.
    #[error(
        "image {width}x{height}: expected {expected} image tokens, llama.cpp produced {actual}"
    )]
    ImageTokens {
        width: u32,
        height: u32,
        expected: u32,
        actual: u32,
    },
    #[error("image error: {0}")]
    Image(String),
    #[error("engine error: {0}")]
    Engine(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_size() {
        let e = PageError::UnknownImageSize {
            width: 7,
            height: 9,
        };
        assert!(e.to_string().contains("7x9"));
    }
}
