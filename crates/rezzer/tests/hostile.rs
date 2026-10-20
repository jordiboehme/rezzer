// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hostile input through the whole pipeline (text, Markdown, CSV and spreadsheet readers, no
//! cache): the run ends cleanly, inside its limits, and says what it could not do. The files
//! are made in memory or in a temp dir, or come from `tests/corpus/office/malformed/`;
//! nothing is executed.
#![cfg(all(
    feature = "fmt-text",
    feature = "fmt-markdown",
    feature = "fmt-csv",
    feature = "fmt-xlsx",
    feature = "fmt-ods"
))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use rezzer_model::{Completeness, LossReason, Registry};
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request, RunResult};

/// Counts the heap of the thread that allocates, so tests that run side by side do not
/// disturb a measurement. The pipeline runs on the calling thread.
struct Counting;

thread_local! {
    static CURRENT: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn grow(n: usize) {
    let now = CURRENT.with(|c| {
        c.set(c.get() + n as isize);
        c.get()
    });
    PEAK.with(|p| p.set(p.get().max(now)));
}

fn shrink(n: usize) {
    CURRENT.with(|c| c.set(c.get() - n as isize));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        grow(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        shrink(l.size());
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        shrink(l.size());
        grow(new);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns its result with the peak heap it added on this thread, in bytes.
fn peak_heap<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = CURRENT.with(Cell::get);
    PEAK.with(|p| p.set(base));
    let out = f();
    let peak = PEAK.with(Cell::get);
    (out, usize::try_from(peak - base).unwrap_or(0))
}

/// Allowed overshoot of a time limit (CI machines are slower than a laptop).
const SLACK: Duration = Duration::from_secs(2);

fn pipeline_with(max_time: Option<Duration>) -> Pipeline {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    if let Some(t) = max_time {
        config.limits.max_time = t;
    }
    Pipeline::new(config, registry)
}

fn request() -> Request {
    Request {
        output: OutputKind::Json,
        generated_at: Some("2026-01-01T00:00:00Z".to_string()),
        ..Request::default()
    }
}

/// What the CLI and the server give by default: Markdown pages, no JSON copy of the model.
fn markdown_request() -> Request {
    Request {
        output: OutputKind::Markdown,
        ..request()
    }
}

fn has_loss(result: &RunResult, why: LossReason) -> bool {
    result
        .document
        .as_ref()
        .is_some_and(|d| d.losses.iter().any(|l| l.why == why))
}

/// A cheap deterministic byte stream (xorshift), so the "random" input is the same each run.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

#[test]
fn deep_markdown_nesting_is_partial() {
    let mut bytes = ">".repeat(100_000).into_bytes();
    bytes.extend_from_slice(b" deep\n");
    let p = pipeline_with(None);
    let result = p.run_bytes("deep.md", bytes, None, &request());
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Limit));
}

#[test]
fn huge_csv_row_respects_time_limit() {
    let limit = Duration::from_secs(1);
    let p = pipeline_with(Some(limit));
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat_n(b'x', 50 * 1024 * 1024));
    bytes.push(b'\n');
    let start = Instant::now();
    let result = p.run_bytes("huge.csv", bytes, None, &request());
    let took = start.elapsed();
    assert!(took < limit + SLACK, "took {took:?}");
    // Either the limit was hit (Partial with a Limit loss) or the run finished in time
    // (Complete); the test machine decides which, both are clean.
    match result.completeness {
        Completeness::Partial => assert!(has_loss(&result, LossReason::Limit), "{result:?}"),
        Completeness::Complete => {}
        Completeness::Failed => panic!("{result:?}"),
    }
    eprintln!("huge_csv_row: {:?} in {took:?}", result.completeness);
}

#[test]
fn many_csv_rows_stop_at_the_time_limit() {
    use std::io::Write;
    // Short limit and a file that needs far more than that to analyze, so the test also
    // fails on a fast machine when the analysis ignores the limit. The file is written in
    // a stream to a temp dir and read with run_path (memory-mapped).
    let limit = Duration::from_millis(50);
    let p = pipeline_with(Some(limit));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.csv");
    {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let mut written = 0usize;
        let mut i = 0u32;
        while written < 100 * 1024 * 1024 {
            let line = format!("{i},v{i}\n");
            out.write_all(line.as_bytes()).unwrap();
            written += line.len();
            i += 1;
        }
        out.flush().unwrap();
    }
    let start = Instant::now();
    let result = p.run_path(&path, &request());
    let took = start.elapsed();
    assert!(took < limit + SLACK, "took {took:?}");
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Limit));
}

#[test]
fn file_over_size_limit_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.txt");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(3 * 1024 * 1024 * 1024).unwrap();
    drop(file);
    let p = pipeline_with(None);
    let result = p.run_path(&path, &request());
    assert_eq!(result.completeness, Completeness::Failed);
    assert!(result.document.is_none());
    let message = result.message.expect("a message");
    assert!(
        message.contains("size") || message.contains("large"),
        "{message}"
    );
}

#[test]
fn binary_garbage_is_unknown_not_failed() {
    let p = pipeline_with(None);
    let result = p.run_bytes("blob.bin", noise(1024 * 1024), None, &request());
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Unsupported));
}

#[test]
fn invalid_utf8_in_markdown_decodes() {
    let p = pipeline_with(None);
    let bytes = b"# Title\n\nbad \xff\xfe\xc3\x28 bytes\n\n- item\n".to_vec();
    let result = p.run_bytes("bad.md", bytes, None, &request());
    assert_ne!(result.completeness, Completeness::Failed, "{result:?}");
    let md = result.markdown_pages.join("");
    assert!(md.contains("Title") && md.contains("item"), "{md}");
}

#[test]
fn nul_and_empty_inputs_do_not_panic() {
    let p = pipeline_with(None);
    for (name, bytes) in [
        ("empty.md", Vec::new()),
        ("nul.txt", vec![0u8; 4096]),
        ("nul.csv", b"a,b\n\0\0,\0\n".to_vec()),
        ("lone.md", b"\xef\xbb\xbf".to_vec()),
    ] {
        let result = p.run_bytes(name, bytes, None, &request());
        assert!(
            result.document.is_some() || result.message.is_some(),
            "{name}"
        );
        if name == "empty.md" || name == "lone.md" {
            assert_ne!(
                result.completeness,
                Completeness::Failed,
                "{name}: {result:?}"
            );
        }
    }
}

/// Writes `unit` repeated `times` times to `name` in a temp dir, in a stream.
fn streamed(dir: &std::path::Path, name: &str, unit: &[u8], times: usize) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    for _ in 0..times {
        out.write_all(unit).unwrap();
    }
    out.flush().unwrap();
    path
}

fn element_limit_loss(result: &RunResult) -> bool {
    result.document.as_ref().is_some_and(|d| {
        d.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.starts_with("element limit reached"))
    })
}

#[test]
fn many_markdown_inlines_stop_at_the_element_limit() {
    // 1.2 million emphasized words are 2.4 million inlines, over the default of 2 million.
    let dir = tempfile::tempdir().unwrap();
    let path = streamed(dir.path(), "inlines.md", b"*a* ", 1_200_000);
    let result = pipeline_with(None).run_path(&path, &request());
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        result.message
    );
    assert!(element_limit_loss(&result));
}

#[test]
fn many_text_paragraphs_stop_at_the_element_limit() {
    // 1.1 million paragraphs are 2.2 million elements.
    let dir = tempfile::tempdir().unwrap();
    let path = streamed(dir.path(), "paras.txt", b"a\n\n", 1_100_000);
    let result = pipeline_with(None).run_path(&path, &request());
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        result.message
    );
    assert!(element_limit_loss(&result));
}

#[test]
fn huge_single_paragraph_is_bounded() {
    // 100 MB without a line break is one paragraph. The file is read from disk (memory
    // mapped), so the heap shows only the copies the pipeline makes.
    const SIZE: usize = 100 * 1000 * 1000;
    let dir = tempfile::tempdir().unwrap();
    let path = streamed(dir.path(), "one.txt", &[b'x'; 1000], SIZE / 1000);

    let (result, peak) = peak_heap(|| pipeline_with(None).run_path(&path, &markdown_request()));
    eprintln!("huge_single_paragraph: peak heap {peak} bytes for {SIZE} bytes of text");
    assert_ne!(result.completeness, Completeness::Failed, "{result:?}");
    assert!(peak < 4 * SIZE, "peak heap {peak} bytes");

    // With a text limit below the input the paragraph is cut and the loss says so.
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    config.limits.max_text_bytes = (SIZE / 2) as u64;
    let p = Pipeline::new(config, {
        let mut registry = Registry::new();
        registry.register(Box::new(rezzer_read_text::TextReader));
        registry
    });
    let (result, peak) = peak_heap(|| p.run_path(&path, &markdown_request()));
    eprintln!("huge_single_paragraph (limit): peak heap {peak} bytes");
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Limit));
    assert!(peak < 4 * SIZE, "peak heap {peak} bytes");
}

// Hostile spreadsheets. The workbooks are built in memory; the two malformed files are public
// test files of calamine (see tests/corpus/MANIFEST.md).

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// A zip of these entries, deflated.
fn zip_of(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::{SimpleFileOptions, ZipWriter};
    let mut w = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        w.start_file(name.as_str(), opts).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// An XLSX with one worksheet part per entry of `sheets` (name, whole sheet XML).
fn xlsx(sheets: &[(&str, String)]) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides = String::new();
    let mut list = String::new();
    let mut rels = String::new();
    for (i, (name, xml)) in sheets.iter().enumerate() {
        let n = i + 1;
        overrides.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
        list.push_str(&format!(
            "<sheet name=\"{name}\" sheetId=\"{n}\" r:id=\"rId{n}\"/>"
        ));
        rels.push_str(&format!(
            "<Relationship Id=\"rId{n}\" Type=\"{REL_NS}/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
        ));
        entries.push((
            format!("xl/worksheets/sheet{n}.xml"),
            xml.clone().into_bytes(),
        ));
    }
    let mut files = vec![
        (
            "[Content_Types].xml".to_string(),
            format!(
                "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>{overrides}</Types>"
            ),
        ),
        (
            "_rels/.rels".to_string(),
            format!(
                "<Relationships xmlns=\"{PKG_NS}\"><Relationship Id=\"rId1\" Type=\"{REL_NS}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
            ),
        ),
        (
            "xl/workbook.xml".to_string(),
            format!(
                "<workbook xmlns=\"{MAIN_NS}\" xmlns:r=\"{REL_NS}\"><sheets>{list}</sheets></workbook>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".to_string(),
            format!("<Relationships xmlns=\"{PKG_NS}\">{rels}</Relationships>"),
        ),
    ]
    .into_iter()
    .map(|(n, s)| (n, s.into_bytes()))
    .collect::<Vec<_>>();
    files.extend(entries);
    zip_of(&files)
}

/// A worksheet with these rows (and anything before or after `sheetData`).
fn worksheet(before: &str, rows: &str, after: &str) -> String {
    format!(
        "<worksheet xmlns=\"{MAIN_NS}\" xmlns:r=\"{REL_NS}\">{before}<sheetData>{rows}</sheetData>{after}</worksheet>"
    )
}

fn small_worksheet() -> String {
    worksheet(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>7</v></c><c r=\"B1\"><f>A1*2</f><v>14</v></c></row>",
        "",
    )
}

const ODS_MIME: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// An ODS with this content.xml.
fn ods(content: &str) -> Vec<u8> {
    let manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{ODS_MIME}\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>"
    );
    zip_of(&[
        ("mimetype".to_string(), ODS_MIME.as_bytes().to_vec()),
        ("META-INF/manifest.xml".to_string(), manifest.into_bytes()),
        ("content.xml".to_string(), content.as_bytes().to_vec()),
    ])
}

/// content.xml with the usual namespaces, `prolog` before the root and `body` inside
/// `office:spreadsheet`.
fn ods_content(prolog: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>{prolog}<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" office:version=\"1.2\"><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"
    )
}

fn sheet_pipeline() -> Pipeline {
    pipeline_with(None)
}

/// Runs `bytes` as `name` and returns the result, its peak heap and its time.
fn measured(p: &Pipeline, name: &str, bytes: Vec<u8>) -> (RunResult, usize, Duration) {
    let start = Instant::now();
    let (result, peak) = peak_heap(|| p.run_bytes(name, bytes, None, &markdown_request()));
    (result, peak, start.elapsed())
}

const MB: usize = 1024 * 1024;

fn losses(result: &RunResult) -> Vec<String> {
    result.document.as_ref().map_or_else(Vec::new, |d| {
        d.losses
            .iter()
            .map(|l| format!("{:?} {} {}", l.why, l.location, l.what))
            .collect()
    })
}

/// The heap measure sees the reader's work: a real workbook costs megabytes on this thread.
#[test]
fn heap_measure_sees_the_sheet_reader() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office/ex012.PVT_z_factor.xlsm");
    let (result, peak) = peak_heap(|| sheet_pipeline().run_path(&path, &markdown_request()));
    eprintln!("pvt workbook: peak heap {} MB", peak / MB);
    assert_eq!(
        result.completeness,
        Completeness::Complete,
        "{:?}",
        losses(&result)
    );
    assert!(peak > MB, "peak heap {peak} bytes");
}

/// Rows of numbers: varied enough to stay under the 100:1 ratio, big when unpacked.
fn number_rows(rows: u32) -> String {
    let mut out = String::new();
    for r in 1..=rows {
        out.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\"><v>{}</v></c><c r=\"B{r}\"><v>{}</v></c></row>",
            r.wrapping_mul(2_654_435_761) % 1_000_003,
            r.wrapping_mul(40_503) % 65_521
        ));
    }
    out
}

#[test]
fn xlsx_zip_bomb_is_partial() {
    // Each sheet fits the entry and ratio limits, together they unpack past the limit: the
    // sheets that still fit are read, the rest is a Limit loss.
    let big = worksheet("", &number_rows(20_000), "");
    let book = xlsx(&[
        ("Small", small_worksheet()),
        ("Big1", big.clone()),
        ("Big2", big.clone()),
        ("Big3", big),
    ]);
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    config.limits.max_unpacked_bytes = 8 * MB as u64;
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    let p = Pipeline::new(config, registry);
    let (result, peak, took) = measured(&p, "bomb.xlsx", book);
    eprintln!(
        "xlsx_zip_bomb_is_partial: peak heap {} MB in {took:?}: {:?}",
        peak / MB,
        losses(&result)
    );
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.starts_with("Limit") && l.contains("sheet not read: unpacked limit reached")),
        "{:?}",
        losses(&result)
    );
    let md = result.markdown_pages.join("");
    assert!(md.contains("## Small") && md.contains("| 7 | 14 |"), "{md}");
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
}

#[test]
fn xlsx_zip_bomb_is_refused() {
    // One sheet of 256 MB of spaces packs into a few hundred kilobytes: far over the 100:1
    // ratio. The read stops before the sheet is unpacked and says why.
    let mut sheet = format!(
        "<worksheet xmlns=\"{MAIN_NS}\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>BOMB"
    );
    sheet.extend(std::iter::repeat_n(' ', 256 * MB));
    sheet.push_str("</t></is></c></row></sheetData></worksheet>");
    let book = xlsx(&[("Small", small_worksheet()), ("Bomb", sheet)]);
    let p = sheet_pipeline();
    let (result, peak, took) = measured(&p, "bomb.xlsx", book);
    eprintln!(
        "xlsx_zip_bomb_is_refused: peak heap {} MB in {took:?}",
        peak / MB
    );
    assert_eq!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(
        has_loss(&result, LossReason::Limit),
        "{:?}",
        losses(&result)
    );
    assert!(!result.markdown_pages.join("").contains("BOMB"));
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(took < Duration::from_secs(120) + SLACK, "took {took:?}");
}

#[test]
fn ods_repeat_bomb_is_bounded() {
    // calamine's issue 594: repeat counts that ask for about 3.9 TB of cells.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office/malformed/issue_594_dos.ods");
    let limit = Duration::from_secs(10);
    let p = pipeline_with(Some(limit));
    let start = Instant::now();
    let (result, peak) = peak_heap(|| p.run_path(&path, &markdown_request()));
    let took = start.elapsed();
    eprintln!("ods_repeat_bomb: peak heap {} MB in {took:?}", peak / MB);
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(took < limit + SLACK, "took {took:?}");
    assert_eq!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(
        has_loss(&result, LossReason::Limit),
        "{:?}",
        losses(&result)
    );
}

#[test]
fn sparse_far_cells_do_not_allocate_grid() {
    // A1 and XFD1048576 span 17 billion cells; only the two that exist may cost memory.
    let sheet = worksheet(
        "<dimension ref=\"A1:XFD1048576\"/>",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"1048576\"><c r=\"XFD1048576\"><f>A1*2</f><v>2</v></c></row>",
        "",
    );
    let p = sheet_pipeline();
    let (result, peak, took) = measured(&p, "sparse.xlsx", xlsx(&[("S", sheet)]));
    eprintln!("sparse_far_cells: peak heap {} MB in {took:?}", peak / MB);
    assert!(peak < 64 * MB, "peak heap {peak} bytes");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_ne!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    let doc = result.document.as_ref().unwrap();
    // The value table stops at its caps and says so; the far formula is still listed.
    assert!(
        doc.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("not shown")),
        "{:?}",
        losses(&result)
    );
    let far = doc
        .facets
        .formulas
        .iter()
        .find(|f| f.anchor.0 == "S!XFD1048576")
        .expect("the far formula");
    assert_eq!(far.references[0].0, "S!A1");
}

#[test]
fn xlsb_panic_file_is_contained() {
    // calamine's issue 666: a workbook record that once made calamine panic.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office/malformed/issue_666_panic.xlsb");
    let result = sheet_pipeline().run_path(&path, &markdown_request());
    let doc = result.document.as_ref().expect("a document, not an abort");
    assert_eq!(doc.source.format, rezzer_model::FormatId::Xlsb);
    assert_ne!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(result.markdown_pages.join("").contains("## Sheet1"));
}

#[test]
fn deep_xml_stops_at_depth() {
    // 100,000 nested elements after the cells, in an XLSX sheet and in ODS content. Each
    // element has its own name, so the part stays under the 100:1 ratio and is parsed.
    let names: Vec<String> = (0..100_000u32)
        .map(|i| format!("x:e{}", i.wrapping_mul(2_654_435_761)))
        .collect();
    let mut deep = String::from("<x:e xmlns:x=\"urn:deep\">");
    names.iter().for_each(|n| deep.push_str(&format!("<{n}>")));
    names
        .iter()
        .rev()
        .for_each(|n| deep.push_str(&format!("</{n}>")));
    deep.push_str("</x:e>");
    let sheet = worksheet(
        "",
        "<row r=\"1\"><c r=\"A1\"><v>5</v></c></row>",
        &format!("<extLst><ext uri=\"urn:deep\">{deep}</ext></extLst>"),
    );
    let p = sheet_pipeline();
    let (result, peak, took) = measured(&p, "deep.xlsx", xlsx(&[("S", sheet)]));
    eprintln!(
        "deep_xml (xlsx): {:?}, peak heap {} MB in {took:?}: {:?}",
        result.completeness,
        peak / MB,
        losses(&result)
    );
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        losses(&result)
    );
    assert!(depth_loss(&result), "{:?}", losses(&result));
    // The cells before the deep part are read.
    assert!(result.markdown_pages.join("").contains("| 5 |"));

    // In ODF content namespaces are declared on the root only: nested spans in a cell.
    let mut spans = String::new();
    (0..100_000u32).for_each(|i| {
        spans.push_str(&format!(
            "<text:span text:style-name=\"T{}\">",
            i.wrapping_mul(2_654_435_761)
        ))
    });
    spans.push_str(&"</text:span>".repeat(100_000));
    let body = format!(
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"float\" office:value=\"5\"><text:p>5</text:p></table:table-cell><table:table-cell office:value-type=\"string\"><text:p>{spans}</text:p></table:table-cell></table:table-row></table:table>"
    );
    let (result, peak, took) = measured(&p, "deep.ods", ods(&ods_content("", &body)));
    eprintln!(
        "deep_xml (ods): {:?}, peak heap {} MB in {took:?}: {:?}",
        result.completeness,
        peak / MB,
        losses(&result)
    );
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    // The ODF part reader stops at the depth limit; the sheet list then disagrees with
    // calamine's, so the whole read is refused, and both reasons are listed.
    assert_ne!(
        result.completeness,
        Completeness::Complete,
        "{:?}",
        losses(&result)
    );
    assert!(depth_loss(&result), "{:?}", losses(&result));
}

fn depth_loss(result: &RunResult) -> bool {
    result.document.as_ref().is_some_and(|d| {
        d.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.contains("nesting deeper than 256"))
    })
}

/// The classic entity bomb: ten levels of ten references, a billion `lol`s when expanded.
fn laughs() -> String {
    let mut dtd = String::from("<!DOCTYPE lolz [<!ENTITY lol \"lol\">");
    for i in 1..=9 {
        let prev = if i == 1 {
            "lol".to_string()
        } else {
            format!("lol{}", i - 1)
        };
        dtd.push_str(&format!(
            "<!ENTITY lol{i} \"{}\">",
            format!("&{prev};").repeat(10)
        ));
    }
    dtd.push_str("]>");
    dtd
}

#[test]
fn billion_laughs_is_refused() {
    let p = sheet_pipeline();
    let sheet = format!(
        "<?xml version=\"1.0\"?>{}{}",
        laughs(),
        worksheet(
            "",
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>&lol9;</t></is></c></row>",
            ""
        )
    );
    let (result, peak, took) = measured(&p, "laughs.xlsx", xlsx(&[("S", sheet)]));
    eprintln!(
        "billion_laughs (xlsx): {:?}, peak heap {} MB in {took:?}: {:?}",
        result.completeness,
        peak / MB,
        losses(&result)
    );
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_ne!(
        result.completeness,
        Completeness::Complete,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result).iter().any(|l| l.contains("DOCTYPE")),
        "{:?}",
        losses(&result)
    );
    assert!(!result.markdown_pages.join("").contains("lollol"));

    let content = ods_content(
        &laughs(),
        "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value-type=\"string\"><text:p>&lol9;</text:p></table:table-cell></table:table-row></table:table>",
    );
    let (result, peak, took) = measured(&p, "laughs.ods", ods(&content));
    eprintln!(
        "billion_laughs (ods): {:?}, peak heap {} MB in {took:?}: {:?}",
        result.completeness,
        peak / MB,
        losses(&result)
    );
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_ne!(
        result.completeness,
        Completeness::Complete,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result).iter().any(|l| l.contains("DOCTYPE")),
        "{:?}",
        losses(&result)
    );
    assert!(!result.markdown_pages.join("").contains("lollol"));
}
