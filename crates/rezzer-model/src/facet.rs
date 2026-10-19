// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::inline::{Anchor, Inline};
use crate::value::Value;

/// Structured knowledge beside the body. Readers may leave any facet empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Facets {
    pub formulas: Vec<Formula>,
    pub graphs: Vec<Graph>,
    pub timeline: Vec<Event>,
    pub annotations: Vec<Annotation>,
    pub embedded: Vec<Embedded>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Formula {
    pub anchor: Anchor,
    /// The original formula text.
    pub expression: String,
    /// For example `excel`, `openformula`, `pdf-js`, `mathcad`, `cad-expression`.
    pub dialect: String,
    pub references: Vec<Anchor>,
    pub cached: Option<Value>,
    pub role: Option<Role>,
    /// Id of the group of formulas that share one pattern.
    pub pattern: Option<String>,
    pub latex: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Input,
    Constant,
    Result,
    Intermediate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Graph {
    pub id: String,
    /// For example `dependency`, `link`, `reply`, `feature-tree`, `assembly`, `netlist`.
    pub kind: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: Option<String>,
    pub anchor: Option<Anchor>,
}

/// A directed edge between two [`Node`] ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Event {
    pub time: Option<String>,
    pub actor: Option<String>,
    /// For example `revision`, `insert`, `delete`, `save`, `log`, `note`.
    pub kind: String,
    pub anchor: Option<Anchor>,
    pub text: Option<String>,
}

/// A comment, highlight, note or other annotation, attached to its passage by anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Annotation {
    /// For example `comment`, `highlight`, `note`, `ink`, `free-text`, `sticky`.
    pub kind: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub anchor: Option<Anchor>,
    pub quoted: Option<String>,
    pub body: Vec<Inline>,
    pub replies: Vec<Annotation>,
    pub resolved: Option<bool>,
}

/// Scripts, macros, embedded objects, attachments and external references. Never run or
/// followed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Embedded {
    pub kind: String,
    pub name: String,
    pub detail: Option<String>,
    pub target: Option<String>,
}
