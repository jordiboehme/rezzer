// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass `language`: sets `properties.language` from the body text when it is empty.

use std::collections::HashSet;

use rezzer_model::{Block, Document, Inline, SectionKind};

use crate::{Pass, walk};

/// Only this many bytes of body text are looked at.
const MAX_BYTES: usize = 100 * 1024;

/// Less text than this many characters is not enough to tell.
const MIN_CHARS: usize = 200;

pub(crate) struct Language;

impl Pass for Language {
    fn name(&self) -> &'static str {
        "language"
    }

    fn run(&self, doc: &mut Document) {
        if doc
            .properties
            .language
            .as_deref()
            .is_some_and(|l| !l.trim().is_empty())
        {
            return;
        }
        let text = body_text(&doc.body);
        if text
            .chars()
            .filter(|c| c.is_alphabetic())
            .take(MIN_CHARS)
            .count()
            < MIN_CHARS
        {
            return;
        }
        if let Some(info) = whatlang::detect(&text)
            && info.is_reliable()
        {
            doc.properties.language = Some(iso_639_1(info.lang().code()).to_string());
        }
    }
}

/// The words of paragraphs, headings, tasks and cells, one line each, cut at [`MAX_BYTES`].
/// Statuses and `found-by` notes are the reader's words and are left out.
/// Only words made of letters count (apostrophes and hyphens inside a word are fine, edge
/// punctuation is dropped): numbers, dates and codes such as `A12-B7` say nothing about the
/// language and make whatlang report a false, "reliable" answer. Code and formulas are left
/// out too, and so are tables with a role (reader-generated summaries) and the region
/// headings a sheet reader generates.
fn body_text(body: &[Block]) -> String {
    // The region headings a reader generates in a sheet (`Table B3:D5`) are the only headings
    // there without an anchor; caption rows and defined-name captions carry one.
    // Sheet sections at any depth.
    let mut generated: HashSet<*const Block> = HashSet::new();
    walk::blocks(body, |b| {
        if let Block::Section {
            kind: SectionKind::Sheet,
            blocks,
            ..
        } = b
        {
            generated.extend(
                blocks
                    .iter()
                    .filter(|x| matches!(x, Block::Heading { anchor: None, .. }))
                    .map(|x| x as *const Block),
            );
        }
        true
    });
    let mut text = String::new();
    walk::blocks(body, |b| {
        // A table with a role (a summary a reader generated) is not the author's text.
        if matches!(b, Block::Table { role: Some(_), .. }) {
            return true;
        }
        if generated.contains(&(b as *const Block)) {
            return true;
        }
        // A note in which the reader says how it found something is not the author's text.
        if matches!(b, Block::Paragraph { role: Some(r), .. } if r == "found-by") {
            return true;
        }
        walk::block_inlines(b, |list| {
            walk::inlines(list, |i| {
                // A status (a page marker, an empty field) is the reader's word, not the
                // author's.
                let Inline::Text { text: t, .. } = i else {
                    return;
                };
                for word in t.split_whitespace() {
                    // One huge text must not be scanned to the end once the sample is full.
                    if text.len() >= MAX_BYTES {
                        break;
                    }
                    let word = word.trim_matches(|c: char| !c.is_alphabetic());
                    if is_word(word) {
                        push_capped(&mut text, word);
                        push_capped(&mut text, " ");
                    }
                }
            });
            push_capped(&mut text, "\n");
        });
        text.len() < MAX_BYTES
    });
    text
}

fn is_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-')
}

fn push_capped(out: &mut String, add: &str) {
    let room = MAX_BYTES.saturating_sub(out.len());
    if add.len() <= room {
        out.push_str(add);
        return;
    }
    let mut end = room;
    while !add.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&add[..end]);
}

/// The ISO 639-1 code for a code of whatlang (ISO 639-3); the 639-3 code itself when the
/// language has no two-letter code.
pub(crate) fn iso_639_1(code: &str) -> &str {
    match code {
        "epo" => "eo",
        "eng" => "en",
        "rus" => "ru",
        "cmn" => "zh",
        "spa" => "es",
        "por" => "pt",
        "ita" => "it",
        "ben" => "bn",
        "fra" => "fr",
        "deu" => "de",
        "ukr" => "uk",
        "kat" => "ka",
        "ara" => "ar",
        "hin" => "hi",
        "jpn" => "ja",
        "heb" => "he",
        "yid" => "yi",
        "pol" => "pl",
        "amh" => "am",
        "jav" => "jv",
        "kor" => "ko",
        "nob" => "nb",
        "dan" => "da",
        "swe" => "sv",
        "fin" => "fi",
        "tur" => "tr",
        "nld" => "nl",
        "hun" => "hu",
        "ces" => "cs",
        "ell" => "el",
        "bul" => "bg",
        "bel" => "be",
        "mar" => "mr",
        "kan" => "kn",
        "ron" => "ro",
        "slv" => "sl",
        "hrv" => "hr",
        "srp" => "sr",
        "mkd" => "mk",
        "lit" => "lt",
        "lav" => "lv",
        "est" => "et",
        "tam" => "ta",
        "vie" => "vi",
        "urd" => "ur",
        "tha" => "th",
        "guj" => "gu",
        "uzb" => "uz",
        "pan" => "pa",
        "aze" => "az",
        "ind" => "id",
        "tel" => "te",
        "pes" => "fa",
        "mal" => "ml",
        "ori" => "or",
        "mya" => "my",
        "nep" => "ne",
        "sin" => "si",
        "khm" => "km",
        "tuk" => "tk",
        "aka" => "ak",
        "zul" => "zu",
        "sna" => "sn",
        "afr" => "af",
        "lat" => "la",
        "slk" => "sk",
        "cat" => "ca",
        "tgl" => "tl",
        "hye" => "hy",
        "cym" => "cy",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_codes_and_keeps_unknown() {
        for (from, to) in [
            ("deu", "de"),
            ("eng", "en"),
            ("jpn", "ja"),
            ("fra", "fr"),
            ("cmn", "zh"),
            ("nob", "nb"),
            ("pes", "fa"),
        ] {
            assert_eq!(iso_639_1(from), to);
        }
        assert_eq!(iso_639_1("xyz"), "xyz");
    }

    #[test]
    fn every_whatlang_language_maps_to_two_letters() {
        for lang in whatlang::Lang::all() {
            assert_eq!(iso_639_1(lang.code()).len(), 2, "{}", lang.code());
        }
    }

    #[test]
    fn only_letter_words_are_kept() {
        assert!(is_word("Gas"));
        assert!(is_word("don't"));
        assert!(!is_word("1999"));
        assert!(!is_word("A12-B7"));
        assert!(!is_word("2025-01-02"));
        assert!(!is_word("6.5e3"));
    }

    /// A form whose text is mostly label and value pairs in tables, with the reader's English
    /// found-by notes and grey `empty` statuses beside it, still reads as German: notes and
    /// statuses are the reader's, not the author's text.
    #[test]
    fn a_form_of_pairs_keeps_its_language() {
        use rezzer_model::{Cell, StatusColor};
        let labels = [
            ("Bruttobezüge", "1.234,56"),
            ("Steuerklasse", ""),
            ("Kirchensteuer", "12,34"),
            ("Solidaritätszuschlag", ""),
            ("Krankenversicherung", "56,78"),
            ("Rentenversicherung", ""),
            ("Arbeitslosenversicherung", ""),
            ("Pflegeversicherung", ""),
            ("Auszahlungsbetrag", "1.000,00"),
            ("Überweisung an die Bank", ""),
            ("Urlaubsanspruch im Jahr", ""),
            ("Resturlaub aus dem Vorjahr", ""),
            ("Freibetrag monatlich", ""),
            ("Eintritt in die Firma", ""),
        ];
        let rows: Vec<Vec<Cell>> = labels
            .iter()
            .map(|(l, v)| {
                let value = if v.is_empty() {
                    vec![Inline::Status {
                        text: "empty".into(),
                        color: StatusColor::Grey,
                    }]
                } else {
                    vec![Inline::plain(*v)]
                };
                vec![Cell::new(vec![Inline::plain(*l)]), Cell::new(value)]
            })
            .collect();
        let note = |t: &str| Block::Paragraph {
            inlines: vec![Inline::plain(t)],
            role: Some("found-by".into()),
            anchor: None,
        };
        let body = vec![
            note(
                "Found by: label and value pairs from rows of fields between drawn rules; each label is the smaller text and pairs with the value beside it or under it.",
            ),
            note(
                "Found by: a column of amounts beside a table, drawn with rules that line up with the table's rules; its amounts are read as one more column of the table, and each total label in it pairs with the amount under it.",
            ),
            note(
                "Found by: 12 decimal separators on this page are drawn marks, not text; they are written as \",\" as in 12 amounts written as text in this file.",
            ),
            Block::Table {
                caption: None,
                header_rows: 0,
                role: None,
                rows,
                alignments: Vec::new(),
                anchor: None,
                layout: None,
            },
        ];
        let text = body_text(&body);
        assert!(!text.contains("Found"), "{text}");
        assert!(!text.contains("empty"), "{text}");
        let info = whatlang::detect(&text).expect("a language");
        assert_eq!(info.lang().code(), "deu", "{text}");
    }

    #[test]
    fn cap_respects_char_boundaries() {
        let mut s = "a".repeat(MAX_BYTES - 1);
        push_capped(&mut s, "äb");
        assert_eq!(s.len(), MAX_BYTES - 1);
    }
}
