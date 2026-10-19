// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Looks inside ZIP and OLE (CFB) containers to tell their formats apart.
//!
//! Only the directory is listed. At most [`READ_BUDGET`] bytes are decompressed in total
//! (`[Content_Types].xml` and `mimetype`). A broken container never panics: it is reported as
//! `Zip` or `Ole` with `Likely` confidence and the error as evidence.

use std::io::{Cursor, Read};

use rezzer_model::{Confidence, FormatId};

use crate::{Found, clean, found};

/// Most bytes decompressed while looking inside one container.
const READ_BUDGET: u64 = 64 * 1024;

/// OOXML main part content types (the `+xml` suffix is left out on purpose: the binary
/// workbook has none). Order matters only where one is a prefix of another, which none is.
const OOXML_MAIN: &[(&str, FormatId, &str)] = &[
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main",
        FormatId::Docx,
        "wordprocessingml",
    ),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main",
        FormatId::Docx,
        "wordprocessingml template",
    ),
    (
        "application/vnd.ms-word.document.macroEnabled.main",
        FormatId::Docx,
        "wordprocessingml macro-enabled",
    ),
    (
        "application/vnd.ms-word.template.macroEnabledTemplate.main",
        FormatId::Docx,
        "wordprocessingml macro-enabled template",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main",
        FormatId::Xlsx,
        "spreadsheetml",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main",
        FormatId::Xlsx,
        "spreadsheetml template",
    ),
    (
        "application/vnd.ms-excel.sheet.macroEnabled.main",
        FormatId::Xlsm,
        "spreadsheetml macro-enabled",
    ),
    (
        "application/vnd.ms-excel.template.macroEnabled.main",
        FormatId::Xlsm,
        "spreadsheetml macro-enabled template",
    ),
    (
        "application/vnd.ms-excel.sheet.binary.macroEnabled.main",
        FormatId::Xlsb,
        "binary spreadsheet",
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main",
        FormatId::Pptx,
        "presentationml",
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main",
        FormatId::Pptx,
        "presentationml slideshow",
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.template.main",
        FormatId::Pptx,
        "presentationml template",
    ),
    (
        "application/vnd.ms-powerpoint.presentation.macroEnabled.main",
        FormatId::Pptx,
        "presentationml macro-enabled",
    ),
    (
        "application/vnd.ms-powerpoint.slideshow.macroEnabled.main",
        FormatId::Pptx,
        "presentationml macro-enabled slideshow",
    ),
];

const THREE_MF_TYPE: &str = "application/vnd.ms-package.3dmanufacturing-3dmodel+xml";

/// ODF and EPUB `mimetype` values.
const MIMETYPES: &[(&str, FormatId)] = &[
    ("application/vnd.oasis.opendocument.text", FormatId::Odt),
    (
        "application/vnd.oasis.opendocument.text-template",
        FormatId::Odt,
    ),
    (
        "application/vnd.oasis.opendocument.text-master",
        FormatId::Odt,
    ),
    (
        "application/vnd.oasis.opendocument.spreadsheet",
        FormatId::Ods,
    ),
    (
        "application/vnd.oasis.opendocument.spreadsheet-template",
        FormatId::Ods,
    ),
    (
        "application/vnd.oasis.opendocument.presentation",
        FormatId::Odp,
    ),
    (
        "application/vnd.oasis.opendocument.presentation-template",
        FormatId::Odp,
    ),
    ("application/epub+zip", FormatId::Epub),
];

/// Detects the format of a ZIP archive.
pub(crate) fn zip(bytes: &[u8]) -> Found {
    match rezzer_limits::contain(|| zip_inner(bytes)) {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => found(
            FormatId::Zip,
            Confidence::Likely,
            format!("zip signature, but the archive directory is unreadable: {e}"),
        ),
        Err(panic) => found(
            FormatId::Zip,
            Confidence::Likely,
            format!("zip signature, but listing the archive failed: {panic}"),
        ),
    }
}

fn zip_inner(bytes: &[u8]) -> zip::result::ZipResult<Found> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut budget = READ_BUDGET;

    let names: Vec<String> = archive
        .file_names()
        .filter_map(Result::ok)
        .map(|n| n.into_owned())
        .collect();
    let has = |wanted: &str| names.iter().any(|n| n.eq_ignore_ascii_case(wanted));

    // 3MF is an OPC package too; its model part decides before the office main parts.
    if has("3D/3dmodel.model") {
        return Ok(found(
            FormatId::ThreeMf,
            Confidence::Certain,
            "zip with 3D/3dmodel.model part",
        ));
    }

    if has("[Content_Types].xml") {
        let ct = read_entry(&mut archive, "[Content_Types].xml", &mut budget);
        if let Some(ct) = &ct {
            if let Some((_, format, label)) = OOXML_MAIN
                .iter()
                .find(|(t, _, _)| content_type_present(ct, t))
            {
                return Ok(found(
                    *format,
                    Confidence::Certain,
                    format!("zip with [Content_Types].xml main part {label}"),
                ));
            }
            if content_type_present(ct, THREE_MF_TYPE) {
                return Ok(found(
                    FormatId::ThreeMf,
                    Confidence::Certain,
                    "zip with [Content_Types].xml 3mf model content type",
                ));
            }
        }
        // Content types unreadable, cut off by the budget, or without a known main part:
        // the usual part names still give the format away without decompressing anything.
        let reason = if ct.is_some() {
            "no known main part"
        } else {
            "unreadable"
        };
        let by_part: &[(&str, FormatId, &str)] = &[
            ("word/document.xml", FormatId::Docx, "word/document.xml"),
            (
                "ppt/presentation.xml",
                FormatId::Pptx,
                "ppt/presentation.xml",
            ),
        ];
        if has("xl/workbook.xml") {
            let (format, extra) = if has("xl/vbaProject.bin") {
                (FormatId::Xlsm, " and xl/vbaProject.bin")
            } else {
                (FormatId::Xlsx, "")
            };
            return Ok(found(
                format,
                Confidence::Likely,
                format!("zip with [Content_Types].xml ({reason}) and part xl/workbook.xml{extra}"),
            ));
        }
        if has("xl/workbook.bin") {
            return Ok(found(
                FormatId::Xlsb,
                Confidence::Likely,
                format!("zip with [Content_Types].xml ({reason}) and part xl/workbook.bin"),
            ));
        }
        if let Some((_, format, part)) = by_part.iter().find(|(p, _, _)| has(p)) {
            return Ok(found(
                *format,
                Confidence::Likely,
                format!("zip with [Content_Types].xml ({reason}) and part {part}"),
            ));
        }
    }

    if let Some(raw) = read_entry(&mut archive, "mimetype", &mut budget) {
        let mime = raw.trim();
        let shown = clean(mime);
        if let Some((_, format)) = MIMETYPES.iter().find(|(m, _)| *m == mime) {
            return Ok(found(
                *format,
                Confidence::Certain,
                format!("zip with mimetype {shown}"),
            ));
        }
        return Ok(found(
            FormatId::Zip,
            Confidence::Certain,
            format!("zip with mimetype {shown} (not a format Rezzer reads)"),
        ));
    }

    if names.iter().any(|n| n == "Document.xml") {
        return Ok(found(
            FormatId::Fcstd,
            Confidence::Likely,
            "zip with FreeCAD Document.xml",
        ));
    }

    Ok(found(
        FormatId::Zip,
        Confidence::Certain,
        format!(
            "zip archive with {} entries, no known document layout",
            names.len()
        ),
    ))
}

/// True when `ct` holds `ContentType="<wanted>"` or `ContentType="<wanted>+xml"`.
fn content_type_present(ct: &str, wanted: &str) -> bool {
    let mut rest = ct;
    while let Some(pos) = rest.find(wanted) {
        let before = &rest[..pos];
        let after = &rest[pos + wanted.len()..];
        let quoted_before = before.ends_with('"') || before.ends_with('\'');
        let tail_ok = after.starts_with('"')
            || after.starts_with('\'')
            || after.starts_with("+xml\"")
            || after.starts_with("+xml'");
        if quoted_before && tail_ok {
            return true;
        }
        rest = after;
    }
    false
}

/// Reads one entry as text, decompressing at most what is left of `budget`.
/// Returns None when the entry is missing, encrypted, broken or not fully read.
fn read_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    budget: &mut u64,
) -> Option<String> {
    let index = archive.index_for_name(name)?;
    let entry = archive.by_index(index).ok()?;
    let declared = entry.size();
    if declared > *budget {
        return None;
    }
    let mut buf = Vec::new();
    // One byte more than allowed tells a full read from a cut one.
    let read = entry.take(*budget + 1).read_to_end(&mut buf).ok()? as u64;
    *budget = budget.saturating_sub(read);
    if read > declared {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Detects the format of an OLE compound file by its root stream names.
pub(crate) fn ole(bytes: &[u8]) -> Found {
    match rezzer_limits::contain(|| ole_inner(bytes)) {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => found(
            FormatId::Ole,
            Confidence::Likely,
            format!("ole signature, but the compound file directory is unreadable: {e}"),
        ),
        Err(panic) => found(
            FormatId::Ole,
            Confidence::Likely,
            format!("ole signature, but listing the compound file failed: {panic}"),
        ),
    }
}

fn ole_inner(bytes: &[u8]) -> std::io::Result<Found> {
    let comp = cfb::CompoundFile::open(Cursor::new(bytes))?;
    let names: Vec<String> = comp.read_root_storage().map(|e| clean(e.name())).collect();
    let has = |wanted: &str| names.iter().any(|n| n == wanted);
    let pick = [
        ("WordDocument", FormatId::Doc),
        ("Workbook", FormatId::Xls),
        ("Book", FormatId::Xls),
        ("PowerPoint Document", FormatId::Ppt),
    ]
    .into_iter()
    .find(|(n, _)| has(n));
    if let Some((stream, format)) = pick {
        return Ok(found(
            format,
            Confidence::Certain,
            format!("ole compound file with stream {stream}"),
        ));
    }
    if has("EncryptedPackage") {
        return Ok(found(
            FormatId::Ole,
            Confidence::Certain,
            "ole compound file with EncryptedPackage (password-protected office file)",
        ));
    }
    let mut listed: Vec<&str> = names.iter().map(String::as_str).take(8).collect();
    listed.sort_unstable();
    Ok(found(
        FormatId::Ole,
        Confidence::Certain,
        format!(
            "ole compound file without a known document stream (root entries: {})",
            listed.join(", ")
        ),
    ))
}
