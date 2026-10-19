// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Signatures at fixed positions. Every check looks at a bounded prefix only.

use rezzer_model::{Confidence, FormatId};

use crate::{Found, found};

pub(crate) const OLE_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
pub(crate) const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
/// An empty ZIP archive is only its end-of-central-directory record.
pub(crate) const ZIP_EMPTY_MAGIC: &[u8] = b"PK\x05\x06";

/// How far into the file a PDF header may start (readers accept leading junk).
const PDF_WINDOW: usize = 1024;
/// How far the STEP header (up to the first `ENDSEC;`) is searched.
const STEP_HEADER_WINDOW: usize = 64 * 1024;
/// How far an ASCII STL is searched for its first `facet normal`.
const STL_WINDOW: usize = 4096;

/// DWG version strings of the readable releases (see docs/specs/dwg-format.md, section 2).
const DWG_VERSIONS: &[(&[u8], &str)] = &[
    (b"AC1012", "R13"),
    (b"AC1013", "R13C3"),
    (b"AC1014", "R14"),
    (b"AC1015", "R2000"),
    (b"AC1018", "R2004"),
    (b"AC1021", "R2007"),
    (b"AC1024", "R2010"),
    (b"AC1027", "R2013"),
    (b"AC1032", "R2018"),
];

/// Checks the fixed magic numbers that are not containers. OLE and ZIP are handled by the
/// caller because they need a look inside.
pub(crate) fn detect(bytes: &[u8]) -> Option<Found> {
    let head = &bytes[..bytes.len().min(PDF_WINDOW)];
    if bytes.starts_with(b"%PDF-") {
        return Some(found(
            FormatId::Pdf,
            Confidence::Certain,
            "pdf header %PDF-",
        ));
    }
    // Leading junk before the header is accepted by PDF readers. The header must still start
    // a line, so a text that only mentions it mid-sentence is not taken for a PDF.
    if let Some(pos) = find(head, b"%PDF-")
        && matches!(head[pos - 1], b'\n' | b'\r')
    {
        return Some(found(
            FormatId::Pdf,
            Confidence::Likely,
            format!("pdf header %PDF- at offset {pos}"),
        ));
    }
    if bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) {
        return Some(found(
            FormatId::Eps,
            Confidence::Certain,
            "dos eps binary header C5D0D3C6",
        ));
    }
    if bytes.starts_with(b"%!PS-Adobe") {
        let line = first_line(bytes, 256);
        if find(line, b" EPSF").is_some() {
            return Some(found(
                FormatId::Eps,
                Confidence::Certain,
                "postscript header %!PS-Adobe with EPSF",
            ));
        }
        return Some(found(
            FormatId::PostScript,
            Confidence::Certain,
            "postscript header %!PS-Adobe",
        ));
    }
    if bytes.starts_with(b"{\\rtf") {
        return Some(found(
            FormatId::Rtf,
            Confidence::Certain,
            "rtf header {\\rtf",
        ));
    }
    if let Some(f) = dwg(bytes) {
        return Some(f);
    }
    if bytes.starts_with(b"ISO-10303-21;") {
        return Some(step(bytes));
    }
    if bytes.starts_with(b"glTF") {
        return Some(found(
            FormatId::Glb,
            Confidence::Certain,
            "binary gltf magic glTF",
        ));
    }
    if bytes.starts_with(b"GCDE") {
        return Some(found(
            FormatId::Bgcode,
            Confidence::Certain,
            "prusa binary gcode magic GCDE",
        ));
    }
    None
}

/// Checks the signatures that are patterns rather than fixed magic numbers. Runs after the
/// container check, so a ZIP or OLE file is never taken for one of these by chance.
pub(crate) fn detect_patterns(bytes: &[u8]) -> Option<Found> {
    if let Some(f) = iges(bytes) {
        return Some(f);
    }
    if let Some(f) = binary_stl(bytes) {
        return Some(f);
    }
    ascii_stl(bytes)
}

/// Pre-R13 version strings. These files have a different structure and are not readable;
/// they are still named, so the output can say what the file is.
const DWG_PRE_R13: &[&[u8]] = &[
    b"AC1.2", b"AC1.40", b"AC1.50", b"AC2.10", b"AC1001", b"AC1002", b"AC1003", b"AC1004",
    b"AC1005", b"AC1006", b"AC1007", b"AC1008", b"AC1009", b"MC0.0",
];

fn dwg(bytes: &[u8]) -> Option<Found> {
    if let Some(magic) = DWG_PRE_R13.iter().find(|m| bytes.starts_with(m)) {
        // The version string is followed by binary header bytes. Text that starts with
        // the same characters ("AC1.2 is a part number") is left to the text checks.
        let next = *bytes.get(magic.len())?;
        if next.is_ascii_graphic() || next.is_ascii_whitespace() {
            return None;
        }
        return Some(found(
            FormatId::Dwg,
            Confidence::Likely,
            format!(
                "dwg version string {} (pre-R13 format, not readable)",
                String::from_utf8_lossy(magic)
            ),
        ));
    }
    let magic = bytes.get(..6)?;
    if !magic.starts_with(b"AC10") || !magic[4..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let version = String::from_utf8_lossy(magic);
    match DWG_VERSIONS.iter().find(|(m, _)| *m == magic) {
        Some((_, release)) => {
            // Bytes 6 to 10 are zero in every R13+ header; text that happens to start with
            // the same six characters does not have them.
            let zeros = bytes.get(6..10).is_some_and(|z| z.iter().all(|&b| b == 0));
            let confidence = if zeros {
                Confidence::Certain
            } else {
                Confidence::Likely
            };
            Some(found(
                FormatId::Dwg,
                confidence,
                format!("dwg version string {version} ({release})"),
            ))
        }
        None => {
            // A digit string next to a NUL byte is binary; plain text starting with
            // "AC10" and digits is left to the text checks.
            if bytes.get(6) != Some(&0) {
                return None;
            }
            Some(found(
                FormatId::Dwg,
                Confidence::Likely,
                format!("dwg version string {version} (not a release Rezzer knows)"),
            ))
        }
    }
}

fn step(bytes: &[u8]) -> Found {
    let window = &bytes[..bytes.len().min(STEP_HEADER_WINDOW)];
    let header_end = find(window, b"ENDSEC;").unwrap_or(window.len());
    let header = &window[..header_end];
    let Some(pos) = find(header, b"FILE_SCHEMA") else {
        return found(
            FormatId::Step,
            Confidence::Likely,
            "iso-10303-21 header without FILE_SCHEMA",
        );
    };
    let rest = &header[pos..];
    let end = rest.iter().position(|&b| b == b';').unwrap_or(rest.len());
    let schema_stmt = &rest[..end];
    // The first quoted name in FILE_SCHEMA(('...')).
    let name = schema_stmt
        .iter()
        .position(|&b| b == b'\'')
        .map(|s| {
            let tail = &schema_stmt[s + 1..];
            let e = tail.iter().position(|&b| b == b'\'').unwrap_or(tail.len());
            String::from_utf8_lossy(&tail[..e]).trim().to_string()
        })
        .unwrap_or_default();
    if name.to_ascii_uppercase().starts_with("IFC") {
        found(
            FormatId::Ifc,
            Confidence::Certain,
            format!("iso-10303-21 header with FILE_SCHEMA {name}"),
        )
    } else {
        found(
            FormatId::Step,
            Confidence::Certain,
            format!("iso-10303-21 header with FILE_SCHEMA {name}"),
        )
    }
}

/// IGES: fixed 80-column records; column 73 of the first record is `S` (start section)
/// and columns 74 to 80 hold the sequence number 1.
fn iges(bytes: &[u8]) -> Option<Found> {
    let line = first_line(bytes, 200);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.len() != 80 || line[72] != b'S' {
        return None;
    }
    let seq = std::str::from_utf8(&line[73..80]).ok()?;
    if seq.trim().parse::<u32>().ok()? != 1 {
        return None;
    }
    Some(found(
        FormatId::Iges,
        Confidence::Certain,
        "iges start record (column 73 S, sequence 1)",
    ))
}

/// Binary STL: 80-byte header, u32 triangle count, 50 bytes per triangle.
fn binary_stl(bytes: &[u8]) -> Option<Found> {
    let count = u32::from_le_bytes(bytes.get(80..84)?.try_into().ok()?);
    if count == 0 || 84 + 50 * u64::from(count) != bytes.len() as u64 {
        return None;
    }
    Some(found(
        FormatId::Stl,
        Confidence::Likely,
        format!("binary stl: 80-byte header and {count} triangles match the file size"),
    ))
}

fn ascii_stl(bytes: &[u8]) -> Option<Found> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace())?;
    let rest = &bytes[start..];
    if !rest.starts_with(b"solid") || !rest.get(5).is_some_and(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let window = &rest[..rest.len().min(STL_WINDOW)];
    find(window, b"facet normal")?;
    Some(found(
        FormatId::Stl,
        Confidence::Certain,
        "ascii stl: solid with facet normal",
    ))
}

fn first_line(bytes: &[u8], max: usize) -> &[u8] {
    let head = &bytes[..bytes.len().min(max)];
    let end = head.iter().position(|&b| b == b'\n').unwrap_or(head.len());
    &head[..end]
}

pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
