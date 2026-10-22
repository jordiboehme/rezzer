// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Facts first: Excel tables, database ranges, pivots and autofilters set exact regions, and
//! defined names join rule regions.

use crate::input::{FactKind, Input, Rect};
use crate::occupancy::Prep;
use crate::{Certainty, Evidence, Issue, IssueKind, Kind, Origin, Region, Work};

/// Header rows a fact may give (rules give at most 4).
const FACT_BAND_MAX: u32 = 10;

/// A fact that sets a region, with the lower facts on its range as evidence.
pub(crate) struct Kept {
    pub rect: Rect,
    pub fact: usize,
    pub evidence: Vec<Evidence>,
}

/// The order of spec 6.1: the lower level wins.
fn level(kind: &FactKind) -> u8 {
    match kind {
        FactKind::ListObject { .. } => 1,
        FactKind::DatabaseRange { .. } => 2,
        FactKind::Pivot { .. } => 3,
        FactKind::AutoFilter => 4,
    }
}

fn evidence_of(kind: &FactKind) -> Evidence {
    match kind {
        FactKind::ListObject { name, .. } => Evidence::ListObject { name: name.clone() },
        FactKind::DatabaseRange { name, .. } => Evidence::DatabaseRange { name: name.clone() },
        FactKind::Pivot { name, .. } => Evidence::Pivot { name: name.clone() },
        FactKind::AutoFilter => Evidence::AutoFilter,
    }
}

fn label(kind: &FactKind) -> String {
    match kind {
        FactKind::ListObject { name, .. } => format!("Excel table {name}"),
        FactKind::DatabaseRange { name, .. } => format!("database range {name}"),
        FactKind::Pivot { name, .. } => format!("pivot table {name}"),
        FactKind::AutoFilter => "autofilter".to_string(),
    }
}

/// Picks the facts that set regions. A fact on the range of a higher one becomes its
/// evidence; facts of one level that overlap both stay, with an `Overlap` issue. At most
/// `cap` facts are kept.
pub(crate) fn resolve(input: &Input, cap: usize, issues: &mut Vec<Issue>) -> Vec<Kept> {
    let mut order: Vec<usize> = (0..input.facts.len()).collect();
    order.sort_by_key(|&i| level(&input.facts[i].kind));
    let mut kept: Vec<Kept> = Vec::new();
    let mut dropped = 0usize;
    for i in order {
        let fact = &input.facts[i];
        let rect = fact.rect.normalized();
        let lv = level(&fact.kind);
        if let Some(k) = kept
            .iter_mut()
            .find(|k| level(&input.facts[k.fact].kind) < lv && k.rect.intersects(&rect))
        {
            let e = evidence_of(&fact.kind);
            if !k.evidence.contains(&e) {
                k.evidence.push(e);
            }
            continue;
        }
        if kept
            .iter()
            .any(|k| k.rect == rect && input.facts[k.fact].kind == fact.kind)
        {
            continue;
        }
        if kept.len() >= cap {
            dropped += 1;
            continue;
        }
        for k in &kept {
            if let Some(common) = k.rect.intersection(&rect) {
                issues.push(Issue {
                    what: format!(
                        "{} and {} overlap in {}; both are kept",
                        label(&input.facts[k.fact].kind),
                        label(&fact.kind),
                        common.a1()
                    ),
                    kind: IssueKind::Overlap,
                    rect: Some(common),
                });
            }
        }
        kept.push(Kept {
            rect,
            fact: i,
            evidence: vec![evidence_of(&fact.kind)],
        });
    }
    if dropped > 0 {
        issues.push(Issue {
            what: format!(
                "{dropped} table, filter or pivot ranges past the region limit are not used"
            ),
            kind: IssueKind::Limit,
            rect: None,
        });
    }
    kept
}

/// What the facts hold.
pub(crate) struct Held {
    /// Per kept fact that was processed, the merges anchored in it.
    pub merges: Vec<Vec<u32>>,
    /// Per item, whether a fact holds it (or a merge reaches into a fact).
    pub taken: Vec<bool>,
    pub stopped: bool,
}

/// Gives each fact the cells and merges anchored in it. A merge that reaches into a fact is
/// not free either: its anchor cell stays free and is cut like any cell.
pub(crate) fn assign(prep: &Prep, kept: &[Kept], stop: &dyn Fn() -> bool) -> Held {
    let ncells = prep.cells.len();
    let mut taken = vec![false; prep.items()];
    let mut merges = Vec::new();
    for k in kept {
        if stop() {
            return Held {
                merges,
                taken,
                stopped: true,
            };
        }
        for c in prep.rows(k.rect.top, k.rect.bottom) {
            if k.rect.contains(prep.cell(c).pos) {
                taken[c] = true;
            }
        }
        merges.push(Vec::new());
    }
    if kept.is_empty() {
        return Held {
            merges,
            taken,
            stopped: false,
        };
    }
    for (j, m) in prep.merges.iter().enumerate() {
        if j % 4096 == 4095 && stop() {
            return Held {
                merges,
                taken,
                stopped: true,
            };
        }
        let anchor = crate::input::Pos {
            row: m.top,
            col: m.left,
        };
        let mut owner = false;
        for (f, k) in kept.iter().enumerate() {
            if k.rect.intersects(m) {
                taken[ncells + j] = true;
                if !owner && k.rect.contains(anchor) {
                    merges[f].push(j as u32);
                    owner = true;
                }
            }
        }
    }
    Held {
        merges,
        taken,
        stopped: false,
    }
}

/// The region of a fact, with its header rows and data area.
pub(crate) fn region(input: &Input, kept: &Kept) -> Region {
    let r = kept.rect;
    let height = r.height().min(u64::from(u32::MAX)) as u32;
    let width = r.width().min(u64::from(u32::MAX)) as u32;
    let mut label_cols = 0u32;
    let mut hidden_header = false;
    let band = match &input.facts[kept.fact].kind {
        FactKind::ListObject { header_rows, .. } => {
            hidden_header = *header_rows == 0;
            *header_rows
        }
        FactKind::DatabaseRange { header, .. } => u32::from(*header),
        FactKind::Pivot {
            first_data_row,
            first_data_col,
            ..
        } => {
            label_cols = (*first_data_col).min(width);
            *first_data_row
        }
        FactKind::AutoFilter => 1,
    };
    let band = band.min(FACT_BAND_MAX).min(height);
    let header_rows: Vec<u32> = (0..band).map(|k| r.top + k).collect();
    let data = (band < height).then(|| Rect {
        top: r.top + band,
        ..r
    });
    Region {
        kind: Kind::Table,
        rect: r,
        data,
        captions: Vec::new(),
        caption_heading: None,
        notes: Vec::new(),
        heading: None,
        header_rows,
        header_paths: Vec::new(),
        label_cols,
        certainty: Certainty::Exact,
        evidence: kept.evidence.clone(),
        unsure: Vec::new(),
        rows: Vec::new(),
        hidden_header,
    }
}

/// Spec 6.1 item 7: a name over two or more rule regions, inside their combined bounds,
/// joins them. A name never cuts. A name whose rectangle is a region's is its evidence.
pub(crate) fn join_names(input: &Input, works: &mut [Work], cap: usize, issues: &mut Vec<Issue>) {
    let mut used = 0usize;
    let mut skipped = 0usize;
    for name in &input.names {
        let rect = name.rect.normalized();
        if name.builtin || rect.height() < 2 || rect.width() < 2 {
            continue;
        }
        if used >= cap {
            skipped += 1;
            continue;
        }
        used += 1;
        let hits: Vec<usize> = works
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.dead && w.origin == Origin::Rule && w.region.rect.intersects(&rect))
            .map(|(i, _)| i)
            .collect();
        let evidence = Evidence::DefinedName {
            name: name.name.clone(),
        };
        if hits.len() == 1 {
            let w = &mut works[hits[0]];
            if w.region.rect == rect && !w.region.evidence.contains(&evidence) {
                w.region.evidence.push(evidence);
            }
            continue;
        }
        if hits.len() < 2 {
            continue;
        }
        let joined = hits
            .iter()
            .map(|&i| works[i].region.rect)
            .reduce(|a, b| a.union(&b))
            .unwrap_or(rect);
        if !joined.covers(&rect) {
            continue;
        }
        let blocked = works
            .iter()
            .enumerate()
            .any(|(i, w)| !w.dead && !hits.contains(&i) && w.region.rect.intersects(&joined));
        if blocked {
            continue;
        }
        let first = hits[0];
        for &i in &hits[1..] {
            let merges = std::mem::take(&mut works[i].merges);
            works[i].dead = true;
            works[first].merges.extend(merges);
        }
        let w = &mut works[first];
        w.region.rect = joined;
        w.region.data = Some(joined);
        if !w.region.evidence.contains(&evidence) {
            w.region.evidence.push(evidence);
        }
    }
    if skipped > 0 {
        issues.push(Issue {
            what: format!("{skipped} defined names past the region limit are not used"),
            kind: IssueKind::Limit,
            rect: None,
        });
    }
}
