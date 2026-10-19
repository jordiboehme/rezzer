# Office test corpus (spike)

Collected 2026-10-19 for the Rezzer office-format spike. Every file comes from the test suite
or the examples of an open-source project and is redistributable under that project's
license. Source links are pinned to the commit that was current on the download date:

- Apache POI `test-data/`, commit 04804ac9, Apache-2.0
- Apache Tika `test-documents/`, commit 043762c6, Apache-2.0
- LibreOffice core `*/qa/*/data/`, commit 79be16a7, MPL-2.0
- calamine `tests/`, commit f2c59482, MIT
- IronCalc `xlsx/tests/`, commit e94e6f1a, MIT OR Apache-2.0
- unifloc_vba `examples/`, commit b19a2b07, MIT (petroleum engineering workbooks with VBA)
- underground-tunnel-structural-analysis `excel/`, commit 5063b548, MIT
- docx4j `sample-docs/`, tag VERSION_17_3_3 (c95b55be), Apache-2.0

Some POI and Tika files originally come from the govdocs1 corpus (US government documents,
public domain); the project license above applies to them as distributed.

These files are untrusted data. Never open them in an office suite with macros enabled, never
execute anything inside them (several contain VBA projects), keep all scripts outside this
folder and pass paths as arguments.

The set has 75 files (70 feature files plus 5 in `malformed/`), more than the planned 25-35.
Most public test files cover one feature each, so a checklist this wide (formulas, names,
validation, threaded comments, tracked changes, content controls, sections, alt text, modern
Excel functions, engineering workbooks, document logic) needs many single-purpose files. The
`malformed/` folder holds 5 known-bad inputs for robustness tests.

Known gaps (no suitable public file found in time): an XLSX with hidden columns (ODS hidden
columns and XLSX hidden rows are covered), a workbook with iterative calculation enabled
(`calcPr iterate="1"`), PPTX modern comments (`p188:cm`), a PPTX hidden slide (`show="0"`),
a DOCX with a horizontal merge (`gridSpan`), a Word table formula field (`=SUM(ABOVE)`),
document variables (`w:docVar`), a percent number format in a custom `numFmt` (built-in
format ids 9 and 10 are not listed in styles.xml).

## Spreadsheets

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `Formatting.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/Formatting.xlsx) | `5a0aa7c36c53b22c` | Custom number formats: dates (yyyy/mm/dd, d/m/yy), decimals, currency (GBP #,##0.00) |
| `55906-MultiSheetRefs.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/55906-MultiSheetRefs.xlsx) | `7a5cb4687ad8835a` | 13 cross-sheet and 3D formulas (SUM(Sheet1:Sheet3!A1) style) |
| `MatrixFormulaEvalTestData.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/MatrixFormulaEvalTestData.xlsx) | `12f1dcef889c20fe` | 32 array (CSE) formulas, 32 merged cells |
| `simple-table-named-range.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/simple-table-named-range.xlsx) | `e0ee2fee568358ed` | Excel table (ListObject) plus defined name using a structured reference (SUM(Table1[c])) |
| `testNames.xlsm` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/testNames.xlsm) | `2283c7bbffb916e9` | 5 defined names (constant, array constant, cell, range, OFFSET formula) plus a VBA project |
| `DataValidations-49244.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/DataValidations-49244.xlsx) | `8066b12c878d74aa` | 52 data validation rules incl. list type, input and error messages; merged cells |
| `SimpleWithComments.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/SimpleWithComments.xlsx) | `d7d337b3436e8634` | Legacy cell notes (comments with author, no date) |
| `threadedComment.xlsx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sc/qa/unit/data/xlsx/threadedComment.xlsx) | `47f1813c00f19e88` | Threaded comments with person and timestamp (dT) plus the legacy fallback note |
| `hidden_sheets.xlsx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sc/qa/unit/data/xlsx/hidden_sheets.xlsx) | `837080e7711525b6` | Hidden and very hidden sheets |
| `GroupTest.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/GroupTest.xlsx) | `a14e6d4780047204` | Hidden rows with outline levels (row grouping, collapsed) |
| `merged_range.xlsx` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/merged_range.xlsx) | `13553289594edda3` | 15 merged ranges |
| `table-multiple.xlsx` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/table-multiple.xlsx) | `8d39e98b17bcd293` | 4 Excel tables (ListObjects) with header names |
| `WithConditionalFormatting.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/WithConditionalFormatting.xlsx) | `ead4aad13d2b975f` | 3 conditional formatting rules |
| `pivots.xlsx` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/pivots.xlsx) | `632407ef8f5a5b45` | 2 pivot tables with worksheet cache sources; 18 shared formulas |
| `WithChart.xlsx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/WithChart.xlsx) | `51f27aabf7417f5a` | One chart with series ranges (c:f Sheet1!$A$1:$A$6) |
| `external-refs.xlsx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sc/qa/unit/data/xlsx/external-refs.xlsx) | `80cd967dad7b889a` | External workbook link (externalLink part) used by 4 formulas |
| `testEXCEL_custom_props.xlsx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testEXCEL_custom_props.xlsx) | `0509234592dacacb` | Core, app and custom document properties (5 custom) |
| `testEXCEL_various.xlsb` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testEXCEL_various.xlsb) | `8c600e97d719b026` | Binary XLSB workbook with values, formulas, a chart and a VBA project |
| `testEXCEL_custom_props.xls` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testEXCEL_custom_props.xls) | `fff0cbf96e178190` | Legacy BIFF8 XLS with custom properties (OLE property sets) |
| `3dFormulas.xls` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/spreadsheet/3dFormulas.xls) | `97a6c32bb985d75c` | Legacy BIFF8 XLS with 3D (cross-sheet) formulas |
| `any_sheets.ods` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/any_sheets.ods) | `1aef869b5e34ad73` | ODS with visible, hidden and very hidden sheets plus a sheet named Chart |
| `with-annotation.ods` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/with-annotation.ods) | `631d930dcef61a93` | ODS cell annotation with author and date |
| `formula-across-sheets.ods` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sc/qa/unit/data/ods/formula-across-sheets.ods) | `e63f058d75938fc5` | ODS with 23 formulas incl. references across 3 sheets |

## Word processing

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `testWORD_various.docx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testWORD_various.docx) | `f7761e3893d795ed` | Mixed DOCX: 52 styled paragraphs (headings, title), lists, footnotes, header, hyperlink, field, custom properties |
| `testComment.docx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testComment.docx) | `740337ba9a0f0c9d` | One comment with author and date |
| `groupshape-trackedchanges.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/groupshape-trackedchanges.docx) | `f4b2445f98cfa979` | Tracked changes: 2 insertions and 2 deletions with author and date |
| `endnotes.docx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/document/endnotes.docx) | `6e2e1b2dabb0ae6b` | Footnotes (4) and endnotes (5) |
| `testWORD_refFields.docx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testWORD_refFields.docx) | `9bebf723b3c9f8ee` | REF and PAGEREF fields (cross references) to a bookmark, hyperlink |
| `simple-sdts.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/simple-sdts.docx) | `9a20342ba44d81a0` | 6 content controls (w:sdt) of several kinds |
| `checkbox-control.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/checkbox-control.docx) | `d7d0314cf914d33a` | Checkbox content control (w14:checkbox) |
| `tdf147646_mergedCellNumbering.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/tdf147646_mergedCellNumbering.docx) | `0ad71290e3392b82` | Table with vertically merged cells (vMerge) and numbered paragraphs |
| `testWORD_omml.docx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testWORD_omml.docx) | `3bbb21e239accfbf` | 6 Office Math (OMML) equations |
| `testWORD_diagramData.docx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testWORD_diagramData.docx) | `c4c3f15c28df87ff` | SmartArt diagram (dgm data model) |
| `testWORD_1img.docx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testWORD_1img.docx) | `d9dbb66913b6af41` | Inline image with alt text (wp:docPr descr) |
| `caption.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/caption.docx) | `df1448c03fdaa41b` | Paragraph in Caption style below a figure |
| `deep-table-cell.docx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/document/deep-table-cell.docx) | `31c5bfe1f68c9869` | Robustness and stack depth: 5000 nested tables |
| `testWORD_various.doc` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testWORD_various.doc) | `87a69bd33b7d6134` | Legacy Word 97 binary (same content family as testWORD_various.docx) |
| `table-merges.doc` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/document/table-merges.doc) | `bceea684bbf492fa` | Legacy DOC table with merged cells |
| `testRTFVarious.rtf` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testRTFVarious.rtf) | `cf00c56fe7099d14` | RTF with 3 table rows, footnote, HYPERLINK field, list overrides, info block (metadata) |
| `testComment.rtf` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testComment.rtf) | `742ec05361e5c9bf` | RTF annotation (comment) with author and date |
| `testODFwithOOo3.odt` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-miscoffice-module/src/test/resources/test-documents/testODFwithOOo3.odt) | `b6cd74b570a98fcb` | ODT with annotation, 3 tracked changes (changed-region), a note, heading, table |
| `comment_with_children.odt` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/comment_with_children.odt) | `564f5f1dfe5fa671` | ODT with 4 annotations incl. replies (comment thread) |

## Presentations

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `testPPT_various.pptx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testPPT_various.pptx) | `73e19e877797b9de` | 7 slides with titles, bullet levels, a table, speaker notes |
| `testPPT_comment.pptx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testPPT_comment.pptx) | `d06691124f534922` | Legacy slide comment (p:cm) with author and date |
| `testPPT_charts.pptx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testPPT_charts.pptx) | `a6763b16b4bdc367` | Embedded chart with series references |
| `smartart-simple.pptx` | Apache-2.0 (Apache POI test-data) | [source](https://github.com/apache/poi/blob/04804ac97487ceded07a8231bc106a258109cbeb/test-data/slideshow/smartart-simple.pptx) | `238ff6433dac00b2` | SmartArt diagram |
| `testPPT_2imgs.pptx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testPPT_2imgs.pptx) | `a5ec2ad60eebe2d7` | Two pictures with alt text (descr), custom property |
| `slide-sections.pptx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sd/qa/unit/data/pptx/slide-sections.pptx) | `2e2d1a6cfa6c282a` | Slide sections (p14:section) and speaker notes on 13 slides |
| `testPPT_various.ppt` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testPPT_various.ppt) | `54790b849e108dca` | Legacy PowerPoint 97 binary with notes and table |
| `LibreOfficeImpress_odp_1.3.odp` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-miscoffice-module/src/test/resources/test-documents/versions/LibreOfficeImpress_odp_1.3.odp) | `ecc8129a5b22c77b` | ODP 1.3 with 2 slides and 4 notes pages (the notes pages are empty) |

## Malformed and robustness

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `malformed/issue_666_panic.xlsb` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/issue_666_panic.xlsb) | `466bbc38d665e16b` | XLSB that made calamine panic (regression file) |
| `malformed/issue_594_dos.ods` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/issue_594_dos.ods) | `dc43c3a709f20e4c` | ODS that caused a denial of service (huge repeat counts) |
| `malformed/OOM_alloc.xls` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/OOM_alloc.xls) | `7f09c2a732fd54b6` | XLS that triggered an out-of-memory allocation |
| `malformed/too_small.xls` | MIT (calamine tests) | [source](https://github.com/tafia/calamine/blob/f2c5948242fdfe390e292313a1f87f9db19b9633/tests/too_small.xls) | `33570d24a4fecd14` | Truncated XLS (68 bytes) |
| `malformed/testODTnotaZipFile.odt` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-miscoffice-module/src/test/resources/test-documents/testODTnotaZipFile.odt) | `4b7cbd32a2514750` | File named .odt that is not a ZIP |

## Engineering and modern formulas

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `tunnel_design_calculations.xlsx` | MIT (underground-tunnel-structural-analysis) | [source](https://github.com/alokpalai/underground-tunnel-structural-analysis/blob/5063b5488ea2b983daeb7b4200d2fbdab0dcac6b/excel/tunnel_design_calculations.xlsx) | `efab83e86b0294f6` | Tunnel structural calculation: 86 formulas in chains across 4 sheets, units in header and value text (10.0 m), PI/TAN/RADIANS |
| `ex051.MF_pipeline.xlsx` | MIT (unifloc_vba examples) | [source](https://github.com/unifloc/unifloc_vba/blob/b19a2b0715d718ecbbd51a0c84ccda611214dfc6/examples/ex051.MF_pipeline.xlsx) | `6305a34b4dc68366` | Pipeline flow calculation: 14 defined names used as variables, add-in user-defined functions via external link ([1]!encode_pipe_diam) |
| `ex054.MF_friction_factor.xlsm` | MIT (unifloc_vba examples) | [source](https://github.com/unifloc/unifloc_vba/blob/b19a2b0715d718ecbbd51a0c84ccda611214dfc6/examples/ex054.MF_friction_factor.xlsm) | `d634997c5ddb0062` | Pipe friction factor: VBA project with user-defined functions used in 151 formulas, 115 array formulas, 25 shared formulas |
| `ex012.PVT_z_factor.xlsm` | MIT (unifloc_vba examples) | [source](https://github.com/unifloc/unifloc_vba/blob/b19a2b0715d718ecbbd51a0c84ccda611214dfc6/examples/ex012.PVT_z_factor.xlsm) | `7bd2d64555c861c2` | Gas z-factor (PVT) workbook: 2133 formulas, 2016 copied array/dynamic formulas over hundreds of rows, VBA, external link |
| `CONVERT.xlsx` | MIT OR Apache-2.0 (IronCalc tests) | [source](https://github.com/ironcalc/IronCalc/blob/e94e6f1aed1e9a4709f038717ca73d9d82e63ab5/xlsx/tests/calc_tests/ENGINEERING/CONVERT.xlsx) | `11494892e2933725` | Unit conversion: 509 formulas (455 shared) using CONVERT with unit names |
| `XLOOKUP.xlsx` | MIT OR Apache-2.0 (IronCalc tests) | [source](https://github.com/ironcalc/IronCalc/blob/e94e6f1aed1e9a4709f038717ca73d9d82e63ab5/xlsx/tests/calc_tests/LOOKUP_AND_REFERENCE/XLOOKUP.xlsx) | `10af43589194e722` | 223 XLOOKUP formulas (_xlfn prefix), 242 shared formulas |
| `LET.xlsx` | MIT OR Apache-2.0 (IronCalc tests) | [source](https://github.com/ironcalc/IronCalc/blob/e94e6f1aed1e9a4709f038717ca73d9d82e63ab5/xlsx/tests/calc_tests/LOGICAL/LET.xlsx) | `47fc0f71b5a425e9` | LET and nested LAMBDA formulas |
| `LAMBDA.xlsx` | MIT OR Apache-2.0 (IronCalc tests) | [source](https://github.com/ironcalc/IronCalc/blob/e94e6f1aed1e9a4709f038717ca73d9d82e63ab5/xlsx/tests/calc_tests/LOGICAL/LAMBDA.xlsx) | `6fda1c171601204e` | LAMBDA in cells and as defined names (CountWords, Temperature with comment), dynamic array cells (cm) |
| `DynamicArrays.xlsx` | MIT OR Apache-2.0 (IronCalc tests) | [source](https://github.com/ironcalc/IronCalc/blob/e94e6f1aed1e9a4709f038717ca73d9d82e63ab5/xlsx/tests/DynamicArrays.xlsx) | `e0b4fce7adc049f9` | Dynamic array formulas with spill ranges (cm attribute, ANCHORARRAY) |
| `one-variable.xlsx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sc/qa/unit/data/xlsx/data-table/one-variable.xlsx) | `97b284e9800fbf8e` | What-if data tables (TABLE(), t="dataTable", row and column input cells) |

## Word and presentation logic

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `CommentReply.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/CommentReply.docx) | `cfc3a47bb7a51a20` | Comment with a reply (commentsExtended paraIdParent) |
| `CommentDone.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/CommentDone.docx) | `f0c35b661ee60f28` | Resolved comment (w15:done) |
| `mailmerge.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/mailmerge.docx) | `94f76ba6a717d33e` | Mail merge: MERGEFIELD and mailMerge settings with an external data source |
| `embedded-xlsx.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/embedded-xlsx.docx) | `37af4bb83e2ea1a9` | Two embedded Excel worksheets (OLE objects with xlsx payload) |
| `field_numeric_formatting_switch.docx` | Apache-2.0 (docx4j sample docs) | [source](https://github.com/plutext/docx4j/blob/c95b55be6832ff716131ffdbd7006755db92f696/docx4j-samples-docx4j/sample-docs/test-docs/field_numeric_formatting_switch.docx) | `efab53d650524443` | DOCPROPERTY fields that read custom properties, with numeric format switches |
| `relative-link.docx` | MPL-2.0 (LibreOffice core) | [source](https://github.com/LibreOffice/core/blob/79be16a7156fadfad7d4a0be0dbbb38f761f242e/sw/qa/extras/ooxmlexport/data/relative-link.docx) | `79cbc17e61ddbffb` | Link to an external file (relative target) |
| `testPPT_oleWorkbook.pptx` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testPPT_oleWorkbook.pptx) | `9f079067f8b693a2` | Slide with an embedded OLE workbook |

## Legacy comments

| File | License | Source | sha256 (first 16) | What it tests |
|---|---|---|---|---|
| `testComment.doc` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testComment.doc) | `132a7833d2f1686b` | Word 97 binary with one comment (author) |
| `testComment.xls` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testComment.xls) | `0d87908acdca3bb8` | BIFF8 workbook with one cell note (author) |
| `testComment.ppt` | Apache-2.0 (Apache Tika test-documents) | [source](https://github.com/apache/tika/blob/043762c6327ff36b3081ce297688e93915c85996/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testComment.ppt) | `9d1d7001c4e0fbbb` | PowerPoint 97 binary with one slide comment |
