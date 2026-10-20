// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parser backend. This is the only module that knows formualizer-parse: it parses Excel
//! formula text and returns a small tree of the parts formulakit needs, with byte spans into
//! the text. Swapping the parser touches this file only.

use std::ops::Range;

use formualizer_parse::parser::{ExternalRefKind, ReferenceType, TableSpecifier};
use formualizer_parse::{ASTNode, ASTNodeType, FormulaDialect, Parser, ParserLimits};

use crate::ParseError;
use crate::refs::{CellPos, MAX_COL, MAX_ROW, Ref, SheetSpan, Whole};

/// What a reference token names.
pub(crate) enum Target {
    /// A cell, range, table or external reference.
    Ref(Ref),
    /// A name as written, possibly qualified (`gg`, `Sheet1!gg`, `[1]!gg`, `_xlpm.x`). The
    /// caller decides whether it is a LET or LAMBDA parameter or a defined name.
    Name(String),
}

/// A parsed formula, reduced to references and calls.
pub(crate) enum Node {
    Ref {
        span: Range<usize>,
        target: Target,
    },
    /// A function call. `name` is the function name as written; `span` covers the name.
    Func {
        name: String,
        span: Range<usize>,
        args: Vec<Node>,
    },
    /// Anything else (operators, calls of computed functions, arrays): only its children count.
    Group(Vec<Node>),
    Leaf,
}

/// Parses Excel formula text (with its leading `=`).
pub(crate) fn parse(text: &str) -> Result<Node, ParseError> {
    let parsed = std::panic::catch_unwind(|| {
        Parser::builder()
            .dialect(FormulaDialect::Excel)
            .limits(ParserLimits::default())
            .parse(text)
    });
    match parsed {
        Ok(Ok(ast)) => Ok(convert(&ast)),
        Ok(Err(e)) => Err(ParseError {
            message: match e.position {
                Some(pos) => format!("{} (at byte {pos})", e.message),
                None => e.message,
            },
        }),
        Err(_) => Err(ParseError {
            message: "the formula parser failed on this formula".to_string(),
        }),
    }
}

fn span_of(node: &ASTNode) -> Option<Range<usize>> {
    node.source_token.as_ref().map(|t| t.start..t.end)
}

// The tree height is bounded by the parser limits (256), so recursion depth is bounded too.
fn convert(node: &ASTNode) -> Node {
    match &node.node_type {
        ASTNodeType::Reference { reference, .. } => match span_of(node) {
            Some(span) => Node::Ref {
                span,
                target: target(reference),
            },
            None => Node::Leaf,
        },
        ASTNodeType::Function { name, args } => {
            let args = args.iter().map(convert).collect();
            match span_of(node) {
                Some(span) => Node::Func {
                    name: name.clone(),
                    span,
                    args,
                },
                None => Node::Group(args),
            }
        }
        ASTNodeType::Call { callee, args } => Node::Group(
            std::iter::once(callee.as_ref())
                .chain(args)
                .map(convert)
                .collect(),
        ),
        ASTNodeType::UnaryOp { expr, .. } => Node::Group(vec![convert(expr)]),
        ASTNodeType::BinaryOp { left, right, .. } => {
            Node::Group(vec![convert(left), convert(right)])
        }
        ASTNodeType::Array(rows) => Node::Group(rows.iter().flatten().map(convert).collect()),
        ASTNodeType::Literal(_) | ASTNodeType::Omitted => Node::Leaf,
    }
}

/// formualizer counts rows and columns from 1; formulakit from 0.
fn zero(n: u32) -> u32 {
    n.saturating_sub(1)
}

fn span(first: &str, last: Option<&str>) -> SheetSpan {
    SheetSpan {
        first: first.to_string(),
        last: last.map(str::to_string),
    }
}

fn cell(sheet: Option<SheetSpan>, row: u32, col: u32, abs_row: bool, abs_col: bool) -> Ref {
    Ref::Cell {
        sheet,
        pos: CellPos {
            row: zero(row),
            col: zero(col),
        },
        abs_row,
        abs_col,
    }
}

fn range(
    sheet: Option<SheetSpan>,
    start_row: Option<u32>,
    start_col: Option<u32>,
    end_row: Option<u32>,
    end_col: Option<u32>,
    abs: [bool; 4],
) -> Ref {
    let whole = match (start_row, end_row, start_col, end_col) {
        (None, None, _, _) => Whole::Cols,
        (_, _, None, None) => Whole::Rows,
        _ => Whole::None,
    };
    // An open end (`A1:A`, a Google Sheets form) runs to the edge of the sheet.
    Ref::Range {
        sheet,
        start: CellPos {
            row: start_row.map_or(0, zero),
            col: start_col.map_or(0, zero),
        },
        end: CellPos {
            row: end_row.map_or(MAX_ROW, zero),
            col: end_col.map_or(MAX_COL, zero),
        },
        abs,
        whole,
    }
}

fn target(reference: &ReferenceType) -> Target {
    match reference {
        ReferenceType::Cell {
            sheet,
            row,
            col,
            row_abs,
            col_abs,
        } => Target::Ref(cell(
            sheet.as_deref().map(|s| span(s, None)),
            *row,
            *col,
            *row_abs,
            *col_abs,
        )),
        ReferenceType::Cell3D {
            sheet_first,
            sheet_last,
            row,
            col,
            row_abs,
            col_abs,
        } => Target::Ref(cell(
            Some(span(sheet_first, Some(sheet_last))),
            *row,
            *col,
            *row_abs,
            *col_abs,
        )),
        ReferenceType::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            start_row_abs,
            start_col_abs,
            end_row_abs,
            end_col_abs,
        } => Target::Ref(range(
            sheet.as_deref().map(|s| span(s, None)),
            *start_row,
            *start_col,
            *end_row,
            *end_col,
            [*start_row_abs, *start_col_abs, *end_row_abs, *end_col_abs],
        )),
        ReferenceType::Range3D {
            sheet_first,
            sheet_last,
            start_row,
            start_col,
            end_row,
            end_col,
            start_row_abs,
            start_col_abs,
            end_row_abs,
            end_col_abs,
        } => Target::Ref(range(
            Some(span(sheet_first, Some(sheet_last))),
            *start_row,
            *start_col,
            *end_row,
            *end_col,
            [*start_row_abs, *start_col_abs, *end_row_abs, *end_col_abs],
        )),
        ReferenceType::External(ext) => {
            let sheet = Some(span(&ext.sheet, None));
            let inner = match ext.kind {
                ExternalRefKind::Cell {
                    row,
                    col,
                    row_abs,
                    col_abs,
                } => cell(sheet, row, col, row_abs, col_abs),
                ExternalRefKind::Range {
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                    start_row_abs,
                    start_col_abs,
                    end_row_abs,
                    end_col_abs,
                } => range(
                    sheet,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                    [start_row_abs, start_col_abs, end_row_abs, end_col_abs],
                ),
            };
            Target::Ref(Ref::External {
                book: ext.book.token().to_string(),
                inner: Box::new(inner),
            })
        }
        ReferenceType::Table(table) => Target::Ref(Ref::Table {
            name: table.name.clone(),
            column: table.specifier.as_ref().and_then(column),
        }),
        ReferenceType::NamedRange(name) => Target::Name(name.clone()),
    }
}

/// The column a table specifier names, if any (`first:last` for a span).
fn column(spec: &TableSpecifier) -> Option<String> {
    match spec {
        TableSpecifier::Column(c) => Some(c.clone()),
        TableSpecifier::ColumnRange(a, b) => Some(format!("{a}:{b}")),
        TableSpecifier::Combination(parts) => parts.iter().find_map(|p| column(p)),
        _ => None,
    }
}
