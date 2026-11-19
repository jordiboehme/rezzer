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
  on Candle. It is pure Rust and runs on the CPU. It is off by default.

Default features build the types, the error, the model description, the image preparation
(`prepare`), the ink rules, the ONNX reader, the resize code and the post-processing of both
detectors.

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
