// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Styles of an ODF spreadsheet: which sheets are hidden and which number format a cell has.
//!
//! A cell names a cell style, the cell style names a data style (or a parent that does), and the
//! data style is a list of parts (digits, text, date fields). This module turns the data style
//! into one format code in the notation of [`crate::numfmt`].

use std::collections::{HashMap, HashSet};

use crate::xml::{Elem, Ev};
use crate::{Issue, IssueKind};

#[derive(Debug, Clone)]
struct Part {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
}

impl Part {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn number(&self, key: &str) -> Option<usize> {
        self.attr(key).and_then(|v| v.trim().parse().ok())
    }

    fn long(&self) -> bool {
        self.attr("style") == Some("long")
    }
}

#[derive(Debug, Clone)]
struct DataStyle {
    kind: String,
    truncate: bool,
    parts: Vec<Part>,
    /// `(condition, style to apply)` pairs.
    maps: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
struct CellStyle {
    data_style: Option<String>,
    parent: Option<String>,
}

const DATA_STYLES: [&str; 7] = [
    "number-style",
    "percentage-style",
    "currency-style",
    "date-style",
    "time-style",
    "boolean-style",
    "text-style",
];

/// Everything the cells need to know about styles, collected from `styles.xml` and the
/// automatic styles of `content.xml`.
#[derive(Default)]
pub(super) struct StyleBook {
    data: HashMap<String, DataStyle>,
    cells: HashMap<String, CellStyle>,
    table_hidden: HashMap<String, bool>,
    cell_cache: HashMap<String, Option<String>>,
    code_cache: HashMap<String, Option<String>>,
    // Parser state.
    table_style: Option<String>,
    data_now: Option<(String, DataStyle)>,
    part_now: Option<Part>,
    part_nested: usize,
}

impl StyleBook {
    /// Whether the table style says the sheet is not displayed.
    pub fn sheet_hidden(&self, style: Option<&str>) -> bool {
        style
            .and_then(|s| self.table_hidden.get(s))
            .copied()
            .unwrap_or(false)
    }

    /// Feeds one XML event. Elements that are no style are ignored.
    pub fn event(&mut self, ev: &Ev<'_>) {
        match ev {
            Ev::Start(e) => self.open(e, false),
            Ev::Empty(e) => self.open(e, true),
            Ev::End(name) => self.close(name),
            Ev::Text(t) => {
                if let Some(p) = &mut self.part_now {
                    p.text.push_str(t);
                }
            }
        }
    }

    fn open(&mut self, e: &Elem, empty: bool) {
        if self.data_now.is_some() {
            self.open_in_data_style(e, empty);
            return;
        }
        if DATA_STYLES.contains(&e.name.as_str()) {
            if let Some(name) = e.attr("name") {
                let style = DataStyle {
                    kind: e.name.clone(),
                    truncate: e.attr("truncate-on-overflow") != Some("false"),
                    parts: Vec::new(),
                    maps: Vec::new(),
                };
                if empty {
                    self.data.insert(name.to_string(), style);
                } else {
                    self.data_now = Some((name.to_string(), style));
                }
            }
            return;
        }
        match e.name.as_str() {
            "style" => {
                let Some(name) = e.attr("name") else { return };
                match e.attr("family") {
                    Some("table-cell") => {
                        self.cells.insert(
                            name.to_string(),
                            CellStyle {
                                data_style: e.attr("data-style-name").map(str::to_string),
                                parent: e.attr("parent-style-name").map(str::to_string),
                            },
                        );
                    }
                    Some("table") if !empty => self.table_style = Some(name.to_string()),
                    _ => {}
                }
            }
            "table-properties" => {
                if let (Some(style), Some(display)) = (&self.table_style, e.attr("display")) {
                    self.table_hidden.insert(style.clone(), display == "false");
                }
            }
            _ => {}
        }
    }

    fn open_in_data_style(&mut self, e: &Elem, empty: bool) {
        if self.part_now.is_some() {
            self.part_nested += usize::from(!empty);
            return;
        }
        let Some((_, style)) = &mut self.data_now else {
            return;
        };
        if e.name == "map" {
            if let (Some(c), Some(s)) = (e.attr("condition"), e.attr("apply-style-name")) {
                style.maps.push((c.to_string(), s.to_string()));
            }
            return;
        }
        let part = Part {
            tag: e.name.clone(),
            attrs: e.attrs.clone(),
            text: String::new(),
        };
        if empty {
            style.parts.push(part);
        } else {
            self.part_now = Some(part);
            self.part_nested = 0;
        }
    }

    fn close(&mut self, name: &str) {
        if self.part_now.is_some() {
            if self.part_nested > 0 {
                self.part_nested -= 1;
            } else if let (Some(p), Some((_, style))) = (self.part_now.take(), &mut self.data_now) {
                style.parts.push(p);
            }
            return;
        }
        if DATA_STYLES.contains(&name) {
            if let Some((n, style)) = self.data_now.take() {
                self.data.insert(n, style);
            }
        } else if name == "style" {
            self.table_style = None;
        }
    }

    /// The format code of a cell style, or `None` when it has no data style or the data style
    /// cannot be written as a format code (then an issue says why, once per style).
    pub fn format_of(&mut self, cell_style: &str, issues: &mut Vec<Issue>) -> Option<String> {
        if let Some(hit) = self.cell_cache.get(cell_style) {
            return hit.clone();
        }
        let mut seen = HashSet::new();
        let mut name = Some(cell_style.to_string());
        let mut data = None;
        while let Some(n) = name {
            if !seen.insert(n.clone()) {
                break;
            }
            let Some(style) = self.cells.get(&n) else {
                break;
            };
            if let Some(d) = &style.data_style {
                data = Some(d.clone());
                break;
            }
            name = style.parent.clone();
        }
        let code = data.and_then(|d| self.code_of(&d, issues));
        self.cell_cache.insert(cell_style.to_string(), code.clone());
        code
    }

    fn code_of(&mut self, name: &str, issues: &mut Vec<Issue>) -> Option<String> {
        if let Some(hit) = self.code_cache.get(name) {
            return hit.clone();
        }
        let code = match self.build(name) {
            Ok(c) => Some(c),
            Err(why) => {
                issues.push(Issue::new(
                    "content.xml",
                    IssueKind::Unsupported,
                    format!("number format {name} is not written as a format code: {why}"),
                ));
                None
            }
        };
        self.code_cache.insert(name.to_string(), code.clone());
        code
    }

    /// A data style as one format code. The styles its maps apply are used by their own
    /// parts only (their maps are not followed), at most two maps are taken and the code is at
    /// most 255 characters, as in the OOXML reader.
    fn build(&self, name: &str) -> Result<String, String> {
        let style = self
            .data
            .get(name)
            .ok_or_else(|| format!("no data style {name}"))?;
        let base = body(style)?;
        let code = if style.maps.is_empty() {
            base
        } else if style.maps.len() > 2 {
            return Err(format!(
                "{} conditions (two are possible)",
                style.maps.len()
            ));
        } else {
            let mut sections = Vec::new();
            for (cond, apply) in &style.maps {
                let cond = condition(cond).ok_or_else(|| format!("condition {cond}"))?;
                let applied = self
                    .data
                    .get(apply)
                    .ok_or_else(|| format!("no data style {apply}"))?;
                sections.push(format!("[{cond}]{}", body(applied)?));
            }
            sections.push(base);
            sections.join(";")
        };
        if code.chars().count() > MAX_CODE {
            return Err(format!("the code is longer than {MAX_CODE} characters"));
        }
        Ok(code)
    }
}

const MAX_CODE: usize = 255;

/// `value()>=0` becomes `>=0`.
fn condition(raw: &str) -> Option<String> {
    let rest = raw.trim().strip_prefix("value()")?.trim();
    let op_len = rest.chars().take_while(|c| "<>=!".contains(*c)).count();
    let (op, number) = rest.split_at(op_len);
    let op = match op {
        "=" | "==" => "=",
        "!=" | "<>" => "<>",
        "<" | ">" | "<=" | ">=" => op,
        _ => return None,
    };
    let number = number.trim();
    (!number.is_empty() && number.parse::<f64>().is_ok()).then(|| format!("{op}{number}"))
}

fn body(style: &DataStyle) -> Result<String, String> {
    let percent = style.kind == "percentage-style";
    let mut out = String::new();
    let mut has_percent = false;
    let mut first_time = true;
    for p in &style.parts {
        match p.tag.as_str() {
            "text" if percent && p.text == "%" => {
                out.push('%');
                has_percent = true;
            }
            "text" => out.push_str(&literal(&p.text)),
            "text-content" => out.push('@'),
            "currency-symbol" => out.push_str(&quoted(&p.text)),
            "number" => out.push_str(&number(p)),
            "scientific-number" => {
                out.push_str(&number(p));
                out.push_str("E+");
                out.push_str(
                    &"0".repeat(p.number("min-exponent-digits").unwrap_or(1).clamp(1, 30)),
                );
            }
            "fraction" => {
                out.push_str("# ");
                out.push_str(
                    &"?".repeat(p.number("min-numerator-digits").unwrap_or(1).clamp(1, 30)),
                );
                out.push('/');
                match p.number("denominator-value") {
                    Some(d) if d > 0 => out.push_str(&d.min(1_000_000).to_string()),
                    _ => out.push_str(
                        &"?".repeat(p.number("min-denominator-digits").unwrap_or(1).clamp(1, 30)),
                    ),
                }
            }
            "day" => out.push_str(if p.long() { "dd" } else { "d" }),
            "month" => out.push_str(match (p.attr("textual") == Some("true"), p.long()) {
                (true, true) => "mmmm",
                (true, false) => "mmm",
                (false, true) => "mm",
                (false, false) => "m",
            }),
            "year" => out.push_str(if p.long() { "yyyy" } else { "yy" }),
            "day-of-week" => out.push_str(if p.long() { "dddd" } else { "ddd" }),
            "am-pm" => out.push_str("AM/PM"),
            "hours" | "minutes" | "seconds" => {
                let letter = match p.tag.as_str() {
                    "hours" => 'h',
                    "minutes" => 'm',
                    _ => 's',
                };
                let mut unit = letter.to_string();
                if p.long() {
                    unit.push(letter);
                }
                if first_time && !style.truncate {
                    unit = format!("[{unit}]");
                }
                first_time = false;
                out.push_str(&unit);
                if letter == 's'
                    && let Some(n) = p.number("decimal-places").filter(|n| *n > 0)
                {
                    out.push('.');
                    out.push_str(&"0".repeat(n.min(30)));
                }
            }
            other => return Err(format!("the part {other} has no format code")),
        }
    }
    if style.kind == "boolean-style" {
        return Err("a boolean format has no format code".to_string());
    }
    if percent && !has_percent {
        out.push('%');
    }
    Ok(out)
}

fn number(p: &Part) -> String {
    let min_int = p.number("min-integer-digits").unwrap_or(1);
    let mut out = String::new();
    if p.attr("grouping") == Some("true") {
        if min_int <= 3 {
            out.push_str("#,");
            out.push_str(&"#".repeat(3 - min_int));
            out.push_str(&"0".repeat(min_int));
        } else {
            let digits = "0".repeat(min_int.min(30));
            let (head, tail) = digits.split_at(digits.len() - 3);
            out.push_str(&format!("{head},{tail}"));
        }
    } else if min_int == 0 {
        out.push('#');
    } else {
        out.push_str(&"0".repeat(min_int.min(30)));
    }
    if let Some(n) = p.number("decimal-places").filter(|n| *n > 0) {
        out.push('.');
        out.push_str(&"0".repeat(n.min(30)));
    }
    match p.number("display-factor") {
        Some(1000) => out.push(','),
        Some(1_000_000) => out.push_str(",,"),
        _ => {}
    }
    out
}

/// Characters a format code writes as they are.
const PLAIN: &str = "$-+/():!^&'~{}<>= ";

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        if c == '"' {
            out.push_str("\"\\\"\"");
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Text between the digits: plain where the notation allows it, a backslash for one character,
/// quotes otherwise.
fn literal(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (None, _) => String::new(),
        _ if text.chars().all(|c| PLAIN.contains(c)) => text.to_string(),
        (Some(c), None) if c != '"' => format!("\\{c}"),
        _ => quoted(text),
    }
}
