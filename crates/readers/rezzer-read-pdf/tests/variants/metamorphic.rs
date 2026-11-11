// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Metamorphic tests of the region engine on every generated shape, one base variant each:
//! each page as the reader hands it to the engine (before any given region), changed in a way
//! that keeps its meaning (see `meta`), must keep the engine's decisions or move a typed
//! region to a spatial one. The breaks found today are pinned per shape and page in
//! `metamorphic-shapes.tsv`, with the task that owns them (see `meta::pin_lines`). The
//! pictures family is read in a fixed sample by default (every [`PICTURE_STEP`]th shape: its
//! shapes are rendered and scanned for codes); `metamorphic_shapes_every_family` (ignored)
//! runs every shape.

use crate::meta::{self, Decisions, Transform};
use crate::{all_shapes, axes, inv};

/// The pictures family's default sample takes every this many shapes.
const PICTURE_STEP: usize = 4;

/// The engine's input of each page of `shape` in the base variant, before any given region,
/// and whether the file is read by its tags.
fn pages_of(shape: &axes::Shape) -> (Vec<layoutregions::PageIn>, bool) {
    let page = axes::page_of(shape, axes::BASE);
    let (_, pages) = inv::read_engine(&page);
    let tagged = pages.first().is_some_and(|p| p.tagged);
    let inputs = pages
        .into_iter()
        .map(|p| {
            let mut input = p.input;
            input.given.clear();
            input
        })
        .collect();
    (inputs, tagged)
}

/// The pinned breaks of the shapes (see `meta::pin_lines`).
fn pins() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/variants/metamorphic-shapes.tsv")
}

/// The pin lines of the shapes of `family`; each break printed (with `--no-capture`).
fn sweep_shapes(family: &[axes::Shape]) -> (Vec<String>, Vec<String>) {
    let (mut names, mut lines) = (Vec::new(), Vec::new());
    for shape in family {
        let (pages, tagged) = pages_of(shape);
        let decide = |p: &[layoutregions::PageIn]| -> Vec<Decisions> {
            rezzer_read_pdf::engine_decisions(p, tagged)
        };
        let findings = meta::sweep(&pages, &Transform::ALL, &decide);
        for l in meta::break_lines(shape.name, &findings) {
            println!("BREAK\t{l}");
        }
        for l in meta::move_lines(shape.name, &findings) {
            println!("MOVE\t{l}");
        }
        names.push(shape.name.to_string());
        lines.extend(meta::pin_lines(shape.name, &findings));
    }
    (names, lines)
}

macro_rules! family_tests {
    ($($test:ident => $family:path,)*) => {
        $(
            /// Every shape of the family in its base variant under every change, against the
            /// pinned breaks (`metamorphic_shapes_every_family` writes them).
            #[test]
            fn $test() {
                if std::env::var_os("REZZER_WRITE_META").is_some() {
                    return;
                }
                let (names, lines) = sweep_shapes(&$family());
                meta::check_pins(&pins(), &names, &lines);
            }
        )*
    };
}

family_tests! {
    metamorphic_forms_hold_their_pins => crate::forms::shapes,
    metamorphic_docs_hold_their_pins => crate::docs::shapes,
    metamorphic_pictures_hold_their_pins => picture_sample,
    metamorphic_classes_hold_their_pins => crate::classes::shapes,
}

/// Every shape of every family under every change, against the pinned breaks, each break
/// printed (with `--no-capture`). `REZZER_WRITE_META` writes the pins instead.
#[test]
#[ignore = "the family tests run the same shapes; run it to print or write the pins"]
fn metamorphic_shapes_every_family() {
    let (names, lines) = sweep_shapes(&all_shapes());
    meta::check_pins(&pins(), &names, &lines);
}

/// The default sample of the pictures family: every [`PICTURE_STEP`]th shape.
fn picture_sample() -> Vec<axes::Shape> {
    crate::pictures::shapes()
        .into_iter()
        .step_by(PICTURE_STEP)
        .collect()
}
