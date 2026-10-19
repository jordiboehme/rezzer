# EPUB

Rezzer reads e-books with their full book metadata (authors and their roles, series, accessibility), the table of contents and a graph of footnotes and cross references.

Status: planned for the first release. Nothing is implemented yet.

File types: `.epub` (EPUB 2 and EPUB 3). Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [rbook](https://crates.io/crates/rbook) 0.7.12 for the package: metadata, spine, table of contents, landmarks and the resource reader
- The same HTML reader as for [HTML](html.md), built on [scraper](https://crates.io/crates/scraper), for the chapters
- Rezzer's own reader for the footnote and cross reference graph and the reading order check. A fallback package reader on [zip](https://crates.io/crates/zip) and [roxmltree](https://crates.io/crates/roxmltree)

## What Rezzer extracts

### Structure

- Chapters in reading order, each through the HTML reader: headings, lists, tables, figures, MathML
- Table of contents with its levels, landmarks
- Pages marked as not part of the main reading order

### Metadata

- Titles (main title, subtitle)
- Creators and contributors with their roles (author, translator, annotator and so on) and their sort names
- Publication and modification dates
- Identifiers, language, publisher, subjects
- Series and position in the series
- Accessibility metadata (conformance, access modes, features, hazards)

### Hidden logic

- Footnotes and endnotes, also across files, with their backlinks
- Cross references between chapters as a graph
- Reading order compared with the table of contents. Differences are noted

### Pictures and embedded files

- Pictures are extracted to files and marked in the text with their alt text

## How it shows up in the output

Footnotes appear as Markdown footnotes at their reference, with the text pulled from the target. A `Cross references` table lists the links between chapters. Big books can get a Mermaid graph of chapter links. A `Reading order` note appears when the spine and the table of contents differ. Accessibility metadata goes into the frontmatter.

## Known limitations

- rbook is young and has one maintainer. Rezzer keeps its own fallback package reader
- No public test file was found with a page list or with series data in the Calibre style. These parts are not proven on a real file
- Rezzer checks footnote links across files itself. rbook does not resolve them

## Tested with

6 public test files from Project Gutenberg, Standard Ebooks and the IDPF EPUB 3 samples. See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
