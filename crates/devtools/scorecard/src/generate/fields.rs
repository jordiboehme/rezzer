// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Facts from the filled fields of a PDF form: the name a field shows to the user (its
//! tooltip, else the last part of its name) and its value are a pair. A pair is a fact of the
//! body, so only a pair the body of the read shows becomes a fact: the name is printed and
//! the value stands with it, as a pair. A field whose value stands only in the list of fields
//! beside the body gives none, nor do check boxes, radio buttons and empty fields.

use rezzer_model::Document;

use super::{CAP, distinct, sample};
use crate::check::check;
use crate::doc::{View, short_name, value_text};
use crate::facts::{Fact, Term};
use crate::text::{has_words, norm};

/// The kinds of fields whose value is text a person typed or chose.
const TEXT_KINDS: [&str; 3] = ["text", "combo", "list"];

/// The facts the filled fields of `doc` give.
pub fn fields(doc: &Document) -> Vec<Fact> {
    let view = View::of(doc);
    let mut out = Vec::new();
    for field in &doc.facets.fields {
        if !TEXT_KINDS.contains(&field.kind.as_str()) {
            continue;
        }
        let Some(value) = field.value.as_ref().and_then(value_text) else {
            continue;
        };
        let label = match &field.label {
            Some(label) => norm(label),
            None => short_name(&field.name),
        };
        let label = label.strip_suffix(':').unwrap_or(&label).trim_end();
        if value.is_empty() || label.is_empty() || !has_words(&view.stream, label) {
            continue;
        }
        let fact = Fact::pair(Term::exact(label), Term::exact(&value));
        if check(&view, &fact).pass {
            out.push(fact);
        }
    }
    sample(distinct(out), CAP)
}
