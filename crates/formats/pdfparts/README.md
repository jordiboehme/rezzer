# pdfparts

pdfparts reads the object layer of a PDF file. It tells what the file is and how it is built, not what the text says.

- Document metadata: the Info dictionary with dates in ISO 8601 (the UTC offset is kept), selected XMP properties, the language and whether the file is tagged
- Pages with their physical number, page label (decimal, roman, letters, prefix and start value), size, rotation, fonts and images
- The outline with levels, titles and target pages. Titles are decoded from PDFDocEncoding or UTF-16 by this crate
- Named destinations and link annotations. A link to a URI, another file or a program is listed and never followed
- Annotations such as comments, highlights, notes and drawings with author, dates, colour, quad points and replies. Links, popups and form widgets are left out
- The structure tree (tags) with roles after the role map, marked content, alternate text and table headers and spans. `role_of` finds the element of a piece of marked content through the parent tree
- Embedded files with name, type, size and dates. Their bytes are unpacked under the same limits as every stream and are never opened
- Layers (optional content) with their names and whether they are shown when the file opens
- Article threads with their title, pages and number of beads
- Form fields with full names, types, values, flags in words, options, pages and scripts. Values that a field takes from its parents are filled in
- Form scripts in words: the fields a script reads, and Acrobat calls such as `AFNumber_Format(2, 0, 0, 0, "$", true)` as `number, 2 decimals, "$" before the number`
- Document scripts, the open action, document, page, field and link actions. They are listed and never run or followed
- Signatures with signer, reason, date, sub-filter and byte range, and whether they cover the whole file. They are described, not verified
- Encryption: the method and whether the file opens without a password
- Revisions: each save of the file with its end offset, its modification date and what it changed in words, such as `field Name value changed from Alice to Bob`, `signed field Signature1` or `changed page 2 content`. Only the newest saves are compared (`max_revisions`), older ones are counted
- A canonical copy of the file for other parsers: only the objects the document uses, decrypted, with a classic cross-reference table and no object streams

```rust
use pdfparts::objects::{pdf_date, text_string};

assert_eq!(text_string(b"Is\xEDs"), "Is\u{ed}s");
assert_eq!(
    pdf_date("D:20151230152248+01'00'").as_deref(),
    Some("2015-12-30T15:22:48+01:00")
);
```

`Pdf::open` takes the bytes, an optional password, limits and a stop function. Before it returns, it measures every stream once. The data goes into counters that keep nothing. Every filter layer of a stream is counted, not only the last one. A stream or a layer may not unpack to more than `max_stream_bytes`. Once it is past 16 MiB, it may also not unpack to more than `max_ratio` times the size of the stream. Smaller streams with a high ratio, such as blank images, are fine. All streams together may not unpack to more than `max_total_bytes`. A page that uses a stream over a limit is refused (`refused_pages`). Do not give such a page to another parser.

pdfparts has its own PDF parser, and it is the only code that reads the file. It follows the whole cross-reference chain (every section and entry counts against `max_items`; past that, objects only older sections define are found by a scan of the file), or rebuilds the table with one scan of the file when the chain is damaged (with an issue that says so). It copies only the objects the trailer reaches. Each object stream is unpacked once, when the first of its members is needed, and object and cross-reference streams may unpack to at most 64 MiB each. Over that, the open ends with `OpenError::Limit`. Every parsed value costs 128 bytes plus its strings, names and stream data, and all of them together may not pass `max_object_bytes` or `max_items`.

From these objects pdfparts writes `canonical()`: a plain PDF with a classic cross-reference table, no object streams, no comments and no encryption. Give other parsers these bytes, never the original file. `original()` keeps the bytes the file was opened from, for checks such as signature byte ranges. The `/Contents` of a signature is not decrypted, because it is not encrypted in the file.

Page numbers follow lopdf's page walk, so they match other parsers built on lopdf. `unsafe_page_tree` is true when the page tree has a cycle, a node listed twice, a node without a type or too much depth. Such a file may get other page numbers in a parser with its own walk. Do not give it to one.

lopdf is used as the object model in memory, to compute the decryption key and decrypt single objects, and to write the canonical copy. It never parses the original file. Trees, the outline, labels, links, annotations, the structure tree and threads are walked by this crate with a visited set and depth and item limits. A cycle or a broken entry becomes an `Issue` and the rest is still read. Nothing panics.

A reader that copies text or lists out of the file may copy at most what is left of `max_object_bytes` after the open. Many entries can name one long string, and this keeps such a file from filling the memory. When the budget runs out, the reader stops with an `Issue` and keeps what it has. An `/Annots` array that several pages share is read once, for the first of these pages. The structure tree and the embedded files are read once and kept.

Revisions are read from the bytes of the file with the same parser. Each section of the cross-reference chain is one save. The first section of a linearized file is part of the first save. Only the objects a save lists are parsed, and each object and object stream is read once. Pages are compared by their content references and stream lengths, not by decoded content. The pages are walked once, and again only after a save that changes an object they were read from. The result is read once and kept.

The password is never written into an error, an issue or a `Debug` output.

lopdf has a `rayon` feature. When any crate in the same build turns it on, lopdf parses objects on rayon threads while it loads a file, for example the canonical copy. Memory checks must measure the whole process (for example its maximum resident set size), not one thread.

License: MPL-2.0
