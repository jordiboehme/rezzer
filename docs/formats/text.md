# Plain text

Rezzer reads plain text in any encoding and says how it found the encoding, which line endings the file uses and which language it is most likely in.

Status: available in the development version (no release yet).

File types: `.txt` and other plain text files. Rezzer detects the format by content, not by extension. A `.txt` file is also checked for delimited data (see [CSV and TSV](csv.md)), mbox `From ` lines and mail headers (see [mail](mail.md)). The output says how the format was decided.

Built on (versions as tested on 2026-10-19):

- [encoding_rs](https://crates.io/crates/encoding_rs) 0.8.42 and [chardetng](https://crates.io/crates/chardetng) 1.0.0 for the encoding. The order is: byte order mark first, then strict UTF-8, then a guess with chardetng
- [whatlang](https://crates.io/crates/whatlang) 0.18.0 for a language hint
- Rezzer's own reader for line endings and the byte order mark

## What Rezzer reads

### Structure

- The text, decoded to UTF-8

### Metadata

- Encoding and how it was found: byte order mark, valid UTF-8 or a guess
- Line endings (CRLF, LF, CR)
- Language hint with its confidence

### Hidden logic

- Not much to find in plain text. Rezzer checks whether the file is really something else, like a CSV or a mailbox, and reads it that way

## How it shows up in the output

The text as is, with the encoding, the way it was found and the language hint in the frontmatter.

## Example

Real output of `rezzer read` for `text-cp1252-derived-gutenberg-alice.txt` from the test corpus, cut after the first line of the text:

```markdown
---
type: document
title: "text-cp1252-derived-gutenberg-alice.txt"
resource: "/path/to/tests/corpus/web-text/text-cp1252-derived-gutenberg-alice.txt"
generated: { by: rezzer/0.1.0, at: 2026-10-21T00:00:00Z }
source_date: 2026-10-20
temporal_confidence: medium
source_sha256: "233b828e6dd75253e67925d33bb427209250ba40eedce0d3b25262b89ff74047"
source_size: 167712
format: text
detected_by: "text (windows-1252)"
encoding: "windows-1252"
reader: "text/0.1.0"
completeness: complete
losses: 0
language: "en"
---

# text-cp1252-derived-gutenberg-alice.txt

## Overview

| Part | Kind | Size | Role | Reads from |
| --- | --- | --- | --- | --- |
| Part 1 | other | 877 paragraphs |  |  |

The Project Gutenberg eBook of Alice's Adventures in Wonderland
```

The `resource` path is shortened here; Rezzer writes the full path of the file.

## Known limitations

- chardetng 1.0.0 gives no confidence score. Short Latin-1 files with few special characters are a weak spot. The output says when the encoding is a guess
- UTF-16 is found only through the byte order mark
- Latin-1 files are reported as windows-1252. That is correct under the WHATWG encoding standard
- The language hint is reliable on prose, not on tables or lists of names

## How it is tested

The corpus has 4 public text files from Project Gutenberg and Aozora Bunko (UTF-8, ISO-8859-1, Windows-1252 and Shift_JIS; the Windows-1252 file was transcoded from a public file). See [MANIFEST.md](../../tests/corpus/MANIFEST.md).

Extreme cases that were held, at the default limits:

- 100 MB without a line break: one paragraph, peak heap under 4 times the file size
- 1.1 million short paragraphs: the read stops at 2 million elements and says so
- A file of 3 GB: refused by its size before it is read

Speed: `text-utf8-gutenberg-faust.txt` (218 KB) reads in 0.02 seconds with a peak memory of 8 MiB. Measured on an Apple M5 (Mac17,3, 32 GB) with the release build of the development version on 2026-10-21.
