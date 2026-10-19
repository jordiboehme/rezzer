# PPTX

Rezzer reads PowerPoint presentations slide by slide, with speaker notes, comments, charts with their data source and embedded files.

Status: planned for the first release. Nothing is implemented yet.

File types: `.pptx`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml). No tested crate gives comments, sections, charts and embedded objects
- [office_oxide](https://crates.io/crates/office_oxide) and [pptx-to-md](https://crates.io/crates/pptx-to-md) only as a reference in tests, not linked

## What Rezzer extracts

### Structure

- Slide order with titles and section names
- Text with list levels
- Tables
- Speaker notes under each slide
- Hidden slides, marked as hidden, not dropped

### Metadata

- Core, app and custom document properties

### Hidden logic

- Comments with author and date
- Charts with their type, series ranges and the workbook the data comes from
- Embedded objects, for example an Excel worksheet, with their type
- Linked files with their path

### Pictures and embedded files

- Pictures are extracted to files and marked with their alt text
- Embedded workbooks and documents are read by Rezzer as their own document
- Charts and SmartArt can become Mermaid

## How it shows up in the output

One section per slide, grouped by presentation section. The speaker notes follow the slide text. Comments are listed under their slide. Charts become a Mermaid block with their series ranges and data source.

## Known limitations

- No public test file was found with modern comments (the newer comment format with threads) or with a hidden slide. Rezzer reads both, but they are not proven on a real file
- Linked files that are not there are listed with their path and marked "not available"
- Against hostile files Rezzer uses size, depth and time limits

## Tested with

7 public test files from Apache POI, Apache Tika and LibreOffice. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
