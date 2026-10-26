// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CSV and TSV reader: one sheet with the tables, titles and notes the layout detector finds
//! in the records, a table of column facts and the sum relations between columns as
//! formulas.

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

/// The records a CSV reader read, for the column facts of its table regions.
struct Records<'a> {
    text: &'a str,
    dialect: &'a Dialect,
    /// The sheet row and the first byte of each record read, in order.
    starts: &'a [(u32, usize)],
    /// The positions of the cells with text, in reading order.
    filled: &'a [(u32, u32)],
    /// The field count of the widest record read.
    widest: usize,
}

/// The Columns tables of a file's table regions, the losses of those that could not be
/// made and the derived relations found in them.
struct RegionFacts {
    tables: Vec<Block>,
    losses: Vec<Loss>,
    formulas: Vec<Formula>,
}

/// One Columns table per table region, in region order, from the region's header and data
/// rows (C8): a title, caption or note line is never counted as data, and a second table in
/// the file gets its own facts. The records from the region's last header row to its last
/// data row are analyzed on their own; the last region of a file whose rows were not all
/// shown reaches to the end of the file. A region whose analysis the time limit cuts gets an
/// Uncertain loss instead of a table. `None` when the layout found no table region.
fn region_facts(
    part: &Records,
    layout: &Layout,
    sheet_q: &str,
    stop: &dyn Fn() -> bool,
) -> Option<RegionFacts> {
    let regions: Vec<&sheetlayout::Region> = layout
        .regions
        .iter()
        .filter(|r| r.kind == Kind::Table && r.data.is_some())
        .collect();
    if regions.is_empty() {
        return None;
    }
    let mut out = RegionFacts {
        tables: Vec::new(),
        losses: Vec::new(),
        formulas: Vec::new(),
    };
    let mut not_counted: Vec<String> = Vec::new();
    // Tables side by side share their records: the last analysis is kept for the next.
    let mut last: Option<((usize, usize, bool), csvsniff::Analysis)> = None;
    for region in regions {
        let Some(data) = region.data else {
            continue;
        };
        let range = region.rect.a1();
        if !not_counted.is_empty() || stop() {
            not_counted.push(range);
            continue;
        }
        let first = region.header_rows.last().copied().unwrap_or(data.top);
        let at = |row: u32| part.starts.partition_point(|s| s.0 < row);
        let from = at(first);
        let to = part.starts.partition_point(|s| s.0 <= data.bottom);
        let Some(&(_, start)) = part.starts.get(from) else {
            continue;
        };
        // The last region of the file also counts the records that were not shown.
        let end = part.starts.get(to).map_or(part.text.len(), |s| s.1);
        let shown = to.saturating_sub(at(data.top));
        let has_header = !region.header_rows.is_empty();
        let slice = (start, end, has_header);
        if last.as_ref().is_none_or(|(at, _)| *at != slice) {
            let dialect = Dialect {
                lead_records: 0,
                has_header,
                ..part.dialect.clone()
            };
            let found =
                csvsniff::analyze_until(&part.text[start..end], &dialect, MAX_TABLE_ROWS, stop);
            last = Some((slice, found));
        }
        let Some((_, analysis)) = last.as_ref() else {
            continue;
        };
        if analysis.truncated {
            out.losses.push(Loss {
                location: format!("{sheet_q}!{range}"),
                what: format!(
                    "time limit reached while the column facts of the table {range} were counted; it has no Columns table"
                ),
                why: LossReason::Uncertain,
                severity: Severity::Info,
            });
            continue;
        }
        let body = region.body.unwrap_or(data);
        let left = body.left as usize;
        // A table that reaches the last shown column also has the fields not shown.
        let mut right = body.right as usize;
        if right + 1 >= MAX_TABLE_COLUMNS {
            right = right.max(part.widest.saturating_sub(1));
        }
        let known = analysis.columns.len();
        // A header band of several rows names the columns by their paths.
        let path = |j: usize| -> Option<String> {
            if region.header_rows.len() < 2 {
                return None;
            }
            let p = region.header_paths.get(j.checked_sub(left)?)?;
            let p = p.split(['\n', '\r']).collect::<Vec<_>>().join(" ");
            let p = p.trim();
            (!p.is_empty()).then(|| p.to_string())
        };
        let name = |j: usize| -> String {
            path(j).unwrap_or_else(|| match analysis.columns.get(j) {
                Some(c) => c.name.clone(),
                None => format!("Column {}", j + 1),
            })
        };
        // Fields of columns that only ragged rows have, counted in the data rows shown.
        let mut extra = vec![0usize; right.saturating_add(1).saturating_sub(known.max(left))];
        let lo = part.filled.partition_point(|p| p.0 < data.top);
        let hi = part.filled.partition_point(|p| p.0 <= data.bottom);
        for &(_, col) in &part.filled[lo..hi] {
            let col = col as usize;
            if col >= known.max(left)
                && col <= right
                && let Some(n) = extra.get_mut(col - known.max(left))
            {
                *n += 1;
            }
        }
        let listed = right.min(left + MAX_FACT_ROWS - 1);
        let facts: Vec<(Vec<String>, bool)> = (left..=listed)
            .map(|j| match analysis.columns.get(j) {
                Some(c) => (fact_row(c, name(j)), false),
                None => {
                    let filled = extra.get(j - known.max(left)).copied().unwrap_or(0);
                    (ragged_row(j, filled, shown), true)
                }
            })
            .collect();
        let caption = format!("Columns {range}");
        if listed < right {
            out.losses.push(Loss {
                location: sheet_q.to_string(),
                what: format!(
                    "columns {} to {} not listed in {caption}",
                    listed + 2,
                    right + 1
                ),
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }
        out.tables.push(columns_block(caption, facts));
        let inside = |j: usize| (left..=right).contains(&j);
        for d in &analysis.derived {
            if inside(d.target) && d.parts.iter().all(|&j| inside(j)) {
                out.formulas.push(derived_formula(sheet_q, d, &name));
            }
        }
    }
    if !not_counted.is_empty() {
        let n = not_counted.len();
        let mut named: Vec<String> = not_counted.into_iter().take(10).collect();
        if n > named.len() {
            named.push(format!("{} more", n - named.len()));
        }
        out.losses.push(Loss {
            location: sheet_q.to_string(),
            what: format!(
                "time limit reached before the column facts were counted; no Columns table for the tables {}",
                named.join(", ")
            ),
            why: LossReason::Uncertain,
            severity: Severity::Info,
        });
    }
    Some(out)
}

/// The Columns table row of a column csvsniff analyzed, named `name`.
fn fact_row(c: &csvsniff::Column, name: String) -> Vec<String> {
    let unique = if c.unique {
        "yes"
    } else if c.unique_exhausted {
        "not checked"
    } else {
        "no"
    };
    vec![
        name,
        kind_label(&c.kind),
        c.unit.clone().unwrap_or_default(),
        c.missing.to_string(),
        unique.to_string(),
        if c.key_like { "yes" } else { "no" }.to_string(),
    ]
}

/// The Columns table row of column `j` (0-based) that only ragged rows have: `filled` of the
/// `rows` data rows have a field there.
fn ragged_row(j: usize, filled: usize, rows: usize) -> Vec<String> {
    vec![
        format!("Column {}", j + 1),
        if filled > 0 { "unknown" } else { "empty" }.to_string(),
        String::new(),
        rows.saturating_sub(filled).to_string(),
        "not checked".to_string(),
        "no".to_string(),
    ]
}

/// A Columns table with `caption`. Each fact says whether its column is only in ragged rows;
/// when one is, the table has a Note column that says so.
fn columns_block(caption: String, facts: Vec<(Vec<String>, bool)>) -> Block {
    let note = facts.iter().any(|(_, ragged)| *ragged);
    let mut head = vec!["Column", "Type", "Unit", "Missing", "Unique", "Key"];
    if note {
        head.push("Note");
    }
    let cells = |values: &[String]| -> Vec<Cell> {
        values
            .iter()
            .map(|v| Cell::new(vec![Inline::plain(v)]))
            .collect()
    };
    let mut rows = vec![
        head.iter()
            .map(|h| Cell::new(vec![Inline::plain(*h)]))
            .collect::<Vec<_>>(),
    ];
    for (mut values, ragged) in facts {
        if note {
            values.push(if ragged { "only in ragged rows" } else { "" }.to_string());
        }
        rows.push(cells(&values));
    }
    Block::Table {
        caption: Some(caption),
        header_rows: 1,
        role: Some("summary".into()),
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: None,
    }
}

/// An observed sum relation as a formula on whole columns; `name` names a column by its
/// 0-based index.
fn derived_formula(
    sheet_q: &str,
    d: &csvsniff::Derived,
    name: &dyn Fn(usize) -> String,
) -> Formula {
    let col_anchor = |j: usize| Anchor(format!("{sheet_q}!{0}:{0}", column_letter(j)));
    let parts: Vec<String> = d.parts.iter().map(|&j| name(j)).collect();
    Formula {
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
    }
}

/// Whether sheet row `row` is a data row of a table or grid region: inside its data rows
/// and with no row kind or the kind `data`. Header rows, group rows, caption, heading and
/// note lines and the rows of label and value, text or title regions are not.
fn in_table_row(layout: &Layout, row: u32) -> bool {
    layout.regions.iter().any(|r| {
        matches!(r.kind, Kind::Table | Kind::Grid)
            && r.data.is_some_and(|d| d.top <= row && row <= d.bottom)
            && !r.captions.contains(&row)
            && !r.notes.contains(&row)
            && !r.above_header.contains(&row)
            && !r.header_rows.contains(&row)
            && r.caption_heading != Some(row)
            && r.heading.is_none_or(|(h, _)| h != row)
            && r.rows
                .iter()
                .find(|n| n.row == row)
                .is_none_or(|n| n.kind == RowKind::Data)
    })
}

/// Puts the ragged-row losses that `keep` accepts (by the line a record starts on; the
/// count of rows not listed is always kept) at `at` among the losses.
fn place_ragged(
    doc: &mut Document,
    at: usize,
    ragged: Vec<(Option<usize>, Loss)>,
    keep: impl Fn(usize) -> bool,
) {
    let kept: Vec<Loss> = ragged
        .into_iter()
        .filter(|(line, _)| line.is_none_or(&keep))
        .map(|(_, loss)| loss)
        .collect();
    doc.losses.splice(at..at, kept);
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

#[cfg(test)]
mod tests {
    use std::cell::Cell as Count;

    use sheetlayout::{Certainty, Rect, Region};

    use super::*;

    /// A table region with header row `header` (0-based) over the rows to `bottom`.
    fn table(header: u32, bottom: u32) -> Region {
        let rect = Rect {
            top: header,
            left: 0,
            bottom,
            right: 1,
        };
        Region {
            kind: Kind::Table,
            rect,
            data: Some(Rect {
                top: header + 1,
                ..rect
            }),
            captions: Vec::new(),
            caption_heading: None,
            notes: Vec::new(),
            heading: None,
            above_header: Vec::new(),
            header_rows: vec![header],
            body: Some(rect),
            header_paths: vec!["a".into(), "b".into()],
            label_cols: 0,
            total_cols: Vec::new(),
            certainty: Certainty::Rule,
            evidence: Vec::new(),
            unsure: Vec::new(),
            rows: Vec::new(),
            hidden_header: false,
            rest: false,
        }
    }

    #[test]
    fn a_cut_region_analysis_gives_a_loss_per_region() {
        // Two tables: rows 0 to 5000, a blank line, then rows 5002 to 5004.
        let mut text = String::from("a,b\n");
        let mut starts = vec![(0u32, 0usize)];
        for i in 1..=5_000u32 {
            starts.push((i, text.len()));
            text.push_str(&format!("{i},{i}\n"));
        }
        text.push('\n');
        for (k, line) in ["c,d\n", "1,2\n", "3,4\n"].iter().enumerate() {
            starts.push((5_002 + k as u32, text.len()));
            text.push_str(line);
        }
        let dialect = csvsniff::sniff(&text).unwrap();
        let layout = Layout {
            regions: vec![table(0, 5_000), table(5_002, 5_004)],
            issues: Vec::new(),
        };
        let part = Records {
            text: &text,
            dialect: &dialect,
            starts: &starts,
            filled: &[],
            widest: 2,
        };
        // The first analysis is cut inside, the second region is not started.
        let calls = Count::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 1
        };
        let found = region_facts(&part, &layout, "data", &stop).unwrap();
        assert!(found.tables.is_empty());
        let losses: Vec<(&str, &str)> = found
            .losses
            .iter()
            .map(|l| (l.location.as_str(), l.what.as_str()))
            .collect();
        assert_eq!(
            losses,
            [
                (
                    "data!A1:B5001",
                    "time limit reached while the column facts of the table A1:B5001 were counted; it has no Columns table"
                ),
                (
                    "data",
                    "time limit reached before the column facts were counted; no Columns table for the tables A5003:B5005"
                ),
            ]
        );
        assert!(
            found
                .losses
                .iter()
                .all(|l| l.why == LossReason::Uncertain && l.severity == Severity::Info)
        );
        // Without a stop each region has its table, the second one from its own rows.
        let found = region_facts(&part, &layout, "data", &|| false).unwrap();
        assert!(found.losses.is_empty());
        let captions: Vec<Option<&str>> = found
            .tables
            .iter()
            .map(|t| match t {
                Block::Table { caption, .. } => caption.as_deref(),
                _ => None,
            })
            .collect();
        assert_eq!(
            captions,
            [Some("Columns A1:B5001"), Some("Columns A5003:B5005")]
        );
        let Block::Table { rows, .. } = &found.tables[1] else {
            panic!()
        };
        assert_eq!(rows.len(), 3);
    }
}
