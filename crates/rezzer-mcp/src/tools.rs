// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The work behind the tools, synchronous and free of MCP types: parameters in, an
//! [`Answer`] out. Bad input is an answer with status `failed` and a message that names
//! the rule, never an error.

use std::path::Path;

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use rezzer_model::{Completeness, Confidence};
use rezzer_pipeline::{OutputKind, OutputSection, Pipeline, Request, RunResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Message for a `source` reference, until the source archive exists.
pub const SOURCE_NOT_AVAILABLE: &str = "The source archive is not available yet.";
/// Message for an `mcp-file:` reference.
pub const MCP_FILE_NOT_SUPPORTED: &str = "mcp-file references are not supported yet.";
/// Message when not exactly one input is given.
pub const EXACTLY_ONE_INPUT: &str = "Give exactly one of path, file or source.";

/// The file name used for a data: URI without `;name=`.
const DEFAULT_NAME: &str = "file";

/// Parameters of the `rez` tool. Every field is optional on the wire so that a wrong
/// combination or value becomes a failed result with a clear message, not a protocol error.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RezParams {
    /// Path of a local file, absolute or relative to the server's working directory.
    #[serde(default)]
    pub path: Option<String>,
    /// The file itself as a data: URI, for example
    /// `data:text/csv;name=budget.csv;base64,<data>`. The type and the name are optional;
    /// the name is percent-encoded.
    #[serde(default)]
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
    /// `markdown` (default) or `json` (the document model, one page).
    #[serde(default)]
    #[schemars(extend("enum" = ["markdown", "json"]))]
    pub format: Option<String>,
}

/// Parameters of the `inspect` tool.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct InspectParams {
    /// Path of a local file, absolute or relative to the server's working directory.
    #[serde(default)]
    pub path: Option<String>,
}

/// What a tool call returns: the text block, the structured content (a JSON object) and
/// whether the call failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub text: String,
    pub structured: Value,
    pub failed: bool,
}

/// The `rez` tool.
pub fn rez(pipeline: &Pipeline, params: RezParams) -> Answer {
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
            return rez_failed(format!("format must be markdown or json, not {other:?}."));
        }
    };
    let page = match params.page {
        None => 1,
        Some(n) if n >= 1 => usize::try_from(n).unwrap_or(usize::MAX),
        Some(n) => return rez_failed(format!("page must be 1 or higher, not {n}.")),
    };
    let sections = match params.sections {
        None => None,
        Some(names) => match names
            .iter()
            .map(|name| name.parse::<OutputSection>())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(sections) => Some(sections),
            Err(message) => return rez_failed(format!("{message}.")),
        },
    };
    let request = Request {
        output,
        sections,
        ..Request::default()
    };
    let result = if let Some(path) = &params.path {
        pipeline.run_path(Path::new(path), &request)
    } else if let Some(file) = &params.file {
        let max = pipeline.config().limits.max_file_bytes;
        match parse_file_uri(file, max) {
            Ok(data) => pipeline.run_bytes(&data.name, data.bytes, data.mime.as_deref(), &request),
            Err(message) => return rez_failed(message),
        }
    } else {
        return rez_failed(SOURCE_NOT_AVAILABLE.to_string());
    };
    rez_answer(result, output, page)
}

/// The answer for a run: the requested page (or the JSON), else the reason it failed.
fn rez_answer(result: RunResult, output: OutputKind, page: usize) -> Answer {
    let Some(document) = &result.document else {
        return rez_failed(
            result
                .message
                .unwrap_or_else(|| "The file could not be read.".to_string()),
        );
    };
    let mut structured = Map::new();
    structured.insert("status".into(), json!(status(result.completeness)));
    structured.insert(
        "format".into(),
        json!(rezzer_render::format_name(document.source.format)),
    );
    structured.insert("sha256".into(), json!(document.source.sha256));
    structured.insert("losses".into(), json!(document.losses.len()));
    let (text, pages) = match (output, &result.json) {
        (OutputKind::Json, Some(json)) => (Some(json.as_str()), 1),
        _ => (
            page.checked_sub(1)
                .and_then(|i| result.markdown_pages.get(i))
                .map(String::as_str),
            result.markdown_pages.len(),
        ),
    };
    structured.insert("pages".into(), json!(pages));
    let Some(text) = text.filter(|_| page <= pages) else {
        let noun = if pages == 1 { "page" } else { "pages" };
        let message = format!("page {page} does not exist: the output has {pages} {noun}.");
        structured.insert("status".into(), json!("failed"));
        structured.insert("page".into(), Value::Null);
        structured.insert("message".into(), json!(message));
        return Answer {
            text: message,
            structured: Value::Object(structured),
            failed: true,
        };
    };
    structured.insert("page".into(), json!(page));
    if let Some(message) = &result.message {
        structured.insert("message".into(), json!(message));
    }
    Answer {
        text: text.to_string(),
        structured: Value::Object(structured),
        failed: result.completeness == Completeness::Failed,
    }
}

/// A failed `rez` answer without a document: the message is the text.
fn rez_failed(message: String) -> Answer {
    Answer {
        structured: json!({
            "status": "failed",
            "page": null,
            "pages": 0,
            "format": null,
            "sha256": null,
            "losses": 0,
            "message": message,
        }),
        text: message,
        failed: true,
    }
}

fn status(completeness: Completeness) -> &'static str {
    match completeness {
        Completeness::Complete => "complete",
        Completeness::Partial => "partial",
        Completeness::Failed => "failed",
    }
}

/// The `inspect` tool.
pub fn inspect(pipeline: &Pipeline, params: InspectParams) -> Answer {
    let Some(path) = params.path else {
        return simple_failed("inspect needs a path.".to_string());
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
/// [;base64],<data>`. The data is base64 or percent-encoded. A file larger than `max` bytes
/// is refused, before decoding when the encoded length already shows it.
pub fn parse_file_uri(uri: &str, max: u64) -> Result<FileData, String> {
    let uri = uri.trim();
    if starts_with_ignore_case(uri, "mcp-file:") {
        return Err(MCP_FILE_NOT_SUPPORTED.to_string());
    }
    if !starts_with_ignore_case(uri, "data:") {
        return Err(
            "file must be a data: URI, for example data:text/csv;name=a.csv;base64,<data>."
                .to_string(),
        );
    }
    let Some((header, data)) = uri["data:".len()..].split_once(',') else {
        return Err("the data: URI has no comma before the data.".to_string());
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
                .map_err(|_| "the name in the data: URI is not valid UTF-8.".to_string())?;
            name = Some(file_name(&decoded));
        }
    }
    let too_large =
        |size: u64| format!("file is too large: {size} bytes, the limit is {max} bytes");
    let bytes = if base64 {
        let encoded: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        // At least this many bytes come out of the encoded text.
        let at_least = (encoded.len() as u64 / 4 * 3).saturating_sub(2);
        if at_least > max {
            return Err(too_large(at_least));
        }
        BASE64
            .decode(&encoded)
            .map_err(|e| format!("the base64 data in the data: URI is not valid: {e}."))?
    } else {
        // A percent escape is three characters for one byte.
        let at_least = data.len() as u64 / 3;
        if at_least > max {
            return Err(too_large(at_least));
        }
        percent_encoding::percent_decode_str(data).collect()
    };
    if bytes.len() as u64 > max {
        return Err(too_large(bytes.len() as u64));
    }
    Ok(FileData {
        name: name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| DEFAULT_NAME.to_string()),
        mime,
        bytes,
    })
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

    #[test]
    fn data_uri_with_type_name_and_base64() {
        let data = parse_file_uri("data:text/csv;name=a%20b.csv;base64,YSxiCg==", 100).unwrap();
        assert_eq!(data.name, "a b.csv");
        assert_eq!(data.mime.as_deref(), Some("text/csv"));
        assert_eq!(data.bytes, b"a,b\n");
    }

    #[test]
    fn data_uri_without_type_or_name() {
        let data = parse_file_uri("data:,hello%20world", 100).unwrap();
        assert_eq!(data.name, DEFAULT_NAME);
        assert_eq!(data.mime, None);
        assert_eq!(data.bytes, b"hello world");
    }

    #[test]
    fn data_uri_ignores_other_parameters_and_unpadded_base64() {
        let data =
            parse_file_uri("DATA:text/plain;charset=utf-8;name=x.txt;BASE64,aGk", 100).unwrap();
        assert_eq!(data.name, "x.txt");
        assert_eq!(data.mime.as_deref(), Some("text/plain"));
        assert_eq!(data.bytes, b"hi");
    }

    #[test]
    fn data_uri_name_keeps_only_the_file_name() {
        let data = parse_file_uri("data:;name=..%2Fsecret%2Fa.csv,x", 100).unwrap();
        assert_eq!(data.name, "a.csv");
    }

    #[test]
    fn data_uri_base64_is_a_parameter_only_at_the_end() {
        // `base64` as the type position is not the encoding flag.
        let data = parse_file_uri("data:base64,aGk", 100).unwrap();
        assert_eq!(data.bytes, b"aGk");
    }

    #[test]
    fn oversize_data_is_refused_before_decoding() {
        let big = format!("data:;base64,{}", "A".repeat(4000));
        let err = parse_file_uri(&big, 100).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        let err = parse_file_uri("data:,12345", 4).unwrap_err();
        assert!(err.contains("5 bytes, the limit is 4 bytes"), "{err}");
    }
}
