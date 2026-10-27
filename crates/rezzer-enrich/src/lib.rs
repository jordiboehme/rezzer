// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Passes that run on every document after reading: anchor check, language detection and the
//! structured overview.
//!
//! The passes run in a fixed order and each can be switched off by name. Each is linear in the
//! size of the document and does not recurse, so a hostile file cannot overflow the stack.
//! Running [`enrich`] again gives the same document: a loss equal to one already present is not
//! added again and the overview is replaced.

mod anchors;
mod language;
mod overview;
mod walk;
mod work;

use rezzer_model::Document;

pub use work::work_steps;

/// One step of enrichment.
pub trait Pass {
    /// The name used in [`EnrichOptions::disabled`].
    fn name(&self) -> &'static str;
    fn run(&self, doc: &mut Document);
}

#[derive(Debug, Clone, Default)]
pub struct EnrichOptions {
    /// Names of passes to skip: `anchors`, `language`, `overview`. Unknown names are ignored.
    pub disabled: Vec<String>,
}

/// The passes in the order they run. Later passes may rely on earlier ones: the overview
/// counts the losses the anchor check adds.
pub fn passes() -> Vec<Box<dyn Pass>> {
    vec![
        Box::new(anchors::Anchors),
        Box::new(language::Language),
        Box::new(overview::Overview),
    ]
}

/// Runs every pass that is not disabled, in order.
pub fn enrich(doc: &mut Document, opts: &EnrichOptions) {
    for pass in passes() {
        if !opts.disabled.iter().any(|d| d == pass.name()) {
            pass.run(doc);
        }
    }
}
