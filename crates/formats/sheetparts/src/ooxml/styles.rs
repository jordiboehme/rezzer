// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use super::Ctx;
use crate::numfmt;
use crate::xml::Ev;

/// The cell formats of a workbook: which number format each cell style index uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Styles {
    /// The number format id of each `cellXfs` entry, in order.
    xf_ids: Vec<u32>,
    custom: BTreeMap<u32, String>,
}

impl Styles {
    /// The format code of the cell style with this index (the `s` attribute of a cell).
    /// Custom formats come first, then the built-in ids (en-US). `None` when the index is
    /// out of range or the id is neither defined nor built in.
    pub fn number_format(&self, style_index: u32) -> Option<&str> {
        let id = *self.xf_ids.get(style_index as usize)?;
        match self.custom.get(&id) {
            Some(code) => Some(code.as_str()),
            None => numfmt::builtin(id),
        }
    }

    /// The number of cell styles.
    pub fn len(&self) -> usize {
        self.xf_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.xf_ids.is_empty()
    }
}

pub(super) fn read_styles(ctx: &mut Ctx<'_, '_>, part: &str) -> Styles {
    let mut styles = Styles::default();
    let mut in_cell_xfs = false;
    ctx.scan(part, true, &mut |ev, _| match ev {
        Ev::Start(e) | Ev::Empty(e) => match e.name.as_str() {
            "numFmt" => {
                if let (Some(id), Some(code)) = (
                    e.attr("numFmtId").and_then(|v| v.parse().ok()),
                    e.attr("formatCode"),
                ) {
                    styles.custom.insert(id, code.to_string());
                }
            }
            "cellXfs" => in_cell_xfs = matches!(ev, Ev::Start(_)),
            "xf" if in_cell_xfs => styles
                .xf_ids
                .push(e.attr("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0)),
            _ => {}
        },
        Ev::End("cellXfs") => in_cell_xfs = false,
        _ => {}
    });
    styles
}
