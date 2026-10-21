// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::block::Block;
use crate::facet::{Facets, FileFacts, Role};
use crate::format::FormatId;
use crate::loss::Loss;
use crate::value::Value;

/// Version of the model JSON. Bump it on every incompatible change.
pub const MODEL_VERSION: u32 = 1;

/// One read file: where it came from, its properties, its body, facets, assets and losses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Document {
    pub model_version: u32,
    pub source: SourceInfo,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<Block>,
    #[serde(default)]
    pub facets: Facets,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<Asset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<Loss>,
    /// The structured overview, filled by the enrich pass `overview`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<Overview>,
}

impl Document {
    /// An empty document of the current [`MODEL_VERSION`].
    pub fn new(source: SourceInfo) -> Document {
        Document {
            model_version: MODEL_VERSION,
            source,
            properties: Properties::default(),
            body: Vec::new(),
            facets: Facets::default(),
            assets: Vec::new(),
            losses: Vec::new(),
            overview: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct SourceInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub name: String,
    /// Lowercase hex.
    pub sha256: String,
    pub size: u64,
    pub format: FormatId,
    pub detection: Detection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader: Option<ReaderInfo>,
}

/// Why the detector chose a format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Detection {
    pub confidence: Confidence,
    pub evidence: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Guess,
    Likely,
    Certain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ReaderInfo {
    pub id: String,
    pub version: String,
}

/// Document properties as typed values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Properties {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom: BTreeMap<String, Value>,
    /// Facts about the file as a whole (page count, signed, encrypted and so on).
    #[serde(default, skip_serializing_if = "FileFacts::is_empty")]
    pub facts: FileFacts,
}

/// An extracted picture or file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Asset {
    pub id: String,
    /// Path inside the output bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub sha256: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    /// True when Rezzer cannot interpret the picture and the calling agent should look at it.
    #[serde(default)]
    pub needs_vision: bool,
    /// The barcodes and QR codes read from the picture, in the order they were found.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codes: Vec<Barcode>,
    /// The picture looks like a barcode or QR code, but none was read from it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub code_unread: bool,
}

/// A barcode or QR code read from a picture. Its payload is data from the file: it is never
/// followed or run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Barcode {
    /// The kind of code: `QR Code`, `Data Matrix`, `Code 128`.
    pub symbology: String,
    /// The payload, when it is text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The payload in base64, when it is bytes that are not text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    /// The size of the payload in bytes. A payload over the reader's limit has neither text
    /// nor base64, only its size.
    pub bytes: u64,
    /// About where the code lies on its page, in points (left, bottom, right, top in the
    /// frame of the page's text), when the picture is the page. A small picture shared by
    /// several places has no box here: each place says where the picture is.
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f32; 4]>,
}

/// A short map of the document: its parts and the main facts. Built by the enrich passes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Overview {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<OverviewPart>,
    /// Label and value pairs in display order, for example `("Formulas", "12")`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<(String, String)>,
}

/// One top-level part of the document, for example a sheet or a chapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct OverviewPart {
    /// Unique id of the part: its anchor, else `part:<n>`. `reads` refers to it.
    pub id: String,
    pub name: String,
    /// The kind in words, for example `sheet` or `chapter`.
    pub kind: String,
    /// For example `12 rows x 4 columns` or `30 paragraphs`.
    pub size: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// Ids of the parts this part reads from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<String>,
}
