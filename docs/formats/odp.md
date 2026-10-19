# ODP

Rezzer reads OpenDocument presentations slide by slide, with speaker notes, comments, charts and pictures with alt text.

Status: planned for the first release. Nothing is implemented yet.

File types: `.odp`. Rezzer detects the format by content, not by extension.

Built on:

- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml). [pptx-to-md](https://crates.io/crates/pptx-to-md) 1.0.0 reads ODP text, but no comments, charts or alt text

## What Rezzer extracts

### Structure

- Slide order with titles
- Text with list levels
- Tables
- Speaker notes under each slide

### Metadata

- Document properties from the ODF meta part

### Hidden logic

- Comments with author and date
- Charts with their data
- Embedded objects and linked files

### Pictures and embedded files

- Pictures are extracted to files and marked with their alt text
- Embedded documents are read by Rezzer as their own document

## How it shows up in the output

The same as for [PPTX](pptx.md): one section per slide, speaker notes after the slide text, comments under their slide and charts as Mermaid.

## Known limitations

- Only one public ODP test file was used, and its notes pages are empty. Speaker notes, comments and charts in ODP are not proven on a real file yet
- Against hostile files Rezzer uses size, depth and time limits

## Tested with

1 public test file from Apache Tika. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
