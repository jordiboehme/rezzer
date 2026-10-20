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
        /// True when the source wrote the link as a wikilink (`[[target]]`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        wiki: bool,
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

impl Inline {
    /// Text without emphasis, strong or strike.
    pub fn plain(text: impl Into<String>) -> Inline {
        Inline::Text {
            text: text.into(),
            emphasis: false,
            strong: false,
            strike: false,
        }
    }
}

/// The text of inlines without markup: text, code, math, image descriptions, status labels
/// and link texts; a line break reads as a space. Walked with a stack, so link nesting of
/// any depth cannot overflow the call stack.
pub fn plain_text(list: &[Inline]) -> String {
    let mut out = String::new();
    let mut stack: Vec<std::slice::Iter<'_, Inline>> = vec![list.iter()];
    while let Some(top) = stack.last_mut() {
        let Some(inline) = top.next() else {
            stack.pop();
            continue;
        };
        match inline {
            Inline::Text { text, .. } | Inline::Code { text } => out.push_str(text),
            Inline::Link { inlines, .. } => stack.push(inlines.iter()),
            Inline::Math { latex } => out.push_str(latex),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Status { text, .. } => out.push_str(text),
            Inline::LineBreak => out.push(' '),
            Inline::FootnoteRef { .. } | Inline::AnchorMark { .. } => {}
        }
    }
    out
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
