// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Detection of columns that are the sum of other columns.

use crate::types::{Derived, DerivedOp};

const MAX_NUMERIC: usize = 12;
const MIN_PARTS: usize = 2;
const MAX_PARTS: usize = 5;
const MIN_CHECKED: usize = 10;

/// A numeric column prepared for the check.
pub(crate) struct NumCol {
    /// Column index in the table.
    pub index: usize,
    /// Largest number of decimal places in the column.
    pub places: u8,
    /// One value per data row; `None` for empty or missing cells.
    pub values: Vec<Option<f64>>,
}

/// Finds, per target column, the subset of other numeric columns whose sum
/// matches it on the most rows. Only the first 12 numeric columns are used.
/// Tolerance is half a unit of the target's last decimal plus half a unit of
/// the finest place per term (rounding error of the source values).
/// Rows with a missing value, or a target of zero, are skipped.
pub(crate) fn find(cols: &[NumCol], rows: usize) -> Vec<Derived> {
    let cols = &cols[..cols.len().min(MAX_NUMERIC)];
    let mut out = Vec::new();
    if rows < MIN_CHECKED {
        return out;
    }
    for (t, target) in cols.iter().enumerate() {
        let others: Vec<usize> = (0..cols.len()).filter(|&i| i != t).collect();
        let mut best: Option<Derived> = None;
        for size in MIN_PARTS..=MAX_PARTS.min(others.len()) {
            for_each_subset(&others, size, &mut |subset| {
                let (holds, checked) = score(target, cols, subset, rows);
                let better = best.as_ref().is_none_or(|b| holds > b.holds);
                if checked >= MIN_CHECKED && holds * 2 >= checked && better {
                    best = Some(Derived {
                        target: target.index,
                        parts: subset.iter().map(|&i| cols[i].index).collect(),
                        op: DerivedOp::Sum,
                        holds,
                        checked,
                    });
                }
            });
        }
        out.extend(best);
    }
    out
}

fn score(target: &NumCol, cols: &[NumCol], subset: &[usize], rows: usize) -> (usize, usize) {
    let max_places = subset
        .iter()
        .map(|&i| cols[i].places)
        .max()
        .unwrap_or(0)
        .max(target.places);
    // Half a unit of the target's last place, plus the rounding of every term.
    // The 1e-9 absorbs binary floating point noise.
    let tol = 0.5 * 10f64.powi(-i32::from(target.places))
        + (subset.len() + 1) as f64 * 0.5 * 10f64.powi(-i32::from(max_places))
        + 1e-9;
    let (mut holds, mut checked) = (0, 0);
    'rows: for r in 0..rows {
        let Some(t) = target.values[r] else { continue };
        if t == 0.0 {
            continue;
        }
        let mut sum = 0.0;
        for &i in subset {
            match cols[i].values[r] {
                Some(v) => sum += v,
                None => continue 'rows,
            }
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
