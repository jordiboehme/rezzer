// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `rezzer derez [<PATH>]`.

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rezzer_model::{Completeness, Registry};
use rezzer_pipeline::{Config, Pipeline, Request};

use crate::DerezArgs;

/// Checks the path, then the terminal. Called before the config is loaded, so without a
/// terminal a broken config still gives the terminal error.
pub(crate) fn check(args: &DerezArgs) -> Result<(), String> {
    if args.path.as_ref().is_some_and(|p| p.as_os_str() == "-") {
        return Err("derez needs a file path, not stdin".to_string());
    }
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err("derez needs a terminal".to_string());
    }
    Ok(())
}

/// Reads the file (if any) with the normal pipeline and runs the rounds until the user quits.
/// [`check`] comes first.
pub(crate) fn run(
    config: Config,
    registry: Registry,
    args: &DerezArgs,
) -> Result<ExitCode, String> {
    let rounds = match &args.path {
        None => rezzer_derez::rounds("", None, None),
        Some(path) => {
            let pipeline = Pipeline::new(config, registry);
            let result = pipeline.run_path(path, &request());
            let Some(doc) = &result.document else {
                return Err(result
                    .message
                    .unwrap_or_else(|| "the file could not be read".to_string()));
            };
            let why_not = why_not(
                pipeline.registry().for_format(doc.source.format).is_some(),
                result.completeness,
                result.message.as_deref(),
            );
            rezzer_derez::rounds(&name_of(path), Some(doc), why_not.as_deref())
        }
    };
    let seed = args.seed.unwrap_or_else(clock_seed);
    rezzer_derez::derez(rounds, seed)?;
    Ok(ExitCode::SUCCESS)
}

/// The request of a derez read: a normal read that also asks for page geometry, so a paged
/// file gives its drawn pages. Such a read skips the cache, which keeps no geometry.
fn request() -> Request {
    Request {
        geometry: true,
        ..Request::default()
    }
}

/// Why a read document gives no grid: no reader for its format, else the message of a failed
/// read; `None` when the plans of the document are to be used.
fn why_not(has_reader: bool, completeness: Completeness, message: Option<&str>) -> Option<String> {
    if !has_reader {
        Some("Rezzer has no reader for this format".to_string())
    } else if completeness == Completeness::Failed {
        message.map(str::to_string)
    } else {
        None
    }
}

/// The name for the status line: the last component of `path`, else (a path ending in `..`
/// or a root) the whole path, lossy UTF-8. Never empty for a non-empty path.
fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The low 64 bits of the nanoseconds since the Unix epoch; 0 for a clock before it.
fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derez_asks_for_page_geometry() {
        let req = request();
        assert!(req.geometry);
        // Nothing else differs from a normal read.
        let rest = Request {
            geometry: false,
            ..req
        };
        assert_eq!(format!("{rest:?}"), format!("{:?}", Request::default()));
    }

    #[test]
    fn why_not_names_a_missing_reader_then_a_failed_read() {
        assert_eq!(
            why_not(false, Completeness::Partial, Some("unknown format")).as_deref(),
            Some("Rezzer has no reader for this format")
        );
        assert_eq!(
            why_not(true, Completeness::Failed, Some("the table is broken")).as_deref(),
            Some("the table is broken")
        );
        assert_eq!(why_not(true, Completeness::Failed, None), None);
        assert_eq!(why_not(true, Completeness::Complete, Some("a note")), None);
        assert_eq!(why_not(true, Completeness::Partial, Some("a note")), None);
    }

    #[test]
    fn name_is_the_last_component_or_the_whole_path() {
        assert_eq!(name_of(Path::new("data/stock.csv")), "stock.csv");
        assert_eq!(name_of(Path::new("data/..")), "data/..");
        assert_eq!(name_of(Path::new("..")), "..");
        assert_eq!(name_of(Path::new("/")), "/");
    }
}
