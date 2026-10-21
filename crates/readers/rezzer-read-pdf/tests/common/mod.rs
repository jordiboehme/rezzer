// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small PDF files written by hand for the tests, with a classic cross-reference table.

#![allow(dead_code)]

use std::io::Write;

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

/// The width of `text` in Helvetica at `size`, for digits, the point, the space and the
/// capital letters and small letters a test uses to set text flush right.
pub fn helvetica_width(text: &str, size: f32) -> f32 {
    let units: u32 = text
        .chars()
        .map(|c| match c {
            '0'..='9' => 556,
            '.' | ' ' | ',' => 278,
            '-' => 333,
            ':' => 278,
            _ => 556,
        })
        .sum();
    units as f32 * size / 1000.0
}
