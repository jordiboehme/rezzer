// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Looks inside ZIP and OLE (CFB) containers to tell their formats apart.
//!
//! Only the directory is listed, and only when the end record says it has at most
//! 100,000 entries and 64 MiB; larger directories are not listed during detection. At most
//! [`READ_BUDGET`] bytes are decompressed in total (`[Content_Types].xml`, `mimetype` and the
//! Fusion `Manifest.dat`). A broken container never panics: it is reported as
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

/// Most entries listed during detection.
const MAX_ENTRIES: u64 = 100_000;
/// Largest central directory listed during detection.
const MAX_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;

/// Entry names that identify a layout. Matched without regard to ASCII case.
const MARKERS: &[&str] = &[
    "[Content_Types].xml",
    "3D/3dmodel.model",
    "mimetype",
    "Document.xml",
    "word/document.xml",
    "xl/workbook.xml",
    "xl/vbaProject.bin",
    "xl/workbook.bin",
    "ppt/presentation.xml",
    "Manifest.dat",
];

/// The entry names found for [`MARKERS`], as stored in the archive.
struct Markers {
    names: Vec<Option<String>>,
    /// Some entry lies under a Fusion `*.BlobParts/` folder.
    blob_parts: bool,
    entries: usize,
}

impl Markers {
    fn get(&self, marker: &str) -> Option<&str> {
        let i = MARKERS.iter().position(|m| *m == marker)?;
        self.names[i].as_deref()
    }

    fn has(&self, marker: &str) -> bool {
        self.get(marker).is_some()
    }
}

fn zip_inner(bytes: &[u8]) -> zip::result::ZipResult<Found> {
    if let Some(why) = directory_too_large(bytes) {
        return Ok(found(
            FormatId::Zip,
            Confidence::Likely,
            format!("zip directory too large to list during detection ({why})"),
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut budget = READ_BUDGET;
    let markers = scan_markers(&archive);

    // 3MF is an OPC package too; its model part decides before the office main parts.
    if markers.has("3D/3dmodel.model") {
        return Ok(found(
            FormatId::ThreeMf,
            Confidence::Certain,
            "zip with 3D/3dmodel.model part",
        ));
    }

    if let Some(ct_name) = markers.get("[Content_Types].xml") {
        let ct = read_entry(&mut archive, ct_name, &mut budget);
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
        let by_part: &[(&str, FormatId)] = &[
            ("word/document.xml", FormatId::Docx),
            ("ppt/presentation.xml", FormatId::Pptx),
            ("xl/workbook.bin", FormatId::Xlsb),
        ];
        if markers.has("xl/workbook.xml") {
            let (format, extra) = if markers.has("xl/vbaProject.bin") {
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
        if let Some((part, format)) = by_part.iter().find(|(p, _)| markers.has(p)) {
            return Ok(found(
                *format,
                Confidence::Likely,
                format!("zip with [Content_Types].xml ({reason}) and part {part}"),
            ));
        }
    }

    if let Some(name) = markers.get("mimetype")
        && let Some(raw) = read_entry(&mut archive, name, &mut budget)
    {
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

    if markers.has("Document.xml") {
        return Ok(found(
            FormatId::Fcstd,
            Confidence::Likely,
            "zip with FreeCAD Document.xml",
        ));
    }

    // Fusion 360: a top-level Manifest.dat that starts with the length-prefixed strings
    // "3-2-0-0" and "FusionDocType", next to `*.BlobParts/` folders. Format facts from the
    // cadmpeg documentation (CC BY 4.0).
    if let Some(name) = markers.get("Manifest.dat") {
        let manifest = read_entry_bytes(&mut archive, name, &mut budget);
        let head = manifest.as_deref().map(|m| &m[..m.len().min(64)]);
        if head.is_some_and(|h| crate::magic::find(h, b"FusionDocType").is_some()) {
            return Ok(found(
                FormatId::F3d,
                Confidence::Certain,
                "zip with Manifest.dat of type FusionDocType",
            ));
        }
        if markers.blob_parts {
            return Ok(found(
                FormatId::F3d,
                Confidence::Likely,
                "zip with Manifest.dat and *.BlobParts/ folders",
            ));
        }
    }

    Ok(found(
        FormatId::Zip,
        Confidence::Certain,
        format!(
            "zip archive with {} entries, no known document layout",
            markers.entries
        ),
    ))
}

/// One pass over the names; only marker entries are kept.
fn scan_markers<R: Read + std::io::Seek>(archive: &zip::ZipArchive<R>) -> Markers {
    let mut m = Markers {
        names: vec![None; MARKERS.len()],
        blob_parts: false,
        entries: archive.len(),
    };
    for name in archive.file_names().filter_map(Result::ok) {
        if let Some(i) = MARKERS.iter().position(|w| w.eq_ignore_ascii_case(&name))
            && m.names[i].is_none()
        {
            m.names[i] = Some(name.into_owned());
            continue;
        }
        if !m.blob_parts {
            m.blob_parts = name
                .as_bytes()
                .windows(11)
                .any(|w| w.eq_ignore_ascii_case(b".BlobParts/"));
        }
    }
    m
}

/// Reads the end of central directory record (and the ZIP64 one) and says why the
/// directory is too large to list, if it is.
fn directory_too_large(bytes: &[u8]) -> Option<String> {
    const EOCD: &[u8] = b"PK\x05\x06";
    let window = bytes.len().saturating_sub(22 + 65_535);
    let tail = &bytes[window..];
    // The last record whose comment length reaches the end of the file; else the last one.
    let mut pos = None;
    let mut i = tail.len().saturating_sub(22);
    loop {
        if tail[i..].starts_with(EOCD) && i + 22 <= tail.len() {
            let comment = u64::from(u16_at(tail, i + 20)?);
            if (i + 22) as u64 + comment == tail.len() as u64 {
                pos = Some(i);
                break;
            }
            pos.get_or_insert(i);
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    let p = pos?;
    let mut entries = u64::from(u16_at(tail, p + 10)?);
    let mut size = u64::from(u32_at(tail, p + 12)?);
    let offset = u32_at(tail, p + 16)?;
    if entries == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        // ZIP64: the locator sits right before the classic record.
        let loc = (window + p).checked_sub(20);
        let z64 = loc
            .filter(|&l| bytes.get(l..l + 4) == Some(b"PK\x06\x07"))
            .and_then(|l| u64_at(bytes, l + 8))
            .and_then(|o| usize::try_from(o).ok())
            .filter(|&o| bytes.get(o..o + 4) == Some(b"PK\x06\x06"));
        match z64 {
            Some(o) => {
                entries = u64_at(bytes, o + 32)?;
                size = u64_at(bytes, o + 40)?;
            }
            // A saturated count without ZIP64 records: every central directory record is
            // at least 46 bytes, so the directory size bounds the count.
            None if entries == 0xFFFF => entries = entries.max(size / 46),
            None => {}
        }
    }
    if entries > MAX_ENTRIES {
        return Some(format!("{entries} entries, more than {MAX_ENTRIES}"));
    }
    if size > MAX_DIRECTORY_BYTES {
        return Some(format!(
            "{size} bytes of directory, more than {MAX_DIRECTORY_BYTES}"
        ));
    }
    None
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
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
    read_entry_bytes(archive, name, budget).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Like [`read_entry`], as bytes. `name` is the name as stored in the archive.
fn read_entry_bytes<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    budget: &mut u64,
) -> Option<Vec<u8>> {
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
    Some(buf)
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

/// Native MCAD compound files: Inventor (`RSeStorage` with `RSeSegInfo` or `V<n>/RSeDb`),
/// legacy NX (`UG_PART/UG_PART`) and SolidWorks (`ISolidWorksInformation`). Format facts
/// from the cadmpeg documentation (CC BY 4.0).
fn ole_inner(bytes: &[u8]) -> std::io::Result<Found> {
    if ole_directory_too_large(bytes) {
        return Ok(found(
            FormatId::Ole,
            Confidence::Likely,
            format!(
                "ole directory too large to list during detection (more than {MAX_OLE_ENTRIES} entries)"
            ),
        ));
    }
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes))?;
    let mut root = Vec::new();
    let mut shown = Vec::new();
    for e in comp.read_root_storage() {
        if shown.len() < 8 {
            shown.push(clean(e.name()));
        }
        let name = e.name();
        let known = [
            "WordDocument",
            "Workbook",
            "Book",
            "PowerPoint Document",
            "EncryptedPackage",
            "RSeStorage",
            "UG_PART",
            "ISolidWorksInformation",
        ];
        if let Some(k) = known.iter().find(|k| k.eq_ignore_ascii_case(name)) {
            root.push((*k, e.is_storage()));
        }
    }
    let has = |wanted: &str| root.iter().any(|(n, _)| *n == wanted);
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
    if has("RSeStorage") {
        if comp.is_stream("/RSeStorage/RSeSegInfo") {
            let mut info = Vec::new();
            if let Ok(stream) = comp.open_stream("/RSeStorage/RSeSegInfo") {
                // A short or broken read only weakens the part/assembly decision.
                let _ = stream.take(READ_BUDGET).read_to_end(&mut info);
            }
            let (format, kind) = inventor_kind(&info);
            return Ok(found(
                format,
                Confidence::Certain,
                format!("ole compound file with RSeStorage/RSeSegInfo (inventor {kind})"),
            ));
        }
        let db = comp.read_storage("/RSeStorage").ok().and_then(|entries| {
            entries
                .filter(|e| e.is_storage())
                .map(|e| e.name().to_string())
                .find(|n| {
                    n.len() > 1
                        && n.starts_with(['V', 'v'])
                        && n[1..].bytes().all(|b| b.is_ascii_digit())
                })
        });
        if let Some(v) = db.filter(|v| comp.is_stream(format!("/RSeStorage/{v}/RSeDb"))) {
            return Ok(found(
                FormatId::Ipt,
                Confidence::Certain,
                format!(
                    "ole compound file with RSeStorage/{}/RSeDb (inventor part or assembly)",
                    clean(&v)
                ),
            ));
        }
        return Ok(found(
            FormatId::Ipt,
            Confidence::Likely,
            "ole compound file with RSeStorage storage (inventor layout)",
        ));
    }
    if has("UG_PART") && comp.is_stream("/UG_PART/UG_PART") {
        return Ok(found(
            FormatId::NxPrt,
            Confidence::Certain,
            "ole compound file with UG_PART/UG_PART (legacy nx part)",
        ));
    }
    if has("ISolidWorksInformation") {
        return Ok(found(
            FormatId::Sldprt,
            Confidence::Certain,
            "ole compound file with ISolidWorksInformation (solidworks)",
        ));
    }
    shown.sort_unstable();
    Ok(found(
        FormatId::Ole,
        Confidence::Certain,
        format!(
            "ole compound file without a known document stream (root entries: {})",
            shown.join(", ")
        ),
    ))
}

/// Inventor document kind from the segment type names in `RSeSegInfo`: `Pm*` families mean a
/// part, `Am*` families an assembly (format facts from the cadmpeg documentation, CC BY 4.0).
/// Type names are u32-counted UTF-16LE strings; any counted ASCII name of the form
/// `Am`/`Pm` + capital letter counts.
fn inventor_kind(info: &[u8]) -> (FormatId, &'static str) {
    let (mut part, mut assembly) = (false, false);
    let mut i = 0;
    while i + 10 <= info.len() && !(part && assembly) {
        let n = u32_at(info, i).unwrap_or(0) as usize;
        let name = info.get(i + 4..i + 4 + 2 * n);
        let ascii = |k: usize| {
            name.and_then(|b| b.get(2 * k..2 * k + 2))
                .filter(|u| u[1] == 0)
                .map(|u| u[0])
        };
        if (3..=64).contains(&n)
            && (0..n).all(|k| ascii(k).is_some_and(|c| c.is_ascii_alphanumeric()))
            && ascii(1) == Some(b'm')
            && ascii(2).is_some_and(|c| c.is_ascii_uppercase())
        {
            match ascii(0) {
                Some(b'P') => part = true,
                Some(b'A') => assembly = true,
                _ => {}
            }
            i += 4 + 2 * n;
            continue;
        }
        i += 1;
    }
    match (part, assembly) {
        (_, true) if part => (FormatId::Iam, "document with part and assembly segments"),
        (_, true) => (FormatId::Iam, "assembly, Am* segments"),
        (true, false) => (FormatId::Ipt, "part, Pm* segments"),
        (false, false) => (FormatId::Ipt, "part or assembly, segment types not found"),
    }
}

/// Most compound file directory entries listed during detection.
const MAX_OLE_ENTRIES: u64 = 10_000;

/// Follows the directory sector chain in the FAT, without loading the directory, and says
/// whether it holds more than [`MAX_OLE_ENTRIES`] entries. Anything unreadable is left to the
/// cfb crate (false).
fn ole_directory_too_large(bytes: &[u8]) -> bool {
    ole_directory_sectors(bytes, MAX_OLE_ENTRIES).is_some_and(|too_large| too_large)
}

fn ole_directory_sectors(bytes: &[u8], max_entries: u64) -> Option<bool> {
    const END: u32 = 0xFFFF_FFFA; // ids from here on are markers, not sectors
    let shift = u16_at(bytes, 0x1E)?;
    if !(9..=12).contains(&shift) {
        return None;
    }
    let ss = 1usize << shift;
    let per_sector = (ss / 4) as u32;
    let sector = |id: u32| -> Option<&[u8]> {
        let start = (id as usize).checked_add(1)?.checked_mul(ss)?;
        bytes.get(start..start.checked_add(ss)?)
    };
    let num_fat = u32_at(bytes, 0x2C)? as usize;
    let max_sectors = bytes.len() / ss + 1;
    if num_fat > max_sectors {
        return None;
    }
    // FAT sector ids: 109 in the header, the rest in the DIFAT chain.
    let mut fat_ids: Vec<u32> = (0..109.min(num_fat))
        .map(|k| u32_at(bytes, 0x4C + 4 * k))
        .collect::<Option<_>>()?;
    let mut difat = u32_at(bytes, 0x44)?;
    let mut guard = 0;
    while fat_ids.len() < num_fat && difat < END {
        let sec = sector(difat)?;
        for k in 0..(per_sector as usize - 1) {
            if fat_ids.len() == num_fat {
                break;
            }
            fat_ids.push(u32_at(sec, 4 * k)?);
        }
        difat = u32_at(sec, ss - 4)?;
        guard += 1;
        if guard > max_sectors {
            return None;
        }
    }
    let next = |id: u32| -> Option<u32> {
        let fat_sector = *fat_ids.get((id / per_sector) as usize)?;
        u32_at(sector(fat_sector)?, 4 * (id % per_sector) as usize)
    };
    let entries_per_sector = (ss / 128) as u64;
    let max_dir_sectors = max_entries.div_ceil(entries_per_sector);
    let mut id = u32_at(bytes, 0x30)?;
    let mut count = 0u64;
    while id < END {
        count += 1;
        if count > max_dir_sectors {
            return Some(true);
        }
        id = next(id)?;
    }
    Some(false)
}
