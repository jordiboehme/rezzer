// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The workbook logic pass and its inputs: the sheet facts, the read state, the stale-results warning.

use super::*;

/// The cells of one sheet the logic pass looks at for labels, values and number formats.
pub(crate) struct SheetFacts {
    pub name: String,
    /// Sorted by row, then column, each position once (formula texts dropped).
    pub cells: Vec<RawCell>,
    /// The format key of each cell that has one (an OOXML style index, or an index into the
    /// reader's own format list); [`ReadState::formats`] gives its code.
    pub styles: HashMap<(u32, u32), u32>,
    /// What names the cells of the sheet's regions.
    pub paths: SheetPaths,
}

impl SheetFacts {
    pub fn new(
        name: &str,
        mut cells: Vec<RawCell>,
        styles: HashMap<(u32, u32), u32>,
        paths: SheetPaths,
    ) -> SheetFacts {
        // Only whether a cell holds a formula matters here; the texts are in the facet.
        for c in &mut cells {
            if c.formula.is_some() {
                c.formula = Some(String::new());
            }
        }
        SheetFacts {
            name: name.to_string(),
            cells,
            styles,
            paths,
        }
    }
}

/// How the workbook was read, for the logic pass.
pub(crate) struct ReadState<'a> {
    /// The calculation settings, when the file gives them.
    pub calc: Option<&'a CalcSettings>,
    /// The number format code of a format key in [`SheetFacts::styles`].
    pub formats: &'a dyn Fn(u32) -> Option<&'a str>,
    /// Some sheet or part of one was not read, or some formula cells were not listed.
    pub incomplete: bool,
}

/// The workbook logic: roles, pattern groups, parameters, the dependency graph and the
/// stored-results warning. Runs after every sheet and name has been read.
pub(crate) fn workbook_logic(
    out: &mut Out,
    formulas: Formulas,
    sheets: Vec<SheetFacts>,
    state: ReadState,
    budget: &Budget,
) {
    stale_results(out, state.calc);
    let Formulas {
        inputs,
        convert_units,
        named_cells,
        literal_names,
        unindexed_formulas,
        unindexed_names,
        tables,
        ..
    } = formulas;
    let LogicInputs {
        workbook,
        formula_ids,
    } = inputs;
    let stop = || budget.check_time().is_err();
    let logic = match contain(AssertUnwindSafe(|| workbook.finish(&stop))) {
        Ok(l) => l,
        Err(panic) => {
            out.loss(
                "document",
                format!("roles, patterns, parameters and the dependency graph not worked out: internal error: {panic}"),
                LossReason::Internal,
                Severity::Warning,
            );
            return;
        }
    };
    let mut facet_of: Vec<Option<usize>> = Vec::new();
    for (index, id) in &formula_ids {
        let i = id.0 as usize;
        if facet_of.len() <= i {
            facet_of.resize(i + 1, None);
        }
        facet_of[i] = Some(*index);
    }
    let facet = |id: FormulaId| facet_of.get(id.0 as usize).copied().flatten();

    // A formula the index does not know may read any cell: no cell is surely a result.
    let mut gaps = Vec::new();
    match unindexed_formulas {
        0 => {}
        1 => gaps.push("1 formula could not be analyzed".to_string()),
        n => gaps.push(format!("{n} formulas could not be analyzed")),
    }
    match unindexed_names {
        0 => {}
        1 => gaps.push("1 defined name could not be analyzed".to_string()),
        n => gaps.push(format!("{n} defined names could not be analyzed")),
    }
    if state.incomplete {
        gaps.push("not every formula cell was read".to_string());
    }
    let certain = gaps.is_empty();
    if !certain {
        out.loss(
            "document",
            format!(
                "the workbook logic is incomplete: {}; no formula is marked as a result and read counts may be low",
                gaps.join("; ")
            ),
            LossReason::Uncertain,
            Severity::Warning,
        );
    }

    let formulas_facet = &mut out.doc.facets.formulas;
    for (id, role) in &logic.roles {
        let role = match role {
            KitRole::Constant => Role::Constant,
            KitRole::Intermediate => Role::Intermediate,
            KitRole::Result if certain => Role::Result,
            KitRole::Result => continue,
        };
        if let Some(f) = facet(*id).and_then(|i| formulas_facet.get_mut(i)) {
            f.role = Some(role);
        }
    }
    for g in &logic.groups {
        if g.members.len() < 2 {
            continue;
        }
        let range = Anchor(format!("{}!{}", quote_sheet(&g.sheet), g.area));
        for m in &g.members {
            if let Some(f) = facet(*m).and_then(|i| formulas_facet.get_mut(i)) {
                f.pattern = Some(g.id.clone());
                // An array, spill or data table keeps the area it fills.
                if f.range.is_none() {
                    f.range = Some(range.clone());
                }
            }
        }
    }
    if logic.stopped {
        let kept = if logic.parameters.is_empty() {
            "patterns and constant formulas are kept; other roles, parameters and the dependency graph are not set"
        } else {
            "roles, patterns and parameters are kept; the dependency graph is not set"
        };
        out.loss(
            "document",
            format!("time limit reached while the workbook logic was worked out: {kept}"),
            LossReason::Limit,
            Severity::Warning,
        );
    }

    let mut path_text = TextMeter::new(budget.limits().max_text_bytes);
    let (mut params, unit_notes) = cell_parameters(
        &logic.parameters,
        &sheets,
        state.formats,
        &convert_units,
        &named_cells,
        &mut path_text,
    );
    if path_text.is_full() {
        out.loss(
            "document",
            format!(
                "text limit of {} bytes reached: later parameters keep their one label, not their path of headings",
                path_text.max()
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    for (anchor, what) in unit_notes {
        out.loss(anchor, what, LossReason::Uncertain, Severity::Info);
    }
    params.extend(name_parameters(&out.doc.facets.formulas, literal_names));
    let total = params.len();
    for (i, p) in params.into_iter().enumerate() {
        if let Err(hit) = budget.add_elements(1) {
            out.loss(
                "document",
                format!("{hit}: {} of {total} parameters not listed", total - i),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.parameters.push(p);
    }

    if !logic.stopped
        && !logic.graph.nodes.is_empty()
        && let Some(g) = dependency_graph(out, &logic.graph, &tables, budget)
    {
        out.doc.facets.graphs.push(g);
    }
}

/// One `Uncertain` warning when the stored results may not match the formulas: manual
/// calculation, data tables left out of automatic calculation, iterative calculation, a file
/// that asks for a full recalculation, or formulas without a stored result.
fn stale_results(out: &mut Out, calc: Option<&CalcSettings>) {
    let formulas = &out.doc.facets.formulas;
    let mut why = Vec::new();
    let default = CalcSettings::default();
    let calc = calc.unwrap_or(&default);
    if calc.mode == CalcMode::Manual {
        why.push("calculation is set to manual".to_string());
    }
    if calc.mode == CalcMode::AutoNoTable
        && formulas
            .iter()
            .any(|f| f.kind == Some(FormulaKind::DataTable))
    {
        why.push("data tables are not recalculated automatically".to_string());
    }
    if calc.iterate {
        why.push("iterative calculation is on".to_string());
    }
    if calc.full_calc_on_load {
        why.push("the file asks for a full recalculation when opened".to_string());
    }
    let missing = formulas
        .iter()
        .filter(|f| {
            f.cached.is_none() && !matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda))
        })
        .count();
    match missing {
        0 => {}
        1 => why.push("1 formula has no stored result".to_string()),
        n => why.push(format!("{n} formulas have no stored result")),
    }
    if !why.is_empty() {
        out.loss(
            "document",
            format!("{STALE_RESULTS}: {}", why.join("; ")),
            LossReason::Uncertain,
            Severity::Warning,
        );
    }
}
