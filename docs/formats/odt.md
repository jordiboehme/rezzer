# ODT

Rezzer reads OpenDocument text files with their structure and the layer behind the text: annotations with replies, tracked changes, notes and fields.

Status: planned for the first release. Nothing is implemented yet.

File types: `.odt`. Rezzer detects the format by content, not by extension. A file named `.odt` that is not an OpenDocument file is reported as such.

Built on:

- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml). The tested crates mix annotations and change data into the body text, so Rezzer reads the document parts itself

## What Rezzer extracts

### Structure

- Headings with level, lists, tables
- Footnotes and endnotes
- Math
- Links and bookmarks

### Metadata

- Document properties from the ODF meta part

### Hidden logic

- Annotations anchored to their passage, with author and date
- Annotation replies
- Tracked changes with author and date. Nothing is accepted silently
- Fields and cross references, the same way as for [DOCX](docx.md)

### Pictures and embedded files

- Pictures are extracted to files and marked in the text
- Embedded objects are read by Rezzer as their own document

## How it shows up in the output

The same as for [DOCX](docx.md): the current text with markers for tracked changes, comment threads with the quoted passage and the replies. A field list follows.

## Known limitations

- anydoc 0.2.4 can read ODT structure well, but it pastes annotation and change author and date into the heading and body text. That is why Rezzer does not use it for ODT
- Only two real ODT test files were used. Fields in ODT are not proven on a real file yet
- Against hostile files Rezzer uses size, depth and time limits

## Tested with

3 public test files from Apache Tika and LibreOffice, 1 of them a file named `.odt` that is not a zip archive. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
