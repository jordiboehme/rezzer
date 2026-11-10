# Dependency audit, 2026-11-10

Branch `chore/deps-2026-11`, made from `origin/main` at 5678c56. All versions were checked on
2026-11-10 at the source: the crates.io API for crates, `gh release list` and the GitHub API
for the toolchain, CI tools and Actions. Changelogs were read from the pinned version to the
new one (CHANGELOG.md in the repository, GitHub releases, or the commit list where a project
keeps neither).

Summary: 8 of 47 pinned crates bumped, 3 held (the hayro family), 36 already current. The Rust
toolchain (1.99.1) is current. 5 CI pins bumped, 1 held. Transitive crates updated with
`cargo update`. The fuzz lock, which no longer resolved, now matches the workspace.

## Gate

On the final tree (macOS, Apple silicon): `cargo nextest run --workspace` 3382 passed, 6
skipped; `cargo clippy --workspace --all-targets -- -D warnings` clean on the host,
`x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`; `cargo fmt --all --check` clean;
`cargo deny --all-features --locked check` ok for advisories, bans, licenses and sources (the
duplicate warnings are the same as on main). No snapshot or golden file changed. The fuzz crate
builds with `cargo check`.

## Release age and the dependabot cooldown

`.github/dependabot.yml` waits 7 days before it proposes a new release, so a broken or
malicious version has time to be yanked. Almost every new release here is younger than that:
zip 9.5.0 and brotli-decompressor 6.3.1 are from 2026-11-09. This branch was asked for
explicitly and is not pushed. Do not merge it before 2026-11-16, and check crates.io for yanks
of the new pins first.

## All pinned crates

Maintenance: "active" means a release or a push to the repository within the last six months
and no RUSTSEC advisory. `cargo deny check advisories` reports no advisory for any crate in the
tree, including unmaintained ones.

| Crate | Old pin | New pin | Latest release | Maintenance |
|---|---|---|---|---|
| rmcp | 3.8.0 | 3.9.0 | 2026-11-05 | active, fast release cycle |
| clap | 4.6.7 | current | 2026-09-14 | active |
| serde | 1.0.229 | current | 2026-07-18 | active |
| serde_json | 1.0.154 | current | 2026-10-11 | active |
| thiserror | 2.0.21 | current | 2026-09-23 | active |
| sha2 | 0.11.1 | current | 2026-10-12 | active |
| memmap2 | 0.9.12 | current | 2026-10-21 | active |
| chardetng | 1.0.0 | current | 2026-03-30 | active, slow and stable |
| encoding_rs | 0.8.44 | current | 2026-10-30 | active |
| pulldown-cmark | 0.13.6 | current | 2026-10-30 | active |
| csv | 1.4.0 | current | 2025-10-17 | active (repository push 2026-08-04), finished crate |
| whatlang | 0.18.0 | current | 2025-10-16 | low activity (last push 2025-12-24), see below |
| tokio | 1.54.1 | current | 2026-10-21 | active |
| schemars | 1.2.3 | current | 2026-10-12 | active |
| toml | 1.1.8 | 1.2.1 | 2026-11-06 | active |
| saphyr-parser | 0.3.3 | current | 2026-10-27 | active |
| anyhow | 1.0.104 | current | 2026-07-18 | active |
| insta | 1.49.0 | current | 2026-10-03 | active |
| tempfile | 3.27.0 | current | 2026-03-11 | active (push 2026-10-30) |
| assert_cmd | 2.2.2 | current | 2026-05-11 | active |
| predicates | 3.1.4 | current | 2026-02-11 | active (push 2026-11-02) |
| zip | 9.2.2 | 9.5.0 | 2026-11-09 | active, six releases in one week |
| cfb | 0.16.0 | current | 2026-10-31 | active |
| libc | 0.2.191 | current | 2026-10-16 | active |
| rustix | 1.1.6 | current | 2026-10-27 | active |
| flate2 | 1.1.10 | 1.1.11 | 2026-11-06 | active |
| base64 | 0.23.1 | current | 2026-08-04 | active |
| percent-encoding | 2.3.2 | current | 2025-08-21 | active (rust-url push 2026-10-30), finished crate |
| unicode-normalization | 0.1.26 | current | 2026-10-16 | active |
| formualizer-parse | 4.1.0 | 4.2.0 | 2026-11-08 | active |
| quick-xml | 0.42.0 | current | 2026-08-22 | active |
| calamine | 0.38.0 | current | 2026-11-02 | active |
| lopdf | 0.46.0 | 0.47.0 | 2026-11-04 | active |
| weezl | 0.2.1 | current | 2026-05-15 | active |
| adler2 | 2.0.1 | current | 2025-06-09 | quiet (no push since 2025-06), finished crate, tiny |
| brotli-decompressor | 6.2.0 | 6.3.1 | 2026-11-09 | active |
| hayro | 0.8.0 | held at 0.8.0 (0.9.0 out) | 2026-11-02 | active |
| hayro-interpret | 0.8.0 | held at 0.8.0 (0.9.0 out) | 2026-11-02 | active |
| hayro-syntax | 0.8.0 | held at 0.8.0 (0.9.0 out) | 2026-11-02 | active |
| kurbo | 0.13.1 | current | 2026-05-13 | active |
| skrifa | 0.49.0 | current | 2026-10-21 | active |
| read-fonts | 0.46.0 | current | 2026-10-21 | active |
| unicode-bidi | 0.3.18 | current | 2024-12-16 | quiet releases, repository push 2026-09-15 |
| rxing | 0.9.3 | 0.9.4 | 2026-11-08 | active |
| ratatui | 0.30.2 | current | 2026-06-19 | active |
| crossterm | 0.29.0 | current | 2025-04-05 | active repository (push 2026-11-01), no release since 2025-04 |
| libfuzzer-sys (fuzz) | 0.4.13 | current | 2026-06-04 | active |

No crate-level pins exist outside the workspace table, except the fuzz crate (`fuzz/Cargo.toml`),
which pins libfuzzer-sys and its own zip.

New transitive crates: zip 9.5.0 now pulls in `zeroize` (Apache-2.0 OR MIT) and
`constant_time_eq` (CC0-1.0 OR MIT-0 OR Apache-2.0) even with default features off. Both are
small and pass cargo-deny.

## Toolchain and CI

| Item | Old | New | Notes |
|---|---|---|---|
| Rust (rust-toolchain.toml, cargo-deny-action `rust-version`) | 1.99.1 | current | 1.99.1 is the latest stable (2026-10-16) |
| cargo-nextest (ci.yml) | 0.9.149 | 0.9.151 | see below |
| cargo-cyclonedx (release.yml) | 0.5.9 | held at 0.5.9 | 0.5.10 released today, not yet in install-action's manifest |
| cargo-about (release.yml) | 0.9.3 | current | |
| cargo-auditable (release.yml) | 0.7.7 | current | |
| cargo-fuzz (fuzz.yml) | 0.13.2 | current | |
| cargo-deny (local and cargo-deny-action) | 0.20.2 | current | |
| actions/checkout | v7.0.1 | current | |
| actions/upload-artifact | v7.0.2 | current | |
| actions/download-artifact | v8.0.2 | current | |
| actions/attest | v4.2.2 | current | |
| EmbarkStudios/cargo-deny-action | v2.1.1 | current | |
| github/codeql-action | v4.39.1 | v4.39.3 | |
| zizmorcore/zizmor-action | v0.8.0 | v0.8.1 | |
| taiki-e/install-action | v2.87.32 | v2.90.3 | its manifest knows nextest 0.9.151 |
| sigstore/cosign-installer | v4.1.2 | v4.1.3 | |
| ossf/scorecard-action | v2.5.0 | current | |
| Swatinem/rust-cache | v2.9.3 | current | |
| devops-actions/actionlint | v0.1.13 | current | |

Every Action stays pinned by commit SHA with the version in a comment. The SHAs were taken
with `gh api repos/<owner>/<repo>/commits/<tag>`, so annotated tags resolve to the commit.

cargo-nextest 0.9.150 fixes GHSA-vc25-qg58-w57c: with the experimental run recording,
`cargo nextest run --rerun` on a crafted portable recording could run code during the build.
Rezzer does not use run recording, so it did not affect us. 0.9.151 fixes test groups with
`threads-required` (not used here) and a panic when reusing builds for custom targets.
nextest suggests `nextest-version = "0.9.150"` in `.config/nextest.toml` as a floor; that is a
small follow-up if wanted.

## Bumped crates

### zip 9.2.2 to 9.5.0

- Breaking: none for us. Rezzer reads entries through `ZipArchive` in zipguard and writes test
  archives with `ZipWriter`.
- Features: `ZipIterable::files_with_reader` (9.4.0), `OwnedZipFileEntry` (9.4.1), an
  `ExtractOptions` root directory filter (9.5.0). None is needed: zipguard does its own entry
  walk and never extracts to disk.
- Bugs and security: eight GitHub advisories were published between 2026-10-26 and
  2026-11-07 (stream extraction, `deep_copy_file` memory, colliding normalized paths, staging
  file permissions). All are fixed in 9.1.0, below our old pin, and all are in extraction or
  copy paths Rezzer does not use. The "Merge commit from fork" commits in 9.2.3 to 9.4.2 are
  further hardening of extraction (symlinks, Windows ADS and reserved names). Nothing that
  could have hit zipguard.
- Six releases in one week. No release reverts or re-fixes an earlier one.

### lopdf 0.46.0 to 0.47.0

Rezzer parses PDF objects with its own parser in pdfparts and uses lopdf for its data types,
the page walk and the standard security handler (`decrypt_object`, `EncryptionState`).

- Breaking for us: `Document::save_to` now raises the header to 1.5 when it writes a
  cross-reference stream (#627). This only touches test fixtures: `pdfparts/tests/meta.rs`
  `version_from_header_or_catalog` built a 1.4 file and now writes a classic cross-reference
  table, as the other fixture builders already did. The reader is unchanged.
- Bug that could have hit us: V4 and V5 encryption with `/StmF /Identity`, `/StrF /Identity`
  or omitted default filter names fell back to RC4 (#624), which corrupts strings and streams
  that should stay clear. This path runs in `pdfparts/src/parse/crypt.rs` for every encrypted
  PDF. Our tests and the corpus have no file with Identity crypt filters, so they would not have
  shown it. A test file with `/StmF /Identity` would be a good addition to the corpus.
- Other fixes (dates without a UT offset, named destinations without `/D`, referenced outline
  titles, inherited `/Resources`, object stream comments, free entries of earlier revisions,
  default decompression bound of 256 MiB) are in lopdf's own loader and document helpers.
  pdfparts has its own versions of these (own parser, own date reader in `objects.rs`, own
  decompression guard), so they did not affect us.
- Features: `/Trapped` in `PdfMetadata`, linearization parameters and revision count. pdfparts
  already reads revisions itself (`revisions.rs`); `/Trapped` could be added to the PDF
  metadata in `meta.rs` if wanted, but it needs no lopdf help.

### rmcp 3.8.0 to 3.9.0

- Breaking: none. The `#[tool]` macros and `schema_for_output` keep their API.
- Behaviour change: `schema_for_output` now builds the output schema for the serialize contract
  (#1378): an `Option` without `skip_serializing_if` is `required`. Rezzer uses it for
  `RezOutput` in `rezzer-mcp/src/lib.rs`. No test or snapshot changed, so the advertised schema
  either stayed the same or is not pinned by a test. Worth a look at `tools/list` once.
- Bugs: "don't treat `_meta` alone as a CallToolResult" (#1405), protocol error replies kept
  across cancellation (#1370). These are relevant to the stdio server but no test showed a
  difference. GHSA-c9f9-cp8h-jcj5 (unbounded response body in Streamable HTTP clients) does
  not affect us: Rezzer is a stdio server and uses the client only in tests. It matters for the
  later server mode.
- Feature: the Skills extension (SEP-2640). Rezzer could offer a skill that tells an agent how
  to use `rez` and how to read OKF output; worth a look for `rezzer-mcp` later.

### rxing 0.9.3 to 0.9.4

No changelog and no GitHub tag for 0.9.4; read from the commit list.

- Breaking: none.
- Bugs that could have hit the barcode reader (`crates/formats/barcodes`): QR byte mode now
  guesses the charset from the data with ISO-8859-1 as the default, as the specification says,
  and honours the `CharacterSet` hint; Code 128 no longer stops at 80 characters and ignores
  function codes in the checksum; Data Matrix Base 256 bytes are kept unchanged and ECIs above
  899 are treated as unknown; PDF417 handles ECI codewords 925 and 926; RSS Expanded uses the
  2023 GS1 AI table; EAN-2 add-ons are decoded from the found guard. The barcode tests all pass
  unchanged, so our test codes do not use these cases. Real QR codes with Latin-1 text or long
  Code 128 codes would read better now.
- Jordi's own fix for the MaxiCode detector loop on narrow images is in this release. Rezzer
  never asks for MaxiCode, so it was not reachable.

### formualizer-parse 4.1.0 to 4.2.0

- Breaking: none in the API.
- `ParserError.position` is now a byte offset everywhere (before, some sites gave a token
  index). formulakit does not use the position.
- The parser now refuses a union with a constant operand: `=1,000` was read as the union of 1
  and 0. The sheet reader will now say it cannot read such a formula instead of showing a wrong
  structure. An improvement; no test changed.

### toml 1.1.8 to 1.2.1

Serializer speedups and a `size_hint` in the deserializer. Rezzer only deserializes (config in
`rezzer-pipeline/src/config.rs`, `+++` frontmatter in `rezzer-read-markdown`). No effect.

### flate2 1.1.10 to 1.1.11

Fixes in gzip header reads after input errors and sync flushes across output buffer
boundaries. Rezzer uses raw `Decompress` with its own loop in the PDF decode guard,
`ZlibEncoder` for packing, and `DeflateDecoder` in `rezzer-detect/src/zipwalk.rs`; no gzip.
Nothing that could have hit us.

### brotli-decompressor 6.2.0 to 6.3.1

A large rework of the decoder internals (Huffman table sizing, command loop, bit reader) for
speed and lower memory, with allocation failures now reported instead of aborting. 6.3.1 sizes
the one-shot ring buffer from the output buffer. Rezzer uses the streaming
`DecompressorWriter` for the PDF `/BrotliDecode` filter in `pdfparts/src/guard/decode.rs`.
The tests pass. Because the rework is large and two days old, this is the bump most worth the
cooldown wait. The older brotli-decompressor 5.0.3 stays in the tree through hayro-cmap and
brotli 8.

### Transitive

aes, cc, crc32fast, hayro-cmap 0.1.0 to 0.1.1, hayro-postscript 0.1.0 to 0.2.0, inout, itoa,
js-sys, log, rxing-one-d-proc-derive, smallvec, syn, unicode-properties, uuid, wasm-bindgen,
winnow. All tests pass.

## Held back

### hayro, hayro-interpret, hayro-syntax 0.8.0 (0.9.0 available)

hayro 0.9 moves fonts into a new `hayro-font` crate. With that, `OutlineGlyph::font_cache_key`
is gone, and nothing public links a glyph to its font dictionary any more:
`OutlineGlyph::identifier` and `font_data().cache_key` depend only on the font program (two
dictionaries over one program, or every non-embedded Helvetica, look the same), and
`font_data()` is `None` for Type 1 fonts.

The PDF text device in `crates/readers/rezzer-read-pdf/src/text.rs` joins every glyph with the
font information Rezzer reads from the font dictionary (`known.fonts`, keyed by the
dictionary's cache key): whether the font is embedded (text from the font's own tables), its
style, and whether it is a barcode font. Without the key, the text output would get worse.

Upstream has an open pull request for exactly this: LaurenzV/hayro#1734 "Expose the font
dictionary of a glyph run" adds `GlyphRun::font_dict()`. Once a release has it, the bump is
small. The other two breaks are easy: `Device::begin_marked_content` takes the property list
dictionary instead of an MCID (read `/MCID` from it), and `InterpreterSettings::render_annotations`
became `render_content` (`RenderContent::PAGE`). `kurbo` stays at 0.13.

What 0.9 would give us: one skrifa and one read-fonts copy (hayro-font uses skrifa 0.49, so
skrifa 0.46.2 and read-fonts 0.43.3 drop out), character codes and the property list given to
devices, better Unicode mapping of glyph names, the `Crypt` filter, text with render mode 7 as
invisible text, and many fixes for panics and memory use on malformed files.

What it would take: a release with `GlyphRun::font_dict()`, then key the text device by the
run's font dictionary (`Dict::cache_key()`, the same key `set_font` already uses), adapt
the two `Device` impls (`text.rs`, `render/cost.rs`) and the settings, and check the PDF
golden files for changed text. This touches rezzer-read-pdf, where another branch is at work.

### cargo-cyclonedx 0.5.9 (0.5.10 available)

Released today. taiki-e/install-action v2.90.3, the newest, does not list it yet, so the
pinned install would fall back to a slower path or fail. Bump once install-action knows it.
0.5.10 adds CycloneDX 1.6 and 1.7 output and license inheritance for target subcomponents.

## Crates to consider replacing

None needs action now. No crate in the tree has a RUSTSEC advisory, and no repository is
archived.

- whatlang 0.18.0: last release 2025-10, last push 2025-12. Still fine; watch it. If it gets
  an unmaintained advisory, a small clean-room trigram detector in its own format crate
  would be the way out (see docs/research/2026-10-27-language-detection.md).
- crossterm 0.29.0: no release since 2025-04 but the repository is active. Only used by the
  terminal view.
- adler2 2.0.1 and unicode-bidi 0.3.18: quiet but finished. adler2 is already in the tree
  through miniz_oxide.
- Duplicates that stay until hayro moves: skrifa 0.46.2 and read-fonts 0.43.3 (from hayro
  0.8), brotli-decompressor 5.0.3 (from brotli 8 under hayro-cmap).
