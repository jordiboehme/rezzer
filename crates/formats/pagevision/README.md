# pagevision

Page OCR with vision models. pagevision reads a page image and returns typed blocks (headings,
paragraphs, lists, tables, diagrams and pictures) with a box on a 0..=1000 grid, a certainty and
Markdown for each block, plus the Markdown of the whole page.

The crate is standalone. It depends on no other rezzer crate.

## Features

- `serde`: `Serialize` and `Deserialize` for all public types.
- `llama`: the llama.cpp backend. This is native code and is off by default. It uses `mtmd` with
  `metal` on macOS and `mtmd` with `openmp` on other systems.

Default features build the types, the error and the model description only.

## Models

Model downloads are explicit. The crate never fetches a file on its own; the caller points it at a
folder that holds the model files, and a missing file is an error (`PageError::ModelMissing`). The
expected file names, sizes and SHA-256 values are in `model::QWEN35_4B`.

Tests that need the model are `#[ignore]` and find the folder through the environment variable
`PAGEVISION_MODEL_DIR`.

## Licence

The crate is MPL-2.0. The supported model, Qwen3.5-4B (GGUF files from `unsloth/Qwen3.5-4B-GGUF`),
is licensed under Apache-2.0.
