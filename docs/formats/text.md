# Plain text

Rezzer reads plain text in any encoding and says how it found the encoding, which line endings the file uses and which language it is most likely in.

Status: available in the development version (no release yet).

File types: `.txt` and other plain text files. Rezzer detects the format by content, not by extension. A `.txt` file is also checked for delimited data (see [CSV and TSV](csv.md)), mbox `From ` lines and mail headers (see [mail](mail.md)). The output says how the format was decided.

Built on (versions as tested on 2026-10-19):

- [encoding_rs](https://crates.io/crates/encoding_rs) 0.8.42 and [chardetng](https://crates.io/crates/chardetng) 1.0.0 for the encoding. The order is: byte order mark first, then strict UTF-8, then a guess with chardetng
- [whatlang](https://crates.io/crates/whatlang) 0.18.0 for a language hint
- Rezzer's own reader for line endings and the byte order mark

## What Rezzer extracts

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

## Known limitations

- chardetng 1.0.0 gives no confidence score. Short Latin-1 files with few special characters are a weak spot. The output says when the encoding is a guess
- UTF-16 is found only through the byte order mark
- Latin-1 files are reported as windows-1252. That is correct under the WHATWG encoding standard
- The language hint is reliable on prose, not on tables or lists of names

## Tested with

4 public text files from Project Gutenberg and Aozora Bunko (UTF-8, ISO-8859-1, Windows-1252 and Shift_JIS; the Windows-1252 file was transcoded from a public file). The encoding detection also ran on the CSV test files: 10 files in total, all decoded without a replacement character. See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
