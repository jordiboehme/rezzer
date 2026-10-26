# DOC (Word 97-2003)

Rezzer reads old binary Word documents: headings, lists, tables, notes, headers and footers, comments with their author and the document properties.

Status: 🗓️ planned for 0.2.0. Nothing is implemented yet.

File types: `.doc`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [office_oxide](https://crates.io/crates/office_oxide) 0.1.15 for headings, headers and footers, footnotes, merged table cells and comments with author
- [anydoc](https://crates.io/crates/anydoc) 0.2.4 as a fallback
- [cfb](https://crates.io/crates/cfb) for document properties
- Rezzer's own reader for comment dates

Everything runs inside the Rezzer binary. No external program is needed.

## What Rezzer extracts

### Structure

- Headings, lists and tables, also with merged cells
- Footnotes
- Headers and footers
- Links

### Metadata

- Document properties from the property sets in the file

### Hidden logic

- Comments with author and date

### Pictures and embedded files

- Pictures are extracted to files and marked in the text

## How it shows up in the output

The same as for [DOCX](docx.md): Markdown with headings, lists and tables, comments next to their passage.

## Known limitations

- The crates for DOC are young (under one year old). Rezzer pins them and keeps reference files with expected output
- office_oxide 0.1.15 gives the comment text and author, but no date. Rezzer reads the date itself
- anydoc 0.2.4 drops comments and has no heading levels for DOC
- Tracked changes in DOC are not tested. Only one public test file with a comment was found. Most of the hidden logic layer of DOC is not proven on real files
- Planned for later: more detail (tracked changes, fields) from Rezzer's own reading of Microsoft's public [MS-DOC](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/) specification

## Tested with

3 public test files from Apache POI and Apache Tika. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
