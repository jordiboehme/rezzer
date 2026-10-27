// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CSV and TSV reader: one sheet with the tables, titles and notes the layout detector finds
//! in the records, a table of column facts and the sum relations between columns as
//! formulas.

mod regions;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use csvsniff::{CellClass, ColumnKind, Dialect};
use rezzer_layout::{Caps, CellText, ColumnNames};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Block, Cell, Completeness, Document, FormatId, Formula, Inline, LayoutKind, Loss,
    LossReason, ReadOptions, ReadOutcome, Reader, ReaderInfo, Role, SectionKind, Severity, Source,
    Value, column_letter, source_info,
};
use sheetlayout::{CellIn, Class, Input, Kind, Layout, Pos, RowKind, Style};

use regions::{
    Records, RegionFacts, columns_block, derived_formula, fact_row, in_table_row, place_ragged,
    ragged_row, region_facts,
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

/// Bytes of a text that still reads as a label, as in the sheet reader.
const MAX_LABEL: usize = 200;

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
        // The file stem as a reference writes a sheet name: cell anchors, table keys, record
        // anchors and the sheet's own anchor use it, like a workbook sheet.
        let sheet_q = sheetparts::quote_sheet(&sheet);

        if text.trim().is_empty() {
            doc.body
                .push(sheet_of(&sheet, &sheet_q, vec![note("The file is empty.")]));
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
                    lead_records: 0,
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

        // The ragged-row losses wait for the layout: a record that became a caption, note or
        // text line is no damaged table row. They keep their place among the losses.
        let record_of_line = record_numbers(text, &dialect, &analysis);
        let ragged_at = doc.losses.len();
        let mut ragged: Vec<(Option<usize>, Loss)> = Vec::new();
        for r in &analysis.ragged_rows {
            let location = match record_of_line.get(&r.line) {
                Some(n) => format!("record {n} (line {})", r.line),
                None => format!("line {}", r.line),
            };
            let loss = Loss {
                location,
                what: format!("row has {} fields, expected {}", r.fields, r.expected),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            };
            ragged.push((Some(r.line), loss));
        }
        if analysis.ragged_total > analysis.ragged_rows.len() {
            let loss = Loss {
                location: "document".into(),
                what: format!(
                    "{} more rows with an unexpected field count not listed",
                    analysis.ragged_total - analysis.ragged_rows.len()
                ),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            };
            ragged.push((None, loss));
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

        if analysis.rows == 0 {
            place_ragged(&mut doc, ragged_at, ragged, |_| true);
            partial |= damaged(&doc);
            let msg = if dialect.has_header {
                "The file has a header row but no data rows."
            } else {
                "The file is empty."
            };
            doc.body.push(sheet_of(&sheet, &sheet_q, vec![note(msg)]));
            return finish(doc, partial);
        }

        // The shown records: title lines, the header and at most MAX_TABLE_ROWS data rows, at
        // most MAX_TABLE_COLUMNS wide and at most MAX_TABLE_CELLS cells after padding. A run of
        // blank lines between two records is one empty sheet row.
        let lead = dialect.lead_records;
        let skip = lead + usize::from(dialect.has_header);
        let known = analysis.columns.len();
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(dialect.delimiter)
            .quote(dialect.quote)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut rec = csv::StringRecord::new();
        let wanted = MAX_TABLE_ROWS + skip;
        // The padded width (capped) and the widest record seen (not capped).
        let (mut width, mut widest) = (0usize, 0usize);
        // Non-empty data cells per column past the known ones, for the facts table.
        let mut extra_filled: Vec<usize> = Vec::new();
        let mut stopped: Option<(u64, &str)> = None;
        let ragged_lines: HashSet<usize> = analysis.ragged_rows.iter().map(|r| r.line).collect();
        let mut row_of_line: HashMap<usize, u32> = HashMap::new();
        let mut texts: Vec<CellText> = Vec::new();
        let mut input: Vec<CellIn> = Vec::new();
        let mut guess_row: Option<u32> = None;
        // The sheet row and the first byte of each record read, in order.
        let mut starts: Vec<(u32, usize)> = Vec::new();
        // Records read, the sheet row of the next one (0-based) and where the last one ended.
        let (mut read, mut row) = (0usize, 0u32);
        let mut after: Option<usize> = None;
        let max_elements = budget.limits().max_elements;
        while !timed_out && read < wanted {
            match reader.read_record(&mut rec) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    partial = true;
                    doc.losses.push(Loss {
                        location: format!("record {}", read + 1),
                        what: format!("record could not be read: {e}"),
                        why: LossReason::Damaged,
                        severity: Severity::Warning,
                    });
                    break;
                }
            }
            if after.is_some_and(|end| blank_line_after(text, end)) {
                row = row.saturating_add(1);
            }
            if read % CHECK_EVERY == CHECK_EVERY - 1 && budget.check_time().is_err() {
                stopped = Some((u64::from(row) + 1, "rest of the file not read"));
                break;
            }
            let shown = rec.len().min(MAX_TABLE_COLUMNS);
            let new_width = width.max(shown);
            // Padding makes every row as wide as the widest one: count padded cells.
            if read > 0 && (read + 1).saturating_mul(new_width) > MAX_TABLE_CELLS {
                break;
            }
            // Stop before the record that would pass the element limit, so the layout can
            // still place the records read. Only fields with text count here: the layout
            // charges the empty positions of its tables itself.
            let filled = rec
                .iter()
                .take(MAX_TABLE_COLUMNS)
                .filter(|v| !v.trim().is_empty())
                .count() as u64;
            if budget.elements().saturating_add(filled) > max_elements
                || budget.add_elements(filled).is_err()
            {
                stopped = Some((
                    u64::from(row) + 1,
                    "element limit reached, rest of the file not read",
                ));
                break;
            }
            width = new_width;
            widest = widest.max(rec.len());
            if read >= skip {
                for (j, v) in rec.iter().enumerate().take(MAX_FACT_ROWS).skip(known) {
                    if extra_filled.len() <= j - known {
                        extra_filled.resize(j - known + 1, 0);
                    }
                    if !value_empty(v) {
                        extra_filled[j - known] += 1;
                    }
                }
            }
            if read == lead {
                guess_row = Some(row);
            }
            starts.push((row, rec.position().map_or(0, |p| p.byte() as usize)));
            if let Some(line) = rec.position().map(|p| p.line() as usize)
                && ragged_lines.contains(&line)
            {
                row_of_line.insert(line, row);
            }
            for (j, v) in rec.iter().take(MAX_TABLE_COLUMNS).enumerate() {
                let shown = shown_text(v);
                if shown.is_empty() {
                    continue;
                }
                let col = j as u32;
                input.push(cell_in(row, col, v, &shown));
                texts.push(CellText {
                    row,
                    col,
                    text: shown,
                });
            }
            after = Some(reader.position().byte() as usize);
            read += 1;
            row = row.saturating_add(1);
        }
        let data_shown = read.saturating_sub(skip);
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
                location: sheet_q.clone(),
                what,
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }
        if widest > MAX_TABLE_COLUMNS {
            doc.losses.push(Loss {
                location: sheet_q.clone(),
                what: format!(
                    "{} columns after column {MAX_TABLE_COLUMNS} not shown",
                    widest - MAX_TABLE_COLUMNS
                ),
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }

        // The layout decides titles, headers and blocks; csvsniff's header guess is one of
        // its signals.
        let mut blocks = Vec::new();
        // The Columns tables of the table regions with their derived relations; `None` keeps
        // the one table of the whole file.
        let mut by_region: Option<RegionFacts> = None;
        if timed_out {
            place_ragged(&mut doc, ragged_at, ragged, |_| true);
        } else {
            let input = Input {
                cells: input,
                merges: Vec::new(),
                facts: Vec::new(),
                names: Vec::new(),
                outline: Vec::new(),
                frozen: None,
                print_titles: None,
                header_guess: guess_row.map(|r| (r, dialect.has_header)),
                styles_read: false,
                is_label: &is_label,
                is_unit_column: &|_| false,
            };
            let stop = || budget.check_time().is_err();
            let layout = sheetlayout::detect(&input, &rezzer_layout::limits(budget), &stop);
            // The cells with text, in reading order: the Columns tables count the fields of
            // columns that only ragged rows have from them.
            let filled: Vec<(u32, u32)> =
                input.cells.iter().map(|c| (c.pos.row, c.pos.col)).collect();
            drop(input);
            let mapped = rezzer_layout::blocks(
                &sheet_q,
                &layout,
                &texts,
                &[],
                ColumnNames::Numbers,
                &[],
                &mut Caps::default(),
                budget,
            );
            drop(texts);
            // A ragged record is listed when it is a row of a table, or was not shown.
            place_ragged(&mut doc, ragged_at, ragged, |line| {
                row_of_line
                    .get(&line)
                    .is_none_or(|&r| in_table_row(&layout, r))
            });
            partial |= mapped.partial || mapped.elements_out;
            doc.losses.extend(mapped.losses);
            blocks = mapped.blocks;
            // The column facts follow the layout (C8): one Columns table per table region,
            // from its header and data rows.
            let part = Records {
                text,
                dialect: &dialect,
                starts: &starts,
                filled: &filled,
                widest,
            };
            by_region = region_facts(&part, &layout, &sheet_q, &out_of_time);
            // The layout decides the header and so the data rows: the rows of every region
            // table without its header rows, plus the records not shown.
            let placed: usize = blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table {
                        rows,
                        header_rows,
                        layout: Some(_),
                        ..
                    } => Some(rows.len().saturating_sub(*header_rows)),
                    _ => None,
                })
                .sum();
            let has_header = blocks.iter().find_map(|b| match b {
                Block::Table {
                    header_rows,
                    layout: Some(l),
                    ..
                } if l.kind == LayoutKind::Table => Some(*header_rows > 0),
                _ => None,
            });
            let custom = &mut doc.properties.custom;
            let rows = placed + analysis.rows.saturating_sub(data_shown);
            custom.insert("rows".into(), Value::number(rows as f64, None));
            custom.insert(
                "has_header".into(),
                Value::Bool {
                    value: has_header.unwrap_or(false),
                },
            );
        }
        partial |= damaged(&doc);

        // The column facts. The column count is the widest record, also columns not shown.
        let ncols_total = if timed_out { known } else { widest.max(known) };
        custom_columns(&mut doc, ncols_total);
        match by_region {
            Some(found) => {
                blocks.extend(found.tables);
                doc.losses.extend(found.losses);
                doc.facets.formulas.extend(found.formulas);
            }
            None => {
                // The whole file as one table, with csvsniff's header guess: after a time
                // cut, or when the layout found no table.
                let mut facts: Vec<(Vec<String>, bool)> = analysis
                    .columns
                    .iter()
                    .take(MAX_FACT_ROWS)
                    .map(|c| (fact_row(c, c.name.clone()), false))
                    .collect();
                // Columns that exist only because some rows have more fields than the rest.
                for j in known..ncols_total.min(MAX_FACT_ROWS) {
                    let filled = extra_filled.get(j - known).copied().unwrap_or(0);
                    facts.push((ragged_row(j, filled, data_shown), true));
                }
                if ncols_total > MAX_FACT_ROWS {
                    doc.losses.push(Loss {
                        location: sheet_q.clone(),
                        what: format!(
                            "columns {} to {ncols_total} not listed in the Columns table",
                            MAX_FACT_ROWS + 1
                        ),
                        why: LossReason::Limit,
                        severity: Severity::Info,
                    });
                }
                blocks.push(columns_block("Columns".into(), facts));
                let name = |j: usize| analysis.columns[j].name.clone();
                for d in &analysis.derived {
                    doc.facets
                        .formulas
                        .push(derived_formula(&sheet_q, d, &name));
                }
            }
        }
        doc.body.push(sheet_of(&sheet, &sheet_q, blocks));

        finish(doc, partial)
    }
}

/// Sets the `columns` property.
fn custom_columns(doc: &mut Document, columns: usize) {
    doc.properties
        .custom
        .insert("columns".into(), Value::number(columns as f64, None));
}

/// True when a field has no visible text: empty, or only whitespace with a line break.
fn value_empty(value: &str) -> bool {
    value.is_empty() || (value.trim().is_empty() && value.contains(['\n', '\r']))
}

/// The shown text of a field: empty when it holds only whitespace, else the field with each
/// run of whitespace that contains a line break made one `\n` (the renderer writes it as one
/// space inside a table).
fn shown_text(value: &str) -> String {
    if value.trim().is_empty() {
        return String::new();
    }
    if !value.contains(['\n', '\r']) {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut run = String::new();
    for c in value.chars() {
        if c.is_whitespace() {
            run.push(c);
            continue;
        }
        flush_run(&mut run, &mut out);
        out.push(c);
    }
    flush_run(&mut run, &mut out);
    out
}

fn flush_run(run: &mut String, out: &mut String) {
    if run.contains(['\n', '\r']) {
        out.push('\n');
    } else {
        out.push_str(run);
    }
    run.clear();
}

/// A field as the layout detector sees it: its class from csvsniff's header rule and no
/// style (CSV has none).
fn cell_in(row: u32, col: u32, value: &str, shown: &str) -> CellIn {
    let (class, number) = match csvsniff::cell_class(value) {
        CellClass::Number => (Class::Number, value.trim().parse::<f64>().ok()),
        CellClass::Date => (Class::Date, None),
        CellClass::Bool => (Class::Bool, None),
        CellClass::Text | CellClass::Empty => (Class::Text, None),
    };
    CellIn {
        pos: Pos { row, col },
        class,
        text: shown.to_string(),
        number,
        style: Style::default(),
        formula: None,
    }
}

/// Whether a text reads as a label: not empty after trimming and at most [`MAX_LABEL`] bytes
/// (the label rule of the sheet reader).
fn is_label(text: &str) -> bool {
    text.len() <= MAX_LABEL && !text.trim().is_empty()
}

/// Whether a blank line follows the record that ended at byte `end` of `text`. The csv
/// reader skips blank lines and puts the next record's position at `end`, so the text from
/// there tells. `end` can sit between the `\r` and the `\n` of the record's line break;
/// that `\n` belongs to the record.
fn blank_line_after(text: &str, end: usize) -> bool {
    let Some(rest) = text.get(end..) else {
        return false;
    };
    let rest = if text[..end].ends_with('\r') {
        rest.strip_prefix('\n').unwrap_or(rest)
    } else {
        rest
    };
    rest.starts_with(['\n', '\r'])
}

/// Whether a loss says the file is damaged.
fn damaged(doc: &Document) -> bool {
    doc.losses.iter().any(|l| l.why == LossReason::Damaged)
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

fn sheet_of(name: &str, quoted: &str, blocks: Vec<Block>) -> Block {
    Block::Section {
        kind: SectionKind::Sheet,
        title: Some(name.to_string()),
        anchor: Some(Anchor(quoted.to_string())),
        blocks,
    }
}

fn note(text: &str) -> Block {
    Block::paragraph(vec![Inline::plain(text)])
}

#[cfg(test)]
mod tests;
