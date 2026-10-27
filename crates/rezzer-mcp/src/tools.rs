// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The work behind the tools, synchronous and free of MCP types: parameters in, an
//! [`Answer`] out. Bad input is an answer with status `failed` and a message that names
//! the rule, never an error.

mod file_uri;

use std::path::{Path, PathBuf};

use rezzer_model::{Completeness, Confidence};
use rezzer_pipeline::{
    CancelToken, OutputKind, OutputSection, Pipeline, Request, RunResult, TableStyle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub use file_uri::{FileData, parse_file_uri};

/// Message for a `source` reference, until the source archive exists.
pub const SOURCE_NOT_AVAILABLE: &str = "The source archive is not available yet";
/// Message for an `mcp-file:` reference.
pub const MCP_FILE_NOT_SUPPORTED: &str = "mcp-file references are not supported yet";
/// Message when not exactly one input is given.
pub const EXACTLY_ONE_INPUT: &str = "Give exactly one of path, file or source";

// Parameters of the `rez` tool, and the tool's input schema (the doc comment below is its
// description). The arguments are read by hand ([`RezParams::from_arguments`]) and
// leniently, so a wrong value or type becomes a failed result with structured content and a
// message, never a protocol error.
/// Read a file: give exactly one of path, file or source.
#[derive(Default, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct RezParams {
    /// Path of a local file, absolute or relative to the server's working directory.
    #[serde(default)]
    pub path: Option<String>,
    /// The file itself as a data: URI, for example
    /// `data:text/csv;name=budget.csv;base64,<data>`. The type and the name are optional;
    /// the name is percent-encoded. The file may be up to maxSize bytes (32 MiB by default);
    /// give a larger file by path.
    #[serde(default)]
    #[schemars(extend("format" = "uri"))]
    pub file: Option<String>,
    /// A rezzer:// reference to a file in the source archive (not available yet).
    #[serde(default)]
    pub source: Option<String>,
    /// The page to return, starting at 1. Default 1. The result says how many pages there are.
    #[serde(default)]
    pub page: Option<i64>,
    /// Only these sections: overview, properties, content, logic, annotations, changes,
    /// embedded, not_read. Default all.
    #[serde(default)]
    pub sections: Option<Vec<String>>,
    /// Show formulas also as LaTeX math in the Logic section. Default true.
    #[serde(default)]
    #[schemars(extend("default" = true))]
    pub math: Option<bool>,
    /// `markdown` (default) or `json` (the document model, one page).
    #[serde(default)]
    #[schemars(schema_with = "format_schema", extend("default" = "markdown"))]
    pub format: Option<String>,
    /// Absolute path of an existing directory. The Markdown is written there too, as
    /// <file name>.md. Some names get a short hash added. A document over part_size is
    /// written as <file name>.md plus parts in a folder next to it, <file name>.parts.
    /// Pictures (for example rendered PDF pages) go to a folder next to it,
    /// <file name>.assets, and the Markdown links to them. In these folder names every
    /// character other than A to Z, a to z, 0 to 9, '.', '_' and '-' becomes '_', and the
    /// name then gets a short hash: 'Q3 report.pdf' gives Q3_report.pdf-6f38fa03.parts.
    /// Older files that Rezzer wrote there and this read does not write are removed. Rezzer
    /// only replaces or removes files it wrote for the same source. Anything else in the way
    /// stops the read. This includes the output of another file with the same name and
    /// other content. It also includes a name that differs only in case, on a file system
    /// that ignores case. Without it nothing is written.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Largest part in tokens when output_dir is set. A larger document is written as a main
    /// document plus parts, and the main document is returned. 0 writes one file. Default
    /// 10000.
    #[serde(default)]
    #[schemars(range(min = 0), extend("default" = 10000))]
    pub part_size: Option<i64>,
    /// The password of an encrypted file. It is never stored or cached.
    #[serde(default)]
    pub password: Option<String>,
    /// How sheet tables are written: `table` (default) as Markdown tables, or `records` as
    /// one list item per row with the column header of each value. Markdown only: `json`
    /// gives the full model.
    #[serde(default)]
    #[schemars(schema_with = "table_style_schema", extend("default" = "table"))]
    pub table_style: Option<String>,
    /// Only these sheets, by name or by position as `#1`, `#2` and so on. Default all.
    /// Markdown only: `json` gives all sheets.
    #[serde(default)]
    pub sheets: Option<Vec<String>>,
}

fn table_style_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "description": "How sheet tables are written: `table` (default) as Markdown tables, or `records` as one list item per row with the column header of each value.",
        "type": "string",
        "enum": ["table", "records"],
        "default": "table",
    })
}

fn format_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "description": "`markdown` (default) or `json` (the document model, one page).",
        "type": "string",
        "enum": ["markdown", "json"],
        "default": "markdown",
    })
}

// Debug hides the password.
impl std::fmt::Debug for RezParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RezParams")
            .field("path", &self.path)
            .field("file", &self.file.as_ref().map(|_| "<data>"))
            .field("source", &self.source)
            .field("page", &self.page)
            .field("sections", &self.sections)
            .field("math", &self.math)
            .field("format", &self.format)
            .field("output_dir", &self.output_dir)
            .field("part_size", &self.part_size)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("table_style", &self.table_style)
            .field("sheets", &self.sheets)
            .finish()
    }
}

impl RezParams {
    /// Reads the tool arguments. A missing or null field is absent. `page` takes an integer,
    /// an integral number (`2.0`) or a numeric string (`"2"`); unknown keys are ignored.
    pub fn from_arguments(mut args: Map<String, Value>) -> Result<RezParams, String> {
        let page = match take(&mut args, "page") {
            None => None,
            Some(value) => Some(integer(&value).ok_or_else(|| {
                format!(
                    "page must be an integer (1 or higher), not {}",
                    shown(&value)
                )
            })?),
        };
        let sections = match take(&mut args, "sections") {
            None => None,
            Some(Value::Array(items)) => Some(
                items
                    .into_iter()
                    .map(|item| match item {
                        Value::String(name) => Ok(name),
                        other => Err(format!(
                            "sections must be a list of section names, but it holds {}",
                            shown(&other)
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Some(other) => {
                return Err(format!(
                    "sections must be a list of section names, not {}",
                    shown(&other)
                ));
            }
        };
        let sheets = match take(&mut args, "sheets") {
            None => None,
            Some(Value::Array(items)) => Some(
                items
                    .into_iter()
                    .map(|item| match item {
                        Value::String(name) => Ok(name),
                        other => Err(format!(
                            "sheets must be a list of sheet names, but it holds {}",
                            shown(&other)
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Some(other) => {
                return Err(format!(
                    "sheets must be a list of sheet names, not {}",
                    shown(&other)
                ));
            }
        };
        let part_size = match take(&mut args, "part_size") {
            None => None,
            Some(value) => Some(integer(&value).filter(|n| *n >= 0).ok_or_else(|| {
                format!(
                    "part_size must be an integer (0 or higher), not {}",
                    shown(&value)
                )
            })?),
        };
        let math = match take(&mut args, "math") {
            None => None,
            Some(Value::Bool(b)) => Some(b),
            Some(Value::String(s)) if s.trim() == "true" => Some(true),
            Some(Value::String(s)) if s.trim() == "false" => Some(false),
            Some(other) => {
                return Err(format!("math must be true or false, not {}", shown(&other)));
            }
        };
        Ok(RezParams {
            path: string(&mut args, "path")?,
            file: string(&mut args, "file")?,
            source: string(&mut args, "source")?,
            page,
            sections,
            math,
            format: string(&mut args, "format")?,
            output_dir: string(&mut args, "output_dir")?,
            part_size,
            password: string(&mut args, "password")?,
            table_style: string(&mut args, "table_style")?,
            sheets,
        })
    }
}

// Parameters of the `inspect` tool, and its input schema.
/// Inspect a local file.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct InspectParams {
    /// Path of a local file, absolute or relative to the server's working directory.
    #[serde(default)]
    pub path: Option<String>,
}

impl InspectParams {
    /// Reads the tool arguments, like [`RezParams::from_arguments`].
    pub fn from_arguments(mut args: Map<String, Value>) -> Result<InspectParams, String> {
        Ok(InspectParams {
            path: string(&mut args, "path")?,
        })
    }
}

/// The value of `key`, with null as absent. The value is moved out, never copied.
fn take(args: &mut Map<String, Value>, key: &str) -> Option<Value> {
    args.remove(key).filter(|value| !value.is_null())
}

fn string(args: &mut Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match take(args, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        // The value of a password is never echoed.
        Some(_) if key == "password" => Err("password must be a string".to_string()),
        Some(other) => Err(format!("{key} must be a string, not {}", shown(&other))),
    }
}

/// An integer from a JSON integer, an integral number or a numeric string. Values beyond
/// the i64 range saturate (a page that large does not exist anyway).
fn integer(value: &Value) -> Option<i64> {
    fn from_f64(f: f64) -> Option<i64> {
        // `as` saturates, which is what a page number wants.
        (f.is_finite() && f.fract() == 0.0).then_some(f as i64)
    }
    match value {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|_| i64::MAX))
            .or_else(|| n.as_f64().and_then(from_f64)),
        Value::String(s) => {
            let s = s.trim();
            s.parse::<i64>()
                .ok()
                .or_else(|| s.parse::<f64>().ok().and_then(from_f64))
        }
        _ => None,
    }
}

/// A short description of a wrong value for a message: its JSON type, and the value when
/// it is short.
fn shown(value: &Value) -> String {
    let kind = match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    };
    let text = value.to_string();
    if text.chars().count() <= 40 && !matches!(value, Value::Array(_) | Value::Object(_)) {
        format!("{kind} ({text})")
    } else {
        kind.to_string()
    }
}

/// What a tool call returns: the text block, the structured content (a JSON object) and
/// whether the call failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub text: String,
    pub structured: Value,
    pub failed: bool,
}

/// The `rez` tool. `cancel` stops a running read at its next budget check.
pub fn rez(pipeline: &Pipeline, args: Map<String, Value>, cancel: CancelToken) -> Answer {
    let mut params = match RezParams::from_arguments(args) {
        Ok(params) => params,
        Err(message) => return rez_failed(message),
    };
    let inputs = [&params.path, &params.file, &params.source]
        .iter()
        .filter(|input| input.is_some())
        .count();
    if inputs != 1 {
        return rez_failed(EXACTLY_ONE_INPUT.to_string());
    }
    let output = match params.format.as_deref().map(str::trim) {
        None | Some("markdown") => OutputKind::Markdown,
        Some("json") => OutputKind::Json,
        Some(other) => {
            return rez_failed(format!("format must be markdown or json, not {other:?}"));
        }
    };
    let page = match params.page {
        None => 1,
        Some(n) if n >= 1 => usize::try_from(n).unwrap_or(usize::MAX),
        Some(n) => return rez_failed(format!("page must be 1 or higher, not {n}")),
    };
    let sections = match params.sections.take() {
        None => None,
        Some(names) => match names
            .iter()
            .map(|name| name.parse::<OutputSection>())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(sections) => Some(sections),
            Err(message) => return rez_failed(message),
        },
    };
    let table_style = match params.table_style.as_deref() {
        None => None,
        Some(text) => match text.parse::<TableStyle>() {
            Ok(style) => Some(style),
            Err(e) => return rez_failed(format!("table_style: {e}")),
        },
    };
    let output_dir = match params.output_dir.take() {
        None => None,
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if !dir.is_absolute() || !dir.is_dir() {
                return rez_failed(
                    "output_dir must be an absolute path of an existing directory".to_string(),
                );
            }
            Some(dir)
        }
    };
    let part_size = match params.part_size {
        None => None,
        Some(_) if output_dir.is_none() => {
            return rez_failed("part_size needs output_dir".to_string());
        }
        Some(n) => {
            let n = usize::try_from(n).unwrap_or(usize::MAX);
            if let Err(message) = rezzer_pipeline::check_part_size(n) {
                return rez_failed(message);
            }
            Some(n)
        }
    };
    let request = Request {
        output,
        sections,
        math: params.math.unwrap_or(true),
        password: params.password.take(),
        output_dir,
        part_size,
        cancel,
        table_style,
        sheets: params.sheets.take().unwrap_or_default(),
        ..Request::default()
    };
    let result = if let Some(path) = &params.path {
        pipeline.run_path(Path::new(path), &request)
    } else if let Some(file) = params.file.take() {
        let max = pipeline.config().limits.inline_bytes();
        // The encoded text is consumed here, so it is gone before the pipeline runs.
        match parse_file_uri(file, max) {
            Ok(data) => pipeline.run_bytes(&data.name, data.bytes, data.mime.as_deref(), &request),
            Err(message) => return rez_failed(message),
        }
    } else {
        return rez_failed(SOURCE_NOT_AVAILABLE.to_string());
    };
    rez_answer(result, output, page)
}

/// The structured content of a `rez` result. Also the tool's output schema.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RezOutput {
    /// complete, partial or failed.
    #[schemars(extend("enum" = ["complete", "partial", "failed"]))]
    pub status: &'static str,
    /// The page in the text, starting at 1; null when there is none.
    pub page: Option<usize>,
    /// How many pages the result has; 0 when nothing could be read.
    pub pages: usize,
    /// The detected format of the file, for example csv; null when nothing could be read.
    pub format: Option<String>,
    /// SHA-256 of the file, lowercase hex; null when nothing could be read.
    pub sha256: Option<String>,
    /// How many parts of the file could not be read fully.
    pub losses: usize,
    /// How many pictures the file has (extracted or not).
    pub assets: usize,
    /// Ids of the pictures Rezzer cannot interpret, for the calling agent to look at. With
    /// `output_dir` they are files in the bundle.
    pub needs_vision: Vec<String>,
    /// Why the call failed or what went wrong, in one line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The parts written next to the main document, in order, with their paths relative to
    /// output_dir. Empty when the document is one file or nothing was written.
    pub parts: Vec<RezPart>,
    /// Absolute path of the main document written to output_dir; null without output_dir.
    pub main_file: Option<String>,
}

/// One part file in the `rez` structured content.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RezPart {
    /// Relative to output_dir, for example `report.pdf.parts/introduction.md`.
    pub path: String,
    /// The part's title.
    pub title: String,
    /// Where the part lies in the document, for example `pages 12 to 20`.
    pub location: String,
}

/// The parts and the main file of a run, for [`RezOutput`].
fn written(result: &RunResult) -> (Vec<RezPart>, Option<String>) {
    let parts = result
        .parts
        .iter()
        .map(|p| RezPart {
            path: p.path.clone(),
            title: p.title.clone(),
            location: p.location.clone(),
        })
        .collect();
    let main_file = result
        .main_file
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    (parts, main_file)
}

/// The answer for a run: the requested page (or the JSON), else the reason it failed.
fn rez_answer(result: RunResult, output: OutputKind, page: usize) -> Answer {
    let Some(document) = &result.document else {
        return rez_failed(
            result
                .message
                .unwrap_or_else(|| "The file could not be read".to_string()),
        );
    };
    let rendered = match output {
        OutputKind::Json => result.json.is_some(),
        OutputKind::Markdown => !result.markdown_pages.is_empty(),
    };
    if !rendered {
        // Nothing to show, as after a render panic: the pipeline's reason is the answer.
        let message = result
            .message
            .clone()
            .unwrap_or_else(|| "The file was read, but no output came out of it".to_string());
        let (parts, main_file) = written(&result);
        let structured = RezOutput {
            status: "failed",
            page: None,
            pages: 0,
            format: Some(rezzer_render::format_name(document.source.format)),
            sha256: Some(document.source.sha256.clone()),
            losses: document.losses.len(),
            assets: document.assets.len(),
            needs_vision: needs_vision(document),
            message: Some(message.clone()),
            parts,
            main_file,
        };
        return Answer {
            text: message,
            structured: to_value(&structured),
            failed: true,
        };
    }
    let (text, pages) = match (output, &result.json) {
        (OutputKind::Json, Some(json)) => (Some(json.clone()), 1),
        _ => {
            let pages = result.markdown_pages.len();
            let text = page
                .checked_sub(1)
                .and_then(|i| result.markdown_pages.get(i))
                .map(|markdown| with_paging_line(markdown, page, pages));
            (text, pages)
        }
    };
    let (parts, main_file) = written(&result);
    let mut structured = RezOutput {
        status: status(result.completeness),
        page: Some(page),
        pages,
        format: Some(rezzer_render::format_name(document.source.format)),
        sha256: Some(document.source.sha256.clone()),
        losses: document.losses.len(),
        assets: document.assets.len(),
        needs_vision: needs_vision(document),
        message: result.message.clone(),
        parts,
        main_file,
    };
    let Some(text) = text.filter(|_| page <= pages) else {
        let noun = if pages == 1 { "page" } else { "pages" };
        let message = format!("page {page} does not exist: the output has {pages} {noun}");
        structured.status = "failed";
        structured.page = None;
        structured.message = Some(message.clone());
        return Answer {
            text: message,
            structured: to_value(&structured),
            failed: true,
        };
    };
    Answer {
        text,
        structured: to_value(&structured),
        failed: result.completeness == Completeness::Failed,
    }
}

/// The Markdown page, unchanged, then for a result of several pages a blank line and one
/// line that says which page this is and how to get the next one.
fn with_paging_line(markdown: &str, page: usize, pages: usize) -> String {
    if pages <= 1 {
        return markdown.to_string();
    }
    let mut text = markdown.to_string();
    if !text.ends_with("\n\n") {
        text.push_str(if text.ends_with('\n') { "\n" } else { "\n\n" });
    }
    if page < pages {
        text.push_str(&format!(
            "Page {page} of {pages}. Call rez again with page: {} for the next page.\n",
            page + 1
        ));
    } else {
        text.push_str(&format!("Page {pages} of {pages}.\n"));
    }
    text
}

/// A failed `rez` answer without a document: the message is the text.
fn rez_failed(message: String) -> Answer {
    let structured = RezOutput {
        status: "failed",
        page: None,
        pages: 0,
        format: None,
        sha256: None,
        losses: 0,
        assets: 0,
        needs_vision: Vec::new(),
        message: Some(message.clone()),
        parts: Vec::new(),
        main_file: None,
    };
    Answer {
        structured: to_value(&structured),
        text: message,
        failed: true,
    }
}

fn needs_vision(document: &rezzer_model::Document) -> Vec<String> {
    document
        .assets
        .iter()
        .filter(|a| a.needs_vision)
        .map(|a| a.id.clone())
        .collect()
}

fn to_value(output: &RezOutput) -> Value {
    serde_json::to_value(output).expect("the rez output serializes")
}

fn status(completeness: Completeness) -> &'static str {
    match completeness {
        Completeness::Complete => "complete",
        Completeness::Partial => "partial",
        Completeness::Failed => "failed",
    }
}

/// The `inspect` tool.
pub fn inspect(pipeline: &Pipeline, args: Map<String, Value>) -> Answer {
    let params = match InspectParams::from_arguments(args) {
        Ok(params) => params,
        Err(message) => return simple_failed(message),
    };
    let Some(path) = params.path else {
        return simple_failed("inspect needs a path".to_string());
    };
    let info = match pipeline.inspect(Path::new(&path)) {
        Ok(info) => info,
        Err(e) => return simple_failed(e.to_string()),
    };
    let format = rezzer_render::format_name(info.format);
    let confidence = match info.detection.confidence {
        Confidence::Certain => "certain",
        Confidence::Likely => "likely",
        Confidence::Guess => "guess",
    };
    let text = format!(
        "name: {}\nformat: {format}\nconfidence: {confidence}\nevidence: {}\nsize: {} bytes\nsha256: {}\nreader: {}\n",
        info.name,
        info.detection.evidence,
        info.size,
        info.sha256,
        info.reader
            .as_deref()
            .unwrap_or("none (this build cannot read the format)"),
    );
    Answer {
        text,
        structured: serde_json::to_value(&info).expect("an inspection serializes"),
        failed: false,
    }
}

/// The `formats` tool.
pub fn formats(pipeline: &Pipeline) -> Answer {
    let rows: Vec<(String, &str, &str)> = pipeline
        .registry()
        .list()
        .into_iter()
        .map(|(format, reader, version)| (rezzer_render::format_name(format), reader, version))
        .collect();
    let text = if rows.is_empty() {
        "This build has no readers.\n".to_string()
    } else {
        rows.iter()
            .map(|(format, reader, version)| format!("{format}: read by {reader} {version}\n"))
            .collect()
    };
    let list: Vec<Value> = rows
        .iter()
        .map(|(format, reader, version)| {
            json!({ "format": format, "reader": reader, "version": version })
        })
        .collect();
    Answer {
        text,
        structured: json!({ "formats": list }),
        failed: false,
    }
}

/// A failed answer for the tools without pages.
pub fn simple_failed(message: String) -> Answer {
    Answer {
        structured: json!({ "status": "failed", "message": message }),
        text: message,
        failed: true,
    }
}

#[cfg(test)]
mod tests;
