// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use super::sheet::{Chart, Pivot, TableInfo};
use super::{Ctx, Rel};
use crate::a1::quote_sheet;
use crate::xml::Ev;
use crate::{Issue, IssueKind};

/// Reads an Excel table part.
pub(super) fn read_table(ctx: &mut Ctx<'_, '_>, part: &str) -> Option<TableInfo> {
    let mut table: Option<TableInfo> = None;
    let mut in_columns = false;
    ctx.scan(part, true, &mut |ev, _| match ev {
        Ev::Start(e) | Ev::Empty(e) => match e.name.as_str() {
            "table" if table.is_none() => {
                let name = e
                    .attr("displayName")
                    .or_else(|| e.attr("name"))
                    .unwrap_or("");
                table = Some(TableInfo {
                    name: name.to_string(),
                    range: e.attr("ref").unwrap_or("").to_string(),
                    columns: Vec::new(),
                    header_rows: e
                        .attr("headerRowCount")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1),
                    totals_rows: e
                        .attr("totalsRowCount")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                });
            }
            "tableColumns" => in_columns = matches!(ev, Ev::Start(_)),
            "tableColumn" if in_columns => {
                if let Some(t) = table.as_mut() {
                    t.columns.push(e.attr("name").unwrap_or("").to_string());
                }
            }
            _ => {}
        },
        Ev::End("tableColumns") => in_columns = false,
        _ => {}
    });
    table
}

/// Finds where pivot caches come from. Loaded on the first pivot table of a sheet.
#[derive(Default)]
pub(super) struct PivotSources {
    /// cacheId to the cache definition part, from the workbook.
    parts: Option<HashMap<String, String>>,
    /// cacheId to the source text, once read.
    sources: HashMap<String, Option<String>>,
}

impl PivotSources {
    fn source(&mut self, ctx: &mut Ctx<'_, '_>, cache_id: &str) -> Option<String> {
        if self.parts.is_none() {
            self.parts = Some(load_cache_parts(ctx));
        }
        if let Some(s) = self.sources.get(cache_id) {
            return s.clone();
        }
        let Some(part) = self.parts.as_ref().and_then(|p| p.get(cache_id)).cloned() else {
            ctx.issues.push(Issue::new(
                "xl/workbook.xml",
                IssueKind::Damaged,
                format!("pivot cache {cache_id} is not in the workbook"),
            ));
            self.sources.insert(cache_id.to_string(), None);
            return None;
        };
        let source = read_cache_source(ctx, &part);
        self.sources.insert(cache_id.to_string(), source.clone());
        source
    }
}

fn load_cache_parts(ctx: &mut Ctx<'_, '_>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let root = ctx.rels("");
    let Some(workbook) = root
        .iter()
        .find(|r| r.is("/officeDocument") && !r.external)
        .map(|r| r.target.clone())
        .or_else(|| {
            ctx.zip
                .contains("xl/workbook.xml")
                .then(|| "xl/workbook.xml".to_string())
        })
    else {
        return out;
    };
    let rels: Vec<Rel> = ctx.rels(&workbook);
    let mut ids: Vec<(String, String)> = Vec::new();
    ctx.scan(&workbook, true, &mut |ev, _| {
        if let Ev::Start(e) | Ev::Empty(e) = ev
            && e.name == "pivotCache"
        {
            ids.push((
                e.attr("cacheId").unwrap_or("").to_string(),
                e.attr("id").unwrap_or("").to_string(),
            ));
        }
    });
    for (cache_id, rid) in ids {
        if let Some(rel) = rels.iter().find(|r| r.id == rid && !r.external) {
            out.insert(cache_id, rel.target.clone());
        }
    }
    out
}

fn read_cache_source(ctx: &mut Ctx<'_, '_>, part: &str) -> Option<String> {
    let mut kind: Option<String> = None;
    let mut source: Option<String> = None;
    let mut external = false;
    ctx.scan(part, true, &mut |ev, _| {
        if let Ev::Start(e) | Ev::Empty(e) = ev {
            match e.name.as_str() {
                "cacheSource" if kind.is_none() => kind = e.attr("type").map(str::to_string),
                "worksheetSource" if source.is_none() && e.attr("id").is_some() => {
                    external = true;
                }
                "worksheetSource" if source.is_none() => {
                    source = if let Some(name) = e.attr("name") {
                        Some(name.to_string())
                    } else {
                        match (e.attr("sheet"), e.attr("ref")) {
                            (Some(sheet), Some(r)) => Some(format!("{}!{r}", quote_sheet(sheet))),
                            (None, Some(r)) => Some(r.to_string()),
                            _ => None,
                        }
                    };
                }
                _ => {}
            }
        }
    });
    if source.is_none() {
        let what = match kind.as_deref() {
            _ if external => {
                "the pivot source is in another workbook and is not resolved".to_string()
            }
            Some("worksheet") | None => "the pivot cache names no source range".to_string(),
            Some(k) => format!("a pivot cache of type {k} has no source range to read"),
        };
        ctx.issues
            .push(Issue::new(part, IssueKind::Unsupported, what));
    }
    source
}

/// Reads a pivot table definition.
pub(super) fn read_pivot(
    ctx: &mut Ctx<'_, '_>,
    part: &str,
    sources: &mut PivotSources,
) -> Option<Pivot> {
    let mut found: Option<Pivot> = None;
    let mut cache_id = String::new();
    ctx.scan(part, true, &mut |ev, _| {
        if let Ev::Start(e) | Ev::Empty(e) = ev {
            let number = |key: &str| e.attr(key).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
            // The grand totals are on unless the file says `0` or `false`.
            let on = |key: &str| !matches!(e.attr(key), Some("0" | "false"));
            match e.name.as_str() {
                "pivotTableDefinition" if found.is_none() => {
                    cache_id = e.attr("cacheId").unwrap_or("").to_string();
                    found = Some(Pivot {
                        name: e.attr("name").unwrap_or("").to_string(),
                        location: String::new(),
                        source: None,
                        first_header_row: 0,
                        first_data_row: 0,
                        first_data_col: 0,
                        row_grand_totals: on("rowGrandTotals"),
                        col_grand_totals: on("colGrandTotals"),
                        row_fields: 0,
                        col_fields: 0,
                    });
                }
                "location" => {
                    if let Some(p) = found.as_mut() {
                        p.location = e.attr("ref").unwrap_or("").to_string();
                        p.first_header_row = number("firstHeaderRow");
                        p.first_data_row = number("firstDataRow");
                        p.first_data_col = number("firstDataCol");
                    }
                }
                "rowFields" => {
                    if let Some(p) = found.as_mut() {
                        p.row_fields = number("count");
                    }
                }
                "colFields" => {
                    if let Some(p) = found.as_mut() {
                        p.col_fields = number("count");
                    }
                }
                _ => {}
            }
        }
    });
    let mut pivot = found?;
    pivot.source = sources.source(ctx, &cache_id);
    Some(pivot)
}

/// Reads the charts a drawing part points at.
pub(super) fn read_charts(ctx: &mut Ctx<'_, '_>, drawing: &str) -> Vec<Chart> {
    let rels = ctx.rels(drawing);
    let mut out = Vec::new();
    for rel in rels.iter().filter(|r| !r.external) {
        if rel.is("/chart") {
            read_chart(ctx, &rel.target, &mut out);
        } else if rel.is("/chartEx") {
            ctx.issues.push(Issue::new(
                &rel.target,
                IssueKind::Unsupported,
                "an extended chart (chartex) is not read",
            ));
        }
    }
    out
}

fn chart_kind(element: &str) -> Option<String> {
    let kind = element.strip_suffix("Chart")?;
    (!kind.is_empty()).then(|| kind.to_string())
}

fn read_chart(ctx: &mut Ctx<'_, '_>, part: &str, out: &mut Vec<Chart>) {
    let mut stack: Vec<String> = Vec::new();
    let mut rich = String::new();
    let mut cached = String::new();
    let mut title_done = false;
    // Charts of this part: one per plot element, so a combination chart gives several.
    let mut charts: Vec<Chart> = Vec::new();
    let mut series_name: Option<String> = None;
    let mut series_values: Option<String> = None;
    let mut text = String::new();
    let mut collecting = false;
    let mut skipped = 0usize;

    let mut handle = |ev: Ev<'_>| match ev {
        Ev::Start(e) | Ev::Empty(e) => {
            let name = e.name.as_str();
            let is_start = matches!(ev, Ev::Start(_));
            let in_plot = stack.iter().any(|n| n == "plotArea");
            if in_plot
                && stack.last().is_some_and(|n| n == "plotArea")
                && let Some(kind) = chart_kind(name)
            {
                charts.push(Chart {
                    kind,
                    title: None,
                    series: Vec::new(),
                });
            }
            if name == "ser" && is_start {
                series_name = None;
                series_values = None;
            }
            if is_start {
                collecting = matches!(name, "t" | "v" | "f");
                text.clear();
                stack.push(name.to_string());
            }
        }
        Ev::Text(t) => {
            if collecting {
                text.push_str(t);
            }
        }
        Ev::End(name) => {
            let depth = stack.len();
            let parent = depth.checked_sub(2).and_then(|i| stack.get(i)).cloned();
            let grand = depth.checked_sub(3).and_then(|i| stack.get(i)).cloned();
            let in_title = !title_done
                && stack.len() >= 3
                && stack[0] == "chartSpace"
                && stack[1] == "chart"
                && stack[2] == "title";
            if collecting && !text.is_empty() {
                if in_title {
                    match name {
                        "t" => rich.push_str(&text),
                        "v" => cached.push_str(&text),
                        _ => {}
                    }
                }
                let in_ser = stack.iter().any(|n| n == "ser");
                if in_ser && name == "f" {
                    let holder = grand.as_deref();
                    let ref_kind = parent.as_deref();
                    if matches!(ref_kind, Some("strRef" | "numRef")) {
                        match holder {
                            Some("tx") => series_name = Some(text.trim().to_string()),
                            Some("val" | "yVal") => series_values = Some(text.trim().to_string()),
                            _ => {}
                        }
                    }
                } else if in_ser && name == "v" && parent.as_deref() == Some("tx") {
                    series_name = Some(text.clone());
                }
            }
            collecting = false;
            text.clear();
            if name == "title" && in_title {
                title_done = true;
            }
            if name == "ser" {
                match (series_values.take(), charts.last_mut()) {
                    (Some(values), Some(chart)) => chart.series.push((series_name.take(), values)),
                    _ => skipped += 1,
                }
                series_name = None;
            }
            stack.pop();
        }
    };
    ctx.scan(part, true, &mut |ev, _| handle(ev));
    let title = if !rich.is_empty() {
        Some(rich)
    } else if !cached.is_empty() {
        Some(cached)
    } else {
        None
    };
    for mut c in charts {
        c.title = title.clone();
        out.push(c);
    }
    if skipped > 0 {
        ctx.issues.push(Issue::new(
            part,
            IssueKind::Unsupported,
            format!(
                "{skipped} chart series without a cell reference for their values are not listed"
            ),
        ));
    }
}
