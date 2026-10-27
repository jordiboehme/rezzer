// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `file` input: a `data:` URI parsed into a name, a declared type and the bytes.

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

use super::MCP_FILE_NOT_SUPPORTED;

/// The file name used for a data: URI without `;name=`.
pub(super) const DEFAULT_NAME: &str = "file";

/// A file sent inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileData {
    pub name: String,
    /// The type the sender declared, a detection hint only.
    pub mime: Option<String>,
    pub bytes: Vec<u8>,
}

/// Parses the `file` input: `data:[<type>][;name=<percent-encoded name>][;<other>=<value>]
/// [;base64],<data>`. The data is base64 or percent-encoded. The URI is consumed: the
/// decoded bytes are the only copy left. The decoded size is counted before anything is
/// copied, so a file larger than `max` bytes is refused without decoding it, and the
/// output buffer is allocated once at that size.
pub fn parse_file_uri(uri: String, max: u64) -> Result<FileData, String> {
    let trimmed = uri.trim();
    if starts_with_ignore_case(trimmed, "mcp-file:") {
        return Err(MCP_FILE_NOT_SUPPORTED.to_string());
    }
    if !starts_with_ignore_case(trimmed, "data:") {
        return Err(
            "file must be a data: URI, for example data:text/csv;name=a.csv;base64,<data>"
                .to_string(),
        );
    }
    let Some((header, data)) = trimmed["data:".len()..].split_once(',') else {
        return Err("the data: URI has no comma before the data".to_string());
    };
    let tokens: Vec<&str> = header.split(';').map(str::trim).collect();
    let mut mime = None;
    let mut name = None;
    let mut base64 = false;
    for (i, token) in tokens.iter().enumerate() {
        if i + 1 == tokens.len() && i > 0 && token.eq_ignore_ascii_case("base64") {
            base64 = true;
        } else if i == 0 {
            if token.contains('/') {
                mime = Some(token.to_ascii_lowercase());
            }
        } else if let Some((key, value)) = token.split_once('=')
            && key.trim().eq_ignore_ascii_case("name")
        {
            let decoded = percent_encoding::percent_decode_str(value.trim())
                .decode_utf8()
                .map_err(|_| "the name in the data: URI is not valid UTF-8".to_string())?;
            name = Some(file_name(&decoded));
        }
    }
    let size = if base64 {
        base64_decoded_len(data.as_bytes())
    } else {
        percent_decoded_len(data.as_bytes())
    };
    if size as u64 > max {
        return Err(format!(
            "file is too large to send inline: {size} bytes, the limit is {max} bytes; give it by path"
        ));
    }
    let bytes = if base64 {
        decode_base64(data.as_bytes(), size)?
    } else {
        let mut bytes = Vec::with_capacity(size);
        bytes.extend(percent_encoding::percent_decode(data.as_bytes()));
        bytes
    };
    drop(uri);
    Ok(FileData {
        name: name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| DEFAULT_NAME.to_string()),
        mime,
        bytes,
    })
}

/// The size base64 text decodes to, counted without copying: whitespace and padding are
/// not symbols, every four symbols are three bytes. Exact for valid input.
pub(super) fn base64_decoded_len(data: &[u8]) -> usize {
    let symbols = data
        .iter()
        .filter(|b| !b.is_ascii_whitespace() && **b != b'=')
        .count();
    symbols / 4 * 3 + (symbols % 4).saturating_sub(1)
}

/// The size percent-encoded text decodes to: every valid `%XX` escape is one byte, all
/// else stays as it is.
pub(super) fn percent_decoded_len(data: &[u8]) -> usize {
    let mut len = 0;
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'%'
            && data.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
            && data.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            i += 3;
        } else {
            i += 1;
        }
        len += 1;
    }
    len
}

/// Symbols per decode step; a multiple of 4, so every step but the last is whole quads.
pub(super) const BASE64_CHUNK: usize = 64 * 1024;

/// Decodes base64 into one buffer of `size` bytes (plus the decoder's small slack). Text
/// without whitespace is decoded in place; wrapped text goes through a small chunk buffer
/// that skips the whitespace, so the encoded text is never copied as a whole.
fn decode_base64(data: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let invalid = |e: base64::DecodeError, offset: usize| {
        let detail = match e {
            base64::DecodeError::InvalidByte(at, byte) => format!(
                "the character {:?} at symbol {} is not base64",
                char::from(byte),
                offset + at + 1
            ),
            base64::DecodeError::InvalidLength(_) => {
                "the length is not valid, the data may be cut off".to_string()
            }
            base64::DecodeError::InvalidLastSymbol { .. } => {
                "the last symbol is not valid, the data may be cut off".to_string()
            }
            base64::DecodeError::InvalidPadding => "the padding is not valid".to_string(),
        };
        format!("the base64 data in the data: URI is not valid: {detail}")
    };
    // decode_vec reserves its estimate, at most two bytes over the exact size.
    let mut out = Vec::with_capacity(size + 3);
    if !data.iter().any(u8::is_ascii_whitespace) {
        BASE64
            .decode_vec(data, &mut out)
            .map_err(|e| invalid(e, 0))?;
        return Ok(out);
    }
    let mut chunk = Vec::with_capacity(BASE64_CHUNK);
    let mut offset = 0;
    for &b in data {
        if b.is_ascii_whitespace() {
            continue;
        }
        chunk.push(b);
        if chunk.len() == BASE64_CHUNK {
            BASE64
                .decode_vec(&chunk, &mut out)
                .map_err(|e| invalid(e, offset))?;
            offset += chunk.len();
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        BASE64
            .decode_vec(&chunk, &mut out)
            .map_err(|e| invalid(e, offset))?;
    }
    Ok(out)
}

/// Standard base64; padding may be there or not.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// The last component of a sent name, so a name like `../x/a.csv` is only `a.csv`.
fn file_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}
