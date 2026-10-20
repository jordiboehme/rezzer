// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `rezzer formats`: a table of format, reader and reader version.

use std::process::ExitCode;

use rezzer_model::Registry;

use crate::print;

pub(crate) fn run(registry: &Registry) -> Result<ExitCode, String> {
    let rows: Vec<[String; 3]> = registry
        .list()
        .into_iter()
        .map(|(format, id, version)| {
            [
                rezzer_render::format_name(format),
                id.to_string(),
                version.to_string(),
            ]
        })
        .collect();
    if rows.is_empty() {
        print("This build has no readers.\n")?;
        return Ok(ExitCode::SUCCESS);
    }
    let header = ["format".to_string(), "reader".into(), "version".into()];
    let width = |i: usize| {
        rows.iter()
            .chain(std::iter::once(&header))
            .map(|r| r[i].len())
            .max()
            .unwrap_or(0)
    };
    let (w0, w1) = (width(0), width(1));
    let mut text = String::new();
    for row in std::iter::once(&header).chain(rows.iter()) {
        text.push_str(&format!("{:<w0$}  {:<w1$}  {}\n", row[0], row[1], row[2]));
    }
    print(&text)?;
    Ok(ExitCode::SUCCESS)
}
