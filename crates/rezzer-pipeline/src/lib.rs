// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One run of Rezzer: open and detect, read (inside a [`Budget`], panics contained), enrich,
//! render and page, with a result cache in front of reading and enriching. The CLI and the
//! MCP server both drive this.
//!
//! What Rezzer cannot read is said in the output: a format without a reader and an unknown
//! format give a document with the source facts, one paragraph that says so and a loss with
//! reason `unsupported` (completeness partial). An empty file is complete and says it is
//! empty. A reader that panics gives a failed result whose loss names the reader.

mod cache;
mod config;
mod panics;

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rezzer_detect::OpenError;
use rezzer_limits::Budget;
use rezzer_model::{
    Block, Completeness, Confidence, Detection, Document, FormatId, Inline, Loss, LossReason,
    ReadOptions, ReaderInfo, Registry, Severity, Source, source_info,
};
use rezzer_render::RenderOptions;

pub use config::{Config, ConfigError, DEFAULT_CACHE_MAX_BYTES, config_path_from};
pub use rezzer_limits::CancelToken;
pub use rezzer_render::OutputSection;

use crate::cache::Cache;
use crate::config::one_line;

/// The Rezzer version written into the output and the cache key.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Identifies the build in the cache key: the git commit (with a dirty marker and build time
/// for a changed checkout) or the build time. Set by build.rs.
pub const BUILD_ID: &str = env!("REZZER_BUILD_ID");

/// Environment variable that fixes `generated.at` when a request has none, for reproducible
/// output (tests, golden files).
pub const GENERATED_AT_ENV: &str = "REZZER_GENERATED_AT";

/// Text of the body paragraph for a format Rezzer detects but has no reader for.
pub const NO_READER_TEXT: &str = "Rezzer can't read this format yet.";
/// Text of the body paragraph for a format Rezzer does not know.
pub const UNKNOWN_TEXT: &str = "Rezzer doesn't know this format.";
/// Text of the body paragraph for an empty file.
pub const EMPTY_TEXT: &str = "The file is empty.";

/// What a run returns besides the Markdown pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputKind {
    #[default]
    Markdown,
    /// Also the model JSON (pretty-printed) in [`RunResult::json`].
    Json,
}

/// Options of one run. Debug hides the password.
#[derive(Clone)]
pub struct Request {
    pub output: OutputKind,
    /// The sections to render; `None` renders all.
    pub sections: Option<Vec<OutputSection>>,
    /// Show formulas also as LaTeX math. Default true. Only changes rendering, so it is not
    /// part of the cache key.
    pub math: bool,
    /// Used for encrypted files. A run with a password is never cached, so no decrypted
    /// content lands in the cache.
    pub password: Option<String>,
    /// RFC 3339 time for `generated.at`. When None: [`GENERATED_AT_ENV`], else now.
    pub generated_at: Option<String>,
    /// Cancelling it stops the reader at its next budget check; the result is then partial
    /// with a limit loss and is not cached.
    pub cancel: CancelToken,
}

impl Default for Request {
    fn default() -> Request {
        Request {
            output: OutputKind::default(),
            sections: None,
            math: true,
            password: None,
            generated_at: None,
            cancel: CancelToken::default(),
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("output", &self.output)
            .field("sections", &self.sections)
            .field("math", &self.math)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("generated_at", &self.generated_at)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish()
    }
}

/// The result of one run.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub completeness: Completeness,
    /// None only when the file could not be opened at all; `message` says why.
    pub document: Option<Document>,
    /// The rendered Markdown in pages; joined they are the whole output. Empty when there is
    /// no document.
    pub markdown_pages: Vec<String>,
    /// The model JSON, for [`OutputKind::Json`].
    pub json: Option<String>,
    /// Why a run failed, in one line.
    pub message: Option<String>,
}

impl RunResult {
    fn failed(message: String) -> RunResult {
        RunResult {
            completeness: Completeness::Failed,
            document: None,
            markdown_pages: Vec::new(),
            json: None,
            message: Some(message),
        }
    }
}

/// What the detector says about a file, without reading it. Serializes with the model's
/// names (`format` in snake_case, `detection` with `confidence` and `evidence`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Inspection {
    pub name: String,
    pub format: FormatId,
    pub detection: Detection,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    /// Id of the reader that would read it, if one is enabled.
    pub reader: Option<String>,
}

/// Detect, read, enrich, render and page, with the cache. Shareable between threads.
pub struct Pipeline {
    config: Config,
    registry: Registry,
    cache: Option<Cache>,
    detector: DetectFn,
}

/// A format detector as a plain function, see [`Pipeline::set_detector`].
pub type DetectFn = fn(&str, Option<&str>, &[u8]) -> (FormatId, Detection);

impl Pipeline {
    pub fn new(config: Config, registry: Registry) -> Pipeline {
        let cache = Cache::of(&config);
        Pipeline {
            config,
            registry,
            cache,
            detector: rezzer_detect::detect_with_hints,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Reads the file at `path`. A file that cannot be opened gives a failed result without
    /// a document. The source path is made canonical and absolute (see [`canonical_path`]).
    pub fn run_path(&self, path: &Path, req: &Request) -> RunResult {
        let path = canonical_path(path);
        match self.open(|d| rezzer_detect::open_with_detector(&path, &self.config.limits, d)) {
            Ok((src, detect_panic)) => self.run_source(&src, detect_panic, req),
            Err(e) => RunResult::failed(e.to_string()),
        }
    }

    /// Reads bytes already in memory (an upload, a data: URI). `name` and `mime` (the type
    /// the sender declared) are detection hints only; see [`rezzer_detect::detect_with_hints`].
    pub fn run_bytes(
        &self,
        name: &str,
        bytes: Vec<u8>,
        mime: Option<&str>,
        req: &Request,
    ) -> RunResult {
        let opened = self.open(|d| {
            rezzer_detect::from_bytes_with_detector(name, mime, bytes, &self.config.limits, d)
        });
        match opened {
            Ok((src, detect_panic)) => self.run_source(&src, detect_panic, req),
            Err(e) => RunResult::failed(e.to_string()),
        }
    }

    /// Opens and detects `path` without reading it.
    pub fn inspect(&self, path: &Path) -> Result<Inspection, OpenError> {
        let path = canonical_path(path);
        let (src, _) =
            self.open(|d| rezzer_detect::open_with_detector(&path, &self.config.limits, d))?;
        Ok(Inspection {
            reader: self
                .registry
                .for_format(src.format)
                .map(|r| r.id().to_string()),
            name: src.name,
            format: src.format,
            detection: src.detection,
            size: src.size,
            sha256: src.sha256,
        })
    }

    /// Replaces the format detector, for tests of the panic containment. The default is
    /// [`rezzer_detect::detect_with_hints`].
    #[doc(hidden)]
    pub fn set_detector(&mut self, detector: DetectFn) {
        self.detector = detector;
    }

    /// Opens a source with panic capture around opening and around detection. A detection
    /// panic gives the source (hash and size are known) with format unknown and the message;
    /// an open error is returned as it is, any other panic as [`OpenError::Internal`].
    fn open(
        &self,
        open: impl FnOnce(&rezzer_detect::Detector<'_>) -> Result<Source, OpenError>,
    ) -> Result<(Source, Option<String>), OpenError> {
        let detect_panic: RefCell<Option<String>> = RefCell::new(None);
        let detector = self.detector;
        let detect = |name: &str, mime: Option<&str>, bytes: &[u8]| match panics::contained(|| {
            detector(name, mime, bytes)
        }) {
            Ok(found) => found,
            Err(panic) => {
                let message = format!(
                    "detection stopped with an internal error: {}",
                    one_line(&panic)
                );
                *detect_panic.borrow_mut() = Some(message.clone());
                let detection = Detection {
                    confidence: Confidence::Guess,
                    evidence: message,
                };
                (FormatId::Unknown, detection)
            }
        };
        match panics::contained(|| open(&detect)) {
            Ok(Ok(src)) => Ok((src, detect_panic.into_inner())),
            Ok(Err(e)) => Err(e),
            Err(panic) => Err(OpenError::Internal(format!(
                "opening the file stopped with an internal error: {}",
                one_line(&panic)
            ))),
        }
    }

    fn run_source(&self, src: &Source, detect_panic: Option<String>, req: &Request) -> RunResult {
        let (document, completeness, message) = match detect_panic {
            Some(message) => {
                let mut document = Document::new(source_info(src));
                document.losses.push(Loss {
                    location: "document".into(),
                    what: format!("the whole file: {message}"),
                    why: LossReason::Internal,
                    severity: Severity::Error,
                });
                (document, Completeness::Failed, Some(message))
            }
            None => self.document(src, req),
        };
        let opts = RenderOptions {
            sections: req.sections.clone(),
            generated_at: generated_at(req),
            rezzer_version: VERSION.to_string(),
            archive_ref: None,
            math: req.math,
        };
        let page_tokens = self.config.limits.page_tokens;
        let rendered = panics::contained(|| {
            let markdown = rezzer_render::render(&document, completeness, &opts);
            rezzer_render::paginate(&markdown, page_tokens)
        });
        let markdown_pages = match rendered {
            Ok(pages) => pages,
            Err(panic) => {
                return RunResult {
                    completeness: Completeness::Failed,
                    document: Some(document),
                    markdown_pages: Vec::new(),
                    json: None,
                    message: Some(format!(
                        "rendering stopped with an internal error: {}",
                        one_line(&panic)
                    )),
                };
            }
        };
        let json = match req.output {
            OutputKind::Json => {
                Some(serde_json::to_string_pretty(&document).expect("the model serializes"))
            }
            OutputKind::Markdown => None,
        };
        RunResult {
            completeness,
            document: Some(document),
            markdown_pages,
            json,
            message,
        }
    }

    /// The enriched document, from the cache or freshly read.
    fn document(&self, src: &Source, req: &Request) -> (Document, Completeness, Option<String>) {
        let reader = self
            .registry
            .for_format(src.format)
            .map(|r| (r.id(), r.version()));
        let cache = self.cache.as_ref().filter(|_| req.password.is_none());
        let key = cache.map(|_| cache::key(src, reader, &self.config));
        if let (Some(cache), Some(key)) = (cache, &key)
            && let Some((mut document, completeness)) = cache.get(key, src)
        {
            // The same bytes and name under another path: the facts of this source count.
            let fresh = source_info(src);
            document.source.path = fresh.path;
            document.source.modified = fresh.modified;
            document.source.detection = fresh.detection;
            return (document, completeness, None);
        }
        let (mut document, mut completeness, message, cacheable) = self.read(src, req);
        let enriched =
            panics::contained(|| rezzer_enrich::enrich(&mut document, &self.config.enrich));
        if let Err(panic) = enriched {
            // The document stays usable (a pass may have stopped half way); say so.
            document.losses.push(Loss {
                location: "document".into(),
                what: format!(
                    "enrichment (overview, language, anchor check) stopped with an internal error: {}",
                    one_line(&panic)
                ),
                why: LossReason::Internal,
                severity: Severity::Warning,
            });
            if completeness == Completeness::Complete {
                completeness = Completeness::Partial;
            }
        } else if cacheable && let (Some(cache), Some(key)) = (cache, &key) {
            cache.put(key, &document, completeness);
        }
        (document, completeness, message)
    }

    /// Reads `src`. The last value says whether the result may be cached: not when the time
    /// limit cut the read short, because a time cut depends on the machine's load.
    fn read(&self, src: &Source, req: &Request) -> (Document, Completeness, Option<String>, bool) {
        if src.format == FormatId::Empty {
            let mut document = Document::new(source_info(src));
            document.body.push(paragraph(EMPTY_TEXT));
            return (document, Completeness::Complete, None, true);
        }
        let Some(reader) = self.registry.for_format(src.format) else {
            return (unsupported(src), Completeness::Partial, None, true);
        };
        let budget = Budget::with_cancel(self.config.limits.clone(), req.cancel.clone());
        let opts = ReadOptions {
            extract_assets: false,
            password: req.password.clone(),
        };
        match panics::contained(|| reader.read(src, &opts, &budget)) {
            Ok(outcome) => {
                let time_cut = budget.check_time().is_err()
                    && outcome
                        .document
                        .losses
                        .iter()
                        .any(|l| l.why == LossReason::Limit);
                (outcome.document, outcome.completeness, None, !time_cut)
            }
            Err(panic) => {
                let message = format!(
                    "the {} reader stopped with an internal error: {}",
                    reader.id(),
                    one_line(&panic)
                );
                let mut info = source_info(src);
                info.reader = Some(ReaderInfo {
                    id: reader.id().to_string(),
                    version: reader.version().to_string(),
                });
                let mut document = Document::new(info);
                document.losses.push(Loss {
                    location: "document".into(),
                    what: format!("the whole file: {message}"),
                    why: LossReason::Internal,
                    severity: Severity::Error,
                });
                (document, Completeness::Failed, Some(message), false)
            }
        }
    }
}

/// The canonical absolute form of `path`, so the same file read from two working directories
/// has one `resource`. The directory is canonicalized and the file name kept: a symlink keeps
/// its own name (the name is a detection hint and shown as the title). On any error the path
/// is returned as given.
pub fn canonical_path(path: &Path) -> PathBuf {
    let canonical = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            std::fs::canonicalize(parent).map(|p| p.join(name))
        }
        _ => std::fs::canonicalize(path),
    };
    canonical.unwrap_or_else(|_| path.to_path_buf())
}

/// A document for a format without a reader or an unknown format: the source facts, one
/// paragraph and an unsupported loss.
fn unsupported(src: &Source) -> Document {
    let mut document = Document::new(source_info(src));
    let (text, what) = if src.format == FormatId::Unknown {
        (UNKNOWN_TEXT, "files of an unknown format".to_string())
    } else {
        (
            NO_READER_TEXT,
            format!("{} files", rezzer_render::format_name(src.format)),
        )
    };
    document.body.push(paragraph(text));
    document.losses.push(Loss {
        location: "document".into(),
        what,
        why: LossReason::Unsupported,
        severity: Severity::Error,
    });
    document
}

fn paragraph(text: &str) -> Block {
    Block::paragraph(vec![Inline::plain(text)])
}

/// The request's time, else [`GENERATED_AT_ENV`], else now.
fn generated_at(req: &Request) -> String {
    req.generated_at
        .clone()
        .or_else(|| {
            std::env::var(GENERATED_AT_ENV)
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .or_else(|| rezzer_detect::rfc3339(SystemTime::now()))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}
