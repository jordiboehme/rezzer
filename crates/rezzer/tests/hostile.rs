// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hostile input through the whole pipeline (text, Markdown, CSV, spreadsheet and PDF readers,
//! no cache): the run ends cleanly, inside its limits, and says what it could not do. The files
//! are made in memory or in a temp dir, or come from `tests/corpus/office/malformed/`;
//! nothing is executed.
#![cfg(all(
    feature = "fmt-text",
    feature = "fmt-markdown",
    feature = "fmt-csv",
    feature = "fmt-xlsx",
    feature = "fmt-ods",
    feature = "fmt-pdf"
))]

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::time::{Duration, Instant};

use rezzer_model::{Completeness, LossReason, Registry};
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request, RunResult};

/// Counts the heap of the thread that allocates, so tests that run side by side do not
/// disturb a measurement. The pipeline runs on the calling thread. The memory itself comes
/// from the allocator of the Rezzer binary, so the RSS these tests measure is the RSS it ships
/// with.
struct Counting;

/// Where [`Counting`] takes its memory from.
const INNER: rezzer_limits::Allocator = rezzer_limits::Allocator;

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
        unsafe { INNER.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        shrink(l.size());
        unsafe { INNER.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        shrink(l.size());
        grow(new);
        unsafe { INNER.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns its result with the peak heap it added on this thread, in bytes.
/// The measure covers the calling thread only: memory that a reader allocates on another
/// thread is not seen. Every reader runs on the calling thread today.
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
    registry.register(Box::new(rezzer_read_pdf::PdfReader));
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

/// Four sheets, three of them big (20,000 rows of numbers each), and their unpacked size.
fn big_book() -> (Vec<u8>, u64) {
    let big = worksheet("", &number_rows(20_000), "");
    let sheets = [
        ("Small", small_worksheet()),
        ("Big1", big.clone()),
        ("Big2", big.clone()),
        ("Big3", big),
    ];
    let unpacked: usize = sheets.iter().map(|(_, xml)| xml.len()).sum();
    (xlsx(&sheets), unpacked as u64)
}

fn sheet_pipeline_with_unpacked(limit: u64) -> Pipeline {
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    config.limits.max_unpacked_bytes = limit;
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    Pipeline::new(config, registry)
}

#[test]
fn xlsx_unpacked_limit_counts_each_part_once() {
    // The limit is the real unpacked size of the package plus a little room for its small
    // parts. The readers read some parts more than once; that must not count again.
    let (book, unpacked) = big_book();
    let p = sheet_pipeline_with_unpacked(unpacked + 64 * 1024);
    let (result, peak, took) = measured(&p, "big.xlsx", book);
    eprintln!(
        "xlsx_unpacked_limit_counts_each_part_once: peak heap {} MB in {took:?}: {:?}",
        peak / MB,
        losses(&result)
    );
    assert_eq!(
        result.completeness,
        Completeness::Complete,
        "{:?}",
        losses(&result)
    );
    let md = result.markdown_pages.join("");
    assert!(md.contains("## Small") && md.contains("## Big3"), "{md}");
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
}

#[test]
fn xlsx_over_unpacked_limit_is_refused() {
    // Every entry fits the ratio, but together the sheets unpack to more than the limit.
    // The package check measures every entry first, so nothing is read and the loss says
    // why.
    let (book, unpacked) = big_book();
    let p = sheet_pipeline_with_unpacked(unpacked - 1);
    let (result, peak, took) = measured(&p, "bomb.xlsx", book);
    eprintln!(
        "xlsx_over_unpacked_limit_is_refused: peak heap {} MB in {took:?}: {:?}",
        peak / MB,
        losses(&result)
    );
    assert_eq!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.starts_with("Limit") && l.contains("unpacked")),
        "{:?}",
        losses(&result)
    );
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
}

#[test]
fn xlsx_zip_bomb_is_refused() {
    // One sheet of 64 MB of spaces packs into well under a megabyte: far over the 100:1
    // ratio. The read stops before the sheet is unpacked and says why.
    let mut sheet = format!(
        "<worksheet xmlns=\"{MAIN_NS}\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>BOMB"
    );
    sheet.extend(std::iter::repeat_n(' ', 64 * MB));
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
    // Two regions, one per cell: the empty rows and columns between them part them, and no
    // grid of the used area is built.
    let ranges: Vec<&str> = doc
        .body
        .iter()
        .flat_map(|b| match b {
            rezzer_model::Block::Section { blocks, .. } => blocks.as_slice(),
            _ => &[],
        })
        .filter_map(|b| match b {
            rezzer_model::Block::Table {
                layout: Some(l), ..
            } => Some(l.range.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ranges, ["A1", "XFD1048576"], "{:?}", losses(&result));
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
    // ODS content is checked as a whole before any parser reads it: too deep, it is
    // refused, and the loss names the depth limit as the reason.
    assert_eq!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(depth_loss(&result), "{:?}", losses(&result));
    assert!(
        !losses(&result).iter().any(|l| l.contains("read two ways")),
        "{:?}",
        losses(&result)
    );
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

#[test]
fn ods_content_cut_at_a_limit_names_that_limit() {
    // Two sheets of 20 rows; the element limit stops the ODF part reader inside the first
    // one. The read is refused, and the reason is the limit, not a sheet name mismatch.
    let mut body = String::new();
    for s in ["A", "B"] {
        body.push_str(&format!("<table:table table:name=\"{s}\">"));
        for _ in 0..20 {
            body.push_str("<table:table-row><table:table-cell office:value-type=\"float\" office:value=\"5\"><text:p>5</text:p></table:table-cell></table:table-row>");
        }
        body.push_str("</table:table>");
    }
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    config.limits.max_elements = 40;
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    let p = Pipeline::new(config, registry);
    let result = p.run_bytes(
        "cut.ods",
        ods(&ods_content("", &body)),
        None,
        &markdown_request(),
    );
    assert_eq!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.starts_with("Limit document") && l.contains("more than 40 items")),
        "{:?}",
        losses(&result)
    );
    assert!(
        !losses(&result).iter().any(|l| l.contains("read two ways")),
        "{:?}",
        losses(&result)
    );
}

// Hostile sheet layouts: many merges, many tiny regions, one merge over the whole sheet. The
// layout works on the sparse cells and on merge intervals, never on a grid of the used area.
// Work is bounded by the CPU time of the calling thread (every sheet reader runs on it), so a
// busy machine does not fail the test; memory by the heap peak of that thread, with the RSS
// of the process as a loose guard.

/// The CPU time the calling thread has used.
#[cfg(unix)]
fn thread_cpu() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills the timespec we pass.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    assert_eq!(r, 0, "clock_gettime");
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

/// No thread CPU clock here: the time bounds are not checked (see their `cfg(unix)`).
#[cfg(not(unix))]
fn thread_cpu() -> Duration {
    Duration::ZERO
}

/// The work bound of the layout tests: the whole read, layout included, in thread CPU time.
#[cfg(unix)]
const LAYOUT_BOUND: Duration = Duration::from_secs(2);

/// Runs `input` (the bytes, or the path of a file) as `name` and returns the result, its peak
/// heap and the thread CPU time it used.
fn layout_run(
    name: &str,
    input: Result<Vec<u8>, std::path::PathBuf>,
) -> (RunResult, usize, Duration) {
    let p = sheet_pipeline();
    let start = thread_cpu();
    let (result, peak) = peak_heap(|| match input {
        Ok(bytes) => p.run_bytes(name, bytes, None, &markdown_request()),
        Err(path) => p.run_path(&path, &markdown_request()),
    });
    (result, peak, thread_cpu() - start)
}

/// The layouts of every table block in the document, in body order.
fn layouts(result: &RunResult) -> Vec<&rezzer_model::TableLayout> {
    result
        .document
        .as_ref()
        .map(|d| d.body.as_slice())
        .unwrap_or(&[])
        .iter()
        .flat_map(|b| match b {
            rezzer_model::Block::Section { blocks, .. } => blocks.as_slice(),
            _ => &[],
        })
        .filter_map(|b| match b {
            rezzer_model::Block::Table {
                layout: Some(l), ..
            } => Some(l.as_ref()),
            _ => None,
        })
        .collect()
}

/// Column letters of a 0-based column: 0 is A, 26 is AA.
fn column_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

#[test]
fn many_merges_without_values_is_bounded() {
    // POI's bug 57893: 50,000 two-cell merges over empty styled cells and no value at all.
    // The merges are occupied areas for the layout; they cost by their count, not their area.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office/malformed/57893-many-merges.xlsx");
    let (result, peak, used) = layout_run("57893-many-merges.xlsx", Err(path));
    eprintln!(
        "many_merges: peak heap {} MB, {used:?} CPU: {:?}",
        peak / MB,
        losses(&result)
    );
    assert!(
        matches!(
            result.completeness,
            Completeness::Complete | Completeness::Partial
        ),
        "{:?}",
        losses(&result)
    );
    // The thread CPU time is measured on unix only.
    #[cfg(unix)]
    assert!(used < LAYOUT_BOUND, "took {used:?} CPU");
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(result.markdown_pages.join("").contains("## Sheet1"));
    #[cfg(unix)]
    assert_rss_bounded("many_merges_without_values_is_bounded");
}

#[test]
fn checkerboard_sheet_hits_region_limit() {
    // 40,000 single cells, one wherever row plus column is even: no row or column is empty,
    // every cell touches the next only at a corner, so each would be a region of its own.
    // Past 10,000 regions the rest of the sheet stays one grid with a Limit loss.
    let (rows, cols) = (400u32, 200u32);
    let mut xml = String::new();
    for r in 0..rows {
        xml.push_str(&format!("<row r=\"{}\">", r + 1));
        for c in (r % 2..cols).step_by(2) {
            xml.push_str(&format!(
                "<c r=\"{}{}\"><v>{}</v></c>",
                column_letters(c),
                r + 1,
                (r * cols + c) % 97
            ));
        }
        xml.push_str("</row>");
    }
    let book = xlsx(&[("Board", worksheet("", &xml, ""))]);
    let (result, peak, used) = layout_run("board.xlsx", Ok(book));
    let regions = layouts(&result);
    eprintln!(
        "checkerboard: {} regions, peak heap {} MB, {used:?} CPU: {:?}",
        regions.len(),
        peak / MB,
        losses(&result)
    );
    assert_ne!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    // The thread CPU time is measured on unix only.
    #[cfg(unix)]
    assert!(used < LAYOUT_BOUND, "took {used:?} CPU");
    assert!(peak < 512 * MB, "peak heap {peak} bytes");
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.starts_with("Limit") && l.contains("region limit")),
        "{:?}",
        losses(&result)
    );
    let grids: Vec<_> = regions
        .iter()
        .filter(|l| l.kind == rezzer_model::LayoutKind::Grid)
        .collect();
    assert_eq!(grids.len(), 1, "{:?}", losses(&result));
    // The rest grid holds the cells past the limit: it ends at the last row of the board.
    assert!(
        grids[0].range.ends_with(&rows.to_string()),
        "{}",
        grids[0].range
    );
}

#[test]
fn one_huge_merge_is_bounded() {
    // One merge over the whole sheet, A1:XFD1048576, with a value at its anchor and one
    // under it. The merge is one interval per axis; no cell of its 17 billion is made.
    let sheet = worksheet(
        "<dimension ref=\"A1:XFD1048576\"/>",
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Anchor</t></is></c></row><row r=\"3\"><c r=\"C3\"><v>42</v></c></row>",
        "<mergeCells count=\"1\"><mergeCell ref=\"A1:XFD1048576\"/></mergeCells>",
    );
    let (result, peak, used) = layout_run("merge.xlsx", Ok(xlsx(&[("M", sheet)])));
    eprintln!(
        "one_huge_merge: peak heap {} MB, {used:?} CPU: {:?}",
        peak / MB,
        losses(&result)
    );
    assert_ne!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    // The thread CPU time is measured on unix only.
    #[cfg(unix)]
    assert!(used < LAYOUT_BOUND, "took {used:?} CPU");
    assert!(peak < 64 * MB, "peak heap {peak} bytes");
    let md = result.markdown_pages.join("");
    assert!(md.contains("Anchor"), "{md}");
    // The value under the merge is not shown, and the output says so.
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.contains("M!C3") && l.contains("under the merged area A1:XFD1048576")),
        "{:?}",
        losses(&result)
    );
}

// Hostile PDFs. The files are written by hand (a classic cross-reference table, or a
// cross-reference stream where an object stream needs one). The PDF reader works on more than
// one thread, so memory is measured as the maximum resident set size of the process, not as
// the heap of the calling thread.

/// Maximum resident set size of the process. It is the figure of one test only under
/// nextest, which runs every test in its own process; under `cargo test` it covers every
/// test that ran before in the same process.
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

/// The bound every hostile PDF test holds.
#[cfg(unix)]
const RSS_BOUND: u64 = 512 * MB as u64;

/// Whether the test runs under nextest, which runs every test in its own process. Only then
/// is the process's maximum RSS the figure of one test; under `cargo test` the RSS checks are
/// skipped (and say so) rather than fail on the tests that ran before.
#[cfg(unix)]
fn own_process(test: &str) -> bool {
    let own = std::env::var_os("NEXTEST").is_some();
    if !own {
        eprintln!("{test}: RSS not checked, the test needs nextest (one process per test)");
    }
    own
}

#[cfg(unix)]
fn assert_rss_bounded(test: &str) {
    if !own_process(test) {
        return;
    }
    let rss = max_rss_bytes();
    eprintln!("{test}: max RSS {} MiB", rss >> 20);
    assert!(rss < RSS_BOUND, "{test}: max RSS {} MiB", rss >> 20);
}

/// A PDF being written: object `n` is `objects[n - 1]`.
#[derive(Default)]
struct PdfFile {
    objects: Vec<Vec<u8>>,
}

impl PdfFile {
    fn reserve(&mut self) -> u32 {
        self.objects.push(Vec::new());
        self.objects.len() as u32
    }

    fn set(&mut self, n: u32, body: impl Into<Vec<u8>>) {
        self.objects[n as usize - 1] = body.into();
    }

    fn add(&mut self, body: impl Into<Vec<u8>>) -> u32 {
        let n = self.reserve();
        self.set(n, body);
        n
    }

    fn stream(&mut self, dict: &str, data: &[u8]) -> u32 {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend(data);
        body.extend(b"\nendstream");
        self.add(body)
    }

    /// The file with a classic cross-reference table.
    fn finish(self, root: u32) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend(body);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

/// A one-page A4 file: Helvetica as `/F1`, `content` as the page's content stream (already
/// packed when `dict` names a filter), `catalog_extra` in the catalog and `build` adding
/// objects and returning more entries for the page's resources.
#[cfg(unix)]
fn pdf_page(
    content: &[u8],
    dict: &str,
    catalog_extra: &str,
    build: impl FnOnce(&mut PdfFile) -> String,
) -> Vec<u8> {
    let mut b = PdfFile::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let resources = build(&mut b);
    let c = b.stream(dict, content);
    let page = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> {resources} >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R {catalog_extra} >>"),
    );
    b.finish(catalog)
}

#[cfg(unix)]
const HELLO: &[u8] = b"BT /F1 12 Tf 72 720 Td (Before the bomb.) Tj ET";

/// `n` bytes of `fill`, packed with zlib in chunks, so building the file never holds them.
#[cfg(unix)]
fn packed_fill(fill: u8, n: usize) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let chunk = vec![fill; MB];
    let mut left = n;
    while left > 0 {
        let k = left.min(MB);
        z.write_all(&chunk[..k]).unwrap();
        left -= k;
    }
    z.finish().unwrap()
}

/// Runs a PDF through the pipeline (default limits unless `max_time`) and returns the result
/// and its time.
#[cfg(unix)]
fn read_pdf(
    bytes: Vec<u8>,
    max_time: Option<Duration>,
    request: &Request,
) -> (RunResult, Duration) {
    let p = pipeline_with(max_time);
    let start = Instant::now();
    let result = p.run_bytes("hostile.pdf", bytes, None, request);
    (result, start.elapsed())
}

#[cfg(unix)]
#[test]
fn objstm_bomb_is_bounded() {
    // An object stream of about 1 MB that inflates to 1 GiB. Object 6 lives in it and the
    // catalog points to it, so a reader that trusts the file must unpack the stream. The file
    // uses a cross-reference stream (type 2 entries name the object stream).
    let packed = packed_fill(0, 1 << 30);
    assert!(packed.len() < 2 * MB, "{}", packed.len());
    let content = HELLO;
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Bomb 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
          /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> \
          /Contents 4 0 R >>"
            .to_vec(),
    ];
    let mut c = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    c.extend(content);
    c.extend(b"\nendstream");
    objects.push(c);
    let mut objstm = format!(
        "<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length {} >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    objstm.extend(&packed);
    objstm.extend(b"\nendstream");
    objects.push(objstm);
    drop(packed);
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    // Object 7 is the cross-reference stream: entries 0 to 7, 1 + 4 + 2 bytes each.
    let xref_at = out.len();
    let mut rows = vec![0u8, 0, 0, 0, 0, 0xff, 0xff];
    for o in &offsets {
        rows.push(1);
        rows.extend((*o as u32).to_be_bytes());
        rows.extend([0, 0]);
    }
    rows.extend([2, 0, 0, 0, 5, 0, 0]);
    rows.push(1);
    rows.extend((xref_at as u32).to_be_bytes());
    rows.extend([0, 0]);
    out.extend(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            rows.len()
        )
        .as_bytes(),
    );
    out.extend(&rows);
    out.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());

    let (result, took) = read_pdf(out, None, &markdown_request());
    eprintln!(
        "objstm_bomb: {:?} in {took:?}: {:?}",
        result.completeness,
        losses(&result)
    );
    // An object stream belongs to the file's structure: the whole file is refused.
    assert_eq!(
        result.completeness,
        Completeness::Failed,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result).iter().any(|l| l.starts_with("Limit")
            && l.contains("an object stream or cross-reference stream unpacks to more than")),
        "{:?}",
        losses(&result)
    );
    assert!(took < Duration::from_secs(120) + SLACK, "took {took:?}");
    assert_rss_bounded("objstm_bomb_is_bounded");
}

#[cfg(unix)]
#[test]
fn content_stream_bomb_is_bounded() {
    // A page whose content stream of about 1 MB inflates to 1 GiB of spaces; a second page is
    // plain. The bomb page is refused, the plain page is read.
    let packed = packed_fill(b' ', 1 << 30);
    let mut b = PdfFile::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let bomb = b.stream("/Filter /FlateDecode", &packed);
    drop(packed);
    let plain = b.stream("", b"BT /F1 12 Tf 72 720 Td (The plain page.) Tj ET");
    let mut kids = String::new();
    for c in [bomb, plain] {
        let page = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
        ));
        kids.push_str(&format!("{page} 0 R "));
    }
    b.set(tree, format!("<< /Type /Pages /Kids [{kids}] /Count 2 >>"));
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let (result, took) = read_pdf(b.finish(catalog), None, &markdown_request());
    eprintln!(
        "content_stream_bomb: {:?} in {took:?}: {:?}",
        result.completeness,
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
            .any(|l| l.starts_with("Limit p1 the page is not read")
                && l.contains("unpacks to more than 100 times its size")),
        "{:?}",
        losses(&result)
    );
    assert!(result.markdown_pages.join("").contains("The plain page."));
    assert!(took < Duration::from_secs(120) + SLACK, "took {took:?}");
    assert_rss_bounded("content_stream_bomb_is_bounded");
}

#[cfg(unix)]
#[test]
fn page_tree_cycle_is_partial() {
    // The page tree lists itself as its own kid.
    let mut b = PdfFile::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let c = b.stream("", b"BT /F1 12 Tf 72 720 Td (In the loop.) Tj ET");
    let page = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{page} 0 R {tree} 0 R] /Count 2 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let (result, took) = read_pdf(b.finish(catalog), None, &markdown_request());
    eprintln!(
        "page_tree_cycle: {:?} in {took:?}: {:?}",
        result.completeness,
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
            .any(|l| l.starts_with("Damaged") && l.contains("cycle")),
        "{:?}",
        losses(&result)
    );
    // Other parsers could number the pages of such a tree differently, so no page text is
    // read, and the output says so.
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.contains("text not read on any page")),
        "{:?}",
        losses(&result)
    );
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_rss_bounded("page_tree_cycle_is_partial");
}

/// Writes a file of `pages` pages to `path`, built in memory and dropped before the read so
/// that its bytes are not part of the measure. With `own`, every page has its own resources,
/// media box and content stream of one line (`Page N`); else the pages inherit their
/// resources and media box and share one content stream of 40 lines (`Line K of a page`).
fn many_pages(path: &std::path::Path, pages: usize, own: bool) {
    use std::io::Write;
    let mut b = PdfFile::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut text = String::from("BT /F1 10 Tf 72 780 Td 14 TL ");
    for line in 0..40 {
        text.push_str(&format!(
            "(Line {line} of a page that is shown again and again.) ' "
        ));
    }
    text.push_str("ET");
    let shared = b.stream("", text.as_bytes());
    let mut kids = String::with_capacity(pages * 10);
    for i in 0..pages {
        let page = if own {
            let c = b.stream(
                "",
                format!("BT /F1 12 Tf 72 720 Td (Page {i}) Tj ET").as_bytes(),
            );
            b.add(format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
            ))
        } else {
            b.add(format!(
                "<< /Type /Page /Parent {tree} 0 R /Contents {shared} 0 R >>"
            ))
        };
        kids.push_str(&format!("{page} 0 R "));
    }
    b.set(
        tree,
        format!(
            "<< /Type /Pages /MediaBox [0 0 595 842] /Resources << /Font << /F1 {font} 0 R >> >> \
             /Kids [{kids}] /Count {pages} >>"
        ),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    let mut out = std::fs::File::create(path).unwrap();
    out.write_all(&b.finish(catalog)).unwrap();
}

/// A pipeline with the PDF reader only, under `memory` bytes of memory and `time`.
#[cfg(unix)]
fn pdf_pipeline(memory: u64, time: Duration) -> Pipeline {
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    config.limits.max_memory_bytes = memory;
    config.limits.max_time = time;
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_pdf::PdfReader));
    Pipeline::new(config, registry)
}

/// The peak RSS of the test stays under the memory limit the read was given.
#[cfg(unix)]
fn assert_rss_under(test: &str, limit: u64) {
    if !own_process(test) {
        return;
    }
    let rss = max_rss_bytes();
    eprintln!(
        "{test}: max RSS {} MiB, limit {} MiB",
        rss >> 20,
        limit >> 20
    );
    assert!(rss < limit, "{test}: max RSS {} MiB", rss >> 20);
}

/// The pages of a page section with text besides its marker.
#[cfg(unix)]
fn pages_with_text(result: &RunResult) -> usize {
    result.document.as_ref().map_or(0, |d| {
        d.body
            .iter()
            .filter(
                |b| matches!(b, rezzer_model::Block::Section { blocks, .. } if blocks.len() > 1),
            )
            .count()
    })
}

#[cfg(unix)]
#[test]
fn hundred_thousand_pages_stop_in_time() {
    // 100,000 pages that share one content stream of 40 lines, far more than 2 s of work, at
    // the default memory limit. The pages read before the stop keep their text, and one loss
    // names the first page without it.
    const PAGES: usize = 100_000;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages.pdf");
    many_pages(&path, PAGES, false);
    let limit = Duration::from_secs(2);
    let p = pipeline_with(Some(limit));
    let start = Instant::now();
    let result = p.run_path(&path, &markdown_request());
    let took = start.elapsed();
    eprintln!(
        "hundred_thousand_pages: {:?} in {took:?}: {:?}",
        result.completeness,
        losses(&result)
    );
    assert!(took < limit + SLACK, "took {took:?}");
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        losses(&result)
    );
    let read = pages_with_text(&result);
    assert!(read > 0 && read < PAGES, "{read} pages with text");
    let md = result.markdown_pages.join("");
    assert!(md.contains("Line 39 of a page"));
    let timed: Vec<String> = losses(&result)
        .into_iter()
        .filter(|l| l.contains("time limit"))
        .collect();
    assert_eq!(
        timed,
        [format!(
            "Limit p{} text of pages {} to {PAGES} not read: the time limit was near, and the \
             pages read before are laid out in the time left",
            read + 1,
            read + 1
        )]
    );
    assert_rss_bounded("hundred_thousand_pages_stop_in_time");
}

/// The files in the cache dir `dir`.
fn cache_entries(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| entries.map(|e| e.unwrap().path()).collect(),
    )
}

#[test]
fn a_text_cut_near_the_time_limit_is_not_cached() {
    // The PDF reader stops its text at three quarters of the time limit, before the budget's
    // own time is over. That cut depends on the machine's load, so it is not cached.
    const PAGES: usize = 20_000;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages.pdf");
    many_pages(&path, PAGES, false);
    let cache = dir.path().join("cache");
    let mut config = Config {
        cache: true,
        cache_dir: Some(cache.clone()),
        ..Config::default()
    };
    config.limits.max_time = Duration::from_secs(1);
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_pdf::PdfReader));
    let p = Pipeline::new(config, registry);
    let result = p.run_path(&path, &markdown_request());
    let doc = result.document.as_ref().unwrap();
    let cut = doc
        .losses
        .iter()
        .find(|l| l.what.contains("the time limit was near"))
        .unwrap_or_else(|| panic!("{:?}", losses(&result)));
    assert_eq!(cut.why, LossReason::Limit);
    assert_eq!(result.completeness, Completeness::Partial);
    assert!(
        cache_entries(&cache).is_empty(),
        "{:?}",
        cache_entries(&cache)
    );
}

#[cfg(unix)]
#[test]
fn pages_with_their_own_resources_are_read_under_the_memory_limit() {
    // 15,000 pages, each with its own resources, media box and content stream, at a memory
    // limit of 256 MiB: the objects and the text parser's page list fit in the object budget,
    // every page is read, and the process stays under the limit.
    const PAGES: usize = 15_000;
    let limit: u64 = 256 << 20;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages.pdf");
    many_pages(&path, PAGES, true);
    let p = pdf_pipeline(limit, Duration::from_secs(120));
    let result = p.run_path(&path, &markdown_request());
    eprintln!(
        "own resources: {:?}: {:?}",
        result.completeness,
        losses(&result)
    );
    assert_eq!(pages_with_text(&result), PAGES, "{:?}", losses(&result));
    let md = result.markdown_pages.join("");
    assert!(md.contains("Page 14999"));
    assert_rss_under(
        "pages_with_their_own_resources_are_read_under_the_memory_limit",
        limit,
    );
}

#[cfg(unix)]
#[test]
fn hundred_thousand_pages_stay_under_the_memory_limit() {
    // 100,000 pages with their own content streams at a memory limit of 512 MiB. The objects
    // take almost all of the object budget; the text parser's list of the pages (about 2 KB a
    // page) does not fit beside them, so the text is not read and the output says why,
    // instead of going over the limit.
    const PAGES: usize = 100_000;
    let limit: u64 = 512 << 20;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages.pdf");
    {
        // The pages inherit their resources and media box, so the objects fit the budget.
        use std::io::Write;
        let mut b = PdfFile::default();
        let catalog = b.reserve();
        let tree = b.reserve();
        let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        let mut kids = String::with_capacity(PAGES * 10);
        for i in 0..PAGES {
            let c = b.stream(
                "",
                format!("BT /F1 12 Tf 72 720 Td (Page {i}) Tj ET").as_bytes(),
            );
            let page = b.add(format!(
                "<< /Type /Page /Parent {tree} 0 R /Contents {c} 0 R >>"
            ));
            kids.push_str(&format!("{page} 0 R "));
        }
        b.set(
            tree,
            format!(
                "<< /Type /Pages /MediaBox [0 0 595 842] /Resources << /Font << /F1 {font} 0 R >> >> \
                 /Kids [{kids}] /Count {PAGES} >>"
            ),
        );
        b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
        let mut out = std::fs::File::create(&path).unwrap();
        out.write_all(&b.finish(catalog)).unwrap();
    }
    let p = pdf_pipeline(limit, Duration::from_secs(120));
    let result = p.run_path(&path, &markdown_request());
    eprintln!(
        "page list: {:?}: {:?}",
        result.completeness,
        losses(&result)
    );
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        losses(&result)
    );
    assert!(
        losses(&result).iter().any(|l| l
            .starts_with("Limit p1 text of pages 1 to 100000 not read")
            && l.contains("the text parser's list of the 100000 pages")),
        "{:?}",
        losses(&result)
    );
    assert_rss_under("hundred_thousand_pages_stay_under_the_memory_limit", limit);
}

#[cfg(unix)]
#[test]
fn image_pixel_bomb_skips_render() {
    // A page covered by a picture of 40,000 by 40,000 one-bit pixels from one row of packed
    // data: 200 MB as bits, 1.6 GB as the bytes a renderer decodes. The page needs vision.
    // Pictures are costed as decoded bytes against the room of one render (a quarter of the
    // memory limit, 512 MiB by default), so the render is skipped with a Limit loss and no
    // PNG is written.
    let image = |b: &mut PdfFile| {
        let n = b.stream(
            "/Type /XObject /Subtype /Image /Width 40000 /Height 40000 \
             /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode",
            &packed_fill(0, 5000),
        );
        format!("/XObject << /Im1 {n} 0 R >>")
    };
    let bytes = pdf_page(b"q 595 0 0 842 0 0 cm /Im1 Do Q", "", "", image);
    let dir = tempfile::tempdir().unwrap();
    let request = Request {
        output_dir: Some(dir.path().to_path_buf()),
        ..markdown_request()
    };
    let (result, took) = read_pdf(bytes, None, &request);
    eprintln!(
        "image_pixel_bomb: {:?} in {took:?}: {:?}",
        result.completeness,
        losses(&result)
    );
    let doc = result.document.as_ref().expect("a document");
    assert!(
        doc.assets
            .iter()
            .any(|a| a.id == "page-1" && a.needs_vision),
        "{:?}",
        doc.assets
    );
    assert!(
        doc.losses.iter().any(|l| l.location == "p1"
            && l.why == LossReason::Limit
            && l.what.contains("not rendered")
            && l.what.contains("pictures")),
        "{:?}",
        losses(&result)
    );
    let bundle = dir.path().join("hostile.pdf.assets");
    let written = std::fs::read_dir(&bundle).map(|d| d.count()).unwrap_or(0);
    assert_eq!(written, 0, "no render is written");
    assert!(took < Duration::from_secs(60), "took {took:?}");
    assert_rss_bounded("image_pixel_bomb_skips_render");
}

#[cfg(unix)]
#[test]
fn deep_array_nesting_stops_at_depth() {
    // 100,000 nested arrays in an object the catalog points to.
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let bytes = pdf_page(HELLO, "", "/Deep 4 0 R", |b| {
        let n = b.add(deep);
        assert_eq!(n, 4);
        String::new()
    });
    let (result, took) = read_pdf(bytes, None, &markdown_request());
    eprintln!(
        "deep_array_nesting: {:?} in {took:?}: {:?}",
        result.completeness,
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
            .any(|l| l == "Damaged object 4 0 values are nested too deeply"),
        "{:?}",
        losses(&result)
    );
    // The page does not depend on the deep object and is read.
    assert!(result.markdown_pages.join("").contains("Before the bomb."));
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_rss_bounded("deep_array_nesting_stops_at_depth");
}

#[cfg(unix)]
#[test]
fn huge_script_is_bounded() {
    // A document script of 64 MiB in the JavaScript name tree. Its text varies enough that it
    // packs at less than 100:1, so the stream guard lets it pass and the script cap must hold:
    // a script over its cap is listed without its text and is never unpacked whole.
    let packed = {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut x: u64 = 12345;
        let mut line = Vec::with_capacity(64);
        let mut written = 0;
        while written < 64 * MB {
            line.clear();
            line.extend_from_slice(b"app.alert(\"");
            for _ in 0..24 {
                x = x
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                line.push(b'a' + (x >> 59) as u8 % 26);
            }
            line.extend_from_slice(b"\");\n");
            z.write_all(&line).unwrap();
            written += line.len();
        }
        z.finish().unwrap()
    };
    assert!(packed.len() > 64 * MB / 100, "{}", packed.len());
    let bytes = pdf_page(HELLO, "", "/Names << /JavaScript 4 0 R >>", |b| {
        let names = b.reserve();
        assert_eq!(names, 4);
        let js = b.stream("/Filter /FlateDecode", &packed);
        let action = b.add(format!("<< /S /JavaScript /JS {js} 0 R >>"));
        b.set(names, format!("<< /Names [(Huge) {action} 0 R] >>"));
        String::new()
    });
    let (result, took) = read_pdf(bytes, None, &markdown_request());
    eprintln!(
        "huge_script: {:?} in {took:?}: {:?}",
        result.completeness,
        losses(&result)
    );
    let doc = result.document.as_ref().expect("a document");
    assert_eq!(doc.properties.facts.has_javascript, Some(true));
    let scripts: Vec<_> = doc
        .facets
        .embedded
        .iter()
        .filter(|e| e.kind == "script")
        .collect();
    assert_eq!(scripts.len(), 1, "{scripts:?}");
    assert_eq!(scripts[0].text, None);
    assert!(
        losses(&result)
            .iter()
            .any(|l| l.starts_with("Limit") && l.contains("script")),
        "{:?}",
        losses(&result)
    );
    let md = result.markdown_pages.join("");
    assert!(md.contains("Before the bomb."));
    assert!(!md.contains("app.alert("));
    assert!(took < Duration::from_secs(30), "took {took:?}");
    assert_rss_bounded("huge_script_is_bounded");
}
