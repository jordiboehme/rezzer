// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small PDF files written by hand for the tests, with a classic cross-reference table.

#![allow(dead_code)]

pub mod barfont;

use std::alloc::{GlobalAlloc, Layout};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Block, Bytes, Confidence, Detection, FormatId, ReadOptions, ReadOutcome, Reader, Source,
    plain_text,
};
use rezzer_read_pdf::PdfReader;

/// One page of a generated file.
#[derive(Default, Clone)]
pub struct Page {
    /// The content stream, uncompressed.
    pub content: String,
    /// Annotation dictionaries, each written as its own object.
    pub annots: Vec<String>,
    /// A stream that the page's font reads as its `/ToUnicode` CMap, already deflated.
    pub packed_cmap: Option<Vec<u8>>,
}

impl Page {
    pub fn text(content: impl Into<String>) -> Page {
        Page {
            content: content.into(),
            ..Page::default()
        }
    }
}

/// A file being written: object `n` is `objects[n - 1]`.
#[derive(Default)]
pub struct Builder {
    objects: Vec<Vec<u8>>,
}

impl Builder {
    /// Reserves the next object number.
    pub fn reserve(&mut self) -> u32 {
        self.objects.push(Vec::new());
        self.objects.len() as u32
    }

    pub fn set(&mut self, n: u32, body: impl Into<Vec<u8>>) {
        self.objects[n as usize - 1] = body.into();
    }

    pub fn add(&mut self, body: impl Into<Vec<u8>>) -> u32 {
        let n = self.reserve();
        self.set(n, body);
        n
    }

    pub fn stream(&mut self, dict: &str, data: &[u8]) -> u32 {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend(data);
        body.extend(b"\nendstream");
        self.add(body)
    }

    pub fn finish(self, root: u32) -> Vec<u8> {
        self.finish_with(root, "")
    }

    /// The file with `trailer_extra` (for example `/Info 5 0 R`) in its trailer.
    pub fn finish_with(self, root: u32, trailer_extra: &str) -> Vec<u8> {
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
                "trailer\n<< /Size {} /Root {root} 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

/// Appends an incremental update to `file`: the objects `(number, body)`, a classic table and
/// a trailer with `keys` (for example `/Size 9 /Root 1 0 R`) and `/Prev`.
pub fn append_update(file: &mut Vec<u8>, objects: &[(u32, Vec<u8>)], keys: &str) {
    let at = file
        .windows(9)
        .rposition(|w| w == b"startxref")
        .expect("startxref")
        + 9;
    let prev: String = file[at..]
        .iter()
        .skip_while(|c| c.is_ascii_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .map(|c| char::from(*c))
        .collect();
    if !file.ends_with(b"\n") {
        file.push(b'\n');
    }
    let mut offsets = Vec::new();
    for (n, body) in objects {
        offsets.push((*n, file.len()));
        file.extend(format!("{n} 0 obj\n").as_bytes());
        file.extend(body);
        file.extend(b"\nendobj\n");
    }
    let xref = file.len();
    file.extend(b"xref\n");
    for (n, off) in offsets {
        file.extend(format!("{n} 1\n{off:010} 00000 n \n").as_bytes());
    }
    file.extend(
        format!("trailer\n<< {keys} /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
}

/// A file with the given pages (A4, Helvetica as `/F1`, Helvetica-Bold as `/F2`).
/// `catalog_extra` goes into the catalog dictionary.
pub fn pdf(pages: &[Page], catalog_extra: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let bold = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let mut kids = Vec::new();
    for page in pages {
        let font = match &page.packed_cmap {
            Some(packed) => {
                let cmap = b.stream("/Filter /FlateDecode", packed);
                b.add(format!(
                    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode {cmap} 0 R >>"
                ))
            }
            None => b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
        };
        let content = b.stream("", page.content.as_bytes());
        let annots: Vec<String> = page
            .annots
            .iter()
            .map(|a| format!("{} 0 R", b.add(a.as_bytes())))
            .collect();
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> >> \
             /Contents {content} 0 R /Annots [{}] >>",
            annots.join(" ")
        ));
        kids.push(format!("{kid} 0 R"));
    }
    b.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        ),
    );
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R {catalog_extra} >>"),
    );
    b.finish(catalog)
}

/// `bytes` as the source `file.pdf`, detected as PDF for certain. This is the one `Source`
/// of the tests, so a new field of `Source` is set in one place.
pub fn source(bytes: &[u8]) -> Source {
    Source {
        path: None,
        name: "file.pdf".into(),
        bytes: Bytes::Owned(Arc::new(bytes.to_vec())),
        sha256: "00".repeat(32),
        size: bytes.len() as u64,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
    }
}

/// `bytes` read with `opts` under `budget`.
pub fn read_in(bytes: &[u8], opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
    PdfReader.read(&source(bytes), opts, budget)
}

/// `bytes` read with the default options and limits.
pub fn read(bytes: &[u8]) -> ReadOutcome {
    read_in(
        bytes,
        &ReadOptions::default(),
        &Budget::new(Limits::default()),
    )
}

/// The text of a block and the blocks inside it: one line per heading, paragraph or task, and
/// one line per table row with each cell followed by `|`.
pub fn block_text(block: &Block, out: &mut String) {
    match block {
        Block::Heading { inlines, .. }
        | Block::Paragraph { inlines, .. }
        | Block::Task { inlines, .. } => {
            out.push_str(&plain_text(inlines));
            out.push('\n');
        }
        Block::Section { blocks, .. }
        | Block::Quote { blocks, .. }
        | Block::Callout { blocks, .. } => blocks.iter().for_each(|b| block_text(b, out)),
        Block::List { items, .. } => items.iter().flatten().for_each(|b| block_text(b, out)),
        Block::Table { rows, .. } => {
            for row in rows {
                for cell in row {
                    out.push_str(&plain_text(&cell.inlines));
                    out.push('|');
                }
                out.push('\n');
            }
        }
        _ => {}
    }
}

/// The text of `blocks`, as [`block_text`] writes it.
pub fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    blocks.iter().for_each(|b| block_text(b, &mut out));
    out
}

/// The text of `blocks` without the folded callouts (what an agent reads as a page's body).
pub fn body_text(blocks: &[Block]) -> String {
    let kept: Vec<Block> = blocks
        .iter()
        .filter(|b| !matches!(b, Block::Callout { .. }))
        .cloned()
        .collect();
    text(&kept)
}

/// The text of the whole body of a read.
pub fn all_text(out: &ReadOutcome) -> String {
    text(&out.document.body)
}

/// Whether each of `order` stands in `text`, each after the one before it; else which one
/// does not.
pub fn in_order(text: &str, order: &[&str]) -> Result<(), String> {
    let mut from = 0;
    for t in order {
        let at = text[from..]
            .find(t)
            .ok_or_else(|| format!("no {t:?} after byte {from} in {text}"))?;
        from += at + t.len();
    }
    Ok(())
}

/// One line of the data layer of a print-run form.
#[derive(Clone)]
pub struct DataLine {
    pub y: f32,
    /// The line as the business system prints it, padding included.
    pub text: String,
}

/// One page of a print-run form: a template drawn as a Form XObject, then the data lines,
/// each moved into place by its own `cm`.
#[derive(Clone)]
pub struct FormPage {
    /// The content of the Form XObject drawn with `/Tpl Do` (labels, rules, boxes, marks).
    pub template: String,
    /// The x where every data line starts (40.0 on the forms this models).
    pub left: f32,
    /// The size of the data font (9.5 on the forms this models).
    pub size: f32,
    pub lines: Vec<DataLine>,
    /// Annotation dictionaries, each written as its own object.
    pub annots: Vec<String>,
    /// The data lines are set in Courier-Bold (`/FDB`), as some print runs set them.
    pub bold_data: bool,
    /// The whole page drawn this many times as large (the page box too), as a form printed
    /// larger or smaller.
    pub scale: f32,
    /// The whole page drawn turned a quarter turn counterclockwise on a landscape page box, as
    /// a form scanned or printed across.
    pub turn: bool,
    /// With `turn`, the page is turned a quarter turn clockwise instead.
    pub clockwise: bool,
    /// The page box before `scale`, in points: A4 (595 by 842) unless set.
    pub paper: (f32, f32),
    /// A picture of the page's own (`/Photo`, different on every page) drawn in this box
    /// (x, y, width, height), in the frame of the template, as the content of a page.
    pub photo: Option<[f32; 4]>,
}

impl Default for FormPage {
    fn default() -> FormPage {
        FormPage {
            template: String::new(),
            left: 40.0,
            size: 9.5,
            lines: Vec::new(),
            annots: Vec::new(),
            bold_data: false,
            scale: 1.0,
            turn: false,
            clockwise: false,
            paper: (595.0, 842.0),
            photo: None,
        }
    }
}

/// A file of print-run form pages, A4 unless `paper` says otherwise. The page and the
/// template have the fonts `/FL` (Helvetica, for labels), `/FD` (Courier, for data lines),
/// `/FB` (Helvetica-Bold) and `/FDB` (Courier-Bold, for data lines with `bold_data`), `/FT`
/// (Times-Roman) and `/FTB` (Times-Bold), all in WinAnsi; a template that shows one of the
/// bar fonts of [`barfont::BAR_FONTS`] gets them all. The template and
/// the data lines are written in that encoding, so labels may hold umlauts, `\u{df}` and the
/// euro sign. The page and the template share one picture, `/Logo` (4 by 4 gray pixels),
/// which [`logo`] draws. `info` holds the entries of the Info dictionary (for example `/Title (Lohn)`); with `""`
/// the file has none. `catalog_extra` goes into the catalog dictionary, where `{annots}`
/// stands for the references of every annotation of every page, in page order (for example
/// `/AcroForm << /Fields [{annots}] >>`).
pub fn print_form(pages: &[FormPage], info: &str, catalog_extra: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = |name: &str| {
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>")
    };
    let label = b.add(font("Helvetica"));
    let data = b.add(font("Courier"));
    let bold = b.add(font("Helvetica-Bold"));
    let bold_data = b.add(font("Courier-Bold"));
    let times = b.add(font("Times-Roman"));
    let times_bold = b.add(font("Times-Bold"));
    let logo = b.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
         /BitsPerComponent 8",
        &(0..16u8).map(|i| 30 + i * 9).collect::<Vec<u8>>(),
    );
    // Barcode fonts only for pages that show them (see [`barfont::BAR_FONTS`]).
    let codes = pages.iter().any(|p| {
        barfont::BAR_FONTS
            .iter()
            .any(|f| p.template.contains(&format!("/{} ", f.0)))
    });
    let mut bar_fonts = String::new();
    if codes {
        for (key, name, set, kind) in barfont::BAR_FONTS {
            let f = barfont::add_bar_font(&mut b, name, set, kind);
            bar_fonts += &format!(" /{key} {f} 0 R");
        }
    }
    let fonts = format!(
        "/Font << /FL {label} 0 R /FD {data} 0 R /FB {bold} 0 R /FDB {bold_data} 0 R \
         /FT {times} 0 R /FTB {times_bold} 0 R{bar_fonts} >>"
    );
    let mut kids = Vec::new();
    let mut all_annots = Vec::new();
    for (n, page) in pages.iter().enumerate() {
        let (pw, ph) = page.paper;
        let tpl = b.stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 {pw} {ph}] \
                 /Resources << {fonts} /XObject << /Logo {logo} 0 R >> >>"
            ),
            &win_ansi(&page.template),
        );
        let s = page.scale;
        let (w, h) = if page.turn {
            (ph * s, pw * s)
        } else {
            (pw * s, ph * s)
        };
        let mut content = if page.turn && page.clockwise {
            format!("q 0 {} {s} 0 0 {h} cm q /Tpl Do Q\n", -s)
        } else if page.turn {
            format!("q 0 {s} {} 0 {w} 0 cm q /Tpl Do Q\n", -s)
        } else {
            format!("q {s} 0 0 {s} 0 0 cm q /Tpl Do Q\n")
        };
        let mut xobjects = format!("/Tpl {tpl} 0 R /Logo {logo} 0 R");
        if let Some([x, y, dx, dy]) = page.photo {
            // Pixels of the page's own, so no two pages share the picture.
            let pixels: Vec<u8> = (0..16u8)
                .map(|i| (n as u8).wrapping_mul(17) ^ (i * 13))
                .collect();
            let photo = b.stream(
                "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
                 /BitsPerComponent 8",
                &pixels,
            );
            xobjects.push_str(&format!(" /Photo {photo} 0 R"));
            content.push_str(&format!("q {dx} 0 0 {dy} {x} {y} cm /Photo Do Q\n"));
        }
        for line in &page.lines {
            let line = data_line(page.left, line.y, page.size, &line.text);
            if page.bold_data {
                content.push_str(&line.replace("/FD ", "/FDB "));
            } else {
                content.push_str(&line);
            }
        }
        content.push_str("Q\n");
        let content = b.stream("", &win_ansi(&content));
        let annots: Vec<String> = page
            .annots
            .iter()
            .map(|a| format!("{} 0 R", b.add(a.as_bytes())))
            .collect();
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] \
             /Resources << {fonts} /XObject << {xobjects} >> >> \
             /Contents {content} 0 R /Annots [{}] >>",
            w,
            h,
            annots.join(" ")
        ));
        all_annots.extend(annots);
        kids.push(format!("{kid} 0 R"));
    }
    b.set(
        tree,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            kids.len()
        ),
    );
    let extra = catalog_extra.replace("{annots}", &all_annots.join(" "));
    b.set(
        catalog,
        format!("<< /Type /Catalog /Pages {tree} 0 R {extra} >>"),
    );
    if info.is_empty() {
        b.finish(catalog)
    } else {
        let info = b.add(format!("<< {info} >>"));
        b.finish_with(catalog, &format!("/Info {info} 0 R"))
    }
}

/// `content` in WinAnsi (Windows-1252), the encoding of the fonts of [`print_form`]. Panics
/// on a character WinAnsi has not, so a test never writes text the file cannot show.
pub fn win_ansi(content: &str) -> Vec<u8> {
    const HIGH: [(char, u8); 27] = [
        ('\u{20ac}', 0x80),
        ('\u{201a}', 0x82),
        ('\u{192}', 0x83),
        ('\u{201e}', 0x84),
        ('\u{2026}', 0x85),
        ('\u{2020}', 0x86),
        ('\u{2021}', 0x87),
        ('\u{2c6}', 0x88),
        ('\u{2030}', 0x89),
        ('\u{160}', 0x8a),
        ('\u{2039}', 0x8b),
        ('\u{152}', 0x8c),
        ('\u{17d}', 0x8e),
        ('\u{2018}', 0x91),
        ('\u{2019}', 0x92),
        ('\u{201c}', 0x93),
        ('\u{201d}', 0x94),
        ('\u{2022}', 0x95),
        ('\u{2013}', 0x96),
        ('\u{2014}', 0x97),
        ('\u{2dc}', 0x98),
        ('\u{2122}', 0x99),
        ('\u{161}', 0x9a),
        ('\u{203a}', 0x9b),
        ('\u{153}', 0x9c),
        ('\u{17e}', 0x9e),
        ('\u{178}', 0x9f),
    ];
    content
        .chars()
        .map(|c| match u32::from(c) {
            0..=0x7f | 0xa0..=0xff => u32::from(c) as u8,
            _ => HIGH
                .iter()
                .find(|(h, _)| *h == c)
                .map(|(_, b)| *b)
                .unwrap_or_else(|| panic!("{c:?} is not in WinAnsi")),
        })
        .collect()
}

/// `text` as a PDF literal string, with `\`, `(` and `)` escaped.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('(');
    for c in text.chars() {
        if matches!(c, '\\' | '(' | ')') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(')');
    out
}

/// One data line as a print run writes it: moved to `(x, y)` by `cm`, shown at the text
/// origin in Courier (`/FD`).
pub fn data_line(x: f32, y: f32, size: f32, text: &str) -> String {
    format!(
        "q 1 0 0 1 {x} {y} cm BT /FD {size} Tf 0 0 Td {} Tj ET Q\n",
        literal(text)
    )
}

/// A template label in Helvetica (`/FL`), scaled to 90 % and kerned in its middle, as form
/// templates set their small labels.
pub fn label(x: f32, y: f32, size: f32, text: &str) -> String {
    let mid = text
        .char_indices()
        .nth(text.chars().count() / 2)
        .map_or(text.len(), |(i, _)| i);
    let (first, second) = text.split_at(mid);
    format!(
        "BT /FL {size} Tf 90 Tz {x} {y} Td [{} -20 {}] TJ ET\n",
        literal(first),
        literal(second)
    )
}

/// The picture `/Logo` of [`print_form`] drawn `w` by `h` points with its lower left corner
/// at `(x, y)`, as a template draws its logos.
pub fn logo(x: f32, y: f32, w: f32, h: f32) -> String {
    format!("q {w} 0 0 {h} {x} {y} cm /Logo Do Q\n")
}

/// The ids of the assets that need vision.
pub fn needing(out: &ReadOutcome) -> Vec<&str> {
    out.document
        .assets
        .iter()
        .filter(|a| a.needs_vision)
        .map(|a| a.id.as_str())
        .collect()
}

/// The Markdown of a read.
pub fn markdown(out: &ReadOutcome) -> String {
    rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    )
}

/// A stroked box of `w` by `h` points with its lower left corner at `(x, y)`.
pub fn boxed(x: f32, y: f32, w: f32, h: f32) -> String {
    format!("{x} {y} {w} {h} re S\n")
}

/// A stroked horizontal rule of 0.5 points from `(x0, y)` to `(x1, y)`.
pub fn hrule(x0: f32, x1: f32, y: f32) -> String {
    format!("0.5 w {x0} {y} m {x1} {y} l S\n")
}

/// A drawn separator: a filled rectangle of 0.24 by 2 points at `(x, y)`, as boxed number
/// fields print their decimal and date separators.
pub fn mark(x: f32, y: f32) -> String {
    format!("{x} {y} 0.24 2 re f\n")
}

/// A text object that shows `text` with font `font` at size `size` at `(x, y)`.
pub fn show(font: &str, size: f32, x: f32, y: f32, text: &str) -> String {
    format!("BT /{font} {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// `n` bytes of zeros, deflated.
pub fn zeros_packed(n: usize) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let chunk = vec![0u8; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        z.write_all(&chunk[..k]).expect("deflate");
        left -= k;
    }
    z.finish().expect("deflate")
}

/// Bytes the test process holds on the heap now. The counters move only in a test binary
/// whose `#[global_allocator]` is [`Counting`]; elsewhere they stay at zero.
static LIVE: AtomicUsize = AtomicUsize::new(0);
/// The highest `LIVE` since the last `reset_heap_peak`. Nextest runs every test in its own
/// process, so one global peak is the figure of one test.
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(n: usize) {
    let now = LIVE.fetch_add(n, Ordering::Relaxed) + n;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

fn shrink(n: usize) {
    LIVE.fetch_sub(n, Ordering::Relaxed);
}

/// `rezzer_limits::Allocator`, counting the live bytes and their peak. A block that grows is
/// counted with its old and its new size until the call returns, as the allocator holds both
/// while it moves a large block; for a block grown in place that is more than it holds, so
/// the peak errs high, never low.
pub struct Counting;

// SAFETY: every call goes to `rezzer_limits::Allocator` unchanged; the counters only add and
// take away the sizes of the blocks.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let p = unsafe { rezzer_limits::Allocator.alloc(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let p = unsafe { rezzer_limits::Allocator.alloc_zeroed(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        shrink(layout.size());
        // SAFETY: forwarded unchanged.
        unsafe { rezzer_limits::Allocator.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        grow(new_size);
        // SAFETY: forwarded unchanged.
        let p = unsafe { rezzer_limits::Allocator.realloc(ptr, layout, new_size) };
        // On failure the old block stays and the new one never was.
        shrink(if p.is_null() { new_size } else { layout.size() });
        p
    }
}

/// Starts a new peak at what the heap holds now.
pub fn reset_heap_peak() {
    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// The heap peak since the last reset, or since the process started.
pub fn heap_peak_bytes() -> usize {
    PEAK.load(Ordering::Relaxed)
}

/// Runs `f` with a new heap peak and returns its result with that peak.
pub fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
    reset_heap_peak();
    let out = f();
    (out, heap_peak_bytes())
}

/// Maximum resident set size of the process. It is the figure of one test only under nextest,
/// which runs every test in its own process.
#[cfg(unix)]
pub fn max_rss_bytes() -> u64 {
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

#[cfg(not(unix))]
pub fn max_rss_bytes() -> u64 {
    0
}

/// One tall tagged page of `n` lines 40 points apart, `w0.` to `w<n-1>.`, each in its own
/// structure element: `kind` is `table` (one row of one cell per line), `list` (one item per
/// line) or `p`. With `uri`, one link with that URI lies over the page.
pub fn tagged_lines(n: usize, kind: &str, uri: Option<&str>) -> Vec<u8> {
    let gap = 40usize;
    let h = (n + 2) * gap;
    let mut c = b"BT /F1 12 Tf ".to_vec();
    for i in 0..n {
        c.extend(
            format!(
                "/P <</MCID {i}>> BDC 1 0 0 1 72 {} Tm (w{i}.) Tj EMC ",
                h - (i + 1) * gap
            )
            .as_bytes(),
        );
    }
    c.extend(b"ET\n");
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let kid = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let annots = match uri {
        Some(u) => {
            let a = b.add(format!(
                "<< /Type /Annot /Subtype /Link /Rect [0 0 595 {h}] /A << /S /URI /URI ({u}) >> >>"
            ));
            format!("/Annots [{a} 0 R]")
        }
        None => String::new(),
    };
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&c).expect("deflate");
    let cs = b.stream("/Filter /FlateDecode", &z.finish().expect("deflate"));
    let root = b.reserve();
    let doc = b.reserve();
    let mut leaves = Vec::with_capacity(n);
    let mut top = Vec::new();
    let refs = |ids: &[u32]| -> String { ids.iter().map(|r| format!("{r} 0 R ")).collect() };
    match kind {
        "table" | "list" => {
            let (group, row, leaf) = if kind == "table" {
                ("Table", "TR", "TD")
            } else {
                ("L", "LI", "LBody")
            };
            let parent = b.reserve();
            let mut rows = Vec::with_capacity(n);
            for i in 0..n {
                let r = b.reserve();
                let l = b.add(format!(
                    "<< /Type /StructElem /S /{leaf} /P {r} 0 R /Pg {kid} 0 R /K {i} >>"
                ));
                b.set(
                    r,
                    format!("<< /Type /StructElem /S /{row} /P {parent} 0 R /K [{l} 0 R] >>"),
                );
                rows.push(r);
                leaves.push(l);
            }
            b.set(
                parent,
                format!(
                    "<< /Type /StructElem /S /{group} /P {doc} 0 R /K [{}] >>",
                    refs(&rows)
                ),
            );
            top.push(parent);
        }
        _ => {
            for i in 0..n {
                let e = b.add(format!(
                    "<< /Type /StructElem /S /P /P {doc} 0 R /Pg {kid} 0 R /K {i} >>"
                ));
                leaves.push(e);
                top.push(e);
            }
        }
    }
    b.set(
        doc,
        format!(
            "<< /Type /StructElem /S /Document /P {root} 0 R /K [{}] >>",
            refs(&top)
        ),
    );
    b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {doc} 0 R /ParentTree << /Nums [0 [{}]] >> >>",
            refs(&leaves)
        ),
    );
    b.set(
        kid,
        format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 {h}] /StructParents 0 \
             /Resources << /Font << /F1 {font} 0 R >> >> {annots} /Contents {cs} 0 R >>"
        ),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

/// One tagged table of one row with `n` cells, cell `i` (text `w<i>`) on page `i + 1`.
pub fn wide_row(n: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let root = b.reserve();
    let doc = b.reserve();
    let table = b.reserve();
    let tr = b.reserve();
    let mut kids = Vec::new();
    let mut tds = Vec::new();
    let mut nums = String::new();
    for i in 0..n {
        let kid = b.reserve();
        let cs = b.stream(
            "",
            format!("/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (w{i}) Tj ET EMC").as_bytes(),
        );
        b.set(
            kid,
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents {i} \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
            ),
        );
        let td = b.add(format!(
            "<< /Type /StructElem /S /TD /P {tr} 0 R /Pg {kid} 0 R /K 0 >>"
        ));
        nums.push_str(&format!("{i} [{td} 0 R] "));
        kids.push(kid);
        tds.push(td);
    }
    let refs = |ids: &[u32]| -> String { ids.iter().map(|r| format!("{r} 0 R ")).collect() };
    b.set(
        tr,
        format!(
            "<< /Type /StructElem /S /TR /P {table} 0 R /K [{}] >>",
            refs(&tds)
        ),
    );
    b.set(
        table,
        format!("<< /Type /StructElem /S /Table /P {doc} 0 R /K [{tr} 0 R] >>"),
    );
    b.set(
        doc,
        format!("<< /Type /StructElem /S /Document /P {root} 0 R /K [{table} 0 R] >>"),
    );
    b.set(
        root,
        format!("<< /Type /StructTreeRoot /K {doc} 0 R /ParentTree << /Nums [{nums}] >> >>"),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", refs(&kids)),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

/// One tagged row of `c` cells over `p` pages: cell `i < p` has the word `w` on page `i + 1`,
/// the other cells have no content. Without `table` the same objects are a `Div` of `Div`s.
pub fn row_pages(p: usize, c: usize, table: bool) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let root = b.reserve();
    let doc = b.reserve();
    let group = b.reserve();
    let row = b.reserve();
    let mut kids = Vec::new();
    let mut cells = Vec::new();
    let mut nums = String::new();
    let cs = b.stream(
        "",
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (w) Tj ET EMC",
    );
    for i in 0..p {
        let kid = b.reserve();
        b.set(
            kid,
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] /StructParents {i} \
                 /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
            ),
        );
        let td = b.add(format!("<< /S /TD /P {row} 0 R /Pg {kid} 0 R /K 0 >>"));
        nums.push_str(&format!("{i} [{td} 0 R] "));
        kids.push(kid);
        cells.push(td);
    }
    for _ in p..c {
        cells.push(b.add(format!("<< /S /TD /P {row} 0 R >>")));
    }
    let refs = |ids: &[u32]| -> String { ids.iter().map(|r| format!("{r} 0 R ")).collect() };
    let (g, r) = if table {
        ("Table", "TR")
    } else {
        ("Div", "Div")
    };
    b.set(
        row,
        format!("<< /S /{r} /P {group} 0 R /K [{}] >>", refs(&cells)),
    );
    b.set(group, format!("<< /S /{g} /P {doc} 0 R /K [{row} 0 R] >>"));
    b.set(
        doc,
        format!("<< /S /Document /P {root} 0 R /K [{group} 0 R] >>"),
    );
    b.set(
        root,
        format!("<< /Type /StructTreeRoot /K {doc} 0 R /ParentTree << /Nums [{nums}] >> >>"),
    );
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{}] /Count {p} >>", refs(&kids)),
    );
    b.set(
        catalog,
        format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> \
             /StructTreeRoot {root} 0 R >>"
        ),
    );
    b.finish(catalog)
}

/// A deflate stream written bit by bit, with fixed Huffman codes (RFC 1951, 3.2.6), for
/// streams no encoder writes.
#[derive(Default)]
pub struct Deflate {
    out: Vec<u8>,
    acc: u64,
    bits: u32,
}

impl Deflate {
    /// A zlib stream: the two header bytes come first.
    pub fn zlib() -> Deflate {
        Deflate {
            out: vec![0x78, 0x9c],
            ..Deflate::default()
        }
    }

    /// `len` bits of `v`, least significant first.
    pub fn bits(&mut self, v: u32, len: u32) {
        self.acc |= u64::from(v) << self.bits;
        self.bits += len;
        while self.bits >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.bits -= 8;
        }
    }

    /// A Huffman code of `len` bits, most significant first.
    fn code(&mut self, code: u32, len: u32) {
        let mut r = 0;
        for i in 0..len {
            r |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.bits(r, len);
    }

    /// Starts a block with fixed codes.
    pub fn fixed_block(&mut self, last: bool) {
        self.bits(u32::from(last), 1);
        self.bits(1, 2);
    }

    /// A literal byte or the end of the block (256), or a length code (257 to 287).
    pub fn symbol(&mut self, s: u32) {
        match s {
            0..=143 => self.code(0x30 + s, 8),
            144..=255 => self.code(0x190 + s - 144, 9),
            256..=279 => self.code(s - 256, 7),
            _ => self.code(0xC0 + s - 280, 8),
        }
    }

    /// A copy of 258 bytes from 1 byte back.
    pub fn run(&mut self) {
        self.symbol(285);
        self.code(0, 5);
    }

    /// A copy of 3 bytes from 32,768 bytes back: past the start of any output so far.
    pub fn far_copy(&mut self) {
        self.symbol(257);
        self.code(29, 5);
        self.bits(8191, 13);
    }

    /// A length code that does not exist (286): every inflater stops at it.
    pub fn bad_symbol(&mut self) {
        self.symbol(286);
    }

    /// A stored block with `data` (at most 65,535 bytes).
    pub fn stored(&mut self, last: bool, data: &[u8]) {
        self.bits(u32::from(last), 1);
        self.bits(0, 2);
        if self.bits > 0 {
            self.bits(0, 8 - self.bits);
        }
        let n = data.len() as u16;
        self.out.extend(n.to_le_bytes());
        self.out.extend((!n).to_le_bytes());
        self.out.extend(data);
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.bits(0, 8 - self.bits);
        }
        self.out
    }
}

/// A zlib stream that inflate stops on at once (a copy from before the start), followed by
/// `runs` copies of 258 bytes that a lenient decoder goes on to unpack.
pub fn lenient_bomb(runs: usize) -> Vec<u8> {
    let mut d = Deflate::zlib();
    d.fixed_block(true);
    d.far_copy();
    d.symbol(u32::from(b' '));
    for _ in 0..runs {
        d.run();
    }
    d.symbol(256);
    d.finish()
}

/// A one-page file of `content` on a page of `width` by `height` points, with `extra` (for
/// example `/Rotate 90`) in the page dictionary; Helvetica as `/F1`, Helvetica-Bold as `/F2`.
pub fn sized_page(content: &str, width: f32, height: f32, extra: &str) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let bold = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>");
    let c = b.stream("", content.as_bytes());
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {width} {height}] {extra} \
         /Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// The width of `text` in Helvetica at `size` (the font's standard widths for letters,
/// digits, the space and common signs), for setting text flush right.
pub fn helvetica_width(text: &str, size: f32) -> f32 {
    const LOWER: [u32; 26] = [
        556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333,
        500, 278, 556, 500, 722, 500, 500, 500,
    ];
    const UPPER: [u32; 26] = [
        667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722,
        667, 611, 722, 667, 944, 667, 667, 611,
    ];
    let units: u32 = text
        .chars()
        .map(|c| match c {
            'a'..='z' => LOWER[c as usize - 'a' as usize],
            'A'..='Z' => UPPER[c as usize - 'A' as usize],
            '0'..='9' => 556,
            '-' => 333,
            _ => 278,
        })
        .sum();
    units as f32 * size / 1000.0
}

/// The width of `text` at `size` in the font that [`print_form`] names `font` (`FL`, `FB`,
/// `FT` or `FTB`): the font's standard widths for letters, digits, the space and the signs
/// `-`, `%`, `.` and `,`; other characters count as a space.
pub fn font_width(font: &str, text: &str, size: f32) -> f32 {
    type Widths = ([u16; 26], [u16; 26], u16, u16, [u16; 4]);
    const HELVETICA: Widths = (
        [
            556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556,
            333, 500, 278, 556, 500, 722, 500, 500, 500,
        ],
        [
            667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778,
            722, 667, 611, 722, 667, 944, 667, 667, 611,
        ],
        556,
        278,
        [333, 889, 278, 278],
    );
    const HELVETICA_BOLD: Widths = (
        [
            556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611,
            389, 556, 333, 611, 556, 778, 556, 556, 500,
        ],
        [
            722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778,
            722, 667, 611, 722, 667, 944, 667, 667, 611,
        ],
        556,
        278,
        [333, 889, 278, 278],
    );
    const TIMES: Widths = (
        [
            444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500, 500, 500,
            333, 389, 278, 500, 500, 722, 500, 500, 444,
        ],
        [
            722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722, 556, 722,
            667, 556, 611, 722, 722, 944, 722, 722, 611,
        ],
        500,
        250,
        [333, 833, 250, 250],
    );
    const TIMES_BOLD: Widths = (
        [
            500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500, 556, 556,
            444, 389, 333, 556, 500, 722, 500, 500, 444,
        ],
        [
            722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, 722, 778, 611, 778,
            722, 556, 667, 722, 722, 1000, 722, 722, 667,
        ],
        500,
        250,
        [333, 1000, 250, 250],
    );
    let (lower, upper, digit, space, signs) = match font {
        "FL" => HELVETICA,
        "FB" => HELVETICA_BOLD,
        "FT" => TIMES,
        "FTB" => TIMES_BOLD,
        other => panic!("no widths for the font {other}"),
    };
    let units: u32 = text
        .chars()
        .map(|c| match c {
            'a'..='z' => lower[c as usize - 'a' as usize],
            'A'..='Z' => upper[c as usize - 'A' as usize],
            '0'..='9' => digit,
            '-' => signs[0],
            '%' => signs[1],
            '.' => signs[2],
            ',' => signs[3],
            _ => space,
        })
        .map(u32::from)
        .sum();
    units as f32 * size / 1000.0
}

/// A page of 10,000 by 10,000 points with a line of text and `n` different small pictures
/// of 30 by 30 points (each over 1 cm2, all of them under 2 % of the page) on a grid in the
/// middle: no band, no corner, so each needs vision and is cut out of the page's render.
/// Each picture is an inline picture of one RGB pixel of its own color, so the file and its
/// objects stay small.
pub fn small_pictures_twin(n: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut content = String::from("BT /F1 12 Tf 3000 8500 Td (Many small pictures.) Tj ET\n");
    let mut state = 7u32;
    for k in 0..n {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let color = (state >> 8) & 0xff_ffff;
        let (x, y) = (2500 + 100 * (k % 50), 2500 + 100 * (k / 50));
        content.push_str(&format!(
            "q 30 0 0 30 {x} {y} cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID {color:06x}> EI Q\n"
        ));
    }
    let c = b.stream("/Filter /FlateDecode", &deflated(content.as_bytes()));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 10000 10000] \
         /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// `data` deflated.
fn deflated(data: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(data).expect("deflate");
    z.finish().expect("deflate")
}

/// A square page with a line of text and one gray picture of 4 by 4 pixels drawn `n` times at
/// 29 by 29 points (each over 1 cm2) on a grid in its middle. The page is large enough that
/// all places together stay under 2 % of it, so each place is a small picture of one shared
/// asset.
pub fn many_places_twin(n: usize) -> Vec<u8> {
    let area = n as f64 * 29.0 * 29.0 / 0.015;
    let side = area.sqrt().max(1_000.0).ceil() as usize;
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    let (x0, y0) = (side / 2 - cols * 20, side / 2 - cols * 20);
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let image = b.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray \
         /BitsPerComponent 8",
        &[
            10, 40, 70, 100, 130, 160, 190, 220, 20, 50, 80, 110, 140, 170, 200, 230,
        ],
    );
    let mut content = format!(
        "BT /F1 12 Tf {} {} Td (Many places of one picture.) Tj ET\n",
        x0,
        y0 + cols * 40 + 100
    );
    for k in 0..n {
        let (x, y) = (x0 + 40 * (k % cols), y0 + 40 * (k / cols));
        content.push_str(&format!("q 29 0 0 29 {x} {y} cm /Im1 Do Q\n"));
    }
    let c = b.stream("/Filter /FlateDecode", &deflated(content.as_bytes()));
    let kid = b.add(format!(
        "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {side} {side}] \
         /Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {image} 0 R >> >> \
         /Contents {c} 0 R >>"
    ));
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// `pages` A4 pages, each with a line of text and `per_page` different inline pictures of 30
/// by 30 points in a row in the middle of the page: small pictures that need vision, each an
/// asset of its own.
pub fn small_picture_pages(pages: usize, per_page: usize) -> Vec<u8> {
    let mut b = Builder::default();
    let catalog = b.reserve();
    let tree = b.reserve();
    let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let mut kids = String::new();
    let mut state = 11u32;
    for n in 0..pages {
        let mut c = format!("BT /F1 12 Tf 72 700 Td (Page {n}.) Tj ET\n");
        for k in 0..per_page {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let color = (state >> 8) & 0xff_ffff;
            let x = 140 + 40 * k;
            c.push_str(&format!(
                "q 30 0 0 30 {x} 400 cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID {color:06x}> EI Q\n"
            ));
        }
        let cs = b.stream("", c.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {cs} 0 R >>"
        ));
        kids.push_str(&format!("{kid} 0 R "));
    }
    b.set(
        tree,
        format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
    b.finish(catalog)
}

/// The modules of a code of kind `s` with `text`.
pub fn modules(s: barcodes::Symbology, text: &str) -> barcodes::encode::Modules {
    barcodes::encode::text(s, text, &barcodes::encode::Options::default()).expect("encode")
}

/// A Code 128 of `text` drawn as bars: one filled `re` for each dark module column, `module`
/// points wide and `height` high, the code's lower left corner (its first module, light or
/// dark) at (`x`, `y`).
pub fn code128_bars(text: &str, x: f32, y: f32, module: f32, height: f32) -> String {
    let m = modules(barcodes::Symbology::Code128, text);
    let mut out = String::from("q 0 g\n");
    for col in 0..m.width {
        if m.dark[col as usize] {
            let left = x + col as f32 * module;
            out += &format!("{left:.3} {y:.3} {module:.3} {height:.3} re\n");
        }
    }
    out + "f Q\n"
}

/// The units of a read that the cell check of [`cells_apart`] looks at, in reading order:
/// each heading, paragraph or task, and each row of a table (its cells joined by ` | `), white
/// space collapsed. Sections, quotes, callouts and lists are opened.
pub fn read_units(blocks: &[Block]) -> Vec<String> {
    let collapse = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Heading { inlines, .. }
            | Block::Paragraph { inlines, .. }
            | Block::Task { inlines, .. } => out.push(collapse(&plain_text(inlines))),
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => out.extend(read_units(blocks)),
            Block::List { items, .. } => {
                for i in items {
                    out.extend(read_units(i));
                }
            }
            Block::Table { rows, .. } => {
                for row in rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|c| collapse(&plain_text(&c.inlines)))
                        .collect();
                    out.push(cells.join(" | "));
                }
            }
            _ => {}
        }
    }
    out
}

/// What is wrong with how the texts of drawn cells stand in the units of a read (see
/// [`read_units`]), found by the span each drawn text produced, not by words (a value glued to
/// the line before it is still found): each cell's texts stand together, no text of another
/// cell between them; with `ordered`, in the order given; and no unit holds texts of two
/// cells. A text is found where it stands in a unit, the longer texts first, so a short text
/// inside a longer one is not found twice. A text that `ambiguous` names (drawn more than once
/// on the page, or inside text no cell holds) or that stands more than once is left out of the
/// check, and counted (see [`cells_check`]).
pub fn cells_apart(
    units: &[String],
    cells: &[Vec<String>],
    ordered: &[bool],
    ambiguous: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    cells_check(units, cells, ordered, ambiguous).wrong
}

/// What [`cells_check`] found: what is wrong, and how many texts of the cells it left out
/// (drawn more than once, inside other text, or found more than once), by cell.
pub struct CellsCheck {
    pub wrong: Vec<String>,
    pub skipped: Vec<usize>,
}

impl CellsCheck {
    /// Whether every text of the cells `listed` (by their place) was left out: the check of
    /// them says nothing then.
    pub fn all_skipped(&self, cells: &[Vec<String>], listed: &[bool]) -> bool {
        let mut any = false;
        for (c, cell) in cells.iter().enumerate() {
            if listed.get(c).copied().unwrap_or(false) {
                any = true;
                if self.skipped[c] < cell.len() {
                    return false;
                }
            }
        }
        any
    }
}

/// [`cells_apart`], with the count of the texts it left out.
pub fn cells_check(
    units: &[String],
    cells: &[Vec<String>],
    ordered: &[bool],
    ambiguous: &dyn Fn(&str) -> bool,
) -> CellsCheck {
    let mut skipped = vec![0usize; cells.len()];
    // Every text with its cell and its place in the cell; a text of two cells is no one's.
    let mut texts: Vec<(&str, usize, usize)> = Vec::new();
    for (c, cell) in cells.iter().enumerate() {
        for (k, t) in cell.iter().enumerate() {
            let t = t.as_str();
            if t.trim().is_empty() || ambiguous(t) {
                skipped[c] += 1;
                continue;
            }
            let shared = cells
                .iter()
                .enumerate()
                .any(|(o, other)| o != c && other.iter().any(|x| x == t));
            if shared {
                skipped[c] += 1;
            } else {
                texts.push((t, c, k));
            }
        }
    }
    texts.sort_by_key(|t| std::cmp::Reverse(t.0.len()));
    // The spans found: unit, start, end, cell, place.
    let mut spans: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
    for (u, unit) in units.iter().enumerate() {
        let mut taken: Vec<(usize, usize)> = Vec::new();
        for &(t, c, k) in &texts {
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut from = 0;
            while let Some(at) = unit[from..].find(&t) {
                let (a, b) = (from + at, from + at + t.len());
                from = b;
                if taken.iter().any(|&(x, y)| a < y && x < b) {
                    continue;
                }
                taken.push((a, b));
                spans.push((u, a, b, c, k));
            }
        }
    }
    // A text found more than once is left out.
    let mut seen: std::collections::HashMap<(usize, usize), usize> = Default::default();
    for s in &spans {
        *seen.entry((s.3, s.4)).or_default() += 1;
    }
    for (&(c, _), &n) in &seen {
        if n > 1 {
            skipped[c] += 1;
        }
    }
    // A text found nowhere is no span and no skip: the value checks find it lost.
    spans.retain(|s| seen[&(s.3, s.4)] == 1);
    spans.sort_by_key(|s| (s.0, s.1));
    let mut wrong = Vec::new();
    let name = |c: usize| cells[c].join(" / ");
    for (u, unit) in units.iter().enumerate() {
        let mut of: Vec<usize> = spans.iter().filter(|s| s.0 == u).map(|s| s.3).collect();
        of.sort_unstable();
        of.dedup();
        if of.len() > 1 {
            let names: Vec<String> = of.iter().map(|&c| name(c)).collect();
            wrong.push(format!("{unit:?} holds texts of the cells {names:?}"));
        }
    }
    for c in 0..cells.len() {
        let mine: Vec<usize> = (0..spans.len()).filter(|&i| spans[i].3 == c).collect();
        let (Some(&first), Some(&last)) = (mine.first(), mine.last()) else {
            continue;
        };
        if let Some(o) = (first..=last).find(|&i| spans[i].3 != c) {
            wrong.push(format!(
                "the cell {:?} is not read together: {:?} stands between its texts",
                name(c),
                units[spans[o].0]
            ));
        }
        if ordered.get(c).copied().unwrap_or(false)
            && mine.windows(2).any(|w| spans[w[0]].4 > spans[w[1]].4)
        {
            wrong.push(format!("the cell {:?} is not read in its order", name(c)));
        }
    }
    CellsCheck { wrong, skipped }
}

/// The pitch of the data font of [`print_form`]: Courier at 9.5 points is 5.7 points a
/// character.
const FORM_PITCH: f32 = 5.7;

/// The data column of a data line from x 40 in Courier 9.5 that starts nearest `x`.
fn form_column(x: f32) -> usize {
    ((x - 40.0) / FORM_PITCH).round() as usize
}

/// A data line of `parts`, each text padded to the data column nearest its x.
fn form_line(y: f32, parts: &[(f32, &str)]) -> DataLine {
    let mut text = String::new();
    for (x, t) in parts {
        let len = text.chars().count();
        let at = form_column(*x).max(if len == 0 { 0 } else { len + 1 });
        text += &" ".repeat(at - len);
        text += t;
    }
    DataLine { y, text }
}

/// A stroked grid of 0.5 points over the column edges `xs` and the row edges `ys`.
fn form_grid(xs: &[f32], ys: &[f32]) -> String {
    let (x0, x1) = (xs[0], xs[xs.len() - 1]);
    let (y0, y1) = (ys[0], ys[ys.len() - 1]);
    let mut c = String::from("0.5 w ");
    for y in ys {
        c += &format!("{x0} {y} m {x1} {y} l S ");
    }
    for x in xs {
        c += &format!("{x} {y0} m {x} {y1} l S ");
    }
    c.push('\n');
    c
}

/// One section of the ruled table of [`synthetic_print_form`]: rows of 16 points from `top`
/// down between the column edges `xs`, every rule drawn, the first row holding the labels
/// `titles`.
fn form_section(xs: &[f32], top: f32, titles: &[&str], rows: usize) -> String {
    let ys: Vec<f32> = (0..=rows + 1).map(|k| top - 16.0 * k as f32).collect();
    let mut c = form_grid(xs, &ys);
    for (k, t) in titles.iter().enumerate() {
        c += &label(xs[k] + 3.0, top - 11.0, 7.0, t);
    }
    c
}

/// A boxed number field of `digits` cells from `x` with the data baseline `y`: one stroked
/// box a digit, and a drawn mark before each cell of `marks_before` (0-based).
fn form_boxed_digits(x: f32, y: f32, digits: usize, marks_before: &[usize]) -> String {
    let mut out = String::new();
    for k in 0..digits {
        out += &boxed(x + k as f32 * FORM_PITCH, y - 2.0, FORM_PITCH, 11.0);
    }
    for &k in marks_before {
        out += &mark(x + k as f32 * FORM_PITCH - 0.12, y - 0.5);
    }
    out
}

/// The invented values of one page of [`synthetic_print_form`].
struct FormValues {
    address: [&'static str; 3],
    head: [&'static str; 3],
    personal: [&'static str; 6],
    earnings: [[&'static str; 5]; 2],
    band: [&'static str; 4],
    amount: &'static str,
    date: &'static str,
    payout: &'static str,
}

/// The pages of [`synthetic_print_form`], so a test can check every data line.
pub fn synthetic_print_form_pages() -> Vec<FormPage> {
    let values = [
        FormValues {
            address: ["Erika Mustermann", "Musterweg 1", "12345 Musterstadt"],
            head: ["00000", "01.10.2026", ""],
            personal: ["0001", "Muster", "1", "0,5", "Kasse A", "100"],
            earnings: [
                ["1001", "Gehalt", "160", "12,34", "1.974,40"],
                ["1002", "Zulage", "", "", "40,00"],
            ],
            band: [
                "1000 Gehalt     2  1.111,11",
                "2000                 22,22",
                "3000 Bonus      1    333,33",
                "",
            ],
            amount: "987654",
            date: "28022026",
            payout: "1.234,56",
        },
        FormValues {
            address: ["Max Mustermann", "Beispielweg 2", "54321 Beispielstadt"],
            head: ["00001", "01.11.2026", ""],
            personal: ["0002", "Beispiel", "3", "1,0", "Kasse B", "200"],
            earnings: [
                ["1001", "Gehalt", "150", "13,45", "2.017,50"],
                ["1003", "Bonus", "", "", "100,00"],
            ],
            band: [
                "1000 Gehalt     2  2.222,22",
                // Drawn twice on purpose: a repeated row is read twice.
                "2000 Zulage     1     44,44",
                "2000 Zulage     1     44,44",
                "4000 Praemie    1     55,55",
            ],
            amount: "876543",
            date: "31032026",
            payout: "2.345,67",
        },
    ];
    values.iter().map(form_page).collect()
}

/// One page of [`synthetic_print_form`] with `v`: the template is the same on every page.
fn form_page(v: &FormValues) -> FormPage {
    let mut t = show("FB", 14.0, 40.0, 800.0, "Entgeltabrechnung");
    t += &label(
        40.0,
        778.0,
        6.0,
        "Muster GmbH, Musterweg 1, 12345 Musterstadt",
    );
    let mut lines = Vec::new();
    // The address block.
    for (k, l) in v.address.iter().enumerate() {
        lines.push(DataLine {
            y: 760.0 - 12.0 * k as f32,
            text: (*l).to_string(),
        });
    }
    // A header of boxed fields, the third left empty.
    let mut head = Vec::new();
    for (k, l) in ["Personalnummer", "Datum", "Steuerklasse"]
        .iter()
        .enumerate()
    {
        let x = 40.0 + 120.0 * k as f32;
        t += &boxed(x, 700.0, 120.0, 24.0);
        t += &label(x + 3.0, 716.0, 6.0, l);
        if !v.head[k].is_empty() {
            head.push((x + 5.0, v.head[k]));
        }
    }
    lines.push(form_line(703.0, &head));
    // A ruled table of two sections that share only their outer edges.
    let six: Vec<f32> = (0..=6).map(|k| 40.0 + 515.0 * k as f32 / 6.0).collect();
    let five: Vec<f32> = (0..=5).map(|k| 40.0 + 515.0 * k as f32 / 5.0).collect();
    t += &form_section(
        &six,
        670.0,
        &["Pers.-Nr.", "Name", "Klasse", "Kinder", "Kasse", "Abt."],
        1,
    );
    let row: Vec<(f32, &str)> = six
        .iter()
        .zip(v.personal)
        .map(|(x, s)| (x + 3.0, s))
        .collect();
    lines.push(form_line(670.0 - 32.0 + 5.0, &row));
    t += &form_section(&five, 638.0, &["Nr.", "Art", "Std.", "Satz", "Brutto"], 2);
    for (r, values) in v.earnings.iter().enumerate() {
        let row: Vec<(f32, &str)> = five
            .iter()
            .zip(values)
            .filter(|(_, s)| !s.is_empty())
            .map(|(x, s)| (x + 3.0, *s))
            .collect();
        lines.push(form_line(638.0 - 16.0 * (r + 2) as f32 + 5.0, &row));
    }
    // A band table: a header band of four drawn cells over value lines padded into its
    // columns, the edges in the spaces after characters 4, 13 and 17.
    let edges = [
        36.0,
        40.0 + 4.5 * FORM_PITCH,
        40.0 + 13.5 * FORM_PITCH,
        40.0 + 18.0 * FORM_PITCH,
        40.0 + 28.0 * FORM_PITCH,
    ];
    for w in edges.windows(2) {
        t += &boxed(w[0], 560.0, w[1] - w[0], 12.0);
    }
    for (x, s) in edges.iter().zip(["Nr", "Text", "Mg", "Betrag"]) {
        t += &label(x + 2.0, 563.0, 6.0, s);
    }
    for (k, l) in v.band.iter().enumerate().filter(|(_, l)| !l.is_empty()) {
        lines.push(DataLine {
            y: 548.0 - 11.4 * k as f32,
            text: (*l).to_string(),
        });
    }
    // A boxed amount and a boxed date in the right half, their separators drawn as marks.
    let x = 40.0 + 60.0 * FORM_PITCH;
    t += &label(300.0, 470.0, 6.0, "Betrag**");
    t += &form_boxed_digits(x, 460.0, v.amount.len(), &[4]);
    lines.push(DataLine {
        y: 460.0,
        text: format!("{}{}", " ".repeat(60), v.amount),
    });
    t += &label(300.0, 440.0, 6.0, "Zahltag");
    t += &form_boxed_digits(x, 430.0, v.date.len(), &[2, 4]);
    lines.push(DataLine {
        y: 430.0,
        text: format!("{}{}", " ".repeat(60), v.date),
    });
    // The total line: its label in the template, its value padded onto the label's baseline.
    t += &label(60.0, 400.0, 8.0, "Auszahlungsbetrag*");
    lines.push(DataLine {
        y: 400.0,
        text: format!("{}{}", " ".repeat(30), v.payout),
    });
    // Two footnotes.
    t += &show("FL", 6.0, 40.0, 60.0, "* nach Abzug der Steuern");
    t += &show("FL", 6.0, 40.0, 52.0, "** ohne Rundung");
    FormPage {
        template: t,
        lines,
        ..FormPage::default()
    }
}

/// A print-run form of two pages with invented values: a header of boxed fields (one empty),
/// an address block, a ruled table of two sections, a band table with a sparse row under a
/// header band of drawn cells, a boxed amount and a boxed date with marks, the total line
/// `Auszahlungsbetrag` with its value and two footnotes. Page 2 repeats the template with
/// other values and two identical lines.
pub fn synthetic_print_form() -> Vec<u8> {
    print_form(&synthetic_print_form_pages(), "", "")
}

/// `text` in the font `font` of [`print_form`] at `size`, its right end at `x`.
fn flush_right(font: &str, size: f32, x: f32, y: f32, text: &str) -> String {
    show(font, size, x - font_width(font, text, size), y, text)
}

/// The parts of [`synthetic_invoice`] that both pages share: three logos and a footer of
/// three columns at 6 pt, each row drawn over a string of 150 spaces from the left margin.
fn invoice_shared() -> String {
    let mut t = [
        logo(40.0, 770.0, 100.0, 50.0),
        logo(455.0, 692.0, 100.0, 40.0),
        logo(40.0, 30.0, 60.0, 30.0),
    ]
    .concat();
    let columns: [(f32, [&str; 3]); 3] = [
        (40.0, ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]),
        (
            230.0,
            [
                "Musterbank",
                "IBAN DE00 0000 0000 0000 0000 00",
                "BIC MUSTDEXXXXX",
            ],
        ),
        (
            420.0,
            [
                "Amtsgericht Musterstadt",
                "HRB 00000",
                "Gesch\u{e4}ftsf\u{fc}hrung Erika Mustermann",
            ],
        ),
    ];
    for (k, y) in [100.0, 92.0, 84.0].into_iter().enumerate() {
        t += &show("FL", 6.0, 40.0, y, &" ".repeat(150));
        for (x, lines) in columns {
            t += &show("FL", 6.0, x, y, lines[k]);
        }
    }
    t
}

/// An invented invoice of two pages, built with [`print_form`] (template only, no data
/// lines). Page 1: the logos, the recipient, the bold title `Rechnung` beside a field list,
/// line items with a header rule and an end rule, the totals, a two line bold sentence, a
/// bold label with its value, a body of 7 pt text, a Code 128 drawn as bars and the footer.
/// Page 2: the logos, one line and the footer.
pub fn synthetic_invoice() -> Vec<u8> {
    let mut one = invoice_shared();
    // The recipient.
    for (k, l) in ["Erika Mustermann", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
    {
        one += &show("FB", 9.0, 40.0, 750.0 - 11.0 * k as f32, l);
    }
    // The title beside the field list, its baseline 1.5 points under the first field line.
    one += &show("FB", 9.0, 40.0, 670.0 - 1.5, "Rechnung");
    for (k, [l, v]) in [
        ["Rechnungsnummer", "0000-0001"],
        ["Kundennummer", "00000"],
        ["Datum", "01.02.2026"],
        ["Seite", "1 von 2"],
    ]
    .iter()
    .enumerate()
    {
        let y = 670.0 - 8.0 * k as f32;
        one += &show("FL", 6.0, 360.0, y, l);
        one += &show("FL", 6.0, 440.0, y, v);
    }
    // Line items: bold titles, `Menge Einheit` as one show, the amount titles flush right at
    // their column edge, a header rule, three rows and an end rule.
    let size = 8.0;
    one += &show("FB", size, 40.0, 600.0, "Pos");
    one += &show("FB", size, 80.0, 600.0, "Artikel");
    one += &show("FB", size, 300.0, 600.0, "Menge Einheit");
    one += &flush_right("FB", size, 420.0, 600.0, "Preis");
    one += &flush_right("FB", size, 540.0, 600.0, "Betrag");
    one += &hrule(40.0, 540.0, 596.0);
    let unit_x = 300.0 + font_width("FB", "Menge ", size);
    for (r, row) in [
        ["1", "Beratung", "2", "Std", "500,00", "1.000,00"],
        ["2", "Material", "1", "Stk", "200,00", "200,00"],
        ["3", "Versand", "1", "Stk", "34,56", "34,56"],
    ]
    .iter()
    .enumerate()
    {
        let y = 590.0 - 10.0 * r as f32;
        one += &show("FL", size, 40.0, y, row[0]);
        one += &show("FL", size, 80.0, y, row[1]);
        one += &show("FL", size, 300.0, y, row[2]);
        one += &show("FL", size, unit_x, y, row[3]);
        one += &flush_right("FL", size, 420.0, y, row[4]);
        one += &flush_right("FL", size, 540.0, y, row[5]);
    }
    one += &hrule(40.0, 540.0, 564.0);
    for (k, (bold, l, v)) in [
        (false, "Nettobetrag", "1.234,56"),
        (false, "USt 19 %", "234,57"),
        (true, "Gesamtbetrag", "1.469,13"),
    ]
    .iter()
    .enumerate()
    {
        let font = if *bold { "FB" } else { "FL" };
        let y = 556.0 - 10.0 * k as f32;
        one += &show(font, size, 340.0, y, l);
        one += &flush_right(font, size, 540.0, y, v);
    }
    // The closing sentence of two bold lines, and a bold label with its plain value.
    one += &show(
        "FB",
        9.0,
        40.0,
        512.0,
        "Bitte zahlen Sie den Betrag bis zum 01.03.2026 auf das Konto unten.",
    );
    one += &show(
        "FB",
        9.0,
        40.0,
        501.0,
        "Vielen Dank f\u{fc}r Ihren Auftrag.",
    );
    one += "BT /FB 9 Tf 40 482 Td (Bestellnummer:) Tj /FL 9 Tf ( 00000) Tj ET\n";
    // The body: plain 7 pt text of at least 600 ink characters.
    let body = [
        "Lieferbedingungen: die Muster GmbH liefert die bestellte Ware frei Haus",
        "an die genannte Anschrift, sofern nichts anderes vereinbart ist. Teillieferungen",
        "sind zulaessig und werden einzeln berechnet. Die Ware bleibt bis zur",
        "vollstaendigen Bezahlung Eigentum der Muster GmbH. Zahlungsbedingungen: der",
        "Betrag ist ohne Abzug innerhalb von vierzehn Tagen nach Erhalt der Rechnung",
        "zu zahlen. Bei Zahlung innerhalb von sieben Tagen gewaehren wir zwei Prozent",
        "Skonto auf den Warenwert. Mahnungen werden mit einer Gebuehr berechnet. Bitte",
        "geben Sie bei jeder Zahlung die Rechnungsnummer an, damit wir den Betrag",
        "richtig zuordnen koennen. Fragen zur Rechnung beantwortet Ihnen gern unser",
        "Kundenservice an allen Werktagen. Es gelten unsere allgemeinen Bedingungen.",
    ];
    let ink: usize = body
        .iter()
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    assert!(ink >= 600, "{ink} ink characters");
    for (k, l) in body.iter().enumerate() {
        one += &show("FL", 7.0, 40.0, 460.0 - 9.1 * k as f32, l);
    }
    one += &code128_bars("0000000001", 100.0, 120.0, 0.8, 20.0);
    let two =
        invoice_shared() + &show("FL", 9.0, 40.0, 700.0, "Zahlbar bis 01.03.2026 ohne Abzug.");
    let pages: Vec<FormPage> = [one, two]
        .into_iter()
        .map(|template| FormPage {
            template,
            ..FormPage::default()
        })
        .collect();
    print_form(&pages, "", "")
}
