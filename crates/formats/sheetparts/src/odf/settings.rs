// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The view settings of an ODF spreadsheet (`settings.xml`): which sheets have a frozen pane.
//!
//! The settings keep, per view and per sheet, a split mode and a split position for each
//! direction. Mode 2 is a fixed split (a frozen pane); mode 1 is a movable split and is no freeze.
//! The position is the number of rows (vertical split) or columns (horizontal split).

use std::collections::HashMap;

use crate::ooxml::Frozen;
use crate::xml::Ev;

const MAX_ROWS: u32 = 1_048_576;
const MAX_COLS: u32 = 16_384;

#[derive(Default)]
struct Split {
    h_mode: i64,
    v_mode: i64,
    h_pos: i64,
    v_pos: i64,
}

/// Collects the frozen panes by sheet name. The first view that names a sheet decides.
#[derive(Default)]
pub(super) struct Settings {
    views: HashMap<String, Frozen>,
    tables_depth: Option<usize>,
    sheet: Option<(String, Split)>,
    item: Option<(String, String)>,
}

impl Settings {
    /// The sheets with a frozen pane.
    pub fn frozen(self) -> HashMap<String, Frozen> {
        let mut views = self.views;
        views.retain(|_, f| f.rows > 0 || f.cols > 0);
        views
    }

    pub fn event(&mut self, ev: &Ev<'_>, depth: usize) {
        match ev {
            Ev::Start(e) | Ev::Empty(e) => {
                let empty = matches!(ev, Ev::Empty(_));
                let name = e.attr("name").unwrap_or("");
                match (e.name.as_str(), self.tables_depth) {
                    ("config-item-map-named", None) if name == "Tables" && !empty => {
                        self.tables_depth = Some(depth);
                    }
                    ("config-item-map-entry", Some(t)) if depth == t + 1 => {
                        if empty {
                            return;
                        }
                        self.sheet = Some((name.to_string(), Split::default()));
                    }
                    ("config-item", Some(t)) if depth == t + 2 && !empty => {
                        self.item = Some((name.to_string(), String::new()));
                    }
                    _ => {}
                }
            }
            Ev::Text(t) => {
                if let Some((_, text)) = &mut self.item {
                    text.push_str(t);
                }
            }
            Ev::End(_) => {
                let Some(t) = self.tables_depth else { return };
                if depth == t + 2 {
                    if let (Some((key, text)), Some((_, split))) =
                        (self.item.take(), &mut self.sheet)
                    {
                        let n = text.trim().parse::<i64>().unwrap_or(0);
                        match key.as_str() {
                            "HorizontalSplitMode" => split.h_mode = n,
                            "VerticalSplitMode" => split.v_mode = n,
                            "HorizontalSplitPosition" => split.h_pos = n,
                            "VerticalSplitPosition" => split.v_pos = n,
                            _ => {}
                        }
                    }
                } else if depth == t + 1 {
                    if let Some((name, split)) = self.sheet.take() {
                        let rows = if split.v_mode == 2 { split.v_pos } else { 0 };
                        let cols = if split.h_mode == 2 { split.h_pos } else { 0 };
                        let rows = rows.clamp(0, i64::from(MAX_ROWS)) as u32;
                        let cols = cols.clamp(0, i64::from(MAX_COLS)) as u32;
                        // The first view that names a sheet decides, even without a freeze.
                        self.views.entry(name).or_insert(Frozen { rows, cols });
                    }
                } else if depth == t {
                    self.tables_depth = None;
                }
            }
        }
    }
}
