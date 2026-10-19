# DOCX

Rezzer reads Word documents with their structure and the layer behind the text: comment threads, tracked changes, fields, cross references, mail merge and embedded objects.

Status: planned for the first release. Nothing is implemented yet.

File types: `.docx`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml). No tested crate covers comments, tracked changes and fields together, so Rezzer reads the document parts itself
- [docx-rs](https://crates.io/crates/docx-rs) and [office_oxide](https://crates.io/crates/office_oxide) only as a reference in tests, not linked

## What Rezzer extracts

### Structure

- Paragraph styles mapped to roles: headings with level, quotes, lists with levels
- Tables, also with merged cells
- Footnotes and endnotes
- Headers and footers
- Hyperlinks and bookmarks
- Equations (OMML)
- Checkboxes and other content controls

### Metadata

- Core, app and custom document properties

### Hidden logic

- Comments anchored to their passage, with author and date
- Comment threads: replies and resolved state
- Tracked changes: insert, delete, move and format, each with author and date. Nothing is accepted silently
- Fields as data: the result stays in the text, the field code goes into a field list
- Cross references (REF, PAGEREF, NOTEREF) as links to their bookmarks, captions (SEQ) as numbered figures and tables
- DOCPROPERTY and DOCVARIABLE fields with the property or variable they read
- Mail merge: the merge fields, the data source path and the query
- Word table formulas like `=SUM(ABOVE)`, listed like spreadsheet formulas
- Linked files with their path

### Pictures and embedded files

- Pictures are extracted to files and marked in the text with alt text and caption
- Embedded objects, for example an Excel worksheet, are read by Rezzer as their own document and shown as a sub-document
- Charts and SmartArt can become Mermaid

## How it shows up in the output

The body shows the current text with markers for tracked changes. Comments come as threads: the quoted passage, then the root comment and the replies with author, date and resolved state. Fields and cross references go into a list and can be drawn as a Mermaid reference graph.

Example (not final):

```markdown
The price is {+120 EUR+}{-100 EUR-}.

### Comment thread on "The price is 120 EUR"

- Reviewer A, 2026-03-02: Is this the net price? (resolved)
  - Reviewer B, 2026-03-03: Yes, net.
```

## Known limitations

- The equation structure is found, but turning OMML into LaTeX is still Rezzer's own work to do
- No public test file was found with a table cell spanning columns (`gridSpan`), a Word table formula or document variables. Rezzer reads these parts, but they are not proven on a real file
- Linked files that are not there are listed with their path and marked "not available"
- Against hostile files Rezzer uses size, depth and time limits. One test file has 5,000 nested tables. Some tested crates crashed or used several GB of memory on it

## Tested with

19 public test files from Apache POI, Apache Tika, LibreOffice and docx4j, one of them a stress file. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
