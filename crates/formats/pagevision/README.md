# pagevision

Page OCR with vision models. pagevision reads a page image and returns typed blocks (headings,
paragraphs, lists, tables, diagrams and pictures) with a box on a 0..=1000 grid, a certainty and
Markdown for each block, plus the Markdown of the whole page.

The crate is standalone. It depends on no other rezzer crate.

## Features

- `serde`: `Serialize` and `Deserialize` for all public types.
- `llama`: the llama.cpp backend. This is native code and is off by default. It uses `mtmd` with
  `metal` on macOS and `mtmd` with `openmp` on other systems.
- `detect`: the layout detector (PP-DocLayoutV3) and the text line detector (PP-OCRv6 small det)
  on Candle. It is pure Rust and runs on the CPU. It is off by default. `detect::Detectors`
  loads both and returns the regions of a page (`Detectors::regions`).
- `engine`: `llama` and `detect` together. `engine::PageVision` loads all four model files and
  reads pages (see "Reading a page").

Default features build the types, the error, the model description, the image preparation
(`prepare`), the ink rules, the ONNX reader, the resize code and the post-processing of both
detectors. `regions_from` turns the detector boxes into regions without the models. The
prompts, the parsing of the answers, the Mermaid check and the page assembly are default code as
well.

## Regions

A page gets text regions and drawing regions. The layout detector finds the drawings. The text
lines and the ink rules make the text regions. Then every region is snapped to its ink:

- A region takes an ink stroke when at least half of the stroke lies in its box.
- When a drawing and a text region share a stroke, the drawing keeps it. A text region whose ink
  all lies in a drawing is dropped, so text under a drawing is not read twice.
- Two drawings or two text regions that share ink are merged.

The ink rules can see a drawing that the layout detector missed. It then stays a text region,
with the note "possible missed drawing" and a lower certainty. Regions come in reading order, top
to bottom and then left to right, with boxes on the 0..=1000 grid.

## Reading a page

`PageVision` has three calls:

- `regions(png)`: the regions of a page, as above.
- `read(png, regions)`: reads the given regions. Use it when you have your own regions.
- `read_page(png)`: `regions`, then `read`.

Qwen reads a page in a few passes:

- Each drawing is classified from its crop, with a call that answers with one word. An answer
  that is not "diagram" counts as a sketch.
- A sketch is described from its crop (`> Sketch: ...`). The model is never asked for Mermaid on
  a sketch.
- If the page has a diagram, one pass over the whole page gives the Mermaid blocks. They go to
  the diagram regions in reading order. The text of that pass is dropped. Each block is checked;
  a block that fails the check becomes a picture (`> Diagram: ...`). A diagram region without a
  block also becomes a picture, so nothing goes missing without a note.
- If the page has text, one pass over the whole page reads it, with every drawing blanked white.
  The text is split over the text regions in reading order, by their height. No line is lost or
  read twice.

The page goes to the model at 1408x1856, a multiple of 32. A crop is the region box plus 16 px,
scaled up when it is small, so the model gets at least 1024 image tokens.

The certainty of a block is the certainty of its region. It is halved when the answer did not
parse and lowered more when the answer hit the token limit. It is never above 0.95. A region
that fails gives no block and marks the page truncated. A page without any block is
`PageError::Unreadable`.

The Mermaid check covers flowcharts and sequence diagrams. Other kinds are named as not checked.

## Models

Model downloads are explicit. The crate never fetches a file on its own; the caller points it at a
folder that holds the model files, and a missing file is an error (`PageError::ModelMissing`). The
expected file names, sizes and SHA-256 values are in `model::QWEN35_4B`.

`model::ALL_FILES` lists the four files a caller downloads, each pinned to a Hugging Face revision
(`model::hub_url`), with its local file name, size, SHA-256 and licence:

- `Qwen3.5-4B-Q4_K_M.gguf` and `mmproj-F16.gguf` from `unsloth/Qwen3.5-4B-GGUF`;
- `pp-doclayoutv3.safetensors` (`model.safetensors` from `PaddlePaddle/PP-DocLayoutV3_safetensors`);
- `pp-ocrv6-small-det.onnx` (`inference.onnx` from `PaddlePaddle/PP-OCRv6_small_det_onnx`).

The `config.json` of PP-DocLayoutV3 is part of the crate and checked by SHA-256.

Tests that need the model are `#[ignore]` and find the folder through the environment variable
`PAGEVISION_MODEL_DIR`.

## Licence

The crate is MPL-2.0. Some files are adapted from Apache-2.0 code:

- the PP-DocLayoutV3 model in `src/detect/v3/` and the OpenCV resize in `src/resize.rs` come from
  oar-ocr-vl 0.10.3 (Copyright the oar-ocr authors, https://github.com/GreatV/oar-ocr). oar-ocr-vl
  ports the model from Hugging Face transformers and the resize from OpenCV. Both are Apache-2.0.
  Each file header names its sources and their copyright holders;
- the post-processing of both detectors (`src/textdet.rs`, `src/layoutpost.rs`) follows PaddleOCR
  and PaddleX.

These files keep the Apache-2.0 licence and say so in their header. The licence text is in
`LICENSE-APACHE`. So the crate licence is `MPL-2.0 AND Apache-2.0`.

The models are licensed under Apache-2.0. Qwen3.5-4B is by the Qwen team at Alibaba Cloud.
PP-DocLayoutV3 and PP-OCRv6 small det are by the PaddlePaddle Authors.
