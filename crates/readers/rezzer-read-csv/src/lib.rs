// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CSV and TSV reader: one sheet with the data table, a table of column facts and the sum
//! relations between columns as formulas.

use std::path::Path;

use csvsniff::{ColumnKind, Dialect};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Block, Cell, Completeness, Document, FormatId, Formula, Inline, Loss, LossReason,
    ReadOptions, ReadOutcome, Reader, ReaderInfo, Role, SectionKind, Severity, Source, Value,
    source_info,
};

/// Data rows in the body table. Further rows are analyzed and counted but not shown.
const MAX_TABLE_ROWS: usize = 10_000;

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

        let dialect = match csvsniff::sniff(text) {
            Some(d) => d,
            None => {
                doc.losses.push(Loss {
                    location: "document".into(),
                    what: "dialect guessed from the file type".into(),
                    why: LossReason::Unsupported,
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
        let analysis = csvsniff::analyze(text, &dialect, MAX_TABLE_ROWS);

        let custom = &mut doc.properties.custom;
        let delimiter = match dialect.delimiter {
            b'\t' => "tab".to_string(),
            d => char::from(d).to_string(),
        };
        custom.insert("delimiter".into(), Value::Text { value: delimiter });
        custom.insert("rows".into(), Value::number(analysis.rows as f64, None));
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

        for r in &analysis.ragged_rows {
            doc.losses.push(Loss {
                location: format!("line {}", r.line),
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
        if analysis.unterminated_quote {
            doc.losses.push(Loss {
                location: "end of file".into(),
                what: "a quoted field is not closed".into(),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
        let damaged = doc.losses.iter().any(|l| l.why == LossReason::Damaged);

        if analysis.rows == 0 {
            let msg = if dialect.has_header {
                "The file has a header row but no data rows."
            } else {
                "The file is empty."
            };
            doc.body.push(sheet_of(&sheet, vec![note(msg)]));
            return finish(doc, partial || damaged);
        }

        // The data table: header plus at most MAX_TABLE_ROWS data rows.
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(dialect.delimiter)
            .quote(dialect.quote)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut rec = csv::StringRecord::new();
        let wanted = MAX_TABLE_ROWS + usize::from(dialect.has_header);
        let mut stopped_at: Option<usize> = None;
        while rows.len() < wanted && reader.read_record(&mut rec).unwrap_or(false) {
            if rows.len() % CHECK_EVERY == CHECK_EVERY - 1 && budget.check_time().is_err() {
                stopped_at = Some(rows.len() + 1);
                break;
            }
            let row_no = rows.len() + 1; // 1-based, the header is row 1
            let cells = rec
                .iter()
                .enumerate()
                .map(|(j, v)| {
                    let mut c = Cell::new(vec![plain(v)]);
                    c.anchor = Some(Anchor(format!("{sheet}!{}{row_no}", column_letter(j))));
                    c
                })
                .collect();
            rows.push(cells);
        }
        if let Some(n) = stopped_at {
            partial = true;
            doc.losses.push(Loss {
                location: format!("row {n}"),
                what: "rest of the file not read".into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        } else if analysis.rows > MAX_TABLE_ROWS {
            doc.losses.push(Loss {
                location: sheet.clone(),
                what: format!("rows {} to {} not shown", MAX_TABLE_ROWS + 1, analysis.rows),
                why: LossReason::Limit,
                severity: Severity::Info,
            });
        }

        let mut blocks = vec![Block::Table {
            caption: None,
            header_rows: usize::from(dialect.has_header),
            rows,
            anchor: None,
        }];

        // The column facts.
        let mut col_rows = vec![
            ["Column", "Type", "Unit", "Missing", "Unique", "Key"]
                .iter()
                .map(|h| Cell::new(vec![plain(h)]))
                .collect::<Vec<_>>(),
        ];
        for c in &analysis.columns {
            let unique = if c.unique {
                "yes"
            } else if c.unique_exhausted {
                "not checked"
            } else {
                "no"
            };
            let values = [
                c.name.clone(),
                kind_label(&c.kind),
                c.unit.clone().unwrap_or_default(),
                c.missing.to_string(),
                unique.to_string(),
                if c.key_like { "yes" } else { "no" }.to_string(),
            ];
            col_rows.push(values.iter().map(|v| Cell::new(vec![plain(v)])).collect());
        }
        blocks.push(Block::Table {
            caption: Some("Columns".into()),
            header_rows: 1,
            rows: col_rows,
            anchor: None,
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
                pattern: Some(format!("holds {} of {} rows", d.holds, d.checked)),
                latex: None,
            });
        }

        let damaged = doc.losses.iter().any(|l| l.why == LossReason::Damaged);
        finish(doc, partial || damaged)
    }
}

fn finish(document: Document, partial: bool) -> ReadOutcome {
    ReadOutcome {
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

fn plain(text: &str) -> Inline {
    Inline::Text {
        text: text.to_string(),
        emphasis: false,
        strong: false,
    }
}

fn note(text: &str) -> Block {
    Block::Paragraph {
        inlines: vec![plain(text)],
        role: None,
        anchor: None,
    }
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

/// Spreadsheet column letters: 0 is A, 25 is Z, 26 is AA.
fn column_letter(mut index: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::column_letter;

    #[test]
    fn letters() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(25), "Z");
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(701), "ZZ");
        assert_eq!(column_letter(702), "AAA");
    }
}
