// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::io::{Cursor, Write};

use rezzer_detect::{
    OpenError, detect, detect_with_hints, from_bytes, from_bytes_with_hints, open,
};
use rezzer_limits::Limits;
use rezzer_model::{Confidence, FormatId};
use zip::write::SimpleFileOptions;

/// The allocator of the Rezzer binary: the memory these tests measure is the memory it
/// ships with.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

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
fn confirmed_media_types_come_from_the_bytes_only() {
    let docx = zip_of(&[
        (
            "[Content_Types].xml",
            &content_types(
                r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
            ),
        ),
        ("word/document.xml", b"<w:document/>"),
    ]);
    let xlsm = zip_of(&[
        (
            "[Content_Types].xml",
            &content_types(
                r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.ms-excel.sheet.macroEnabled.main+xml"/>"#,
            ),
        ),
        ("xl/workbook.xml", b"<w/>"),
    ]);
    let cases: Vec<(&str, Vec<u8>, Option<&str>)> = vec![
        (
            "png",
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec(),
            Some("image/png"),
        ),
        (
            "jpeg",
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10],
            Some("image/jpeg"),
        ),
        ("gif", b"GIF89a\x01\0\x01\0".to_vec(), Some("image/gif")),
        (
            "webp",
            b"RIFF\x10\0\0\0WEBPVP8 ".to_vec(),
            Some("image/webp"),
        ),
        ("pdf", b"%PDF-1.7\n".to_vec(), Some("application/pdf")),
        (
            "docx",
            docx,
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        ),
        (
            "odt",
            odf("application/vnd.oasis.opendocument.text"),
            Some("application/vnd.oasis.opendocument.text"),
        ),
        // Text: a JSON object or array is JSON, other UTF-8 without NUL plain text.
        (
            "json",
            b" {\"a\": [1, 2]}\n".to_vec(),
            Some("application/json"),
        ),
        ("json array", b"[1, 2]".to_vec(), Some("application/json")),
        ("broken json", b"{\"a\": ".to_vec(), Some("text/plain")),
        ("a json number is text", b"42".to_vec(), Some("text/plain")),
        (
            "text",
            "just text named x.png, \u{e4}\n".as_bytes().to_vec(),
            Some("text/plain"),
        ),
        ("csv", b"a,b\n1,2\n".to_vec(), Some("text/plain")),
        ("empty", Vec::new(), Some("text/plain")),
        // A macro workbook, binary bytes and a name alone confirm nothing.
        ("xlsm", xlsm, None),
        ("text with nul", b"text\0more".to_vec(), None),
        ("latin-1", b"caf\xe9".to_vec(), None),
        (
            "riff without webp",
            b"RIFF\x10\0\0\0WAVEfmt ".to_vec(),
            None,
        ),
    ];
    for (what, bytes, want) in cases {
        assert_eq!(rezzer_detect::confirmed_media_type(&bytes), want, "{what}");
    }
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
fn encrypted_package_is_named_by_its_extension() {
    let bytes = cfb_with(&["EncryptionInfo", "EncryptedPackage"]);
    for (name, format) in [
        ("secret.xlsx", FormatId::Xlsx),
        ("Secret.XLSM", FormatId::Xlsm),
        ("secret.xlsb", FormatId::Xlsb),
    ] {
        let (f, d) = detect(name, &bytes);
        assert_eq!(f, format, "{name}: {}", d.evidence);
        assert_eq!(d.confidence, Confidence::Likely, "{name}");
        assert!(d.evidence.contains("EncryptedPackage"), "{}", d.evidence);
    }
    // Encrypted documents and decks use the same container: they stay OLE.
    for name in ["secret.docx", "secret.pptx", "secret"] {
        let (f, d) = detect(name, &bytes);
        assert_eq!(f, FormatId::Ole, "{name}");
        assert_eq!(d.confidence, Confidence::Certain, "{name}");
    }
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
fn csv_with_title_lines_is_csv() {
    let text = b"Station report 2024\nSource: test data\nid,name,value\n1,alpha,10\n2,beta,20\n3,gamma,30\n";
    let (f, d) = detect("x.txt", text);
    assert_eq!(f, FormatId::Csv, "{}", d.evidence);
}

#[test]
fn prose_and_lists_under_a_title_line_are_text() {
    let letter = b"Dear Anna\nThank you for the letter, it was nice\nThe kids are fine, school is ok\nSee you soon, Jordi\n";
    assert_eq!(detect("letter.txt", letter).0, FormatId::Text);
    let list = b"Shopping list\nmilk, eggs\nbread, butter\njam, tea\n";
    assert_eq!(detect("shop.txt", list).0, FormatId::Text);
    let three = b"Hello all\nWe met, talked, and left early\nAnna came, saw it, and smiled\nBen wrote, read, and slept\n";
    assert_eq!(detect("three.txt", three).0, FormatId::Text);
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
fn markdown_with_a_pipe_table_is_not_csv() {
    let text = b"# Results\n\nThe numbers below came from the last run of the suite.\n\n| name | value |\n| --- | :---: |\n| a | 1 |\n| b | 2 |\n";
    let (f, d) = detect("results.md", text);
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    // Heading and table without a file name hint.
    assert_eq!(
        detect("x", b"# T\n\n| a | b |\n|---|---|\n| 1 | 2 |\n").0,
        FormatId::Markdown
    );
    // Prose and table.
    let prose =
        b"Some words about the table that follows here.\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
    assert_eq!(detect("x.txt", prose).0, FormatId::Markdown);
    // The extension wins when only the table is there.
    assert_eq!(
        detect("t.md", b"| a | b |\n| --- | --- |\n| 1 | 2 |\n").0,
        FormatId::Markdown
    );
    // Pipe separated data without a delimiter row stays CSV.
    assert_eq!(
        detect("t.txt", b"a|b|c\n1|2|3\n4|5|6\n7|8|9\n").0,
        FormatId::Csv
    );
}

#[test]
fn pipe_table_evidence_names_what_decided() {
    let prose =
        b"Some words about the table that follows here.\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
    let (f, d) = detect("x.txt", prose);
    assert_eq!(f, FormatId::Markdown);
    assert!(d.evidence.contains("prose"), "{}", d.evidence);
    assert!(!d.evidence.contains("extension"), "{}", d.evidence);
    let (_, d) = detect("t.md", b"| a | b |\n| --- | --- |\n| 1 | 2 |\n");
    assert!(d.evidence.contains("extension"), "{}", d.evidence);
}

#[test]
fn markdown_file_starting_with_a_reference_definition_is_markdown() {
    let text = b"[foo]: https://example.org/x\n\nSome text with a [foo] link and [[foo]].\n";
    let (f, d) = detect("w.md", text);
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    assert_eq!(detect("w.markdown", text).0, FormatId::Markdown);
    assert_ne!(detect("w.dat", text).0, FormatId::Markdown);
}

#[test]
fn small_broken_csv_is_csv_by_extension() {
    let text = b"a,b,c\n1,2,3\n4,5\n6,7,8,9\n1,2,3\n4,5,6\n7,8,9\n";
    let (f, d) = detect("x.csv", text);
    assert_eq!(f, FormatId::Csv, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Guess);
    assert!(d.evidence.contains(".csv"), "{}", d.evidence);
    assert_ne!(detect("x.txt", text).0, FormatId::Csv);
    // Prose in a .csv file is still not CSV.
    let prose = b"Hello there, my friend.\nHow are you, today?\nFine.\nThanks, and you?\nWell, ok.\nGood.\nBye.\n";
    assert_ne!(detect("x.csv", prose).0, FormatId::Csv);
}

#[test]
fn very_small_ragged_csv_is_csv_by_extension() {
    for text in [
        &b"name,mass,count\nalpha,1.5,3\nbeta,2.5,4,9\ngamma,3.5,5\n"[..],
        &b"a,b\n1,2\n3,4,5\n"[..],
    ] {
        let (f, d) = detect("x.csv", text);
        assert_eq!(f, FormatId::Csv, "{}", d.evidence);
        assert_eq!(d.confidence, Confidence::Guess);
    }
    // Two lines are too few to tell a header from a sentence.
    assert_ne!(detect("x.csv", b"a,b\n1,2,3\n").0, FormatId::Csv);
    // Short prose with commas in a .csv file stays text.
    let prose = b"Hello there, my friend.\nHow are you, today, then?\nFine.\n";
    assert_ne!(detect("x.csv", prose).0, FormatId::Csv);
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
    assert!(is_rfc3339_utc(&modified), "{modified}");
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

// ---- Review fix round: container-first PDF, CAD formats, text order, bounded ZIP listing ----

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn cfb_tree(storages: &[&str], streams: &[&str]) -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    for s in storages {
        comp.create_storage(s).unwrap();
    }
    for name in streams {
        let mut s = comp.create_stream(name).unwrap();
        s.write_all(&[0u8; 600]).unwrap();
    }
    comp.flush().unwrap();
    comp.into_inner().into_inner()
}

/// A ZIP of `n` empty stored entries, written by hand so it stays fast in debug builds.
/// Above 65,535 entries it writes the ZIP64 end records, as conforming writers do, unless
/// `zip64` is false.
fn raw_zip_empty_entries(n: u32) -> Vec<u8> {
    raw_zip(n, n > 0xFFFF)
}

fn raw_zip(n: u32, zip64: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for i in 0..n {
        let name = format!("{i:07}");
        let offset = out.len() as u32;
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method, time, date
        out.extend_from_slice(&[0u8; 12]); // crc, sizes
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&[0u8; 12]);
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 12]); // extra, comment, disk, attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    if zip64 {
        let z64 = out.len() as u64;
        out.extend_from_slice(b"PK\x06\x06");
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&u64::from(n).to_le_bytes());
        out.extend_from_slice(&u64::from(n).to_le_bytes());
        out.extend_from_slice(&u64::from(cd_size).to_le_bytes());
        out.extend_from_slice(&u64::from(cd_offset).to_le_bytes());
        out.extend_from_slice(b"PK\x06\x07");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&z64.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0u8; 4]);
    let count = if zip64 {
        0xFFFF
    } else {
        u16::try_from(n).unwrap_or(0xFFFF)
    };
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let (size_field, offset_field) = if zip64 {
        (0xFFFF_FFFF, 0xFFFF_FFFF)
    } else {
        (cd_size, cd_offset)
    };
    out.extend_from_slice(&size_field.to_le_bytes());
    out.extend_from_slice(&offset_field.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[test]
fn pdf_stored_in_zip_is_zip() {
    // The entry name ends in a line feed, so "\n%PDF-" appears right after the local header.
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("a.pdf\n", stored).unwrap();
    w.write_all(b"%PDF-1.4\n%%EOF\n").unwrap();
    let bytes = w.finish().unwrap().into_inner();
    let (f, d) = detect("pdfs.zip", &bytes);
    assert_eq!(f, FormatId::Zip, "{}", d.evidence);
}

#[test]
fn prose_quoting_pdf_header_on_its_own_line_is_text() {
    let text = b"Notes on the format, see the reference:\n%PDF-1.7\nThe header comes first.\n";
    assert_eq!(detect("notes.txt", text).0, FormatId::Text);
}

#[test]
fn pdf_after_binary_junk_is_pdf_likely() {
    let mut bytes = vec![0x00, 0x01, 0x02, 0xFF, 0x10];
    bytes.extend_from_slice(b"%PDF-1.4\n1 0 obj\n");
    let (f, d) = detect("x", &bytes);
    assert_eq!(f, FormatId::Pdf, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn markdown_about_openscad_is_markdown() {
    let post = b"---\ntitle: \"Printing a box\"\n---\n\nA short post.\n\n    cube(10); translate([2,0,0]) sphere(3);\n\nThat is all.\n";
    assert_eq!(detect("post.md", post).0, FormatId::Markdown);
    let readme = b"# Box\n\nSome prose about the model.\n\n```openscad\nmodule box(s) { cube(s); }\ntranslate([1,0,0]) sphere(2);\n```\n\n## Usage\n\nRender it.\n";
    assert_eq!(detect("readme.md", readme).0, FormatId::Markdown);
    assert_eq!(detect("readme.txt", readme).0, FormatId::Markdown);
}

#[test]
fn python_with_primitive_names_is_not_openscad() {
    let py = b"import turtle\n\ndef square(t, n):\n    for _ in range(4):\n        t.forward(n)\n\ndef circle(t, r):\n    t.circle(r)\n\nsquare(turtle.Turtle(), 10)\ncircle(turtle.Turtle(), 5)\ncube(3)\nsphere(2)\n";
    assert_eq!(detect("turtle.py", py).0, FormatId::Text);
}

#[test]
fn corpus_openscad_files() {
    for name in [
        "bosl2_std.scad",
        "bosl2_version.scad",
        "openscad_echo.scad",
        "openscad_recursion.scad",
        "openscad_list_comprehensions.scad",
    ] {
        let (f, d) = detect("x.txt", &fixture(name));
        assert_eq!(f, FormatId::Openscad, "{name}: {}", d.evidence);
    }
}

#[test]
fn gcode_with_line_numbers() {
    let (f, d) = detect("x.txt", &fixture("syn_cnc_mill.nc"));
    assert_eq!(f, FormatId::Gcode, "{}", d.evidence);
    let cnc = b"%\nO1000\nN10 G21 G90\nN20 T1 M6\nN30 S8000 M3\nN40 G0 X0 Y0\nN50 G1 Z-1 F100\nN60 M30\n%\n";
    assert_eq!(detect("cnc.nc", cnc).0, FormatId::Gcode);
}

#[test]
fn dxf_ascii_and_binary() {
    let ascii = b"  0\nSECTION\n  2\nHEADER\n  9\n$ACADVER\n  1\nAC1009\n  0\nENDSEC\n  0\nEOF\n";
    let (f, d) = detect("x.txt", ascii);
    assert_eq!(f, FormatId::Dxf, "{}", d.evidence);
    let crlf =
        b"  0\r\nSECTION\r\n  2\r\nENTITIES\r\n  0\r\nLINE\r\n  0\r\nENDSEC\r\n  0\r\nEOF\r\n";
    assert_eq!(detect("x", crlf).0, FormatId::Dxf);
    let comment = b"999\ndxfrw 0.6.3\n  0\nSECTION\n  2\nHEADER\n  0\nENDSEC\n";
    assert_eq!(detect("x", comment).0, FormatId::Dxf);
    let mut binary = b"AutoCAD Binary DXF\r\n\x1a\x00".to_vec();
    binary.extend_from_slice(b"\x00\x00SECTION\x00\x02HEADER\x00");
    let (f, d) = detect("x", &binary);
    assert_eq!(f, FormatId::Dxf, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn obj_by_keywords() {
    let obj = b"# cube\nmtllib cube.mtl\no Cube\nv 1.0 1.0 -1.0\nv 1.0 -1.0 -1.0\nv 1.0 1.0 1.0\nvn 0 1 0\nusemtl Material\ns off\nf 1//1 2//1 3//1\n";
    let (f, d) = detect("x.txt", obj);
    assert_eq!(f, FormatId::Obj, "{}", d.evidence);
    assert_eq!(
        detect("x", b"v is a letter.\nf is another.\nSo is o.\n").0,
        FormatId::Text
    );
}

#[test]
fn rhino_and_kicad_legacy() {
    let mut rhino = b"3D Geometry File Format       60".to_vec();
    rhino.extend_from_slice(&[1, 0, 0, 0, 0xa5, 1, 0, 0]);
    assert_eq!(detect("x", &rhino).0, FormatId::Rhino3dm);
    assert_eq!(
        detect(
            "x",
            b"EESchema Schematic File Version 4\nEELAYER 30 0\nEELAYER END\n"
        )
        .0,
        FormatId::KicadSch
    );
    assert_eq!(
        detect("x", b"PCBNEW-BOARD Version 1 date 2019-01-01\n\n$GENERAL\n").0,
        FormatId::KicadPcb
    );
}

#[test]
fn mcad_native_by_magic() {
    let mut catia = b"V5_CFV2\x00".to_vec();
    catia.extend_from_slice(&[0, 0x10, 0x67, 0xd7, 0, 0, 0x38, 0x96]);
    catia.extend_from_slice(&[0xff; 8]);
    let (f, d) = detect("x", &catia);
    assert_eq!(f, FormatId::Catpart, "{}", d.evidence);

    let mut nx = b"SPLMSSTR\x06\x3d\x21\x01\x00\x00\x00\x00\x00".to_vec();
    nx.extend_from_slice(&[0x2b, 0xdd, 0x0e, 0, 0, 0, 0, 0]);
    nx.extend_from_slice(b"\x00HEADER\x0f\x00\x00\x00");
    assert_eq!(detect("x", &nx).0, FormatId::NxPrt);

    let creo = b"#UGC:2 PART 2149 1860 800 1 1 15 4100 2023294 00\n#-END_OF_UGC_HEADER\n";
    assert_eq!(detect("x.prt", creo).0, FormatId::CreoPrt);

    // SolidWorks block envelope: file id, version 4 (big-endian), then a block marker.
    let mut sw = vec![0x4a, 0xcb, 0xa7, 0x2b, 0, 0, 0, 4];
    sw.extend_from_slice(&[0xae; 12]);
    sw.extend_from_slice(&[0x14, 0, 0x06, 0, 0x08, 0]);
    sw.extend_from_slice(&[0xdf; 40]);
    let (f, d) = detect("x", &sw);
    assert_eq!(f, FormatId::Sldprt, "{}", d.evidence);
}

#[test]
fn mcad_native_by_ole_streams() {
    let inv = cfb_tree(&["/RSeStorage"], &["/RSeStorage/RSeSegInfo"]);
    let (f, d) = detect("x", &inv);
    assert_eq!(f, FormatId::Ipt, "{}", d.evidence);
    let inv_db = cfb_tree(
        &["/RSeStorage", "/RSeStorage/V3"],
        &["/RSeStorage/V3/RSeDb"],
    );
    assert_eq!(detect("x", &inv_db).0, FormatId::Ipt);
    let nx = cfb_tree(&["/UG_PART"], &["/UG_PART/UG_PART"]);
    assert_eq!(detect("x", &nx).0, FormatId::NxPrt);
    let sw = cfb_tree(&[], &["/ISolidWorksInformation"]);
    assert_eq!(detect("x", &sw).0, FormatId::Sldprt);
}

#[test]
fn f3d_by_manifest() {
    let mut manifest = Vec::new();
    manifest.extend_from_slice(&7u32.to_le_bytes());
    manifest.extend_from_slice(b"3-2-0-0");
    manifest.extend_from_slice(&13u32.to_le_bytes());
    manifest.extend_from_slice(b"FusionDocType");
    let bytes = zip_of(&[
        (
            "FusionAssetName[Active]/Breps.BlobParts/BREP.0.smb",
            b"\x00",
        ),
        ("Manifest.dat", &manifest),
        ("Properties.dat", b"\x00"),
    ]);
    let (f, d) = detect("x.zip", &bytes);
    assert_eq!(f, FormatId::F3d, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn zip_with_too_many_entries_is_not_listed() {
    let bytes = raw_zip_empty_entries(200_000);
    let start = std::time::Instant::now();
    let (f, d) = detect("many.zip", &bytes);
    assert_eq!(f, FormatId::Zip, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(d.evidence.contains("too large to list"), "{}", d.evidence);
    assert!(start.elapsed().as_millis() < 500, "{:?}", start.elapsed());
    // Without the ZIP64 records the saturated 16-bit count is replaced by an estimate from
    // the directory size.
    let (f, d) = detect("many.zip", &raw_zip(200_000, false));
    assert_eq!(f, FormatId::Zip);
    assert!(d.evidence.contains("too large to list"), "{}", d.evidence);
    // A small hand-written archive still lists.
    assert_eq!(
        detect("few.zip", &raw_zip_empty_entries(3)).1.confidence,
        Confidence::Certain
    );
}

#[test]
fn zip64_directory_claim_is_checked() {
    // Local header, then a ZIP64 end record claiming 10^9 entries, its locator and the
    // classic end record with the ZIP64 markers.
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(&[0u8; 26]);
    let z64_offset = bytes.len() as u64;
    bytes.extend_from_slice(b"PK\x06\x06");
    bytes.extend_from_slice(&44u64.to_le_bytes());
    bytes.extend_from_slice(&[45, 0, 45, 0]);
    bytes.extend_from_slice(&[0u8; 8]); // disk numbers
    bytes.extend_from_slice(&1_000_000_000u64.to_le_bytes());
    bytes.extend_from_slice(&1_000_000_000u64.to_le_bytes());
    bytes.extend_from_slice(&46_000_000_000u64.to_le_bytes());
    bytes.extend_from_slice(&30u64.to_le_bytes());
    bytes.extend_from_slice(b"PK\x06\x07");
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&z64_offset.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(b"PK\x05\x06");
    bytes.extend_from_slice(&[0u8; 4]);
    bytes.extend_from_slice(&[0xFF; 4]);
    bytes.extend_from_slice(&[0xFF; 8]);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    let (f, d) = detect("x.zip", &bytes);
    assert_eq!(f, FormatId::Zip);
    assert_eq!(d.confidence, Confidence::Likely);
    // The claim does not fit the file, so the directory is not even located.
    assert!(
        d.evidence.contains("too large to list") || d.evidence.contains("not readable"),
        "{}",
        d.evidence
    );
}

#[test]
fn content_types_bomb_is_not_decompressed() {
    let mut ct = content_types(
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    ct.extend(std::iter::repeat_n(b' ', 2 * 1024 * 1024));
    let bytes = zip_of(&[("[Content_Types].xml", &ct), ("xl/workbook.xml", b"<w/>")]);
    let (f, d) = detect("x", &bytes);
    assert_eq!(f, FormatId::Xlsx, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(d.evidence.contains("unreadable"), "{}", d.evidence);
}

#[test]
fn container_names_match_case_insensitively() {
    let ct = content_types(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    );
    let bytes = zip_of(&[("[content_types].xml", &ct), ("word/document.xml", b"<w/>")]);
    let (f, d) = detect("x", &bytes);
    assert_eq!(f, FormatId::Docx, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain);
    assert_eq!(
        detect("x", &zip_of(&[("document.xml", b"<D/>")])).0,
        FormatId::Fcstd
    );
}

#[test]
fn multi_document_yaml_is_not_markdown() {
    let yaml = b"---\nname: a\nvalue: 1\n---\nname: b\nvalue: 2\n";
    assert_eq!(detect("x.yaml", yaml).0, FormatId::Text);
}

#[test]
fn excellon_without_m48() {
    let allegro = b";LEADER: 12\n;HEADER:\n;CODE  : ASCII\n;T01 Holesize 1. = 8.000000 PLATED MILS\n%\nG90\nT01\nX00130500Y00184500\nX00133000Y00195500\nM30\n";
    let (f, d) = detect("x.drl", allegro);
    assert_eq!(f, FormatId::Excellon, "{}", d.evidence);
    let metric = b"METRIC,LZ\nT1C0.800\nT2C1.000\n%\nT1\nX1000Y1000\nM30\n";
    assert_eq!(detect("x", metric).0, FormatId::Excellon);
}

/// Sweep over the local spike corpus (not in git). Run with `--ignored` when it exists.
#[test]
#[ignore = "needs the local spike corpus"]
fn spike_corpus_sweep() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spike/corpus");
    let expect = |ext: &str| match ext {
        "dxf" => Some(FormatId::Dxf),
        "3dm" => Some(FormatId::Rhino3dm),
        "catpart" => Some(FormatId::Catpart),
        "sldprt" => Some(FormatId::Sldprt),
        "ipt" => Some(FormatId::Ipt),
        "iam" => Some(FormatId::Iam),
        "f3d" => Some(FormatId::F3d),
        "obj" => Some(FormatId::Obj),
        "scad" => Some(FormatId::Openscad),
        "nc" | "gcode" => Some(FormatId::Gcode),
        "drl" | "xln" => Some(FormatId::Excellon),
        "gbr" | "gtl" | "gbl" | "gm1" => Some(FormatId::Gerber),
        "sch" | "kicad_sch" => Some(FormatId::KicadSch),
        "kicad_pcb" => Some(FormatId::KicadPcb),
        "stl" => Some(FormatId::Stl),
        "stp" | "step" => Some(FormatId::Step),
        "igs" | "iges" => Some(FormatId::Iges),
        "ifc" => Some(FormatId::Ifc),
        "3mf" => Some(FormatId::ThreeMf),
        "fcstd" => Some(FormatId::Fcstd),
        "dwg" => Some(FormatId::Dwg),
        "glb" => Some(FormatId::Glb),
        "gltf" => Some(FormatId::Gltf),
        "bgcode" => Some(FormatId::Bgcode),
        "docx" => Some(FormatId::Docx),
        "xlsx" => Some(FormatId::Xlsx),
        "xlsm" => Some(FormatId::Xlsm),
        "pptx" => Some(FormatId::Pptx),
        "odt" => Some(FormatId::Odt),
        "ods" => Some(FormatId::Ods),
        "odp" => Some(FormatId::Odp),
        "epub" => Some(FormatId::Epub),
        "pdf" => Some(FormatId::Pdf),
        _ => None,
    };
    let mut bad = Vec::new();
    let mut checked = 0;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase());
            let lower = name.to_ascii_lowercase();
            let want = if lower.contains(".prt") && lower.contains("_nx") {
                Some(FormatId::NxPrt)
            } else if lower.contains(".prt.") {
                Some(FormatId::CreoPrt)
            } else {
                ext.as_deref().and_then(expect)
            };
            let Some(want) = want else {
                continue;
            };
            let shown = p.to_string_lossy();
            // Extensions shared with other formats (Eagle .sch, NetCDF .nc) are only
            // expected under cad/; malformed samples are expected to fail.
            let shared = matches!(ext.as_deref(), Some("sch" | "nc")) && !shown.contains("/cad/");
            if shared || shown.contains("/malformed/") {
                continue;
            }
            if name.contains("synthetic") || shown.contains("/synthetic/") {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap();
            let (f, d) = detect(&name, &bytes);
            checked += 1;
            if matches!(f, FormatId::Ipt | FormatId::Iam) {
                eprintln!("{name}: {f:?} ({})", d.evidence);
            }
            if f != want {
                bad.push(format!("{}: {f:?} ({})", p.display(), d.evidence));
            }
        }
    }
    assert!(checked > 0, "no corpus files found");
    eprintln!("checked {checked} corpus files");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

// ---- Fix round 3: non-blocking open, OLE cap, stricter Excellon, Inventor assemblies ----

#[test]
fn cnc_program_with_g90_and_bare_coordinates_is_gcode() {
    let cnc = b"%\nG90\nG21\nX10.0 Y10.0\nX20.0 Y20.0\nG1 X30 F200\nM30\n%\n";
    let (f, d) = detect("mill.nc", cnc);
    assert_eq!(f, FormatId::Gcode, "{}", d.evidence);
}

#[cfg(unix)]
#[test]
fn open_refuses_fifo_without_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipe");
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: plain libc call with a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.clone();
    std::thread::spawn(move || {
        let _ = tx.send(open(&p, &Limits::default()));
    });
    let res = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("open blocked on a FIFO");
    assert!(matches!(res, Err(OpenError::NotAFile(_))), "{res:?}");
}

/// A compound file whose directory chain runs over `dir_sectors` sectors (512-byte sectors).
/// Only the header and the FAT are real; the directory sectors are zeros.
fn cfb_with_long_directory(dir_sectors: u32) -> Vec<u8> {
    let ss = 512usize;
    let per_fat = (ss / 4) as u32;
    let total = dir_sectors + dir_sectors.div_ceil(per_fat) + 1;
    let fat_sectors = total.div_ceil(per_fat);
    assert!(fat_sectors <= 109);
    let mut out = vec![0u8; ss];
    out[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    out[0x18..0x1A].copy_from_slice(&0x3Eu16.to_le_bytes());
    out[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
    out[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
    out[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
    out[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
    out[0x2C..0x30].copy_from_slice(&fat_sectors.to_le_bytes());
    out[0x30..0x34].copy_from_slice(&fat_sectors.to_le_bytes()); // directory starts after FAT
    out[0x38..0x3C].copy_from_slice(&4096u32.to_le_bytes());
    out[0x3C..0x40].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
    out[0x44..0x48].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
    for i in 0..109u32 {
        let v = if i < fat_sectors { i } else { 0xFFFF_FFFF };
        let at = 0x4C + 4 * i as usize;
        out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut fat = vec![0xFFFF_FFFFu32; (fat_sectors * per_fat) as usize];
    for i in 0..fat_sectors {
        fat[i as usize] = 0xFFFF_FFFD; // FAT sector
    }
    let first = fat_sectors;
    for k in 0..dir_sectors {
        let id = first + k;
        fat[id as usize] = if k + 1 < dir_sectors {
            id + 1
        } else {
            0xFFFF_FFFE
        };
    }
    for v in &fat {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.resize(ss * (1 + (first + dir_sectors) as usize), 0);
    out
}

#[test]
fn ole_with_huge_directory_is_not_listed() {
    // 2,600 sectors of 4 entries each: 10,400 directory entries.
    let bytes = cfb_with_long_directory(2_600);
    let (f, d) = detect("x.doc", &bytes);
    assert_eq!(f, FormatId::Ole, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(
        d.evidence.contains("ole directory too large to list"),
        "{}",
        d.evidence
    );
}

fn counted_utf16(s: &str) -> Vec<u8> {
    let mut out = (s.encode_utf16().count() as u32).to_le_bytes().to_vec();
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn cfb_inventor(type_names: &[&str]) -> Vec<u8> {
    let mut info = 2u32.to_le_bytes().to_vec();
    for t in type_names {
        info.extend(counted_utf16("Segment"));
        info.extend(counted_utf16(t));
        info.extend_from_slice(&[0u8; 40]);
    }
    let mut comp = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    comp.create_storage("/RSeStorage").unwrap();
    comp.create_stream("/RSeStorage/RSeSegInfo")
        .unwrap()
        .write_all(&info)
        .unwrap();
    comp.flush().unwrap();
    comp.into_inner().into_inner()
}

#[test]
fn inventor_assembly_vs_part() {
    let (f, d) = detect("x", &cfb_inventor(&["AmDc", "AmGraphics"]));
    assert_eq!(f, FormatId::Iam, "{}", d.evidence);
    let (f, d) = detect("x", &cfb_inventor(&["PmDc", "PmBRep"]));
    assert_eq!(f, FormatId::Ipt, "{}", d.evidence);
    assert!(d.evidence.contains("part"), "{}", d.evidence);
}

// ---- Fix round 4: own bounded ZIP directory walk, stricter binary PDF prefix ----

/// Appends a decoy end record: one entry, a 46-byte directory at an offset past the file.
fn with_decoy_eocd(mut zip: Vec<u8>) -> Vec<u8> {
    zip.extend_from_slice(b"PK\x05\x06");
    for v in [0u16, 0, 1, 1] {
        zip.extend_from_slice(&v.to_le_bytes());
    }
    zip.extend_from_slice(&46u32.to_le_bytes());
    zip.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip
}

/// Inserts 4 junk bytes before the ZIP64 end record (so the locator no longer points at it),
/// lowers its directory offset by 4 and sets the classic directory size to 1000.
fn with_shifted_zip64(zip: Vec<u8>) -> Vec<u8> {
    let n = zip.len();
    let (body, tail) = zip.split_at(n - 98);
    let (e64, rest) = tail.split_at(56);
    let (loc, eocd) = rest.split_at(20);
    assert_eq!(&e64[..4], b"PK\x06\x06");
    assert_eq!(&loc[..4], b"PK\x06\x07");
    assert_eq!(&eocd[..4], b"PK\x05\x06");
    let cd_off = u64::from_le_bytes(e64[48..56].try_into().unwrap());
    let mut out = body.to_vec();
    out.extend_from_slice(b"JUNK");
    out.extend_from_slice(&e64[..48]);
    out.extend_from_slice(&(cd_off - 4).to_le_bytes());
    out.extend_from_slice(loc);
    out.extend_from_slice(&eocd[..12]);
    out.extend_from_slice(&1000u32.to_le_bytes());
    out.extend_from_slice(&eocd[16..]);
    out
}

#[test]
fn latin1_prose_quoting_pdf_header_is_text() {
    let bytes = b"Caf\xe9 notes, see header:\n%PDF-1.7\nend\n";
    let (f, d) = detect("latin1.txt", bytes);
    assert_eq!(f, FormatId::Text, "{}", d.evidence);
}

#[test]
fn decoy_end_record_is_skipped() {
    // Small archive: the decoy is skipped and the real directory is listed.
    let ct = content_types(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    );
    let docx = zip_of(&[("[Content_Types].xml", &ct), ("word/document.xml", b"<w/>")]);
    let (f, d) = detect("x", &with_decoy_eocd(docx));
    assert_eq!(f, FormatId::Docx, "{}", d.evidence);
    // Large archive: the decoy does not hide the real entry count.
    let (f, d) = detect(
        "decoy.zip",
        &with_decoy_eocd(raw_zip_empty_entries(200_000)),
    );
    assert_eq!(f, FormatId::Zip);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(d.evidence.contains("too large to list"), "{}", d.evidence);
}

#[test]
fn shifted_zip64_end_record_is_found() {
    let (f, d) = detect("x.zip", &with_shifted_zip64(raw_zip(3, true)));
    assert_eq!(f, FormatId::Zip, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain, "{}", d.evidence);
    assert!(d.evidence.contains("3 entries"), "{}", d.evidence);
    let (f, d) = detect(
        "locshift.zip",
        &with_shifted_zip64(raw_zip_empty_entries(200_000)),
    );
    assert_eq!(f, FormatId::Zip);
    assert!(d.evidence.contains("too large to list"), "{}", d.evidence);
}

#[test]
fn zip_without_consistent_end_record_is_not_readable() {
    let mut bytes = zip_of(&[("a.txt", b"hello")]);
    let n = bytes.len();
    // Point the directory past the end of the file.
    bytes[n - 6..n - 2].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    let (f, d) = detect("x.zip", &bytes);
    assert_eq!(f, FormatId::Zip);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(
        d.evidence.contains("the zip directory is not readable"),
        "{}",
        d.evidence
    );
}

#[test]
fn frontmatter_with_key_like_first_body_line() {
    let post = b"---\ntitle: Release\n---\nUpdate: the new version is out.\n\nIt fixes two bugs.\n";
    let (f, d) = detect("post2.txt", post);
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
}

#[test]
fn yaml_with_list_document_is_not_markdown() {
    assert_eq!(
        detect("x.yaml", b"---\nname: a\n---\n- x\n- y\n").0,
        FormatId::Text
    );
}

/// Memory check for 1,000,000-entry archives (about 90 MB each), with a decoy end record
/// and with a moved ZIP64 record. Run by hand:
/// `cargo test -p rezzer-detect --release --test detect zip_million_entries_memory -- --ignored --nocapture`
/// With `REZZER_ZIP_PROBES=<dir>` the files `decoy.zip` and `locshift.zip` from that directory
/// are checked too.
#[cfg(unix)]
#[test]
#[ignore = "builds 90 MB archives; run by hand for the memory check"]
fn zip_million_entries_memory() {
    let mut inputs = vec![
        (
            "locshift (built)",
            with_shifted_zip64(raw_zip_empty_entries(1_000_000)),
        ),
        (
            "decoy (built)",
            with_decoy_eocd(raw_zip_empty_entries(1_000_000)),
        ),
    ];
    if let Ok(dir) = std::env::var("REZZER_ZIP_PROBES") {
        for name in ["decoy.zip", "locshift.zip"] {
            let path = std::path::Path::new(&dir).join(name);
            inputs.push((name, std::fs::read(&path).unwrap()));
        }
    }
    let before = max_rss_bytes();
    for (label, bytes) in &inputs {
        let start = std::time::Instant::now();
        let (f, d) = detect(label, bytes);
        eprintln!(
            "{label}: {} bytes, {:?}, {f:?} ({})",
            bytes.len(),
            start.elapsed(),
            d.evidence
        );
        assert_eq!(f, FormatId::Zip);
        assert!(d.evidence.contains("too large to list"), "{}", d.evidence);
    }
    let after = max_rss_bytes();
    eprintln!(
        "max RSS with all inputs in memory: {} MB before detection, {} MB after",
        before / 1_000_000,
        after / 1_000_000
    );
}

#[cfg(unix)]
fn max_rss_bytes() -> u64 {
    // SAFETY: getrusage fills the struct we pass.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    }
}

// ---- Re-review 2: bounded stored markers, zip64 shift order, markdown after frontmatter ----

/// A ZIP of stored entries written by hand: `(name, data, comment)`. `between` is put
/// between the central directory and the ZIP64 records (if `zip64`) or the end record.
/// The end record is never saturated.
fn stored_zip(entries: &[(&str, &[u8], &[u8])], zip64: bool, between: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, comment) in entries {
        let offset = out.len() as u32;
        let len = (data.len() as u32).to_le_bytes();
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&[0u8; 4]); // crc
        out.extend_from_slice(&len);
        out.extend_from_slice(&len);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&[0u8; 4]); // crc
        central.extend_from_slice(&len);
        central.extend_from_slice(&len);
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // disk, attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        central.extend_from_slice(comment);
    }
    let n = entries.len() as u16;
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(between);
    if zip64 {
        let z64 = out.len() as u64;
        out.extend_from_slice(b"PK\x06\x06");
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&u64::from(n).to_le_bytes());
        out.extend_from_slice(&u64::from(n).to_le_bytes());
        out.extend_from_slice(&u64::from(cd_size).to_le_bytes());
        out.extend_from_slice(&u64::from(cd_offset).to_le_bytes());
        out.extend_from_slice(b"PK\x06\x07");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&z64.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Three stored docx entries whose first central header is exactly 76 bytes long, the size
/// of a ZIP64 end record plus locator.
fn docx_with_76_byte_first_header(zip64: bool, between: &[u8]) -> Vec<u8> {
    let ct = content_types(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    );
    let comment = b"eleven byte";
    assert_eq!(46 + "[Content_Types].xml".len() + comment.len(), 76);
    stored_zip(
        &[
            ("[Content_Types].xml", &ct, comment),
            ("word/document.xml", b"<w/>", b""),
            ("_rels/.rels", b"<r/>", b""),
        ],
        zip64,
        between,
    )
}

#[test]
fn zip64_records_with_unsaturated_end_record_and_76_byte_header() {
    let (f, d) = detect("gap76.docx", &docx_with_76_byte_first_header(true, b""));
    assert_eq!(f, FormatId::Docx, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain, "{}", d.evidence);
}

#[test]
fn zero_shift_is_tried_before_the_gap() {
    // No ZIP64 records, but 76 bytes of padding between the directory and the end record.
    let (f, d) = detect("x", &docx_with_76_byte_first_header(false, &[0u8; 76]));
    assert_eq!(f, FormatId::Docx, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain, "{}", d.evidence);
}

#[test]
fn walk_stops_at_the_declared_directory_end() {
    let mut bytes = raw_zip(3, false);
    let n = bytes.len();
    // Each header is 53 bytes; 60 covers one and a part, not three, and the gap shift does
    // not land on a header.
    bytes[n - 10..n - 6].copy_from_slice(&60u32.to_le_bytes());
    let (f, d) = detect("x.zip", &bytes);
    assert_eq!(f, FormatId::Zip);
    assert_eq!(d.confidence, Confidence::Likely, "{}", d.evidence);
    assert!(
        d.evidence.contains("directory ends before entry 3"),
        "{}",
        d.evidence
    );
}

#[test]
fn stored_marker_with_huge_declared_size_is_not_read() {
    // A stored mimetype whose central header claims a compressed size that spans the rest
    // of the file (storedbig). The marker is unreadable, the archive is a plain zip.
    let mime = b"application/vnd.oasis.opendocument.text";
    let mut bytes = stored_zip(
        &[("mimetype", mime, b""), ("pad.bin", &[7u8; 200_000], b"")],
        false,
        b"",
    );
    let cd = bytes.len() - 22 - 2 * 46 - "mimetype".len() - "pad.bin".len();
    assert_eq!(&bytes[cd..cd + 4], b"PK\x01\x02");
    let data = 30 + "mimetype".len();
    bytes[cd + 20..cd + 24].copy_from_slice(&((cd - data) as u32).to_le_bytes());
    let (f, d) = detect("storedbig.zip", &bytes);
    assert_eq!(f, FormatId::Zip, "{}", d.evidence);
    assert!(d.evidence.contains("2 entries"), "{}", d.evidence);
}

#[test]
fn frontmatter_then_headings_and_lists_is_markdown() {
    let note = b"---\ntitle: Notes\ntags: [a]\n---\n# Shopping\n\n- milk\n- bread\n";
    let (f, d) = detect("fmlist.txt", note);
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    let (f, d) = detect("fmhead.txt", b"---\ntitle: Notes\n---\n# Shopping\n");
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    // A comment in a later YAML document, next to keys, stays YAML.
    let yaml = b"---\nname: a\n---\n# second\nname: b\n";
    assert_eq!(detect("x.yaml", yaml).0, FormatId::Text);
}

/// A binary stub (`MZ` or ELF) of 5,000 bytes in front of a docx.
fn sfx(stub_magic: &[u8], zip: &[u8]) -> Vec<u8> {
    let mut out = stub_magic.to_vec();
    out.extend((0..5_000 - stub_magic.len()).map(|i| (i % 7) as u8));
    out.extend_from_slice(zip);
    out
}

#[test]
fn zip_after_a_stub_is_zip_likely() {
    let ct = content_types(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    );
    let docx = zip_of(&[("[Content_Types].xml", &ct), ("word/document.xml", b"<w/>")]);
    for magic in [&b"MZ"[..], b"\x7fELF"] {
        let (f, d) = detect("setup.exe", &sfx(magic, &docx));
        assert_eq!(f, FormatId::Zip, "{}", d.evidence);
        assert_eq!(d.confidence, Confidence::Likely);
        assert_eq!(d.evidence, "zip archive after a stub");
    }
    // An empty end record after binary noise is not enough.
    let mut noise: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    noise.extend_from_slice(b"PK\x05\x06");
    noise.extend_from_slice(&[0u8; 18]);
    assert_eq!(detect("x", &noise).0, FormatId::Unknown);
}

#[test]
fn multi_document_yaml_with_comment_and_list_is_not_markdown() {
    let k8s =
        b"---\napiVersion: v1\nkind: Pod\n---\n# second resource\nkind: List\nitems:\n- a\n- b\n";
    let (f, d) = detect("k8s.yaml", k8s);
    assert_eq!(f, FormatId::Text, "{}", d.evidence);
}

#[test]
fn mime_hint_never_overrides_magic() {
    let (f, d) = detect_with_hints("report.txt", Some("text/plain"), b"%PDF-1.7\n%\xE2\xE3\n");
    assert_eq!(f, FormatId::Pdf, "{}", d.evidence);
    let (f, _) = detect_with_hints("report.txt", Some("text/csv"), b"%PDF-1.7\n%\xE2\xE3\n");
    assert_eq!(f, FormatId::Pdf);
}

#[test]
fn mime_csv_breaks_tie_when_sniff_declines() {
    // Two records only: sniffing declines and the name gives no tie-breaker.
    let (f, d) = detect_with_hints("upload", Some("text/csv"), b"a,b\n1,2\n");
    assert_eq!(f, FormatId::Csv, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    assert!(d.evidence.contains("MIME type text/csv"), "{}", d.evidence);
    let (f, _) = detect_with_hints("upload", Some("text/tab-separated-values"), b"a\tb\n1\t2\n");
    assert_eq!(f, FormatId::Tsv);
}

#[test]
fn mime_with_parameters_and_case_is_normalized() {
    let (f, d) = detect_with_hints("upload", Some(" Text/CSV ; charset=utf-8"), b"a,b\n1,2\n");
    assert_eq!(f, FormatId::Csv, "{}", d.evidence);
    assert!(d.evidence.contains("MIME type text/csv"), "{}", d.evidence);
}

#[test]
fn mime_csv_keeps_the_width_check() {
    let (f, d) = detect_with_hints("upload", Some("text/csv"), b"a,b\n1,2,3\nfree text\n");
    assert_eq!(f, FormatId::Text, "{}", d.evidence);
}

#[test]
fn mime_markdown_on_plain_text_gives_markdown() {
    let (f, d) = detect_with_hints("upload", Some("text/markdown"), b"Just a line of prose.\n");
    assert_eq!(f, FormatId::Markdown, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Likely);
    assert_eq!(
        detect_with_hints("upload", None, b"Just a line of prose.\n").0,
        FormatId::Text
    );
}

#[test]
fn mime_beats_extension_on_a_tie() {
    // Plain prose named .md with a declared text/plain: the MIME type is no format hint,
    // the extension still decides.
    let (f, _) = detect_with_hints("x.md", Some("text/plain"), b"Just prose.\n");
    assert_eq!(f, FormatId::Markdown);
    // A declared text/markdown wins over a .csv name when the content is no table.
    let (f, _) = detect_with_hints("x.csv", Some("text/markdown"), b"Just prose.\n");
    assert_eq!(f, FormatId::Markdown);
}

#[test]
fn disagreeing_mime_lowers_confidence() {
    let csv = b"name,age,city\nann,31,Bonn\nbob,42,Koeln\n";
    let (f, plain) = detect_with_hints("upload", None, csv);
    assert_eq!(f, FormatId::Csv);
    assert_eq!(plain.confidence, Confidence::Likely);
    let (f, d) = detect_with_hints("upload", Some("text/markdown"), csv);
    assert_eq!(f, FormatId::Csv, "content evidence stays");
    assert_eq!(d.confidence, Confidence::Guess);
    assert!(d.evidence.contains("text/markdown"), "{}", d.evidence);
    // text/plain is the default of data: URIs and says nothing: no change.
    assert_eq!(
        detect_with_hints("upload", Some("text/plain"), csv).1,
        plain
    );
}

#[test]
fn detect_is_detect_with_no_hints() {
    let samples: [(&str, &[u8]); 4] = [
        ("x.csv", b"a,b\n1,2\n"),
        ("x.md", b"Just prose.\n"),
        ("x.txt", b"%PDF-1.7\n"),
        ("x", b""),
    ];
    for (name, bytes) in samples {
        assert_eq!(detect(name, bytes), detect_with_hints(name, None, bytes));
    }
}

#[test]
fn from_bytes_with_hints_uses_the_mime_type() {
    let src = from_bytes_with_hints(
        "upload",
        Some("text/csv"),
        b"a,b\n1,2\n".to_vec(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(src.format, FormatId::Csv);
    assert_eq!(
        from_bytes("upload", b"a,b\n1,2\n".to_vec(), &Limits::default())
            .unwrap()
            .format,
        FormatId::Text
    );
}

#[test]
fn a_custom_detector_sees_name_mime_and_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, b"hello").unwrap();
    let detector = |name: &str, mime: Option<&str>, bytes: &[u8]| {
        let evidence = format!("{name} {mime:?} {}", bytes.len());
        (
            FormatId::Csv,
            rezzer_model::Detection {
                confidence: Confidence::Guess,
                evidence,
            },
        )
    };
    let src = rezzer_detect::open_with_detector(&path, &Limits::default(), &detector).unwrap();
    assert_eq!(src.format, FormatId::Csv);
    assert_eq!(src.detection.evidence, "a.txt None 5");
    assert_eq!(src.size, 5);
    let src = rezzer_detect::from_bytes_with_detector(
        "b",
        Some("text/csv"),
        b"xy".to_vec(),
        &Limits::default(),
        &detector,
    )
    .unwrap();
    assert_eq!(src.detection.evidence, "b Some(\"text/csv\") 2");
    assert_eq!(src.sha256.len(), 64);
}

#[test]
fn xlsm_detected_by_vba_part() {
    // The content types name no main part, so the part names decide: a VBA project next
    // to the workbook makes it a macro workbook, whatever the file name says.
    let ct = content_types("");
    let with_vba = zip_of(&[
        ("[Content_Types].xml", &ct),
        ("xl/workbook.xml", b"<w/>"),
        ("xl/vbaProject.bin", b"\0"),
    ]);
    let (f, d) = detect("book.xlsx", &with_vba);
    assert_eq!(f, FormatId::Xlsm, "{}", d.evidence);
    assert!(d.evidence.contains("xl/vbaProject.bin"), "{}", d.evidence);
    let without = zip_of(&[("[Content_Types].xml", &ct), ("xl/workbook.xml", b"<w/>")]);
    assert_eq!(detect("book.xlsm", &without).0, FormatId::Xlsx);
}

#[test]
fn ods_detected_by_mimetype() {
    let bytes = odf("application/vnd.oasis.opendocument.spreadsheet");
    let (f, d) = detect("data.csv", &bytes);
    assert_eq!(f, FormatId::Ods, "{}", d.evidence);
    assert_eq!(d.confidence, Confidence::Certain);
}

#[test]
fn xlsx_named_csv_still_reads_as_xlsx() {
    let ct = content_types(
        r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    let bytes = zip_of(&[
        ("[Content_Types].xml", &ct),
        ("xl/workbook.xml", b"<workbook/>"),
    ]);
    for name in ["data.csv", "data.tsv", "data.txt"] {
        let (f, d) = detect(name, &bytes);
        assert_eq!(f, FormatId::Xlsx, "{name}: {}", d.evidence);
        assert_eq!(d.confidence, Confidence::Certain, "{name}");
    }
}

#[test]
fn encrypted_package_named_csv_is_not_taken_for_a_workbook() {
    // The extension only breaks the tie between the OLE office formats for .xlsx, .xlsm and
    // .xlsb; any other name leaves the container as plain OLE.
    let bytes = cfb_with(&["EncryptionInfo", "EncryptedPackage"]);
    assert_eq!(detect("secret.csv", &bytes).0, FormatId::Ole);
    assert_eq!(detect("secret.xlsx", &bytes).0, FormatId::Xlsx);
}

// ---------------------------------------------------------------------------------------
// File system facts: birth time, download source, file tags

/// RFC 3339 in UTC with whole seconds: `YYYY-MM-DDTHH:MM:SSZ`.
fn is_rfc3339_utc(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

#[test]
fn created_time_when_the_system_has_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.txt");
    std::fs::write(&path, b"hello\n").unwrap();
    let src = open(&path, &Limits::default()).unwrap();
    #[cfg(target_os = "macos")]
    {
        let fs = fs_type(dir.path());
        if fs != "apfs" {
            eprintln!("skipped: the temporary folder is on {fs}, not APFS");
            return;
        }
        let created = src.created.as_deref().expect("APFS keeps a birth time");
        assert!(is_rfc3339_utc(created), "{created}");
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(created) = src.created.as_deref() {
        assert!(is_rfc3339_utc(created), "{created}");
    }
}

/// The file system type of the volume that holds `path`, for example `apfs`.
#[cfg(target_os = "macos")]
fn fs_type(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: statfs only writes into the zeroed struct it is given; the path is a
    // NUL-terminated string.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let rc = unsafe { libc::statfs(path.as_ptr(), &raw mut stat) };
    assert_eq!(rc, 0, "{}", std::io::Error::last_os_error());
    // SAFETY: the kernel fills f_fstypename with a NUL-terminated name.
    unsafe { std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn missing_attributes_leave_keys_out() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.txt");
    std::fs::write(&path, b"hello\n").unwrap();
    let src = open(&path, &Limits::default()).unwrap();
    assert_eq!(src.downloaded_from, None);
    assert!(src.file_tags.is_empty(), "{:?}", src.file_tags);
    let json = serde_json::to_value(rezzer_model::source_info(&src)).unwrap();
    assert!(json.get("downloaded_from").is_none(), "{json}");
    assert!(json.get("file_tags").is_none(), "{json}");
}

#[test]
fn bytes_have_no_file_facts() {
    let src = from_bytes("x.txt", b"hello\n".to_vec(), &Limits::default()).unwrap();
    assert_eq!(src.modified, None);
    assert_eq!(src.created, None);
    assert_eq!(src.downloaded_from, None);
    assert!(src.file_tags.is_empty());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod xattrs {
    use std::path::{Path, PathBuf};

    use fileattrs::test_support::set_xattr_at as set;
    use rezzer_detect::open;
    use rezzer_limits::Limits;

    #[cfg(target_os = "macos")]
    const WHERE_FROMS: &[u8] =
        include_bytes!("../../formats/fileattrs/tests/data/where_froms_plutil.bplist");
    #[cfg(target_os = "macos")]
    const TAGS: &[u8] = include_bytes!("../../formats/fileattrs/tests/data/tags_plutil.bplist");

    /// A binary plist holding an empty array: header, the array object (0xA0), a one-byte
    /// offset table and the 32-byte trailer (one object, top 0, offset table at 9).
    #[cfg(target_os = "macos")]
    const EMPTY_ARRAY: &[u8] = &[
        b'b', b'p', b'l', b'i', b's', b't', b'0', b'0', 0xA0, 8, //
        0, 0, 0, 0, 0, 0, 1, 1, //
        0, 0, 0, 0, 0, 0, 0, 1, //
        0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 9,
    ];

    /// A binary plist holding the array `["javascript:alert(1)"]`: the array (0xA1, ref 1)
    /// at offset 8, the ASCII string (0x5F, length 19 as a one-byte integer) at offset 10,
    /// the offset table at 32 and the trailer (two objects, top 0).
    #[cfg(target_os = "macos")]
    fn javascript_where_froms() -> Vec<u8> {
        let mut bytes = b"bplist00".to_vec();
        bytes.extend_from_slice(&[0xA1, 1, 0x5F, 0x10, 19]);
        bytes.extend_from_slice(b"javascript:alert(1)");
        bytes.extend_from_slice(&[8, 10]);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
        bytes.extend_from_slice(&2u64.to_be_bytes());
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes.extend_from_slice(&32u64.to_be_bytes());
        bytes
    }

    /// A new file in its own temporary folder.
    fn file(dir: &tempfile::TempDir) -> PathBuf {
        let path = dir.path().join("x.pdf");
        std::fs::write(&path, b"%PDF-1.4\n").unwrap();
        path
    }

    /// Sets the attribute, or says why the test is skipped and returns false.
    fn try_set(path: &Path, name: &str, value: &[u8]) -> bool {
        match set(path, name, value) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipped: this file system refuses {name} ({error})");
                false
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn where_froms_from_xattr() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        set(&path, "com.apple.metadata:kMDItemWhereFroms", WHERE_FROMS).unwrap();
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(
            src.downloaded_from.as_deref(),
            Some("https://example.com/files/report.pdf")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_tags_without_colors() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        set(&path, "com.apple.metadata:_kMDItemUserTags", TAGS).unwrap();
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(src.file_tags, ["Größe", "Red", "日本"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn broken_where_froms_leave_the_key_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        set(
            &path,
            "com.apple.metadata:kMDItemWhereFroms",
            &WHERE_FROMS[..20],
        )
        .unwrap();
        set(&path, "com.apple.metadata:_kMDItemUserTags", b"not a plist").unwrap();
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(src.downloaded_from, None);
        assert!(src.file_tags.is_empty(), "{:?}", src.file_tags);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plists_built_here_read_as_meant() {
        assert!(fileattrs::string_array(EMPTY_ARRAY).unwrap().is_empty());
        assert_eq!(
            fileattrs::string_array(&javascript_where_froms()).unwrap(),
            ["javascript:alert(1)"]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn broken_or_empty_where_froms_fall_back_to_the_origin_url() {
        for where_froms in [&WHERE_FROMS[..20], EMPTY_ARRAY, b"not a plist"] {
            let dir = tempfile::tempdir().unwrap();
            let path = file(&dir);
            set(&path, "com.apple.metadata:kMDItemWhereFroms", where_froms).unwrap();
            set(&path, "user.xdg.origin.url", b"https://example.org/a.pdf").unwrap();
            let src = open(&path, &Limits::default()).unwrap();
            assert_eq!(
                src.downloaded_from.as_deref(),
                Some("https://example.org/a.pdf"),
                "{where_froms:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unsafe_first_where_from_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        set(
            &path,
            "com.apple.metadata:kMDItemWhereFroms",
            &javascript_where_froms(),
        )
        .unwrap();
        set(&path, "user.xdg.origin.url", b"https://example.org/a.pdf").unwrap();
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(src.downloaded_from, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn broken_or_empty_finder_tags_fall_back_to_xdg_tags() {
        for finder in [&TAGS[..20], EMPTY_ARRAY, b"not a plist"] {
            let dir = tempfile::tempdir().unwrap();
            let path = file(&dir);
            set(&path, "com.apple.metadata:_kMDItemUserTags", finder).unwrap();
            set(&path, "user.xdg.tags", b"Rot,Projekt").unwrap();
            let src = open(&path, &Limits::default()).unwrap();
            assert_eq!(src.file_tags, ["Rot", "Projekt"], "{finder:?}");
        }
        // Readable Finder tags win over the freedesktop ones.
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        set(&path, "com.apple.metadata:_kMDItemUserTags", TAGS).unwrap();
        set(&path, "user.xdg.tags", b"Rot,Projekt").unwrap();
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(src.file_tags, ["Größe", "Red", "日本"]);
    }

    #[test]
    fn xdg_origin_url_from_xattr() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        if !try_set(&path, "user.xdg.origin.url", b"https://example.org/a.pdf") {
            return;
        }
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(
            src.downloaded_from.as_deref(),
            Some("https://example.org/a.pdf")
        );
    }

    #[test]
    fn unsafe_origin_urls_are_dropped() {
        for (value, expected) in [
            (&b"javascript:alert(1)"[..], None),
            (
                b"https://user:pw@example.org/a.pdf",
                Some("https://example.org/a.pdf"),
            ),
            (b"\xff\xfe", None),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = file(&dir);
            if !try_set(&path, "user.xdg.origin.url", value) {
                return;
            }
            let src = open(&path, &Limits::default()).unwrap();
            assert_eq!(src.downloaded_from.as_deref(), expected, "{value:?}");
        }
    }

    #[test]
    fn xdg_tags_from_xattr() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        if !try_set(&path, "user.xdg.tags", b"Rot, Projekt,,") {
            return;
        }
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(src.file_tags, ["Rot", "Projekt"]);
    }

    #[test]
    fn file_tags_are_capped_at_64() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(&dir);
        let many: Vec<String> = (0..100).map(|i| format!("t{i}")).collect();
        if !try_set(&path, "user.xdg.tags", many.join(",").as_bytes()) {
            return;
        }
        let src = open(&path, &Limits::default()).unwrap();
        assert_eq!(src.file_tags, many[..64]);
    }
}
