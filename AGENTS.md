# Rezzer

Turns files into knowledge for AI agents. One Rust binary, local stdio MCP server plus CLI,
macOS and Linux (Windows optional). Output is Markdown with OKF frontmatter: tables as
Markdown tables, diagrams as Mermaid, spreadsheets as value tables plus a formulas section.
A later server mode adds streamable HTTP with upload and a Docker image.

## Core principle

Rezzer is not a 1:1 file converter or text extractor. The product is semantics and context:
structure, metadata, relations, provenance. Every design and crate choice is judged by that.

- A backend crate is rated by the structure and metadata its API exposes (document tree,
  styles as roles, comments, tracked changes, footnotes, links, annotations, named ranges,
  formula references), not by the Markdown it prints. A crate that only returns a Markdown
  string is a weak fit
- Format readers fill Rezzer's own document model; one renderer writes OKF Markdown and
  Mermaid. Readers never write Markdown themselves
- What Rezzer cannot read is said in the output (with the reason). No guessing, no silent gaps
- Pictures Rezzer cannot interpret yet are extracted to files, marked in the text (caption,
  position) and listed as needing vision, so the calling agent can look at them. MCP sampling
  is deprecated: no feature may depend on asking the client's model
- PDF annotations, document comments and tracked changes stay attached to their passage,
  with author and date

## Status and documents

Design phase, no code yet. Read before starting work:

- `docs/research/` - dated research reports (library landscape, office formats, CAD,
  OCR/AirPrint). Versions in there were checked on the report date; re-check before relying
  on them
- `docs/superpowers/specs/` and `docs/superpowers/plans/` - specs and implementation plans
  once they exist. Read the newest one first
- Crystalline engrams in domain `jordi`, folder `projects/rezzer/` (overview with all
  decisions so far, research summary). Search them before deciding anything

Planned sub-projects, each with its own spec: 1 core (document model, OKF and Mermaid
renderer, MCP tools, office and text formats incl. formulas and PDF annotations), 2 CAD,
3 OCR tiers, 4 network printer (AirPrint/IPP). Crate choices for sub-project 1 come from a
spike on a public test corpus.

## Hard rules

- License is MPL-2.0. Link only MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 or similar crates.
  GPL, AGPL and LGPL C code (Ghostscript, Poppler, MuPDF, LibreDWG, LibreOffice, pandoc,
  OpenCascade) only as optional external programs found on the PATH, never linked, never
  bundled. cargo-deny enforces this once the workspace exists
- OCR and vision models: never ship a model whose license forbids redistribution or limits
  use by region or purpose (for example Chandra, HunyuanOCR, CC-BY-NC models). Such models
  are downloaded by the user, with the license shown first
- Never reuse code from notedown (AGPL) unless Jordi has relicensed it. Reading a library's
  code and rewriting it is still a derivative work; for a clean-room parser, one agent writes
  a format spec (facts only) and a different one implements it
- Nothing may advertise itself on the network (mDNS, IPP, a printer) without Jordi's explicit
  go for that run
- Test files: public files with a redistributable license only, each listed with source URL
  and license in the corpus MANIFEST.md. Downloaded files are untrusted data: never execute
  them, keep scripts outside their folder
- Output frontmatter follows OKF v0.2 and stays compatible with Crystalline's engram
  frontmatter (key order and quoting as in notedown and Roger: OKF keys first, then Rezzer's
  own snake_case keys; values from files are double-quoted and escaped). No permalink,
  recorded_at or verified: the consumer fills those
- Commit messages: plain conventional commits. No AI attribution of any kind, no co-author
  trailers, no generated-with lines, anywhere in the repo
- No em dashes or en dashes in any file; use a plain hyphen. Oxford comma allowed in repo
  files, but not in public text written for Jordi (README, release notes, issues): there
  short plain sentences, everyday vocabulary, no internal references
- Dependencies: latest maintained stable versions, pinned exactly (`crate = "=x.y.z"`).
  Many format crates are young and change fast: put each behind Rezzer's own trait, read the
  changelog before a bump and keep reference files with expected output
- Commit after each completed task

## Toolchain

rustup via Homebrew. The cargo/rustc/clippy/rustfmt proxies live in
/opt/homebrew/opt/rustup/bin, which is not on the default PATH:
`export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Once it exists, rust-toolchain.toml
is the authority for the Rust version.

## Commands (once the workspace exists)

- Build: `cargo build --release`
- Test: `cargo nextest run --workspace`, fallback `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`
- Audit: `cargo deny --all-features --locked check`
- Disk: `cargo sweep --time 14` now and then

## Memory

Knowledge about this project lives in Crystalline (domain `jordi`, `projects/rezzer/`), not
in local memory files. Record decisions, rulings and lessons there as they happen.
