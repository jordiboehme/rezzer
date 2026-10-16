# Rezzer research: OCR/VLM backends, diagram-to-Mermaid, AirPrint intake

Date: 2026-10-16. Scope: optional OCR backends for scanned PDFs and images, Mermaid extraction, and receiving print jobs from iPhone/macOS.

Legend: **[V]** verified today from a primary source (Hugging Face API, crates.io API, GitHub source, local run). **[S]** from a secondary source (blog, aggregator). **[E]** my estimate, not measured. **[U]** unverified.

---

## 0. Executive summary

- **Default OCR (fast, no download):** Apple Vision on macOS (free, on device, about 1 s per page [E]), and PP-OCRv5/v6 through `oar-ocr` (ONNX Runtime, Apache-2.0) on Linux and Windows. Tesseract only as a fallback for exotic languages.
- **Opt-in VLM tier (model download, 0.6 to 2 GB):** a small permissive model. Best candidates: **PaddleOCR-VL 1.5/1.6 (0.9B, Apache-2.0)**, **GLM-OCR (0.9B per vendor, 1.3B params on HF, MIT)**, **LightOnOCR-2-1B (Apache-2.0)**. All have official or ggml-org GGUF builds and llama.cpp support. LightOnOCR-2 (and probably GLM-OCR, which takes a plain `OCR` prompt on the whole page in its llama.cpp PR) are single-stage and drop into a generalized notedown loop (`llama-cpp-2`). PaddleOCR-VL (and MinerU) need a layout stage first, so their route is `oar-ocr-vl` (Candle, Metal/CUDA/CPU), which already implements those pipelines.
- **Opt-in "heavy quality" tier:** **dots.mocr / dots.ocr (3B, MIT)** for best open quality with a permissive license, **Chandra OCR 2 (5.3B)** for best handwriting/forms and diagram-to-Mermaid, but with a restrictive modified OpenRAIL-M license (user must accept; no bundling).
- **Docker tier:** Linux + NVIDIA only. Run vLLM (Chandra 2, olmOCR 2, dots.mocr) and let Rezzer call it via an OpenAI-compatible API. Docker on macOS has no Metal, so it is not useful there.
- **Diagrams:** structured sources (draw.io, Visio, SmartArt, PlantUML, Mermaid) should be converted deterministically; only raster/vector figures go to a VLM. Validate every VLM Mermaid output with the `merman` parser and fall back to "image + caption" on failure.
- **AirPrint:** feasible. A prototype works today with zero code: `/usr/bin/ippeveprinter` ships with macOS and accepts PDF/URF/PWG and runs a command per job (tested locally, a LAN device queried it within one second). A native Rust implementation (`ippper` + `mdns-sd` or system DNS-SD + `print_raster`) is about 1 to 3 weeks of work [E].
- **Note:** `docling.rs` (MIT, Rust port of IBM docling, created 2026-06) already does PDF/Office/scans to Markdown with ONNX layout + PP-OCR + TableFormer, plus Visio and iWork. It is a strong candidate to reuse or at least benchmark against.

---

## 1. OCR / VLM backends

### 1.1 Reference: notedown-ocr (read from ~/git/notedown/crates/notedown-ocr)

- License: **AGPL-3.0-or-later** (workspace). Rezzer can only reuse it if Rezzer is AGPL too, or if the author relicenses (he owns the copyright, so this is a decision, not a blocker).
- Size: about 2,100 lines. `llama.rs` (246 lines) loads model + mmproj with `llama-cpp-2 =0.1.158` (features `mtmd`, `metal` on macOS, `openmp` elsewhere, optional `vulkan`/`cuda`), tokenizes ChatML with the image marker, runs greedy decoding. Current `llama-cpp-2` is 0.1.160 (2026-10-15); notedown's 0.1.158 is from 2026-09-30, so it already includes the llama.cpp support for dots.ocr (merged 2026-04-09), GLM-OCR and PaddleOCR-VL (2026-02) [V, dates].
- Good parts to reuse: `OcrEngine` trait (`ocr_png(&[u8]) -> PageOcr`), `ModelSpec` with size + SHA-256 verification and cache dir, the llama/mtmd loop, Chandra HTML to Markdown converter, Mermaid detection.
- Chandra-specific parts: the prompt (`OCR_LAYOUT_PROMPT`, ChatML + empty think block), the HTML block converter, `IMAGE_MIN_TOKENS = 1024` (Qwen-VL), the hardcoded `DEFAULT_MODEL`.
- Packaging issues: the prompt is `include_str!("../../../tests/fixtures/...")`, outside the crate, so it cannot be published to crates.io as is. llama.cpp backend is process-global ("only one engine per process").
- Linux: `GPU_LAYERS = 0` on non-macOS, so Linux is CPU-only unless a Vulkan/CUDA build is used and the constant is changed. That explains slow CPU runs on Linux.
- Mermaid check is shallow: `looks_like_mermaid` only checks the first keyword. No syntax validation.

**Reuse verdict:** generalize into a `rezzer-vlm` crate: `ModelSpec { files, prompt_template, image_min_tokens, postprocessor }`, one llama/mtmd loop, one post-processor per model family (Chandra HTML, dots JSON, plain Markdown). This works for **single-stage** models (Chandra, dots.ocr/dots.mocr, LightOnOCR, DeepSeek-OCR, olmOCR). Probably also GLM-OCR (its llama.cpp PR uses a plain `OCR` prompt on a full image; the vendor SDK may still add a layout step [U]). For **two-stage** models (PaddleOCR-VL, MinerU2.5) a layout stage is needed first; for those, delegating to `oar-ocr-vl` (which already implements the pipelines on Candle) is cheaper than re-implementing them. llama.cpp docs state explicitly: "OCR models are trained with specific prompt and input structure" [V].

### 1.2 Comparison table

Params from the Hugging Face API (`safetensors.total`). olmOCR-Bench "Overall" from the Chandra README and the dots.mocr model card (vendor-reported, not independent).

| Backend | Model license | Code license | Params / download | olmOCR-Bench | Tables | Handwriting | Diagrams | Apple Silicon speed | CPU-only Linux | Rust path |
|---|---|---|---|---|---|---|---|---|---|---|
| **Chandra OCR 2** (datalab, 2026-03) | modified OpenRAIL-M: free for research, personal, startups under $2M funding/revenue; not competitive with Datalab API; "commercial self-hosting requires a license" [V] | Apache-2.0 [V] | 5.3B; Q4_K_M GGUF 3.07 GB + mmproj 0.37 GB [V] | 85.8 (vendor) [V] | 92.1, best [V] | excellent (vendor, examples) | yes, Mermaid in layout HTML (notedown) | **45 s/page on M5 (measured by author)** | several minutes/page [E] | llama-cpp-2 mtmd (done in notedown) |
| **olmOCR 2** (Ai2, 2025-10) | Apache-2.0 [V] (base Qwen2.5-VL-7B, Apache) | Apache-2.0 [V] | 8.3B; Q4 GGUF about 5 GB [E] | 82.4 [V] | 84.9 | moderate | no special support | 60 to 90 s/page [E] | impractical | llama-cpp-2 (Qwen2.5-VL is supported); community GGUFs (bartowski, lmstudio) [V] |
| **dots.ocr / dots.mocr** (rednote, moved to `dots-studio` org) | MIT [V] | MIT (README front matter) [V] | 3.0B [V]; ggml-org GGUF for dots.ocr [V] | dots.ocr 79.1, dots.mocr ("dots.ocr 1.5") 83.9 [V] | 90.7 (mocr) | good | dots.mocr parses charts/figures to **SVG**; separate `dots.mocr-svg` variant [V] | 20 to 35 s/page [E] | 2 to 5 min/page [E] | llama.cpp support merged 2026-04-09 (PR 17575) [V]; single-stage layout prompt |
| **MinerU 2.5** (opendatalab) | 2509 weights **AGPL-3.0**; Pro-2604/2605 weights **Apache-2.0** [V] | Apache-2.0 + extra terms: separate license above 100M MAU or USD 20M monthly revenue; attribution for online services [V] | 1.2B [V]; community GGUFs [V] | 75.2 (2.5.4) [V] | 84.9 | weak | no | 8 to 15 s/page [E] | 1 to 2 min/page [E] | `oar-ocr-vl` (native, two-step pipeline) [V]; llama.cpp GGUF exists, pipeline must be rebuilt |
| **PaddleOCR-VL 1.5 / 1.6** (Baidu) | Apache-2.0 [V] | Apache-2.0 | 0.96B; official GGUF 0.94 GB + mmproj 0.88 GB [V] | 80.0 (v1) [V]; OmniDocBench 1.5 text edit 0.035 (top tier) [V] | strong | moderate | chart recognition, no Mermaid | 5 to 15 s/page [E] | 30 to 90 s/page [E] | `oar-ocr-vl` native (Candle) [V]; llama.cpp support merged 2026-02-19 (PR 18825) [V]. **Two-stage**: layout model (PP-DocLayout) then per-region prompts |
| **GLM-OCR** (Zhipu, 2026-01) | MIT [V] | MIT [U] | 1.3B total per HF (vendor says 0.9B) [V] | not listed; OmniDocBench text edit 0.04 [V] | strong | moderate | no | 5 to 15 s/page [E] | 30 to 90 s/page [E] | ggml-org GGUF + llama.cpp (PR 19677, merged 2026-02-18), plain `OCR` prompt on the full page [V]; `oar-ocr-vl` native [V]. Pipeline stage count [U] |
| **LightOnOCR-2-1B** | Apache-2.0 [V] | Apache-2.0 [U] | 1.0B; Q8 GGUF 0.64 GB + mmproj 0.45 GB [V] | not checked | good [U] | weak to moderate [U] | no | 5 to 10 s/page [E] | 30 to 60 s/page [E] | ggml-org GGUF [V]; single-stage page to Markdown |
| **Nanonets-OCR2-3B** | **unclear**: no license on HF card; base Qwen2.5-VL-3B is **qwen-research** (non-commercial) [V] | - | 3.75B [V] | 69.5 [V] | 86.8 | good (vendor) | yes, Mermaid for flowcharts (vendor claim) [U] | 20 to 30 s/page [E] | slow | GGUF community builds [V]. **Avoid for licensing.** |
| **GOT-OCR 2.0** (StepFun, 2024) | Apache-2.0 on HF [V] | Apache-2.0 [U] | 0.72B [V] | not competitive any more | basic | weak | no | fast | ok | no llama.cpp support known [U]; Candle port would be needed. **Outdated, skip.** |
| **Granite-Docling-258M** (IBM) | Apache-2.0 [V] | MIT (docling) | 0.26B; ggml-org GGUF [V] | not listed | DocTags tables, ok | weak | no | 2 to 5 s/page [E] | 10 to 30 s/page [E] | llama-cpp-2 with ggml-org GGUF; output DocTags must be converted |
| **SmolDocling-256M** | CDLA-Permissive-2.0 [V] | MIT | 0.26B [V] | - | - | - | - | - | - | superseded by Granite-Docling. Skip. |
| **DeepSeek-OCR / OCR 2** | OCR: MIT; OCR 2: Apache-2.0 [V] | MIT/Apache | 3.3B / 3.4B [V] | 75.7 (OCR 1) [V] | 80.2 | moderate | figure parsing mode | 15 to 30 s/page [E] | slow | llama.cpp DeepSeek-OCR merged 2026-03-25 (PR 17400) [V]; OCR 2 support reported in build b9414 [S] |
| **HunyuanOCR** (Tencent) | Tencent Hunyuan Community License: **"does not apply in the European Union, United Kingdom and South Korea"** [V] | - | 1.1B [V] | Elo good | - | - | - | - | - | **Not usable for an EU author or EU users. Exclude.** |
| **jina-ocr-v1** | CC-BY-NC-4.0 [V] | - | 3.4B | - | - | - | - | - | - | **Non-commercial. Exclude.** |
| **Mistral OCR (cloud)** | proprietary API | - | - | 72.0 (older version) [V] | 60.6 | - | - | network | network | HTTP API. Now "OCR 4", about USD 4 per 1,000 pages per aggregators (docuocr.com, gotoolradar), not checked on mistral.ai [S]. Breaks "local". Optional plugin at most. |
| **Apple Vision** `VNRecognizeTextRequest` | OS framework, free | - | 0 (built in) | no benchmark | none (lines only) | decent for neat handwriting [U] | no | 0.3 to 1.5 s/page "accurate" [E] | n/a | `objc2-vision` 0.3.2 (Zlib/Apache/MIT) [V], or `apple-vision` 0.17.0 (safe wrapper, needs Xcode for a Swift bridge) [V], `native-ocr` 0.1.0 (Vision + Windows.Media.Ocr) [V] |
| **Apple Vision** `RecognizeDocumentsRequest` (macOS 26+) | OS | - | 0 | - | **yes: paragraphs, tables, lists** [V, Apple docs] | ? | no | fast [E] | n/a | **Swift-only API**: present in `Vision.swiftmodule/*.swiftinterface`, absent from the Objective-C headers of the local macOS SDK [V]. Needs a small Swift shim (swift-rs / swift-bridge); `objc2-vision` cannot reach it |
| **Tesseract 5.5.3** (2026-07) | Apache-2.0 [V] | Apache-2.0 | about 15 MB per language (tessdata_fast) | low on complex layouts | poor | poor | no | 1 to 3 s/page [E] | 1 to 3 s/page [E] | `tesseract-rs` 0.4.0 (MIT, can build Tesseract from source, 2026-07) [V]; `leptess` 0.14 (stale since 2023) [V]; `tesseract` 0.15.2 [V]. System dependency or C++ build. |
| **ocrs** | MIT/Apache-2.0 [V] | MIT/Apache-2.0 | about 10 MB models | - | none | poor | no | 1 to 3 s/page [E] | 2 to 5 s/page [E] | pure Rust (`rten` runtime), `ocrs` 0.13.1 [V]. **Latin alphabet only, "early preview"** [V] |
| **oar-ocr** (PP-OCRv5/v6, PP-StructureV3) | Apache-2.0 (Paddle models) | Apache-2.0 [V] | 10 to 200 MB depending on preset | PP-StructureV3 is decent | **yes: SLANet/SLANeXt table structure** [V] | weak | no | 1 to 3 s/page [E] | 2 to 6 s/page [E] | `oar-ocr` 0.10.0 (2026-10-03) on `ort` 2.0.0-rc.14 [V]; `oar-ocr-vl` for VLMs on Candle [V] |

Notes:
- `oar-ocr-vl` supports natively (Candle, CPU/CUDA/Metal) [V]: GLM-OCR, HPD-Parsing, HunyuanOCR, jina-ocr-v1, MinerU2.5 (2509, Pro-2605), MinerU-Diffusion, MonkeyOCRv2 (Apache-2.0, 0.6 to 0.8B), OvisOCR2 (Apache-2.0, 0.85B), PaddleOCR-VL 1.0/1.5/1.6, TeleOCR, WeVisDoc. It does not cover Chandra, dots or LightOnOCR. Its Metal speed vs. llama.cpp Metal is [U]; benchmark before choosing.
- Elo ranking in the dots.mocr card (Gemini-3-Flash judge, vendor-run): PaddleOCR-VL-1.5 920, GLM-OCR 892, HunyuanOCR 984, dots.ocr 1086, dots.mocr 1125, Gemini 3 Pro 1211 [V, vendor numbers].
- `ort` 2.0 is still a release candidate (rc.14) after a long time. Works, but pin it.

### 1.3 Recommended tiered design

```
tier 0  native text layer        PDF text extraction, no OCR (always first; per page decide "needs OCR?")
tier 1  fast default (no model download, or < 100 MB)
          macOS:   Apple Vision VNRecognizeTextRequest (objc2-vision) - lines + boxes
                   optional Swift shim for RecognizeDocumentsRequest (tables) on macOS 26+
          Linux/Win: oar-ocr PP-OCRv5/v6 + PP-StructureV3 (tables) via ort, models auto-download (~100 MB)
          fallback: Tesseract via tesseract-rs (feature flag, for rare languages)
tier 2  opt-in VLM "rezzer models pull <name>", 1 to 2 GB, permissive
          default pick (llama-cpp-2, notedown loop): LightOnOCR-2-1B (Apache) or GLM-OCR (MIT), single-stage
          alternative (oar-ocr-vl, Candle): PaddleOCR-VL 1.6 (Apache), two-stage with layout model
          heavy pick:   dots.mocr (MIT, 3B) - charts/figures to SVG
          licensed pick: Chandra OCR 2 (5.3B) - best handwriting + Mermaid, user must accept OpenRAIL-M terms
tier 3  remote/Docker "OpenAI-compatible endpoint"
          Linux+NVIDIA: vLLM image serving chandra-ocr-2 / olmOCR-2 / dots.mocr
          or any llama-server, or a cloud API (Mistral OCR) as explicit opt-in
```

Design rules:
1. **One trait, many engines**: `PageOcr { markdown, blocks: Vec<Block{kind, bbox, text}>, engine, timings }`. Engines declare capabilities (tables, handwriting, diagrams, languages) so the router can choose per page.
2. **Per-page routing**: text layer present -> skip OCR. Page classified as "table heavy" or "handwritten" (cheap heuristics or tier-1 confidence) -> escalate to tier 2 only if installed. MCP tool parameter `quality: fast|best` lets the agent choose.
3. **Model management**: reuse notedown's `ModelSpec` + SHA-256 verification; store licenses next to the model; show the license text and require an explicit flag (`--accept-license chandra-openrail`) for non-permissive weights. Never bundle restricted weights in releases or Docker images.
4. **Backend choice for tier 2**: start with `llama-cpp-2` (proven in notedown, Metal works, many OCR models already supported by llama.cpp). Add `oar-ocr-vl` for the two-stage models (PaddleOCR-VL, MinerU) instead of re-implementing their layout pipelines. On Linux, build with Vulkan by default and set GPU layers when a GPU exists (notedown uses 0 on non-macOS).
5. **Docker tier**: for Linux servers only. Docker Desktop on macOS gives no Metal access, so Mac users should use the native binary.
6. **Licensing of Rezzer itself**: if Rezzer is MIT/Apache, avoid AGPL deps (MinerU2.5-2509 weights, `mupdf` crate AGPL, notedown code unless relicensed, `libvisio-rs` GPL-3.0). If Rezzer is AGPL like notedown, all of them become possible.

### 1.4 Worth knowing: docling.rs

`docling-project/docling.rs` (MIT, created 2026-06-27, crates `docling`, `docling-core`, `docling-pdf` 2.34.0 published 2026-10-16) [V]. Claims: byte-identical output to Python docling, PDF 4.3x faster than Python docling, ONNX models (RT-DETR layout, PP-OCR, TableFormer, Whisper) under 1 GB, formats include Visio, SVG, Pages/Numbers/Keynote, DjVu, HEIC, a "remote VLM pipeline" and a "DeepSeek-OCR Markdown" path [V, README]. This overlaps heavily with Rezzer's scope (other research tracks should check it). For this track: it could serve as the tier-1 PDF layout + table engine instead of building one.

---

## 2. Diagram to Mermaid

### 2.1 Structured sources (deterministic, no ML)

| Source | Where the structure lives | Approach | Rust crates |
|---|---|---|---|
| **Mermaid** (in Markdown, .mmd) | text | pass through, validate | `merman` 0.9.0 (MIT/Apache, "parity-focused headless Rust implementation of Mermaid for parsing, layout, rendering") [V]; `tree-sitter-mermaid` 0.3.1 [V] |
| **draw.io / diagrams.net** (.drawio, .xml, also embedded in .png/.svg) | `mxfile > diagram` (often deflate + URL-encode + base64) `> mxGraphModel > root > mxCell` with `vertex="1"` / `edge="1" source= target=` and `value` labels; style string gives shape (rhombus = decision, ellipse = start/end, swimlane = subgraph via `parent`) | decode, build graph, emit `flowchart` (TD or LR by geometry), subgraphs from parent containers | `diagram-ir` 0.1.0 (MIT/Apache, 2026-09, very new, 13 downloads) parses draw.io (incl. compression) and Mermaid into a normalized IR [V]. Use it or copy the approach with `quick-xml` + `flate2` + `base64` (about 300 to 500 lines) |
| **Visio .vsdx** | OPC zip: `visio/pages/pageN.xml`: `<Shape ID>` with `<Text>`, master names (Process, Decision) via `visio/masters/`, and `<Connects><Connect FromSheet ToSheet FromCell="BeginX/EndX">` | connector shape with BeginX -> A and EndX -> B gives edge A->B; masters give node shape | `libvisio-rs` 0.2.1 is **GPL-3.0-or-later** (vsdx/vsd to SVG) [V]. For a permissive Rezzer: own parser with `zip` + `quick-xml` (about 1 week [E]). Legacy binary .vsd: skip or use docling.rs (claims Visio support [V]) |
| **DOCX/PPTX SmartArt** | `word/diagrams/dataN.xml` (`dgm:dataModel`: `dgm:ptLst` points with `type` doc/node/parTrans/sibTrans/pres and text in `dgm:t`, `dgm:cxnLst` with `srcId`/`destId` and type `parOf`); layout in `layoutN.xml` (`uniqueId` like `urn:microsoft.com/office/officeart/2005/8/layout/process1`, `hierarchy1`, `cycle2`); `drawingN.xml` is a pre-rendered fallback | build tree from `parOf` connections between `node` points; map layout family: process -> `flowchart LR` chain, hierarchy/orgchart -> `flowchart TD` or `mindmap`, cycle -> `flowchart` with closing edge, list -> plain nested Markdown list | **no crate exists** [V: crates.io search]. Hand-parse with `quick-xml` (about 3 to 5 days [E]). PPTX uses the same `ppt/diagrams/` parts |
| **PlantUML** (.puml, fenced blocks) | text | **pass through as a fenced `plantuml` block** (agents read PlantUML fine). Optional later: convert the easy subsets (sequence, simple activity, class) | no PlantUML to Mermaid converter in Rust [V: crates.io search]. `plantuml-little` (renderer), `plantuml-parser` (server client) exist but do not convert |
| **Graphviz DOT**, **BPMN XML**, **Excalidraw JSON** | text/XML/JSON | DOT: pass through or convert with a DOT parser (`dot-parser`/`graphviz-rust`, [U]); BPMN: process/task/sequenceFlow -> flowchart; Excalidraw: arrows with `startBinding`/`endBinding` -> edges | small custom converters |

### 2.2 Raster images and vector drawings

- **Raster images (embedded PNG/JPEG, scanned pages):** VLM prompt, as notedown/Chandra do. Chandra's layout prompt has a "Diagram" block type and outputs Mermaid inside the HTML [V: notedown prompt test]. With a smaller model, use a two-step approach: (1) the layout/OCR model marks a region as figure/diagram, (2) crop the region and ask a VLM with a dedicated prompt: "If this is a flowchart, sequence, state, class, ER or mind-map diagram, output only valid Mermaid. Otherwise output NONE." Provide the OCR'd text of the region as a hint to reduce hallucinated labels.
- **dots.mocr-svg** (MIT, 3B) converts charts, UI and scientific figures to **SVG** [V]. SVG with text + lines can then be converted to a graph deterministically (text elements as nodes, paths with markers as edges) [E: feasible but fragile].
- **Vector drawings in PDF/DOCX (DrawingML shapes, PDF paths):** reconstructing a graph from raw paths is possible (rectangles containing text = nodes, lines/arrows whose endpoints touch node boxes = edges) but brittle. Recommendation: detect the drawing region (cluster of vector paths), render it to an image (`hayro` 0.8.0 pure Rust, Apache/MIT, or `pdfium-render` 0.9.4) [V], then use the VLM path with the exact text from the PDF text layer as a hint. For DOCX DrawingML shapes with `wps:wsp` connectors (`stCxn`/`endCxn` ids), a deterministic conversion like Visio is possible.
- **Validation (important):** run every generated Mermaid through `merman` (parse only). On parse failure: one retry with the parser error in the prompt, then fall back to `![figure](...)` plus a short caption/description. Never emit invalid Mermaid; agents and renderers choke on it. Keep the original image reference next to the Mermaid block so the agent can check.
- Rendering is not needed for Rezzer's output, but `mermaid-rs-renderer` 0.3.1 (MIT, pure Rust) or `merman` can render for a preview/debug tool [V].

---

## 3. AirPrint / IPP Everywhere intake ("print to Rezzer")

### 3.1 What a client needs (ground truth)

Local test today (macOS, `/usr/bin/ippeveprinter -p 8632 -r _universal,_print -f application/pdf,image/urf,image/pwg-raster,image/jpeg -k RezzerTest`), read with `dns-sd -L` and `ipptool` [V]:

```
_ipp._tcp  (subtypes _universal, _print)   port 8632
TXT: rp=ipp/print ty=Example\ Printer adminurl=https://host.local.:8632/ note=
     pdl=application/pdf,image/jpeg,image/pwg-raster,image/urf Color=F Duplex=F
     UUID=729b104c-... TLS=1.2 URF=CP1,IS1-4-5-19,MT1-2-3-4-5-6,RS600,V1.4,W8
     txtvers=1 qtotal=1
IPP: document-format-supported = application/octet-stream,application/pdf,image/jpeg,image/pwg-raster,image/urf
     urf-supported, pwg-raster-document-type-supported = black_1,sgray_8,
     pwg-raster-document-resolution-supported = 300dpi,600dpi,
     ipp-versions-supported = 1.1,2.0, ipp-features-supported = ipp-everywhere, printer-uuid
```

Note: the macOS build advertised `TLS=1.2`, while upstream `ippeveprinter.c` and PAPPL advertise `TLS=1.3`; macOS ships an older Apple CUPS build. Use the upstream value.

Within one second another device on the LAN sent `Get-Printer-Attributes` to it, so discovery works out of the box.

Primary-source TXT keys:
- **CUPS ippeveprinter** (OpenPrinting/cups `tools/ippeveprinter.c`): `rp, ty, adminurl, note, pdl, Color, Duplex, UUID, TLS, URF, txtvers, qtotal`; default subtype `_print`; also registers `_ipps._tcp` and a `_printer._tcp` with port 0 [V].
- **PAPPL** (`pappl/dnssd.c`): `rp, ty, adminurl, note, pdl, kind, UUID, URF, Color, Duplex, TLS=1.3, txtvers, qtotal, priority, mopria-certified, product, Fax, PaperMax, Scan`; subtypes come from `papplSystemCreate(..., subtypes, ...)`, not a fixed default [V].
- **AirPrint-specific**: the `_universal` subtype on `_ipp._tcp` and a non-empty `URF` key are required for iOS to list the printer (long-standing, widely reported; Apple's AirPrint spec is under NDA) [S]. `pdl` should contain `image/urf`. `kind` (document, envelope, photo...) is optional.
- Claims that iOS 27 (Sept 2026) became stricter (mandatory `image/urf` in `pdl`, certificate max 825 days, SAN required, no IPP 1.1 fallback) come from one blog post that reads like generated content [S, low trust, **unverified**]. Treat as risk to test, not as fact. Safe defaults anyway: IPP 2.0, `image/urf` in pdl, valid UUID, a fresh self-signed cert with SAN `host.local` and validity under 398 days if IPPS is offered.

What will the client send? Hypothesis to test: when `application/pdf` is in `pdl` and `document-format-supported`, macOS and iOS send **PDF**, often with a real text layer (AirPdfPrinter's README says this keeps "the actual text rather than just images" [S]). Photos may arrive as JPEG; some apps or settings may send URF raster. If PDF with text arrives, Rezzer needs no OCR for most print jobs. **Must be verified on real iPhone/Mac.**

### 3.2 Building blocks

| Option | License | Status | Fit |
|---|---|---|---|
| `/usr/bin/ippeveprinter` (CUPS, preinstalled on macOS; Linux package `cups-ipp-utils` or similar [U]) as sidecar | Apache-2.0 (CUPS) [V] | maintained, CUPS 2.4.20 (2026-10-05) [V] | **Prototype in one hour**: `-c /path/to/rezzer-print-hook` runs a command per document (file path + env vars), `-f` sets formats, `-r _universal,_print`, `-K` keys for TLS, advertises itself. Good enough for a "beta" feature |
| **PAPPL** 1.4.12 (2026-08-20) as sidecar or linked C library | Apache-2.0 with GPLv2 linking exceptions [V] | maintained by Michael Sweet | full printer application framework (web UI, TLS, DNS-SD, job queue). Needs a C build + libcups3. Overkill unless Rezzer wants a real printer app with web UI |
| **`ippper`** 0.6.0 (Rust) | BSD-3-Clause [V] | active (pushed 2026-08), small (8 stars) | IPP server on hyper/tokio, TLS (rustls), `SimpleIppService` with configurable `document-format-supported` (PDF default), `urf-supported`, `pwg-raster-*`, `printer-uuid` [V: source]. README says it "fools Android Printer Service and Windows Print Spooler"; **no iOS/AirPrint mention, no issues about it** [V]. No mDNS built in. Best Rust base, but iOS compatibility must be proven |
| `ipp-printer-app` 0.10.0 (Rust) | MIT [V] | new (2026-05), 2 stars | IPP Everywhere framework on axum + `ipp` crate + `mdns-sd`; built for label printers: **only PWG/CUPS raster, no URF, 1-bpp default, no PDF passthrough** [V]. Not a fit without forking |
| `ipp` 7.0.0 | MIT/Apache [V] | client + parser/types | use for attribute types if building own server |
| `ipp-server` 0.3.0 | MIT | dead since 2019 [V] | no |
| `print_raster` 0.1.0 | BSD-3-Clause [V] | 2024, one release | decodes **URF (Apple Raster)**, CUPS Raster v1/v2/v3 incl. **PWG Raster**, async [V]. Output to `image` crate -> PNG for OCR |
| `pwgraster` 0.1.0 | MIT/Apache | 2017, PWG only [V] | fallback |
| `mdns-sd` 0.21.5 | Apache/MIT [V] | very active | pure Rust responder; supports one subtype per `ServiceInfo` via `"_universal._sub._ipp._tcp.local."` [V: source]. Tested by authors against Avahi, macOS `dns-sd` and iOS Bonjour [V: README]. On a Mac it runs its own socket on 5353 next to mDNSResponder; usually works with SO_REUSEPORT but is a known source of flakiness [U] |
| `astro-dnssd` 0.3.6 / `zeroconf` 0.18.0 | MIT/Apache / non-standard [V] | wrap system Bonjour (macOS) / Avahi (Linux) | **safer on macOS** (system mDNSResponder owns 5353). On Linux use Avahi if running, else `mdns-sd` |

Other constraints:
- **Port**: any port works, the SRV record carries it (ippeveprinter defaults to 8000 to 8999). No root needed.
- **TLS/IPPS**: plain `_ipp._tcp` works for discovery today (test above). Offer `_ipps._tcp` with a self-signed cert as well; iOS may prompt or behave differently [U].
- **Docker**: mDNS multicast needs `--network host`, which only works properly on Linux. On Docker Desktop for Mac, AirPrint from the container will not be discoverable.
- **macOS alternative without network printing**: a "PDF Services" entry (`~/Library/PDF Services/`, an app or script that receives the PDF) appears in every macOS print dialog. Cheap macOS-only path. iOS alternative: Share Sheet / Shortcuts to an HTTP endpoint.
- **Security**: a LAN printer that accepts anonymous jobs is an ingestion path for untrusted documents into an agent context (prompt injection). Default off, bind to chosen interfaces, optional HTTP Basic auth (iOS supports printer auth [U]), show a job log, and mark printed content as untrusted in MCP output.

### 3.3 Job to Markdown pipeline

```
IPP Print-Job / Send-Document
  -> document-format?
     application/pdf  -> normal PDF path (text layer first, OCR only for image-only pages)
     image/jpeg       -> image OCR path
     image/urf        -> print_raster (URF) -> per-page RGB/gray bitmap -> PNG -> OCR tier
     image/pwg-raster -> print_raster (PWG) -> same
  -> Markdown stored in Rezzer's inbox; MCP resource/notification "new printed document"
```
Raster jobs at 300 to 600 dpi are good OCR input. Tell clients `RS300` (plus 600) to keep jobs small; 300 dpi is enough for OCR.

### 3.4 Feasibility and effort [E]

| Path | Effort | Risk |
|---|---|---|
| A. ippeveprinter sidecar + hook command (`-c`) | 0.5 to 1 day incl. tests on iPhone/Mac | low; depends on CUPS tools on Linux; less control over attributes |
| B. Native Rust: `ippper` + `mdns-sd` (Linux) / `astro-dnssd` (macOS) + `print_raster` | first iPhone print 3 to 5 days; robust (IPPS, Get-Jobs, Validate-Job, ipptool conformance runs, iOS/macOS version matrix) 2 to 3 weeks | medium: iOS acceptance is a black box (NDA spec), must test on devices each iOS release |
| C. PAPPL printer app in C | 1 to 2 weeks | low protocol risk, but C code + libcups3 in an otherwise Rust project |

**Feasibility rating: high.** Recommend A for the first release (feature-flagged, "experimental"), then B once the format behavior (PDF vs. URF) is known from real devices. Use `ippeveprinter` and `ipptool` (CUPS test suites `ipp-everywhere.test`) as the reference and test harness for B.

---

## 4. Open questions / to verify next

1. Real iPhone (iOS 26/27) and macOS print to ippeveprinter: which format arrives, with text layer or not?
2. Benchmark on M5 and a CPU-only Linux box: Apple Vision, oar-ocr PP-OCRv6, PaddleOCR-VL 1.6 (oar-ocr-vl vs. llama.cpp), LightOnOCR-2, dots.mocr Q4, Chandra 2 Q4 (baseline 45 s). All speed numbers above except Chandra are estimates.
3. Rezzer license decision (MIT/Apache vs. MPL vs. AGPL; no decision recorded yet): decides reuse of notedown-ocr, libvisio-rs, mupdf, MinerU-2509.
4. GLM-OCR and LightOnOCR code licenses (and whether GLM-OCR needs a layout stage for good results) (only weight licenses checked).
5. Evaluate docling.rs as a dependency vs. competitor.
