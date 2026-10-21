# PDF

Rezzer reads the text of a PDF in reading order, and also the layer behind it: outline, annotations with author and date, form fields with their calculations, scripts, signatures and saved revisions.

Status: available in the development version (no release yet).

File types: `.pdf`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- Rezzer's own crate pdfparts for the object layer. It has its own bounded parser for the file and uses [lopdf](https://crates.io/crates/lopdf) 0.45.0 only on a clean copy it writes itself. It reads metadata, page labels, outline, links, annotations, the tag tree, attachments, layers, threads, forms, scripts, signatures and revisions
- [hayro](https://crates.io/crates/hayro) 0.8.0 (with hayro-interpret and hayro-syntax) for the text with its position on the page, the exact glyph boxes under an annotation, page renders and as a fallback parser for broken files
- Rezzer's own reader for the interpretation: reading order, headings, tables, the form logic, field formats in words and revision diffs

## What Rezzer reads

### Structure

- Text in reading order, also on two-column pages. Every page starts with a marker that names its label and its number in the file
- Headings, lists and tables from the tag tree when the PDF is tagged. Untagged PDFs get headings from the font size
- Tables as cells. A tagged PDF gives its tables from the tag tree. On untagged pages Rezzer rebuilds a table from a grid of drawn lines, or from text in columns with white space between them. A grid with more than 15 columns or with more than 60 % empty cells is read as a form or a chart. In a table without lines at least 80 % of the rows must stay inside the columns and the cells must be short (under 40 characters on median, unless most are numbers). A table that fails a check stays text, with a note that names the check
- Running headers and footers are marked in the JSON model; the Markdown keeps them as text. Text that is invisible on the page is listed apart, with its render mode
- Outline (bookmarks) with level, title and target page
- Page labels (i, ii, iii, then 1, 2, 3 and so on)
- Links: web links in the text, internal targets and named destinations. Internal links stay text, because there is no target to link to in the output
- Article threads (reading paths across pages)
- Arabic text and vertical Japanese text in logical order

### Metadata

- Info dictionary: title, author, subject, keywords, creator, producer, dates with their time zone
- XMP metadata, also PDF/A
- Document language and PDF version
- File facts in the frontmatter: pages, tagged, encrypted, signed, signatures, revisions, form fields, whether it has scripts and layers

### Hidden logic

- Annotations: highlight, underline, strikeout, squiggly, free text, sticky notes, ink, caret, stamps, shapes and others, with author and date. A markup annotation quotes the exact words under it. Replies stay with their annotation, and a review state marks it as resolved
- Form fields: name, label, type, value, flags (read-only, required, multiline and others) and options
- Form calculations: the calculation order and the fields each calculation reads (from `getField("...")` calls)
- Field formats in words: `AFNumber_Format(2, 0, 0, 0, "$", true)` becomes `number, 2 decimals, "$" before the number`. The same for date, time, percent and special formats (ZIP code, phone number, social security number)
- Each field gets a role: input (the user fills it), result (a calculation sets it) or constant (read-only, no calculation)
- Scripts: document JavaScript, open actions, page and field actions. Listed with their text, never run
- Digital signatures: signer, date, reason, location, kind (field, usage rights, certification), whether the signature covers the whole file and how many saves came after it. Signatures are not verified
- Revisions: each incremental save, with its date and what changed
- Layers (optional content) with name and default visibility
- XFA form data is found and said, not read

### Pictures and embedded files

- A page without a text layer, with glyphs that have no Unicode mapping or with large pictures needs vision. It is listed with the reason. With an output directory (`-o` on the command line), Rezzer renders it to a PNG file at 144 dpi, at most 16 million pixels
- Attachments are listed with name and size. They are never opened
- A PDF with a user password is read with `--password-stdin` (command line) or the `password` option (MCP). The password never shows up in the output, in the logs or in the cache

## Example

Real output of `rezzer read` for two files of the test corpus, cut to the parts that show the form logic and an annotation. From `pdfjs-text-field-calc.pdf` (the empty page markers and the Embedded section are cut):

````markdown
## Logic

### Form

| Field | Type | Role | Value | Format | Rule | Reads | Flags |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Source | text | input |  |  |  |  |  |
| Mirror | text | result (calc 1) |  |  | script, see below | Source | read-only |

### Graph: form

Kind dependency, 2 nodes, 1 edge.
```mermaid
flowchart LR
    n_f1["Source (input)"]
    n_f2["Mirror (result, calc 1)"]
    n_f2 -->|"reads"| n_f1
```

### Calculations

Mirror, calc 1:
```javascript
event.value = this.getField("Source").value;
```
````

From `pdfjs-annotation-highlight.pdf` (the frontmatter and the overview are cut):

```markdown
:status[Page 1]{grey}

Highlight

Lorem ipsum dolor sit amet

> [!NOTE] Highlight by Tim van der Meij, 2015-12-30T15:22:48+01:00
> > Lorem ipsum dolor sit amet
>
> Highlight content
```

## Known limitations

- XFA forms are not read. The output says "XFA form data present, not read"
- Scripts are listed, never run. When a script builds field names at run time, the dependency is marked "unknown, see script"
- Tables marked only by a rule at the top and at the bottom (as in many scientific papers) stay paragraphs. A note says "Possible table on page N not rebuilt" with the reason
- A header row of several lines is found by bold text or font size, not by its fill color alone
- Header icons drawn with curves are not marked
- Some line breaks are joined: lines of verse with five words or more, two-line blocks that are aligned right, label pairs and lines like "Suite 200 of the / Old Mill Building". No text is lost
- Tables on pages with form fields and tables inside pictures are not rebuilt
- Scans without a text layer need OCR. OCR is a separate, optional part of Rezzer. Until then the page is rendered and listed as needing vision
- A page render cannot stop in the middle of a page. A page whose pictures would take too much memory is not rendered, and the output says so
- At a low memory limit (256 MiB) a large file like the NIST report with 492 pages is not read, and the output says which limit was hit
- The text parser holds about 2 KB for every page of the file. When that does not fit in the memory left after the objects of the file (for example 100,000 pages with their own content streams at a 512 MiB limit), the text is not read and the output says so
- A word split by a hyphen at a line end keeps its hyphen ("adip-iscing"). Rezzer cannot tell a break hyphen from a real one, so it keeps both
- Paragraphs marked only by an indent of their first line, with no space between them, stay one paragraph with line breaks (as in LaTeX two-column papers)
- On a form page, a block of two text columns can be read in bands across both columns, with each printed line as its own paragraph (the directions on the front of the SF 1199A form)
- When the object layer cannot be read, Rezzer falls back to hayro for the text and says so in the output. Annotations, forms, scripts and signatures are then not read
- Revision diffs count the first save of a linearized file as one save
- No public test file was found for annotation replies, `AFSimple_Calculate`, `AFPercent_Format`, a field with the required flag or table header scope. These parts follow the format specification but are not proven on a real file

## How it is tested

The corpus has 40 public PDF files. Most come from the test suites of pdf.js, qpdf, PDFBox, py-pdf and pdfplumber. The others come from the IRS, the National Archives, NIST and arXiv. 38 of them are in the repository. The two large ones (NIST and arXiv) are fetched by a script.

Extreme cases that were held, at the default limits unless a limit is named. Figures from the release build, measured as below:

- A content stream of 1 MB that unpacks to 1 GiB: the page is not read, the other page is, peak memory 9 MiB
- An object stream of 1 MB that unpacks to 1 GiB: the file is refused, peak memory 73 MiB
- A picture of 40,000 by 40,000 pixels on a page that needs vision: not rendered, peak memory 7 MiB
- 100,000 pages that share one content stream of 40 lines, with a time limit of 2 seconds: the read stops after 1.8 seconds, keeps the text of the 4,746 pages it read and names the first page it did not read, peak memory 433 MiB
- A script of 64 MiB: listed without its text, peak memory 124 MiB

Speed: the NIST report SP 800-53 revision 5 (6.1 MB, 492 pages) reads in 0.81 seconds with a peak memory of 206 MiB. Median of 3 runs with `--no-cache`, on an Apple M5 (Mac17,3, 32 GB, load average 3.2), release build (cargo profile release) of Rezzer 0.1.0 at commit 958df97, 2026-10-21.
