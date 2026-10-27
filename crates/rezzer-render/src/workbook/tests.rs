// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the workbook writer.

use super::*;

#[test]
fn split_sheet_handles_quotes() {
    assert_eq!(split_sheet("Calc!A1"), Some(("Calc", "A1")));
    assert_eq!(
        split_sheet("'It''s !x'!A1:B2"),
        Some(("'It''s !x'", "A1:B2"))
    );
    assert_eq!(split_sheet("'open"), None);
    assert_eq!(split_sheet("name:Calc!x"), None);
    assert_eq!(split_sheet("ext:[1]Calc!A1"), None);
    assert_eq!(split_sheet("A1"), None);
    assert_eq!(unquote("'It''s'"), "It's");
}

#[test]
fn corners_read_cells_and_ranges() {
    assert_eq!(corners("$B$2"), Some(((1, 1), (1, 1))));
    assert_eq!(corners("AA10:AB12"), Some(((26, 9), (27, 11))));
    assert_eq!(corners("B:B"), None);
    assert_eq!(corners("B0"), None);
}

#[test]
fn called_functions_skip_strings_and_sheet_names() {
    let mut found = Vec::new();
    called_functions("=NOW()+\"RAND()\"+'Sheet (1)'!A1+[1]!f(SUM(A1))", |n| {
        found.push(n.to_string())
    });
    assert_eq!(found, vec!["NOW", "[1]!f", "SUM"]);
}
