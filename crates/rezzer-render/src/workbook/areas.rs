// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Regions and areas of formulas: which cells a formula fills and reads.

use super::*;

/// A sheet region: its cells and the author headings its table stands under.
pub(super) struct Region {
    pub(super) area: CellArea,
    pub(super) headings: Vec<String>,
}

/// The sheet regions among a sheet's blocks, in reading order.
pub(super) fn regions_of(blocks: &[Block]) -> Vec<Region> {
    table_headings(blocks)
        .into_iter()
        .filter_map(|(range, headings)| {
            let ((left, top), (right, bottom)) = corners(range)?;
            Some(Region {
                area: CellArea {
                    top,
                    left,
                    bottom,
                    right,
                },
                headings,
            })
        })
        .collect()
}

/// The full label of every labelled cell parameter, by its anchor in upper case: what a
/// read from another sheet and a defined name show.
pub(super) struct Labels {
    by_cell: HashMap<String, String>,
}

impl Labels {
    pub(super) fn of(params: &[Parameter]) -> Labels {
        let by_cell = params
            .iter()
            .filter(|p| !p.anchor.0.starts_with("name:"))
            .filter_map(|p| {
                let parts = parts_of(p);
                (!parts.is_empty()).then(|| (p.anchor.0.to_uppercase(), parts.join(" / ")))
            })
            .collect();
        Labels { by_cell }
    }

    pub(super) fn get(&self, anchor: &str) -> Option<&str> {
        self.by_cell.get(&anchor.to_uppercase()).map(String::as_str)
    }
}

/// The parts that name a parameter: its path, else its label as one part.
pub(super) fn parts_of(p: &Parameter) -> Vec<&str> {
    if p.path.is_empty() {
        p.label.as_deref().into_iter().collect()
    } else {
        p.path.iter().map(String::as_str).collect()
    }
}

pub(super) fn is_name(f: &Formula) -> bool {
    matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda))
        || f.anchor.0.starts_with("name:")
}

/// One row of the Formulas table: a formula of its own or a pattern group.
pub(super) struct Entry<'a> {
    pub(super) members: Vec<&'a Formula>,
    /// Only some members of a pattern (the results of a group with mixed roles): the
    /// group's own area does not apply.
    pub(super) partial: bool,
}

impl<'a> Entry<'a> {
    pub(super) fn first(&self) -> &'a Formula {
        self.members[0]
    }

    pub(super) fn has_role(&self, role: Role) -> bool {
        self.members.iter().any(|f| f.role == Some(role))
    }
}

/// The formulas in order of first appearance, members of one pattern folded into one entry.
pub(super) fn entries<'a>(list: &[&'a Formula]) -> Vec<Entry<'a>> {
    let mut out: Vec<Entry<'a>> = Vec::new();
    let mut by_pattern: HashMap<&str, usize> = HashMap::new();
    for &f in list {
        match f.pattern.as_deref() {
            Some(p) => match by_pattern.get(p) {
                Some(&i) => out[i].members.push(f),
                None => {
                    by_pattern.insert(p, out.len());
                    out.push(Entry {
                        members: vec![f],
                        partial: false,
                    });
                }
            },
            None => out.push(Entry {
                members: vec![f],
                partial: false,
            }),
        }
    }
    out
}

/// A cell or range part (`$B$2`, `B2:C9`) as 0-based (column, row) corners.
pub(super) fn corners(rest: &str) -> Option<((u32, u32), (u32, u32))> {
    let cell = |s: &str| -> Option<(u32, u32)> {
        let s = s.replace('$', "");
        let letters = s.chars().take_while(char::is_ascii_alphabetic).count();
        if letters == 0 || letters > 3 || letters == s.len() {
            return None;
        }
        let mut col: u32 = 0;
        for c in s[..letters].chars() {
            col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        }
        let row: u32 = s[letters..].parse().ok()?;
        (row >= 1).then(|| (col - 1, row - 1))
    };
    match rest.split_once(':') {
        Some((a, b)) => Some((cell(a)?, cell(b)?)),
        None => cell(rest).map(|c| (c, c)),
    }
}

/// The area a row covers. A formula of its own: the area it fills, else its cell. A pattern:
/// the area all members share, else the group's node in the dependency graph, else the box
/// around its members.
fn area(entry: &Entry<'_>, groups: &HashMap<&str, &str>) -> Vec<String> {
    let found = group_area(entry, groups);
    let n = entry.members.len();
    if n > 1 && n <= MAX_LISTED && cell_count(&found) != Some(n as u64) {
        // A small group that does not fill its area: its cells, not the area.
        return entry.members.iter().map(|f| f.anchor.0.clone()).collect();
    }
    vec![found]
}

/// The Cells text of a row, written for the part of `sheet`. A larger group of single cells
/// that does not fill its area says how many of its cells it holds.
pub(super) fn area_text(
    entry: &Entry<'_>,
    groups: &HashMap<&str, &str>,
    sheet: Option<&str>,
) -> String {
    let list = area(entry, groups);
    let mut out = list
        .iter()
        .map(|a| cell_text(local(a, sheet)))
        .collect::<Vec<_>>()
        .join(", ");
    let n = entry.members.len() as u64;
    if let [one] = list.as_slice()
        && n > 1
        && entry.members.iter().all(|f| fills_one_cell(f))
        && let Some(m) = cell_count(one).filter(|&m| m > n)
    {
        out.push_str(&format!(" ({n} of {m} cells)"));
    }
    out
}

/// An array, spill or data table formula fills the area it names; any other formula its cell.
fn fills_area(f: &Formula) -> bool {
    matches!(
        f.kind,
        Some(FormulaKind::Array | FormulaKind::Spill | FormulaKind::DataTable)
    ) && f.range.as_ref().is_some_and(|r| r.0 != f.anchor.0)
}

fn fills_one_cell(f: &Formula) -> bool {
    !fills_area(f)
}

/// A group with up to this many members that does not fill its area lists its cells.
const MAX_LISTED: usize = 5;

/// The number of cells of a `Sheet!A1:B9` area.
fn cell_count(anchor: &str) -> Option<u64> {
    let ((c0, r0), (c1, r1)) = corners(split_sheet(anchor)?.1)?;
    Some((u64::from(c1.abs_diff(c0)) + 1) * (u64::from(r1.abs_diff(r0)) + 1))
}

fn group_area(entry: &Entry<'_>, groups: &HashMap<&str, &str>) -> String {
    let first = entry.first();
    if entry.members.len() == 1 {
        // A pattern member's range is the group's area; only a filling formula owns one.
        if entry.partial && !fills_area(first) {
            return first.anchor.0.clone();
        }
        return first
            .range
            .as_ref()
            .map_or_else(|| first.anchor.0.clone(), |r| r.0.clone());
    }
    if entry.partial {
        return bounding_box(entry);
    }
    if let Some(range) = &first.range
        && range.0 != first.anchor.0
        && entry
            .members
            .iter()
            .all(|f| f.range.as_ref() == Some(range))
    {
        return range.0.clone();
    }
    if let Some(node) = first.pattern.as_deref().and_then(|p| groups.get(p)) {
        return (*node).to_string();
    }
    bounding_box(entry)
}

/// The box around the members' cells, or the first cell when they do not share a sheet.
fn bounding_box(entry: &Entry<'_>) -> String {
    let first = entry.first();
    let Some((sheet, _)) = split_sheet(&first.anchor.0) else {
        return first.anchor.0.clone();
    };
    let mut bounds: Option<((u32, u32), (u32, u32))> = None;
    for f in &entry.members {
        let Some((s, rest)) = split_sheet(&f.anchor.0) else {
            return first.anchor.0.clone();
        };
        let Some((a, b)) = corners(rest).filter(|_| s == sheet) else {
            return first.anchor.0.clone();
        };
        bounds = Some(match bounds {
            None => (a, b),
            Some((lo, hi)) => (
                (lo.0.min(a.0), lo.1.min(a.1)),
                (hi.0.max(b.0), hi.1.max(b.1)),
            ),
        });
    }
    let Some(((c0, r0), (c1, r1))) = bounds else {
        return first.anchor.0.clone();
    };
    format!(
        "{sheet}!{}{}:{}{}",
        column_letter(c0 as usize),
        r0 + 1,
        column_letter(c1 as usize),
        r1 + 1
    )
}
