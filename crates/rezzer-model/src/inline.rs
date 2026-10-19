// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A stable id for a block or inline span, for example `Costs!C12`, `p4` or `h:introduction`.
/// Facets and losses refer to anchors.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Anchor(pub String);

/// Inline content inside a paragraph, heading, cell or annotation body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "is_false")]
        emphasis: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        strong: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        strike: bool,
    },
    /// A hard line break inside a paragraph. Soft breaks are a space in the text.
    LineBreak,
    /// A picture kept as a reference: Rezzer does not fetch it.
    Image {
        target: String,
        #[serde(default)]
        alt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Code {
        text: String,
    },
    Link {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// True when the target is an anchor inside this document.
        #[serde(default)]
        internal: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inlines: Vec<Inline>,
    },
    FootnoteRef {
        id: String,
    },
    Math {
        latex: String,
    },
    /// A status label, rendered as `:status[Text]{color}`.
    Status {
        text: String,
        color: StatusColor,
    },
    AnchorMark {
        anchor: Anchor,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusColor {
    Grey,
    Blue,
    Green,
    Yellow,
    Red,
    Purple,
}

fn is_false(b: &bool) -> bool {
    !*b
}
