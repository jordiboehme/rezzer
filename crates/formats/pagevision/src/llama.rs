// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The llama.cpp backend: one vision model, greedy decoding, a checked image token count.

use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Once, OnceLock};
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::mtmd::{
    MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputChunkType, MtmdInputText,
    mtmd_default_marker,
};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::{LogOptions, send_logs_to_tracing};

use crate::error::PageError;
use crate::model::{ModelSpec, PREPROCESS_VERSION, prompt_sha256};
use crate::prepare::{Prepared, expected_image_tokens};
use crate::types::{Fingerprint, Stats};

/// The pinned `llama-cpp-2` version; keep equal to the workspace pin.
pub const LLAMA_CPP_2_VERSION: &str = "0.1.161";

/// Context size cap, prompt plus output.
const MAX_CTX: u32 = 16384;
const N_BATCH: u32 = 2048;
const MAX_DEFAULT_THREADS: usize = 8;

#[cfg(target_os = "macos")]
const GPU_LAYERS: u32 = 999;
#[cfg(not(target_os = "macos"))]
const GPU_LAYERS: u32 = 0;

/// Limits for one run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The most tokens one `generate` call may produce.
    pub max_tokens: u32,
    /// CPU threads; `None` picks up to eight.
    pub threads: Option<i32>,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            max_tokens: 4096,
            threads: None,
        }
    }
}

/// The text of one run.
#[derive(Debug, Clone)]
pub struct Generation {
    /// The output with any think block removed.
    pub text: String,
    pub stats: Stats,
    /// True when the token budget ran out before the model stopped.
    pub truncated: bool,
}

/// A vision model loaded in process. The llama.cpp backend is shared by all of them.
pub struct VisionModel {
    // Field order is drop order: the mtmd context goes before the model.
    mtmd: MtmdContext,
    model: LlamaModel,
    spec: &'static ModelSpec,
    opts: RunOptions,
    threads: i32,
}

impl std::fmt::Debug for VisionModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisionModel")
            .field("spec", &self.spec.id)
            .field("opts", &self.opts)
            .field("threads", &self.threads)
            .finish_non_exhaustive()
    }
}

fn engine_err(what: &str, e: impl std::fmt::Display) -> PageError {
    PageError::Engine(format!("{what}: {e}"))
}

/// Keeps llama.cpp, ggml and mtmd quiet unless `PAGEVISION_LLAMA_LOG=1`.
fn configure_logging() {
    static LOGGING: Once = Once::new();
    LOGGING.call_once(|| {
        if std::env::var("PAGEVISION_LLAMA_LOG").as_deref() != Ok("1") {
            send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
        }
    });
}

/// The process-wide backend, initialised by the first load.
fn backend() -> Result<&'static LlamaBackend, PageError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| engine_err("llama.cpp backend", e))
}

fn default_threads() -> i32 {
    let n = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(MAX_DEFAULT_THREADS);
    i32::try_from(n).unwrap_or(4)
}

/// Prompt plus output budget, capped.
fn context_size(prompt_tokens: u32, max_tokens: u32) -> u32 {
    prompt_tokens.saturating_add(max_tokens).min(MAX_CTX)
}

fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Removes a think block: everything up to and including the last `</think>`.
fn strip_think(text: &str) -> &str {
    match text.rfind("</think>") {
        Some(i) => text[i + "</think>".len()..].trim(),
        None => text.trim(),
    }
}

impl VisionModel {
    /// Loads `spec.model.name` and `spec.mmproj.name` from `dir`. No download.
    pub fn load(
        spec: &'static ModelSpec,
        dir: &Path,
        opts: RunOptions,
    ) -> Result<VisionModel, PageError> {
        let model_path = dir.join(spec.model.name);
        let mmproj_path = dir.join(spec.mmproj.name);
        for path in [&model_path, &mmproj_path] {
            if !path.is_file() {
                return Err(PageError::ModelMissing { path: path.clone() });
            }
        }
        configure_logging();
        let backend = backend()?;
        let threads = opts.threads.unwrap_or_else(default_threads);
        let model_params = LlamaModelParams::default().with_n_gpu_layers(GPU_LAYERS);
        let model = LlamaModel::load_from_file(backend, &model_path, &model_params)
            .map_err(|e| engine_err(&format!("loading {}", model_path.display()), e))?;
        let mmproj = mmproj_path.to_str().ok_or_else(|| {
            PageError::Engine(format!("path is not UTF-8: {}", mmproj_path.display()))
        })?;
        let mtmd_params = MtmdContextParams {
            use_gpu: cfg!(target_os = "macos"),
            print_timings: false,
            n_threads: threads,
            media_marker: CString::new(mtmd_default_marker())
                .map_err(|e| engine_err("media marker", e))?,
            image_min_tokens: spec.image.min_tokens,
            image_max_tokens: -1,
        };
        let mtmd = MtmdContext::init_from_file(mmproj, &model, &mtmd_params)
            .map_err(|e| engine_err(&format!("loading {}", mmproj_path.display()), e))?;
        if !mtmd.support_vision() {
            return Err(PageError::Engine(format!(
                "{} has no vision support",
                mmproj_path.display()
            )));
        }
        Ok(VisionModel {
            mtmd,
            model,
            spec,
            opts,
            threads,
        })
    }

    /// Runs one prompt on one prepared image, greedy, thinking off. Fails with
    /// `UnknownImageSize` or `ImageTokens` before decoding when the image chunk's token
    /// count is not the table's.
    pub fn generate(&mut self, image: &Prepared, prompt: &str) -> Result<Generation, PageError> {
        let (width, height) = (image.width, image.height);
        let expected = expected_image_tokens(width, height)
            .ok_or(PageError::UnknownImageSize { width, height })?;
        let backend = backend()?;

        let encode_start = Instant::now();
        let bitmap = MtmdBitmap::from_image_data(width, height, &image.rgb)
            .map_err(|e| engine_err("building the image", e))?;
        let text = self
            .spec
            .template
            .replace("{MEDIA}", mtmd_default_marker())
            .replace("{PROMPT}", prompt);
        let chunks = self
            .mtmd
            .tokenize(
                MtmdInputText {
                    text,
                    add_special: true,
                    parse_special: true,
                },
                &[&bitmap],
            )
            .map_err(|e| engine_err("tokenizing prompt", e))?;

        let mut image_tokens = 0usize;
        for i in 0..chunks.len() {
            if let Some(chunk) = chunks.get(i)
                && chunk.chunk_type() == MtmdInputChunkType::Image
            {
                image_tokens += chunk.n_tokens();
            }
        }
        let image_tokens = u32::try_from(image_tokens).unwrap_or(u32::MAX);
        if image_tokens != expected {
            return Err(PageError::ImageTokens {
                width,
                height,
                expected,
                actual: image_tokens,
            });
        }
        let prompt_tokens = u32::try_from(chunks.total_tokens()).unwrap_or(u32::MAX);
        let n_ctx = context_size(prompt_tokens, self.opts.max_tokens);
        if prompt_tokens >= n_ctx {
            return Err(PageError::Engine(format!(
                "prompt needs {prompt_tokens} tokens, context holds {n_ctx}"
            )));
        }

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(N_BATCH)
            .with_n_threads(self.threads)
            .with_n_threads_batch(self.threads);
        let mut ctx = self
            .model
            .new_context(backend, ctx_params)
            .map_err(|e| engine_err("creating llama context", e))?;
        #[allow(clippy::cast_possible_wrap)] // N_BATCH is a small constant
        let mut n_past = chunks
            .eval_chunks(&mut self.mtmd, &mut ctx, 0, 0, N_BATCH as i32, true)
            .map_err(|e| engine_err("evaluating prompt", e))?;
        let encode_ms = millis(encode_start);

        let decode_start = Instant::now();
        let vocab = self.model.vocab();
        let mut sampler = LlamaSampler::greedy();
        let mut batch = LlamaBatch::new(1, 1);
        let mut bytes = Vec::new();
        let mut output_tokens = 0u32;
        let mut truncated = true;
        let budget = self
            .opts
            .max_tokens
            .min(n_ctx.saturating_sub(prompt_tokens));
        while output_tokens < budget {
            // `sample` also accepts the token into the sampler.
            let token = sampler.sample(&ctx, -1);
            if vocab.is_eog(token) {
                truncated = false;
                break;
            }
            vocab.token_to_piece_into(token, &mut bytes, true, None);
            output_tokens += 1;
            if output_tokens == budget {
                break;
            }
            batch.clear();
            batch
                .add(token, n_past, &[0], true)
                .map_err(|e| engine_err("building batch", e))?;
            ctx.decode(&mut batch)
                .map_err(|e| engine_err("decoding", e))?;
            n_past += 1;
        }
        let decode_ms = millis(decode_start);

        let raw = String::from_utf8_lossy(&bytes);
        Ok(Generation {
            text: strip_think(&raw).to_string(),
            stats: Stats {
                prompt_tokens,
                image_tokens,
                output_tokens,
                encode_ms,
                decode_ms,
            },
            truncated,
        })
    }

    /// The versions and hashes a result depends on.
    pub fn fingerprint(&self, prompt: &str) -> Fingerprint {
        Fingerprint {
            crate_version: env!("CARGO_PKG_VERSION").to_string(),
            llama_cpp_2: LLAMA_CPP_2_VERSION.to_string(),
            model_sha256: vec![
                self.spec.model.sha256.to_string(),
                self.spec.mmproj.sha256.to_string(),
            ],
            prompt_sha256: prompt_sha256(prompt),
            preprocess: PREPROCESS_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QWEN35_4B;

    #[test]
    fn context_size_adds_output_budget_and_caps() {
        assert_eq!(context_size(3000, 4096), 7096);
        assert_eq!(context_size(3000, 100_000), MAX_CTX);
        assert_eq!(context_size(u32::MAX, u32::MAX), MAX_CTX);
    }

    #[test]
    fn default_threads_is_bounded() {
        assert!((1..=8).contains(&default_threads()));
    }

    #[test]
    fn strip_think_removes_the_block() {
        assert_eq!(strip_think("<think>\nhm\n</think>\n\nHello"), "Hello");
        assert_eq!(strip_think("  plain \n"), "plain");
    }

    #[test]
    fn version_constant_matches_the_pin() {
        let manifest = include_str!("../../../../Cargo.toml");
        assert!(manifest.contains(&format!(
            "llama-cpp-2 = {{ version = \"={LLAMA_CPP_2_VERSION}\""
        )));
    }

    #[test]
    fn load_reports_missing_model() {
        let err = VisionModel::load(&QWEN35_4B, Path::new("/nonexistent"), RunOptions::default())
            .unwrap_err();
        assert!(matches!(err, PageError::ModelMissing { .. }), "{err}");
    }
}
