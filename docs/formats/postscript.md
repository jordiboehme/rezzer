# PostScript and EPS

Rezzer reads the document facts of a PostScript or EPS file from its comment header (title, creator, date, pages, fonts) without running the file. Text is read only by an optional extra in a locked-down process.

Status: planned for the first release. Nothing is implemented yet.

File types: `.ps`, `.eps`. Rezzer detects the format by content, not by extension. It reads the DSC comment header (the `%%` lines at the start and end of the file) and the binary header of DOS EPS files.

Built on (versions as tested on 2026-10-19):

- Rezzer's own reader for the DSC comments. It never runs the PostScript program
- [stet](https://crates.io/crates/stet) 0.8.7 as an optional extra for the text. It runs only in a separate child process with an operating system sandbox: no file writes, no network, no reads outside the input file, with a time and memory limit
- [Ghostscript](https://www.ghostscript.com/) as an optional external program, when it is installed. It is never bundled

## What Rezzer extracts

### Structure

- Page count, also when it is only given at the end of the file (`%%Pages: (atend)`)
- With the optional text reader: the text of each page

### Metadata

- Title, creator, creation date
- Bounding box, fonts used, PostScript language level

### Hidden logic

- How the text was read, said in the output: from the header only, with an interpreter or not at all (with the reason)

### Pictures and embedded files

- Without an interpreter Rezzer can't see the pictures on the pages. The output says so

## How it shows up in the output

A `Document` table with the header facts. Then the text when the optional reader is used, and a clear line on how the text was read.

Example (not final):

```markdown
## Document

| Key | Value |
|---|---|
| Pages | 35 (from the trailer) |
| Language level | 2 |

Text: not read. PostScript is a program, and Rezzer does not run it in the core. Install the optional text reader to read the text.
```

## Known limitations

- PostScript is a programming language. Rezzer never runs a PostScript file in its own process
- stet 0.8.7 has no safe mode. A PostScript file can delete and read files through it. That is why stet runs only in a sandboxed child process, and only when the user turns it on
- stet ships 35 URW fonts. Their license is not fully clear, which is one more reason to keep stet optional
- stet adds about 9 MB. It is not part of the core binary
- The header holds no text. Without an interpreter Rezzer reports the header facts and says that the text was not read
- No public test file was found with a `%%For` line or with a DOS EPS binary header holding a TIFF preview

## Tested with

3 PostScript and 1 EPS public test files from the RFC Editor and CTAN. See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
