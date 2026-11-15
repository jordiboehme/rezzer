// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Truth from the files themselves, with no hand work per file: the tags of a tagged PDF
//! ([`tagged`]), the invoice data a PDF carries as XML ([`invoice`]) and the filled fields of
//! a form ([`fields`]). Each gives facts in the format of [`crate::facts`], each distinct
//! fact once per file.

pub mod fields;
pub mod invoice;
pub mod tagged;

/// The most facts of one kind a generator writes for one file, so that a long file does not
/// swamp its set.
pub const CAP: usize = 200;

/// Whether a generated fact may be written to an assertion file of the repo: no file of the
/// repo holds an en dash or an em dash, so a fact whose text has one is left out.
pub fn writable(fact: &crate::facts::Fact) -> bool {
    !fact.to_string().contains(['\u{2013}', '\u{2014}'])
}

/// `facts` with each distinct fact once, in their order.
pub fn distinct(facts: Vec<crate::facts::Fact>) -> Vec<crate::facts::Fact> {
    let mut seen = std::collections::HashSet::new();
    facts
        .into_iter()
        .filter(|f| seen.insert(f.to_string()))
        .collect()
}

/// At most `cap` of `items`, evenly spread over them, in their order.
pub fn sample<T>(items: Vec<T>, cap: usize) -> Vec<T> {
    let n = items.len();
    if n <= cap {
        return items;
    }
    items
        .into_iter()
        .enumerate()
        .filter(|(k, _)| (k * cap) / n != ((k + 1) * cap) / n)
        .map(|(_, item)| item)
        .collect()
}

/// The first `words` words of `text` (all of it when it has no more).
pub fn lead(text: &str, words: usize) -> String {
    let parts: Vec<&str> = text.split(' ').take(words).collect();
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_is_spread_evenly_and_never_over_the_cap() {
        assert_eq!(sample(vec![1, 2, 3], 5), [1, 2, 3]);
        assert_eq!(sample((0..10).collect(), 5), [1, 3, 5, 7, 9]);
        assert_eq!(sample((0..1000).collect::<Vec<_>>(), 200).len(), 200);
        assert_eq!(sample((0..7).collect::<Vec<_>>(), 3).len(), 3);
        assert!(sample(vec![1, 2], 0).is_empty());
    }

    #[test]
    fn a_fact_with_a_dash_of_the_file_is_not_written() {
        use crate::facts::{Fact, Term};
        assert!(writable(&Fact::text(Term::exact("Seite 1 - 2"))));
        assert!(!writable(&Fact::text(Term::exact("Seite 1 \u{2013} 2"))));
        assert!(!writable(&Fact::heading("Teil \u{2014} Zwei", None)));
    }

    #[test]
    fn the_lead_is_the_first_words() {
        assert_eq!(lead("a b c d", 2), "a b");
        assert_eq!(lead("a b", 5), "a b");
    }
}
