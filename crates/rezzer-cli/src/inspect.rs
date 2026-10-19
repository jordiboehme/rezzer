// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `rezzer inspect <PATH>`.

use std::path::Path;
use std::process::ExitCode;

use rezzer_model::{Confidence, Registry};
use rezzer_pipeline::{Config, Pipeline};

use crate::print;

pub(crate) fn run(config: Config, registry: Registry, path: &Path) -> Result<ExitCode, String> {
    let pipeline = Pipeline::new(config, registry);
    let info = pipeline.inspect(path).map_err(|e| e.to_string())?;
    let confidence = match info.detection.confidence {
        Confidence::Certain => "certain",
        Confidence::Likely => "likely",
        Confidence::Guess => "guess",
    };
    let text = format!(
        "name: {}\nformat: {}\nconfidence: {confidence}\nevidence: {}\nsize: {} bytes\nsha256: {}\nreader: {}\n",
        one_line(&info.name),
        rezzer_render::format_name(info.format),
        one_line(&info.detection.evidence),
        info.size,
        info.sha256,
        info.reader
            .as_deref()
            .unwrap_or("none (this build cannot read the format)"),
    );
    print(&text)?;
    Ok(ExitCode::SUCCESS)
}

/// Control characters from file names or evidence never reach the terminal.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
