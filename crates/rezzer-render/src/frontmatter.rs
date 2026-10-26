// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OKF v0.2 frontmatter, compatible with Crystalline's engram frontmatter: OKF keys first,
//! then Rezzer's own snake_case keys (document properties last). Every value that comes from a file goes through
//! [`quote`].

use rezzer_model::{Completeness, Document, FileFacts, FormatId, Value};

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

/// The `YYYY-MM-DD` at the start of `value`, when it has the shape of a date. Whether it is
/// a real calendar date is [`is_real_date`].
fn date_prefix(value: &str) -> Option<&str> {
    let date = value.get(..10)?;
    let b = date.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)) {
        return None;
    }
    // Only a date, or a date followed by a time part.
    match value.as_bytes().get(10) {
        None | Some(b'T' | b't' | b' ') => Some(date),
        _ => None,
    }
}

/// True for a date that exists in the Gregorian calendar (`2024-02-29` yes, `2026-02-30`
/// no).
fn is_real_date(date: &str) -> bool {
    let (Ok(year), Ok(month), Ok(day)) = (
        date[0..4].parse::<u32>(),
        date[5..7].parse::<u32>(),
        date[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// A real calendar date from a document property; anything else is no date.
fn value_date(value: &Value) -> Option<&str> {
    match value {
        Value::Date { value } | Value::DateTime { value } | Value::Text { value } => {
            date_prefix(value).filter(|d| is_real_date(d))
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

/// The document's title: its own, else the file name.
pub(crate) fn title(doc: &Document) -> &str {
    doc.properties
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(&doc.source.name)
}

/// The title of this output, in the frontmatter and the `#` heading: a part's own title,
/// else the document's.
pub(crate) fn shown_title<'a>(doc: &'a Document, opts: &'a RenderOptions) -> &'a str {
    match &opts.part {
        Some(part) if !part.title.trim().is_empty() => &part.title,
        _ => title(doc),
    }
}

/// The description of this output: for a part `Part <k> of <n> of "<title>": <location>.`
/// with the title cut as in the part's title, else the document's own or the template
/// ([`crate::describe::description`]).
pub(crate) fn shown_description(doc: &Document, opts: &RenderOptions) -> String {
    match &opts.part {
        Some(part) => format!(
            "Part {} of {} of \"{}\": {}.",
            part.index,
            part.count,
            crate::split::capped(title(doc), crate::split::TITLE_CAP),
            part.location
        ),
        None => crate::describe::description(doc),
    }
}

/// The document's own date (`YYYY-MM-DD`): its modified date, else its created date, when
/// that is a real calendar date.
pub(crate) fn document_date(doc: &Document) -> Option<&str> {
    let p = &doc.properties;
    p.modified
        .as_ref()
        .and_then(value_date)
        .or_else(|| p.created.as_ref().and_then(value_date))
}

/// The date of the source with how sure it is: the document's own date is `explicit`, a
/// file system date (modified, else created) `inferred`. Only a real calendar date counts
/// (Crystalline's T003); without one there is no date.
fn source_date(doc: &Document) -> Option<(&str, &'static str)> {
    if let Some(date) = document_date(doc) {
        return Some((date, "explicit"));
    }
    let s = &doc.source;
    [&s.modified, &s.created]
        .into_iter()
        .flatten()
        .find_map(|v| date_prefix(v).filter(|d| is_real_date(d)))
        .map(|date| (date, "inferred"))
}

/// The frontmatter block with both `---` lines and a final newline. Key order: the OKF keys
/// (`type`, `title`, `description`, `resource`, `tags`, `generated`, `source_date`,
/// `temporal_confidence`, `sources`, `status`), then Rezzer's: the source and its file
/// system facts, the read, the part keys, the file facts, the render options, the document
/// properties.
pub(crate) fn render(doc: &Document, completeness: Completeness, opts: &RenderOptions) -> String {
    let p = &doc.properties;
    let s = &doc.source;
    let mut out = String::from("---\ntype: source\n");
    out.push_str(&format!("title: {}\n", quote(shown_title(doc, opts))));
    out.push_str(&format!(
        "description: {}\n",
        quote(&shown_description(doc, opts))
    ));
    let resource = s.path.as_deref().unwrap_or(&s.name);
    out.push_str(&format!("resource: {}\n", quote(resource)));
    out.push_str(&format!(
        "tags: {}\n",
        quoted_list(&crate::describe::tags(doc))
    ));
    out.push_str(&format!(
        "generated: {{ by: {}, at: {} }}\n",
        token(&format!("rezzer/{}", opts.rezzer_version)),
        token(&opts.generated_at)
    ));
    if let Some((date, confidence)) = source_date(doc) {
        out.push_str(&format!("source_date: {date}\n"));
        out.push_str(&format!("temporal_confidence: {confidence}\n"));
    }
    if let Some(archive) = &opts.archive_ref {
        out.push_str(&format!(
            "sources: {}\n",
            quoted_list(std::slice::from_ref(archive))
        ));
    }
    out.push_str("status: stable\n");
    out.push_str(&format!("source_sha256: {}\n", quote(&s.sha256)));
    out.push_str(&format!("source_size: {}\n", s.size));
    if let Some(t) = &s.created {
        out.push_str(&format!("file_created: {}\n", token(t)));
    }
    if let Some(t) = &s.modified {
        out.push_str(&format!("file_modified: {}\n", token(t)));
    }
    if let Some(url) = &s.downloaded_from {
        out.push_str(&format!("downloaded_from: {}\n", quote(url)));
    }
    if !s.file_tags.is_empty() {
        out.push_str(&format!("file_tags: {}\n", quoted_list(&s.file_tags)));
    }
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
    match &opts.part {
        Some(part) => {
            out.push_str(&format!("parts: {}\n", part.count));
            out.push_str(&format!("part: {}\n", part.index));
            out.push_str(&format!("location: {}\n", quote(&part.location)));
        }
        None => {
            if let Some(n) = opts.parts {
                out.push_str(&format!("parts: {n}\n"));
            }
        }
    }
    facts(&p.facts, &mut out);
    // The render options that change what the output holds; left out at their defaults.
    if opts.table_style != crate::TableStyle::Table {
        out.push_str(&format!(
            "table_style: {}\n",
            quote(opts.table_style.name())
        ));
    }
    if let Some(sheets) = crate::chosen_sheets(opts) {
        out.push_str(&format!("sheets_selected: {}\n", quoted_list(sheets)));
    }
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

/// The facts about the file that are set, in the order of [`FileFacts`], as plain YAML
/// numbers and booleans.
fn facts(f: &FileFacts, out: &mut String) {
    let numbers = |key: &str, v: Option<u32>, out: &mut String| {
        if let Some(v) = v {
            out.push_str(&format!("{key}: {v}\n"));
        }
    };
    let flags = |key: &str, v: Option<bool>, out: &mut String| {
        if let Some(v) = v {
            out.push_str(&format!("{key}: {v}\n"));
        }
    };
    // Destructured, so a new fact cannot be forgotten here.
    let FileFacts {
        pages,
        tagged,
        encrypted,
        signed,
        signatures,
        revisions,
        form_fields,
        has_javascript,
        layers,
    } = f;
    numbers("pages", *pages, out);
    flags("tagged", *tagged, out);
    flags("encrypted", *encrypted, out);
    flags("signed", *signed, out);
    numbers("signatures", *signatures, out);
    numbers("revisions", *revisions, out);
    numbers("form_fields", *form_fields, out);
    flags("has_javascript", *has_javascript, out);
    numbers("layers", *layers, out);
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
        assert!(!is_real_date(date_prefix("2025-13-01").unwrap()));
        assert!(!is_real_date("2025-00-10"));
        assert!(!is_real_date("2025-01-00"));
        assert!(is_real_date("2000-02-29"));
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
