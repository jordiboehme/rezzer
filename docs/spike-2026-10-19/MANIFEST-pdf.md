# PDF test corpus

Collected 2026-10-19 for the Rezzer crate spike. All files are untrusted data: never
execute them, keep scripts outside this folder.

Source trees (commit at download time):

- pdf.js: https://github.com/mozilla/pdf.js, `test/pdfs/`, commit 33734f1fdf1b8987c656c0a5f67b4e2cc57e9d13.
  Repository license Apache-2.0; the annotation files were made for the pdf.js test suite
  by Tim van der Meij (author field in the files).
- py-pdf sample-files: https://github.com/py-pdf/sample-files, commit 02b8203bf416db3dbdb03438bc8278dfcb0555f3.
  README: "Every PDF in this repository is under the Creative Commons
  Attribution-ShareAlike License (CC-BY-SA-4.0)".
- qpdf: https://github.com/qpdf/qpdf, `qpdf/qtest/qpdf/`, commit 4eba95899886e851cc41d76886483b347612f2a8.
  Repository license Apache-2.0.

| File | Source URL | License | Tests | SHA-256 |
|---|---|---|---|---|
| arxiv-2408.09869-docling-report.pdf | https://arxiv.org/pdf/2408.09869v5 | CC-BY-4.0 (arXiv OAI record) | Real paper: tables, figures, outline (10), 54 internal Link annotations (GoTo named dests), untagged, LaTeX | 82dd470712ce8389f19f20eb9330475e2166a281f8c7990a9f1d0763d73b4d22 |
| irs-f1040-2025.pdf | https://www.irs.gov/pub/irs-pdf/f1040.pdf (2025 form, downloaded 2026-10-19; URL content changes yearly) | US government work, public domain | Tagged PDF (StructTreeRoot, /Lang en-US), XMP, AcroForm (229 fields, 199 widgets, Tx and Btn), form layout tables | 3d31c226df0d189ced80e039d01cf0f8820c1019681a0f0ca6264de277b7e982 |
| nist-sp800-53r5.pdf | https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-53r5.pdf | US government work (NIST), public domain | Largest file (6 MB, 492 pages): speed, tagged, XMP, outline (358), 7692 Link annotations (URI and internal), many tables, Info keywords | fc63bcd61715d0181dd8e85998b1e6201ae3515fc6626102101cab1841e11ec6 |
| pdfjs-annotation-caret-ink.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/annotation-caret-ink.pdf | Apache-2.0 (pdf.js repo) | Caret and Ink annotations with /T, /M, /CreationDate, /C, Popup; tagged (Word 2013) | afdc6fb72e8dcb8f0796a48388fc011e9db212e18d1d7b8cf95b327e88cf6ce9 |
| pdfjs-annotation-freetext.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/annotation-freetext.pdf | Apache-2.0 (pdf.js repo) | FreeText annotations with author and dates | 318ed4bead93fa6ff023571b17ddca41e82fe2347b497ea24903a11e1892fb71 |
| pdfjs-annotation-highlight.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/annotation-highlight.pdf | Apache-2.0 (pdf.js repo) | Highlight with /T, /M, /CreationDate, /C, /QuadPoints, Popup; highlighted words "Lorem ipsum dolor sit amet" (to check) | 72b6387356ccfdfb95a0a5eff5e1ceb9b7667019807d7b04abaf64dad7ea143a |
| pdfjs-annotation-link-text-popup.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/annotation-link-text-popup.pdf | Apache-2.0 (pdf.js repo) | Text (sticky note) with Popup, URI Link | 06917202feda6b6bb8b57b048655f946f749bdff5a8bfcb35eb0116f38910312 |
| pdfjs-annotation-strikeout.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/annotation-strikeout.pdf | Apache-2.0 (pdf.js repo) | StrikeOut with QuadPoints, author, dates, Popup | ca7225dbdc6f8a26814cf0d67f5c509589e2cc09ace6bdb04a375a266db5b708 |
| pdfjs-annotation-underline.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/annotation-underline.pdf | Apache-2.0 (pdf.js repo) | Underline with QuadPoints, author, dates, Popup; slightly broken xref (object 5 offset) | ef04b7049d5cab49c702887c423f20cfecf76d36905772209cc6781425d8f671 |
| pdfjs-structure-simple.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/structure_simple.pdf | Apache-2.0 (pdf.js repo) | Minimal tagged PDF from Chrome/Skia: H1, P, H2, P roles | 5b5510bb5f91c86aa0bbd4882842d4583c814e0e69a0cfbff0e70dc250a8d0f3 |
| pypdf-annotated.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/024-annotations/annotated_pdf.pdf | CC-BY-SA-4.0 | Text, Highlight (with /M and QuadPoints, no author), Ink (with /T "Lucas"); highlight covers "Line 1 Line 2" | c327f921abfba23a5c42d5c429ba99ded1cf5511521003aba6d2aff9c940d9cc |
| pypdf-arabic-habibi.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/015-arabic/habibi.pdf | CC-BY-SA-4.0 | Non-Latin script (Arabic, right to left, with harakat) | 1017c4559eb7d0ccf7d151a3f051c8c1da27a7c1dc8050b2b687e3d3228e1b6f |
| pypdf-attachment.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/025-attachment/with-attachment.pdf | CC-BY-SA-4.0 | Embedded file (EmbeddedFiles name tree: image.png) | 1c7e5f3bb3bbf9a2424cac84f46b17e5b62af0719d63b445c280c76634b316ba |
| pypdf-libreoffice-form.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/012-libreoffice-form/libreoffice-form.pdf | CC-BY-SA-4.0 | AcroForm: 8 fields (text with value "Alice", check boxes, radio, list) | 9105eeef8c8cafdb141b7edd768a5e08adffe320d1d4f89e1a7112a2b37d1c57 |
| pypdf-multicolumn.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/026-latex-multicolumn/multicolumn.pdf | CC-BY-SA-4.0 | Untagged two-column LaTeX document: reading order | bdb495e95b3e1afae95013099dc59b0cea047f1fa70f677ee9cb33f10faa1c6c |
| pypdf-outline.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/006-pdflatex-outline/pdflatex-outline.pdf | CC-BY-SA-4.0 | Outline (9 items) and internal Link annotations with named destinations | 17b5a4dac75613b82749c7538fc93991a385a5d419cc9832fdba24c1726a031a |
| pypdf-xmp.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/020-xmp/output_with_metadata_pymupdf.pdf | CC-BY-SA-4.0 | XMP metadata stream, odd Info date | 8e52052b0d7242a6d3df844e0f24ed3c09ca36440761d1b00f85bbabc15f7369 |
| qpdf-aes128-empty-user-password.pdf | https://raw.githubusercontent.com/qpdf/qpdf/main/qpdf/qtest/qpdf/V4-aes.pdf | Apache-2.0 (qpdf repo) | Encrypted, V4 AES-128, empty user password | 1d8a812844b8d8c7ba085a627eb3b7ab52aa651c6143ca07315084220a113240 |
| qpdf-aes256-empty-user-password.pdf | https://raw.githubusercontent.com/qpdf/qpdf/main/qpdf/qtest/qpdf/enc-XI-R6,V5,O=master.pdf | Apache-2.0 (qpdf repo) | Encrypted, V5 R6 AES-256, empty user password; also page labels, outline (11), XMP | 63c99503fc9c6b44c47df124ce040d005e26d2c5d8d0999ad59c48b31a426863 |
| qpdf-malformed-bad-xref-entry.pdf | https://raw.githubusercontent.com/qpdf/qpdf/main/qpdf/qtest/qpdf/bad-xref-entry.pdf | Apache-2.0 (qpdf repo) | Robustness: broken xref entry (text "Potato") | 1d3b6ed9362c92c3f24f6ee7650e1768387204b1bbb66327a4eba226ce801bb4 |
| qpdf-malformed-damaged-stream.pdf | https://raw.githubusercontent.com/qpdf/qpdf/main/qpdf/qtest/qpdf/damaged-stream.pdf | Apache-2.0 (qpdf repo) | Robustness: damaged content stream | 91e119f2db2e9f738f696ddacea90498d460b0afda0fcdf71bb6281fc5085819 |
| qpdf-page-labels-and-outlines.pdf | https://raw.githubusercontent.com/qpdf/qpdf/main/qpdf/qtest/qpdf/page-labels-and-outlines.pdf | Apache-2.0 (qpdf repo) | Page labels (roman, arabic, prefixes over 30 pages), nested outline (11) | 08b0f999c02abb75cae0854eb042f4ed75ad0b65017a0a584276b90d5e8194b7 |
| scan-truman-letter-nara.pdf | https://commons.wikimedia.org/wiki/File:Letter_from_Harry_S._Truman_to_Eleanor_Roosevelt,_accompanied_by_a_copy_of_a_letter_from_Harry_S._Truman_to_Henry_Morgenthau,_Jr._-_DPLA_-_6abe2984cf4a9f4e9286e8f17f122421.pdf | Public domain (Wikimedia Commons; NARA via DPLA) | Scanned typewritten letter, image only, no fonts, no text layer: OCR needed | a775d592b0780a097bd41a3738b9ce369a98cc2eabf4831a17251beb9f0cb7fd |

## Hidden logic files (added 2026-10-19)

Additional source tree: Apache PDFBox, https://github.com/apache/pdfbox, commit
ccb7498b942d99d7644f4c1d14906b646f814d84, repository license Apache-2.0. PDFBox test files
often come from issue attachments; the two used here are a US government form (public domain)
and a test file made for PDFBox issue 3110.

| File | Source URL | License | Tests | SHA-256 |
|---|---|---|---|---|
| pdfjs-acroform-calculation-order.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/acroform_calculation_order.pdf | Apache-2.0 (pdf.js repo) | AcroForm `/CO` calculation order | 9081aad78cb6bb1871eb5c51addbe75ce73e4617a3e1fa15aa3da21232b6b755 |
| pdfjs-text-field-calc.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/text_field_own_canvas_calc.pdf | Apache-2.0 (pdf.js repo) | `/CO` plus a calculate action (`/AA /C`): field Mirror = getField("Source"); Mirror is ReadOnly | b307ec462e469e7fdc7acc2630641f41ef9da4dadf8dd5c736720c60ee19248c |
| pdfjs-js-buttons.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/js-buttons.pdf | Apache-2.0 (pdf.js repo) | Document-level JavaScript (Names/JavaScript) and button actions | 4679ee03ee84b48d8b67ba39975e946de49216ba744a729fa30ef8f32c3ea9ba |
| pdfjs-dates-afdate.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/dates.pdf | Apache-2.0 (pdf.js repo) | Format and keystroke actions AFDate_FormatEx, AFTime_Format | 7bf9b6c569b52adf0c73ff608aa0353d7e8dd22e9cdb97eed95b38bba2a66027 |
| pdfbox-sf1199a-direct-deposit.pdf | https://raw.githubusercontent.com/apache/pdfbox/trunk/pdfbox/src/test/resources/org/apache/pdfbox/pdmodel/interactive/form/PDFBOX-3656-SF1199AEG%20(Complete).pdf | US government form (Standard Form 1199A, Treasury), public domain; distributed in PDFBox repo (Apache-2.0) | AFNumber_Format with currency "$" and 2 decimals, AFSpecial_Format (phone), AFDate_FormatEx; 3 incremental saves | 61d5dd9542b40b7dcff2e23f217e12d7c930ca2fb7a62cfdd4177711f5b47567 |
| pdfbox-poems-article-threads.pdf | https://raw.githubusercontent.com/apache/pdfbox/trunk/pdfbox/src/test/resources/input/PDFBOX-3110-poems-beads.pdf | Apache-2.0 (PDFBox repo test resource; poems by Goethe and others, public domain text) | Article threads (`/Threads`, beads) with non-ASCII titles; 2 incremental saves | 9d715ce4e964f6ccbf50e1d1c42ca5f01425a079a5c39a9581c876c56afca1b3 |
| pypdf-ocg-layers.pdf | https://raw.githubusercontent.com/py-pdf/sample-files/main/029-optional-content-groups/ocg_map_test.pdf | CC-BY-SA-4.0 | Optional content groups (6 named layers of a map) | 0d2b6fbcc9e58bda84d4f910d63499451b1825841572ef35616e24ab27f20182 |
| pdfjs-signed.pdf | https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/signed_verified.pdf | Apache-2.0 (pdf.js repo) | Digital signature field (adbe.pkcs7.detached, reason, date, ByteRange covering whole file) | 1fe9051d9ac1f39828988040db471916251dc2953c1a32efd6596e4975da848c |
| qpdf-incremental-updates.pdf | https://raw.githubusercontent.com/qpdf/qpdf/main/qpdf/qtest/qpdf/incremental-1.pdf | Apache-2.0 (qpdf repo) | 3 revisions (incremental updates); last one adds a content stream to page 1 | 5f9c43d3f201d8e9ea77aa162bf4eb527c563f9db3626a736455e003cf5baaf3 |
| pdfbox-actualtext-din91379.pdf | https://raw.githubusercontent.com/apache/pdfbox/trunk/pdfbox-layout-awt/src/test/resources/pdf/GlyphLayoutDIN91379Form_ActualText.pdf | Apache-2.0 (PDFBox repo, made with LibreOffice for the PDFBox test suite) | Tagged, /Lang de-DE, about 1900 `/ActualText` entries on marked content, DIN 91379 Latin characters | b2ef9c9cb857bfce8f3c68cdaf206f9dbc0b6cb14516d193fd00231d17e73d03 |

Other hidden logic found in files listed above: irs-f1040-2025.pdf has 3 saves, document
JavaScript (Adobe version check) and a usage-rights signature (ByteRange, not a form field);
nist-sp800-53r5.pdf has 36 `/Alt` texts; irs-f1040-2025.pdf has table `Headers`, `ColSpan`,
`RowSpan` attributes; arxiv and pypdf-outline have named destinations.

Not found as a redistributable file in time: `AFSimple_Calculate` (sum/product of fields),
`AFPercent_Format`, a field with the Required flag, annotation replies (`/IRT`) and `/Scope`
on table headers. pdfjs-text-field-calc.pdf covers a custom calculate script instead.

Ground truth (dumped with pypdf 6.14.2 as reference, outside this folder) is in
`spike/reports/pdf-ground-truth.txt`.

Known gap: no redistributable file with annotation replies (`/IRT`) was found in time.
pdf.js `comments.pdf` has popups but no `/IRT` and is based on a paper with unclear license.
