# Web and text corpus (spike, 2026-10-19)

All files are public and allow redistribution. Downloaded files are untrusted data: no file in
this folder may be executed. Scripts that built the derived files live outside this folder in
`spike/web-text-test/scripts/make_derived.py` (run with `python3 -I`).

"Derived" means the spike changed a public file (transcoding, cutting, wrapping). A derived
file keeps the license of its source.

## HTML

| File | Source | License | Tests |
|---|---|---|---|
| html-wikipedia-pythagorean-theorem.html | https://en.wikipedia.org/wiki/Pythagorean_theorem | CC BY-SA 4.0 (Wikipedia contributors) | MathML (99 `<math>` with TeX alttext), figure/figcaption, abbr, JSON-LD Article, OG, canonical, nav landmarks |
| html-wikipedia-periodic-table.html | https://en.wikipedia.org/wiki/Periodic_table | CC BY-SA 4.0 | big tables with colspan/rowspan and caption, MathML, figures, data-* attributes in tables |
| html-wikipedia-nobel-physics.html | https://en.wikipedia.org/wiki/List_of_Nobel_laureates_in_Physics | CC BY-SA 4.0 | 256 rowspan and 130 colspan cells, data-sort-value, link graph |
| html-mdn-details-element.html | https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/details | CC BY-SA 2.5 (prose), CC0 (code samples) | details/summary (11), time datetime, aside, nav, OG |
| html-govuk-guidance.html | https://www.gov.uk/guidance/the-highway-code | Open Government Licence v3.0 | JSON-LD Article and BreadcrumbList, OG, govuk:* meta with dates, main, forms |
| html-whatwg-tables.html | https://html.spec.whatwg.org/multipage/tables.html | CC BY 4.0 (WHATWG) | example tables with caption, rowspan, colspan, thead/tbody, dense in-page anchors |
| html-w3c-blog-post.html | https://www.w3.org/blog/2026/crafting-wcag-3-for-more-accessible-user-experiences/ | W3C Document License | article, time datetime, abbr title, OG and Twitter cards |
| html-schemaorg-microdata-blogposting.html | derived: MICRODATA block of example #eg-0476 in https://github.com/schemaorg/schemaorg/blob/main/data/sdo-blogpost-examples.txt, wrapped in a minimal page | schema.org examples CC BY-SA 3.0 (repo Apache-2.0) | microdata (BlogPosting, Person, Comment, Organization), nested itemscope, time datetime, abbr, article/section/header/footer |
| html-w3c-wai-forms-validation.html | https://www.w3.org/WAI/tutorials/forms/validation/ | W3C Software and Document License | forms: required, pattern, types email/url/number/range/date/time, min/max, labels |
| html-w3c-wai-forms-grouping.html | https://www.w3.org/WAI/tutorials/forms/grouping/ | W3C Software and Document License | forms: fieldset/legend groups, radio groups, select with options and optgroup |

## EPUB

| File | Source | License | Tests |
|---|---|---|---|
| epub2-gutenberg-alice.epub | https://www.gutenberg.org/ebooks/11.epub.noimages | Public domain in the USA (Project Gutenberg license text inside) | EPUB 2, NCX TOC, opf:file-as, opf:event dates, subjects |
| epub3-standardebooks-hound-of-the-baskervilles.epub | https://standardebooks.org/ebooks/arthur-conan-doyle/the-hound-of-the-baskervilles | CC0 1.0 (Standard Ebooks), text public domain | EPUB 3, belongs-to-collection with collection-type series and group-position 5, MARC relator roles, accessibility metadata, nav + NCX |
| epub3-standardebooks-meditations.epub | https://standardebooks.org/ebooks/marcus-aurelius/meditations/george-long | CC0 1.0, text public domain | 122 endnotes (epub:type noteref/endnote, backlinks across files), translator role (trl), set collection |
| epub3-idpf-wasteland.epub | https://github.com/IDPF/epub3-samples/releases/download/20230704/wasteland.epub | CC BY-SA 3.0 (IDPF samples default) | in-document notes via epub:type, single spine item, nav + NCX |
| epub3-idpf-epub30-spec.epub | https://github.com/IDPF/epub3-samples/releases/download/20230704/epub30-spec.epub | CC BY-SA 3.0 (IDPF samples default, per README) | nav TOC with 316 entries and depth 7, non-linear spine items, dense cross-document links |
| epub3-idpf-linear-algebra.epub | https://github.com/IDPF/epub3-samples/releases/download/20230704/linear-algebra.epub | GNU FDL 1.2 (stated in the OPF). Redistribution must include the license text | MathML-heavy content, 94 spine items, nested TOC |

## PostScript and EPS

| File | Source | License | Tests |
|---|---|---|---|
| ps-rfc1341-mime-ditroff.ps | ftp://ftp.rfc-editor.org/in-notes/rfc1341.ps | RFC: "Distribution of this memo is unlimited" (IETF/ISOC copying terms) | ditroff output, DSC 1.0 with Title, Creator, CreationDate, 70 pages |
| ps-rfc1950-zlib-dvips.ps | ftp://ftp.rfc-editor.org/in-notes/rfc1950.ps | RFC, unlimited distribution | dvips (TeX) with Type 1 fonts, DSC 2.0 Pages, BoundingBox, DocumentFonts, ligatures |
| ps-rfc2490-msword.ps | ftp://ftp.rfc-editor.org/in-notes/rfc2490.ps | RFC, ISOC copyright notice that allows copies and derivative works | Word/Windows PSCRIPT driver, DSC 3.0, `%%Pages: (atend)` resolved from the trailer, LanguageLevel 2, glyph-by-glyph text |
| eps-ctan-mwe-example-image-a.eps | https://mirrors.ctan.org/macros/latex/contrib/mwe/example-image-a.eps | LPPL 1.3 (CTAN package mwe) | EPS from dvips, BoundingBox, CreationDate, a single glyph of text |

Not covered: no file has `%%For`, and no EPS has a DOS binary header with a TIFF/WMF preview.

## CSV and TSV

| File | Source | License | Tests |
|---|---|---|---|
| csv-spectrum-comma_in_quotes.csv, csv-spectrum-escaped_quotes.csv, csv-spectrum-quotes_and_newlines.csv, csv-spectrum-newlines_crlf.csv, csv-spectrum-empty.csv, csv-spectrum-json.csv | https://github.com/max-mapper/csv-spectrum/tree/master/csvs | BSD-2-Clause (package.json; not MIT) | quoting, escaped quotes, newlines inside quotes, CRLF, empty fields, JSON in a field |
| csv-pandas-fdic-banklist.csv | https://github.com/pandas-dev/pandas/blob/main/pandas/tests/io/data/csv/banklist.csv | BSD-3-Clause (pandas); data from the US FDIC, public domain | real CSV, quoted commas, dates like 31-May-13, unique id column CERT |
| csv-shiftjis-pandas-sauron.csv | https://github.com/pandas-dev/pandas/blob/main/pandas/tests/io/parser/data/sauron.SHIFT_JIS.csv | BSD-3-Clause (pandas); text looks taken from Japanese Wikipedia (CC BY-SA) | Shift_JIS CSV |
| tsv-utf16le-bom-pandas-elections.txt | https://github.com/pandas-dev/pandas/blob/main/pandas/tests/io/parser/data/utf16_ex.txt | BSD-3-Clause via pandas; upstream provenance unclear (election candidate list) | UTF-16LE with BOM, TSV, CRLF, missing values |
| csv-semicolon-latin1-dwd-stations.txt | https://opendata.dwd.de/climate_environment/CDC/observations_germany/climate/multi_annual/mean_91-20/Temperatur_1991-2020_Stationsliste.txt | CC BY 4.0 (Deutscher Wetterdienst) | semicolon, ISO-8859-1, CRLF, padded fields, trailing delimiter (empty last column) |
| tsv-noheader-geonames-admin1.txt | https://download.geonames.org/export/dump/admin1CodesASCII.txt | CC BY 4.0 (GeoNames) | header-less TSV, UTF-8, key-like columns |
| csv-datahub-co2-fossil-global.csv | https://github.com/datasets/co2-fossil-global/blob/main/data/global.csv | ODC-PDDL-1.0 | numeric columns where Total is (mostly) the sum of the fuel columns, many missing values, units only in the datapackage.json |
| csv-latin1-noheader-derived-geonames.csv | derived from admin1CodesASCII.txt (rows AT, CH, DE, ES, FR, comma separated, Latin-1) | CC BY 4.0 (GeoNames) | header-less CSV in Latin-1 with few non-ASCII bytes (hard for detection) |
| csv-broken-derived-banklist.csv | derived from the first 12 lines of banklist.csv | BSD-3-Clause / public domain | ragged rows (too many and too few fields) and an unterminated quote |

## Plain text

| File | Source | License | Tests |
|---|---|---|---|
| text-utf8-gutenberg-faust.txt | https://www.gutenberg.org/cache/epub/2229/pg2229.txt | Public domain in the USA (Project Gutenberg) | UTF-8 without BOM, CRLF, German |
| text-latin1-gutenberg-verwandlung.txt | https://www.gutenberg.org/files/22367/22367-8.txt | Public domain in the USA | ISO-8859-1, CRLF, German |
| text-cp1252-derived-gutenberg-alice.txt | derived: https://www.gutenberg.org/cache/epub/11/pg11.txt transcoded from UTF-8 to Windows-1252 | Public domain in the USA | Windows-1252 with curly quotes in 0x80-0x9F, English |
| text-shiftjis-aozora-botchan.txt | https://www.aozora.gr.jp/cards/000148/files/752_ruby_2438.zip (bocchan.txt) | Public domain (Aozora Bunko) | Shift_JIS, Japanese, Aozora ruby notation, `file` misdetects it as "OpenPGP Secret Key" |

## Markdown

| File | Source | License | Tests |
|---|---|---|---|
| md-yaml-frontmatter-github-docs-about-readmes.md | https://github.com/github/docs/blob/main/content/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-readmes.md | CC BY 4.0 (GitHub docs content) | YAML frontmatter with lists and maps (redirect_from, versions), Liquid tags |
| md-toml-frontmatter-rust-blog-traits.md | https://github.com/rust-lang/blog.rust-lang.org/blob/main/content/traits.md | MIT OR Apache-2.0 | TOML frontmatter (`+++`), reference links, code blocks |
| md-wikilinks-foam-docs.md | https://github.com/foambubble/foam/blob/main/docs/user/features/wikilinks.md | MIT (Foam) | `[[wikilinks]]`, section links, wikilinks inside code spans |

## Mail

| File | Source | License | Tests |
|---|---|---|---|
| eml-cpython-msg07-gif-attachment.eml | https://github.com/python/cpython/blob/main/Lib/test/test_email/data/msg_07.txt | PSF License 2.0 | multipart/mixed with a base64 GIF attachment and file name |
| eml-cpython-msg15-alternative-html-latin1.eml | https://github.com/python/cpython/blob/main/Lib/test/test_email/data/msg_15.txt | PSF License 2.0 | multipart/alternative with HTML, ISO-8859-1 quoted-printable, reused boundary |
| eml-cpython-msg43-delivery-report.eml | https://github.com/python/cpython/blob/main/Lib/test/test_email/data/msg_43.txt | PSF License 2.0 | multipart/report, message/delivery-status, text/rfc822-headers |
| eml-rfc2046-multipart-example.eml | https://github.com/stalwartlabs/mail-parser/blob/main/resources/eml/rfc/000.eml (the RFC 2046 section 5.1.1 example) | RFC text (IETF Trust); repo Apache-2.0 OR MIT | multipart/parallel, message/rfc822 nested, parts without Content-Type |
| mbox-derived-rfc5322-thread.mbox | derived: the three messages of RFC 5322 Appendix A.2 (https://www.rfc-editor.org/rfc/rfc5322.txt) plus CPython msg_07.txt, joined as mbox with From_ lines | RFC text (IETF Trust) and PSF License 2.0 | mbox splitting, Message-ID, In-Reply-To, References, Reply-To, thread graph, attachment |
