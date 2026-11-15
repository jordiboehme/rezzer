// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `pagevision-eval score --manifest <m>... --results <r> [--baseline] --card <scorecard.json>`

use std::path::PathBuf;

use pagevision_eval::scorecard::{FORMAT, render_markdown};
use pagevision_eval::{EvalError, Scorecard, load_manifest, load_results, score};

const USAGE: &str = "usage: pagevision-eval score --manifest <m>... --results <r> [--baseline] --card <scorecard.json>";

fn run() -> Result<(), EvalError> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("score") {
        return Err(EvalError::Usage(USAGE.into()));
    }
    let (mut manifests, mut results, mut card, mut baseline) = (Vec::new(), None, None, false);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .map(PathBuf::from)
                .ok_or_else(|| EvalError::Usage(format!("{name} needs a value; {USAGE}")))
        };
        match arg.as_str() {
            "--manifest" => manifests.push(value("--manifest")?),
            "--results" => results = Some(value("--results")?),
            "--card" => card = Some(value("--card")?),
            "--baseline" => baseline = true,
            other => {
                return Err(EvalError::Usage(format!(
                    "unknown argument {other}; {USAGE}"
                )));
            }
        }
    }
    let (Some(results), Some(card_path)) = (results, card) else {
        return Err(EvalError::Usage(USAGE.into()));
    };
    if manifests.is_empty() {
        return Err(EvalError::Usage(USAGE.into()));
    }
    let manifests = manifests
        .iter()
        .map(|p| load_manifest(p))
        .collect::<Result<Vec<_>, _>>()?;
    let entry = score(&manifests, &load_results(&results)?, baseline)?;
    let mut card = if card_path.exists() {
        let raw = std::fs::read_to_string(&card_path).map_err(|source| EvalError::Io {
            path: card_path.clone(),
            source,
        })?;
        let card: Scorecard = serde_json::from_str(&raw)?;
        if card.format != FORMAT {
            return Err(EvalError::UnknownFormat { found: card.format });
        }
        card
    } else {
        Scorecard::new()
    };
    print!("{}", render_markdown(&entry));
    card.upsert(entry);
    let mut json = serde_json::to_string_pretty(&card)?;
    json.push('\n');
    std::fs::write(&card_path, json).map_err(|source| EvalError::Io {
        path: card_path,
        source,
    })
}

fn main() {
    if let Err(e) = run() {
        eprintln!("pagevision-eval: {e}");
        std::process::exit(1);
    }
}
