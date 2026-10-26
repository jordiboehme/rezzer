// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The work behind the tools, synchronous and free of MCP types: parameters in, an
//! [`Answer`] out. Bad input is an answer with status `failed` and a message that names
//! the rule, never an error.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use rezzer_model::{Completeness, Confidence};
use rezzer_pipeline::{
    CancelToken, OutputKind, OutputSection, Pipeline, Request, RunResult, TableStyle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Message for a `source` reference, until the source archive exists.
pub const SOURCE_NOT_AVAILABLE: &str = "The source archive is not available yet";
/// Message for an `mcp-file:` reference.
pub const MCP_FILE_NOT_SUPPORTED: &str = "mcp-file references are not supported yet";
/// Message when not exactly one input is given.
pub const EXACTLY_ONE_INPUT: &str = "Give exactly one of path, file or source";

/// The file name used for a data: URI without `;name=`.
const DEFAULT_NAME: &str = "file";

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
    /// the name is percent-encoded.
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
    /// Absolute path of an existing directory. Pictures (for example rendered PDF pages) are
    /// written to `<output_dir>/<file name>.assets/` and the Markdown links to them. Older
    /// files there that this read does not write are removed. Without it nothing is written.
    #[serde(default)]
    pub output_dir: Option<String>,
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
    let request = Request {
        output,
        sections,
        math: params.math.unwrap_or(true),
        password: params.password.take(),
        output_dir,
        cancel,
        table_style,
        sheets: params.sheets.take().unwrap_or_default(),
        ..Request::default()
    };
    let result = if let Some(path) = &params.path {
        pipeline.run_path(Path::new(path), &request)
    } else if let Some(file) = params.file.take() {
        let max = pipeline.config().limits.max_file_bytes;
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

/// A file sent inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileData {
    pub name: String,
    /// The type the sender declared, a detection hint only.
    pub mime: Option<String>,
    pub bytes: Vec<u8>,
}

/// Parses the `file` input: `data:[<type>][;name=<percent-encoded name>][;<other>=<value>]
/// [;base64],<data>`. The data is base64 or percent-encoded. The URI is consumed: the
/// decoded bytes are the only copy left. The decoded size is counted before anything is
/// copied, so a file larger than `max` bytes is refused without decoding it, and the
/// output buffer is allocated once at that size.
pub fn parse_file_uri(uri: String, max: u64) -> Result<FileData, String> {
    let trimmed = uri.trim();
    if starts_with_ignore_case(trimmed, "mcp-file:") {
        return Err(MCP_FILE_NOT_SUPPORTED.to_string());
    }
    if !starts_with_ignore_case(trimmed, "data:") {
        return Err(
            "file must be a data: URI, for example data:text/csv;name=a.csv;base64,<data>"
                .to_string(),
        );
    }
    let Some((header, data)) = trimmed["data:".len()..].split_once(',') else {
        return Err("the data: URI has no comma before the data".to_string());
    };
    let tokens: Vec<&str> = header.split(';').map(str::trim).collect();
    let mut mime = None;
    let mut name = None;
    let mut base64 = false;
    for (i, token) in tokens.iter().enumerate() {
        if i + 1 == tokens.len() && i > 0 && token.eq_ignore_ascii_case("base64") {
            base64 = true;
        } else if i == 0 {
            if token.contains('/') {
                mime = Some(token.to_ascii_lowercase());
            }
        } else if let Some((key, value)) = token.split_once('=')
            && key.trim().eq_ignore_ascii_case("name")
        {
            let decoded = percent_encoding::percent_decode_str(value.trim())
                .decode_utf8()
                .map_err(|_| "the name in the data: URI is not valid UTF-8".to_string())?;
            name = Some(file_name(&decoded));
        }
    }
    let size = if base64 {
        base64_decoded_len(data.as_bytes())
    } else {
        percent_decoded_len(data.as_bytes())
    };
    if size as u64 > max {
        return Err(format!(
            "file is too large: {size} bytes, the limit is {max} bytes"
        ));
    }
    let bytes = if base64 {
        decode_base64(data.as_bytes(), size)?
    } else {
        let mut bytes = Vec::with_capacity(size);
        bytes.extend(percent_encoding::percent_decode(data.as_bytes()));
        bytes
    };
    drop(uri);
    Ok(FileData {
        name: name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| DEFAULT_NAME.to_string()),
        mime,
        bytes,
    })
}

/// The size base64 text decodes to, counted without copying: whitespace and padding are
/// not symbols, every four symbols are three bytes. Exact for valid input.
fn base64_decoded_len(data: &[u8]) -> usize {
    let symbols = data
        .iter()
        .filter(|b| !b.is_ascii_whitespace() && **b != b'=')
        .count();
    symbols / 4 * 3 + (symbols % 4).saturating_sub(1)
}

/// The size percent-encoded text decodes to: every valid `%XX` escape is one byte, all
/// else stays as it is.
fn percent_decoded_len(data: &[u8]) -> usize {
    let mut len = 0;
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'%'
            && data.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
            && data.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            i += 3;
        } else {
            i += 1;
        }
        len += 1;
    }
    len
}

/// Symbols per decode step; a multiple of 4, so every step but the last is whole quads.
const BASE64_CHUNK: usize = 64 * 1024;

/// Decodes base64 into one buffer of `size` bytes (plus the decoder's small slack). Text
/// without whitespace is decoded in place; wrapped text goes through a small chunk buffer
/// that skips the whitespace, so the encoded text is never copied as a whole.
fn decode_base64(data: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let invalid = |e: base64::DecodeError, offset: usize| {
        let detail = match e {
            base64::DecodeError::InvalidByte(at, byte) => format!(
                "the character {:?} at symbol {} is not base64",
                char::from(byte),
                offset + at + 1
            ),
            base64::DecodeError::InvalidLength(_) => {
                "the length is not valid, the data may be cut off".to_string()
            }
            base64::DecodeError::InvalidLastSymbol { .. } => {
                "the last symbol is not valid, the data may be cut off".to_string()
            }
            base64::DecodeError::InvalidPadding => "the padding is not valid".to_string(),
        };
        format!("the base64 data in the data: URI is not valid: {detail}")
    };
    // decode_vec reserves its estimate, at most two bytes over the exact size.
    let mut out = Vec::with_capacity(size + 3);
    if !data.iter().any(u8::is_ascii_whitespace) {
        BASE64
            .decode_vec(data, &mut out)
            .map_err(|e| invalid(e, 0))?;
        return Ok(out);
    }
    let mut chunk = Vec::with_capacity(BASE64_CHUNK);
    let mut offset = 0;
    for &b in data {
        if b.is_ascii_whitespace() {
            continue;
        }
        chunk.push(b);
        if chunk.len() == BASE64_CHUNK {
            BASE64
                .decode_vec(&chunk, &mut out)
                .map_err(|e| invalid(e, offset))?;
            offset += chunk.len();
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        BASE64
            .decode_vec(&chunk, &mut out)
            .map_err(|e| invalid(e, offset))?;
    }
    Ok(out)
}

/// Standard base64; padding may be there or not.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// The last component of a sent name, so a name like `../x/a.csv` is only `a.csv`.
fn file_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run that has a document but no pages, as after a render panic.
    fn run_without_pages(output: OutputKind) -> RunResult {
        let mut registry = rezzer_model::Registry::new();
        registry.register(Box::new(rezzer_read_csv::CsvReader));
        let config = rezzer_pipeline::Config {
            cache: false,
            ..rezzer_pipeline::Config::default()
        };
        let pipeline = Pipeline::new(config, registry);
        let request = Request {
            output,
            ..Request::default()
        };
        let mut result = pipeline.run_bytes("a.csv", b"a,b\n1,2\n".to_vec(), None, &request);
        assert!(result.document.is_some());
        result.completeness = Completeness::Failed;
        result.markdown_pages.clear();
        result.json = None;
        result.message = Some("rendering stopped with an internal error: boom".into());
        result
    }

    #[test]
    fn a_run_without_pages_answers_with_its_message() {
        for output in [OutputKind::Markdown, OutputKind::Json] {
            let answer = rez_answer(run_without_pages(output), output, 1);
            let message = "rendering stopped with an internal error: boom";
            assert!(answer.failed);
            assert_eq!(answer.text, message, "{output:?}");
            let s = &answer.structured;
            assert_eq!(s["status"], "failed");
            assert_eq!(s["message"], message);
            assert_eq!(s["pages"], 0);
            assert_eq!(s["page"], Value::Null);
            assert_eq!(s["format"], "csv");
            assert_eq!(s["sha256"].as_str().unwrap().len(), 64);
        }
    }

    #[test]
    fn wrapped_base64_across_chunks_decodes_into_one_exact_buffer() {
        use base64::Engine as _;
        let raw: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
        let wrapped: Vec<&str> = encoded
            .as_bytes()
            .chunks(76)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect();
        let uri = format!("data:;base64,{}", wrapped.join("\r\n"));
        let data = parse_file_uri(uri, u64::MAX).unwrap();
        assert_eq!(data.bytes, raw);
        assert!(
            data.bytes.capacity() <= raw.len() + 3,
            "{}",
            data.bytes.capacity()
        );
        let flat = parse_file_uri(format!("data:;base64,{encoded}"), u64::MAX).unwrap();
        assert_eq!(flat.bytes, raw);
        assert!(flat.bytes.capacity() <= raw.len() + 3);
    }

    #[test]
    fn decoded_sizes_are_exact() {
        assert_eq!(base64_decoded_len(b"YSxiCg=="), 4);
        assert_eq!(base64_decoded_len(b"YSxi\nCg"), 4);
        assert_eq!(base64_decoded_len(b"aGk"), 2);
        assert_eq!(base64_decoded_len(b""), 0);
        assert_eq!(percent_decoded_len(b"a%2Cb%0A%zz%4"), 9);
        let data = parse_file_uri("data:,a%2Cb%0A%zz%4".into(), 100).unwrap();
        assert_eq!(data.bytes, b"a,b\n%zz%4");
        assert_eq!(data.bytes.capacity(), data.bytes.len());
    }

    #[test]
    fn a_bad_symbol_is_named_with_its_position_after_a_chunk() {
        let mut encoded = "A".repeat(BASE64_CHUNK + 8);
        encoded.insert(BASE64_CHUNK + 4, '$');
        let err = parse_file_uri(format!("data:;base64,{encoded}\n"), u64::MAX).unwrap_err();
        assert!(
            err.contains(&format!("at symbol {}", BASE64_CHUNK + 5)),
            "{err}"
        );
        assert!(err.ends_with("not base64"), "{err}");
    }

    #[test]
    fn data_uri_with_type_name_and_base64() {
        let data = parse_file_uri(
            "data:text/csv;name=a%20b.csv;base64,YSxiCg==".to_string(),
            100,
        )
        .unwrap();
        assert_eq!(data.name, "a b.csv");
        assert_eq!(data.mime.as_deref(), Some("text/csv"));
        assert_eq!(data.bytes, b"a,b\n");
    }

    #[test]
    fn data_uri_without_type_or_name() {
        let data = parse_file_uri("data:,hello%20world".to_string(), 100).unwrap();
        assert_eq!(data.name, DEFAULT_NAME);
        assert_eq!(data.mime, None);
        assert_eq!(data.bytes, b"hello world");
    }

    #[test]
    fn data_uri_ignores_other_parameters_and_unpadded_base64() {
        let data = parse_file_uri(
            "DATA:text/plain;charset=utf-8;name=x.txt;BASE64,aGk".into(),
            100,
        )
        .unwrap();
        assert_eq!(data.name, "x.txt");
        assert_eq!(data.mime.as_deref(), Some("text/plain"));
        assert_eq!(data.bytes, b"hi");
    }

    #[test]
    fn data_uri_name_keeps_only_the_file_name() {
        let data = parse_file_uri("data:;name=..%2Fsecret%2Fa.csv,x".to_string(), 100).unwrap();
        assert_eq!(data.name, "a.csv");
    }

    #[test]
    fn data_uri_base64_is_a_parameter_only_at_the_end() {
        // `base64` as the type position is not the encoding flag.
        let data = parse_file_uri("data:base64,aGk".to_string(), 100).unwrap();
        assert_eq!(data.bytes, b"aGk");
    }

    #[test]
    fn oversize_data_is_refused_before_decoding() {
        let big = format!("data:;base64,{}", "A".repeat(4000));
        let err = parse_file_uri(big, 100).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        let err = parse_file_uri("data:,12345".to_string(), 4).unwrap_err();
        assert!(err.contains("5 bytes, the limit is 4 bytes"), "{err}");
    }
}
