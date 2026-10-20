// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! formulakit on arbitrary formula text: analysis in both dialects, LaTeX and the workbook
//! logic over all lines of the input. It must never panic.
#![no_main]

use formulakit::logic::Workbook;
use formulakit::{CellPos, Dialect, analyze, latex, worth_math};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut book = Workbook::new();
    // Each line is the formula of one cell in column A; the first byte of a line picks the
    // dialect, so both parsers see every kind of text.
    for (row, line) in text.lines().take(256).enumerate() {
        let dialect = if line.as_bytes().first().is_some_and(|b| b % 2 == 1) {
            Dialect::OpenFormula
        } else {
            Dialect::Excel
        };
        let at = CellPos {
            row: row as u32,
            col: 0,
        };
        let Ok(analysis) = analyze(line, dialect, at) else {
            book.add_constant("S", at);
            continue;
        };
        if worth_math(&analysis) {
            let _ = latex(&analysis.display, Dialect::Excel);
        }
        let _ = latex(line, dialect);
        book.add_formula("S", at, &analysis);
        if row % 7 == 0 {
            book.add_name(&format!("n{row}"), None, Some(&analysis));
        }
    }
    let _ = book.finish(&|| false);
});
