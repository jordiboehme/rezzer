// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parts of an ODF spreadsheet (ods) that a spreadsheet crate does not expose: hidden
//! sheets, rows and columns, annotations with author and date, data validations, number
//! formats built from data styles, named ranges and the document metadata.
//!
//! Positions are 0-based like the rest of the crate. `number-rows-repeated` and
//! `number-columns-repeated` move the position by arithmetic and are never expanded into items.
//! A repeated cell that carries content (a value, a formula, a note) is listed per cell, and
//! those cells count against [`Limits::max_items`](crate::Limits). Counts that run past the
//! sheet size (1,048,576 rows, 16,384 columns) are clamped and reported as an [`Issue`].
//!
//! Nothing in the file is executed. The part is XML read through the bounded scanner.

mod budget;
mod content;
mod rules;
mod settings;
mod state;
mod styles;

use zipguard::Archive;

use crate::ooxml::{Ctx, DocProps, Frozen, Validation};
use crate::xml::Ev;
use crate::{CellPos, DefinedName, Issue, Limits, Look, PropValue, RowSpan};
use content::Content;
use settings::Settings;
use styles::StyleBook;

const CONTENT: &str = "content.xml";
const SETTINGS: &str = "settings.xml";

/// A note on a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// The A1 address of the cell.
    pub cell: String,
    pub author: Option<String>,
    /// The date as written in the file, for example `2024-09-18T18:27:00`.
    pub date: Option<String>,
    /// The paragraphs of the note, joined with newlines.
    pub text: String,
}

/// One sheet of an ODF spreadsheet, in tab order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdsSheet {
    pub name: String,
    /// The table style says `display="false"`. ODF has no "very hidden" state.
    pub hidden: bool,
    /// Rows with `visibility` `collapse` or `filter`, merged into runs.
    pub hidden_rows: Vec<RowSpan>,
    /// Columns with `visibility` `collapse` or `filter`, merged into runs. The `first` and
    /// `last` of a span are column numbers.
    pub hidden_cols: Vec<RowSpan>,
    pub annotations: Vec<Annotation>,
    /// Data validations. `range` is the A1 range of the cells that use the rule, several
    /// ranges separated by spaces. `kind` is `unknown` when the condition is not understood.
    pub validations: Vec<Validation>,
    /// The number format code of each cell with content whose style has a data style.
    pub cell_formats: Vec<(CellPos, String)>,
    /// Merged cells as (top left, bottom right), from `number-columns-spanned` and
    /// `number-rows-spanned`. A span of one row and one column is no merge.
    pub merges: Vec<(CellPos, CellPos)>,
    /// The look of each cell whose look is not plain: cells with content and cells that name
    /// a style of their own. The style comes from the cell, else its row, else its column.
    pub looks: Vec<(CellPos, Look)>,
    /// Whether the file had cell styles at all. When it did not, the looks are missing, which
    /// is not the same as plain.
    pub has_looks: bool,
    /// The rows inside `table:table-header-rows`, as one span (first to last when there are
    /// several groups).
    pub header_rows: Option<RowSpan>,
    /// Row groups: spans of rows and the nesting depth of `table:table-row-group` around them
    /// (1 for the outermost). Rows outside any group are not listed. Sibling groups stay apart.
    pub outline: Vec<(RowSpan, u8)>,
    /// Rows and columns fixed by a frozen pane, from `settings.xml`.
    pub frozen: Option<Frozen>,
}

/// A database range (`table:database-range`): a named table, an autofilter or a sort area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRange {
    pub name: String,
    pub sheet: String,
    /// The cells as an A1 range (`A1:C10`, `C4` for one cell).
    pub range: String,
    /// `table:contains-header`, true when the file does not say.
    pub header: bool,
    /// `calcext:contains-footer`.
    pub footer: bool,
    /// `table:display-filter-buttons`.
    pub filter_buttons: bool,
}

/// A pivot table (`table:data-pilot-table`) and where it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPilot {
    pub name: String,
    /// The sheet of the target range.
    pub sheet: String,
    /// The target range as an A1 range.
    pub range: String,
}

/// What `read` finds in an ODF spreadsheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OdsParts {
    pub sheets: Vec<OdsSheet>,
    /// Global names have no scope, sheet-local ones the sheet name. A named range gives
    /// `refers_to` as a sheet reference (`Sheet1!$A$1:$B$2`); a named expression keeps its
    /// expression as written (`of:=0.19`).
    pub names: Vec<DefinedName>,
    pub database_ranges: Vec<DatabaseRange>,
    pub data_pilots: Vec<DataPilot>,
    /// Core keys: `creator` (the first author), `lastModifiedBy`, `created`, `modified`,
    /// `title`, `subject`, `description`, `keywords`, `language`. App keys: `Application`,
    /// `EditingDuration`, `EditingCycles`.
    pub props: DocProps,
    pub issues: Vec<Issue>,
}

/// Reads the parts of an ODF spreadsheet. A problem becomes an issue; the other parts are
/// still read.
pub fn read(zip: &mut Archive<'_>, limits: &Limits, stop: &dyn Fn() -> bool) -> OdsParts {
    let mut ctx = Ctx {
        zip,
        limits,
        stop,
        issues: Vec::new(),
    };
    let mut book = StyleBook::default();
    ctx.scan("styles.xml", false, &mut |ev, _| book.event(&ev));

    let mut settings = Settings::default();
    ctx.scan(SETTINGS, false, &mut |ev, depth| settings.event(&ev, depth));

    let mut content = Content::new(book, limits.max_items as u64);
    content.frozen = settings.frozen();
    ctx.scan(CONTENT, true, &mut |ev, depth| content.event(&ev, depth));
    let props = read_meta(&mut ctx);
    let mut issues = ctx.issues;
    issues.append(&mut content.issues);
    content.summary(&mut issues);
    OdsParts {
        sheets: content.sheets,
        names: content.names,
        database_ranges: content.database_ranges,
        data_pilots: content.data_pilots,
        props,
        issues,
    }
}

fn read_meta(ctx: &mut Ctx<'_, '_>) -> DocProps {
    let mut props = DocProps::default();
    let mut key: Option<(String, Option<String>, Option<String>)> = None;
    let mut text = String::new();
    let mut keywords: Vec<String> = Vec::new();
    ctx.scan("meta.xml", false, &mut |ev, depth| match ev {
        Ev::Start(e) if depth == 3 => {
            key = Some((
                e.name.clone(),
                e.attr("name").map(str::to_string),
                e.attr("value-type").map(str::to_string),
            ));
            text.clear();
        }
        Ev::Empty(e) if depth == 3 && e.name == "user-defined" => {
            if let Some(n) = e.attr("name") {
                props
                    .custom
                    .push((n.to_string(), custom_value(e.attr("value-type"), "")));
            }
        }
        Ev::Text(t) if key.is_some() => text.push_str(t),
        Ev::End(_) if depth == 3 => {
            let Some((name, prop, ty)) = key.take() else {
                return;
            };
            let value = text.trim().to_string();
            if name == "user-defined" {
                if let Some(n) = prop {
                    props.custom.push((n, custom_value(ty.as_deref(), &value)));
                }
            } else if name == "keyword" {
                if !value.is_empty() {
                    keywords.push(value);
                }
            } else if !value.is_empty() {
                let (map, k) = match name.as_str() {
                    "initial-creator" => (true, "creator"),
                    "creator" => (true, "lastModifiedBy"),
                    "creation-date" => (true, "created"),
                    "date" => (true, "modified"),
                    "title" => (true, "title"),
                    "subject" => (true, "subject"),
                    "description" => (true, "description"),
                    "language" => (true, "language"),
                    "generator" => (false, "Application"),
                    "editing-duration" => (false, "EditingDuration"),
                    "editing-cycles" => (false, "EditingCycles"),
                    _ => return,
                };
                let target = if map { &mut props.core } else { &mut props.app };
                target.insert(k.to_string(), value);
            }
        }
        _ => {}
    });
    if !keywords.is_empty() {
        props
            .core
            .insert("keywords".to_string(), keywords.join(", "));
    }
    props
}

fn custom_value(ty: Option<&str>, value: &str) -> PropValue {
    let other = |t: &str| PropValue::Other {
        ty: t.to_string(),
        value: value.to_string(),
    };
    match ty {
        None | Some("string") => PropValue::Text(value.to_string()),
        Some("float") => value
            .parse()
            .map_or_else(|_| other("float"), PropValue::Number),
        Some("boolean") => match value {
            "true" => PropValue::Bool(true),
            "false" => PropValue::Bool(false),
            _ => other("boolean"),
        },
        Some("date") => PropValue::DateTime(value.to_string()),
        Some(t) => other(t),
    }
}
