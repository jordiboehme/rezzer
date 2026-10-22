// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CSV and TSV reader: one sheet with the data table, a table of column facts and the sum
//! relations between columns as formulas.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use csvsniff::{ColumnKind, Dialect};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Block, Cell, Completeness, Document, FormatId, Formula, Inline, Loss, LossReason,
    ReadOptions, ReadOutcome, Reader, ReaderInfo, Role, SectionKind, Severity, Source, Value,
    column_letter, source_info,
};

/// Data rows in the body table. Further rows are analyzed and counted but not shown.
const MAX_TABLE_ROWS: usize = 10_000;

/// Cells in the body table after padding. Wide files show fewer rows so memory stays bounded.
const MAX_TABLE_CELLS: usize = 200_000;

/// Columns in the body table, the renderer's cap. Further fields are counted, not shown.
const MAX_TABLE_COLUMNS: usize = 1_000;

/// Columns listed in the Columns facts table.
const MAX_FACT_ROWS: usize = 500;

/// Rows read between two time checks.
const CHECK_EVERY: usize = 1000;

/// Reads CSV and TSV. The encoding and the dialect are detected; the output has the data as a
/// table, the facts per column and observed sum relations.
#[derive(Debug, Clone, Copy, Default)]
pub struct CsvReader;

impl Reader for CsvReader {
    fn id(&self) -> &'static str {
        "csv"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Csv, FormatId::Tsv]
    }

    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        let decoded = textenc::decode(src.bytes());
        let mut info = source_info(src);
        info.encoding = Some(decoded.encoding.to_string());
        info.reader = Some(ReaderInfo {
            id: self.id().to_string(),
            version: self.version().to_string(),
        });
        let mut doc = Document::new(info);
        let mut partial = false;

        if !decoded.confident {
            partial = true;
            doc.losses.push(Loss {
                location: "document".into(),
                what: "some bytes could not be decoded and were replaced with U+FFFD".into(),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }

        let text = decoded
            .text
            .strip_prefix('\u{feff}')
            .unwrap_or(&decoded.text);
        let sheet = Path::new(&src.name)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&src.name)
            .to_string();

        if text.trim().is_empty() {
            doc.body
                .push(sheet_of(&sheet, vec![note("The file is empty.")]));
            return finish(doc, partial);
        }

        let mut guessed = false;
        let mut dialect = match csvsniff::sniff(text) {
            Some(d) => d,
            None => {
                guessed = true;
                doc.losses.push(Loss {
                    location: "document".into(),
                    what: "dialect guessed from the file type".into(),
                    why: LossReason::Uncertain,
                    severity: Severity::Info,
                });
                Dialect {
                    delimiter: if src.format == FormatId::Tsv {
                        b'\t'
                    } else {
                        b','
                    },
                    quote: b'"',
                    has_header: false,
                    consistent_rows: 0.0,
                }
            }
        };
        let out_of_time = || budget.check_time().is_err();
        let mut analysis = csvsniff::analyze_until(text, &dialect, MAX_TABLE_ROWS, &out_of_time);
        if guessed && analysis.rows == 1 && !analysis.truncated {
            // A single record is most likely a header row.
            dialect.has_header = true;
            analysis = csvsniff::analyze_until(text, &dialect, MAX_TABLE_ROWS, &out_of_time);
        }
        let timed_out = analysis.truncated || budget.check_time().is_err();
        if timed_out {
            partial = true;
            // One loss per time event.
            let what = if analysis.truncated {
                format!(
                    "time limit reached during the analysis, only the first {} rows are counted, table not built",
                    analysis.rows
                )
            } else {
                "time limit reached after the analysis, table not built".to_string()
            };
            doc.losses.push(Loss {
                location: "document".into(),
                what,
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        }

        let custom = &mut doc.properties.custom;
        let delimiter = match dialect.delimiter {
            b'\t' => "tab".to_string(),
            d => char::from(d).to_string(),
        };
        custom.insert("delimiter".into(), Value::Text { value: delimiter });
        custom.insert("rows".into(), Value::number(analysis.rows as f64, None));
        // Replaced by the width of the padded table once it is known.
        custom.insert(
            "columns".into(),
            Value::number(analysis.columns.len() as f64, None),
        );
        custom.insert(
            "has_header".into(),
            Value::Bool {
                value: dialect.has_header,
            },
        );

        let record_of_line = record_numbers(text, &dialect, &analysis);
        for r in &analysis.ragged_rows {
            let location = match record_of_line.get(&r.line) {
                Some(n) => format!("record {n} (line {})", r.line),
                None => format!("line {}", r.line),
            };
            doc.losses.push(Loss {
                location,
                what: format!("row has {} fields, expected {}", r.fields, r.expected),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
        if analysis.ragged_total > analysis.ragged_rows.len() {
            doc.losses.push(Loss {
                location: "document".into(),
                what: format!(
                    "{} more rows with an unexpected field count not listed",
                    analysis.ragged_total - analysis.ragged_rows.len()
                ),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
        for problem in &analysis.quote_problems {
            let place = match record_at(text, &dialect, problem.offset) {
                Some((n, line)) => format!("record {n} (line {line})"),
                None => "end of file".to_string(),
            };
            let what = if problem.at_end {
                "unterminated quote: the rest of the file was read as one cell"
            } else {
                "unterminated quote: the cell ran over the following lines up to a quote inside a later field"
            };
            doc.losses.push(Loss {
                location: place,
                what: what.into(),
                why: LossReason::Damaged,
                severity: Severity::Error,
            });
        }
        let damaged = doc.losses.iter().any(|l| l.why == LossReason::Damaged);
        partial |= damaged;

        if analysis.rows == 0 {
            let msg = if dialect.has_header {
                "The file has a header row but no data rows."
            } else {
                "The file is empty."
            };
            doc.body.push(sheet_of(&sheet, vec![note(msg)]));
            return finish(doc, partial);
        }

        // The data table: header plus at most MAX_TABLE_ROWS data rows, at most
        // MAX_TABLE_COLUMNS wide and at most MAX_TABLE_CELLS cells after padding.
        let header = usize::from(dialect.has_header);
        let known = analysis.columns.len();
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(dialect.delimiter)
            .quote(dialect.quote)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut rec = csv::StringRecord::new();
        let wanted = MAX_TABLE_ROWS + header;
        // The padded width (capped) and the widest record seen (not capped).
        let (mut width, mut widest) = (0usize, 0usize);
        // Non-empty data cells per column past the known ones, for the facts table.
        let mut extra_filled: Vec<usize> = Vec::new();
        let mut stopped: Option<(usize, &str)> = None;
        while !timed_out && rows.len() < wanted {
            match reader.read_record(&mut rec) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    partial = true;
                    doc.losses.push(Loss {
                        location: format!("record {}", rows.len() + 1),
                        what: format!("record could not be read: {e}"),
                        why: LossReason::Damaged,
                        severity: Severity::Warning,
                    });
                    break;
                }
            }
            if rows.len() % CHECK_EVERY == CHECK_EVERY - 1 && budget.check_time().is_err() {
                stopped = Some((rows.len() + 1, "rest of the file not read"));
                break;
            }
            let shown = rec.len().min(MAX_TABLE_COLUMNS);
            let new_width = width.max(shown);
            // Padding makes every row as wide as the widest one: count padded cells.
            if !rows.is_empty() && (rows.len() + 1).saturating_mul(new_width) > MAX_TABLE_CELLS {
                break;
            }
            if budget.add_elements(shown as u64).is_err() {
                stopped = Some((
                    rows.len() + 1,
                    "element limit reached, rest of the file not read",
                ));
                break;
            }
            width = new_width;
            widest = widest.max(rec.len());
            if rows.len() >= header {
                for (j, v) in rec.iter().enumerate().take(MAX_FACT_ROWS).skip(known) {
                    if extra_filled.len() <= j - known {
                        extra_filled.resize(j - known + 1, 0);
                    }
                    if !value_empty(v) {
                        extra_filled[j - known] += 1;
                    }
                }
            }
            let row_no = rows.len() + 1; // 1-based, the header is row 1
            let cells = rec
                .iter()
                .take(MAX_TABLE_COLUMNS)
                .enumerate()
                .map(|(j, v)| anchored(&sheet, j, row_no, cell_inlines(v)))
                .collect();
            rows.push(cells);
        }
        let data_shown = rows.len().saturating_sub(header);
        if let Some((n, what)) = stopped {
            partial = true;
            doc.losses.push(Loss {
                location: format!("row {n}"),
                what: what.into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        } else if !timed_out && analysis.rows > data_shown {
            let what = if analysis.rows == data_shown + 1 {
                format!("row {} not shown", analysis.rows)
            } else {
                format!("rows {} to {} not shown", data_shown + 1, analysis.rows)
            };
            doc.losses.push(Loss {
                location: sheet.clone(),
                what,
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }
        if widest > MAX_TABLE_COLUMNS {
            doc.losses.push(Loss {
                location: sheet.clone(),
                what: format!(
                    "{} columns after column {MAX_TABLE_COLUMNS} not shown",
                    widest - MAX_TABLE_COLUMNS
                ),
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }

        // Rectangular: pad short rows (and the header) to the widest row.
        for (i, row) in rows.iter_mut().enumerate() {
            for j in row.len()..width {
                let label = if dialect.has_header && i == 0 {
                    format!("Column {}", j + 1)
                } else {
                    String::new()
                };
                row.push(anchored(&sheet, j, i + 1, vec![Inline::plain(label)]));
            }
        }

        let mut blocks = Vec::new();
        if !timed_out {
            blocks.push(Block::Table {
                caption: None,
                header_rows: header,
                role: None,
                rows,
                alignments: Vec::new(),
                anchor: None,
                layout: None,
            });
        }

        // The column facts. The column count is the widest record, also columns not shown.
        let ncols_total = if timed_out { known } else { widest.max(known) };
        custom_columns(&mut doc, ncols_total);
        let extra = ncols_total > known;
        let mut head = vec!["Column", "Type", "Unit", "Missing", "Unique", "Key"];
        if extra {
            head.push("Note");
        }
        let mut col_rows = vec![
            head.iter()
                .map(|h| Cell::new(vec![Inline::plain(*h)]))
                .collect::<Vec<_>>(),
        ];
        for c in analysis.columns.iter().take(MAX_FACT_ROWS) {
            let unique = if c.unique {
                "yes"
            } else if c.unique_exhausted {
                "not checked"
            } else {
                "no"
            };
            let mut values = vec![
                c.name.clone(),
                kind_label(&c.kind),
                c.unit.clone().unwrap_or_default(),
                c.missing.to_string(),
                unique.to_string(),
                if c.key_like { "yes" } else { "no" }.to_string(),
            ];
            if extra {
                values.push(String::new());
            }
            col_rows.push(
                values
                    .iter()
                    .map(|v| Cell::new(vec![Inline::plain(v)]))
                    .collect(),
            );
        }
        // Columns that exist only because some rows have more fields than the rest.
        for j in known..ncols_total.min(MAX_FACT_ROWS) {
            let filled = extra_filled.get(j - known).copied().unwrap_or(0);
            let values = [
                format!("Column {}", j + 1),
                if filled > 0 { "unknown" } else { "empty" }.to_string(),
                String::new(),
                (data_shown - filled).to_string(),
                "not checked".to_string(),
                "no".to_string(),
                "only in ragged rows".to_string(),
            ];
            col_rows.push(
                values
                    .iter()
                    .map(|v| Cell::new(vec![Inline::plain(v)]))
                    .collect(),
            );
        }
        if ncols_total > MAX_FACT_ROWS {
            doc.losses.push(Loss {
                location: sheet.clone(),
                what: format!(
                    "columns {} to {ncols_total} not listed in the Columns table",
                    MAX_FACT_ROWS + 1
                ),
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }
        blocks.push(Block::Table {
            caption: Some("Columns".into()),
            header_rows: 1,
            role: Some("summary".into()),
            rows: col_rows,
            alignments: Vec::new(),
            anchor: None,
            layout: None,
        });
        doc.body.push(sheet_of(&sheet, blocks));

        // Observed sum relations.
        let col_anchor = |j: usize| Anchor(format!("{sheet}!{0}:{0}", column_letter(j)));
        let name = |j: usize| analysis.columns[j].name.as_str();
        for d in &analysis.derived {
            let parts: Vec<&str> = d.parts.iter().map(|&j| name(j)).collect();
            doc.facets.formulas.push(Formula {
                anchor: col_anchor(d.target),
                expression: format!("{} = {}", name(d.target), parts.join(" + ")),
                dialect: "derived".into(),
                references: d.parts.iter().map(|&j| col_anchor(j)).collect(),
                cached: None,
                role: Some(Role::Result),
                pattern: None,
                latex: None,
                note: Some(format!("holds {} of {} rows", d.holds, d.checked)),
                kind: None,
                range: None,
                original: None,
            });
        }

        finish(doc, partial)
    }
}

/// A cell with its spreadsheet-style anchor (`sheet!B3`, `row` is 1-based, header is row 1).
fn anchored(sheet: &str, column: usize, row: usize, inlines: Vec<Inline>) -> Cell {
    let mut cell = Cell::new(inlines);
    cell.anchor = Some(Anchor(format!("{sheet}!{}{row}", column_letter(column))));
    cell
}

/// Sets the `columns` property.
fn custom_columns(doc: &mut Document, columns: usize) {
    doc.properties
        .custom
        .insert("columns".into(), Value::number(columns as f64, None));
}

/// True when a field has no visible text: empty, or only whitespace with a line break (which
/// [`cell_inlines`] turns into a lone line break).
fn value_empty(value: &str) -> bool {
    value.is_empty() || (value.trim().is_empty() && value.contains(['\n', '\r']))
}

/// The inlines of a cell: its text, with each run of whitespace that contains a line break
/// replaced by `Inline::LineBreak` (the renderer writes it as one space inside a table).
fn cell_inlines(value: &str) -> Vec<Inline> {
    if !value.contains(['\n', '\r']) {
        return vec![Inline::plain(value)];
    }
    let mut out: Vec<Inline> = Vec::new();
    let mut text = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, text: &mut String, out: &mut Vec<Inline>| {
        if run.contains(['\n', '\r']) {
            if !text.is_empty() {
                out.push(Inline::plain(text.as_str()));
                text.clear();
            }
            out.push(Inline::LineBreak);
        } else {
            text.push_str(run);
        }
        run.clear();
    };
    for c in value.chars() {
        if c.is_whitespace() {
            run.push(c);
        } else {
            flush(&mut run, &mut text, &mut out);
            text.push(c);
        }
    }
    flush(&mut run, &mut text, &mut out);
    if !text.is_empty() {
        out.push(Inline::plain(&text));
    }
    out
}

/// The 1-based record that contains the byte `offset`, with the line it starts on.
fn record_at(text: &str, dialect: &Dialect, offset: usize) -> Option<(usize, usize)> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(dialect.delimiter)
        .quote(dialect.quote)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rec = csv::StringRecord::new();
    let (mut n, mut found) = (0, None);
    while reader.read_record(&mut rec).unwrap_or(false) {
        n += 1;
        let pos = rec.position()?;
        if pos.byte() as usize > offset {
            break;
        }
        found = Some((n, pos.line() as usize));
    }
    found
}

/// Maps the physical start line of each listed ragged record to its 1-based record number.
fn record_numbers(
    text: &str,
    dialect: &Dialect,
    analysis: &csvsniff::Analysis,
) -> HashMap<usize, usize> {
    let mut map = HashMap::new();
    if analysis.ragged_rows.is_empty() {
        return map;
    }
    let wanted: HashSet<usize> = analysis.ragged_rows.iter().map(|r| r.line).collect();
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(dialect.delimiter)
        .quote(dialect.quote)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rec = csv::StringRecord::new();
    let mut n = 0;
    while reader.read_record(&mut rec).unwrap_or(false) {
        n += 1;
        if let Some(line) = rec.position().map(|p| p.line() as usize)
            && wanted.contains(&line)
        {
            map.insert(line, n);
            if map.len() == wanted.len() {
                break;
            }
        }
    }
    map
}

fn finish(document: Document, partial: bool) -> ReadOutcome {
    ReadOutcome {
        blobs: Vec::new(),
        document,
        completeness: if partial {
            Completeness::Partial
        } else {
            Completeness::Complete
        },
    }
}

fn sheet_of(name: &str, blocks: Vec<Block>) -> Block {
    Block::Section {
        kind: SectionKind::Sheet,
        title: Some(name.to_string()),
        anchor: Some(Anchor(name.to_string())),
        blocks,
    }
}

fn note(text: &str) -> Block {
    Block::paragraph(vec![Inline::plain(text)])
}

fn kind_label(kind: &ColumnKind) -> String {
    match kind {
        ColumnKind::Integer => "integer".into(),
        ColumnKind::Decimal { places } => {
            let s = if *places == 1 { "" } else { "s" };
            format!("decimal ({places} place{s})")
        }
        ColumnKind::Date => "date".into(),
        ColumnKind::Bool => "bool".into(),
        ColumnKind::Text => "text".into(),
        ColumnKind::Empty => "empty".into(),
    }
}
