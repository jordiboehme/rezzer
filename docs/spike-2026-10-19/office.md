# Spike report: office formats

Date: 2026-10-19. Throwaway spike, no Rezzer code. Probes in `spike/office-test/`, corpus in
`spike/corpus/office/` (see `MANIFEST-office.md`). Raw probe output was throwaway and is not kept.
Versions were checked today; re-check before relying on them.

Legend for the matrices: **E** exposed in the crate's public API, **P** partial (see note),
**N** not exposed, **-** format not supported by the crate. A crate that only puts something
into Markdown text, with no field in its model, scores **P** at best.

## 1. Recommendation in short

No crate gives Rezzer the knowledge layer it needs. The value tables are solved, the logic
around them is not. Rezzer should combine a few crates for the hard binary and value parts and
read the OOXML/ODF parts that carry intent (comments, revisions, fields, names, validation,
charts, links, calc settings) itself with `zip` + `quick-xml`.

| Format | Use | Read ourselves | Formula analysis |
|---|---|---|---|
| XLSX, XLSM | **calamine** for values, cached results, formulas (shared expanded), tables, merged cells, sheet visibility, VBA module names | names with scope, number format codes, validation, conditional formats, notes, threaded comments with persons and replies, pivot sources, chart series refs, external links, calc settings, data tables, array and spill ranges, hidden rows/cols, custom properties | **formualizer-parse** for references, own R1C1 grouping and dependency graph |
| XLSB, XLS | **calamine** (only real option for formulas in binary formats) | XLS defined names and 3D references need checks (bugs found), custom properties via `cfb` | same |
| ODS | **calamine** for values and formulas (OpenFormula text) | annotations, hidden sheets/rows/cols, validation, styles (spreadsheet-ods has them but blew up on a malformed file) | formualizer-parse with the OpenFormula dialect |
| DOCX | own reader on quick-xml, using **docx-rs** or **office_oxide** only as reference/benchmark | everything: styles to roles, lists, tables, comments with threads and resolved state, tracked changes, fields, bookmarks, content controls, notes, headers/footers, OMML, alt text, embedded objects | - |
| DOC | **office_oxide** (headings, header/footer, notes, merged cells, comments with author; MIT OR Apache-2.0) with **anydoc** as fallback | metadata via `cfb` property sets; comment dates | - |
| PPT | weak everywhere: office_oxide, anydoc and undoc give titles and text only, no notes or comments. Use office_oxide for text and offer LibreOffice (`soffice --convert-to pptx`, optional external program) for full fidelity | metadata via `cfb` | - |
| RTF | **anydoc** (tables, notes, lists, links) | comments and info block (anydoc drops comments) | - |
| ODT | own reader (anydoc mixes annotations and change metadata into body text) | all | - |
| PPTX | own reader; **office_oxide** or **pptx-to-md** as reference | comments with authors, sections, hidden slides, chart refs and data, OLE objects, linked files, alt text | - |
| ODP | own reader (pptx-to-md reads ODP text, but no comments, charts or alt text) | all | - |

Reasons in short:

1. The Markdown-first frameworks (anydoc, undoc, Xberg) drop or flatten exactly what Rezzer
   sells: comments, tracked changes, field logic, formulas. anydoc has the cleanest model but
   it has no slot for comments or revisions, and in ODT it pastes annotation author and date
   into the body text.
2. office_oxide has the richest IR (names with scope, validation, conditional formats, number
   formats, notes, alt text, custom properties), but it accepts tracked changes silently, has
   no ODF or RTF, no formulas for XLS and XLSB, and leaks field codes into text.
3. Reading the OOXML parts ourselves is cheap: one generic element scanner plus extractors for
   every gap above is 236 lines (`probes/p-raw/src/main.rs`), compiles in 3 s and adds about
   0.3 MB. It worked on every OOXML file in the corpus.
4. The two strongest model crates are not the base either. **docx-rs** scores best for the DOCX
   knowledge layer (comment ranges, replies and resolved state, insert/delete with author and
   date, content controls, raw fields), but it failed on two small valid files without
   optional parts, its model has no footnotes or equations, and its default `image` feature
   pulls an image stack. Use it as a reference for the own reader. **umya-spreadsheet** exposes
   validation, conditional formats, pivots, charts, hidden rows and number format codes, but it
   mangles array-constant names, assigns name scope wrongly, has an empty `sheet_state()`,
   costs 4.2 MB, and reads XLSX only. Those parts are small to read directly.
5. formualizer-parse parsed all 3,470 formulas in the corpus (including LET, LAMBDA, XLOOKUP,
   dynamic arrays, add-in UDF calls, 3D references, OpenFormula) with zero failures, and with a
   small R1C1 step it turns 2,133 formulas of an engineering workbook into 10 patterns.

## 2. Corpus

75 files, all from public test suites or examples with redistributable licenses, pinned to a commit:
Apache POI and Apache Tika (Apache-2.0), LibreOffice core (MPL-2.0), calamine (MIT), IronCalc
(MIT OR Apache-2.0), unifloc_vba (MIT, petroleum engineering workbooks), a tunnel design
workbook (MIT), docx4j (Apache-2.0). Full list with source URL, license, sha256 and purpose:
`spike/corpus/office/MANIFEST-office.md`.

| Group | Files | Covers |
|---|---|---|
| Spreadsheets | 24 | cross-sheet and 3D formulas, array formulas, shared formulas, defined names (constant, array, range, OFFSET), number formats, 52 validation rules, legacy notes, threaded comments with reply, hidden and very hidden sheets, hidden rows with outline levels, merged cells, 4 Excel tables, conditional formats, 2 pivot tables, chart, external workbook link, custom properties, VBA, XLSB, XLS (one with a note), ODS with formulas and annotations |
| Engineering and modern formulas | 10 | tunnel structural calc (86 chained formulas, units in text), pipeline calc with 14 names as variables and add-in UDFs, friction factor with VBA UDFs, PVT z-factor with 2,133 formulas over hundreds of rows, CONVERT units (509 formulas), XLOOKUP (439), LET, LAMBDA incl. defined-name lambdas, dynamic arrays and spill, what-if data tables |
| Word processing | 25 | heading and list styles, comments, comment reply, resolved comment, tracked insert and delete with author and date, footnotes and endnotes, REF/PAGEREF/NOTEREF/SEQ fields, bookmarks, content controls, checkbox, merged table cells, OMML, SmartArt, alt text, caption, mail merge with data source, DOCPROPERTY fields, embedded Excel worksheets, external file link, DOC (one with a comment), RTF with comment, ODT with annotations, changes and replies |
| Presentations | 10 | titles, bullet levels, tables, speaker notes, legacy comment, chart with linked workbook, SmartArt, alt text, sections, embedded OLE workbook, PPT (one with a comment), ODP (its notes pages are empty) |
| Malformed | 5 | calamine panic and OOM regressions, ODS repeat-count DoS, truncated XLS, fake ODT |
| Stress | 1 | `deep-table-cell.docx`: 5,000 nested tables |

Known gaps (no public file found in time): XLSX hidden columns, iterative calculation enabled,
PPTX modern comments (`p188:cm`), hidden slide, DOCX `gridSpan`, Word table formula
(`=SUM(ABOVE)`), document variables, custom percent `numFmt`. The raw probe reads all of these
attributes, but they are not proven on a real file.

## 3. Method

- One small probe binary per crate (`spike/office-test/probes/p-*`), each prints the crate's
  model for one file (JSON where the crate serializes its model). `run.sh` runs every
  (probe, file) pair as its own process with a 20 s alarm and records exit code and peak RSS.
- APIs scored from the source in `~/.cargo/registry` first, then confirmed on corpus output.
  Cells marked `(src)` were scored from the source only, because no probe output shows them.
- `p-raw` reads the parts directly with zip 9.0.5 + quick-xml 0.42.0.
- `p-formula` feeds calamine's formula strings to formualizer-parse and builds R1C1 patterns,
  cross-sheet edges and name uses.
- Licenses from `cargo metadata` over the full transitive dependency tree per probe.
- Build cost: `measure.sh`, a clean `--release` build per probe in its own target dir, one
  after the other (Apple M5, 10 cores). Binary sizes are stripped. Baseline hello world:
  0.34 MB.

## 4. Candidates

| Crate | Version (date) | License | MSRV | Activity | Clean build | Binary |
|---|---|---|---|---|---|---|
| calamine | 0.37.0 (2026-10-18) | MIT | 1.88 | 2,455 stars, 23 commits in 90 days, since 2016 | 6 s | 1.3 MB |
| umya-spreadsheet | 3.1.1 (2026-10-12) | MIT | 1.89 | 489 stars, 94 commits/90 d, 80 open issues | 13 s | 4.2 MB |
| office_oxide | 0.1.15 (2026-10-15) | MIT OR Apache-2.0 | 1.88 | 154 stars, 31 commits/90 d, created 2026-04 | 17 s | 3.4 MB |
| undoc | 0.27.0 (2026-10-15) | MIT | 1.88 | 36 stars, 100+ commits/90 d, 58 releases in 10 months | 8 s | 2.4 MB |
| anydoc | 0.2.4 (2026-08-27) | MIT | 1.88 | 22,962 stars, 100+ commits/90 d, no release since August | 35 s | 9.5 MB |
| docx-rs | 0.4.22 (2026-07-21) | MIT | none | 560 stars, 35 commits/90 d | 9 s | 1.8 MB |
| ooxmlsdk | 0.14.0 (2026-10-19) | MIT OR Apache-2.0 | 1.88 | 72 stars, 100+ commits/90 d | 109 s | 20.9 MB |
| spreadsheet-ods | 1.0.4 (2026-03-20) | MIT/Apache-2.0 | none | 37 stars, no commits in 90 d | 11 s | 1.4 MB |
| pptx-to-md | 1.0.0 (2026-07-17) | MIT OR Apache-2.0 | none | 6 stars, no commits in 90 d | 22 s | 1.1 MB |
| rtf-parser | 0.4.3 (2026-06-11) | MIT | none | 17 stars, no commits in 90 d | 8 s | 0.4 MB |
| formualizer-parse | 3.2.0 (2026-10-07) | MIT OR Apache-2.0 | none | 202 stars, 100+ commits/90 d | with calamine: 5 s | 1.3 MB |
| Xberg (`office`, `excel` features) | 1.3.10 (2026-10-13) | MIT | 1.92 | 9,427 stars, 100+ commits/90 d | 83 s | 19.4 MB |
| zip + quick-xml + cfb (own reader) | 9.0.5 / 0.42.0 / 0.15.0 | MIT | - | - | 3 s | 0.6 MB |
| ferrodoc-docx / -odt | 1.0.1 (2026-09-02) | MIT OR Apache-2.0 | 1.85 | 12 stars, created 2026-08 | not measured | - |

ooxmlsdk is not in the matrices: it is a generated, typed copy of the OOXML schema (like the
.NET Open XML SDK). Every element is reachable as a typed struct, nothing is interpreted, so
it scores like the own reader minus the work saved on XML parsing. It opened all 52 OOXML
files in the corpus, but costs 109 s of clean build and 20.9 MB of binary for a probe that only
touches three root parts, and it aborted with a stack overflow on the nested-table file. As a
base for Rezzer's own reader, plain quick-xml (3 s, 0.3 MB over baseline) is the better trade.

Not tested further: ferrodoc (see licenses), docx-rust, ovba (non-standard license on
crates.io, and calamine already lists VBA module names), oletools_rs (one release),
litchi (0.0.1).

## 5. Scoring: spreadsheets (XLSX/XLSM unless noted)

Xberg is not listed separately for the cell features: it uses calamine and puts formulas and
comments into a flat metadata string. undoc scores like anydoc here (values only).

| Item | calamine | umya | office_oxide | anydoc | spreadsheet-ods (ODS) | own reader |
|---|---|---|---|---|---|---|
| Cell values with type | E (`Data` enum, dates from formats) | E (data type + raw) | E (`data_type`, `raw_number`) | P (rendered text) | E (`Value`) | E |
| Formulas with the cells and ranges they read | P (text only) | P (text) | P (text) | N | P (OpenFormula text) | E with formualizer-parse |
| Shared formulas | E (expanded; stream API gives `Shared{range}`/`SharedDerived`) | E (shared index) | E (expanded text) | N | - | E |
| Array formulas | P (anchor text, no range, no array flag) | E (`Array` + ref range) | P (text) | N | P (matrix span) | E |
| Defined names with scope | P (no scope; XLS built-in names come back as control characters) | P (scope guessed wrongly; array constant `{"A","CC","BBB"}` returned as `BBB}`) | E (`local_sheet_id`, hidden) | N | N | E |
| Number formats (0.09 is 9 %) | P (only to detect dates) | E (code + formatted value) | E (code + id per cell) | N | E (format tables) | E |
| Data validation rules and lists | N | E (type, operator, formula1, ranges, messages) | E (range, type, operator, formula1) | N | E (src; no ODS with validation in corpus) | E |
| Legacy notes with author | N | E | P (text with author, no position field) | N | E (ODS annotation with creator and date) | E |
| Threaded comments, author, date, replies | N | P (dT and id, no person, no parent) | P (flattened to one note per cell, no date) | N | - | E (person, dT, parentId) |
| Hidden sheets | E (incl. very hidden) | E (`state()`; `sheet_state()` returns empty) | E | N | P (sheet hidden by style not resolved) | E |
| Hidden rows and columns | N | E (rows; columns src) | N | N | P (rows/cols API, no hidden ones in corpus) | E (rows; columns src) |
| Merged cells | E | E | E | E | E | E |
| Excel tables with header names | E (name, sheet, columns, data) | E (name, area, columns) | N | N | - | E |
| Conditional formatting | N | E | E | N | N | E |
| Pivot source ranges | P (names and sheets) (src) | E (source sheet + range) | N | N | N | E |
| Chart type and source ranges | N | E (plot area type, series refs) | N | N | N | E |
| External links | N (formula keeps `[1]`) | N | N | N | N | E (target path) |
| Core, app, custom properties | P (core + app, no custom) | E | E | N | E (meta) | E |
| VBA presence and module names | E (module names, never run) | P (bytes only) | N | N | P (scripts) | P (presence; names via cfb) |
| **Engineering and modern formulas** | | | | | | |
| Dynamic arrays and spill range | P (text, no spill range) | P (`Array` + ref = spill range, no `cm` flag) | P (text) | N | - | E |
| LET, LAMBDA, defined-name lambdas | P (raw text with `_xlfn.`/`_xlpm.`) | P (same) | P (same) | N | - | E (text) + parser |
| What-if data tables (TABLE) | N (formula dropped) | P (type `DataTable` and ref, no input cells) | N (dropped) | N | N | E (ref, r1, r2, dt2D, dtr) |
| Iterative calc and calc mode | N | N (writer only) | N | N | N | E (`calcPr`) |
| Cached values next to formulas | E | E | E | P | E | E |
| Group copied formulas (R1C1) | P (shared groups only) | P (shared index only) | N | N | N | E with own R1C1 step |
| Dependency graph (precedents across sheets and names) | N | N | N | N | N | E with formualizer-parse |
| Units from formats and headers | N | P (format codes) | P (format codes) | N | P | own heuristics |
| VBA UDFs used in formulas | P (module names, UDF calls in text) | N | N | N | N | E (add-in path from external link) |
| Formula truncation | none (3,438 of 3,448; the 10 missing are data table and spill cells) | none (3,448 of 3,448) | none (3,438 of 3,448, same 10) | - | none | none |

Legacy and other spreadsheet formats:

| Item | calamine | office_oxide | undoc | anydoc |
|---|---|---|---|---|
| XLSB values / formulas / VBA | E / E / E | E / N / N | - (fails: "Missing component") | E / N / N |
| XLS values / formulas | E / P (3D refs decoded wrongly, see 7.3) | E / N | E / N | E / N |
| XLS names with scope | P (control characters for built-ins) | E | N | N |
| ODS values / formulas / names | E / P (`of:=[.B4]` raw) / E | - | - | E / N / N |
| XLS cell note with author | N | P (text + author, cell) | P (text inline in the cell: `[Comment: Author: ...]`) | N |

## 6. Scoring: word processing (DOCX unless noted)

| Item | anydoc | undoc | office_oxide | docx-rs | Xberg | own reader |
|---|---|---|---|---|---|---|
| Paragraph styles mapped to roles | P (heading level, quote, code; no style name) | E (`style_name`, heading level) | P (heading level; no style names) | E (style ids and style table, raw) | N (plain text by default) | E |
| Lists with levels | E | E | E | P (numbering raw) (src) | P (plain text) | E |
| Tables incl. merged cells | E (covered slots) | E (row span) | E | P (raw vMerge) (src) | P (separate `tables` field) | E |
| Comments anchored, author, date | N (dropped) | N | P (anchor + author, no date) | E (range start/end, author, date) | N | E |
| Comment replies and resolved state | N | N | N (flattened) | E (`commentsExtended`: parent, done) | N | E |
| Tracked changes with author and date | N (inserts accepted, deletes dropped) | P (inserted/deleted flag, no author or date) | N | E (insert/delete with author, date) | N | E |
| Footnotes and endnotes | E | E | E | N (empty in model) | P (text, `[^2]` marker) | E |
| Fields and cross references | P (internal links resolved to anchors) | N | P (field code text leaks into body) | E (instr text raw) | N | E (REF, PAGEREF, NOTEREF, SEQ, MERGEFIELD, DOCPROPERTY) |
| Hyperlinks and bookmarks | E | P (links) | P | E | P (text) | E |
| Content controls | N (unwrapped) | N | N | E (`structuredDataTag`) | N | E |
| Headers and footers | N | E | E | E | N | E |
| Equations (OMML) | E (to LaTeX) | N | N | N (fails to open the file) | P (LaTeX for display math, inline math lost) | P (find; OMML to LaTeX is own work) |
| Images with alt text, caption | P (alt) | E (alt, table caption) | E (alt) | P (src) | N | E |
| Checkboxes | P (rendered as a box character) | N | N | P (sdt) | N | E |
| Document properties | N | P (core) | E (core + custom; app keys mixed into custom) | P (src) | E | E |
| Mail merge fields and data source | P (`«Name»`) | N | P (`«Name»`) | P (field text) | N | E (field, data source path, SQL query) |
| Custom properties used in fields | N | N | P | P | N | E |
| Embedded OLE objects (e.g. Excel) | P (placeholder "Embedded object: Excel.Sheet.12") | N | E (embedded sheet content inline) | N | N | E (part, ProgID; recurse) |
| Linked files | N | N | N | P (rels) | N | E |
| Word table formulas, document variables | not in corpus | | | | | reads `=` fields and `w:docVar` |

Legacy and other text formats:

| Item | anydoc | office_oxide | undoc | rtf-parser |
|---|---|---|---|---|
| DOC headings, lists, table, footnote, link | E (no heading levels) | E (headings, header/footer, footnote) | E (style names, header/footer, note) | - |
| DOC merged cells | E (9 covered slots) | E (col_span 3) | N | - |
| DOC comment with author | N (dropped) | P (text + author, no date) | N (only document metadata) | - |
| RTF structure (table, footnote, list, link) | E | - | - | N (styled text only; info block text mixed into body) |
| RTF comment with author and date | N (dropped) | - | - | N |
| ODT headings, lists, notes, math, image | E | - | - | - |
| ODT annotations and tracked changes | N (author and date pasted into the text: "Apache Tika Bart Hanssens2009-10-05T19:10:26Tika is part...") | - | - | - |

## 7. Scoring: presentations (PPTX unless noted)

| Item | anydoc | undoc | office_oxide | pptx-to-md | own reader |
|---|---|---|---|---|---|
| Slide titles and order | P (H2 when a title placeholder exists) | E | E | E (`TextRole::Title`) | E |
| Text with levels | E | P | E | E | E |
| Speaker notes | P (as a block quote, no role) | E | E | E (ODP not tested: notes pages in the corpus file are empty) | E |
| Tables | E | E | E | E (spans) | E |
| Charts (type, refs, data) | N | N | N | N (`Unknown`) | E (series refs, `externalData`, linked workbook path) |
| Images with alt text | P | E | E | N (no alt field) | E |
| Comments with author and date | N | N | P (text + author) | P (text only) | E (author, dt, text) |
| Sections and layouts | N | N | N | N | E (sections) |
| Hidden slides | not in corpus | | | | reads `show="0"` |
| Embedded OLE and linked files | N | N | N | N | E (ProgID `Excel.Sheet.12`, external targets) |
| Properties | N | P (core) | E (incl. custom) | P (core) | E |
| PPT (legacy) | P (text, lists) | P (slide name) | P (title) | - | - |
| PPT comment | N | N | N | - | - |

## 8. Evidence

### 8.1 Formulas and engineering sheets

```
# calamine stream API: shared formula groups (pivots.xlsx)
formula-meta DataSheet (2, 6) Shared { shared_index: 0, range: Some(Dimensions { start: (2, 6), end: (10, 6) }), formula: "D3/E3" }
formula-meta DataSheet (3, 6) SharedDerived { shared_index: 0 }

# umya: dynamic array spill and data table (DynamicArrays.xlsx, one-variable.xlsx)
formula A3 =_xlfn.SEQUENCE(10,2,5,2) type=Some(Array) ref=Some("A3:B12") cached="5"
formula B5 = type=Some(DataTable) ref=Some("B5:B11") cached="2"

# own reader: data table inputs and threaded replies
data_tables=["ref=\"B5:B11\" r1=\"A2\" dt2D=\"0\" dtr=\"0\"", "ref=\"E5:I5\" r1=\"B2\" dt2D=\"0\" dtr=\"1\""]
threaded ref="A2" dT="2026-04-07T14:14:05.49" personId="{4A73...}" parentId="{FB8E...}"
person displayName="Mike Kaganski" id="{4A73...}" providerId="Windows Live"

# formualizer-parse + own R1C1 grouping (ex012.PVT_z_factor.xlsm)
formulas=2133 parse_failures=0 reference_edges=8180 pattern_groups=10
  group x672 first=z!R62C6 last=z!R82C37 key=R[-24]C4,R36C[0],name:gg,name:zcor :: =[1]!unf_pvt_dZdt(#,#,gg,zcor)*100
named-range uses: {"gg": 2016, "zcor": 2016}

# ex054.MF_friction_factor.xlsm: add-in UDF, names as variables, add-in path
group x115 key=R[0]C3,R15C[0],name:type,name:smooth_transition :: =[1]!unf_friction_factor(#,#,type,smooth_transition)
external-link xl/externalLinks/_rels/externalLink1.xml.rels -> /unifloc/unifloc_vba/UniflocVBA_7.xlam

# Xberg 1.3.10: cap and wrong cell addresses
FORMULAS formulas_z count=200 first=["D1=[1]!unf_version()", ...]      (file has 2,133 formulas; the cell is G1)
FORMULAS formulas_Structural_Lining first=["B1=B5/2", ...]              (the cell is B16)
```

Formula counts against the raw XML (cells with an `<f>` child, 3,448 in the XLSX/XLSM files):
umya matches on every file. calamine and office_oxide match too, except for two kinds of cells
they skip: data table cells (one-variable.xlsx: 3 of 5) and array cells with an empty `<f>`
(DynamicArrays.xlsx: 6 of 14). No crate truncates; only Xberg does (section 8.3).

### 8.2 Word and presentation logic

```
# own reader: tracked changes, comment thread, resolved state, fields, mail merge
revision del id="1" author="vmiklos" date="2014-06-08T12:37:00Z"
comment-ex para=02000000 -> comment 0 parent=Some("01000000") done=None
comment-ex para=78D78669 -> comment 0 parent=None done=Some("1")
field "REF _Ref100 \\h"            field "SEQ Table \\* ARABIC"
field "DOCPROPERTY  currency  \\# \"0.00 'is sales tax' -000000\" \\* MERGEFORMAT"
external-target in word/_rels/settings.xml.rels: mailMergeSource -> file:///D:\bug\data%20source.xls
settings val="SELECT * FROM `Sheet1$` "

# own reader: presentation logic
section name="section-demo" id="{FEA1...}"
chart ppt/charts/chart1.xml refs=["Sheet1!$B$1", "Sheet1!$A$2:$A$9", "Sheet1!$B$2:$B$9"] externalData=["id=\"rId3\""]
external-target in ppt/charts/_rels/chart1.xml.rels: oleObject -> file:///C:\Users\tallison\Downloads\testEXCEL_charts.xlsb
slide ppt/slides/slide2.xml: ole=["progId=\"Excel.Sheet.12\" name=\"Worksheet\"", ...]
comment ppt/comments/comment1.xml authorId="1" dt="2015-09-30T13:57:43.705" idx="1"

# anydoc: tracked change accepted silently, comment gone (groupshape-trackedchanges.docx, testComment.docx)
## markdown (9 chars)  | Inserted
## markdown (19 chars) | Here is some text.

# office_oxide: field code leaks into text (field_numeric_formatting_switch.docx)
DOCPROPERTY  currency  \\# "0.00 'is sales tax' -000000" \\\* MERGEFORMAT€180,000.00 EUR

# office_oxide: embedded worksheet read inline (embedded-xlsx.docx)
Sheet1 / A1 B1 / A2 B2
```

### 8.3 Bugs found in the crates

- calamine 0.37.0, XLS: `3dFormulas.xls` decodes a 3D range as `SUM(Sheet1!$BTRM$1:$BTRM$4)`,
  built-in defined names (print area and so on) come back with control characters as names,
  and sheet names with spaces are not quoted (`EIM New Deals!$A$1:$Q$33`), so the text is not a
  valid formula any more. ODS formulas come back as raw OpenFormula (`of:=[$Data.A3]`).
- umya-spreadsheet 3.1.1: defined name `{"A","CC","BBB"}` is returned as `BBB}`; workbook
  names are moved to sheets by their address; `sheet_state()` returns an empty string while
  `state()` is right.
- office_oxide 0.1.15: tracked changes accepted without a trace; DOCPROPERTY field code text
  appears in the body; app.xml values are listed as custom properties.
- anydoc 0.2.4: ODT annotation and change-region author and date are concatenated into the
  heading and body text.
- docx-rs 0.4.22: fails with `ZipError(FileNotFound)` on small valid DOCX files without
  optional parts (`testWORD_omml.docx`, `testWORD_refFields.docx`); footnotes are empty in the
  model.
- spreadsheet-ods 1.0.4: hidden sheets report `display=true` (visibility lives in the table
  style).
- Xberg 1.3.10: formula metadata capped at 200 per sheet and cell addresses relative to the
  formula range (wrong when the first formula is not in A1).

## 9. Robustness

Each pair ran in its own process with a 20 s timeout.

| File | calamine | umya | office_oxide | anydoc | undoc | docx-rs | spreadsheet-ods | ooxmlsdk | Xberg | own reader |
|---|---|---|---|---|---|---|---|---|---|---|
| issue_666_panic.xlsb | ok | error | ok | ok (empty) | error | - | - | - | ok (empty) | ok |
| issue_594_dos.ods | error, cell limit | - | - (no ODS) | error, expansion limit | - | - | **timeout, 5.5 GB RSS** | - | error, cell limit | - |
| OOM_alloc.xls | ok | - | ok | ok | ok | - | - | - | ok | - |
| too_small.xls | error | - | error | error | error | - | - | - | error | - |
| testODTnotaZipFile.odt | error | error | - | error | error | error | error | - | error | error |
| deep-table-cell.docx (5,000 nested tables) | - | - | ok (stops at 101 tables) | error, depth limit 256 | **timeout, 2.2 to 3.8 GB RSS** | error (missing part) | - | **stack overflow, abort** | error, depth limit 1024 | ok |

calamine, anydoc and Xberg have explicit resource limits. undoc, spreadsheet-ods and ooxmlsdk
need a guard (process isolation or limits) before Rezzer runs them on untrusted files.

## 10. Licenses

Full transitive trees from `cargo metadata` for every probe: all MIT, Apache-2.0, BSD-2/3,
ISC, Zlib, Unicode-3.0, 0BSD, Unlicense or MPL-2.0. Notes:

- `libbz2-rs-sys` (pulled in by zip's default features through umya, anydoc, docx-rs,
  pptx-to-md, Xberg) is under the bzip2-1.0.6 license, a permissive BSD-style license.
  cargo-deny needs an explicit allow entry, or zip without the bzip2 feature.
- `option-ext` (Xberg) is MPL-2.0, `r-efi` is MIT OR Apache-2.0 OR LGPL-2.1 (MIT applies).
- `color-rs2` (spreadsheet-ods) has only a license file, which is Apache-2.0.
- **ferrodoc-docx** is labelled MIT OR Apache-2.0, but its docs say the inline structure is
  rebuilt with "a port of pandoc's `Docx.Combine` smushing algorithm" and that it follows
  pandoc's reader closely. pandoc is GPL-2.0-or-later. Under Rezzer's rule (rewriting someone's
  code is a derivative work) this is a provenance risk. Do not use it, and do not use its
  source as reference.
- No crate links LibreOffice, Poppler or other GPL/LGPL C code.

## 11. How Rezzer should present engineering spreadsheets

The probes show that the logic of an engineering workbook is small once copies are folded:
2,133 formulas are 10 patterns, 151 are 6, 509 CONVERT formulas are 22. Proposal for the
output of one sheet:

1. **Parameters.** Constant input cells that formulas read, with their label from the cell to
   the left or the header above, the value, and the unit (from the label text like
   "Excavated diameter ... 10.0 m", the number format code, or a CONVERT unit argument).
   Defined names used as variables (`gg`, `zcor`, `pipe_diam`) are listed here with their
   cell.
2. **Formulas grouped by pattern.** One row per R1C1 pattern: range it covers
   (`z!F62:AK82`, 672 cells), the formula in A1 form of the first cell, the inputs it reads.
   Single formulas stay single. Inside a group only the first formula is shown.
3. **Results.** Cells with formulas that no other formula reads: the outputs of the
   calculation, with cached value and unit.
4. **Dependency graph** as Mermaid `flowchart LR`: nodes are parameter blocks, formula groups
   and results, edges from precedents. Cross-sheet edges and names are kept; for large sheets
   the graph is drawn on group level, not cell level.
5. **Hidden logic** listed explicitly: UDFs and where they come from (VBA module or add-in
   path from the external link, for example `UniflocVBA_7.xlam`, and the note that Rezzer
   cannot evaluate them), external workbooks, data tables with their input cells, LET and
   LAMBDA (defined-name lambdas as functions with parameters, `_xlfn.`/`_xlpm.` prefixes
   removed), dynamic arrays with spill range, iterative calculation and manual calc mode
   (results may be stale), volatile functions.
6. **Honesty notes.** When a cached value is missing, say "not calculated in file". When a
   formula fails to parse, show the text and say so.

A formula parser is needed for this: no crate exposes references. **formualizer-parse**
(MIT OR Apache-2.0, 3.2.0) parsed all 3,470 corpus formulas (XLSX, XLSM, XLSB, XLS, ODS),
including OpenFormula. It
returns `Cell`, `Range`, `Cell3D`, `Range3D`, `External`, `Table` and `NamedRange` references
with absolute flags, which is enough for R1C1 and the graph. Two caveats: LET and LAMBDA
parameters (`_xlpm.x`) come back as named ranges and must be filtered, and the crate is young
(15 releases in 13 months), so it goes behind a Rezzer trait. IronCalc's `ironcalc_base`
(MIT OR Apache-2.0) is the fallback; it is a full engine and larger.

## 12. How Rezzer should present the hidden logic of documents and presentations

The same idea applies: the visible text is one layer, the logic behind it is another, and the
output should keep them apart but linked.

- **Change history.** Tracked changes as a list grouped by author, each with date, kind
  (insert, delete, move, format) and the passage. The body shows the current text with
  markers (`{+inserted+}`, `{-deleted-}` or footnote-style references), so nothing is
  accepted silently.
- **Comment threads.** One thread per anchored range: the quoted passage, then root comment
  and replies with author and date, and the resolved state. Same for PPTX slide comments and
  XLSX threaded comments.
- **Fields as data.** Fields keep their result in the text and their code in a field list:
  cross references (REF, PAGEREF, NOTEREF) become edges to bookmarks, captions (SEQ) become
  numbered figures and tables; together they form a reference graph that can be drawn as
  Mermaid. Mail merge fields list the data source path and query. DOCPROPERTY and DOCVARIABLE
  fields show which property or variable they read. Table formulas (`=SUM(ABOVE)`) are
  listed like spreadsheet formulas.
- **Embedded and linked objects.** Embedded workbooks, documents and chart data are extracted
  and converted by Rezzer recursively, then shown as a sub-document with its source part.
  Linked files (external chart data, OLE links, relative links) are listed with their path and
  marked "not available" when Rezzer cannot read them.
- **Presentations.** Slide order with section names, hidden slides marked as hidden (not
  dropped), speaker notes under each slide, charts as Mermaid with their series ranges and
  the workbook they come from, alt text for every picture.

## 13. Risks

1. **Own reader effort.** DOCX and PPTX structure (styles, numbering, tables, notes) is more
   than the 236-line probe; the probe only covers the knowledge layer. Number formats alone
   need `cellXfs` plus `numFmts` plus the built-in id table (ids 9 and 10 are percent, some
   ids depend on the locale) and a format-code interpreter to show 0.09 as 9 %. Estimate several
   thousand lines for full DOCX, PPTX and ODF readers. Mitigation: keep office_oxide or
   anydoc as a reference and diff against them on the corpus.
2. **Young crates.** office_oxide (6 months), undoc (58 releases in 10 months),
   formualizer-parse, pptx-to-md (6 stars, no commits in 90 days). Pin exactly, wrap behind
   traits, keep golden files.
3. **Legacy binaries.** DOC and PPT structure comes from crates that are under one year old,
   and calamine has XLS bugs (3D references, built-in names). Test with more real files before
   relying on them. LibreOffice as an optional external converter stays the fallback.
4. **Resource limits.** undoc, spreadsheet-ods and ooxmlsdk blew up on stress files. Every
   reader needs depth, size and cell limits, and conversions should run with a timeout.
5. **Corpus gaps** listed in section 2 are untested on real files. In legacy binaries only one
   comment per format was tested (DOC, XLS, PPT); tracked changes in DOC and everything else in
   the knowledge layer of DOC, XLS and PPT are untested.
6. **Units** are heuristics (labels, number formats, CONVERT arguments); the output must say
   where a unit came from.
