# Language detection: what could take whatlang's place (2026-10-27)

Checked on 2026-10-27 against crates.io, GitHub and probe builds on an Apple M5 (macOS, Rust
1.99.1, release profile as Rezzer's: `strip = true`, `codegen-units = 1`). Re-check versions
before relying on them.

Why: `whatlang =0.18.0` is the only crate in the workspace that needs `hashbrown 0.15.5`, so
`deny.toml` carries a skip for it (`{ crate = "hashbrown@0.15.5" }`). Everything else uses
`hashbrown 0.17.1` (through `indexmap 2.14.2` and `vello_cpu 0.3.0`; `cargo tree -i` with
`--target all`). AGENTS.md asks to judge such a crate against a replacement or an own crate.

What Rezzer needs (`crates/rezzer-enrich/src/language.rs`, read only): one call on a body text
sample of at most 100 KiB (letter words only), run only when the file names no language and
the sample has at least 200 letters; an answer only when the detector is sure
(`Info::is_reliable`, confidence above 0.9), else no language; the code mapped to ISO 639-1.
"No answer" is a valid and wanted outcome: a wrong language set in the frontmatter is worse
than none.

## How it was measured

- A probe workspace outside the repository (scratch folder, own target dir), one binary per
  candidate plus a baseline binary with the same harness and no detector. Each probe built
  alone with `cargo build --release -p <probe>` (built together, Cargo merges lingua's
  language features across the probes)
- Samples go through a copy of Rezzer's `body_text`, linked against the workspace's
  `rezzer-model` crate. Two variants: today's (fix/known-issues 84dc6f6) and plan 6's
  (branch fix/pdf-print-forms at fa89bfc, read only: `empty` statuses and `found-by` notes are
  left out). The swap lands after plan 6, so the main numbers use plan 6's variant
- Samples: the language assertions of `crates/rezzer-enrich/tests/enrich.rs`; plan 6's
  `a_form_of_pairs_keeps_its_language` labels; four invented form texts made of labels (two
  German, two English, built as plan 6 builds a form: a table of label and value rows, empty
  values as `empty` statuses, two English found-by notes); invented Polish and Czech prose
  (outside whichlang's 16 languages); and the body text of every corpus text, Markdown and PDF
  file, from `rezzer read --json --no-cache` of this branch's release binary (48 files)
- `qpdf-user-password.pdf` is left out: it needs a password (exit 1) and has no body text.
  Exit 2 (read with losses) for `irs-f1040-2025.pdf` (XFA data not read),
  `mustang-gnuaccounting-invoice-508.pdf` (role map loop) and the two `qpdf-malformed-*` files;
  their body text is used as read
- 59 samples: 5 from enrich.rs, 1 plan 6 form, 4 invented forms, 2 invented prose texts and
  47 corpus files. The 5 from enrich.rs are `language_detected_for_german_text` (de),
  `language_not_set_for_short_text` (none), the chapter case of
  `generated_sheet_headings_are_not_sampled_for_the_language` (en),
  `csv_of_english_sentences_is_english` (en) and
  `csv_columns_summary_does_not_set_a_language` (none); the two CSV tests go through
  `rezzer read --json` of the same invented CSV text. Not samples:
  `language_kept_when_already_set`, `disabled_pass_is_skipped` and `enrich_is_idempotent`
  (they reuse the German text above and test the pass, not the detector),
  `numeric_table_gets_no_language` (26 letters after `body_text`) and the three sheet cases
  of `generated_sheet_headings_are_not_sampled_for_the_language` (decided by `body_text`)
- 27 of the 59 have fewer than 200 letters and never reach a detector (the same for every
  candidate): `language_not_set_for_short_text`, `csv_columns_summary_does_not_set_a_language`
  (99 letters), `form:en-invoice` (194 letters) and 24 small PDFs. The other 32 are counted:
  3 from enrich.rs, plan 6's form, 3 invented forms, 2 prose texts and 23 corpus files
- Expected languages were set by reading each sample, never from `properties.language` (that
  is either whatlang's answer or the file's own `/Lang`). Where a text has no one language
  (a list of place names, Lorem ipsum, outline junk), "no answer" counts as right, and so does
  the one language a reasonable reader could accept (marked `-/en` and similar)
- "Abstaining" is Rezzer's rule for whatlang (`is_reliable`). For lingua with default
  settings and py3langid the answer counts only when the top confidence is at least 0.5 (one
  threshold, chosen before the run; py3langid's README uses 0.5 in its own example). whichlang
  has no confidence and always answers (an empty text gives English, per its docs)
- lingua was also run with `with_minimum_relative_distance(0.5)`, where `detect_language_of`
  itself returns `None` when "language detection is not reliably possible" (its docs), and
  with `with_low_accuracy_mode()`, alone and with the same distance. The value 0.5 was chosen
  before the run: the allowed range is 0.0 to 0.99, the docs say the distance grows with the
  text length and warn against high values for short phrases (their 0.9 example rejects a
  three-word phrase), and every counted sample has at least 200 letters, so the middle of
  the range should keep real prose and drop near ties. For these runs the answer is what
  `detect_language_of` returns, without the 0.5 confidence rule
- Time: one detector call on a ready 100 KiB sample, after one warm-up call, mean of 20 calls

## Facts

| | whatlang | whichlang | lingua | rust-py3langid |
|---|---|---|---|---|
| Version tested (newest on crates.io, 2026-10-27) | 0.18.0 (2025-10-16) | 0.1.1 (2025-01-21) | 1.8.0 (2026-03-09) | 0.4.0 (2026-09-16, the only release) |
| Licence | MIT | MIT | Apache-2.0 (models Apache-2.0) | BSD-3-Clause (code and embedded model) |
| Last commit (default branch) | 2025-12-24 (README) | 2026-03-30 (a CI bump) | 2026-10-21 | 2026-09-12 (the release) |
| Open issues / PRs | 12 / 8 | 6 / 6 | 24 / 18 | 0 / 0 (repo created 2026-09-12, 0 stars) |
| MSRV | not declared, edition 2024 | not declared, edition 2021 | not declared, edition 2024 | 1.98.1 |
| `unsafe` in own `src/` | none | none | none (rayon, dashmap and crossbeam below it use it) | none, `#![forbid(unsafe_code)]` |
| Crates in the tree (normal deps, the crate itself counted) | 5: whatlang, hashbrown 0.15.5, allocator-api2, equivalent, foldhash 0.1.5 | 1, no dependencies | 38 plus one model crate per language (95 with 57 languages): counter, dashmap 6.2.1 (hashbrown 0.14.5), rayon, regex, fst, include_dir, itertools, strum (syn 2), serde | 34: flate2, icu_casemap =2.1.1, icu_properties =2.1.2 (Unicode-3.0 data crates), unicode-normalization =0.1.25 |
| Languages | 70 | 16 | 75 | 139 plus `zxx` |
| Codes | ISO 639-3 (Rezzer maps to 639-1) | ISO 639-3 | 639-1 and 639-3 | 639-1, and 639-3 for 26 labels (`gcf`, `bcl`, `yue` and others) |
| Confidence / abstaining | confidence and `is_reliable` | none | confidence values; `with_minimum_relative_distance` makes `detect_language_of` return `None` when not reliable (default 0.0: `None` only on a tie) | normalized score, `min_confidence` gives `und` |
| Probe size, stripped (baseline harness 0.45 MB) | 0.70 MB (+0.25 MB) | 0.71 MB (+0.26 MB) | 249.4 MB with Rezzer's 57 languages; 291.8 MB with all 75; 12.2 MB with German and English only | 9.8 MB (+9.3 MB; the model file is 8.96 MB) |
| Peak RSS over all samples | 4 MB | 3 MB | 162 MB (57 languages), 195 MB (75) | 110 MB |
| ms per 100 KiB (English, German, place names) | 1.33, 1.42, 1.37 | 1.01, 0.99, 1.00 | 51 to 57, 56 to 63, 60 to 69 (57 languages, default settings; range over the runs in the lingua settings table below) | 1.07, 1.13, 1.10 |

For scale: Rezzer's own release binary on this branch is 16.6 MB.

Lingua lacks 13 of whatlang's languages: Akan, Amharic, Javanese, Khmer, Kannada, Malayalam,
Burmese, Nepali, Oriya, Sinhala, Turkmen, Uzbek and Yiddish. The lingua probe "for Rezzer"
uses the 57 languages both have. `detect_language_of` runs serially; lingua loads its models
lazily on the first call and has rayon in its tree for parallel loading and the
`*_in_parallel` calls.

rust-py3langid is not among options A to D. It was probed because the brief lists it.

## Results (plan 6's body text, 32 samples that reach a detector)

| Probe | Right (abstaining) | Wrong language set | No answer where one was expected | Same as whatlang today |
|---|---|---|---|---|
| whatlang | 30 | 1 | 1 | 32 |
| whichlang | 26 | 6 | 0 | 25 |
| lingua, 57 languages | 27 | 5 | 0 | 25 |
| lingua, 57 languages, distance 0.5 | 28 | 4 | 0 | 25 |
| lingua, 57 languages, low accuracy | 25 | 7 | 0 | 23 |
| lingua, 57 languages, low accuracy, distance 0.5 | 26 | 6 | 0 | 23 |
| lingua, all 75 | 27 | 5 | 0 | 25 |
| lingua, German and English | 25 | 6 | 1 | 22 |
| rust-py3langid | 30 | 2 | 0 | 25 |

Counting the raw top answer instead (no abstaining) gives the same number right for every
probe except the two distance 0.5 rows, so that column is left out. In those two rows the `None` of
`detect_language_of` is the abstention: their one extra right answer is the `None` on qpdf-aes128,
where the raw top answer is `lt` (raw: 27 and 25 right). For whatlang the two differ only in which
samples are right: the raw answer is right on `form:en-application` (`en`, not reliable) and wrong
on the place names (`it`, not reliable); abstaining turns the first into a missed answer and the
second into a right "no answer". For py3langid the three answers below 0.5 are on samples where both
`en` and "no answer" count as right.

lingua with the settings (57 languages, the same 249.4 MB binary for all four):

| Setting | Abstained on | Wrong languages set | ms per 100 KiB (English, German, place names; two runs) | Peak RSS, time mode |
|---|---|---|---|---|
| default | none | mustang `de`, the three qpdf outlines `lt`, place names `sn` | 51 to 57, 56 to 63, 60 to 69 | 162 MB |
| distance 0.5 | qpdf-aes128 only | mustang `de`, qpdf-aes256 and qpdf-page-labels `lt`, place names `sn` | 51 to 57, 56 to 62, 61 to 67 | 162 MB |
| low accuracy | none | mustang `de`, qpdf-aes128 `lt`, the other two qpdf outlines `el`, place names `sn`, the Rust blog Markdown `sv`, `pypdf-outline.pdf` `de` | 77 to 83, 94, 93 to 94 | 171 to 173 MB |
| low accuracy, distance 0.5 | qpdf-aes128 only | as low accuracy without qpdf-aes128 | 76 to 77, 94, 93 to 94 | 172 to 173 MB |

On this corpus low accuracy mode is slower, uses more memory and reads two English texts
wrong; the distance setting changes the time little and drops one of five wrong answers. The
place names still get `sn` with confidence 1.00, so the distance does not catch lists of names.

The samples where the candidates differ (all others: every candidate right):

| Sample | Expected | whatlang | whichlang | lingua 57 | py3langid |
|---|---|---|---|---|---|
| form:en-application | en | en 0.63, not reliable: none | en | en | en 0.98 |
| extra:pl-prose | pl | pl | vi | pl | pl |
| extra:cs-prose | cs | cs | vi | cs | cs |
| mustang-gnuaccounting-invoice-508.pdf | en/- | en 0.51, not reliable: none | en | de 0.81 | en 0.97 |
| pdfbox-actualtext-din91379.pdf (a table of Unicode letters) | -/en | vi 1.00 | vi | en | gcf 0.88 |
| pypdf-attachment.pdf (Lorem ipsum) | -/la | la | en | la | la |
| pypdf-multicolumn.pdf (Lorem ipsum, English headings) | -/la | la | fr | la | la |
| qpdf-aes128, qpdf-aes256, qpdf-page-labels (outline junk, "Potato" repeated) | -/en | en | en | lt (0.56, 0.99, 0.99) | en below 0.5: none |
| tsv-noheader-geonames-admin1.txt (place names) | - | it 0.08, not reliable: none | en | sn 1.00 | bcl 0.97 |
| text-shiftjis-aozora-botchan.txt | ja | ja | ja | ja | ja (lingua German and English: none) |

The two invented German forms, plan 6's form, Faust and Die Verwandlung, the DWD station
list, the Markdown files and the English PDFs were right for every candidate except lingua
with two languages (and lingua in low accuracy mode on two English texts, see above).
`pdfbox-poems-article-threads.pdf` is one PDF with a German poem (Erlkönig) and a French text
(Molière); it is a mixed text like the others above, expected `fr/de/-`, and every candidate
except lingua with two languages answered `fr`.

With today's `body_text` (statuses and found-by notes kept), the German forms are the weak spot:
whatlang reads `form:de-rechnung` as English (reliable) and plan 6's form as not reliable;
whichlang and py3langid read all three German forms as English; lingua reads all three as
German. Plan 6's change to `body_text` fixes this for whatlang, whichlang and py3langid alike.

The files `pdfbox-actualtext-din91379.pdf` and `mustang-gnuaccounting-invoice-508.pdf` carry
`/Lang de-DE`, so Rezzer does not run a detector on them today; whatlang's `vi` for the letter
table would only show on a file without its own language.

## The hashbrown 0.15 skip

Checked in four copies of the workspace (`git archive HEAD`, no `.git`): whatlang replaced or
removed in `Cargo.toml` and `rezzer-enrich`, the `hashbrown@0.15.5` skip removed from
`deny.toml`, then `cargo tree -d -e normal --target all` and `cargo deny --all-features check
bans licenses advisories`.

- whichlang: `hashbrown 0.15.5` and `foldhash 0.1.5` leave the tree, no new duplicate. The
  skip can go. bans, licenses and advisories ok
- lingua (57 language features): `hashbrown 0.15.5` leaves, `hashbrown 0.14.5` (through
  `dashmap 6.2.1`) comes in, so the skip stays with another version; `strum_macros 0.27.2`
  adds one more user of the already skipped `syn 2`. 76 new crates in `Cargo.lock` (57 of them
  model crates). bans, licenses and advisories ok
- rust-py3langid: does not resolve. It pins `unicode-normalization =0.1.25`, the workspace pins
  `=0.1.26` (rezzer-pipeline): "failed to select a version for `unicode-normalization`"
- whatlang removed and nothing added (the state of option D): `hashbrown 0.15.5` and
  `foldhash 0.1.5` leave; the skip can go
- Option A: upstream PR greyblake/whatlang-rs#154 "deps: bump hashbrown to 0.16" is open since
  2026-03-29 (mergeable). Even merged and released, it would need a skip for 0.16 while the
  workspace uses 0.17. whatlang uses `hashbrown::HashMap` in two places; earlier PRs to drop
  hashbrown (#48, #98, #119) were closed

## Option D: an own clean-room crate (estimate, nothing built)

A trigram profile detector of the kind whatlang uses (script detection first, then distance
to per-language trigram profiles; the method is public in Cavnar and Trenkle, "N-Gram-Based
Text Categorization", 1994), as `crates/formats/<name>` with its own API, plus a small
training tool that never ships in the binary.

- Work, rough estimate: spec (facts only, from the paper and public script tables, written by
  one agent that has not read whatlang's code) about 1 day; code and tests by a different
  agent about 2 to 3 days; training data pipeline and a reference set with expected answers
  about 1 to 2 days. Profiles for about 70 languages at 300 trigrams each would be in the
  order of 100 to 200 KB of tables
- A spec written after reading whatlang's code would be a derivative work (AGENTS.md
  clean-room rule), so spec and code come from different agents and neither reads whatlang
- It could add what Rezzer wants and no candidate has: "no answer" for lists of names, letter
  tables and junk (geonames, din91379, the qpdf outlines), tuned on Rezzer's own corpus
- Training data, licences read on 2026-10-27:
  - Tatoeba sentence downloads (tatoeba.org/en/downloads): "These files are released under
    CC BY 2.0 FR"; "A part of our sentences are also available under CC0 1.0" (a CC0 file for
    some languages only, among them English, French and German). CC BY needs attribution
    with the shipped tables; CC0 needs none
  - FLORES+ (huggingface.co/datasets/openlanguagedata/flores_plus): "released under
    CC BY-SA 4.0". Share-alike on the data that produces the tables is a question for an
    MPL-2.0 binary; use it for testing only, not for training, unless Jordi rules otherwise
  - Leipzig Wortschatz corpora (lingua's training data, per its README): the site's terms page
    could not be read (a bot check page). A search summary says the download corpora are
    CC BY and the rest of the data CC BY-NC. Unverified; read the terms before any use
- Upkeep: Rezzer then owns accuracy across languages, which whatlang's maintainer carries
  today

## Assessment

- A, keep whatlang: best on the corpus (30 of 32 right, one wrong answer, on a letter table),
  0.25 MB, 1.4 ms per 100 KiB, 5 small crates. But by this report's own facts whatlang meets
  AGENTS.md's "outdated" test: no release in over 12 months (0.18.0 on 2025-10-16), only a
  README commit since (2025-12-24), and its `hashbrown 0.15` is two versions behind the
  workspace's 0.17, with the bump PR (#154, to 0.16 only) open since 2026-03-29. That cargo
  deny reports no advisory does not change this: AGENTS.md treats a skip as a reason to
  evaluate a replacement, not a resting state. A is therefore only possible as a time-boxed
  exception that Jordi rules on
- B, whichlang: tiny, fast and removes the skip, but 16 languages and no way to say "not
  sure". Every text in another language gets a wrong answer (Polish and Czech gave `vi`,
  Lorem ipsum `en` and `fr`, place names `en`). That breaks the rule that Rezzer does not
  guess. Not fit
- C, lingua: the most robust on short label texts (it read every German form right, even
  with today's statuses), but 249 MB for Rezzer's languages (15 times Rezzer's whole binary;
  the settings do not change the binary), 162 MB memory, about 40 times slower, 76 new
  crates, and it keeps a hashbrown skip (0.14.5). Lacks 13 of whatlang's languages. With a
  minimum relative distance of 0.5 it abstains on one junk text and is right on 28 of 32, but
  still sets `lt` (0.99) on two outlines and `sn` (1.00) on place names. Low accuracy mode
  does not help here: slower, more memory, and two English texts wrong. Fails "no oversized
  crates"
- D, own crate: removes the skip and can abstain on names and junk, at about a week of work
  plus a training data source with a usable licence (Tatoeba CC0 or CC BY 2.0 FR) and the
  long-term care for accuracy
- rust-py3langid (not an option): good answers (30 of 32), but it cannot be added (exact pin
  conflict on `unicode-normalization`), adds 9.3 MB and 34 crates, is one release old with no
  history, and returns 639-3 labels for small languages

## Decision needed from Jordi

Recommendation: A as a time-boxed exception for 0.1.0, then D. whatlang is outdated by
AGENTS.md's test (see Assessment), so keeping it needs Jordi's ruling. Keep `whatlang =0.18.0`
pinned and the `hashbrown@0.15.5` skip for 0.1.0 only, because no candidate is a better fit
today: B guesses on every language it does not know, C is too large. Plan 6's `body_text`
change fixes the form texts for whatlang.

Trigger: at the start of 0.2.0, or on 2027-04-30, whichever comes first, if whatlang has no
release by then that moves to hashbrown 0.17 or to std's `HashMap` (either removes the
skip), start D: an own crate, spec and code by different agents, trained on Tatoeba's CC0
sentences (or CC BY 2.0 FR with attribution). A whatlang release before then is checked
against this report's samples before the bump.

Questions:
1. Do you allow whatlang, outdated by AGENTS.md's test, as an exception for 0.1.0 with the
   trigger above (0.2.0 or 2027-04-30, then D)? Or D now, before 0.1.0?
2. For D: is CC BY 2.0 FR training data with an attribution notice fine, or CC0 only?

### Decision (Jordi, 2026-10-27)

1. whatlang =0.18.0 stays as a time-boxed exception for 0.1.0, with the hashbrown@0.15.5 skip.
   Trigger: at the start of 0.2.0 or on 2027-04-30, whichever comes first, start an own
   clean-room crate (option D), unless a whatlang release by then removes the skip.
2. For D, all of Tatoeba is allowed, CC BY 2.0 FR included, with an attribution notice in
   THIRD-PARTY-NOTICES.
