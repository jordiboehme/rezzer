// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};

use rezzer_detect::{OpenError, detect, from_bytes, open};
use rezzer_limits::Limits;
use rezzer_model::{Confidence, FormatId};
use zip::write::SimpleFileOptions;

const CT_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
"#;

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        w.start_file(*name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn content_types(overrides: &str) -> Vec<u8> {
    format!("{CT_HEAD}{overrides}</Types>").into_bytes()
}

fn odf(mimetype: &str) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("mimetype", stored).unwrap();
    w.write_all(mimetype.as_bytes()).unwrap();
    w.start_file("content.xml", stored).unwrap();
    w.write_all(b"<office:document-content/>").unwrap();
    w.finish().unwrap().into_inner()
}

fn cfb_with(streams: &[&str]) -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    for name in streams {
        let mut s = comp.create_stream(format!("/{name}")).unwrap();
        s.write_all(&[0u8; 600]).unwrap();
    }
    comp.flush().unwrap();
    comp.into_inner().into_inner()
}

#[test]
fn empty_file_detects_as_empty() {
    let (f, d) = detect("x.txt", b"");
    assert_eq!(f, FormatId::Empty);
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn pdf_by_magic_ignores_extension() {
    let (f, d) = detect("x.txt", b"%PDF-1.7\n");
    assert_eq!(f, FormatId::Pdf);
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn docx_by_content_types() {
    let ct = content_types(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/embeddings/x.xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>"#,
    );
    let bytes = zip_of(&[
        ("[Content_Types].xml", &ct),
        ("word/document.xml", b"<w:document/>"),
    ]);
    let (f, d) = detect("x.zip", &bytes);
    assert_eq!(f, FormatId::Docx, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain);
    assert!(d.evidence.contains("wordprocessingml"), "{}", d.evidence);
}

#[test]
fn xlsx_by_content_types() {
    let ct = content_types(
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    let bytes = zip_of(&[
        ("[Content_Types].xml", &ct),
        ("xl/workbook.xml", b"<workbook/>"),
    ]);
    let (f, d) = detect("data.bin", &bytes);
    assert_eq!(f, FormatId::Xlsx, "{}", d.evidence);
    assert!(d.evidence.contains("spreadsheetml"), "{}", d.evidence);
}

#[test]
fn xlsm_by_content_types() {
    let ct = content_types(
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.ms-excel.sheet.macroEnabled.main+xml"/>"#,
    );
    let bytes = zip_of(&[("[Content_Types].xml", &ct), ("xl/workbook.xml", b"<w/>")]);
    assert_eq!(detect("a.xlsx", &bytes).0, FormatId::Xlsm);
}

#[test]
fn pptx_by_content_types() {
    let ct = content_types(
        r#"<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>"#,
    );
    let bytes = zip_of(&[
        ("[Content_Types].xml", &ct),
        ("ppt/presentation.xml", b"<p/>"),
    ]);
    assert_eq!(detect("a", &bytes).0, FormatId::Pptx);
}

#[test]
fn ooxml_without_content_type_match_falls_back_to_part_names() {
    let ct = content_types("");
    let bytes = zip_of(&[("[Content_Types].xml", &ct), ("xl/workbook.xml", b"<w/>")]);
    let (f, d) = detect("a", &bytes);
    assert_eq!(f, FormatId::Xlsx, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn odt_by_mimetype() {
    let (f, d) = detect("x.zip", &odf("application/vnd.oasis.opendocument.text"));
    assert_eq!(f, FormatId::Odt, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(
        detect("x", &odf("application/vnd.oasis.opendocument.spreadsheet")).0,
        FormatId::Ods
    );
    assert_eq!(
        detect("x", &odf("application/vnd.oasis.opendocument.presentation")).0,
        FormatId::Odp
    );
    assert_eq!(detect("x", &odf("application/epub+zip")).0, FormatId::Epub);
}

#[test]
fn three_mf_by_model_part() {
    let ct = content_types(
        r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
    );
    let bytes = zip_of(&[
        ("[Content_Types].xml", &ct),
        ("_rels/.rels", b"<Relationships/>"),
        ("3D/3dmodel.model", b"<model/>"),
    ]);
    let (f, d) = detect("x.zip", &bytes);
    assert_eq!(f, FormatId::ThreeMf, "{}", d.evidence);
}

#[test]
fn fcstd_by_document_xml() {
    let bytes = zip_of(&[
        ("Document.xml", b"<Document/>"),
        ("GuiDocument.xml", b"<x/>"),
    ]);
    assert_eq!(detect("x", &bytes).0, FormatId::Fcstd);
}

#[test]
fn plain_zip_is_zip() {
    let bytes = zip_of(&[("a.txt", b"hello")]);
    let (f, d) = detect("x.docx", &bytes);
    assert_eq!(f, FormatId::Zip);
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn broken_zip_is_zip_likely() {
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(&[0xAB; 200]);
    let (f, d) = detect("x.docx", &bytes);
    assert_eq!(f, FormatId::Zip);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn ole_word_by_stream_name() {
    let bytes = cfb_with(&["WordDocument", "1Table"]);
    let (f, d) = detect("x.bin", &bytes);
    assert_eq!(f, FormatId::Doc, "{}", d.evidence);
    assert!(d.evidence.contains("WordDocument"), "{}", d.evidence);
    assert_eq!(detect("x", &cfb_with(&["Workbook"])).0, FormatId::Xls);
    assert_eq!(detect("x", &cfb_with(&["Book"])).0, FormatId::Xls);
    assert_eq!(
        detect("x", &cfb_with(&["PowerPoint Document"])).0,
        FormatId::Ppt
    );
    assert_eq!(detect("x", &cfb_with(&["Other"])).0, FormatId::Ole);
}

#[test]
fn broken_ole_is_ole_likely() {
    let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    bytes.extend_from_slice(&[0xFF; 600]);
    let (f, d) = detect("x.doc", &bytes);
    assert_eq!(f, FormatId::Ole);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn csv_saved_as_txt_is_csv() {
    let (f, d) = detect("x.txt", b"a,b\n1,2\n3,4\n");
    assert_eq!(f, FormatId::Csv, "{}", d.evidence);
}

#[test]
fn tsv_by_tabs() {
    let (f, d) = detect("x.csv", b"name\tage\tcity\nann\t31\tBonn\nbob\t42\tKoeln\n");
    assert_eq!(f, FormatId::Tsv, "{}", d.evidence);
}

#[test]
fn csv_extension_breaks_tie_when_sniff_declines() {
    // Two records only: sniffing declines, the extension decides.
    let (f, d) = detect("x.csv", b"a,b\n1,2\n");
    assert_eq!(f, FormatId::Csv, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Guess);
    assert!(d.evidence.contains(".csv"), "{}", d.evidence);
    assert_eq!(detect("x.txt", b"a,b\n1,2\n").0, FormatId::Text);
}

#[test]
fn markdown_by_frontmatter() {
    let text = b"---\ntitle: \"Notes\"\ntags: [a]\n---\n\nSome text here.\n";
    let (f, d) = detect("notes.txt", text);
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    assert!(d.evidence.contains("frontmatter"), "{}", d.evidence);
}

#[test]
fn markdown_by_headings_and_lists() {
    let text = b"# Title\n\nSome prose.\n\n- one\n- two\n";
    assert_eq!(detect("x", text).0, FormatId::Markdown);
}

#[test]
fn dwg_version_detected() {
    let mut bytes = b"AC1032".to_vec();
    bytes.extend_from_slice(&[0u8; 128]);
    let (f, d) = detect("x.bin", &bytes);
    assert_eq!(f, FormatId::Dwg);
    assert!(d.evidence.contains("AC1032"), "{}", d.evidence);
}

#[test]
fn step_vs_ifc() {
    let step = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('x'),'2;1');\nFILE_NAME('a.stp','2020-01-01',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
    let ifc = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');\nFILE_NAME('a.ifc','2020-01-01',(''),(''),'','','');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
    let (f, d) = detect("a.ifc", step);
    assert_eq!(f, FormatId::Step, "{}", d.evidence);
    let (f, d) = detect("a.stp", ifc);
    assert_eq!(f, FormatId::Ifc, "{}", d.evidence);
    assert!(d.evidence.contains("IFC4"), "{}", d.evidence);
}

#[test]
fn iges_by_column_73() {
    let line1 = format!("{:<72}S{:>7}\r\n", "IGES file written by a test", 1);
    let line2 = format!("{:<72}G{:>7}\r\n", "1H,,1H;,", 1);
    let bytes = format!("{line1}{line2}");
    assert_eq!(detect("x.txt", bytes.as_bytes()).0, FormatId::Iges);
}

#[test]
fn other_magic_numbers() {
    assert_eq!(detect("x", b"glTF\x02\0\0\0").0, FormatId::Glb);
    assert_eq!(detect("x", b"GCDE\x01\0\0\0").0, FormatId::Bgcode);
    assert_eq!(detect("x", b"{\\rtf1\\ansi hello}").0, FormatId::Rtf);
    assert_eq!(
        detect("x", b"%!PS-Adobe-3.0\n%%Title: x\n").0,
        FormatId::PostScript
    );
    assert_eq!(detect("x", b"%!PS-Adobe-3.0 EPSF-3.0\n").0, FormatId::Eps);
    assert_eq!(detect("x", b"\xC5\xD0\xD3\xC6\x1e\0\0\0").0, FormatId::Eps);
}

#[test]
fn stl_ascii_and_binary() {
    let ascii = b"solid cube\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n    endloop\n  endfacet\nendsolid cube\n";
    assert_eq!(detect("x.txt", ascii).0, FormatId::Stl);
    // Binary STL whose header starts with "solid", as many exporters write it.
    let mut bin = vec![0u8; 80];
    bin[..5].copy_from_slice(b"solid");
    bin.extend_from_slice(&2u32.to_le_bytes());
    bin.extend_from_slice(&[0u8; 100]);
    assert_eq!(detect("x", &bin).0, FormatId::Stl);
}

#[test]
fn structured_text_formats() {
    assert_eq!(
        detect(
            "x",
            b"(kicad_sch (version 20230121) (generator eeschema))\n"
        )
        .0,
        FormatId::KicadSch
    );
    assert_eq!(
        detect("x", b"(kicad_pcb (version 20221018) (generator pcbnew))\n").0,
        FormatId::KicadPcb
    );
    assert_eq!(
        detect(
            "x",
            b"G04 made by a test*\n%FSLAX26Y26*%\n%MOMM*%\nD10*\nX0Y0D02*\nM02*\n"
        )
        .0,
        FormatId::Gerber
    );
    assert_eq!(
        detect(
            "x",
            b"; drill file\nM48\nMETRIC\nT1C0.8\n%\nT1\nX1000Y1000\nM30\n"
        )
        .0,
        FormatId::Excellon
    );
    assert_eq!(
        detect(
            "x",
            b"; generated\nG21\nG90\nM104 S200\nG1 X10 Y10 F3000\nG1 X20\nM2\n"
        )
        .0,
        FormatId::Gcode
    );
    assert_eq!(
        detect("x", b"module box(s) {\n  difference() {\n    cube(s);\n    sphere(r = 3);\n  }\n}\nbox(10);\n").0,
        FormatId::Openscad
    );
    assert_eq!(
        detect("x", b"<!DOCTYPE html>\n<html><body>Hi</body></html>\n").0,
        FormatId::Html
    );
    assert_eq!(
        detect("x", b"From: a@example.org\nTo: b@example.org\nDate: Mon, 1 Jan 2024 10:00:00 +0000\nSubject: hi\n\nbody\n").0,
        FormatId::Eml
    );
    assert_eq!(
        detect("x", b"From a@example.org Mon Jan  1 10:00:00 2024\nFrom: a@example.org\nDate: Mon, 1 Jan 2024 10:00:00 +0000\nSubject: hi\n\nbody\n").0,
        FormatId::Mbox
    );
}

#[test]
fn json_stays_text() {
    let (f, _) = detect(
        "x.csv",
        b"[\n  {\"a\": 1, \"b\": 2},\n  {\"a\": 3, \"b\": 4}\n]\n",
    );
    assert_eq!(f, FormatId::Text);
}

#[test]
fn plain_prose_is_text() {
    let (f, d) = detect(
        "x.bin",
        "Hello, world. This is plain text with Umlaute: \u{e4}\u{f6}\u{fc}.\n".as_bytes(),
    );
    assert_eq!(f, FormatId::Text, "{}", d.evidence);
}

#[test]
fn latin1_text_is_text() {
    let bytes = b"Gr\xfc\xdfe aus K\xf6ln, sch\xf6ne Stra\xdfe und \xc4rger.\n";
    assert_eq!(detect("x", bytes).0, FormatId::Text);
}

#[test]
fn utf16_text_is_text() {
    let mut bytes = vec![0xFF, 0xFE];
    for u in "Hello, world.\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    assert_eq!(detect("x", &bytes).0, FormatId::Text);
}

#[test]
fn binary_noise_is_unknown() {
    let bytes: Vec<u8> = (0..1024u32).map(|i| (i % 256) as u8).collect();
    let (f, _) = detect("x.txt", &bytes);
    assert_eq!(f, FormatId::Unknown);
}

#[test]
fn open_refuses_directory() {
    let dir = tempfile::tempdir().unwrap();
    let err = open(dir.path(), &Limits::default()).unwrap_err();
    assert!(matches!(err, OpenError::NotAFile(_)), "{err:?}");
}

#[test]
fn open_reports_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let err = open(&dir.path().join("nope.txt"), &Limits::default()).unwrap_err();
    assert!(matches!(err, OpenError::NotFound(_)), "{err:?}");
}

#[test]
fn open_refuses_too_large() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("five.txt");
    std::fs::write(&path, b"12345").unwrap();
    let limits = Limits {
        max_file_bytes: 4,
        ..Limits::default()
    };
    let err = open(&path, &limits).unwrap_err();
    assert!(
        matches!(err, OpenError::TooLarge { size: 5, max: 4 }),
        "{err:?}"
    );
    let err = from_bytes("five.txt", b"12345".to_vec(), &limits).unwrap_err();
    assert!(
        matches!(err, OpenError::TooLarge { size: 5, max: 4 }),
        "{err:?}"
    );
}

#[test]
fn open_reads_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.txt");
    std::fs::write(&path, b"%PDF-1.4\n").unwrap();
    let src = open(&path, &Limits::default()).unwrap();
    assert_eq!(src.name, "x.txt");
    assert_eq!(src.size, 9);
    assert_eq!(src.bytes(), b"%PDF-1.4\n");
    assert_eq!(src.format, FormatId::Pdf);
    let modified = src.modified.expect("modified time");
    assert_eq!(modified.len(), 20, "{modified}");
    assert!(modified.ends_with('Z'), "{modified}");
}

#[test]
fn open_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty");
    std::fs::write(&path, b"").unwrap();
    let src = open(&path, &Limits::default()).unwrap();
    assert_eq!(src.size, 0);
    assert_eq!(src.format, FormatId::Empty);
}

#[test]
fn sha256_is_hex_lowercase() {
    let src = from_bytes("abc.txt", b"abc".to_vec(), &Limits::default()).unwrap();
    assert_eq!(
        src.sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(src.size, 3);
    assert!(src.path.is_none());
}

#[test]
fn yaml_and_scripts_are_not_markdown() {
    let yaml = b"# settings\nname: demo\nitems:\n  - a\n  - b\n";
    assert_eq!(detect("x", yaml).0, FormatId::Text);
    let py = b"# Copyright\n# License: MIT\nimport os\n\n\nprint(os.getcwd())\n";
    assert_eq!(detect("x", py).0, FormatId::Text);
}

#[test]
fn truncated_sample_keeps_utf8() {
    // A two-byte character straddles the 1 MiB sniffing boundary.
    let mut bytes = vec![b'a'; 1024 * 1024 - 1];
    bytes.extend_from_slice("\u{e4} more text\n".as_bytes());
    let (f, d) = detect("x", &bytes);
    assert_eq!(f, FormatId::Text);
    assert!(d.evidence.contains("UTF-8"), "{}", d.evidence);
}

#[test]
fn prose_mentioning_pdf_header_is_text() {
    let text = b"The header of a PDF file is %PDF-1.7 on the first line.\n";
    assert_eq!(detect("x", text).0, FormatId::Text);
}

#[test]
fn eml_with_long_received_headers() {
    let mut mail = String::new();
    for i in 0..35 {
        mail.push_str(&format!(
            "Received: from relay{i}.example.org\n\tby mx.example.org; Mon, 1 Jan 2024\n"
        ));
    }
    mail.push_str("From: a@example.org\nTo: b@example.org\nDate: Mon, 1 Jan 2024 10:00:00 +0000\nSubject: hi\n\nbody\n");
    let (f, d) = detect("x", mail.as_bytes());
    assert_eq!(f, FormatId::Eml, "{}", d.evidence);
}

#[test]
fn gcode_with_long_thumbnail_comment_block() {
    let mut g = String::from("; generated by PrusaSlicer\n;\n; thumbnail begin 16x16 1000\n");
    for _ in 0..300 {
        g.push_str("; iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAA\n");
    }
    g.push_str("; thumbnail end\n\nG21\nG90\nM104 S200\nG1 X10 Y10 F3000\nG1 X20\n");
    let (f, d) = detect("x", g.as_bytes());
    assert_eq!(f, FormatId::Gcode, "{}", d.evidence);
}

#[test]
fn gltf_json_by_asset_version() {
    let gltf = br#"{
  "buffers": [{"byteLength": 4, "uri": "data:application/octet-stream;base64,AAAA"}],
  "asset": {"generator": "test", "version": "2.0"},
  "scenes": [{"nodes": [0]}]
}"#;
    let (f, d) = detect("model.json", gltf);
    assert_eq!(f, FormatId::Gltf, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    // "asset" nested deeper or without a version string is plain JSON.
    let nested = br#"{"meta": {"asset": {"version": "2.0"}}}"#;
    assert_eq!(detect("x.gltf", nested).0, FormatId::Text);
    let no_version = br#"{"asset": {"version": 2}}"#;
    assert_eq!(detect("x.gltf", no_version).0, FormatId::Text);
}

#[test]
fn markdown_by_extension_for_plain_readme() {
    let readme = b"# Title\n\nText.\n\n## Install\n\nRun it.\n";
    let (f, d) = detect("README.md", readme);
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Guess);
    assert!(d.evidence.contains(".md"), "{}", d.evidence);
    assert_eq!(
        detect("notes.markdown", b"Just prose.\n").0,
        FormatId::Markdown
    );
    assert_eq!(detect("README.txt", readme).0, FormatId::Text);
    // Another detected format wins over the extension.
    assert_eq!(detect("x.md", b"a,b,c\n1,2,3\n4,5,6\n").0, FormatId::Csv);
    // Not valid UTF-8: the extension alone does not make it Markdown.
    assert_eq!(
        detect(
            "x.md",
            b"Gr\xfc\xdfe aus K\xf6ln und sch\xf6ne Stra\xdfen.\n"
        )
        .0,
        FormatId::Text
    );
}

#[test]
fn open_small_file_is_owned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("small.txt");
    std::fs::write(&path, b"hello\n").unwrap();
    let src = open(&path, &Limits::default()).unwrap();
    assert!(
        matches!(src.bytes, rezzer_model::Bytes::Owned(_)),
        "{:?}",
        src.bytes
    );
    assert_eq!(src.bytes(), b"hello\n");
}

#[test]
fn pre_r13_dwg_versions() {
    for v in [
        "AC1.2", "AC1.40", "AC1.50", "AC2.10", "AC1001", "AC1002", "AC1003", "AC1004", "AC1005",
        "AC1006", "AC1007", "AC1008", "AC1009", "MC0.0",
    ] {
        let mut bytes = v.as_bytes().to_vec();
        bytes.extend_from_slice(&[0u8; 64]);
        let (f, d) = detect("x.bin", &bytes);
        assert_eq!(f, FormatId::Dwg, "{v}: {}", d.evidence);
        assert!(d.evidence.contains(v), "{v}: {}", d.evidence);
        assert!(d.evidence.contains("pre-R13"), "{v}: {}", d.evidence);
    }
    // Text that starts with the same characters stays text.
    assert_eq!(detect("x", b"AC1.2 is a part number.\n").0, FormatId::Text);
}
