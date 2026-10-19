// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Standalone CSV sniffing: dialect, header, column types, keys and derived columns.
//!
//! Input is already-decoded text. Use [`sniff`] to find the dialect, then
//! [`analyze`] to get column facts and observed relations.
//!
//! ```
//! let text = "id,name,mass (kg)\n1,a,1.5\n2,b,2.5\n";
//! let dialect = csvsniff::sniff(text).unwrap();
//! let analysis = csvsniff::analyze(text, &dialect, 100_000);
//! assert_eq!(analysis.columns[2].unit.as_deref(), Some("kg"));
//! ```

mod derived;
mod sniff;
mod types;

pub use sniff::{analyze, sniff};
pub use types::{Analysis, Column, ColumnKind, Derived, DerivedOp, Dialect, RaggedRow};
