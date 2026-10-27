// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Detection of columns that are the sum of other columns.

use crate::types::{Derived, DerivedOp};

/// Most numeric columns the derived check looks at.
pub(crate) const MAX_NUMERIC_COLUMNS: usize = 12;
const MIN_PARTS: usize = 2;
const MAX_PARTS: usize = 5;
const MIN_CHECKED: usize = 10;
const MIN_DISTINCT_TARGET: usize = 10;
const MIN_RANGE_IN_TOLERANCES: f64 = 100.0;
const AMBIGUITY_MARGIN: f64 = 0.05;

/// A numeric column prepared for the check.
pub(crate) struct NumCol {
    /// Column index in the table.
    pub index: usize,
    /// Largest number of decimal places in the column.
    pub places: u8,
    /// One value per checked row; `None` for empty or missing cells.
    pub values: Vec<Option<f64>>,
}

struct Summary {
    mean: f64,
    min: f64,
    max: f64,
    distinct: usize,
}

fn summarize(c: &NumCol) -> Summary {
    let present: Vec<f64> = c.values.iter().flatten().copied().collect();
    let n = present.len().max(1) as f64;
    let distinct: std::collections::HashSet<u64> = present.iter().map(|v| v.to_bits()).collect();
    Summary {
        mean: present.iter().sum::<f64>() / n,
        min: present.iter().copied().fold(f64::INFINITY, f64::min),
        max: present.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        distinct: distinct.len(),
    }
}

struct Candidate {
    ratio: f64,
    holds: usize,
    checked: usize,
    subset: Vec<usize>,
}

/// Finds, per target column, the subset of other numeric columns whose sum
/// matches it best. Only the first 12 numeric columns are used.
///
/// Tolerance is half a unit of the target's last decimal plus half a unit of
/// the finest place per term (rounding error of the source values). A missing
/// part counts as 0 when the target and at least one part are present; rows
/// with a missing target, no part at all, or a target of zero are skipped.
///
/// Guards against chance matches on small-range data: a target needs at least
/// 10 distinct values and a value range of at least 100 tolerances, and every
/// part must have a smaller mean than the target. Subsets are ranked by
/// holds/checked, then holds; when a second subset comes within 0.05 of the
/// best ratio (and is not a subset or superset of the best) the result is
/// ambiguous and nothing is reported for that target. At most the shortest column's number of
/// values is checked.
pub(crate) fn find(cols: &[NumCol], rows: usize) -> Vec<Derived> {
    let cols = &cols[..cols.len().min(MAX_NUMERIC_COLUMNS)];
    // A column with fewer values (a read that ended early) bounds the rows checked.
    let rows = cols.iter().map(|c| c.values.len()).fold(rows, usize::min);
    let mut out = Vec::new();
    if rows < MIN_CHECKED {
        return out;
    }
    let sums: Vec<Summary> = cols.iter().map(summarize).collect();
    for (t, target) in cols.iter().enumerate() {
        if sums[t].distinct < MIN_DISTINCT_TARGET {
            continue;
        }
        let others: Vec<usize> = (0..cols.len())
            .filter(|&i| i != t && sums[i].mean < sums[t].mean)
            .collect();
        let mut found: Vec<Candidate> = Vec::new();
        for size in MIN_PARTS..=MAX_PARTS.min(others.len()) {
            for_each_subset(&others, size, &mut |subset| {
                let tol = tolerance(target, cols, subset);
                if sums[t].max - sums[t].min < MIN_RANGE_IN_TOLERANCES * tol {
                    return;
                }
                let (holds, checked) = score(target, cols, subset, rows, tol);
                if checked >= MIN_CHECKED && holds * 2 >= checked {
                    found.push(Candidate {
                        ratio: holds as f64 / checked as f64,
                        holds,
                        checked,
                        subset: subset.to_vec(),
                    });
                }
            });
        }
        // First candidate wins ties, so smaller subsets are preferred.
        let mut best: Option<usize> = None;
        for (i, c) in found.iter().enumerate() {
            let better = best.is_none_or(|b| (c.ratio, c.holds) > (found[b].ratio, found[b].holds));
            if better {
                best = Some(i);
            }
        }
        let Some(b) = best else { continue };
        let top = &found[b];
        // A subset or superset of the best parts is the same formula with
        // columns added or dropped, not a rival.
        let related = |c: &Candidate| {
            c.subset.iter().all(|x| top.subset.contains(x))
                || top.subset.iter().all(|x| c.subset.contains(x))
        };
        let ambiguous = found
            .iter()
            .enumerate()
            .any(|(i, c)| i != b && !related(c) && c.ratio >= top.ratio - AMBIGUITY_MARGIN);
        if !ambiguous {
            out.push(Derived {
                target: target.index,
                parts: top.subset.iter().map(|&i| cols[i].index).collect(),
                op: DerivedOp::Sum,
                holds: top.holds,
                checked: top.checked,
            });
        }
    }
    out
}

fn tolerance(target: &NumCol, cols: &[NumCol], subset: &[usize]) -> f64 {
    let max_places = subset
        .iter()
        .map(|&i| cols[i].places)
        .max()
        .unwrap_or(0)
        .max(target.places);
    // Half a unit of the target's last place, plus the rounding of every term.
    // The 1e-9 absorbs binary floating point noise.
    0.5 * 10f64.powi(-i32::from(target.places))
        + (subset.len() + 1) as f64 * 0.5 * 10f64.powi(-i32::from(max_places))
        + 1e-9
}

fn score(
    target: &NumCol,
    cols: &[NumCol],
    subset: &[usize],
    rows: usize,
    tol: f64,
) -> (usize, usize) {
    let (mut holds, mut checked) = (0, 0);
    for r in 0..rows {
        let Some(t) = target.values[r] else { continue };
        if t == 0.0 {
            continue;
        }
        let (mut sum, mut any) = (0.0, false);
        for &i in subset {
            if let Some(v) = cols[i].values[r] {
                sum += v;
                any = true;
            }
        }
        if !any {
            continue;
        }
        checked += 1;
        if (t - sum).abs() <= tol {
            holds += 1;
        }
    }
    (holds, checked)
}

/// Calls `f` with every ascending subset of `items` of the given size.
fn for_each_subset(items: &[usize], size: usize, f: &mut impl FnMut(&[usize])) {
    let mut idx: Vec<usize> = (0..size).collect();
    let n = items.len();
    let mut buf = vec![0; size];
    loop {
        for (b, &i) in buf.iter_mut().zip(&idx) {
            *b = items[i];
        }
        f(&buf);
        let mut k = size;
        loop {
            if k == 0 {
                return;
            }
            k -= 1;
            if idx[k] != k + n - size {
                break;
            }
            if k == 0 {
                return;
            }
        }
        idx[k] += 1;
        for j in k + 1..size {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_columns_bound_the_rows() {
        // Columns of 14 values that sum, checked for 20 rows: only 14 are checked.
        let col = |index: usize, f: fn(f64) -> f64| NumCol {
            index,
            places: 0,
            values: (1..=14).map(|j| Some(f(f64::from(j)))).collect(),
        };
        let cols = [
            col(0, |j| (j * 371.0) % 900.0 + 100.0),
            col(1, |j| (j * 533.0) % 700.0 + 200.0),
            col(2, |j| {
                (j * 371.0) % 900.0 + 100.0 + (j * 533.0) % 700.0 + 200.0
            }),
        ];
        let found = find(&cols, 20);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].target, found[0].checked), (2, 14));
        // One column of 5 values leaves too few rows to check.
        let mut cols = cols;
        cols[1].values.truncate(5);
        assert!(find(&cols, 20).is_empty());
    }
}
