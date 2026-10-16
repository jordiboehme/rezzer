# Rezzer research: tool landscape, language choice, rmcp state

Date: 2026-10-16. Scope: (1) existing "anything to Markdown for LLMs" tools, (2) language comparison for a local file-to-Markdown MCP server, (3) state of the official Rust MCP SDK (rmcp).

Legend: **[V]** = verified today from a primary source (GitHub API, repo README, source code, official docs). **[VR]** = vendor-reported (a project's own README or benchmark about itself or its competitors). **[U]** = unverified (from memory or secondary sources; check before relying on it).

---

## 0. Key findings up front

1. **Kreuzberg is now "Xberg".** `kreuzberg-dev/kreuzberg` redirects to `xberg-io/xberg` [V]. The README says "Xberg is the next iteration of Kreuzberg ... rebuilt and rebranded under a fresh v1 line" [V]. The old v4 line lives on as `kreuzberg-dev/kreuzberg-lts` ("legacy; superseded by xberg for v5+", 18 stars) [V]. Xberg v1.0.14 shipped 2026-08-04, v1.3.10 on 2026-10-13, and the workspace is already at 1.3.11 [V]. That is a very fast release cadence plus one rebrand and one version renumbering within a few months.
2. **Xberg already ships an MCP server built on rmcp 3.5.1**, the same version notedown pins [V]. It also has a plugin registry for custom extractors, post-processors, OCR backends, validators and **renderers** [V, docs.xberg.io]. "Extend Xberg" is therefore a concrete option, not a theoretical one.
3. **Rust is now the most active language for local document-to-Markdown work.** New, popular Rust engines from 2026 include firecrawl/anydoc (22.9k stars, MIT, Office/RTF/EPUB/PDF incl. legacy .doc/.xls/.ppt), firecrawl/pdf-inspector (19.7k, MIT), run-llama/liteparse v2 (12.9k, Apache-2.0, PDF), SylphxAI/anymd (1.0k, MIT, Rust MCP server) and pdf_oxide (1.1k) [V]. Python used to be the clear leader [U]. Today it still is for ML layout models (Docling, Marker, MinerU), but no longer for format breadth or speed.
4. **No surveyed tool produces OKF frontmatter, Mermaid diagrams, a formulas section for spreadsheets, CAD, PostScript or AirPrint input.** Those are Rezzer's real differentiators. Everything else (PDF, Office, EPUB, HTML, CSV, OCR) is already solved well enough by existing Rust crates.
5. **rmcp is mature**: v3.5.1 (2026-10-05), implements MCP spec 2026-07-28, stdio plus streamable HTTP (stateful and stateless), resources, prompts, tasks, elicitation [V]. **Sampling (`create_message`) is deprecated** (SEP-2577) [V]. Rezzer must not rely on asking the client's LLM via sampling (for example to draw Mermaid diagrams).

---

## 1. Landscape of existing tools

GitHub numbers are from the GitHub API on 2026-10-16 [V] unless noted.

### 1.1 Summary table

| Tool | Language | License | Stars | Latest release | Local / cloud | MCP server | Distribution |
|---|---|---|---|---|---|---|---|
| Xberg (ex-Kreuzberg) | Rust core, 15 bindings | MIT | 9.4k | v1.3.10, 2026-10-13 | local (opt. hosted LLMs) | yes, built in (`xberg mcp`, rmcp 3.5.1) | single CLI binary (brew, scoop), Docker, pip/npm/etc. |
| firecrawl/anydoc | Rust | MIT | 22.9k | v0.2.4, 2026-08-27 | local; OCR only via hosted Firecrawl | no (ships an Agent Skill + CLI) | crate, npm/PyPI prebuilt binary, WASM |
| firecrawl/pdf-inspector | Rust | MIT | 19.7k | v1.25.2, 2026-09-28 | local | no | crate, npm, PyPI, WASM |
| run-llama/liteparse (v2) | Rust | Apache-2.0 | 12.9k | 2026-10-12 (docker-v2.16.0 tag) | local | not found [U] | crate, npm, PyPI, WASM, Docker |
| SylphxAI/anymd | Rust | MIT (core) + paid "Pro" | 1.0k | v8.5.4, 2026-10-16 | local | yes (4 tools) | npm-installed native binary |
| pdf_oxide | Rust | MIT OR Apache-2.0 (README badge) | 1.1k | v0.3.78, 2026-09-08 | local | yes (`pdf-oxide-mcp`) | crate, brew, 19+ bindings |
| extractous | Rust + Tika via GraalVM | Apache-2.0 | 1.8k | no GitHub release; last push 2024-12-21 | local | no | crate/PyPI with native libs |
| Microsoft MarkItDown | Python | MIT | 190.5k | v0.1.8, 2026-09-21 | local (opt. Azure Doc Intel / LLM) | yes, `markitdown-mcp` (1 tool) | pip / uvx, Docker |
| IBM Docling | Python | MIT | 69.0k | v2.140.0, 2026-10-16 | local (ML models) | yes, `docling-mcp` v3.3.0 | pip / uvx; docling-serve container |
| docling-serve | Python | MIT | 1.9k | v1.37.0, 2026-10-13 | local server | via docling-mcp | container images |
| Unstructured | Python | Apache-2.0 | 15.6k | 0.27.27, 2026-10-15 | local lib + paid platform | UNS-MCP (43 stars, targets their platform) | pip + system deps |
| Apache Tika | Java | Apache-2.0 | 4.1k | 4.0.0 stable (tag 2026-08-18); tag 4.1.0 dated 2026-09-26, no release note found | local | none found [U] | JVM zip distribution, Docker |
| pandoc | Haskell | GPL-2.0 | 46.6k | 3.12.1, 2026-10-08 | local | none official [U] | single static binary |
| MinerU | Python | "MinerU Open Source License" (Apache-2.0 plus extra conditions) | 81.9k | 4.0.12, 2026-10-11 | local, GPU preferred | not checked [U] | pip, Docker, model downloads |
| Marker | Python | code Apache-2.0, weights modified OpenRAIL-M | 40.5k | v2.0.0, 2026-07-20 | local, GPU preferred | not checked [U] | pip, model downloads |
| Surya | Python | Apache-2.0 (code) | 21.5k | v0.22.1, 2026-07-20 | local OCR/layout | no [U] | pip |
| Datalab Chandra | Python | Apache-2.0 (code); weights license not verified | 12.5k | last push 2026-06-26 | local model + hosted API | no [U] | pip |
| LlamaParse | hosted | proprietary SaaS | n/a | n/a | cloud only, ~$1.25 per 1000 credits, 1 to 45 credits/page by tier [secondary source] | no first-party one checked [U] | API |
| opendataloader-pdf | Java | Apache-2.0 | 29.7k | last push 2026-10-16 | local | not checked [U] | JVM |

Go options are small: unitedrhino/docling (pure Go, 44 stars), muthuishere/anymd (Go, 2 stars), recally-io/go-markitdown (26 stars) [V]. Nothing in Go is close to production breadth.

### 1.2 Per-tool notes

**Xberg (formerly Kreuzberg)** [V unless marked]
- Rust core, MIT. 107 formats / 141 extensions: docx/doc/odt/pages/WordPerfect, xlsx/xls/xlsb/ods/numbers, pptx/ppt/key, PDF, EPUB/FB2, RTF, HTML/XML, CSV/TSV/JSON/YAML/TOML, email (eml/msg/pst), archives (recursive), LaTeX/Typst/JATS, images with OCR, audio/video via Whisper.
- Output formats: plain, Markdown, Djot, HTML, JSON tree, DocTags, plus registered custom renderers.
- OCR backends: Tesseract, PaddleOCR, Candle, VLM. Layout and table models (PP-DocLayout-V3, RT-DETR, TATR, SLANet) behind ORT features. Default PDF backend is a native Rust one (`xberg-native-pdf` + lopdf); PDFium is an optional feature (`pdf-pdfium`).
- MCP: `xberg mcp --transport stdio` (or HTTP). 9 tools (extract, extract_batch, detect_mime_type, list_formats, cache_*...), 3 prompts, 4 resources. Built on `rmcp = 3.5.1`.
- Plugin system: "every format extractor, OCR engine, post-processor, validator, and renderer is a plugin that registers itself into a typed registry". Rust users can register custom extractors without FFI.
- Spreadsheet formulas: read via calamine `worksheet_formula`, but put into **metadata** (`formulas_<sheet>` as `A1=...` entries), **capped at 200 entries per sheet** (hard-coded defensive cap). The Markdown body shows cached values. So Rezzer's "formulas section" would need its own renderer or its own spreadsheet path.
- Size: release CLI tarballs are 45 to 60 MB compressed per platform (e.g. `xberg-cli-aarch64-apple-darwin.tar.gz` 51 MB, x86_64 macOS 60 MB, aarch64 musl 81 MB) [V]. Minimal from-source build with only needed Cargo features should be much smaller [U].
- Pins: `ort = "=2.0.0-rc.14"`, `candle 0.11`, rust-version 1.95 [V]. notedown's OCR crate uses HTTP (reqwest/scraper) only, no ort or candle, so no direct conflict there [V].
- Weak spots: very fast churn (rebrand, renumbered versions, near-daily releases), huge feature surface (embeddings, NER, crawling, 371 code languages) far beyond Rezzer's need, effectively one vendor. In anymd's AgentDocBench, Kreuzberg 4.10.4 with default config (OCR on, no layout/table models) scored table-cell F1 38.4 vs Docling 89.9 [VR: competitor-run benchmark, old v4 version, default features]. Treat that table score as "default build has weak PDF table recovery", not as a verdict on Xberg with layout models enabled.
- No CAD, no PostScript, no OKF, no Mermaid.

**firecrawl/anydoc** [V]
- Rust, MIT, created 2026-08-03 and already 22.9k stars. Formats: .doc/.docx/.docm, .ppt/.pps/.pot/.pptx/..., .xls/.xlsx/.xlsm/.xlsb, .odt/.ods/.odp, .rtf, .epub, .csv, .pdf (text PDFs via pdf-inspector).
- One shared document model, one Markdown serializer: headings with anchors, lists with source numbering, tables with merged cells and header rows, footnotes, speaker notes. Content-based format detection. Embedded assets kept on the document model.
- No OCR in the Rust crate (and the crate makes no network calls); CLI/Node/Python can send scanned docs to hosted Firecrawl Parse.
- Spreadsheet formulas: legacy .xls FORMULA records render their **cached values**; no evidence of a formula listing [V from source grep, partial].
- No MCP server (ships an Agent Skill + CLI). Own benchmark: 81/100 vs unstructured 63, libreoffice 40 [VR].
- Strong candidate as an Office/RTF/EPUB backend crate. Young (v0.2.x), API may change.

**firecrawl/pdf-inspector** [V]
- Rust, MIT. Classifies PDFs (text/scanned/mixed) in 10 to 50 ms, position-aware extraction, multi-column order, heading detection, rectangle and heuristic table detection, CID fonts, broken-encoding detection, selective OCR (PP-OCRv6 via external PDFium/ORT in Python/Node packages).
- Own benchmark on opendataloader-bench (200 PDFs, no ML): overall 0.875, tables TEDS 0.814; MarkItDown 0.589, tables 0.273 [VR].

**LiteParse v2** [V]: Rust rewrite (V1 was earlier code), Apache-2.0, PDF only, PDFium-based spatial text with bounding boxes, bundled Tesseract, complexity detection. Upsell to LlamaParse cloud.

**SylphxAI/anymd** [V]: Rust MCP server + CLI (formerly pdf-reader-mcp). PDF, docx, pptx, xlsx/xls/ods, CSV, EPUB, HTML/URLs, images (tesseract or local doc-VLM), audio/video (ffmpeg, Qwen3-ASR). 4 MCP tools: `outline`, `read` (with token budget and cursor), `search`, `inspect`. Spreadsheets capped at 2000 rows per sheet. Paid "Pro" add-on (US$29). Own benchmark (AgentDocBench, 38 docs): anymd 96.4, Docling 93.0, Kreuzberg 81.7, Unstructured 81.2, MarkItDown 76.8, Marker 71.0 [VR]. Interesting as a design reference for MCP tool shape (token budget, cursor, page markers).

**pdf_oxide** [V]: Rust PDF toolkit, text/image extraction and Markdown, brew install includes `pdf-oxide-mcp`. PDF only.

**extractous** [V]: Rust core, and "for file formats not natively supported by the Rust core, we compile Apache Tika into native shared libraries using GraalVM" and link them from Rust. Proof that Tika can be AOT-compiled, but the repo has had no push since 2024-12-21: stale, do not depend on it.

**Microsoft MarkItDown** [V]
- Python 3.10 to 3.14, MIT, 190k stars, still 0.1.x (v0.1.8, 2026-09-21).
- Formats: PDF (pdfminer.six + pdfplumber), PPTX, DOCX (mammoth), XLSX/XLS (pandas), images (EXIF, OCR via LLM), audio, HTML, CSV/JSON/XML, ZIP, YouTube, EPUB, Outlook msg. **No legacy .doc, no RTF** listed. OCR plugin `markitdown-ocr` uses an LLM vision client (OpenAI-style), not a local engine. Optional Azure Document Intelligence / Content Understanding (cloud).
- `markitdown-mcp`: one tool `convert_to_markdown(uri)` (http, https, file, data URIs), stdio or Streamable HTTP/SSE on localhost.
- Weak spots: PDF tables and headings are weak (pdf-inspector bench: tables 0.273, headings 0.0 [VR]); needs a Python environment; plugin system is Python-only.

**IBM Docling** [V]
- Python, MIT, very active (v2.140.0 released today). Inputs: PDF, DOCX/XLSX/PPTX, DOC/XLS/PPT and RTF **only via LibreOffice**, ODF, EPUB, Apple iWork, Markdown, AsciiDoc, LaTeX, HTML, MHTML, CSV, images, audio/video (ASR), WebVTT, email, AFP, plus JATS/USPTO/XBRL.
- Best open-source PDF layout and table structure (TableFormer, Granite-Docling VLM). Outputs Markdown, HTML, lossless JSON, DocTags, chunks.
- `docling-mcp` (v3.3.0): uvx, stdio/SSE/streamable-http, can call a local install or docling-serve.
- Weak spots: heavy (PyTorch + models), slow on CPU (anymd bench: 2432 s for 38 docs vs 11.5 s [VR]). docling-serve image size is several GB [U].

**Unstructured** [V for version/license, U for the rest]: Python, Apache-2.0, 0.27.27. Broad formats, but many need system tools (LibreOffice, poppler, tesseract) [U]. Element-based output, Markdown is secondary. MCP server (UNS-MCP) targets their paid platform API. Not a good fit.

**Apache Tika** [V]
- Java, Apache-2.0. 4.0.0 is the first stable 4.x release (tag 2026-08-18). Notable: **default content handler is now Markdown**; parsing runs in forked, crash-isolated JVM workers; Java 17+; new VLM parsers (Claude, Gemini, OpenAI) and in-process Tesseract (tess4j). tika-app/server now ship as zip with `lib/` dir; Docker images.
- Widest breadth for legacy formats via Apache POI (doc/xls/ppt, msg, many niche formats) and PDFBox.
- Weak spots: JVM, multi-process sizing, Markdown quality for tables in 4.0 untested by me [U]. No MCP server found [U].

**pandoc** [V]
- Haskell, GPL-2.0, single static binary, 3.12.1. Readers include docx, odt, pptx, xlsx, rtf, epub, html, csv, latex, typst, ipynb, docbook, jats, fb2. **No PDF, no legacy .doc** reader.
- Excellent structural fidelity for docx/odt/epub/html to Markdown (tables, footnotes). GPL: fine as an optional subprocess, problematic to statically link into an MIT binary.

**MinerU / Marker / Surya / Chandra / LlamaParse**: all PDF/image-centric, ML-heavy, GPU preferred. Best quality for scanned and complex-layout PDFs. Licenses need care: MinerU has a custom license, Marker's model weights are free only for research, personal use and orgs under US$5M funding/revenue [V]. LlamaParse and Datalab's API are cloud services and conflict with "local". Suitable only as optional heavy backends.

### 1.3 Gaps in the whole landscape (Rezzer's niche)

- **OKF frontmatter**: nobody. Xberg has a "frontmatter_utils" for Markdown inputs but no OKF output [V for module name, U for behavior].
- **Mermaid diagrams**: nobody converts diagrams (Visio, drawio, DOCX DrawingML shapes, SVG flowcharts) to Mermaid. This is new work in any language.
- **Spreadsheet formulas as a section**: Xberg collects formulas into metadata (capped at 200/sheet); others render cached values only.
- **PostScript**: no surveyed tool reads PS natively. Practical route is Ghostscript `ps2pdf` (AGPL) then the PDF path [U for Tika PS support].
- **CAD (DXF, 3MF, FreeCAD, Pro/E)**: none of these tools. Separate research.
- **AirPrint input**: none. Separate research.

---

## 2. Language comparison

| Criterion | Rust | Python | Java (Tika/POI/PDFBox) | Go | C#/.NET | Kotlin + GraalVM native-image |
|---|---|---|---|---|---|---|
| Format coverage today | Very good and growing fast: Xberg 107 formats incl. .doc/.xls/.ppt/RTF/EPUB/WordPerfect; anydoc Office+RTF+EPUB+PDF; calamine (xlsx/xls/ods, formulas) | Very good via aggregation (Docling, MarkItDown, Unstructured), legacy Office often via LibreOffice | Best breadth and legacy fidelity (POI HWPF/HSSF, PDFBox, very many Tika types [U for exact count]) | Weak: few mature parsers, PDF text extraction weak or cgo/MuPDF (AGPL) [U] | Good for OOXML (Open XML SDK, MIT); PDF via PdfPig; legacy .doc via NPOI is limited; best options are commercial (Aspose) [U] | Same as Java in principle |
| PDF layout/tables | Good heuristics (pdf-inspector, xberg-native-pdf), ML via ORT models in Xberg | Best (Docling TableFormer, Marker, MinerU) | Fair (PDFBox text, Tika tables weak [U]); opendataloader-pdf is Java and benchmarks well [VR] | Weak | Fair [U] | Fair |
| Single-binary distribution (macOS/Linux/Windows) | Excellent: static binary, cross-compiled, brew/scoop; Xberg full CLI is 45-60 MB compressed [V], a narrow build should be much smaller [U] | Poor: venv/uv, PyTorch, model downloads; PyInstaller bundles are large and fragile [U] | Poor: JVM or jlink image (~tens of MB plus JRE); Tika 4 even ships as zip + lib/ | Excellent | Good: self-contained single-file publish, NativeAOT with trimming caveats [U] | Possible but hard: Tika/POI/PDFBox rely on reflection, ServiceLoader, AWT; extractous shows it can be done, then went stale [V for extractous, U for effort] |
| MCP SDK | rmcp 3.5.1, official, spec 2026-07-28 [V] | python-sdk v2.3.0, official, most used [V] | java-sdk v2.1.0 (released today) [V] | go-sdk v1.8.0 [V] | csharp-sdk v2.2.0 [V] | kotlin-sdk 0.15.0, still pre-1.0 [V] |
| Author fit | Ships Rust MCP servers with rmcp already (notedown, Roger) | Second language at most | No | No | No | No |

**Is any language clearly better at file reading?**
- **Java** has the deepest legacy-format support (POI, PDFBox, Tika) and Tika 4 now emits Markdown by default. But distribution (JVM, forked workers) is the opposite of a lean local MCP server, and Rust has now closed most of the gap (Xberg and anydoc both read .doc/.xls/.ppt natively).
- **Python** is clearly better only for ML-based PDF layout and scanned documents (Docling, Marker, MinerU). These are heavy and slow, and they are easy to call as subprocesses or HTTP services.
- **Go and C#** offer nothing that Rust lacks for this job and have weaker parser ecosystems (Go) or rely on commercial libraries for legacy formats (.NET) [U].
- **Kotlin/GraalVM** would give Java's breadth in a native binary in theory; in practice it is a fight with reflection metadata, and the one project that did it (extractous) has stalled.

Conclusion: no language is clearly better overall. Rust is the best fit for the core, and the Python and Java strengths can be reached through optional backends.

Caveat: this verdict depends on the Rust legacy readers being good enough. That Xberg and anydoc read .doc/.xls/.ppt is verified as a format list [V]; how their fidelity compares to Apache POI is only vendor-reported or untested [VR/U]. The choice stays robust anyway: if the native readers turn out weak, `soffice --headless --convert-to docx|xlsx` normalizes legacy files to OOXML and the Rust core reads those. The spike corpus must include a legacy .doc and .xls with real tables to test this.

### 2.1 Hybrid architecture options

1. **Rust core + subprocess backends (recommended).** Rezzer is a Rust binary with an internal document model and an OKF/Markdown renderer. Native backends (crates) cover the common cases. Optional external backends are discovered on PATH or configured: `pandoc` (GPL-2, subprocess only), `soffice --headless` (convert .doc/.rtf/.ppt to OOXML or PDF), `gs`/`ps2pdf` (AGPL, PostScript), `docling` CLI or `docling-serve` HTTP (hard PDFs, scans), `tesseract`. Each backend returns Markdown or JSON that Rezzer normalizes. Simple, license-clean (no linking), testable.
2. **WASM plugins (wasmtime / extism).** Sandboxed, language-neutral plugins. Attractive for third-party extractors (CAD formats), but adds a plugin ABI to design and version, and most heavy backends (LibreOffice, Docling, Tesseract) cannot run in WASM. Premature for v1; keep the internal backend trait clean so it can be added later.
3. **Docker "full" image.** One prebuilt image with Rezzer + LibreOffice + pandoc + Ghostscript + Tesseract (+ optionally Docling). Good for the "everything optional" users. License check needed before publishing an image that bundles GPL/AGPL tools (fine to distribute with source offers, but needs care) [U legal detail].
4. **Embed Xberg vs depend on smaller crates.** See section 4.

---

## 3. rmcp (official Rust MCP SDK) state

Verified from GitHub, the rmcp README and docs.rs via Context7 [V]:
- Latest: **rmcp 3.5.1 and rmcp-macros 3.5.1, released 2026-10-05**. Previous: 3.5.0 (2026-09-28), 3.4.1 (2026-09-23). A 3.x migration guide exists (breaking changes from 2.x).
- Implements MCP spec **2026-07-28** and stays compatible with 2025-11-25 and earlier. New 2026-07-28 features covered: server discovery and negotiation (`server/discover`), transport-neutral subscriptions, **long-running tasks**, response caching, multi-round-trip requests, standard HTTP routing headers.
- Server features: tools (`#[tool]`, `#[tool_router]`, `#[tool_handler]` macros), resources, prompts, completions, logging, progress notifications (`notify_progress`), roots, pagination, elicitation (feature `elicitation`), JSON Schema 2020-12 via schemars, OAuth (feature `auth`).
- Transports: `transport-io` (stdio server), `transport-child-process` (client), `transport-streamable-http-server` (tower `StreamableHttpService`, session manager in-memory or stateless via `NeverSessionManager`), streamable HTTP client (reqwest or unix socket).
- **Sampling: `Peer::create_message` is deprecated since 1.8.0 because SEP-2577 deprecates sampling in the protocol; from protocol 2026-07-28 it must be issued while handling a client request.** Do not design Mermaid generation or image description around sampling.
- Known gotcha from notedown (engram): in rmcp 3.5 the `#[tool_handler]` expansion needs unqualified `Result` and `ErrorData` in scope.
- Relevance for Rezzer: tasks fit slow OCR/Docling conversions; resources fit "converted document as resource"; progress notifications fit long PDFs.

Other official SDKs today [V]: python-sdk v2.3.0 (2026-10-02), go-sdk v1.8.0 (2026-09-14), java-sdk v2.1.0 (2026-10-16), csharp-sdk v2.2.0 (2026-08-13), kotlin-sdk 0.15.0 (2026-07-28).

---

## 4. Build vs wrap/extend

### Option A: Wrap MarkItDown (or markitdown-mcp)
Reject. Python runtime, weakest PDF tables/headings in every benchmark seen, no .doc/.rtf, OCR only via an LLM client, still 0.1.x. Its only advantage is popularity. Any Rust option is better on quality and distribution.

### Option B: Extend Xberg (contribute an OKF renderer plugin, use `xberg mcp`)
- Pro: 107 formats, OCR, layout models, MCP server, all MIT and in Rust; renderer and extractor plugins exist.
- Con: Rezzer would then be "a config + plugin for Xberg", and its MCP tool shape, frontmatter and release cadence would be set by another vendor. Formula listing is capped at 200 per sheet and lives in metadata. CAD, PostScript, Mermaid and AirPrint are still all new work. Very fast churn (rebrand plus renumbering in 2026).

### Option C: Build Rezzer in Rust, embed backends behind its own document model (recommended)
- Rezzer owns: MCP surface (rmcp 3.5.x, stdio + streamable HTTP), internal document IR, OKF frontmatter, Markdown table rendering, formulas section, Mermaid generation, CAD and PostScript routing, AirPrint intake, caching.
- Backends behind a `Backend` trait:
  - Office/RTF/EPUB/legacy: **anydoc** crate or the **xberg** crate with a narrow feature set (decide by a spike on a real corpus).
  - PDF: **pdf-inspector** or xberg-native-pdf; OCR via notedown's OCR path or tesseract.
  - Spreadsheets: **calamine** directly (values + `worksheet_formula`), so formulas are complete and not capped.
  - HTML: html-to-markdown-rs (used by Xberg) or similar.
  - Optional subprocesses: pandoc, LibreOffice headless, Ghostscript, Docling (CLI or docling-serve).
- Pro: keeps control of output format and MCP tools, swappable backends, can still use Xberg where it is best, single binary for the core.
- Con: more code to own; must track upstream crates.

**Why build at all, given xberg, anymd, markitdown-mcp, docling-mcp and pdf-oxide-mcp exist?** Only because of OKF frontmatter, Mermaid, formula sections, CAD, PostScript, AirPrint and a stable tool contract for Crystalline ingestion. If those were dropped, `xberg mcp` or anymd would cover most of the need today.

### Suggested next steps
1. Spike: build a small corpus from Jordi's real files (PDF with tables, scanned PDF, .doc, .docx, .rtf, .xlsx with formulas, .epub, .ps, HTML). Run xberg CLI, anydoc, pdf-inspector, docling, markitdown. Compare Markdown quality and tables by hand. This replaces the vendor benchmarks.
2. Check embedding cost: build the xberg crate with a minimal feature set and measure binary size, compile time and dependency conflicts (ort `=2.0.0-rc.14` pin, candle 0.11, rust-version 1.95).
3. Define Rezzer's document IR and OKF renderer first; then plug in backends.
4. Licenses: keep GPL (pandoc) and AGPL (Ghostscript) as optional subprocesses; check before bundling in a Docker image. Avoid MinerU and Marker weights as defaults.
5. Xberg native deps: the wrapper crates `xberg-libwpd` (WordPerfect, vendors libwpd/librevenge + boost, built statically) and `xberg-libheif` (HEIC, needs system libheif) are MIT themselves [V], but upstream libwpd is LGPL-2.1/MPL-2.0 and libheif is LGPL-3.0 [U, from memory]. For an MIT single binary, leave the `heic` and WordPerfect features off unless the LGPL terms are handled (dynamic linking or relinkable objects).

---

## Sources

- GitHub API (repos, releases, tags, READMEs, Cargo.toml, source files) for: xberg-io/xberg, kreuzberg-dev/kreuzberg-lts, firecrawl/anydoc, firecrawl/pdf-inspector, run-llama/liteparse, SylphxAI/anymd, yfedoseev/pdf_oxide, yobix-ai/extractous, microsoft/markitdown, docling-project/docling, docling-project/docling-mcp, docling-project/docling-serve, Unstructured-IO/unstructured, Unstructured-IO/UNS-MCP, apache/tika, jgm/pandoc, opendatalab/MinerU, datalab-to/marker, datalab-to/surya, datalab-to/chandra, opendataloader-project/opendataloader-pdf, modelcontextprotocol/{rust,python,go,java,csharp,kotlin}-sdk.
- https://tika.apache.org/4.0.0/ (Tika 4.0.0 release notes)
- https://docling-project.github.io/docling/usage/supported_formats/
- https://docs.xberg.io/guides/plugins/ and https://docs.xberg.io/concepts/plugin-system/ (search snippets)
- https://docs.rs/rmcp/latest/rmcp/ via Context7 (/websites/rs_rmcp_rmcp)
- https://developers.llamaindex.ai/llamaparse/general/pricing/ (via search summary, secondary)
- Local: ~/git/notedown/Cargo.toml and crates/notedown-ocr/Cargo.toml (rmcp =3.5.1, OCR via HTTP)
