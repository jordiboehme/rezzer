# PostScript and EPS

Rezzer reads the document facts of a PostScript or EPS file from its comment header (title, creator, date, pages, fonts) without running the file.

Status: planned for the first release (document facts). Text is planned for a later release.

File types: `.ps`, `.eps`. Rezzer detects the format by content, not by extension. It reads the DSC comment header (the `%%` lines at the start and end of the file) and the binary header of DOS EPS files.

Built on:

- Rezzer's own reader for the DSC comments. It never runs the PostScript program

Everything runs inside the Rezzer binary. No external program is needed.

## What Rezzer extracts

### Structure

- Page count, also when it is only given at the end of the file (`%%Pages: (atend)`)

### Metadata

- Title, creator, creation date
- Bounding box, fonts used, PostScript language level

### Hidden logic

- How the file was read, said in the output: header only, with the reason the text is missing

### Pictures and embedded files

- Rezzer can't see the pictures on the pages. The output says so

## How it shows up in the output

A `Document` table with the header facts and a clear line that the text was not read.

Example (not final):

```markdown
## Document

| Key | Value |
|---|---|
| Pages | 35 (from the trailer) |
| Language level | 2 |

Text: not read. PostScript is a program and Rezzer does not run it.
```

## Known limitations

- PostScript is a programming language. Rezzer never runs a PostScript file
- The header holds no text. In the first release Rezzer reports the header facts and says that the text was not read
- Planned for later: a small text reader of Rezzer's own that only follows the text-drawing commands and never runs the program. Most PostScript files from Word, dvips and similar tools draw their text in a simple way. Files that compute their text will stay unreadable, and the output will say so
- The tested PostScript interpreter crate (stet 0.8.7) has no safe mode: a file can delete and read files through it. Rezzer does not use it
- No public test file was found with a `%%For` line or with a DOS EPS binary header holding a TIFF preview

## Tested with

3 PostScript and 1 EPS public test files from the RFC Editor and CTAN. See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
