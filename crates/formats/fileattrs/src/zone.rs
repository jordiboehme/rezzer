// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Windows `Zone.Identifier` stream: INI text with a `[ZoneTransfer]` section.

/// Decodes the bytes of a `Zone.Identifier` stream.
///
/// A UTF-16LE byte order mark (FF FE) means UTF-16LE, a UTF-8 byte order mark is dropped,
/// anything else is read as UTF-8. Bytes that are not valid text become U+FFFD. Returns
/// `None` for an empty stream.
#[must_use]
pub fn zone_text(bytes: &[u8]) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units = rest
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair.get(1).copied().unwrap_or(0)]));
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    } else {
        let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        String::from_utf8_lossy(rest).into_owned()
    };
    (!text.is_empty()).then_some(text)
}

/// The `HostUrl` value of the `[ZoneTransfer]` section: the address the file was
/// downloaded from.
///
/// Section and key names match without regard to case, a line splits at its first `=`,
/// and spaces around keys and values are trimmed. Returns `None` when there is no
/// `[ZoneTransfer]` section, no `HostUrl` key in it, or an empty value. The first
/// `HostUrl` wins.
#[must_use]
pub fn zone_host_url(text: &str) -> Option<String> {
    let mut in_zone = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_zone = name.trim().eq_ignore_ascii_case("ZoneTransfer");
            continue;
        }
        if !in_zone {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("HostUrl") {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}
