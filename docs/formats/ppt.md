# PPT (PowerPoint 97-2003)

Rezzer reads the slide titles, text and document properties of old binary PowerPoint files. For full detail it can use LibreOffice, when it is installed.

Status: planned for the first release. Nothing is implemented yet.

File types: `.ppt`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [office_oxide](https://crates.io/crates/office_oxide) 0.1.15 for slide titles and text
- [cfb](https://crates.io/crates/cfb) for document properties
- [LibreOffice](https://www.libreoffice.org/) as an optional external program, when it is installed. Rezzer converts the file to PPTX with it and then reads it like a [PPTX](pptx.md). LibreOffice is never bundled

## What Rezzer extracts

### Structure

- Slide order and titles
- Slide text with lists

### Metadata

- Document properties from the property sets in the file

### Hidden logic

- Without LibreOffice: none. With LibreOffice: what the [PPTX](pptx.md) reader gets from the converted file

## How it shows up in the output

One section per slide with its title and text. The output says which way the file was read: directly, or converted with LibreOffice.

## Known limitations

- PPT support is weak in every tested crate (office_oxide 0.1.15, anydoc 0.2.4, undoc 0.27.0): titles and text only, no speaker notes, no comments
- Without LibreOffice, speaker notes, comments, charts and embedded objects are not read. The output says so
- Only one public test file with a comment was found. Most of the hidden logic layer of PPT is not proven on real files

## Tested with

2 public test files from Apache Tika. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
