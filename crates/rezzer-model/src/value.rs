// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A typed property or cell value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Value {
    Text {
        value: String,
    },
    Number {
        value: f64,
        unit: Option<String>,
    },
    /// A fraction, so 0.25 means 25 %.
    Percent {
        value: f64,
    },
    /// An ISO 8601 date.
    Date {
        value: String,
    },
    /// An ISO 8601 date and time.
    DateTime {
        value: String,
    },
    Bool {
        value: bool,
    },
    List {
        value: Vec<Value>,
    },
}
