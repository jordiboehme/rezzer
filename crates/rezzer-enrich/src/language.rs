// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass `language`: sets `properties.language` from the body text when it is empty.

use rezzer_model::{Block, Document, Inline};

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
        if text.chars().take(MIN_CHARS).count() < MIN_CHARS {
            return;
        }
        if let Some(info) = whatlang::detect(&text)
            && info.is_reliable()
        {
            doc.properties.language = Some(iso_639_1(info.lang().code()).to_string());
        }
    }
}

/// The text of paragraphs, headings, tasks and cells, one line each, cut at [`MAX_BYTES`].
/// Code and formulas are left out: they say nothing about the language.
fn body_text(body: &[Block]) -> String {
    let mut text = String::new();
    walk::blocks(body, |b| {
        walk::block_inlines(b, |list| {
            walk::inlines(list, |i| match i {
                Inline::Text { text: t, .. } | Inline::Status { text: t, .. } => {
                    push_capped(&mut text, t);
                }
                _ => {}
            });
            push_capped(&mut text, "\n");
        });
        text.len() < MAX_BYTES
    });
    text
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
        assert_eq!(iso_639_1("deu"), "de");
        assert_eq!(iso_639_1("cmn"), "zh");
        assert_eq!(iso_639_1("xyz"), "xyz");
    }

    #[test]
    fn every_whatlang_language_maps_to_two_letters() {
        for lang in whatlang::Lang::all() {
            assert_eq!(iso_639_1(lang.code()).len(), 2, "{}", lang.code());
        }
    }

    #[test]
    fn cap_respects_char_boundaries() {
        let mut s = "a".repeat(MAX_BYTES - 1);
        push_capped(&mut s, "äb");
        assert_eq!(s.len(), MAX_BYTES - 1);
    }
}
