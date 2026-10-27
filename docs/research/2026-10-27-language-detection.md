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
  (`/Users/jordi/git/rezzer-pdf` fa89bfc, read only: `empty` statuses and `found-by` notes are
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
- Of 59 samples, 27 have fewer than 200 letters and never reach a detector (the same for every
  candidate): `language_not_set_for_short_text`, `csv_columns_summary_does_not_set_a_language`
  (99 letters), `form:en-invoice` (194 letters) and 24 small PDFs. The rest, 32 samples, are
  counted. `numeric_table_gets_no_language` and the sheet cases of
  `generated_sheet_headings_are_not_sampled_for_the_language` are decided by `body_text` and
  the 200-letter gate before any detector runs, so they are not counted
- Expected languages were set by reading each sample, never from `properties.language` (that
  is either whatlang's answer or the file's own `/Lang`). Where a text has no one language
  (a list of place names, Lorem ipsum, outline junk), "no answer" counts as right, and so does
  the one language a reasonable reader could accept (marked `-/en` and similar)
- "Abstaining" is Rezzer's rule for whatlang (`is_reliable`). For lingua and py3langid the
  answer counts only when the top confidence is at least 0.5 (one threshold, chosen before the
  run; py3langid's README uses 0.5 in its own example). whichlang has no confidence and always
  answers (an empty text gives English, per its docs)
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
| Confidence / abstaining | confidence and `is_reliable` | none | confidence values; `None` only on a tie | normalized score, `min_confidence` gives `und` |
| Probe size, stripped (baseline harness 0.45 MB) | 0.70 MB (+0.25 MB) | 0.71 MB (+0.26 MB) | 249.4 MB with Rezzer's 57 languages; 291.8 MB with all 75; 12.2 MB with German and English only | 9.8 MB (+9.3 MB; the model file is 8.96 MB) |
| Peak RSS over all samples | 4 MB | 3 MB | 162 MB (57 languages), 195 MB (75) | 110 MB |
| ms per 100 KiB (English, German, place names) | 1.33, 1.42, 1.37 | 1.01, 0.99, 1.00 | 50.6, 55.8, 60.4 (57 languages) | 1.07, 1.13, 1.10 |

For scale: Rezzer's own release binary on this branch is 16.6 MB.

Lingua lacks 13 of whatlang's languages: Akan, Amharic, Javanese, Khmer, Kannada, Malayalam,
Burmese, Nepali, Oriya, Sinhala, Turkmen, Uzbek and Yiddish. The lingua probe "for Rezzer"
uses the 57 languages both have. `detect_language_of` runs serially; lingua loads its models
lazily on the first call and has rayon in its tree for parallel loading and the
`*_in_parallel` calls.

rust-py3langid is not among options A to D. It was probed because the brief lists it.

## Results (plan 6's body text, 32 samples that reach a detector)

| Probe | Right (raw top answer) | Right (abstaining) | Wrong language set | No answer where one was expected | Same as whatlang today |
|---|---|---|---|---|---|
| whatlang | 30 | 30 | 1 | 1 | 32 |
| whichlang | 26 | 26 | 6 | 0 | 25 |
| lingua, 57 languages | 27 | 27 | 5 | 0 | 25 |
| lingua, all 75 | 27 | 27 | 5 | 0 | 25 |
| lingua, German and English | 25 | 25 | 6 | 1 | 22 |
| rust-py3langid | 30 | 30 | 2 | 0 | 25 |

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

The two invented German forms, plan 6's form, Faust and Die Verwandlung, the Erlkönig and Molière
poems (`fr` from all of them), the DWD station list, the Markdown
files and the English PDFs were right for every candidate except lingua with two languages.

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
  0.25 MB, 1.4 ms per 100 KiB, 5 small crates. Costs: the skip stays, upstream is slow (last
  release 2025-10-16, the hashbrown bump PR open seven months and one major version behind).
  Not flagged unmaintained (cargo deny advisories ok)
- B, whichlang: tiny, fast and removes the skip, but 16 languages and no way to say "not
  sure". Every text in another language gets a wrong answer (Polish and Czech gave `vi`,
  Lorem ipsum `en` and `fr`, place names `en`). That breaks the rule that Rezzer does not
  guess. Not fit
- C, lingua: the most robust on short label texts (it read every German form right, even
  with today's statuses), but 249 MB for Rezzer's languages (15 times Rezzer's whole binary),
  162 MB memory, about 40 times slower, 76 new crates, and it keeps a hashbrown skip
  (0.14.5). Lacks 13 of whatlang's languages. It also answered with high confidence on junk
  (`lt` 0.99, `sn` 1.00). Fails "no oversized crates"
- D, own crate: removes the skip and can abstain on names and junk, at about a week of work
  plus a training data source with a usable licence (Tatoeba CC0 or CC BY 2.0 FR) and the
  long-term care for accuracy
- rust-py3langid (not an option): good answers (30 of 32), but it cannot be added (exact pin
  conflict on `unicode-normalization`), adds 9.3 MB and 34 crates, is one release old with no
  history, and returns 639-3 labels for small languages

## Decision needed from Jordi

Recommendation: A for 0.1.0. Keep `whatlang =0.18.0` pinned and the `hashbrown@0.15.5` skip,
and re-check at each whatlang update (a release that moves to hashbrown 0.17, or to std's
`HashMap`, removes the skip). Plan 6's `body_text` change fixes the form texts for whatlang.

If the skip itself is the problem, D is the way, not B or C: an own crate after 0.1.0, spec
and code by different agents, trained on Tatoeba's CC0 sentences (or CC BY 2.0 FR with
attribution). B guesses on every language it does not know; C is too large.

Questions:
1. A (keep whatlang, skip stays) for 0.1.0? Or D now?
2. If D later: is CC BY 2.0 FR training data with an attribution notice fine, or CC0 only?
