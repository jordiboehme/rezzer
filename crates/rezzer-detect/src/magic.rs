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

/// Fixed prefixes: (magic, format, evidence).
///
/// Native MCAD signatures (CATIA `V5_CFV2`, NX `SPLMSSTR`, Creo `#UGC:`): format facts from
/// the cadmpeg documentation (CC BY 4.0), checked against public sample files.
const PREFIXES: &[(&[u8], FormatId, &str)] = &[
    (b"%PDF-", FormatId::Pdf, "pdf header %PDF-"),
    (
        &[0xC5, 0xD0, 0xD3, 0xC6],
        FormatId::Eps,
        "dos eps binary header C5D0D3C6",
    ),
    (b"{\\rtf", FormatId::Rtf, "rtf header {\\rtf"),
    (b"glTF", FormatId::Glb, "binary gltf magic glTF"),
    (b"GCDE", FormatId::Bgcode, "prusa binary gcode magic GCDE"),
    (
        b"AutoCAD Binary DXF\r\n\x1a\x00",
        FormatId::Dxf,
        "binary dxf sentinel AutoCAD Binary DXF",
    ),
    (
        b"3D Geometry File Format ",
        FormatId::Rhino3dm,
        "rhino 3dm header 3D Geometry File Format",
    ),
    (
        b"V5_CFV2\x00",
        FormatId::Catpart,
        "catia v5 container magic V5_CFV2",
    ),
    (
        b"SPLMSSTR",
        FormatId::NxPrt,
        "siemens nx container magic SPLMSSTR",
    ),
    (b"#UGC:", FormatId::CreoPrt, "creo ugc header #UGC:"),
    (
        b"EESchema Schematic File Version",
        FormatId::KicadSch,
        "kicad legacy schematic header EESchema Schematic File Version",
    ),
    (
        b"PCBNEW-BOARD Version",
        FormatId::KicadPcb,
        "kicad legacy board header PCBNEW-BOARD Version",
    ),
];

/// Checks the fixed magic numbers that are not containers. OLE and ZIP are handled by the
/// caller because they need a look inside.
pub(crate) fn detect(bytes: &[u8]) -> Option<Found> {
    if let Some((_, format, evidence)) = PREFIXES.iter().find(|(m, _, _)| bytes.starts_with(m)) {
        return Some(found(*format, Confidence::Certain, *evidence));
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
    if let Some(f) = dwg(bytes) {
        return Some(f);
    }
    if bytes.starts_with(b"ISO-10303-21;") {
        return Some(step(bytes));
    }
    None
}

/// Checks the signatures that are patterns rather than fixed magic numbers. Runs after the
/// container check, so a ZIP or OLE file is never taken for one of these by chance.
pub(crate) fn detect_patterns(bytes: &[u8]) -> Option<Found> {
    if let Some(f) = pdf_after_junk(bytes) {
        return Some(f);
    }
    if let Some(f) = iges(bytes) {
        return Some(f);
    }
    if let Some(f) = ascii_dxf(bytes) {
        return Some(f);
    }
    if let Some(f) = sldprt_blocks(bytes) {
        return Some(f);
    }
    if let Some(f) = binary_stl(bytes) {
        return Some(f);
    }
    ascii_stl(bytes)
}

/// PDF readers accept junk before the header. Only a binary prefix counts: it must hold a
/// control byte (0x00 to 0x08, 0x0E to 0x1F or 0x7F). Text before `%PDF-`, in any encoding,
/// is a text that quotes the header.
fn pdf_after_junk(bytes: &[u8]) -> Option<Found> {
    let head = &bytes[..bytes.len().min(PDF_WINDOW)];
    let pos = find(head, b"%PDF-")?;
    let binary = head[..pos]
        .iter()
        .any(|&b| matches!(b, 0x00..=0x08 | 0x0E..=0x1F | 0x7F));
    binary.then(|| {
        found(
            FormatId::Pdf,
            Confidence::Likely,
            format!("pdf header %PDF- at offset {pos} after a binary prefix"),
        )
    })
}

/// ASCII DXF: group code / value line pairs, starting with `0`/`SECTION`/`2`/<section name>.
/// Leading `999` comment pairs are skipped.
fn ascii_dxf(bytes: &[u8]) -> Option<Found> {
    let head = &bytes[..bytes.len().min(4096)];
    // Later bytes may be cut or in a legacy code page; the first lines are ASCII.
    let text = match std::str::from_utf8(head) {
        Ok(t) => t,
        Err(e) => std::str::from_utf8(&head[..e.valid_up_to()]).ok()?,
    };
    let mut lines = text.lines().map(str::trim);
    let mut first = lines.next()?;
    while first == "999" {
        lines.next()?;
        first = lines.next()?;
    }
    let pair = [first, lines.next()?, lines.next()?, lines.next()?];
    let sections = [
        "HEADER", "CLASSES", "TABLES", "BLOCKS", "ENTITIES", "OBJECTS",
    ];
    if pair[0] != "0" || pair[1] != "SECTION" || pair[2] != "2" || !sections.contains(&pair[3]) {
        return None;
    }
    Some(found(
        FormatId::Dxf,
        Confidence::Certain,
        format!("ascii dxf group codes 0 SECTION 2 {}", pair[3]),
    ))
}

/// SolidWorks block envelope: u32 file id, u32 big-endian version 4, then blocks that start
/// with the marker `14 00 06 00 08 00`. Format facts from the cadmpeg documentation
/// (CC BY 4.0). The compound-file envelope is handled in the container module.
fn sldprt_blocks(bytes: &[u8]) -> Option<Found> {
    if bytes.get(4..8)? != [0, 0, 0, 4] {
        return None;
    }
    let head = &bytes[8..bytes.len().min(4096)];
    let pos = find(head, &[0x14, 0x00, 0x06, 0x00, 0x08, 0x00])?;
    Some(found(
        FormatId::Sldprt,
        Confidence::Likely,
        format!(
            "solidworks block envelope (version 4, block marker at offset {})",
            pos + 8
        ),
    ))
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
