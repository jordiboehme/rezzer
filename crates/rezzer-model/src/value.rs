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
    /// A finite number. JSON has no NaN or infinity; build it with [`Value::number`].
    Number {
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// A finite fraction, so 0.25 means 25 %. Build it with [`Value::percent`].
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        value: Vec<Value>,
    },
}

impl Value {
    /// A [`Value::Number`], or [`Value::Text`] with the printed value (`NaN`, `inf`, `-inf`)
    /// when `value` is not finite, because JSON cannot carry it.
    pub fn number(value: f64, unit: Option<String>) -> Value {
        if value.is_finite() {
            Value::Number { value, unit }
        } else {
            Value::Text {
                value: value.to_string(),
            }
        }
    }

    /// A [`Value::Percent`], or [`Value::Text`] with the printed value when `value` is not
    /// finite.
    pub fn percent(value: f64) -> Value {
        if value.is_finite() {
            Value::Percent { value }
        } else {
            Value::Text {
                value: value.to_string(),
            }
        }
    }
}
