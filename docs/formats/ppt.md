# PPT (PowerPoint 97-2003)

Rezzer reads old binary PowerPoint files with its own reader: slides with their text, speaker notes, comments and document properties.

Status: planned for the first release. Nothing is implemented yet.

File types: `.ppt`. Rezzer detects the format by content, not by extension. A `.ppt` file is an OLE compound file with a `PowerPoint Document` stream.

Built on:

- Rezzer's own reader, written from Microsoft's public [MS-PPT] specification of the binary PowerPoint format
- [cfb](https://crates.io/crates/cfb) to open the compound file and read the document properties

Everything runs inside the Rezzer binary. No external program is needed.

## What Rezzer extracts

### Structure

- Slide order and titles
- Slide text with list levels
- Speaker notes
- Tables, as far as the file stores them as tables

### Metadata

- Document properties from the property sets in the file (title, author, dates, custom properties)

### Hidden logic

- Comments with author and date, tied to their slide
- Hidden slides, marked and not dropped
- Embedded and linked objects (for example an Excel chart), listed with their type and link target

### Pictures and embedded files

- Pictures extracted to files, marked on their slide

## How it shows up in the output

One section per slide with its title, text and speaker notes. Comments, hidden slides and embedded objects are listed below the slides.

## Known limitations

- The reader is new work. It is not written yet and has not been tested
- In the tested crates (office_oxide 0.1.15, anydoc 0.2.4, undoc 0.27.0) PPT support was weak: titles and text only. That is why Rezzer gets its own reader
- Chart data inside embedded objects is read only when the embedded object is a format Rezzer reads
- Only one public test file with a comment was found. Most of the hidden logic layer is not proven on real files yet

## Tested with

2 public test files from Apache Tika (crate tests only). See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).

[MS-PPT]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-ppt/
