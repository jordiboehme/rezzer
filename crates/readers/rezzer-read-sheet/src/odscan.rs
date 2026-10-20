// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The checks on an ODS `content.xml` that run before calamine reads it.
//!
//! calamine 0.37.0 parses the whole of `content.xml` when it opens the file and keeps every
//! sheet as two dense ranges (values and formula texts), with repeated cells copied out. It
//! matches element and attribute names literally (`table:table-cell`), while sheetparts
//! matches local names. So, before calamine runs:
//! - [`canonical`] accepts only markup both read the same way: UTF-8, well formed, every
//!   namespace declared on the root, the ODF prefixes bound to their namespaces and to no
//!   other prefix, the names both parsers react to written with their usual prefix, and
//!   sheets only as children of `office:spreadsheet`;
//! - [`calamine_bytes`] follows calamine's reading of the cells and returns an upper bound
//!   of the memory its ranges take.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};

const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";

/// The prefix every element with this local name must have.
fn element_prefix(local: &str) -> Option<&'static str> {
    Some(match local {
        "table"
        | "table-row"
        | "table-cell"
        | "covered-table-cell"
        | "table-column"
        | "table-rows"
        | "table-header-rows"
        | "table-row-group"
        | "table-columns"
        | "table-header-columns"
        | "table-column-group"
        | "named-expressions"
        | "named-range"
        | "named-expression"
        | "content-validations"
        | "content-validation"
        | "help-message"
        | "error-message" => "table",
        "annotation" | "spreadsheet" | "body" | "document-content" => "office",
        "p" | "s" | "tab" | "line-break" => "text",
        "style" | "table-properties" => "style",
        _ => return None,
    })
}

/// The prefixes an attribute with this local name may have on an element both parsers read.
fn attribute_prefixes(local: &str) -> Option<&'static [&'static str]> {
    Some(match local {
        "value" | "string-value" | "date-value" | "time-value" | "boolean-value" => &["office"],
        // LibreOffice writes `calcext:value-type` next to `office:value-type`.
        "value-type" => &["office", "calcext"],
        "formula"
        | "number-rows-repeated"
        | "number-columns-repeated"
        | "visibility"
        | "content-validation-name"
        | "cell-range-address"
        | "expression" => &["table"],
        "c" => &["text"],
        _ => return None,
    })
}

/// Why `content.xml` is refused: a reason for the loss.
pub(crate) type Refused = String;

/// Checks that `content.xml` is canonical, so calamine and sheetparts read the same sheets
/// and cells.
pub(crate) fn canonical(bytes: &[u8], stop: &dyn Fn() -> bool) -> Result<(), Refused> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("content.xml is not UTF-8: {e}"))?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<String> = Vec::new();
    let mut root_seen = false;
    let mut events = 0u64;
    loop {
        events += 1;
        if events.is_multiple_of(4_096) && stop() {
            return Err("content.xml: the time limit was reached while it was checked".into());
        }
        let event = reader
            .read_event()
            .map_err(|e| format!("content.xml: {e}"))?;
        let (e, empty) = match event {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(_) => {
                stack.pop();
                continue;
            }
            Event::Decl(d) => {
                if let Some(enc) = d.encoding() {
                    let enc = enc.map_err(|e| format!("content.xml: {e}"))?;
                    if !enc.eq_ignore_ascii_case("utf-8") {
                        return Err("content.xml declares an encoding other than UTF-8".into());
                    }
                }
                continue;
            }
            Event::DocType(_) => return Err("content.xml has a DOCTYPE".into()),
            Event::Eof => break,
            _ => continue,
        };
        let name = e.name().as_ref().to_string();
        check_name(&name)?;
        let (prefix, local) = split(&name);
        if let Some(want) = element_prefix(local)
            && prefix != Some(want)
        {
            return Err(format!(
                "content.xml: the element {name} is not in its usual namespace prefix {want}:"
            ));
        }
        // Attributes count on the elements both parsers read.
        let read_by_both = element_prefix(local).is_some();
        let mut seen: HashMap<String, String> = HashMap::new();
        let mut namespaces: Vec<(String, String)> = Vec::new();
        for a in e.attributes() {
            let a = a.map_err(|e| format!("content.xml: {e}"))?;
            let key = a.key.as_ref().to_string();
            check_name(&key)?;
            let value = unescape(&a.value)
                .map_err(|e| format!("content.xml: {e}"))?
                .into_owned();
            if key == "xmlns" || key.starts_with("xmlns:") {
                namespaces.push((key, value));
                continue;
            }
            if !read_by_both {
                continue;
            }
            let (ap, al) = split(&key);
            if let Some(allowed) = attribute_prefixes(al)
                && !ap.is_some_and(|p| allowed.contains(&p))
            {
                return Err(format!(
                    "content.xml: the attribute {key} on {name} is not in its usual namespace"
                ));
            }
            // Two attributes with one local name read differently by name and by prefix;
            // only the value type LibreOffice writes twice is allowed.
            if let Some(other) = seen.insert(al.to_string(), key.clone())
                && al != "value-type"
            {
                return Err(format!(
                    "content.xml: {name} has the attributes {other} and {key}"
                ));
            }
        }
        if root_seen {
            if !namespaces.is_empty() {
                return Err(format!(
                    "content.xml: a namespace is declared on {name}, below the root"
                ));
            }
        } else {
            root_seen = true;
            if name != "office:document-content" {
                return Err(format!("content.xml: the root is {name}"));
            }
            root_namespaces(&namespaces)?;
        }
        if local == "table" {
            let parent = stack.last().map(String::as_str);
            if stack.len() != 3 || parent != Some("office:spreadsheet") {
                return Err(format!(
                    "content.xml: a table outside office:spreadsheet (inside {})",
                    parent.unwrap_or("nothing")
                ));
            }
        }
        if !empty {
            stack.push(name);
        }
    }
    if !stack.is_empty() {
        return Err("content.xml ends inside an element".into());
    }
    if !root_seen {
        return Err("content.xml has no root element".into());
    }
    Ok(())
}

fn split(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, name),
    }
}

fn check_name(name: &str) -> Result<(), Refused> {
    if name.bytes().filter(|b| *b == b':').count() > 1 {
        return Err(format!("content.xml: the name {name} has two colons"));
    }
    Ok(())
}

/// The root binds `office`, `table` and `text` (and `style`, when declared) to the ODF
/// namespaces; no other prefix and no default namespace is bound to them.
fn root_namespaces(namespaces: &[(String, String)]) -> Result<(), Refused> {
    let fixed = [
        ("office", OFFICE),
        ("table", TABLE),
        ("text", TEXT),
        ("style", STYLE),
    ];
    for (key, uri) in namespaces {
        let prefix = key.strip_prefix("xmlns:").unwrap_or("");
        for (p, u) in fixed {
            if (prefix == p) != (uri == u) {
                return Err(format!(
                    "content.xml: the namespace declaration {key}=\"{uri}\" binds an ODF prefix or namespace elsewhere"
                ));
            }
        }
    }
    for (p, u) in &fixed[..3] {
        let key = format!("xmlns:{p}");
        if !namespaces.iter().any(|(k, v)| *k == key && v == u) {
            return Err(format!(
                "content.xml: the root does not declare the namespace prefix {p}"
            ));
        }
    }
    Ok(())
}

/// Attributes as calamine reads them: from the raw attribute text, key up to `=`, value
/// between quotes, not unescaped. None when calamine would stop with an error.
fn raw_attrs(e: &BytesStart<'_>) -> Option<Vec<(String, String)>> {
    let raw = e.attributes_raw();
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut out = Vec::new();
    loop {
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= len {
            return Some(out);
        }
        let key_start = pos;
        while pos < len && bytes[pos] != b'=' {
            pos += 1;
        }
        if pos >= len {
            return None;
        }
        let key = raw[key_start..pos].trim_ascii_end();
        pos += 1;
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let quote = *bytes.get(pos)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        pos += 1;
        let start = pos;
        while pos < len && bytes[pos] != quote {
            pos += 1;
        }
        let value = &raw[start..pos];
        if pos < len {
            pos += 1;
        }
        out.push((key.to_string(), value.to_string()));
    }
}

fn raw_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The calamine limits of a sheet.
const MAX_ROWS: u64 = 1_048_576;
const MAX_COLUMNS: u64 = 16_384;

/// One row element as calamine reads it.
struct Row {
    /// Cells calamine pushes for it (repeats expanded up to the last cell with content).
    pushed: u64,
    /// Bytes of value and formula text in those cells.
    text: u64,
    /// First and last pushed cell with content.
    first: Option<u64>,
    last: u64,
    repeats: u64,
}

/// Bytes calamine keeps per row element: its start in the cells and its repeat count, in
/// two vectors that may hold twice what they use.
const ROW_BYTES: u64 = 2 * 2 * 8;

/// What calamine holds for one table, added up row by row.
#[derive(Default)]
struct Table {
    total_rows: u64,
    records: u64,
    pushed: u64,
    pushed_text: u64,
    /// Rows (repeats counted) from the first row with content to the last one.
    span: u64,
    /// Rows after the last row with content so far; they count once a row with content
    /// follows.
    pending: u64,
    span_text: u64,
    content: bool,
    col_min: u64,
    col_max: u64,
}

impl Table {
    fn add(&mut self, r: &Row) {
        self.records += 1;
        self.pushed = self.pushed.saturating_add(r.pushed);
        self.pushed_text = self.pushed_text.saturating_add(r.text);
        match r.first {
            Some(first) => {
                if self.content {
                    self.col_min = self.col_min.min(first);
                    self.col_max = self.col_max.max(r.last);
                } else {
                    (self.col_min, self.col_max) = (first, r.last);
                }
                self.span = self
                    .span
                    .saturating_add(self.pending)
                    .saturating_add(r.repeats);
                self.pending = 0;
                self.span_text = self
                    .span_text
                    .saturating_add(r.repeats.saturating_mul(r.text));
                self.content = true;
            }
            None if self.content => self.pending = self.pending.saturating_add(r.repeats),
            None => {}
        }
    }

    /// Bytes for what is pushed while reading and for the dense range built after. A vector
    /// that grows by pushing may hold up to twice its cells.
    fn bytes(&self, cell: u64) -> (u64, u64) {
        let read = self
            .pushed
            .saturating_mul(2 * cell)
            .saturating_add(self.pushed_text)
            .saturating_add(self.records.saturating_mul(ROW_BYTES));
        if !self.content {
            return (read, 0);
        }
        let width = self.col_max + 1 - self.col_min.min(self.col_max);
        let range = self
            .span
            .saturating_mul(width)
            .saturating_mul(2 * cell)
            .saturating_add(self.span_text);
        (read, range)
    }
}

/// An upper bound of the bytes calamine's ODS reader holds for the cells of `content.xml`
/// (`cell` bytes per value and formula slot, plus the text), including one more copy of the
/// largest sheet range for reading it. `content.xml` must have passed [`canonical`].
pub(crate) fn calamine_bytes(
    bytes: &[u8],
    cell: u64,
    stop: &dyn Fn() -> bool,
) -> Result<u64, Refused> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("content.xml is not UTF-8: {e}"))?;
    let mut scan = Scan {
        reader: Reader::from_str(text),
        stop,
        events: 0,
    };
    let config = scan.reader.config_mut();
    config.check_end_names = false;
    config.trim_text(false);
    config.check_comments = false;
    config.expand_empty_elements = true;
    let mut total = 0u64;
    let mut largest = 0u64;
    while let Some(event) = scan.next()? {
        if let Event::Start(e) = event
            && e.name().as_ref() == "table:table"
            && raw_attrs(&e).is_some_and(|a| raw_attr(&a, "table:name").is_some())
        {
            let mut table = Table::default();
            let ended = scan.table(&mut table)?;
            let (read, range) = table.bytes(cell);
            total = total.saturating_add(read).saturating_add(range);
            largest = largest.max(range);
            if !ended {
                // calamine stops with an error here: nothing more is built.
                break;
            }
        }
    }
    Ok(total.saturating_add(largest))
}

struct Scan<'a> {
    reader: Reader<&'a [u8]>,
    stop: &'a dyn Fn() -> bool,
    events: u64,
}

impl<'a> Scan<'a> {
    /// The next event; None at the end or where calamine stops with an XML error.
    fn next(&mut self) -> Result<Option<Event<'a>>, Refused> {
        self.events += 1;
        if self.events.is_multiple_of(4_096) && (self.stop)() {
            return Err("content.xml: the time limit was reached while it was measured".into());
        }
        match self.reader.read_event() {
            Ok(Event::Eof) | Err(_) => Ok(None),
            Ok(e) => Ok(Some(e)),
        }
    }

    /// Reads one table like calamine's `read_table`. False when calamine would stop with an
    /// error inside it.
    fn table(&mut self, table: &mut Table) -> Result<bool, Refused> {
        loop {
            let Some(event) = self.next()? else {
                return Ok(false);
            };
            match event {
                Event::Start(e) if e.name().as_ref() == "table:table-row" => {
                    let Some(attrs) = raw_attrs(&e) else {
                        return Ok(false);
                    };
                    let repeats = match raw_attr(&attrs, "table:number-rows-repeated") {
                        None => 1,
                        Some(v) => match unescape(v).ok().and_then(|v| v.parse::<u64>().ok()) {
                            Some(n) => n,
                            None => return Ok(false),
                        },
                    };
                    let left = MAX_ROWS.saturating_sub(table.total_rows);
                    let repeats = repeats.min(left);
                    table.total_rows = table.total_rows.saturating_add(repeats);
                    let mut row = Row {
                        pushed: 0,
                        text: 0,
                        first: None,
                        last: 0,
                        repeats,
                    };
                    let ok = self.row(&mut row)?;
                    table.add(&row);
                    if !ok {
                        return Ok(false);
                    }
                }
                Event::End(e) if e.name().as_ref() == "table:table" => return Ok(true),
                _ => {}
            }
        }
    }

    /// Reads one row like calamine's `read_row`. False when calamine would stop with an
    /// error.
    fn row(&mut self, row: &mut Row) -> Result<bool, Refused> {
        let mut empty_run = 0u64;
        loop {
            let Some(event) = self.next()? else {
                return Ok(false);
            };
            match event {
                Event::Start(e)
                    if matches!(
                        e.name().as_ref(),
                        "table:table-cell" | "table:covered-table-cell"
                    ) =>
                {
                    let Some(attrs) = raw_attrs(&e) else {
                        return Ok(false);
                    };
                    let repeats = match raw_attr(&attrs, "table:number-columns-repeated") {
                        None => 1,
                        Some(v) => match v.parse::<u64>() {
                            Ok(n) => n,
                            Err(_) => return Ok(false),
                        },
                    };
                    let Some((content, text, closed)) = self.cell(&attrs)? else {
                        return Ok(false);
                    };
                    let room = MAX_COLUMNS.saturating_sub(row.pushed);
                    row.pushed += empty_run.min(room);
                    empty_run = 0;
                    let room = MAX_COLUMNS.saturating_sub(row.pushed);
                    let repeats = repeats.min(room);
                    if content {
                        if repeats > 0 {
                            row.first.get_or_insert(row.pushed);
                            row.last = row.pushed + repeats - 1;
                        }
                        row.pushed += repeats;
                        row.text = row.text.saturating_add(text.saturating_mul(repeats));
                    } else {
                        empty_run = repeats;
                    }
                    if !closed && !self.skip(e.name().as_ref().to_string())? {
                        return Ok(false);
                    }
                }
                Event::End(e) if e.name().as_ref() == "table:table-row" => return Ok(true),
                _ => return Ok(false),
            }
        }
    }

    /// Reads a cell like calamine's `get_datatype`: whether it has content, the bytes of its
    /// value and formula text at most, and whether its end tag was read. None when calamine
    /// would stop with an error.
    fn cell(&mut self, attrs: &[(String, String)]) -> Result<Option<(bool, u64, bool)>, Refused> {
        let mut is_string = false;
        let mut value_set = false;
        let mut text = 0u64;
        let mut formula = 0u64;
        for (key, value) in attrs {
            match key.as_str() {
                "office:value" | "office:boolean-value" if !value_set => value_set = true,
                "office:string-value" | "office:date-value" | "office:time-value" if !value_set => {
                    value_set = true;
                    text = value.len() as u64;
                }
                "office:value-type" if !value_set => is_string = value == "string",
                "table:formula" => formula = value.len() as u64,
                _ => {}
            }
        }
        if value_set || !is_string {
            return Ok(Some((value_set || formula > 0, text + formula, false)));
        }
        // A string cell without a string value: its text is the paragraphs inside.
        let mut first = true;
        loop {
            let Some(event) = self.next()? else {
                return Ok(None);
            };
            match event {
                Event::Text(t) => text = text.saturating_add(t.as_ref().len() as u64),
                Event::GeneralRef(_) => text = text.saturating_add(4),
                Event::End(e)
                    if matches!(
                        e.name().as_ref(),
                        "table:table-cell" | "table:covered-table-cell"
                    ) =>
                {
                    return Ok(Some((true, text + formula, true)));
                }
                Event::Start(e) if e.name().as_ref() == "office:annotation" => loop {
                    match self.next()? {
                        Some(Event::End(e)) if e.name().as_ref() == "office:annotation" => break,
                        Some(_) => {}
                        None => return Ok(None),
                    }
                },
                Event::Start(e) if e.name().as_ref() == "text:p" => {
                    if !first {
                        text = text.saturating_add(1);
                    }
                    first = false;
                }
                Event::Start(e) if e.name().as_ref() == "text:s" => {
                    let Some(attrs) = raw_attrs(&e) else {
                        return Ok(None);
                    };
                    let count = match raw_attr(&attrs, "text:c") {
                        None => 1,
                        Some(v) => match unescape(v).ok().and_then(|v| v.parse::<u64>().ok()) {
                            Some(n) => n,
                            None => return Ok(None),
                        },
                    };
                    text = text.saturating_add(count);
                }
                _ => {}
            }
        }
    }

    /// Skips to the end tag of `name`, counting nested elements of the same name. False at
    /// the end of the text.
    fn skip(&mut self, name: String) -> Result<bool, Refused> {
        let mut depth = 0u64;
        loop {
            match self.next()? {
                Some(Event::Start(e)) if e.name().as_ref() == name.as_str() => depth += 1,
                Some(Event::End(e)) if e.name().as_ref() == name.as_str() => {
                    if depth == 0 {
                        return Ok(true);
                    }
                    depth -= 1;
                }
                Some(_) => {}
                None => return Ok(false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(body: &str) -> String {
        format!(
            "<office:document-content xmlns:office=\"{OFFICE}\" xmlns:table=\"{TABLE}\" xmlns:text=\"{TEXT}\"><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"
        )
    }

    fn bytes(body: &str) -> u64 {
        calamine_bytes(doc(body).as_bytes(), 1, &|| false).unwrap()
    }

    #[test]
    fn repeated_rows_and_columns_count_as_calamine_expands_them() {
        // One row of three value cells, repeated 10 times: 30 cells in the range, 3 pushed,
        // plus one more copy of the range.
        let body = "<table:table table:name=\"S\"><table:table-row table:number-rows-repeated=\"10\"><table:table-cell table:number-columns-repeated=\"3\" office:value=\"1\"/></table:table-row></table:table>";
        // Pushed 3 (doubled) and one row element; the range 30 (doubled), and its copy.
        assert_eq!(bytes(body), (2 * 3 + 32) + 60 + 60);
        // Trailing empty cells and rows cost nothing.
        let body = "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value=\"1\"/><table:table-cell table:number-columns-repeated=\"16000\"/></table:table-row><table:table-row table:number-rows-repeated=\"1000000\"><table:table-cell table:number-columns-repeated=\"16384\"/></table:table-row></table:table>";
        assert_eq!(bytes(body), (2 + 64) + 2 + 2);
        // Empty cells before a value are pushed.
        let body = "<table:table table:name=\"S\"><table:table-row><table:table-cell table:number-columns-repeated=\"9\"/><table:table-cell office:value=\"1\"/></table:table-row></table:table>";
        assert_eq!(bytes(body), (20 + 32) + 2 + 2);
    }

    #[test]
    fn texts_count_per_copy() {
        let body = "<table:table table:name=\"S\"><table:table-row table:number-rows-repeated=\"2\"><table:table-cell table:number-columns-repeated=\"5\" office:value-type=\"string\"><text:p>abc</text:p><text:p>d<text:s text:c=\"7\"/></text:p></table:table-cell></table:table-row></table:table>";
        // Text per cell: 3 + newline + 1 + 7 = 12; pushed 5 cells, range 10 cells.
        assert_eq!(bytes(body), (10 + 5 * 12 + 32) + (20 + 10 * 12) * 2);
    }

    #[test]
    fn entity_repeat_counts_are_read_as_calamine_reads_them() {
        let body = "<table:table table:name=\"S\"><table:table-row table:number-rows-repeated=\"&#49;0\"><table:table-cell office:value=\"1\"/></table:table-row></table:table>";
        assert_eq!(bytes(body), (2 + 32) + 20 + 20);
    }

    #[test]
    fn canonical_markup() {
        assert!(canonical(doc("<table:table table:name=\"S\"/>").as_bytes(), &|| false).is_ok());
        for bad in [
            doc("<table:table table:name=\"S\" xmlns:x=\"urn:x\"/>"),
            doc(
                "<table:table table:name=\"S\"><table:table-row><x:table-cell xmlns:x=\"urn:x\"/></table:table-row></table:table>",
            ),
            doc(
                "<table:table table:name=\"S\"><table:table-row><table:table-cell x:formula=\"1\"/></table:table-row></table:table>",
            ),
            doc(
                "<table:table table:name=\"S\"><table:table-row><table:table-cell office:value=\"1\" table:value=\"2\"/></table:table-row></table:table>",
            ),
            doc("<table:table table:name=\"S\"><table:table-row>"),
            doc(
                "<table:table table:name=\"S\"><table:table-row><table:table-cell><table:table table:name=\"T\"/></table:table-cell></table:table-row></table:table>",
            ),
            doc("<a:b:c/>"),
        ] {
            assert!(canonical(bad.as_bytes(), &|| false).is_err(), "{bad}");
        }
    }
}
