# CAD spike part B: 2D drawings (DXF, DWG)

Group `drawing`, 2026-10-19. Probes in `spike/cad-test/drawing/`, corpus in
`spike/corpus/cad/drawing/` (95 files, 36 MB, see `MANIFEST-part.md`), raw results in
`spike/cad-test/drawing/results/` (`runs.tsv` with time and peak RSS per pair, one JSON per
probe and file, `compare.md` with the paired diffs). `run.sh` rebuilds all probes and runs every
(probe, file) pair in its own process with a 20 s alarm and `/usr/bin/time -l`;
`compare.py` writes the paired diff tables. Clean build time and binary size are left to the
coordinator.

## 1. Short verdict

- **DXF:** `acadrust` 0.6.5 (MPL-2.0) has by far the richest model and read every real DXF in
  the corpus, but only in failsafe mode: strict mode aborts a real AutoCAD R2004 DXF on one
  unsupported object (`DATATABLE cell type 6`). `cadkit` 0.3.4 (MIT OR Apache-2.0) is the most
  honest reader (typed warnings for every unmodelled type, `None` for absent values) but drops
  TABLE, MULTILEADER, MLINE, TOLERANCE, groups and XRECORDs into `Unknown`. `dxf` 0.6.2 (MIT),
  the "safe mature choice" of the research report, silently drops HATCH, VIEWPORT,
  MULTILEADER, ACAD_TABLE, MESH and 2-line angular dimensions, has no layer frozen/locked
  flags, fabricates `$TDCREATE`/`$FINGERPRINTGUID` when they are absent, truncates text, and
  fails hard on three real AutoCAD files (R12 ASCII, R12 binary, R2000 binary). Not
  recommended. An own group-code reader is about 255 lines and matched the crates on the core
  items (header, layers, title block, text, dimensions, counts); it trails acadrust on
  dynamic blocks, TABLE grid structure and dimension style tolerances.
- **DWG:** yes, permissive Rust crates read real DWG R14 to R2018. On the ACadSharp reference
  drawing (same drawing saved as DWG and DXF in every version) `acadrust` and `cadkit` both
  matched the DXF twin exactly for R2000 to R2018: 169/169 top-level entities, 29/29 TEXT,
  5/5 MTEXT, 11/11 dimension values, all layer names and flags, all layouts, all ATTRIB values.
  `acadrust` additionally reads TABLE cells, MULTILEADER text, dynamic block parameters,
  groups, XRECORDs and the DWG summary info. Neither reads pre-R13 DWG: all 8 real US Forest
  Service drawings (AC1006 and AC1009, 1987 to 1996) failed in both crates.
- **Robustness:** `acadrust` panics (index out of bounds) on a truncated R2007 DWG, even in
  failsafe mode, and accepts a bit-flipped R2004 DWG without any diagnostic. `cadkit` never
  panicked and reported checksum and CRC mismatches. Rezzer's own DWG reader should verify
  checksums like cadkit and be fuzzed with the hostile files of this corpus.

## 2. Candidates

| Crate | Version (date) | License | MSRV | Activity | Transitive licenses (`cargo metadata`) |
|---|---|---|---|---|---|
| dxf (ixmilia/dxf-rs) | 0.6.2 (2026-10-03) | MIT | none | 142 stars, since 2016, 4 commits in 90 days, releases 2024-11, 2026-03, 2026-10 | 72 crates, all permissive (pulls `image` for thumbnails; `r-efi` is MIT OR Apache-2.0 OR LGPL-2.1, Windows UEFI only, MIT option) |
| acadrust (hakanaktt) | 0.6.5 (2026-10-18) | MPL-2.0 | none | 134 stars, created 2026-01, 100+ commits in 90 days, 34 releases in 8 months (weekly) | 63 crates, all permissive (rayon, memmap2, nom, flate2, encoding_rs, oem_cp) |
| cadkit (TNYCL) | 0.3.4 (2026-10-19) | MIT OR Apache-2.0 | 1.85 | 3 stars, created 2026-10-01, 28 commits, 8 releases in 19 days | 49 crates, all permissive |
| undxf (iyulab) + uncad-model 0.9.0 | 0.8.0 (2026-10-07) | MIT | 1.88 | 0 stars, created 2026-09-23, 81 commits, 9 releases in 2 weeks | 21 crates, all permissive |
| own group-code reader | - | MPL-2.0 (Rezzer) | - | 255 lines reader core, 330 lines extraction and JSON | 12 crates (serde_json only) |

Looked at, not tested: `dxf-tools-rs` 0.1.3 (same author as acadrust, older), `dxfscan` 0.1.0
(binary DXF only, ISC), `dwg2geo` 0.2.5 (wraps acadrust, GeoJSON output), `exav-render`
0.0.2 (renderer), `cadkit-cli`. Not touched (GPL): `libredwg-sys`, `uncad`, `uncad-cli`.
`undxf` is a sibling of the GPL `uncad` by the same organisation, but it is MIT and its
transitive tree is clean; it is a rendering-oriented model (no hatch pattern names, no ATTDEF
prompts, no XDATA, no groups), so it is scored but not a contender.

External programs: none. Rezzer runs every reader in-process in its one binary (owner
decision, 2026-10-19), so LibreDWG `dwg2dxf` and ODA File Converter are out of scope. The DWG
crates are scored here as a comparison and as a reference for Rezzer's own DWG reader, which
will be written from a separate format specification.

## 3. Applicable items

All items of the brief apply to DXF and DWG, except: DWG summary info (title, author,
keywords, custom properties) and the DWG preview image apply to DWG only; DXF has neither.
Dynamic block parameters exist in both (DXF stores them as objects). Geometry is only summary
facts (extents, counts).

## 4. Scoring matrix: DXF

Legend: E exposed in the public API, P partial (reason in the cell), N not exposed, - not
supported. `(src)` scored from source only.

| Item | dxf 0.6.2 | acadrust 0.6.5 | cadkit 0.3.4 | undxf 0.8.0 | own reader |
|---|---|---|---|---|---|
| $INSUNITS, $MEASUREMENT | E (enums) | E (raw ints) | E (typed `Units` with meters per unit, plus raw props) | P (insunits typed, rest raw strings) | E |
| $ACADVER | E | E | E | P (raw) | E |
| $EXTMIN/$EXTMAX vs computed extents | E | E (absent = sentinel 1e20) | P (raw props) | P (raw) | E (both; differ on the ACadSharp sample: header x min 0, computed -3024) |
| $TDCREATE, $TDUPDATE, $LASTSAVEDBY, $FINGERPRINTGUID | P (fabricates now() and a random GUID when absent, +1 s offset, tags local wall time with the machine's time zone) | E (raw Julian, absent = 0) | P (raw props, absent = None) | P (raw strings) | E |
| Layers: name, color, on/off, frozen, locked, plot, linetype | P (no frozen, no locked) | E (+ description) | E (+ description) | E | E |
| Blocks, INSERT with ATTRIB values (title block) | E | E | E | E | E |
| ATTDEF tag, prompt, default | E | E | P (in `props` as `dxf.attdef.*`) | P (no prompt) | E |
| Multi-line ATTRIB (embedded MTEXT) | E | E | E | N (value empty) | E |
| Dynamic block parameters | N | E (visibility state, lookup, distances) | N | N | - |
| TEXT content | E (raw, `%%c` not decoded) | E (raw) | E (raw) | E (raw) | E (raw) |
| MTEXT formatting decoded | N (raw only, and truncation bug) | E (typed spans, `to_plain_text`) | E (`plain` field) | N (raw) | E (subset: \P \S \U+ %% fonts colors, 60 lines) |
| DIMENSION measured value, override text, style | P (2-line angular dimension dropped) | E | E | E (`TextOverride` enum) | E |
| Dimension tolerances (DIMSTYLE DIMTOL/DIMTP/DIMTM) | E (src, DimStyle fields) | E (src) | N | E (src, `DimStyleRecord`) | N (not written, about 20 lines) |
| LEADER / MULTILEADER text | P (LEADER only) | E | P (LEADER vertices; MULTILEADER is Unknown with warning) | E | E |
| TABLE entity (BOM table cells) | - (dropped silently) | E (rows, columns, cell text) | N (Unknown with warning) | E (grid with spans) | P (cell strings, no grid) |
| Layouts and paper-space viewports | P (LAYOUT objects; VIEWPORT dropped) | E | E (one `Model` per layout) | E (with plot settings) | P (names, viewport count) |
| XREF blocks with path (synthetic file only; no real xref in the corpus) | E | E | E | E | E |
| Image and underlay paths | E (IMAGEDEF) | E | E (path on IMAGE) | E | E (IMAGEDEF) |
| Hatch patterns | - (HATCH dropped) | E | E | P (no pattern name) | E |
| XDATA app names, XRECORD, dictionaries | E | E | P (XDATA only, in props) | N | E |
| Groups | E | E | N | N | E |
| Entity counts per layer and type | P (only supported types) | E | E | E | E |
| Machine-readable losses | N (unsupported entities skipped silently) | E (`ReadStats` diagnostics, semantic inventory `Unsupported`, `Unknown` with raw codes) but strict mode aborts | E (typed warnings per type with offset) | P (warnings, `Unknown` entity) | E (every record type counted by name) |

## 5. Scoring matrix: DWG

dxf, undxf and the own reader do not read DWG (they fail cleanly; own reports the version
from the 6-byte magic). Paired fidelity is in section 6.

| Item | acadrust 0.6.5 | cadkit 0.3.4 |
|---|---|---|
| Versions read (real files) | AC1014 to AC1032 | AC1014 to AC1032 |
| AC1009 (R11/R12) and AC1006 (R10) real files | - (claims AC1009, failed on all 6 real AC1009 files; AC1006 unsupported) | - (by design R14+, but error says "unrecognized file format") |
| Header variables | E (typed `HeaderVariables`) | E (all 277 vars by spec name in the native API, raw in `props`) |
| Units | E | E (typed) |
| Dates, fingerprint GUID | E | P (raw `[day, ms]`) |
| DWG summary info: title, author, keywords, custom properties, last saved by | E | N |
| Preview image | E (PNG/BMP bytes) | P (block previews only) (src) |
| Layers with all flags | E | E |
| Blocks, ATTRIB, ATTDEF | E | E (ATTDEF via props) |
| Dynamic block parameters | E (`dynamic_block_properties`: visibility "VisibilityState0", lookup "Custom", distance 5.0) | N |
| TEXT, MTEXT (+ decoded) | E | E |
| DIMENSION value, override, style | E (R14: 0.0 instead of absent) | E (R14: None) |
| LEADER / MULTILEADER | E | P (MULTILEADER Unknown with warning) |
| TABLE cells | E (e.g. "Hello this is a title", "A2 cell", "$100.00") | N (Unknown with warning) |
| Layouts and viewports | E | E |
| XREF path | E (src; no real DWG xref in corpus) | E (src; same) |
| Images, PDF underlays | E | P (IMAGE yes, PDFUNDERLAY Unknown) |
| Hatch patterns | E | E |
| XDATA, XRECORD, dictionaries, groups | E (97 XRECORDs, groups `*A1`, `*A2`) | P (XDATA in props, object type names in native API) |
| MLINE, TOLERANCE, 3DSOLID, MESH, REGION, WIPEOUT, RAY, XLINE, SHAPE | E | N (Unknown with warning) |
| AEC (AutoCAD Architecture) proxies | P (reported as unsupported with names: AEC_DOOR, AEC_WALL, AEC_WINDOW, 300+ object types) | P (3 warnings with names) |
| Corruption detection (CRC, checksums) | N (bit-flipped R2004 file read with 0 diagnostics) | E (35 typed checksum/CRC warnings) |
| Machine-readable losses | E (stats: decoded, skipped, diagnostics with stage) | E |

## 6. Paired DWG vs DXF twin

Reference: own reader on the DXF twin of the same ACadSharp drawing (R2000 binary DXF for
AC1014/AC1015, R2004 ASCII for AC1018/AC1021, R2018 ASCII for AC1024 to AC1032). Independent
check: ACadSharp's own tree dump of `sample_base.dwg` lists 163 model-space entities and 19
layers; acadrust reports 163 model space plus 6 paper space and 19 layers.

| DWG | probe | entities | ATTRIB values | TEXT | MTEXT raw | dim values | layers names/flags | layouts |
|---|---|---|---|---|---|---|---|---|
| AC1014 | acadrust | 169/169 | 4/4 | 29/29 | 5/5 | 2/11 | 21/21 (R14 upper-cases names) | ok |
| AC1014 | cadkit | 169/169 | 4/4 | 29/29 | 5/5 | 0/11 (None) | 21/21 (same) | R14 block names, no layout objects |
| AC1015 to AC1032 | acadrust | 169/169 | 4/4 | 29/29 | 5/5 | 11/11 | all | ok |
| AC1015 to AC1032 | cadkit | 169/169 | 4/4 | 29/29 | 5/5 | 11/11 | all | ok |

(`compare.md` shows 3/4 ATTRIB in some rows: that is the tag of the multi-line attribute,
`MULTI_LINE_ATT` vs `MULTI_LINE_ATT_001`; both tags exist in the files, the value is
always found.) R14 has no stored dimension measurement; acadrust reports 0.0 there (looks like
a real value), cadkit reports None.

## 7. Evidence excerpts

`synthetic/titleblock_xref_r2000.dxf`, title block INSERT (all five readers agree):

```
{"block":"TITLEBLOCK","attribs":[{"tag":"TITLE","value":"Mounting bracket"},{"tag":"DRAWING_NO","value":"DWG-0815"},
 {"tag":"REV","value":"C"},{"tag":"MATERIAL","value":"S235JR"},{"tag":"PART_NO","value":"BR-100-42"}]}
```

Same file, xref and dimension (acadrust):

```
xrefs [{"block":"XREF_BRACKET","path":"..\\parts\\bracket.dwg","overlay":false}]
dims  [{"type":"linear","measurement":120.0,"text":"<> H7","style":"ISO-25"}]
```

Same file, MTEXT `...\PCaf\U+00E9 \~ok`:

```
own      "NOTES:\n1. Break all edges 0.5x45°\n2. Fit +0.02/-0.01 red\nCafé  ok"
acadrust "NOTES:\n1. Break all edges 0.5x45°\n2. Fit +0.02 -0.01 red\nCafé  ok"
dxf      raw only: "...\\PCaf\\U+00E9 "        (tail "\~ok" lost)
```

`acadsharp/sample_AC1032.dwg`, acadrust TABLE and dynamic block:

```
tables [{"rows":4,"cols":5,"cells":["Hello this is a title","A2 cell","This is my content","CELL FORMAT 1","0.500000","60",...,"35.00%","$100.00"]}]
dynamic_blocks [{"block":"my-dynamic-block","properties":[{"name":"dynamic-diameter","value":"Real(2.0)"},
 {"name":"custom_visibility","value":"Text(\"visibility_state_default\")"}]}]
summary_info {"last_saved_by":"alber","title":"","author":"","keywords":"","custom":[]}
```

`acadsharp/sample_AC1018.dwg`, cadkit losses:

```
{"code":"dwg.unsupported_entity","message":"15 MULTILEADER entities mapped to Unknown"}
{"code":"dwg.unsupported_entity","message":"2 ACAD_TABLE entities mapped to Unknown"}
```

`ezdxf/acad_table_simple.dxf`: dxf crate `counts_by_type {}` (the only entity, an ACAD_TABLE,
is gone without a trace); cadkit `entity type ACAD_TABLE is not modelled (1 occurrence(s))`;
acadrust cells `["Title","1","2","3","4","5","6"]`.

`ezdxf/Minimal_DXF_AC1009.dxf` (no date variables in the file), dxf crate header:

```
"tdcreate":"2026-10-19T19:49:13.942584+02:00","fingerprintguid":"9011e0f6-13d5-4909-88f3-2b240c2e7c23"
```

## 8. Bugs found

dxf 0.6.2:
1. Unsupported entities are skipped without any record: HATCH and VIEWPORT are commented out
   as TODO in `spec/EntitiesSpec.xml`; MULTILEADER, ACAD_TABLE, MESH, 2-line angular
   DIMENSION are missing. On the ACadSharp R2018 DXF 135 of 169 entities survive.
2. Text truncation: `un_escape_ascii_to_unicode` (`code_pair_value.rs:197`) drops any
   backslash sequence that starts within 6 characters of the string end, for files read as
   ANSI_1252 (the crate's Windows-1252 path). `a \~ok` becomes `a `, a trailing `\Pend`
   is lost.
3. Absent `$TDCREATE`/`$TDUPDATE` become `Local::now()`, absent `$FINGERPRINTGUID` a random
   UUID: invented metadata that cannot be told from real values.
4. Date conversion adds one second (`seconds - secs_per_day + 1`) and labels the wall time
   with the machine's time zone offset.
5. Hard parse failure on R12 DIMSTYLE code 5 (DIMBLK name read as a handle): real R12 ASCII
   and binary DXF from AutoCAD fail completely; an AutoCAD R2000 binary DXF also fails.
6. Layer frozen and locked flags (code 70) are not in the model.

acadrust 0.6.5:
1. Panic `index out of bounds: the len is 30` in `io/dwg/decompressor_ac21.rs:468` on a
   truncated R2007 DWG, in strict and failsafe mode.
2. Strict mode aborts a real AutoCAD R2004 DXF (`Not implemented: DATATABLE cell type 6`);
   failsafe reads it fully. Rezzer must always use failsafe.
3. Failsafe on an unreadable file returns an empty document whose version is the default
   `AC1032` (also in `stats.source_version`), for files that are AC1006 or AC1009.
4. A bit-flipped R2004 DWG (200 flips) is read in strict mode with 169 entities and zero
   diagnostics: no CRC or checksum check is surfaced.
5. Strict mode returns `Ok` with 0 entities for random bytes behind an `AC1015` magic and for
   a half-truncated R2000 DWG; only `stats.diagnostics` says something failed.
6. All 6 real AC1009 DWG files fail (`AC1009 SEQEND ... names owner ...`,
   `Non-finite AC1009 table number`), although the README lists R12 DWG read support.
7. R14 dimension measurement reported as 0.0 instead of absent.

cadkit 0.3.4:
1. Pre-R14 DWG is reported as `unrecognized file format` instead of "DWG R12, unsupported".
2. Random bytes behind `AC1021`/`AC1024`/`AC1032` sometimes give `password-encrypted DWG
   data sections` (misleading message, clean failure).
3. Accepts a DXF with a non-numeric group code as a valid empty drawing (with a
   `dxf.truncated` warning).

undxf 0.8.0: fails on `R12_with_trash_beyond_EOF.dxf` (data after EOF), loses multi-line
ATTRIB values, peak RSS 823 MB for the 45 MB DXF.

Own reader lessons (fixed during the spike, they matter for Rezzer): DXF writes newlines in
strings as caret escapes (`^J`), multi-line ATTRIB text lives in an embedded MTEXT after
`101 Embedded Object`, and only the active layout's paper space is in ENTITIES; the other
layouts live in `*Paper_Space0..n` blocks. Without these three the own reader lost 4
viewports and one title block value. Still open: `\U+XXXX` in table names is not decoded
(acadrust, cadkit, undxf do it).

## 9. Robustness

All pairs in `results/runs.tsv`. Times in seconds, RSS in MB, 20 s alarm (never hit).

| File | dxf | acadrust (strict / failsafe) | cadkit | undxf | own |
|---|---|---|---|---|---|
| huge R12 DXF, 45 MB, 613,963 entities, parse only | 0.53 s, 1027 MB | 0.65 s, 593 MB | 0.29 s, 689 MB | 0.26 s, 823 MB | 0.24 s, 214 MB |
| same, full probe incl. JSON (serde inflates cadkit/undxf) | 0.91 s, 1027 MB | 1.35 s, 595 MB | 1.28 s, 3.3 GB | 4.8 s, 16.5 GB | 0.38 s, 248 MB |
| biggest real files (2.6 MB DXF, 1.3 MB DWG), best of 5, parse only | 8 ms | 6 ms / DWG 3 ms | 9 ms / DWG 3 ms | 1 ms | 3 ms |
| self-referencing block, mutual A-B blocks | ok, no warning | ok, no warning | ok, no warning | ok | ok, reports `SELF -> SELF`, `B -> A` |
| 1000 / 20000 nested blocks | ok / 0.11 s 106 MB | ok / 0.6 s 54 MB | ok / 0.13 s 290 MB | ok / 0.6 s 398 MB | ok / 0.06 s 41 MB |
| INSERT of a missing block | ok, silent | ok, silent | ok, silent | ok (`Ref::Unresolved`) | ok |
| LWPOLYLINE claiming 2e9 vertices | ok | ok | ok | ok | ok |
| truncated ASCII DXF (50 %) | error | error / ok partial | ok partial | error | ok partial + "no EOF" |
| truncated binary DXF (50 %) | error | error / ok partial | ok partial | error | error |
| truncated DWG R2000/R2004/R2013/R2018 (50 %), 4 KiB | n/a | error or empty Ok / empty + diagnostic | clean errors | n/a | n/a |
| truncated DWG R2007 (50 %) | n/a | **panic** / **panic** | clean error | n/a | n/a |
| 200 bit flips R2000 / R2004 / R2018 DWG | n/a | error / 169 entities, **no diagnostic** / error | 169, 169, 66 entities with checksum warnings | n/a | n/a |
| random bytes with DWG magic (6 versions) | n/a | clean errors (one empty Ok) / empty doc | clean errors | n/a | n/a |
| real AC1006/AC1009 DWG (8 files) | n/a | errors / empty doc labelled AC1032 | "unrecognized file format" | n/a | version from magic |
| empty file, bad group code | ok empty / error | error / ok empty | error / ok empty + warning | error | ok / error |
| damaged ezdxf files (recover01, recover02, duplicate handles, broken layout links) | 2 errors | 1 error / all ok | all ok | 1 error | 1 error |

## 10. License notes

- All four crates and their full trees are permissive. acadrust is MPL-2.0 (file-level
  copyleft, fine to link; changes to its files must be shared).
- Corpus: ACadSharp samples MIT, ezdxf MIT, acadrust test files MPL-2.0, USFS drawings US
  public domain (via innerscene.com; that site also hosts files under its own license, only
  USFS items were taken), synthetic files generated. LibreDWG test data was not used (GPL
  repository, provenance of the files unclear). The cadxchange.net title block templates were
  not used: free to use, but no redistribution grant.
- No external programs: LibreDWG (GPL-3.0) and ODA File Converter (proprietary) are not
  used in any form.

## 11. Recommendation

- **DXF: own reader as the base, acadrust as an optional deep reader.** DXF is a simple pair
  format; 255 lines gave ASCII and binary DXF, header, layers with all flags, blocks, ATTDEF,
  ATTRIB (incl. multi-line), xrefs, TEXT, MTEXT with decoded codes, dimensions, MULTILEADER
  text, TABLE cell text, images, hatch names, XDATA, groups, XRECORDs, every unknown type by
  name, and cycle detection. It was the fastest and leanest reader. It still needs a lenient mode for damaged files:
  it failed on `ezdxf/recover01.dxf` (empty group-code line) that cadkit and acadrust failsafe
  read. What it does not do well yet: TABLE grid structure (cells are also listed twice, codes 1 and 302), dimension style tolerances,
  dynamic block parameters, `\U+` in names, code page handling for pre-2007 files. Rezzer's
  reader fills its own document model, so this keeps all knowledge items under Rezzer's
  control. Ready for the first release.
- **DWG: own in-process reader from the format specification.** The crate results are the
  comparison baseline, not the recommendation. What they show for the own reader: R2000 to
  R2018 are reachable in pure Rust (acadrust and cadkit both match the DXF twin exactly);
  the items worth the effort are TABLE cells, MULTILEADER text, dynamic block parameters,
  summary info, groups and XRECORDs (only acadrust has them today); corruption must be
  reported (verify page checksums and CRCs like cadkit does, acadrust does not); the reader
  must never panic on truncated input (acadrust does on R2007) and must take the version from
  the 6-byte magic. Until the own reader exists, Rezzer reports the DWG version and says the
  content cannot be read yet. The ACadSharp paired corpus (same drawing as DWG and DXF in
  every version, plus ACadSharp's own tree dump) is the regression test for it; acadrust and
  cadkit outputs in `results/` are reference outputs to diff against. Spec sources to
  check: acadrust ships `src/docs/OpenDesign_Specification_for_.dwg_files.pdf` inside its
  crate (check its terms before use); ACadSharp (MIT) is a legally clean reference
  implementation, and its samples plus `sample_base_tree.json` are already in the corpus.
- **Pre-R13 DWG (R12 and older):** no permissive reader works, and all 8 real government
  drawings in the corpus are this case. The own reader's spec should cover AC1009 (and if
  possible AC1006) as a separate, simpler format; until then report the version and that it
  cannot be read.
- **Not recommended:** dxf 0.6.2 (silent drops, invented metadata, text truncation, hard
  failures on real files), undxf (rendering model, memory use).

## 12. How Rezzer should present a drawing

- Frontmatter: format and version (`AC1032` = "AutoCAD 2018"), units (from $INSUNITS, with
  meters per unit), created/updated (local wall time, no time zone), last saved by,
  fingerprint GUID, summary info fields, code page, extents (header and computed, with a note
  when they differ).
- "Title block" section: every INSERT with attributes as a key/value table, ordered by the
  ATTDEF order, with prompt as description; flag blocks named like TITLE/TB/BORDER first.
- "Parts list" section: TABLE entities as Markdown tables; INSERT counts per block name as a
  BOM-like table (block, count, attribute columns such as PART_NO).
- "Annotations" section: TEXT and decoded MTEXT grouped by layout and layer, dimensions as a
  table (type, measured value with units, override text, style, tolerance from the style),
  leader texts.
- "Layers" table: name, color, on, frozen, locked, plot, linetype, description, entity count.
- "Layouts" table: name, paper size, viewports, scale.
- "References" list: xrefs (path, overlay), images and PDF underlays (paths), dynamic block
  states.
- Mermaid: block hierarchy (`graph TD` layout -> INSERT -> block -> nested block, with counts,
  cycles marked) and xref graph.
- "Not read" section: every unsupported entity or object type with count and reason
  (acadrust diagnostics, cadkit warnings, own reader unknown types), plus corruption warnings.
