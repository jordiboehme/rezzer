// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sheet layout, tables, formulas and drawing objects of one sheet.

use super::*;

/// The layout of an XLSX sheet: its cells with their classes and looks, its merges, Excel
/// tables, pivots, autofilter, names, outline, panes and print titles.
#[allow(clippy::too_many_arguments)]
pub(super) fn sheet_layout(
    q: &str,
    name: &str,
    wb: &WorkbookParts,
    parts: &SheetParts,
    cells: &SheetCells,
    merges: &[calamine::Dimensions],
    styles: &HashMap<(u32, u32), u32>,
    fmt: &Formatter,
    formulas: HashMap<(u32, u32), sheetlayout::FormulaIn>,
    budget: &Budget,
) -> layout::SheetLayout {
    let looks = wb.styles.has_looks();
    let style = |row: u32, col: u32| {
        if !looks {
            return sheetlayout::Style::default();
        }
        let index = styles.get(&(row, col)).copied().unwrap_or(0);
        wb.styles
            .look(index)
            .map(layout::style_of)
            .unwrap_or_default()
    };
    let shown = |c: &RawCell| {
        let (text, class, number) = match &c.val {
            Val::Num(v) => {
                let (text, kind) = fmt.number(c.row, c.col, *v);
                let class = if matches!(kind, NumKind::Date | NumKind::Time) {
                    sheetlayout::Class::Date
                } else {
                    sheetlayout::Class::Number
                };
                (text, class, Some(*v))
            }
            Val::Iso(_) => (fmt.shown(c), sheetlayout::Class::Date, None),
            Val::Bool(_) => (fmt.shown(c), sheetlayout::Class::Bool, None),
            Val::Error(_) => (fmt.shown(c), sheetlayout::Class::Error, None),
            Val::Text(_) | Val::Empty => (fmt.shown(c), sheetlayout::Class::Text, None),
        };
        layout::Shown {
            text,
            class,
            number,
        }
    };
    let centered: Vec<(u32, u32)> = if looks {
        let mut list: Vec<(u32, u32)> = parts
            .cell_styles
            .iter()
            .filter(|(_, s)| wb.styles.look(*s).is_some_and(|l| l.center_across))
            .map(|(p, _)| (p.row, p.col))
            .collect();
        list.sort_unstable();
        list
    } else {
        Vec::new()
    };
    let mut facts = Vec::new();
    for t in &parts.tables {
        if let Some(area) = grid::parse_area(&t.range) {
            facts.push(layout::table_fact(
                area,
                &t.name,
                t.header_rows,
                t.totals_rows,
                &t.columns,
            ));
        }
    }
    for p in &parts.pivots {
        if let Some(area) = grid::parse_area(&p.location) {
            // Decision 13: `colGrandTotals` draws the total row at the bottom, when the pivot
            // has row fields; `rowGrandTotals` the total column at the right, when it has
            // column fields.
            facts.push(sheetlayout::Fact {
                rect: layout::rect_of(area),
                kind: sheetlayout::FactKind::Pivot {
                    name: p.name.clone(),
                    first_header_row: p.first_header_row,
                    first_data_row: p.first_data_row,
                    first_data_col: p.first_data_col,
                    total_row: p.col_grand_totals && p.row_fields > 0,
                    total_col: p.row_grand_totals && p.col_fields > 0,
                },
            });
        }
    }
    if let Some(area) = parts.auto_filter.as_deref().and_then(grid::parse_area) {
        facts.push(sheetlayout::Fact {
            rect: layout::rect_of(area),
            kind: sheetlayout::FactKind::AutoFilter,
        });
    }
    let (names, caption_names) = layout::sheet_names(&wb.names, name);
    let signals = layout::Signals {
        facts,
        names,
        outline: layout::outline_runs(&parts.outline_rows),
        frozen: parts.frozen.map(|f| (f.rows, f.cols)),
        print_titles: print_title_rows(&wb.names, name).map(|s| (s.first, s.last)),
        styles_read: looks,
    };
    let merges: Vec<sheetlayout::Rect> = merges
        .iter()
        .map(|d| layout::rect_of((d.start, d.end)).normalized())
        .collect();
    layout::map_sheet(
        q,
        &cells.cells,
        &shown,
        &style,
        &centered,
        &merges,
        formulas,
        signals,
        &caption_names,
        budget,
    )
}

/// Registers the Excel tables of a sheet with the logic index.
pub(super) fn add_tables(out: &mut Out, formulas: &mut Formulas, sheet: &str, parts: &SheetParts) {
    for t in &parts.tables {
        let location = format!("table:{}", t.name);
        let Some(area) = grid::parse_area(&t.range) else {
            out.loss(
                location,
                format!(
                    "table {} has an unreadable range {:?}; its references read no cells",
                    t.name, t.range
                ),
                LossReason::Damaged,
                Severity::Warning,
            );
            continue;
        };
        if let Err(why) = formulas.add_table(
            &t.name,
            sheet,
            area,
            t.header_rows,
            t.totals_rows,
            &t.columns,
        ) {
            out.loss(
                location,
                format!("{why}; its references read no cells"),
                LossReason::Damaged,
                Severity::Warning,
            );
        }
    }
}

/// The formulas of a sheet into the formulas facet, and the value cells into the logic
/// index as constants. Returns the formulas that could not be read.
pub(super) fn sheet_formulas(
    out: &mut Out,
    formulas: &mut Formulas,
    sheet: &str,
    cells: &SheetCells,
    parts: &SheetParts,
    fmt: &Formatter,
    budget: &Budget,
) -> (Vec<FormulaProblem>, bool) {
    let q = quote_sheet(sheet);
    let attrs: HashMap<(u32, u32), &FormulaAttr> = parts
        .formula_attrs
        .iter()
        .map(|a| ((a.at.row, a.at.col), a))
        .collect();
    // Shared formula masters from the XML, for cells calamine met before their master.
    let mut masters: HashMap<u32, (&str, (u32, u32))> = HashMap::new();
    for a in &parts.formula_attrs {
        if let (AttrKind::Shared { index }, Some(text), Some(_)) = (&a.kind, &a.text, &a.range)
            && !text.is_empty()
        {
            masters.insert(*index, (text.as_str(), (a.at.row, a.at.col)));
        }
    }
    // Areas filled by array, spill and data table formulas: their cells are results, not
    // constants. Swept down the rows.
    let mut areas: Vec<((u32, u32), (u32, u32))> = parts
        .formula_attrs
        .iter()
        .filter(|a| !matches!(a.kind, AttrKind::Shared { .. }))
        .filter_map(|a| a.range.as_deref().and_then(grid::parse_area))
        .collect();
    areas.sort_by_key(|a| a.0.0);
    let mut next_area = 0usize;
    let mut active: Vec<((u32, u32), (u32, u32))> = Vec::new();

    let mut problems = Vec::new();
    let mut dropped: (u64, Option<(u32, u32)>) = (0, None);
    let mut lost_shared: Vec<(u32, u32)> = Vec::new();
    let mut stopped = false;
    for (i, c) in cells.cells.iter().enumerate() {
        if i % 1_024 == 1_023 && budget.check_time().is_err() {
            out.loss(
                format!("{q}!{}", grid::a1(c.row, c.col)),
                "time limit reached: the formulas from this cell on and the later sheets not read",
                LossReason::Limit,
                Severity::Warning,
            );
            stopped = true;
            break;
        }
        let pos = (c.row, c.col);
        if c.text_dropped {
            dropped.0 += 1;
            dropped.1.get_or_insert(pos);
            continue;
        }
        let attr = attrs.get(&pos).copied();
        let mut text = c.formula.clone();
        if text.is_none()
            && c.shared_unknown
            && let Some(AttrKind::Shared { index }) = attr.map(|a| &a.kind)
            && let Some((t, start)) = masters.get(index)
        {
            // The same meter as every other formula text: checked before the copy is made,
            // charged with its real length after.
            let copy = if formulas.meter.fits(t.len()) {
                calamine::expand_shared_formula(t, *start, pos).ok()
            } else {
                None
            };
            match copy {
                Some(f) if formulas.meter.take(f.len()) => text = Some(f),
                Some(_) | None if formulas.meter.is_full() => {
                    dropped.0 += 1;
                    dropped.1.get_or_insert(pos);
                    continue;
                }
                _ => {}
            }
        }
        let index = out.doc.facets.formulas.len();
        if let Some(text) = text {
            let (kind, range) = match attr.map(|a| &a.kind) {
                Some(AttrKind::Array) => (FormulaKind::Array, attr.and_then(|a| a.range.clone())),
                Some(AttrKind::DynamicArray) => {
                    (FormulaKind::Spill, attr.and_then(|a| a.range.clone()))
                }
                Some(AttrKind::Shared { .. }) => (FormulaKind::Shared, None),
                _ if c.shared => (FormulaKind::Shared, None),
                _ => (FormulaKind::Cell, None),
            };
            let (f, problem) = formulas.cell(
                sheet,
                c.row,
                c.col,
                &text,
                kind,
                range,
                fmt.cached(c),
                index,
            );
            out.doc.facets.formulas.push(f);
            problems.extend(problem);
            continue;
        }
        if let Some(FormulaAttr {
            kind:
                AttrKind::DataTable {
                    r1,
                    r2,
                    two_d,
                    row_input,
                },
            range,
            ..
        }) = attr
        {
            let f = formulas.data_table(
                sheet,
                c.row,
                c.col,
                range.as_deref(),
                r1.as_deref(),
                r2.as_deref(),
                *two_d,
                *row_input,
                fmt.cached(c),
                index,
            );
            out.doc.facets.formulas.push(f);
            continue;
        }
        if c.shared_unknown {
            lost_shared.push(pos);
            continue;
        }
        // A value cell: a constant unless an array, spill or data table fills it.
        while let Some(a) = areas.get(next_area).filter(|a| a.0.0 <= c.row) {
            active.push(*a);
            next_area += 1;
        }
        active.retain(|a| a.1.0 >= c.row);
        let filled = active.iter().any(|a| (a.0.1..=a.1.1).contains(&c.col));
        if !filled && c.val != Val::Empty {
            formulas.add_constant(sheet, c.row, c.col);
        }
    }
    formulas.unindexed_formulas += dropped.0 + lost_shared.len() as u64;
    if let (n, Some((r, c))) = dropped {
        out.loss(
            format!("{q}!{}", grid::a1(r, c)),
            format!(
                "formula text limit reached ({} bytes in all): {n} formula cells from {} on not listed; their values are shown",
                formulas.meter.max(),
                grid::a1(r, c)
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if !lost_shared.is_empty() {
        let first: Vec<String> = lost_shared
            .iter()
            .take(5)
            .map(|(r, c)| grid::a1(*r, *c))
            .collect();
        out.loss(
            q.as_str(),
            format!(
                "{} cells of a shared formula whose master cell is missing; their formula text is not known (first {})",
                lost_shared.len(),
                first.join(", ")
            ),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    (problems, stopped)
}

/// Excel tables, pivot tables and charts as small tables in the sheet section.
pub(super) fn objects(blocks: &mut Vec<Block>, q: &str, parts: &SheetParts) {
    let plain = |s: &str| Cell::new(vec![Inline::plain(s)]);
    let head = |cols: &[&str]| cols.iter().map(|c| plain(c)).collect::<Vec<_>>();
    let small = |caption: &str, role: &str, rows: Vec<Vec<Cell>>| Block::Table {
        caption: Some(caption.to_string()),
        header_rows: 1,
        role: Some(role.to_string()),
        rows,
        alignments: Vec::new(),
        anchor: None,
        layout: None,
    };
    if !parts.tables.is_empty() {
        let mut rows = vec![head(&["Table", "Range", "Columns"])];
        for t in &parts.tables {
            let mut name = plain(&t.name);
            name.anchor = Some(Anchor(format!("table:{}", t.name)));
            let mut range = plain(&t.range);
            range.anchor = Some(Anchor(format!("{q}!{}", t.range.replace('$', ""))));
            rows.push(vec![name, range, plain(&t.columns.join(", "))]);
        }
        blocks.push(small("Excel tables", "excel-table", rows));
    }
    if !parts.pivots.is_empty() {
        let mut rows = vec![head(&["Pivot table", "Location", "Source"])];
        for p in &parts.pivots {
            let mut location = plain(&p.location);
            location.anchor = Some(Anchor(format!("{q}!{}", p.location.replace('$', ""))));
            rows.push(vec![
                plain(&p.name),
                location,
                plain(p.source.as_deref().unwrap_or("not known")),
            ]);
        }
        blocks.push(small("Pivot tables", "pivot", rows));
    }
    if !parts.charts.is_empty() {
        let mut rows = vec![head(&["Chart", "Title", "Series"])];
        for ch in &parts.charts {
            let series: Vec<String> = ch
                .series
                .iter()
                .map(|(name, values)| match name {
                    Some(n) => format!("{n}: {values}"),
                    None => values.clone(),
                })
                .collect();
            rows.push(vec![
                plain(&ch.kind),
                plain(ch.title.as_deref().unwrap_or("")),
                plain(&series.join("; ")),
            ]);
        }
        blocks.push(small("Charts", "chart", rows));
    }
}
