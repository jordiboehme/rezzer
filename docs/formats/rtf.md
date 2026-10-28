# RTF

Rezzer reads Rich Text Format files with their tables, notes, lists and links, plus the comments and the info block that the base crate leaves out.

Status: 📝 planned. The release is not set yet. Nothing is implemented yet.

File types: `.rtf`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [anydoc](https://crates.io/crates/anydoc) 0.2.4 for tables, notes, lists and links
- Rezzer's own reader for comments and the info block (title, author, dates)

## What Rezzer extracts

### Structure

- Tables
- Footnotes
- Lists
- Links

### Metadata

- The info block: title, author, dates and the other entries

### Hidden logic

- Comments with author and date, next to their passage

### Pictures and embedded files

- Pictures are extracted to files and marked in the text

## How it shows up in the output

Markdown with headings, lists, tables and footnotes. Comments stay next to the passage they belong to, the same way as for [DOCX](docx.md).

## Known limitations

- anydoc 0.2.4 drops comments. Rezzer reads them itself
- rtf-parser 0.4.3 was tested too. It gives styled text only and mixes the info block into the body text, so Rezzer does not use it
- Only two public test files were used. Tracked changes in RTF are not tested

## Tested with

2 public test files from Apache Tika. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
