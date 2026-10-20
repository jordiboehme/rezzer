// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parser backend. This is the only module that knows formualizer-parse: it parses Excel
//! formula text and returns a small tree of the parts formulakit needs, with byte spans into
//! the text. Swapping the parser touches this file only.

use std::ops::Range;

use formualizer_parse::parser::{
    BinaryOperator, ExternalRefKind, ReferenceType, TableSpecifier, UnaryOperator,
};
use formualizer_parse::{ASTNode, ASTNodeType, FormulaDialect, LiteralValue, Parser, ParserLimits};

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

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Plus,
    Minus,
    Percent,
    /// `A1#`, the spill range of a dynamic array.
    Spill,
    /// `@A1`, implicit intersection.
    At,
    /// An operator added to the parser after this code was written.
    Other,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// `:` between parts that do not fold into one range (a name, a function result).
    Range,
    /// `,` inside parentheses.
    Union,
    /// A space between two references.
    Intersect,
    /// An operator added to the parser after this code was written.
    Other,
}

/// A literal value.
pub(crate) enum Lit {
    /// A number as written (`1000`, `1.5E+3`).
    Number(String),
    /// A string without its quotes, `""` already turned into `"`.
    Text(String),
    Bool(bool),
    /// An error literal as written (`#N/A`).
    Error(String),
}

/// A parsed formula: references, calls, operators and literals, with byte spans where the
/// caller edits the text.
pub(crate) enum Node {
    Ref {
        span: Range<usize>,
        target: Target,
    },
    /// References joined by the range operator on one sheet (`Sheet1!A1:Sheet1!B2`,
    /// `A1:B2:C3`), which read the bounding area `folded`. Each part keeps its own span so
    /// the pattern key can rewrite it without touching the text between the parts.
    Area {
        parts: Vec<(Range<usize>, Ref)>,
        folded: Ref,
    },
    /// A function call. `name` is the function name as written; `span` covers the name.
    Func {
        name: String,
        span: Range<usize>,
        args: Vec<Node>,
    },
    /// A call of a computed function (`LAMBDA(x,x+1)(2)`).
    Call {
        callee: Box<Node>,
        args: Vec<Node>,
    },
    Unary {
        op: UnOp,
        expr: Box<Node>,
    },
    Binary {
        op: BinOp,
        left: Box<Node>,
        right: Box<Node>,
    },
    Array(Vec<Vec<Node>>),
    Literal(Lit),
    /// An omitted argument (`IF(A1,,2)`), or a node the parser gave no source for.
    Empty,
}

/// Parser limits. The Pratt frame limit is raised from formualizer's default (72) so that
/// Excel's 64 levels of nested functions parse even when each level adds parentheses and an
/// operator (`SUM(1+(...))` takes about three frames per level; 66 levels pass). The
/// deepest formulas these limits let through parse on a 512 KiB thread stack in a debug
/// build. The tree height stays at the default 256, which bounds the recursion of every
/// walk over the tree.
const PRATT_FRAMES: usize = 200;

/// Parses Excel formula text (with its leading `=`).
pub(crate) fn parse(text: &str) -> Result<Node, ParseError> {
    let parsed = std::panic::catch_unwind(|| {
        Parser::builder()
            .dialect(FormulaDialect::Excel)
            .limits(ParserLimits::default().with_pratt_frames(PRATT_FRAMES))
            .parse(text)
    });
    match parsed {
        Ok(Ok(ast)) => Ok(convert(&ast, text)),
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

fn un_op(op: UnaryOperator) -> UnOp {
    match op {
        UnaryOperator::Plus => UnOp::Plus,
        UnaryOperator::Minus => UnOp::Minus,
        UnaryOperator::Percent => UnOp::Percent,
        UnaryOperator::Spill => UnOp::Spill,
        UnaryOperator::ImplicitIntersection => UnOp::At,
        _ => UnOp::Other,
    }
}

fn bin_op(op: BinaryOperator) -> BinOp {
    match op {
        BinaryOperator::Add => BinOp::Add,
        BinaryOperator::Subtract => BinOp::Sub,
        BinaryOperator::Multiply => BinOp::Mul,
        BinaryOperator::Divide => BinOp::Div,
        BinaryOperator::Power => BinOp::Pow,
        BinaryOperator::Concat => BinOp::Concat,
        BinaryOperator::Equal => BinOp::Eq,
        BinaryOperator::NotEqual => BinOp::Ne,
        BinaryOperator::Less => BinOp::Lt,
        BinaryOperator::Greater => BinOp::Gt,
        BinaryOperator::LessEqual => BinOp::Le,
        BinaryOperator::GreaterEqual => BinOp::Ge,
        BinaryOperator::Range => BinOp::Range,
        BinaryOperator::Union => BinOp::Union,
        BinaryOperator::Intersection => BinOp::Intersect,
        _ => BinOp::Other,
    }
}

fn literal(node: &ASTNode, value: &LiteralValue, text: &str) -> Node {
    let written = || {
        span_of(node)
            .and_then(|s| text.get(s))
            .map(str::to_string)
            .unwrap_or_default()
    };
    match value {
        LiteralValue::Text(t) => Node::Literal(Lit::Text(t.clone())),
        LiteralValue::Boolean(b) => Node::Literal(Lit::Bool(*b)),
        LiteralValue::Error(_) => Node::Literal(Lit::Error(written())),
        LiteralValue::Int(_) | LiteralValue::Number(_) => Node::Literal(Lit::Number(written())),
        _ => Node::Literal(Lit::Text(written())),
    }
}

// The tree height is bounded by the parser limits (256), so recursion depth is bounded too.
fn convert(node: &ASTNode, text: &str) -> Node {
    let conv = |n: &ASTNode| convert(n, text);
    match &node.node_type {
        ASTNodeType::Reference { reference, .. } => match span_of(node) {
            Some(span) => Node::Ref {
                span,
                target: target(reference),
            },
            None => Node::Empty,
        },
        ASTNodeType::Function { name, args } => {
            let args: Vec<Node> = args.iter().map(conv).collect();
            match span_of(node) {
                Some(span) => Node::Func {
                    name: name.clone(),
                    span,
                    args,
                },
                // Without a source span the name cannot be edited; keep the arguments.
                None => Node::Call {
                    callee: Box::new(Node::Empty),
                    args,
                },
            }
        }
        ASTNodeType::Call { callee, args } => Node::Call {
            callee: Box::new(conv(callee)),
            args: args.iter().map(conv).collect(),
        },
        ASTNodeType::UnaryOp { op, expr } => Node::Unary {
            op: un_op(*op),
            expr: Box::new(conv(expr)),
        },
        ASTNodeType::BinaryOp { op, left, right } => {
            let left = conv(left);
            let right = conv(right);
            if *op == BinaryOperator::Range {
                fold(left, right)
            } else {
                Node::Binary {
                    op: bin_op(*op),
                    left: Box::new(left),
                    right: Box::new(right),
                }
            }
        }
        ASTNodeType::Array(rows) => {
            Node::Array(rows.iter().map(|r| r.iter().map(conv).collect()).collect())
        }
        ASTNodeType::Literal(value) => literal(node, value, text),
        ASTNodeType::Omitted => Node::Empty,
    }
}

/// The parts of a folded area (each with its span) and the area they span.
type Parts = (Vec<(Range<usize>, Ref)>, Ref);

/// The corners of a cell or range: start, end, `$` flags as in [`Ref::Range`], whole, sheet.
type Corners<'a> = (CellPos, CellPos, [bool; 4], Whole, &'a Option<SheetSpan>);

/// The parts and the area of a node that can take part in a range fold.
fn area_parts(node: Node) -> Result<Parts, Node> {
    match node {
        Node::Ref {
            span,
            target: Target::Ref(r @ (Ref::Cell { .. } | Ref::Range { .. })),
        } => Ok((vec![(span, r.clone())], r)),
        Node::Area { parts, folded } => Ok((parts, folded)),
        other => Err(other),
    }
}

fn corners(r: &Ref) -> Option<Corners<'_>> {
    match r {
        Ref::Cell {
            sheet,
            pos,
            abs_row,
            abs_col,
        } => Some((
            *pos,
            *pos,
            [*abs_row, *abs_col, *abs_row, *abs_col],
            Whole::None,
            sheet,
        )),
        Ref::Range {
            sheet,
            start,
            end,
            abs,
            whole,
        } => Some((*start, *end, *abs, *whole, sheet)),
        _ => None,
    }
}

/// The bounding range of two cells or ranges on the same sheet, if they are on one.
fn bounding(a: &Ref, b: &Ref) -> Option<Ref> {
    let (s1, e1, f1, w1, sh1) = corners(a)?;
    let (s2, e2, f2, w2, sh2) = corners(b)?;
    if sh1 != sh2 {
        return None;
    }
    // Each coordinate keeps the `$` flag of the reference it comes from.
    let lo = |x: u32, fx: bool, y: u32, fy: bool| if y < x { (y, fy) } else { (x, fx) };
    let hi = |x: u32, fx: bool, y: u32, fy: bool| if y > x { (y, fy) } else { (x, fx) };
    let (sr, fsr) = lo(s1.row, f1[0], s2.row, f2[0]);
    let (sc, fsc) = lo(s1.col, f1[1], s2.col, f2[1]);
    let (er, fer) = hi(e1.row, f1[2], e2.row, f2[2]);
    let (ec, fec) = hi(e1.col, f1[3], e2.col, f2[3]);
    let whole = if (w1 == Whole::Cols || w2 == Whole::Cols) && sr == 0 && er == MAX_ROW {
        Whole::Cols
    } else if (w1 == Whole::Rows || w2 == Whole::Rows) && sc == 0 && ec == MAX_COL {
        Whole::Rows
    } else {
        Whole::None
    };
    Some(Ref::Range {
        sheet: sh1.clone(),
        start: CellPos { row: sr, col: sc },
        end: CellPos { row: er, col: ec },
        abs: [fsr, fsc, fer, fec],
        whole,
    })
}

/// `left:right`: folds two references on one sheet into the area they span; anything else
/// stays a range operator.
fn fold(left: Node, right: Node) -> Node {
    let (left, right) = match (area_parts(left), area_parts(right)) {
        (Ok((mut lp, la)), Ok((rp, ra))) => match bounding(&la, &ra) {
            Some(folded) => {
                lp.extend(rp);
                return Node::Area { parts: lp, folded };
            }
            None => (unfold(lp, la), unfold(rp, ra)),
        },
        (l, r) => (back(l), back(r)),
    };
    Node::Binary {
        op: BinOp::Range,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn back(taken: Result<Parts, Node>) -> Node {
    match taken {
        Ok((parts, folded)) => unfold(parts, folded),
        Err(node) => node,
    }
}

/// Puts parts taken out by [`area_parts`] back into a node.
fn unfold(mut parts: Vec<(Range<usize>, Ref)>, folded: Ref) -> Node {
    if parts.len() == 1
        && let Some((span, r)) = parts.pop()
    {
        return Node::Ref {
            span,
            target: Target::Ref(r),
        };
    }
    Node::Area { parts, folded }
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
    // An open end (`A1:A`, a Google Sheets form) runs to the edge of the sheet. It is
    // marked absolute, so the pattern key names the edge and copies share the key.
    let abs = [
        abs[0],
        abs[1],
        abs[2] || (end_row.is_none() && start_row.is_some()),
        abs[3] || (end_col.is_none() && start_col.is_some()),
    ];
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
