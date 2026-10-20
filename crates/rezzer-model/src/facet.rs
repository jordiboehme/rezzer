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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formulas: Vec<Formula>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphs: Vec<Graph>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedded: Vec<Embedded>,
    /// Inputs a reader recognized as parameters of the document (assumptions, rates, limits).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
    /// Validation and conditional-format rules, attached to the range they apply to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Formula {
    pub anchor: Anchor,
    /// The original formula text.
    pub expression: String,
    /// For example `excel`, `openformula`, `pdf-js`, `mathcad`, `cad-expression`.
    pub dialect: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// Id of the group of formulas that share one pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<String>,
    /// Evidence or remarks, for example how often an observed relation holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// What sort of formula this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<FormulaKind>,
    /// The area an array, spill or data table formula fills, or the cells a pattern group
    /// covers (set on every member).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Anchor>,
    /// The formula text as stored, only when `expression` differs (OpenFormula `of:=[.B4]`
    /// shown as `=B4`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormulaKind {
    Cell,
    Shared,
    Array,
    Spill,
    DataTable,
    Name,
    Lambda,
}

/// An input a reader recognized as a parameter, with its label and unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Parameter {
    pub anchor: Anchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Where the unit came from: `label`, `unit-cell` (the cell right of the value),
    /// `number-format`, `convert` or `header`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_source: Option<String>,
    /// How many formulas read this parameter.
    pub read_by: u32,
}

/// A validation or conditional-format rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Rule {
    /// `validation` or `conditional-format`.
    pub kind: String,
    /// The range the rule applies to.
    pub anchor: Anchor,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formulas: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Node {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

/// A directed edge between two [`Node`] ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// For example `revision`, `insert`, `delete`, `save`, `log`, `note`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A comment, highlight, note or other annotation, attached to its passage by anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Annotation {
    /// For example `comment`, `highlight`, `note`, `ink`, `free-text`, `sticky`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<Inline>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<bool>,
}

/// Scripts, macros, embedded objects, attachments and external references. Never run or
/// followed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Embedded {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}
