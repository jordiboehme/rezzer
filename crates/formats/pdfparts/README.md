# pdfparts

pdfparts reads the object layer of a PDF file. It tells what the file is and how it is built, not what the text says.

- Document metadata: the Info dictionary with dates in ISO 8601 (the UTC offset is kept), selected XMP properties, the language and whether the file is tagged
- Pages with their physical number, page label (decimal, roman, letters, prefix and start value), size, rotation, fonts and images
- The outline with levels, titles and target pages. Titles are decoded from PDFDocEncoding or UTF-16 by this crate
- Named destinations and link annotations. A link to a URI, another file or a program is listed and never followed
- Encryption: the method, whether the file opens without a password and a decrypted copy of the file for other parsers

```rust
use pdfparts::objects::{pdf_date, text_string};

assert_eq!(text_string(b"Is\xEDs"), "Is\u{ed}s");
assert_eq!(
    pdf_date("D:20151230152248+01'00'").as_deref(),
    Some("2015-12-30T15:22:48+01:00")
);
```

`Pdf::open` takes the bytes, an optional password, limits and a stop function. Before it returns, it measures every stream once. The data goes into counters that keep nothing. Every filter layer of a stream is counted, not only the last one. A stream or a layer may not unpack to more than `max_stream_bytes`. Once it is past 16 MiB, it may also not unpack to more than `max_ratio` times the size of the stream. Smaller streams with a high ratio, such as blank images, are fine. All streams together may not unpack to more than `max_total_bytes`. A page that uses a stream over a limit is refused (`refused_pages`). Do not give such a page to another parser.

Object streams and cross-reference streams are unpacked while the file loads. So they are measured in the raw file first, before the parser sees them. The scan reads every object header in file order and follows the `startxref` chain. Each of these streams may unpack to at most 64 MiB, here and in the parser. A stream over that or over another limit ends the open with `OpenError::Limit`. In an encrypted file the object streams are still encrypted at that point. The parser unpacks them one at a time with the 64 MiB cap, and they are measured again after the load.

Page numbers follow lopdf's page walk, so they match other parsers built on lopdf. `unsafe_page_tree` is true when the page tree has a cycle, a node listed twice, a node without a type or too much depth. Such a file may get other page numbers in a parser with its own walk. Do not give it to one.

The parser underneath is lopdf, used as an object store only. Trees, the outline, labels and links are walked by this crate with a visited set and depth and item limits. A cycle or a broken entry becomes an `Issue` and the rest is still read. Nothing panics.

The password is never written into an error, an issue or a `Debug` output.

lopdf has a `rayon` feature. When any crate in the same build turns it on, lopdf parses objects on rayon threads while it loads a file. Memory checks must then measure the whole process (for example its maximum resident set size), not one thread.

License: MPL-2.0
