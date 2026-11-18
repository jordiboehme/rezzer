// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout and text line detectors on Candle (feature `detect`, CPU only).

/// The `config.json` of PP-DocLayoutV3 at the pinned revision of
/// [`crate::model::LAYOUT_V3`], embedded so the model folder needs no fifth file.
pub const V3_CONFIG_JSON: &[u8] = include_bytes!("v3/config.json");
