// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The overview through the whole pipeline: sheet sizes come from the reader.
#![cfg(feature = "fmt-csv")]

use rezzer_model::Registry;
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request};

#[test]
fn a_cut_sheet_states_all_rows_and_the_rows_shown() {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let pipeline = Pipeline::new(
        Config {
            cache: false,
            ..Config::default()
        },
        registry,
    );
    let mut csv = String::from("n,v\n");
    for i in 0..10_005 {
        csv.push_str(&format!("{i},x{i}\n"));
    }
    let result = pipeline.run_bytes(
        "big.csv",
        csv.into_bytes(),
        None,
        &Request {
            output: OutputKind::Json,
            generated_at: Some("2026-01-01T00:00:00Z".to_string()),
            ..Request::default()
        },
    );
    let doc = result.document.expect("document");
    let overview = doc.overview.expect("overview");
    assert_eq!(
        overview.parts[0].size,
        "10005 rows x 2 columns (10000 shown)"
    );
}
