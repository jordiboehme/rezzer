# PDF crate spike (2026-10-19)

Group: PDF. PostScript/EPS is left to another agent: none of the tested PDF crates reads
PostScript (hayro-postscript is only a tokenizer for fonts and CMaps).

Question: which Rust crates should Rezzer use to read PDF, rated by the structure and
metadata their API exposes, not by the Markdown they print.

Short answer: **lopdf for the object layer (metadata, annotations, forms, hidden logic,
revisions), pdf-inspector for reading order and layout, hayro for page rendering and
glyph boxes.** pdf_oxide exposes the richest model but is too big, too young and has too many
bugs for the default build. Keep it, Xberg and unpdf as benchmarks behind the same trait.

Everything here is throwaway spike work in `spike/` (gitignored):

- Corpus and manifest: `spike/corpus/pdf/` and `spike/corpus/pdf/MANIFEST-pdf.md`
- Probe code, one cargo package per crate: `spike/pdf-test/p-*`
- Raw probe output for every file: `spike/reports/pdf-probe-output/*.txt`
- Reference dump with pypdf 6.14.2 (Python, run with `-I` outside the corpus folder):
  `spike/reports/pdf-ground-truth.txt`

Typographic dashes from the source PDFs have been replaced by "-" in the copied output files.

## 1. Corpus

33 files, all with a license that allows redistribution (Apache-2.0 test suites of pdf.js,
qpdf and PDFBox; CC-BY-SA-4.0 py-pdf sample-files; CC-BY-4.0 arXiv; US government works;
a public domain NARA scan). Source URL, license, commit and SHA-256 for each file are in the
manifest.

| Intent | Files |
|---|---|
| Tagged PDF | irs-f1040-2025, nist-sp800-53r5, pdfjs-structure-simple, the 6 pdfjs-annotation-* files (Word 2013), pdfbox-actualtext-din91379 |
| Untagged multi-column | pypdf-multicolumn, arxiv-2408.09869-docling-report |
| Tables | arxiv (benchmark tables), nist (26 tagged tables), pypdf-multicolumn page 3, irs (tagged Table with Headers) |
| Outline | nist (358 items), arxiv (10), pypdf-outline (9), qpdf-page-labels-and-outlines (11, nested, non-ASCII titles) |
| Page labels | qpdf-page-labels-and-outlines, qpdf-aes256 (roman, arabic, restarts) |
| AcroForm | irs (229 fields, XFA too), pypdf-libreoffice-form (8 fields, value "Alice"), pdfbox-sf1199a |
| Annotations | Highlight, Underline, StrikeOut, FreeText, Text, Caret, Ink, Popup, Link (pdf.js, all with /T, /M, /CreationDate, /C); pypdf-annotated (direct dictionaries, odd Rect) |
| XMP | irs, nist, pypdf-xmp, scan, qpdf-aes* |
| Encryption, empty user password | qpdf-aes128 (V4), qpdf-aes256 (V5 R6) |
| Scan without text layer | scan-truman-letter-nara |
| Non-Latin script | pypdf-arabic-habibi |
| Embedded file | pypdf-attachment |
| Malformed | qpdf-malformed-bad-xref-entry, qpdf-malformed-damaged-stream, pdfjs-annotation-underline (wrong offset) |
| Hidden logic | calculation order and calculate script (pdfjs-acroform-calculation-order, pdfjs-text-field-calc), AFNumber/AFSpecial/AFDate formats (pdfbox-sf1199a, pdfjs-dates-afdate), document JavaScript (pdfjs-js-buttons, irs), named destinations, layers (pypdf-ocg-layers), article threads (pdfbox-poems-article-threads), signatures (pdfjs-signed, irs usage rights), incremental saves (qpdf-incremental-updates, sf1199a, irs, poems), Alt/ActualText/table header attributes (nist, din91379, irs) |

Gaps: no redistributable file was found in the time box with annotation replies (`/IRT`),
`AFSimple_Calculate`, `AFPercent_Format`, a field with the Required bit, or `/Scope` on table
headers. These cells are rated from the API, not from a test file. The probe also reads /Ff
only on each field's own dictionary; inherited flags from parent fields are not handled yet.

## 2. Candidates and measurements

Versions are the latest stable on crates.io on 2026-10-19, pinned exactly. APIs were read from
the crate sources in the cargo registry (exact versions) and checked by compiling the probes. Machine: Apple M5,
10 cores, rustc 1.97.0. Release profile in every probe: `strip = true`, `lto = "thin"`,
`codegen-units = 16`. Cold build = `cargo clean` then `cargo build --release` with its own
target dir. Size delta = probe binary minus an empty hello-world (339,008 bytes). Other spike
agents ran at the same time, so times are rough (repeat runs varied by up to 2x).

| Crate | Version | License (own) | MSRV | Cold build | Binary delta | Crates in tree | Releases last 90 days | Stars / open issues | Last push |
|---|---|---|---|---|---|---|---|---|---|
| lopdf | 0.45.0 | MIT | 1.88 | 22 s | +1.8 MB | 67 | 1 | 2,280 / 86 | 2026-10-19 |
| hayro (+ hayro-syntax, hayro-interpret) | 0.8.0 | Apache-2.0 OR MIT | 1.92 | 32 s | +5.2 MB (embedded fonts and CMaps on by default) | 75 | 1 | 806 / 28 | 2026-10-18 |
| pdf-inspector (Firecrawl) | 1.25.2 | MIT | 1.88 | 58 s | +7.9 MB (includes lopdf 0.45.0) | 87 | 19 | 19,691 / 250 | 2026-10-07 |
| unpdf (iyulab), `default-features = false, features = ["fast-parse"]` | 0.46.0 | MIT | 1.89 | 26 s | +3.3 MB | 66 | 45 | 61 / 0 | 2026-10-15 |
| pdf_oxide | 0.3.78 | MIT OR Apache-2.0 (crate has both files; GitHub detects Apache-2.0) | 1.88 | 150 s | +8.7 MB | 141 | 4 | 1,088 / 530 | 2026-10-14 |
| xberg, `default-features = false, features = ["tokio-runtime", "pdf"]` | 1.3.10 | MIT (no LICENSE file in the crate package) | 1.92 | 95 s | +24.0 MB | 250 | 52 | 9,427 / 19 | 2026-10-19 |

Notes:

- pdf_oxide default features pull office_oxide, taffy, image, brotli, qcms, md-5. Its source is
  about 450,000 lines of Rust (lopdf: 22,000; hayro-interpret: 27,000). Digital signatures need
  the `signatures` feature (x509-parser, cms, rsa, p256, p384).
- pdf-inspector and lopdf can share one lopdf 0.45.0 in the tree (pdf-inspector depends on
  it), so lopdf plus pdf-inspector costs the same +7.9 MB as pdf-inspector alone.
- pdfium-render (needs a native libpdfium per platform) was not built; it stays an optional
  feature as decided in the 2026-10-16 research. MuPDF and Poppler bindings were not touched
  (AGPL/GPL).

Speed and memory on the largest file (nist-sp800-53r5, 6 MB, 492 pages, wall time of the
probe, maximum RSS):

| Crate | Workload | Time | Max RSS |
|---|---|---|---|
| hayro | parse + interpret all pages through a text device | 0.2 to 0.5 s | 56 MB |
| lopdf | load + annotations + images + `extract_text` per page | 0.35 to 0.85 s | 205 MB |
| unpdf | `parse_file` (full model) | 1.6 s | 167 MB |
| pdf_oxide | `extract_text` all pages 3.0 s; `to_markdown_all` 2.9 s; `extract_tables` all pages 2.5 s | 8.5 s for all three | 272 MB |
| pdf-inspector | `process_pdf` + `extract_text_with_positions` (two passes) | 3.3 to 5.2 s | 351 MB |
| xberg | full extract with tables, images, annotations, structure | 4.5 to 5.1 s | 480 MB |

## 3. Scoring matrix

E = exposed, P = partial (raw only, heuristic, or with a defect), N = not exposed.
"Raw" means the crate gives the PDF objects and Rezzer has to interpret them.

### 3.1 Content and metadata

| Intent | lopdf | pdf_oxide | pdf-inspector | hayro | unpdf | xberg |
|---|---|---|---|---|---|---|
| Text in reading order | P: content-stream order, words split ("800 -53"), no column logic | E: chars/words/spans/lines with bbox, font, MCID; two columns right. Defect: Arabic file gives "ibibah" (Latin reversed) | E: best multi-column order; TextItem with x, y, w, h, font, bold, MCID. Items are runs, not glyphs | P: glyph runs with Unicode, transform and MCID through `Device`; no word or line assembly | P: paragraphs and runs; two-column order wrong ("Abstract pellentesque ante...") | P: two-column order wrong; Arabic Latin reversed like pdf_oxide |
| Headings and structure from the tag tree | P: raw StructTreeRoot walk with roles and RoleMap (works on all tagged files) | E: typed StructTreeRoot, StructElem (type, page, alt_text, actual_text, attributes), MCID refs, ParentTree, RoleMap | P: (page, MCID) to nearest role only; structure-simple gives only "NonStruct", so headings come from font size | P: MCIDs on marked content only; tree reachable as raw objects | N: reports `tagged: false` for tagged files; headings from font size | P: Heading nodes from heuristics, tag roles not surfaced |
| Tables as cells | N | E: rows, cells, bbox, colspan/rowspan, MCIDs. Defect: 1 table in NIST (26 tagged), ignores tag tree tables | E: `tables::detect_tables` gives a cell grid and Data/Toc kind; struct-tree table code exists | N | E: TableRow/TableCell with spans and paragraphs. Defect: IRS page 1 becomes one 59 x 55 table | P: cells as strings plus Markdown, page number; 61 in NIST |
| Lists | P: L/LI/LBody roles raw | P: roles and Markdown | P: Markdown only | N | E: list info on paragraphs | E: List and ListItem nodes |
| Footnotes | P: "Footnote" role raw (NIST 15), Hfootnote named dests | P: maps Footnote to "Note" | N | N | N | N (node type exists, none found) |
| Outline with targets | E: `get_toc` level, title, page. Defect: "Isís" decoded as "Is?s" | E: tree, PageIndex or Named target, correct decoding | N | P: raw | P: flat, page missing for named dests | P: only as anchor URIs ("#page=6"), no tree |
| Page labels | P: raw number tree (Rezzer formats it) | E: ranges and formatted labels, equal to pypdf | N | N | N | N |
| Links (internal and URI) | E: raw /A, /URI, /Dest, Rect | E: LinkAction Uri / GoTo(Named) / GoToR, rect | P: URI links as items with text; internal links lost | P: raw | N: no links found | E: URIs with label, page, kind (anchor or web) |
| Info dictionary | E: raw, decode_text_string | P: producer and creator accessors, rest raw bytes | E: title, author, subject, keywords, creator, producer, raw dates | E: typed DateTime with UTC offset | P: dates converted to UTC with the offset dropped (wrong instant) | P: same offset loss, keywords split |
| XMP | P: raw stream only | E: parsed dc/xmp/pdf fields plus raw XML. Defect: fails on the AES-128 file | N | N | N | P: merged into metadata (title, authors) |
| Language (/Lang) | E: catalog and struct /Lang raw | P: catalog raw only | N | P: raw | N | N (IRS en-US not reported) |
| Embedded files | E: name tree raw, stream reachable | E: name and bytes | N | N | N | P: tries, fails on image.png |
| Images with position | P: size and filters, no placement | E: size, bbox, colour space, filters | P: image placeholder items with x, y | E: `draw_image` with transform | N: no images reported | P: 41 images in arxiv, position not checked |
| Text layer present (OCR needed) | P: fonts + extract_text; wrong on bad-xref file | E: `has_text_layer`, `classify_page` | E: Scanned/TextBased/Mixed, pages_needing_ocr with reasons, garbled-text detection. Defect: bad-xref file reported as "Scanned" | P: glyph count per page | P: image_coverage, text_ops | P: no explicit flag in result |
| Robustness | 33/33 no panic; bad-xref file loads but text lost; AES-128 and AES-256 fine | 33/33; recovers bad xref; AES fine | 33/33; same bad-xref loss (uses lopdf) | 33/33; recovers everything, renders the scan | 33/33; recovers bad xref | 33/33; recovers bad xref |

### 3.2 Annotations

| Intent | lopdf | pdf_oxide | pdf-inspector | hayro | unpdf | xberg |
|---|---|---|---|---|---|---|
| Subtype, contents, author /T, /M, /CreationDate, /C, Rect, QuadPoints | E: all raw, all files | E: typed fields for all. Defect: annotations written as direct dictionaries in /Annots are dropped (pypdf-annotated: 0 of 3) | N | P: raw dicts | N | P: no CreationDate, FreeText reported as "Text", same direct-dictionary loss |
| Text covered by Highlight/Underline/StrikeOut | P: quads only; E when combined with a text crate | E by composition: quads x `extract_chars` gives "Lorem ipsum dolor sit amet" | P by composition: lopdf quads x inspector runs gives "Lorem ipsum dolor sit amet" and "Line 1", "Line 2" (run granularity) | P: glyph boxes available for composition | N | E: `marked_text` built in |
| Replies (/IRT) and popups | E: raw (/IRT, /Popup refs) | P: raw_dict keeps Popup and IRT, no typed link | N | P: raw | N | N |

### 3.3 Forms and hidden logic

| Intent | lopdf | pdf_oxide | pdf-inspector | hayro | unpdf | xberg |
|---|---|---|---|---|---|---|
| AcroForm fields: names, types, values | E: raw tree walk, full names, FT, V (IRS 199 terminal fields, LibreOffice 8) | E: typed FieldType/FieldValue, full_name, bounds, flags (counts non-terminal nodes: 229 / 10) | P: field items with text; UTF-16 values not decoded ("?? A l i c e") | N | E: typed (199 / 9) | E: typed, values decoded (229 / 10) |
| Calculation order /CO | E: raw, resolved to field names by Rezzer ("Mirror") | N | N | N | N | N |
| Calculate scripts and field dependencies | E: /AA /C JavaScript text; Rezzer finds `getField("Source")`, so Mirror depends on Source | N | N | N | N | N |
| Format, keystroke, validate actions (AFNumber_Format and similar) | E: raw JS, e.g. `AFNumber_Format(2, 0, 0, 0, "$", true)` = 2 decimals, currency "$" before the number; `AFSpecial_Format(2)` = phone; `AFDate_FormatEx("mm/dd/yyyy")` | N | N | N | N | N |
| Required / read-only flags | E: /Ff bits (Mirror is ReadOnly) | E: flags as integer | N | N | N | E: flags |
| Document and page JavaScript, OpenAction, /AA | E: Names/JavaScript tree, OpenAction, catalog and page /AA (IRS: 3 Adobe version-check scripts; js-buttons: `callback`) | N (only in its PDF/A validator) | N | N | N | N |
| Named destinations | E: Names/Dests tree and catalog /Dests (arxiv 54) | E: `resolve_named_destination` to page | N | P: raw | N | P: anchors only |
| Optional content groups (layers) | E: names and default OFF list (6 map layers) | P: `get_layers` returns nothing for the map file because it decodes /Name with `String::from_utf8` and skips UTF-16BE names (all 6 names start with FE FF) | N | P: raw | N | P: used internally to hide OFF content, not exposed |
| Article threads | E: with a short bead walk (Erlkönig 7 beads, Moulière 4) | E: `parse_article_threads` with beads, page, rect. Defect: titles decoded as "Erlk?nig" | N | N | N | N |
| Digital signatures | E: raw /Sig dict: Name, M, Reason, SubFilter, ByteRange; coverage check "covers whole file". Also finds the IRS usage-rights signature | E (feature `signatures`): signer, time, reason, sub-filter, covers_whole_document, certificate CN, CMS verify. Misses the IRS usage-rights signature (not a field) | N | N | N | N |
| Revisions / incremental updates | E with Rezzer code: load each `%%EOF` prefix and diff objects. IRS rev 3 added a /Sig and changed the Catalog; sf1199a rev 3 changed 17 objects; qpdf rev 3 added a content stream to page 1 | N | N | N | N | P: list of saves with xref offset and Info dates, no change content |
| Structure attributes: Alt, ActualText, table Headers/Scope | E: raw. Struct tree: NIST 36 /Alt, IRS TD Headers, TH ColSpan/RowSpan. Marked content (BDC property dicts in page and Form XObject streams): 1931 /ActualText in the DIN file | P: StructElem alt_text/actual_text and attributes map; the source has an ActualText index for marked content, effect on text not verified here | P: the source uses ActualText in text extraction, effect not verified here | N | N | N |

## 4. Evidence (short excerpts)

From `pdf-probe-output/` (paths shortened):

```text
lopdf  pdfjs-annotation-highlight.pdf
  annot p1 /Highlight T=Some("Tim van der Meij") M=Some("D:20151230152248+01'00'")
  CreationDate=Some("D:20151230152231+01'00'") C=Some("[1 0.929 0]") Contents=Some("Highlight content")
  Quad=Some("[70.860 745.586 193.880 745.586 70.860 736.177 193...") Popup=Some("<<9 keys>>")

pdf_oxide  same file
  markup_covered_text(chars x quads) "Lorem ipsum dolor sit amet"

pdf_oxide  pypdf-annotated.pdf (3 annotations, direct dictionaries)
  annot kinds {}                       <- dropped; lopdf and pypdf see Text, Highlight, Ink

pdf-inspector  pypdf-libreoffice-form.pdf
  fields ["First Name: ?? A l i c e", ...]   <- UTF-16 value not decoded

pdf-inspector  qpdf-malformed-bad-xref-entry.pdf
  detect Scanned pages 1 needing_ocr [1]     <- wrong, file has text "Potato"; hayro, pdf_oxide, unpdf, xberg recover it

pdf_oxide  pypdf-arabic-habibi.pdf
  md_p0 "ibibah ..."                            <- Latin word "habibi" reversed (xberg the same)

pdf_oxide  qpdf-page-labels-and-outlines.pdf
  pagelabels ["", "", "i", "ii", "iii", "iv", "v", "", "", "vi", "vii", "", "2", "3"]   (= pypdf)

lopdf logic  pdfjs-text-field-calc.pdf
  calc_order(CO) ["Mirror"]
  fields 2 with_flags(ReadOnly/Required) 1 ["Mirror[ReadOnly ]"]
  field "Mirror" FT=/Tx calculate(C): "event.value = this.getField(\"Source\").value;" deps {"Source"}

lopdf logic  pdfbox-sf1199a-direct-deposit.pdf
  field "Allottment Amount" FT=/Tx format(F): "AFNumber_Format(2, 0, 0, 0, \"$\", true);"
  field "Bank Rep Date" FT=/Tx format(F): "AFDate_FormatEx(\"mm/dd/yyyy\");"
  revision 3 ends_at 79002 objects 266 ModDate Some("D:20151124142707-05'00'") diff_vs_prev 17

lopdf logic  irs-f1040-2025.pdf
  signature Name=Some("ARE Production V8.1 G3 P24 1007685") M=Some("D:20260102090628-05'00'")
  SubFilter=Some("/adbe.pkcs7.detached") ByteRange=[0, 206928, 219198, 1039] covers_to_byte=220237 of 220237 (whole_file=true)
  revision 3 ... diff_vs_prev 7 ["2399 changed /Catalog", ..., "2448 added /Sig", ...]

xberg  pdfjs-annotation-strikeout.pdf
  StrikeOut p1 author Some("Tim van der Meij") modified Some("D:20151229150516+01'00'") color Some("#f94244")
  content Some("StrikeOut content") marked_text Some("Lorem ipsum dolor sit amet") quads 1

unpdf  irs-f1040-2025.pdf
  meta ... created Some(2026-01-02T07:24:31Z) ... tagged false   <- source is 07:24:31-06'00' and tagged
```

## 5. License findings

`cargo tree -e normal,build` per probe, every license field checked:

- All six trees are permissive only: MIT, Apache-2.0, BSD-2/3-Clause, Zlib, ISC-like,
  Unicode-3.0, Unlicense OR MIT, 0BSD. No GPL, LGPL or AGPL anywhere.
- xberg adds MPL-2.0 (`option-ext`, via `dirs`), CC0-1.0 (`tiny-keccak`) and BSL-1.0 dual
  options; all acceptable. Its only C build step is `cc` for `blake3` SIMD code (CC0/Apache).
  The xberg crate package has no LICENSE file, only `license = "MIT"` in Cargo.toml.
- No `-sys` crates except `core-foundation-sys` and `dirs-sys` (macOS system APIs, MIT/Apache).
  No crate links C PDF libraries in the tested feature sets.
- pdf_oxide: crate ships LICENSE-MIT and LICENSE-APACHE, but GitHub detects the repository as
  Apache-2.0 only. Ask upstream before relying on MIT; Apache-2.0 alone is fine for MPL-2.0.
- Features to keep off: pdf_oxide `ocr`/`gpu`/`ml` (ort, ONNX Runtime) and `pdfium-render`;
  pdf-inspector `ocr`, `model-download` (ureq: downloads models over the network) and
  `render-pdfium`; unpdf default feature `ai` (unparser-shared AI client); xberg everything
  except `pdf` (OCR, LLM, crawler `crawlberg`, embeddings).

## 6. Recommendation

| Part of the PDF reader | Crate | Why |
|---|---|---|
| Object layer: Info, XMP stream, /Lang, outline, page labels, named dests, links, annotations (all subtypes, replies, popups), AcroForm, hidden logic, signatures, layers, threads, embedded files, structure tree and its attributes, revisions | **lopdf 0.45.0** | Exposes every object the PDF has, verified on all 33 files. Small (+1.8 MB, 22 s), MIT, since 2016, 25 M downloads, one release in 90 days (stable API). It is the only crate that gives calculation order, scripts and revision diffs at all. Rezzer writes the interpretation (number tree, name tree, AF* parser, revision diff, XMP via quick-xml), which is a few hundred lines and stays under Rezzer's control |
| Text in reading order, line and block layout, heading guesses, tables as cells, scan detection | **pdf-inspector 1.25.2** | Best reading order on the two-column file, TextItems with position, font, bold and MCID, a table cell grid, page classification with OCR reasons. Shares lopdf 0.45.0, so it adds no second parser crate. MIT, Firecrawl, 19.7 k stars |
| Page rendering for OCR and vision, glyph-level boxes, fallback parser for broken files | **hayro 0.8.0** (hayro-syntax, hayro-interpret) | Needed anyway to render pages to PNG without pdfium. Fastest and most robust in the spike (recovers the bad-xref file, 56 MB RSS on NIST). Its `Device` gives every glyph with Unicode, transform and MCID, which maps highlights to exact words when a highlight covers only part of a text run. Few open issues, careful author |
| Benchmarks, not linked by default | pdf_oxide 0.3.78, xberg 1.3.10, unpdf 0.46.0 | Run them in the golden-file test suite to catch regressions in Rezzer's own output |

Why not pdf_oxide as the core, although it exposes the most: 450,000 lines, 530 open issues,
pre-1.0, 150 s cold build and +8.7 MB. In this small corpus it already showed five defects
in exactly the semantic parts Rezzer cares about: annotations given as direct dictionaries are
dropped, Arabic text reverses a Latin word, article thread titles are mis-decoded, layers with UTF-16
names are skipped, tagged tables are ignored. Everything it does better than lopdf at object level
(typed outline, page labels, XMP, typed forms) is a small amount of Rezzer code on top of lopdf.
If pdf-inspector's text turns out weak on real files, pdf_oxide is the first replacement behind
the same text trait (its `extract_chars`/`extract_spans` are the best glyph API tested).

Feature-set caveat: xberg ran with only the `pdf` feature (no layout or table models) and
unpdf with only `fast-parse` (no `raster`, no `ai`). Their reading-order, table and image
results apply to these lean builds, the ones Rezzer could link. The 2026-10-16 research rates
Xberg's default-build tables the same way.

Why not Xberg: +24 MB, 250 crates, 52 releases in 90 days, wrong two-column order, offsets
dropped from dates, no creation date or replies on annotations, no hidden logic. Its
`marked_text` on markup annotations and its revision list are good ideas to copy.

Why not unpdf: says `tagged: false` on tagged files, wrong two-column order, no annotations,
no images reported (may be the missing `raster` feature),
date offsets dropped, 45 releases in 90 days, one maintainer.

### Shape inside Rezzer

1. Parse once with lopdf for the object layer. On a lopdf load error or empty text on a page
   that has fonts (the bad-xref case), fall back to hayro-syntax for that file and say so in
   the output.
2. Give the bytes to pdf-inspector for positioned text items, tables and page
   classification. Join its MCIDs with lopdf's structure tree to get roles; prefer tag roles
   over font-size guesses when `/MarkInfo /Marked true`.
3. For markup annotations, intersect lopdf QuadPoints with text boxes; use hayro glyph boxes
   when a quad cuts a run.
4. Pages that need OCR or contain pictures: render with hayro to PNG and list them as
   "needs vision" with page number and reason.

Each crate sits behind a Rezzer trait (`PdfObjects`, `PdfText`, `PdfRender`) with golden files
for the corpus above.

### Risks

- pdf-inspector releases very often (19 in 90 days) and is tuned for Firecrawl's Markdown.
  Pin it, read the changelog before each bump, depend only on TextItem, tables and detection,
  not on its Markdown. Known defects to report upstream: UTF-16 form values, bad-xref file
  classified as scanned.
- lopdf's text extraction and repair are weak; Rezzer must not use `extract_text` for
  content, only for "has text" checks. Outline title decoding bug (PDFDocEncoding) means
  Rezzer should decode titles itself with `decode_text_string` on the raw object.
- hayro MSRV is 1.92 and its text Unicode mapping is marked "somewhat experimental".
- Revision diffs by re-loading each `%%EOF` prefix fail on linearized files (the first
  `%%EOF` belongs to the linearization section); skip that one.
- Reading JavaScript is safe, running it is not: Rezzer lists scripts as text, never executes.
- Tables on untagged PDFs stay heuristic in every crate; expect gaps compared to ML models.

## 7. How Rezzer should present the hidden logic layer

The aim: an LLM reads the output and can explain what the document does, not only what it
says. All of it is listed, nothing is run.

1. **A `## Form logic` section**, one table per form, like the spreadsheet formulas section:

   | Field | Type | Role | Value | Format | Rule | Reads | Flags |
   |---|---|---|---|---|---|---|---|
   | Source | text | input | | | | | |
   | Mirror | text | result (calc #1) | | | `event.value = this.getField("Source").value` | Source | read-only |
   | Allottment Amount | text | input | | number, 2 decimals, "$" before | | | |

   Role is "input" when the user fills it, "result" when a calculate action sets it, "constant"
   when read-only without a calculation. Calc number comes from `/CO`. AF* calls are translated
   into words (AFNumber_Format arguments: decimals, separator style, negative style, currency
   string, currency before or after; AFPercent_Format; AFDate_FormatEx with the pattern;
   AFSpecial_Format 0 to 3 = ZIP, ZIP+4, phone, SSN; AFSimple_Calculate("SUM", [fields])).
   The raw script stays in a fenced `javascript` block below the table.
2. **A Mermaid dependency graph** when at least one field reads another:

   ```mermaid
   flowchart LR
     Source["Source (input)"] --> Mirror["Mirror (result, calc 1)"]
   ```

   Edges come from `getField("...")` references and AFSimple_Calculate field lists. When a
   script builds field names at run time, the edge is marked "unknown, see script".
3. **A `## Scripts` section**: document-level JavaScript (name tree), OpenAction, document and
   page actions, button actions; each with trigger, target and the script text, marked
   "listed, not executed".
4. **Frontmatter facts** (Rezzer snake_case keys after the OKF keys): `signed: true`,
   `signatures` (signer, date, reason, sub-filter, `covers_whole_file`, whether there are
   saves after the signature), `revisions: 3`, `layers` (name, default visible or hidden),
   `has_javascript`, `form_fields: 35`.
5. **A `## Revisions` section**: one row per save with date (Info /ModDate of that revision)
   and what changed in words ("added signature field", "changed page 1 content",
   "field Phone value changed from A to B").
6. **Navigation and reading aids**: named destinations and article threads as lists
   ("Thread Erlkönig: page 1, 2, 3 ..."), layers as a table, structure attributes inline: Alt
   text as the picture caption, ActualText instead of the glyph text, table Headers/Scope used
   to build the Markdown table header.
7. **Gaps said in the output**: XFA forms (IRS has XFA too: "XFA form data present, not
   read"), scripts Rezzer cannot map to fields, layers whose content Rezzer could not
   separate.
