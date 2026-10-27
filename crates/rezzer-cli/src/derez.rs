// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `rezzer derez [<PATH>]`.

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rezzer_model::{Completeness, Registry};
use rezzer_pipeline::{Config, Pipeline, Request};

use crate::DerezArgs;

/// Checks the path and the terminal before any read, reads the file (if any) with the normal
/// pipeline and runs the rounds until the user quits.
pub(crate) fn run(
    config: Config,
    registry: Registry,
    args: &DerezArgs,
) -> Result<ExitCode, String> {
    if args.path.as_ref().is_some_and(|p| p.as_os_str() == "-") {
        return Err("derez needs a file path, not stdin".to_string());
    }
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err("derez needs a terminal".to_string());
    }
    let rounds = match &args.path {
        None => rezzer_derez::rounds("", None, None),
        Some(path) => {
            let pipeline = Pipeline::new(config, registry);
            let result = pipeline.run_path(path, &Request::default());
            let Some(doc) = &result.document else {
                return Err(result
                    .message
                    .unwrap_or_else(|| "the file could not be read".to_string()));
            };
            let why_not = if pipeline.registry().for_format(doc.source.format).is_none() {
                Some("Rezzer has no reader for this format".to_string())
            } else if result.completeness == Completeness::Failed {
                result.message.clone()
            } else {
                None
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            rezzer_derez::rounds(&name, Some(doc), why_not.as_deref())
        }
    };
    let seed = args.seed.unwrap_or_else(clock_seed);
    rezzer_derez::derez(rounds, seed)?;
    Ok(ExitCode::SUCCESS)
}

/// The low 64 bits of the nanoseconds since the Unix epoch; 0 for a clock before it.
fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}
