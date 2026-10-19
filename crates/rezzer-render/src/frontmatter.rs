// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OKF v0.2 frontmatter, compatible with Crystalline's engram frontmatter: OKF keys first,
//! then Rezzer's own snake_case keys (document properties last). Every value that comes from a file goes through
//! [`quote`].

use rezzer_model::{Completeness, Document, FormatId, Value};

use crate::RenderOptions;

/// A YAML double-quoted string. Escapes `\`, `"`, newline, tab, carriage return and every
/// other character YAML or a reader could take as a line end or control: C0, DEL, C1,
/// U+2028, U+2029, U+FEFF, U+FFFE and U+FFFF as `\uXXXX`.
pub(crate) fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control()
                || matches!(
                    c,
                    '\u{2028}' | '\u{2029}' | '\u{feff}' | '\u{fffe}' | '\u{ffff}'
                ) =>
            {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A value written plain when it is a simple token (a version, a time stamp), else quoted.
fn token(value: &str) -> String {
    let simple = !value.is_empty()
        && !value.ends_with(':')
        && value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_' | '/' | ':'));
    if simple {
        value.to_string()
    } else {
        quote(value)
    }
}

/// A typed value: plain numbers without a unit and booleans, everything else quoted.
fn yaml_value(value: &Value, depth: usize) -> String {
    match value {
        Value::Number { value, unit: None } if value.is_finite() => crate::facets::number(*value),
        Value::Bool { value } => value.to_string(),
        Value::List { value } if depth < crate::blocks::MAX_DEPTH => {
            let items: Vec<String> = value.iter().map(|v| yaml_value(v, depth + 1)).collect();
            format!("[{}]", items.join(", "))
        }
        other => quote(&crate::facets::value_text(other)),
    }
}

fn quoted_list(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|v| quote(v)).collect();
    format!("[{}]", items.join(", "))
}

/// The `YYYY-MM-DD` at the start of `value`, when it is a real calendar date shape.
fn date_prefix(value: &str) -> Option<&str> {
    let date = value.get(..10)?;
    let b = date.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)) {
        return None;
    }
    let month: u32 = date[5..7].parse().ok()?;
    let day: u32 = date[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Only a date, or a date followed by a time part.
    match value.as_bytes().get(10) {
        None | Some(b'T' | b't' | b' ') => Some(date),
        _ => None,
    }
}

fn value_date(value: &Value) -> Option<&str> {
    match value {
        Value::Date { value } | Value::DateTime { value } | Value::Text { value } => {
            date_prefix(value)
        }
        _ => None,
    }
}

pub(crate) fn completeness_name(c: Completeness) -> &'static str {
    match c {
        Completeness::Complete => "complete",
        Completeness::Partial => "partial",
        Completeness::Failed => "failed",
    }
}

/// The snake_case name of a format, as in the model JSON.
pub(crate) fn format_name(format: FormatId) -> String {
    let debug = format!("{format:?}");
    let mut out = String::new();
    for (i, c) in debug.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The title used in the frontmatter and the `#` heading: the document title, else the file
/// name.
pub(crate) fn title(doc: &Document) -> &str {
    doc.properties
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(&doc.source.name)
}

/// The frontmatter block with both `---` lines and a final newline.
pub(crate) fn render(doc: &Document, completeness: Completeness, opts: &RenderOptions) -> String {
    let p = &doc.properties;
    let s = &doc.source;
    let mut out = String::from("---\ntype: document\n");
    out.push_str(&format!("title: {}\n", quote(title(doc))));
    if let Some(d) = p.description.as_deref().filter(|d| !d.trim().is_empty()) {
        out.push_str(&format!("description: {}\n", quote(d)));
    }
    let resource = s.path.as_deref().unwrap_or(&s.name);
    out.push_str(&format!("resource: {}\n", quote(resource)));
    if !p.keywords.is_empty() {
        out.push_str(&format!("tags: {}\n", quoted_list(&p.keywords)));
    }
    out.push_str(&format!(
        "generated: {{ by: {}, at: {} }}\n",
        token(&format!("rezzer/{}", opts.rezzer_version)),
        token(&opts.generated_at)
    ));
    let from_document = p
        .modified
        .as_ref()
        .and_then(value_date)
        .or_else(|| p.created.as_ref().and_then(value_date));
    let (date, confidence) = match from_document {
        Some(d) => (Some(d), "high"),
        None => (s.modified.as_deref().and_then(date_prefix), "medium"),
    };
    if let Some(date) = date {
        out.push_str(&format!("source_date: {date}\n"));
        out.push_str(&format!("temporal_confidence: {confidence}\n"));
    }
    if let Some(archive) = &opts.archive_ref {
        out.push_str(&format!(
            "sources: {}\n",
            quoted_list(std::slice::from_ref(archive))
        ));
    }
    out.push_str(&format!("source_sha256: {}\n", quote(&s.sha256)));
    out.push_str(&format!("source_size: {}\n", s.size));
    out.push_str(&format!("format: {}\n", format_name(s.format)));
    out.push_str(&format!("detected_by: {}\n", quote(&s.detection.evidence)));
    if let Some(e) = &s.encoding {
        out.push_str(&format!("encoding: {}\n", quote(e)));
    }
    if let Some(r) = &s.reader {
        out.push_str(&format!(
            "reader: {}\n",
            quote(&format!("{}/{}", r.id, r.version))
        ));
    }
    out.push_str(&format!(
        "completeness: {}\n",
        completeness_name(completeness)
    ));
    out.push_str(&format!("losses: {}\n", doc.losses.len()));
    if !p.authors.is_empty() {
        out.push_str(&format!("authors: {}\n", quoted_list(&p.authors)));
    }
    if let Some(l) = p.language.as_deref().filter(|l| !l.trim().is_empty()) {
        out.push_str(&format!("language: {}\n", quote(l)));
    }
    if !p.keywords.is_empty() {
        out.push_str(&format!("keywords: {}\n", quoted_list(&p.keywords)));
    }
    if let Some(v) = &p.created {
        out.push_str(&format!("created: {}\n", yaml_value(v, 0)));
    }
    if let Some(v) = &p.modified {
        out.push_str(&format!("modified: {}\n", yaml_value(v, 0)));
    }
    out.push_str("---\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_controls_and_line_separators() {
        assert_eq!(
            quote("a\\b\"c\nd\te\r\u{1}\u{7f}\u{85}\u{2028}\u{feff}"),
            "\"a\\\\b\\\"c\\nd\\te\\r\\u0001\\u007F\\u0085\\u2028\\uFEFF\""
        );
    }

    #[test]
    fn date_prefix_is_strict() {
        assert_eq!(date_prefix("2025-12-01"), Some("2025-12-01"));
        assert_eq!(date_prefix("2025-12-01T10:00:00Z"), Some("2025-12-01"));
        assert_eq!(date_prefix("2025-13-01"), None);
        assert_eq!(date_prefix("2025-12-011"), None);
        assert_eq!(date_prefix("yesterday"), None);
        assert_eq!(date_prefix("2025-12-0é"), None);
    }

    #[test]
    fn token_quotes_anything_unusual() {
        assert_eq!(token("2026-10-20T12:00:00Z"), "2026-10-20T12:00:00Z");
        assert_eq!(token("rezzer/0.1.0"), "rezzer/0.1.0");
        assert_eq!(token("a, b: c"), "\"a, b: c\"");
        assert_eq!(token(""), "\"\"");
    }

    #[test]
    fn format_names_match_the_model_json() {
        assert_eq!(format_name(FormatId::ThreeMf), "three_mf");
        assert_eq!(format_name(FormatId::KicadSch), "kicad_sch");
        assert_eq!(format_name(FormatId::Markdown), "markdown");
    }
}
