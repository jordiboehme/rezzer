# Rezzer research: retro formats (old office files, disk and tape images)

Date: 2026-10-19. Scope: a "retro formats" release for Rezzer. Part A covers old word
processor, spreadsheet and database files. Part B covers floppy, disk and tape images as
containers. Part C recommends scope, order, shared building blocks and lists the license
blockers.

Sources and confidence labels (same as the 2026-10-16 reports):

- **[crates.io]** = version, date, license, downloads from the crates.io API, queried 2026-10-19.
- **[GH]** = license, stars, last push from the GitHub API (`gh api`), queried 2026-10-19.
- **[hdr]** = license read from the per-file source headers or the LICENSE/COPYING file.
- **[src]** = verified in the source code (no GPL or LGPL source was read).
- **[spec]** = from the format documentation linked in the row.
- **[est]** = our estimate (archive presence, effort).
- **[unverified]** = from memory, a secondary source or inference, not checked.

Rules that shape every recommendation here (from AGENTS.md and the task): one binary, every
reader in-process, no external programs; never run, emulate or disassemble code from a file;
link only MIT, Apache-2.0, BSD, ISC, Zlib or MPL-2.0 Rust crates; C/C++ only if permissive, pure
Rust preferred; GPL and LGPL code is never read for a reimplementation (only its license is
noted). Effort is in person-days (pd) for one Rust reader that fills Rezzer's document model,
with tests. Archive presence is rated 0 (practically never seen) to 5 (very common) [est].

Samples: 39 small files were downloaded to `spike/corpus/retro/` with source URL, license and
SHA-256 in `spike/corpus/retro/MANIFEST-retro.md`. They were only hashed and inspected with
`xxd`; nothing was executed.

---

## Summary table

| Family | Presence | Knowledge it carries | Public docs | Best permissive base | Blocker | Effort (pd) | Wave |
|---|---|---|---|---|---|---|---|
| WordPerfect 4.2 / 5.x / 6+ (.wp, .wpd) | 5 | styles, footnotes, headers, tables, comments, redline, document summary (about 50 fields in WP6) | medium (WP 4.2 spec, Corel FF SDK on Wayback) | port from libwpd (MPL-2.0 arm) | MPL port needs Jordi's go | 25-40 (+3-5 for 4.2) | 2 |
| WordStar 3.x-7.0 (DOS) | 3 | dot commands (.HE/.FO/.PN), emphasis toggles, footnotes, endnotes, annotations, comments (WS 5+) | good (official WS 7.0 file format text, 1992) | own reader from spec | none | 5-8 | 1 |
| man / mdoc / roff | 5 | title, section, date, sections, options, cross-refs (.Xr), tbl tables | very good (mandoc man pages, ISC) | own reader; mandoc (ISC) as reference | none | 10-15 | 1 |
| RTF 1.0-1.9.1 | 5 | \info metadata, styles, footnotes, annotations, revisions, fields | good (MS specs 1.0, 1.5, 1.9.1) | in core already (anydoc); add early-variant checks | none | 2 on top of core | 1 |
| dBase / FoxPro / Clipper (.dbf .dbt .fpt) | 5 | schema, record count, last-update date, code page, memo, deleted rows | good | `dbase` 0.8.0 (MIT) | none | 3-5 | 1 |
| Lotus 1-2-3 WKS / WK1 / Symphony WR1 | 3 | cells, formulas, named ranges, formats | good (Lotus WSFF 1984, public domain text) | own reader from spec | none | 3-4 + 3-4 formulas | 1 |
| SYLK (.slk), DIF | 2 | SYLK: values, R1C1 formulas, formats, names. DIF: values only | medium (SheetJS notes, Software Arts DIF spec) | own reader | none | SYLK 4-6, DIF 1-2 | 1 |
| Lotus WK3 / WK4 / .123 | 3 | 3D sheets, names, styles, formulas | none public | port from libwps (MPL) or Apache OpenOffice (Apache-2.0) | none with AOO | 8-12 + 4-6 | 2 |
| Quattro Pro (.wq1 .wq2 .wb1-3 .qpw) | 2 | notebooks, formulas, names, styles | notes only | libwps (MPL), AOO qpro (Apache-2.0) | none | 10-15 + 5-7 | 2 |
| MS Works (.wps .wks .wdb), Word for DOS, XyWrite, MS Write | 3 | styles, headers, footnotes, OLE summary (Works 4.5+), formulas (.wks), fields (.wdb) | none official | libwps (MPL) | MPL port | about 40 together | 2 |
| Excel 2-4 BIFF (raw, no OLE) | 2 | values, formulas, names, formats | good (Rentz excelfileformat.pdf, Apache-2.0) | own reader; calamine cannot read it [src] | none | 5-7 + 4-6 | 2 |
| FrameMaker MIF | 2 | paragraph and character catalogs, cross-refs, markers, variables, conditional text, tables | excellent (Adobe MIF Reference) | own reader; Tika MIFParser (Apache-2.0) | none | 10-15 | 2 |
| Mac word processors: MacWrite, MacWrite II/Pro, WriteNow, ClarisWorks/AppleWorks | 2 | styles, footnotes, headers, DB/SS parts; Finder metadata | poor (MacWrite 1-5 tech note only) | libmwaw (MPL) | MPL port; needs Mac container layer | 45-65 | 3 |
| Paradox (.db .px .mb) | 2 | schema, primary key, record count, blobs | medium (Randy Beck text) | paradoxReader (BSD-3, Java) | none | 6-9 | 3 |
| Ami Pro (.sam) | 2 | named styles, notes, footnotes | community RFC (MIT, see caveat) | amipro-sam (MIT) | spec informed by LGPL code | 5-8 | 3 |
| DCA/RFT, DisplayWrite | 1 | document profile, text and format units, footnotes | good (IBM manuals on bitsavers) | own reader | no samples | 10-15 | 3 |
| Lotus Word Pro, WordStar 2000 / Win, PFS:Write, Interleaf, Framework, VisiCalc | 0-2 | varies | none or poor | Word Pro: LGPL/SISSL only | license or no docs | defer | detect + "not readable" |
| Container layer (shared, also for ZIP/TAR/ISO) | - | entries with name, type, size, dates, forks, labels, nested detection | - | own | model gap (see C.2) | 6-8 | 0 |
| Commodore D64 / D71 / D81 / T64 / PRG / P00 / LNX / CRT | 4 | disk name, ID, DOS type, BAM, free blocks, file type, locked, load address, error bytes | excellent (unusedino.de) | own reader; unarc-rs and `cbm` (MIT) as reference | none | 10 | 1 |
| C64 BASIC V2/V7, PETSCII text | 4 | program listing as text, line numbers | good | own detokenizer | PETSCII table license | 3 | 1 |
| GEOS (CVT, VLIR, info block), geoWrite | 2 | author, class, description, dates, documents | good (geos.html) | own reader | no clean sample | 6 | 2 |
| Atari 8-bit ATR / XFD / DCM + DOS 2.x / MyDOS / SpartaDOS + Atari BASIC | 3 | files, dates (SpartaDOS), locked, sector layout | medium | own; atrcopy (MPL-2.0, Python) as reference | ATASCII table license | 11 | 1 (ATR, DOS 2, Sparta) |
| Atari ST .ST / MSA / STX | 3 | FAT12 tree with dates, volume label | medium | `fatfs` 0.3.6 (MIT) | none | 5 | 1 (ST, MSA) |
| Apple II DSK / DO / PO / 2MG / NIB / WOZ, DOS 3.3 / ProDOS / Pascal, Applesoft | 3 | catalog, file types, aux type, dates (ProDOS), WOZ META (title, publisher) | excellent (CiderPress II docs, Apache-2.0) | port from CiderPress II; `a2kit` (MIT) as reference | none | 12-16 | 1 (sector images) |
| Classic Mac: MacBinary, BinHex, AppleSingle/Double, resource forks | 4 | type/creator, dates, Finder flags, comments, both forks | good (RFC 1740, 1741, specs) | cyco crates (Apache-2.0) | none | 1-3 | 1 |
| Classic Mac: HFS, MFS, DiskCopy 4.2, NDIF, StuffIt, Compact Pro | 3 | full catalog with type/creator, dates, forks | good (Inside Macintosh) | `fstool`, `macfs`, `sit-rs`, `compact-pro` | young crates | 8-12 | 2 |
| Amiga ADF (OFS/FFS/INTL/DCFS/DOS6-7), HDF/RDB, DMS, LhA | 3 | volume name, dates, protection bits, file comments | good (ADF FAQ, xDMS public domain) | `amiga-ffs`, `amiga-rdb`, `unarc-rs`, `delharc` | WTFPL for LZX | 5-7 | 1 (ADF, LhA) |
| PC floppies IMG/IMA (FAT12), IMD, TD0, CQM | 4 | FAT tree with dates, volume label, IMD/TD0 comment header | good (fatgen103, MAME BSD-3 files) | `fatfs`/`hadris-fat`, `retrocompressor`, fluxfox (MIT) | fluxfox crates.io release stale | 5-7 | 1 (IMG, IMD), 2 (TD0, CQM) |
| ZX Spectrum TAP / TZX / TRD / SCL / +3 DSK / MDR, Sinclair BASIC | 3 | header blocks (name, type, start line), TZX archive info, text blocks | good (TZX spec, Sinclair wiki) | own reader; Clock Signal (MIT) as reference | ZX table license | 12 | 1 (TAP, TZX byte blocks) |
| Amstrad CPC DSK/EDSK + AMSDOS, CP/M disks | 2 | AMSDOS headers, user areas, CP/M directory | good (seasip.info, CPCEMU spec) | `cpclib-disc` (MIT), `a2kit` DPBs | do not copy cpmtools diskdefs (GPL) | 10 | 2 |
| Pulse, bitstream and flux: C64 TAP, CSW, G64, P64, ATX, STX, WOZ 2.1 FLUX, IPF, SCP, KryoFlux | 2 | standard-loader files only | varies | Clock Signal (MIT), ipf-flux (BSD-3), P64 code (Zlib) | CAPS library forbidden | list only in wave 1 | 3 (partial decode) |

---

## A. Old word processors, spreadsheets and databases

### A.1 Reference libraries and their licenses

The libwpd family covers most of this list. Its license decides whether Rezzer can use it.

| Project | Formats here | License | Status |
|---|---|---|---|
| libwpd 0.10.3 (sourceforge.net/p/libwpd) | WordPerfect DOS 4.2, 5.x, 6.x to X-series; WP Mac 1-3.5 | MPL-2.0 / LGPL-2.1+ dual [hdr: WPDocument.cpp; COPYING.MPL and COPYING.LGPL in tree] | usable via the MPL-2.0 arm |
| libwps 0.4.14 (sourceforge.net/p/libwps) | MS Works WP (DOS, Win, WPS8 OLE), Word for DOS, MS Write, XyWrite, PocketWord, Lotus WKS/WK1/WK3/WK4/.123, Symphony, Quattro WQ1-QPW, Works WKS/WDB, Multiplan DOS | MPL-2.0 / LGPL-2.1+ dual [hdr: WPSDocument.cpp, Lotus.cpp, QuattroFormula.cpp, Multiplan.cpp] | usable via the MPL-2.0 arm |
| libmwaw 0.3.23 (2026-07-19) | MacWrite (all), MacWrite II/Pro, WriteNow, ClarisWorks/AppleWorks WP/SS/DB, Mac Word 1-5.1, Mac Works, Nisus, Wingz, Jazz, about 60 parsers | MPL-2.0 / LGPL-2+ dual [hdr: MWAWDocument.cxx, ClarisWksDatabase.cxx] | usable via the MPL-2.0 arm |
| librevenge 0.0.6 | interface layer | MPL-2.0 / LGPL-2.1+ [hdr] | not needed in Rust |
| libabw, libetonyek | AbiWord; Pages, Keynote, Numbers | MPL-2.0 only [GH] | out of scope here |
| LibreOffice `sc/source/filter/{lotus,qpro,dif}`, `sc` SYLK import | Lotus, Quattro WB2, DIF, SYLK | MPL-2.0 per file, incorporating Apache-2.0 work [hdr]. GitHub shows GPL-3.0 only because the root COPYING is GPLv3 | readable |
| Apache OpenOffice (github.com/apache/openoffice) `main/sc/source/filter/lotus` | Lotus WK1-WK4 (lotread, lotform, op, optab) | Apache-2.0 [GH] | cleanest reference for Lotus |
| LibreOffice `lotuswordpro/` | Lotus Word Pro .lwp | LGPL-2.1 / SISSL-1.1 (IBM 2008) [hdr: lwpfilter.cxx, bento.hxx] | **blocked**: do not read |
| Apache Tika WordPerfectParser, MIFParser; Apache POI OldExcelExtractor | WP 5/6 text and metadata, MIF, BIFF2-5 text | Apache-2.0 | readable, text level only |
| mandoc (mandoc.bsd.lv) | man(7), mdoc(7), roff subset, tbl, eqn | ISC for the distribution [hdr: LICENSE v1.23, checked] | readable; C, so reference only |
| CiderPress II (fadden/CiderPress2) | Apple II AppleWorks WP/SS/DB | Apache-2.0 [GH] | readable and portable |
| ctrl-kd, soft-return (jonmichaels) | WordStar DOS 4-7 | MIT [GH], created 2026-07 | readable; has MIT samples |
| amipro-sam (gadicc) | Ami Pro .sam, with its own format RFC | MIT [GH]; README says it was "informed by" an archived LGPL KOffice importer | readable with caution (see C.4) |
| dbfread (olemb) | DBF + memo | MIT [hdr] | readable |
| teverett/paradoxReader | Paradox .db/.px/.mb | BSD-3-Clause [GH] | readable |
| xberg-libwpd 1.3.10 | WPD via C++ FFI | crate says MIT, but it vendors libwpd + librevenge (MPL arm) and Boost (BSL-1.0) [src: vendor/PROVENANCE.md] | not pure Rust, C++ link: no |
| unfair-codec-wk1, -qpw, -wp, -legacy (crates.io, 2026-09-07, 0.1.0) | WK1, QPW, WP, Write, Word DOS, MacWrite, ClarisWorks | MPL-2.0 on crates.io, but the repo github.com/taunio/unfair-office returns 404; the wk1 crate names Gnumeric (GPL) as a "source of truth" [src: crate docs] | **do not use** (provenance) |
| GPL or LGPL, note only, never read | pandoc (GPL-2.0), groff (GPL-3.0+ [unverified]), AbiWord (GPL-2.0), antiword (GPL-2.0 [unverified]), Gnumeric plugins (GPL), pxlib (GPL-2.0), wsconvert and fpwsconv (GPL-3.0), wordstar-rs (GPL-2.0), WP_Reader (LGPL-3.0), sylk_parser (GPL-3.0), KOffice/Calligra filters (LGPL) | | |

Two findings on Rust crates:

- **No mature pure-Rust reader exists** for any format in part A except DBF (`dbase`) and RTF
  at the tokenizer level (`rtf-parser` 0.4.3 MIT, `rtf-grimoire` 0.2.1 MIT). Small roff crates
  exist (`man_parser` 0.1.0, license mismatch MIT vs Apache-2.0 between crates.io and GitHub;
  `libmandoc-rs` 0.12.0 is FFI to vendored C mandoc). `ws2markdown` 0.3.0 is CDDL-1.1, not
  on the allow list.
- **calamine 0.37.0 does not read pre-OLE Excel 2-4 files** [src: `src/xls.rs` always calls
  `Cfb::new` and only looks up the `Workbook`/`Book` streams; the `Biff2..4` enum only applies
  when such a version sits inside a CFB container]. Our sample `formula_import_biff2.xls` starts
  with the raw BOF record `09 00`, no OLE header, so calamine cannot open it. calamine also has
  an open issue #719 (2026-10-18): cross-sheet references in decoded xls formulas name the wrong
  sheet. That matters for the core spec, not only for retro.

### A.2 Word processors

**WordPerfect DOS 4.2, 5.x, 6.x+, Windows .wpd**

- Presence 5. The most common legacy word processor format in institutional archives. The
  Library of Congress holds close to 100,000 files, mostly in its Manuscript Division
  (https://blogs.loc.gov/thesignal/2023/12/growing-the-file-format-fam/). Legal and government
  archives are full of it.
- Knowledge (richest in part A): WP5 prefix packets (fonts, graphics), styles,
  footnotes/endnotes, headers/footers, tables, document summary (descriptive name, type,
  author, typist, subject, account, keywords, abstract, dates) [WP5 summary fields
  unverified]. WP6+ extended document summary with about 50 fields (libwpd emits creator,
  initial creator, creation and revision date, revision number and notes, version, typist,
  subject, keywords), comment annotation packets, hyperlinks, outline and named styles,
  columns, and redline/strikeout used as tracked changes. Magic `FF 57 50 43` ("\xFFWPC") with
  a product and file type byte; our samples show major version `00`/`02` at offset 10 [spec].
- Docs: medium. WP 4.2 spec on the 1994 PerfectFit SDK ISO (archive.org
  `wordperfectsdkperfectfit1994`, file `51PCSDK/WP42FF.TXT`, no license stated). Corel File
  Format SDK pages on the Wayback Machine:
  https://web.archive.org/web/20021022035637/http://www.corel.com/partners_developers/ds/CO32SDK/docs/ff/A_FRNTFF.htm.
  LoC FDD: https://www.loc.gov/preservation/digital/formats/fdd/fdd000621.shtml.
- Readers: libwpd (MPL arm), Tika (Apache-2.0, text only). No Rust crate.
- Samples: `WP5.wp`, `WP6.wpd` (LibreOffice), `testWordPerfect_5_1.wp` (Tika).
- Effort: 25-40 pd for WP 5.x and 6.x+ with summary, comments, footnotes, tables and styles,
  as an MPL-2.0 port with libwpd as the reference. WP 4.2 adds 3-5, WP Mac adds 8-12.

**WordStar DOS 3.x to 7.0 (incl. 4.0, the version George R.R. Martin writes on)**

- Presence 3. Huge 1980s base; many author and personal archives. LoC holds "several
  hundred" files [unverified count].
- Knowledge: dot commands (.HE/.FO headers and footers, .PA page break, .PN page number, .OP),
  `..` comment lines (author notes), emphasis as toggle control codes (bold, underline,
  italic), high bit set on the last letter of words (soft formatting, must be stripped). From
  5.0: "symmetrical sequences" carry footnotes, endnotes, annotations, comments and font tags,
  and a leading sequence holds a document header. Our WS7 sample starts with `1D 7D 00 00`
  followed by the printer name, the WS4 sample starts directly with `.op` [spec, xxd]. No
  author/title summary in the DOS versions [unverified for 7.0].
- Docs: good. Official "File Format for WordStar Release 7.0" (WordStar International,
  1992-03-17): https://www.sfwriter.com/wsformat.txt. LoC FDD:
  https://www.loc.gov/preservation/digital/formats/fdd/fdd000552.shtml. WordStar 3.3 reference
  manual on bitsavers. wordstar.org has history and command pages but no file spec.
- Readers: ctrl-kd (MIT, Python) and soft-return (MIT, Swift). Others are GPL or CDDL.
- Samples: `LYING.WS` (WS7, with a footnote) and `OCAPTAIN.WS` (WS4), both MIT transcriptions
  of public-domain texts.
- Effort: 5-8 pd for WS 3-7 from the public spec. Best knowledge per day in part A.
- WordStar 2000 and WordStar for Windows are different formats with no public docs and no
  open reader [unverified]: 10-15 pd each of reverse engineering. Defer.

**MS Word for DOS 1.x to 6.0**

- Presence 2. Magic `31 BE` (our sample), PRONOM x-fmt/274-276. Styles live in a separate
  .STY file, so style names (roles) need the companion file. Footnotes, running heads,
  summary sheet (author, operator, keywords, comments, dates) [unverified].
- Docs: poor; MS-DOC covers Word 97+ only. Reader: libwps DosWord.cpp (MPL arm). Effort 8-12 pd.

**MS Works word processor (.wps)**

- Presence 3 (bundled on millions of home PCs). Works DOS 2/3 and Win 3/4 (WPS4) is flat
  binary; Works 4.5 to 9 (WPS8) is an OLE file with a SummaryInformation stream, so title,
  author and dates come from the same CFB property reader that DOC/XLS need. Our
  `Works_3.0.wps` is already OLE.
- Docs: none official. Reader: libwps (MPL arm). Effort 15-20 pd for both families.

**XyWrite / Nota Bene**

- Presence 1-2, concentrated among journalists, publishers and academics. CP437 text with
  embedded commands in guillemets: styles, footnotes, headers, comments. Docs: manuals only.
  Reader: libwps XYWrite.cpp (MPL). Effort 4-6 pd. No sample with a clear license found.

**Ami Pro (.sam) and Lotus Word Pro (.lwp)**

- Ami Pro: presence 2. ASCII-tagged sections ([ver], [sty], [edoc]); named paragraph styles
  become roles; notes and footnotes [unverified detail]. Best description: the amipro-sam RFC
  (https://github.com/gadicc/amipro-sam/blob/HEAD/spec/sam-format.md, MIT). Effort 5-8 pd.
- Word Pro: presence 2 (SmartSuite 96 to 9.8, Japan and some governments). Bento container,
  full object model. The only reader is LibreOffice `lotuswordpro/` under LGPL-2.1 / SISSL-1.1.
  No public docs. Effort 30-50 pd. **Defer and report "not readable" with the reason.**

**Mac word processors: MacWrite 1-5, MacWrite II/Pro, WriteNow, ClarisWorks/AppleWorks 5-6**

- Presence 2 (Mac and school archives). Knowledge: styles (named from ClarisWorks 4),
  footnotes, headers/footers; much metadata sits in the Finder info and the resource fork, so
  these readers depend on the Mac container layer (B.2). ClarisWorks is one zone-based format
  shared by WP, draw, SS and DB.
- Docs: MacWrite 1-5 in Apple tech notes (copy at http://web.mit.edu/~jik/sipbsrc/src/mcread/mwformat,
  license unknown); nothing public for the rest.
- Reader: libmwaw (MPL arm), "all versions" per its wiki
  (https://sourceforge.net/p/libmwaw/wiki/Home/).
- Effort: MacWrite 5-7, WriteNow 6-8, MacWrite II/Pro 10-15, ClarisWorks 20-30 (WP part)
  plus 8-12 for SS/DB and 4 for formulas.
- Apple II AppleWorks (.awp/.asp/.adb) is a separate, simpler format documented in the Apple
  II File Type Notes ($19/$1A/$1B) and CiderPress II (Apache-2.0,
  https://github.com/fadden/CiderPress2/blob/main/FileConv/Doc/AppleWorks-notes.md):
  WP 3-5 pd, SS 2-3 + 2 formulas, DB 2 pd.

**PFS:Write, Interleaf**

- PFS:Write: presence 1 (PRONOM fmt/1414 exists, so collections exist); no docs, no reader.
- Interleaf: presence 1 (defense and engineering); ASCII markup with component tags, binary
  undocumented (only https://www.w3.org/Tools/interleaf.html).
- Both: detect by signature and report "not readable: no public documentation". Defer.

**DisplayWrite and DCA/RFT, DCA/FFT**

- Presence 1 (IBM DisplayWriter and OfficeVision office archives). EBCDIC structured fields:
  text units, format units, a document profile (author, comments, dates [unverified detail]),
  headers, footers, footnotes.
- Docs: good. IBM SC23-0758-0 "Document Content Architecture: Revisable-Form Text Reference"
  (1983) and SC23-0757-0 (Final-Form Text) on bitsavers.org/pdf/ibm/sna/dia/; DisplayWrite 4
  Technical Reference on bitsavers. IANA `application/dca-rft`.
- No open reader, no clean sample. Effort RFT 10-15, FFT 5. A clean-room reader from the IBM
  manuals is possible. Needs an EBCDIC code page (C.3).

**FrameMaker MIF**

- Presence 2 (technical documentation departments). Fully textual and very rich: paragraph and
  character format catalogs (named roles), variables, cross-references, markers (index,
  hypertext, comments), conditional text, tables, footnotes, document properties, book files.
- Docs: excellent. Adobe MIF Reference: https://help.adobe.com/en_US/framemaker/mifreference/mifref.pdf.
- Readers: Tika MIFParser (Apache-2.0), mif2xml-j (Apache-2.0; its C++ sibling is GPL). No
  Rust crate. Sample `testMIF.mif` starts with `<MakerFile 5.5Q>`.
- Effort 10-15 pd. Binary .fm and .book: detect only and ask for MIF.

**troff / nroff / man pages (man, mdoc)**

- Presence 5 in software archives (every Unix system and source tarball).
- Knowledge: mdoc is semantic (.Nm name, .Fn, .Fl flags, .Ar, .Xr cross-references, .Dd date,
  .Dt title and section, .Os, .Sh/.Sx). man has .TH (title, section, date, source, manual),
  .SH/.SS, .TP lists. tbl gives real tables, eqn gives math. The .Xr links build a graph.
- Docs: very good. https://mandoc.bsd.lv/man/mdoc.7.html and https://mandoc.bsd.lv/man/man.7.html
  (ISC), https://man.openbsd.org/roff.7. groff_man(7) may be read for facts, groff code may not.
- Readers: mandoc (ISC, C), doclifter (BSD-2-Clause, Python; good reference for lifting ms/me
  macros), libmandoc-rs (FFI). pandoc is GPL: do not read.
- Samples: `mdoc.7` and `man.7` (ISC, mandoc sources).
- Effort 10-15 pd for man + mdoc + basic roff requests + tbl in pure Rust. Full roff with user
  macro packages (ms, me, mm) is not feasible: report "custom macros not interpreted".

**RTF 1.0 to 1.9.1**

- Presence 5. Already planned in the core spec via anydoc 0.2.4. The retro work is a check that
  early variants work: RTF 1.0 has no `\uN` (from 1.5), uses `\ansi`/`\mac`/`\pc`/`\pca`
  charsets (Mac Roman, CP437, CP850 tables needed, see C.3) and has no `\listtable`
  [unverified exact version boundaries]. Annotations (`\atnid`, `\atnauthor`, `\annotation`,
  `\atrfstart`/`\atrfend`) and revisions (`\revauth`, `\revdttm`, `\revtbl`) must land in the
  annotations and timeline facets; check whether anydoc exposes them [unverified].
- Docs: RTF 1.0 (MS application note GC0165, may be copied unmodified):
  http://www.textfiles.com/programming/FORMATS/rtfspec.txt; 1.5:
  http://www.biblioscape.com/rtf15_spec.htm; 1.9.1 on the Wayback Machine.
- Effort: about 2 pd on top of the core RTF reader; 12-18 pd if Rezzer writes its own.

### A.3 Spreadsheets and databases

**dBase III/IV, FoxPro, Clipper (.dbf, .dbt, .fpt; indexes .ndx .mdx .cdx .ntx)**

- Presence 5 (shapefiles keep .dbf alive, plus legacy business data).
- Knowledge: field definitions (name, type, length, decimals), record count, last-update date
  (YYMMDD in header bytes 1-3), code page mark (byte 29), deleted flags, memo text, VFP field
  flags and the .dbc link. Index files: list only, maybe the key expression.
- Docs: https://www.clicketyclick.dk/databases/xbase/format/index.html,
  http://www.dbase.com/KnowledgeBase/int/db7_file_fmt.htm, Microsoft VFP "Table File
  Structure", LoC fdd000325.
- Crate: **`dbase` 0.8.0** [crates.io: MIT, 2026-07-05, 1.30M downloads; GH: MIT,
  tmontaigu/dbase-rs, pushed 2026-09-12]. Public `Header` with `last_update`, `num_records`,
  `code_page_mark`; reads memo .dbt and .fpt; types C D F N L Y T I B M; no index files.
  Its optional `datafusion` feature is heavy: keep it off.
- Samples: `memotest.dbf` + `memotest.FPT` (MIT, dbfread). More: dbase-rs tests (MIT), Natural
  Earth .dbf (public domain).
- Effort 3-5 pd (schema table, records as a table, memo, header metadata, code page, deleted rows).

**Lotus 1-2-3 WKS / WK1 and Symphony WRK/WR1**

- Presence 3. Knowledge: cells, formulas (RPN with relative and absolute references), named
  ranges, cell format byte (type and precision, dates), column widths, hidden columns, print
  and graph settings, table and query ranges. Our `Lotus.wk1` starts with BOF `00 00 02 00 06 04`
  (0x0406 = WK1).
- Docs: good. Lotus "Worksheet File Format" (1984), WSFF1-5.TXT (WSFF2 records, WSFF4 formula
  layout, WSFF5 opcodes), released "into the public domain" by Lotus. Copies at
  https://raw.githubusercontent.com/SheetJS/notes/main/lotus/WSFF2.TXT (the SheetJS notes repo
  itself has no license file; read it for facts only).
- No Rust crate (unfair-codec-wk1 excluded, see A.1).
- Effort: reader 3-4 pd, formula decoder 3-4 pd. A shared Lotus-family formula engine (WK1,
  WKS, Works, Quattro DOS, Symphony share opcodes) saves about 30% across those [est].

**Lotus WK3 / WK4 / .123**

- Presence 3. 3D multi-sheet, named ranges, styles (WK3 keeps them in a separate .FM3 file),
  formulas, charts. No public spec. References: Apache OpenOffice `lotread.cxx` (Apache-2.0,
  has WK3/WK4 sections) and libwps (MPL). .123 (97/98, Millennium) is a different record layout.
- Samples: `testLotus123.wk3` (Tika), `Lotus123_98.123` (LibreOffice).
- Effort: reader 8-12 pd, formulas 4-6 pd.

**Quattro Pro (.wq1 .wq2 DOS, .wb1 .wb2 .wb3 Windows, .qpw 9+)**

- Presence 2. Notebooks with multiple sheets, formulas, names, styles. WB3 and QPW sit in CFB
  (stream `PerfectOffice_MAIN` or `NativeContent_MAIN`). WQ1 is Lotus-like (our sample has BOF
  0x5120).
- Docs: only community notes (SheetJS notes `qpw/README.md`). References: libwps Quattro*.cpp
  (MPL), LibreOffice/AOO `qpro` (WB2).
- Effort: WQ1/WQ2 2-3, WB1-3 4-6, QPW 4-6, formulas 5-7 pd.

**MS Works spreadsheet (.wks) and database (.wdb)**

- Presence 2. Cells and formulas (Lotus-like); field definitions, records and form/report
  layout. No public docs. Reference libwps WKS4*.cpp (MPL). Effort SS 3-4 + 2-3 formulas, WDB
  4-5 pd.

**Multiplan and SYLK (.slk)**

- SYLK: presence 2. ID;P header, C records (values, E = R1C1 expression), F formats, P number
  format table, NN names, O options, shared formulas, comments. Docs: Wikipedia
  (https://en.wikipedia.org/wiki/Symbolic_Link_(SYLK)) and SheetJS notes `sylk/README.md`; no
  Microsoft spec. No Rust crate (`sylk` on crates.io is a Dioxus framework). Sample
  `ooo80484-1.slk`. Effort reader 2-3 + R1C1 parser 2-3 pd.
- Multiplan DOS binary: presence 1. libwps Multiplan.cpp (MPL). Effort 4-5 + 3 pd.

**DIF**

- Presence 2. Values only, with a header (TABLE, VECTORS, TUPLES, LABEL), no formulas.
  Docs: Software Arts "DIF Technical Specification" (1983 scan,
  https://www.atarimania.com/8bit/files/DIF%20Technical%20Specification.pdf). Watch out: Tika's
  `.dif` test files are NASA Directory Interchange Format XML, not spreadsheet DIF. Sample
  `universal-content.dif`. Effort 1-2 pd.

**Excel 2-4 BIFF (raw .xls, .xlw)**

- Presence 2. Values, formulas (RPN tokens close to BIFF5), names, formats.
- Docs: Daniel Rentz "Excel File Format" (BIFF2-8,
  https://svn.apache.org/repos/asf/openoffice/ooo-site/trunk/content/sc/excelfileformat.pdf,
  openoffice.org site content Apache-2.0).
- calamine cannot read it (A.1). Effort reader 5-7 + formulas 4-6 pd. Rezzer's own BIFF formula
  token decoder can then also fix calamine's cross-sheet bug for BIFF5/8 if needed.
- Samples: `formula_import_biff2.xls` (openoffice.org testdocs); more BIFF3/4 files on the same
  page and in Apache POI test-data.

**Paradox (.db, .px, .mb, .Xnn/.Ynn)**

- Presence 2-3 (Delphi/BDE business data). Schema (incl. BCD, blob, graphic, autoinc), record
  count, primary key (.px), memo/blob (.mb), sort order, code page, password flag.
- Docs: Randy Beck "The PARADOX File Structure" (a format text; the only copy found sits in the
  GPL pxlib repo with no license of its own, so use it for facts with care). PRONOM fmt/350-352.
- References: paradoxReader (BSD-3, Java), ParadoxTableBrowser (MIT, Pascal). pxlib (GPL-2.0):
  note only. Effort 6-9 pd; encrypted tables are reported as unreadable.

**ClarisWorks / AppleWorks spreadsheet and database (.cwk)**: see A.2 (libmwaw, MPL).
Sample `ClarisWorks_DB_5.0.cwk` (magic `BOBO` at offset 4).

**VisiCalc (.vc), Framework (.fw2-.fw4), Lotus Symphony beyond WR1**

- VisiCalc: presence 1; a text keystroke stream; manuals on bitsavers, grammar in
  visicalc_engine (BSD-3, Dart). Effort 2 + 2 pd. No sample found.
- Framework: presence 0-1; no spec, no reader, no sample. Detect and say "not readable".

---

## B. Floppy, disk and tape images as containers

### B.1 How Rezzer should treat an image

An image is a container, like ZIP. Rezzer:

1. Detects the image format (magic, size, layout probes).
2. Reads the volume knowledge: disk name, ID, DOS type, volume label, BAM or free-space map,
   free blocks, interleave, error bytes, copy-protection marks, WOZ/IMD/TD0 comment headers,
   TZX archive info blocks. These go into the document as a section and into properties.
3. Lists every entry with name (raw bytes and decoded), type, size in blocks and bytes, dates
   if the filesystem has them, load address, locked/protection flags, type/creator codes,
   Finder comments, forks, record length (REL), user area (CP/M).
4. Hands each file to detection and the normal readers, with nesting (a .sit inside a
   MacBinary inside an HFS image inside a DiskCopy file). BASIC programs are detokenized to a
   text listing. Machine code (PRG with a non-BASIC load address, CRT, XEX, executables) is
   listed with its load address and size, never run, emulated or disassembled.
5. Says what it could not read: protected or unreadable tracks, custom loaders, compressed
   methods it does not know, encrypted DMS, all with the reason.

### B.2 Formats, docs, parsers, complexity

Effort is for a reader on top of the shared container layer (C.2), with tests.

#### Commodore (C64, C128, VIC-20, PET)

| Format | Docs | Level | Notes | Effort |
|---|---|---|---|---|
| D64 (35, 40, 42 tracks; with error bytes: 175531 / 197376 bytes) | http://unusedino.de/ec64/technical/formats/d64.html | sector | BAM at 18/0: disk name, ID, DOS type; 40-track BAM extension differs per DOS (DolphinDOS $AC, SpeedDOS $C0) [spec]; error bytes reported per sector as knowledge | 3 |
| D71, D81 (incl. partitions), D80/D82 | unusedino.de d71.html, d81.html, d80-d82.html | sector | | 4.5 |
| T64, PRG, P00 (PC64), LNX, CRT | unusedino.de t64.html, pc64.html, crt.html; unarc-rs doc/lynx.md | byte | T64 end addresses are often wrong (unarc-rs has a fixture for it); CRT: list chips and banks only | 3 |
| GEOS (CVT, VLIR, info block), geoWrite | unusedino.de geos.html; https://ist.uwaterloo.ca/~schepers/formats/GEOS.TXT; https://www.pagetable.com/?p=1471 | sector | info block has class, author, description, dates: real knowledge. geoWrite pages and ruler escapes [partly documented] | 6 |
| BASIC V2 / V7 detokenize | https://www.c64-wiki.com/wiki/BASIC_token, .../BASIC_7.0 | data | V7 has two-byte $CE and $FE prefixes; dialect hint from load address ($0801 C64, $1C01 C128, $1001 VIC-20 or C16/Plus4: ambiguous) [unverified addresses] | 2 |
| PETSCII SEQ text | mapping tables (C.3) | data | control codes (colour, cursor, case switch $0E/$8E) handled by Rezzer | 1 |
| G64 | unusedino.de g64.html | GCR bitstream | standard GCR decodes to sectors; protected tracks (half tracks, killer tracks) listed only | 3 |
| P64 | no spec; reference code by B. Rosseaux in the VICE tree (`src/lib/p64/p64.c`, `p64.h`) is **Zlib** [hdr] | flux | flux to GCR to sectors, partial | 5 |
| C64 TAP | unusedino.de tap.html | pulse | decode the standard ROM loader only; turbo loaders: "not decoded, custom loader" | 4 |

Crates and references:

- **unarc-rs 0.8.0** [crates.io: MIT OR Apache-2.0, 2026-10-18; GH: Apache-2.0, mkrueger]. Reads
  D64 (35/40/42 tracks), T64, LNX, ATR (DOS 2.0S/2.0D/2.5, MyDOS, SpartaDOS/BW-DOS with dates),
  FAT12 .ST, ADF, DMS, LHA and many archives. It is an extractor: for D64 it exposes name, raw
  PETSCII name, type, flags, blocks, `disk_name()`, but not the disk ID, DOS type, BAM, free
  blocks or error bytes, and it does not interpret GEOS VLIR. No D71/D81. Its default `zstd`
  feature builds C `zstd-sys` and it always pulls `rars`. Best use: reference docs
  (`doc/d64.md`, `doc/atr.md`) and fixtures; as a dependency only with `default-features =
  false` and only for raw extraction.
- `cbm` 0.1.0 (MIT/Apache-2.0, 2018, stale): D64/D71/D81, GEOS border directory, info block,
  VLIR, REL, PETSCII. Best metadata coverage, but depends on `memmap 0.6` (unmaintained) and
  `clap 2`. Read and port, do not depend.
- `petscii` 0.2.1: license-file with MIT text (needs a cargo-deny clarify entry), repo 404.
- Clock Signal (TomHarte/CLK, MIT, C++, active): D64, G64, CommodoreTAP, TapePRG and the
  Commodore tape-loader decoders. Best permissive reference to read.
- cc1541: MIT per packager metadata [unverified upstream file].
- **Excluded (GPL or LGPL): never read.** The `format198x-*` crate family (C64 BASIC, ZX, CPC,
  Atari XEX; GPL-2.0-or-later, 2026), `play198x-core`, `zinc64-*`, `rs1541`, `xum1541`,
  `remanence`, VICE c1541 and petcat (except the Zlib P64 files), python-d64, cbmcodecs,
  1541ultimate. DirMaster is closed.
- Samples: `test.d64` (unarc-rs, all file types, two directory sectors), `calc64.d64` (MIT
  homebrew; BAM shows disk name CALC64, ID CX, DOS type 2A), `test.t64`, `basic.lnx`. More MIT
  homebrew: davervw repos (C64, C128, VIC-20 D64s), ReadyOS (D81, D71), eightbitjim (VIC-20 CRT).
  No clean GEOS sample: GEOS disks contain Berkeley Softworks code.

#### Atari 8-bit

| Format | Docs | Level | Notes | Effort |
|---|---|---|---|---|
| ATR | unarc-rs `doc/atr.md` (Apache/MIT); Jindroush description (mirror on ftp.pigwa.net) | sector | magic $0296; paragraph count often wrong, derive size from file length; first three sectors are 128 bytes on DD disks | 1 |
| XFD | raw | sector | | 0.5 |
| DCM | Ernest Schreurs DCM spec v3.2 (https://atariarchives.org/cfn/05/07/0060.php) | compressed sector | | 2 |
| Atari DOS 2.x, MyDOS, SpartaDOS | unarc-rs doc/atr.md; Ken Siders DOS notes (ftp.pigwa.net mirror) | filesystem | SpartaDOS 23-byte entries with dates | 4 |
| Atari BASIC | https://gury.atari8.info/refs/file_formats_bas.php | data | needs the variable name table | 2 |
| CAS (Atari) | https://a8cas.sourceforge.net/format-cas.html | byte | "FUJI" chunk; standard 132-byte records decode, turbo/FSK listed | 1.5 |
| ATX (VAPI) | no official spec [unverified pointers] | sector + timing | filesystem readable when no duplicate sectors; weak sectors reported as protection | 3 |

References: **atrcopy** (robmcmullen, Python, **MPL-2.0**, same license as Rezzer) for ATR, XFD,
DOS 2/2.5, XEX. bw-atari8-tools main directory is WTFPL (not on the allow list; facts only),
its `ksiders/` (no license) and `jindroush/` (GPL-2) parts must not be read. jhallen/atari-tools
has no license: all rights reserved, do not read. atari800 and Altirra: GPL. Samples: `dos25.atr`
(unarc-rs, zeroed boot sector). More: savetz repos (MIT, may contain Atari DOS; check the boot
sectors first).

#### Atari ST

| Format | Docs | Level | Notes | Effort |
|---|---|---|---|---|
| .ST raw + FAT12/GEMDOS | Microsoft fatgen103 | sector | `fatfs` 0.3.6 [crates.io: MIT, 2023-01; GH: MIT, pushed 2026-10-16] | 1 |
| MSA | IANA vnd.msa-disk-image; PRONOM fmt/1472 | compressed sector | magic 0x0E0F, track RLE; header offsets [unverified] | 1 |
| STX (Pasti) | community: https://atari.8bitchip.info/STXdesc.html (IANA says no official spec) | sector + track images, fuzzy bits | sector data usually present, so FAT12 often readable | 3 |

Clock Signal MSA.cpp and STX.cpp (MIT) can be read. Hatari is GPL-2: do not read. fluxfox
covers .ST via rust-fatfs but has no MSA or STX. Sample `atari.st` (unarc-rs).

#### Apple II

| Format | Docs | Level | Notes | Effort |
|---|---|---|---|---|
| DSK / DO / PO (140K), 2MG/2IMG | CiderPress II formatdoc (TwoIMG-notes, DOS-notes, ProDOS-notes) | sector | .dsk order is ambiguous: probe the DOS VTOC (track 17) and ProDOS key block 2 | 3 |
| DOS 3.2/3.3, ProDOS, Pascal | Beneath Apple DOS / ProDOS (archive.org), ProDOS 8 Technical Reference (https://prodos8.com/docs/techref/) | filesystem | ProDOS has creation/modification dates, file type and aux type | 5-6 |
| Applesoft, Integer BASIC | CiderPress II BASIC-notes | data | | 1-2 |
| NIB | Nibble-notes; MAME ap2_dsk.cpp (BSD-3-Clause [hdr]) | nibble | decode 6-and-2 GCR for standard disks; protected tracks listed | 3 |
| WOZ 1/2/2.1 | https://applesaucefdc.com/woz/reference2/ | bitstream (+FLUX chunk in 2.1) | INFO and META chunks are knowledge (title, publisher, developer, copyright, language); FLUX tracks listed only | 3-5 |

- **CiderPress II** (fadden/CiderPress2) [GH: Apache-2.0, pushed 2026-10-19, v1.2.0
  2026-03-21]: C#, excellent format docs at https://ciderpress2.com/formatdoc/ and Apache-2.0
  code. Best primary reference for Apple II and classic Mac; port with attribution and NOTICE.
  Caveat: its StuffIt notes name unar (LGPL) as their reference; take facts only from those.
- **a2kit 4.4.2** [crates.io: MIT, 2026-05-03, 58,903 downloads; GH: MIT]: DOS 3.2/3.3,
  ProDOS, Pascal, CP/M, FAT; 2MG, D13, DO, DSK, IMD, IMG, NIB, PO, TD0, WOZ; Applesoft and
  Integer BASIC. Good code, but no `[features]` section: every user pulls tree-sitter with C
  grammars (needs `cc`), lsp-server, clap and `atty` (RustSec advisory, fails cargo-deny)
  [src: Cargo.toml has no [features] section and plain dependencies on tree-sitter 0.25.8 plus three tree-sitter grammars, lsp-server, clap 4 and atty 0.2]. Use as an MIT reference or ask upstream
  for a lib-only feature.
- Samples: `simple-dir-test.po`, `prodos-disk.2mg` (CiderPress II TestData, CC-BY-4.0). Avoid
  CiderPress II `woz/*`, DOS master images and anything with Apple system software.

#### Classic Mac

| Format | Docs | Level | Crate | Effort |
|---|---|---|---|---|
| MacBinary I/II/III | https://github.com/mietek/theunarchiver/wiki/MacBinarySpecs (wiki facts; XADMaster code is LGPL-2.1: do not read) | wrapper | `macbinary-rs` (cyco, Apache-2.0), `macbinary` (wezm, Apache-2.0) | 1 |
| BinHex 4.0 | RFC 1741 | wrapper | `binhex-rs` (cyco, Apache-2.0), `binhex4` 0.1.2 (MIT). `binhex` crate is GPL-3.0: excluded | 0.5 |
| AppleSingle / AppleDouble | RFC 1740; v2 developer note | wrapper | own (small) | 0.5 |
| Resource fork | Inside Macintosh: More Macintosh Toolbox | data | `resource-fork-rs` (cyco, Apache-2.0); the `resource-fork` crate is a macOS xattr wrapper: useless | 1 |
| HFS | Inside Macintosh: Files (File Manager PDF) | filesystem | `fstool` 0.5.0 (KarpelesLab, MIT, 2026-10-17): HFS with forks and DC42 unwrap, HFS+, AFFS, ISO 9660, FAT, APM/MBR/GPT; very new, dependency footprint unchecked | 3 with crate, 10-12 own |
| MFS | Inside Macintosh Vol II; Apple MFSLives sample | filesystem | `macfs` 0.2.0 (MIT OR Apache-2.0, 37 downloads) | 1-2 |
| DiskCopy 4.2 | https://www.discferret.com/wiki/Apple_DiskCopy_4.2 | wrapper | in fstool, macfs | 0.5 |
| NDIF (Disk Copy 6) | no Apple spec; chunk map in a 'bcem' resource (ADC, KenCode) | compressed | `unretro` 0.2.0 (MIT OR Apache-2.0) as reference [KenCode provenance unverified] | 3-4 |
| StuffIt 1.x and SIT5, Compact Pro | mietek theunarchiver wiki (facts only) | archive | `sit-rs` 0.3.0 (cyco, Apache-2.0; no LGPL references found by grep, provenance [unverified]), `compact-pro` (cyco, Apache-2.0) | 2-3 with crate, 12-15 own (Arsenic is the hard codec) |
| HFS+ | TN1150 | filesystem | `hfsplus` 0.4.0 (MIT) | 1-2 |

- **Tainted or excluded:** `stuffit` (benletchford; MIT OR Apache-2.0 on crates.io, but its
  source comments cite `XADStuffIt5Parser.m` and the README names The Unarchiver as
  "reference implementation": likely derivative of LGPL code, do not use), `newtua-*`
  (LGPL-3.0+), hfsutils/libhfs (GPL-2.0), XADMaster/unar (LGPL-2.1).
- Readable permissive references: machfs and macresources (Python, MIT).
- Encoding: HFS names are Mac Roman (or another Mac script per volume). `encoding_rs` has
  `MACINTOSH` (WHATWG "macintosh", 0xDB = euro). `mac-encoding` 0.1.1 (MIT OR Apache-2.0) has
  other Mac scripts.
- Samples: `simple-linux.po` (HFS, CC-BY-4.0), `hello__.as` (AppleSingle, CC-BY-4.0). Apple
  system disks (incl. the once free System 7.5.3) are not redistributable.

#### Amiga

| Format | Docs | Crate | Effort |
|---|---|---|---|
| ADF OFS/FFS, INTL, DIRCACHE, DOS6/7 long names | Laurent Clevy "The ADF FAQ" (http://lclevy.free.fr/adflib/adf_info.html, freely distributable) | `amiga-ffs` 0.4.0 (MIT OR Apache-2.0, 2026-09-16; DOS0-DOS7, tested against xdftool as an oracle, not read) | 2-3 |
| HDF / RDB | ADF FAQ s6; http://wiki.amigaos.net/wiki/RDB | `amiga-rdb` 0.4.1 (MIT OR Apache-2.0) | 1 |
| DMS | no spec; xDMS is public domain (https://zakalwe.fi/~shd/foss/xdms/xdms.txt) | `unarc-rs` (all 7 modes) | 1 |
| LhA (.lha, .lzh) | widely documented header levels 0-2 | **`delharc` 0.8.0** [crates.io: MIT OR Apache-2.0, 2026-07-22, 158k downloads; GH: Apache-2.0] | 0.5 |
| ADZ | gzip-wrapped ADF | `flate2` | 0.5 |
| IPF | SPS docs now 404 [unverified copy]; MAME ipf_dsk.cpp (BSD-3-Clause [hdr]) | `ipf-flux` 0.1.0 (BSD-3-Clause, clean-room from MAME) | 3 (AmigaDOS tracks only) |

- **IPF and the CAPS/SPS library:** the CAPS license forbids commercial use and enabling other
  formats (Gentoo copy of the Kryoflux-MAME license v1.02). Not OSI-compatible: never link,
  bundle or require it. Most IPFs are copy-protected games; decode AmigaDOS tracks via ipf-flux,
  list the rest.
- Excluded: `format198x-commodore-amiga-adf` (GPL-2.0+), ADFlib (LGPL-2.1), amitools/xdftool
  (GPL-2), `newtua-amiga` (LGPL-3.0+), `amiga-lzx` (WTFPL: not on the allow list, ask Jordi).
  `adflib` crate (vschwaberow, MIT/Apache-2.0) says it is "inspired by" ADFlib concepts:
  provenance [unverified]; prefer amiga-ffs.
- Amiga file names are Latin-1. `encoding_rs` maps the label iso-8859-1 to windows-1252, so use
  a direct byte-to-char map for true Latin-1.
- Samples: none collected. Fred Fish disks are "freely distributable" per program, not under a
  clear license; ADFlib dumps sit in an LGPL repo with no data license. Plan synthetic ADF/HDF
  fixtures made with amiga-ffs and amiga-rdb.

#### MS-DOS and PC floppies

| Format | Docs | Level | Crate / reference | Effort |
|---|---|---|---|---|
| IMG / IMA (FAT12/16/32) | Microsoft fatgen103 | sector | `fatfs` 0.3.6 (MIT); `hadris-fat` 2.5.0 (MIT, 2026-10-15) | 1-2 |
| IMD (ImageDisk) | Dave Dunfield, http://dunfield.classiccmp.org/img/index.htm (spec in the imd zip) | sector + comment header | own; fluxfox (MIT) reference | 1-2 |
| TD0 (Teledisk, normal and advanced) | "closed/undocumented" (Dunfield); MAME td0_dsk.cpp (BSD-3-Clause [hdr]); archived wteledsk notes | sector, LZHUF or LZW | `retrocompressor` 1.0.1 (MIT, dfgordon) for the codecs | 2-3 |
| CQM (CopyQM) | MAME cqm_dsk.cpp (BSD-3-Clause [hdr]); Sydex manual on bitsavers | sector | port from MAME file | 1-2 |
| ISO 9660 + Joliet + Rock Ridge | ECMA-119 | filesystem | `hadris-iso` 2.5.0 (MIT) | 1-2 |

- **fluxfox** [crates.io: MIT, but only 0.1.0 from 2024-11-07; GH: MIT, dbalsom/fluxfox, 164
  stars, pushed 2026-09-03]: IMG, TD0, IMD, PSI, PRI, MFM, HFE, 86F, IPF, SCP, KryoFlux, PFI,
  MFI, TC, MOOF. Strong for PC formats, but the exact-pin rule means a git dependency or waiting
  for a new crates.io release. It loads GCR tracks but does not decode them.
- Note only: LibDsk (license reported as GPL or LGPL by different packagers [unverified]),
  Dunfield's TD02IMD (non-free terms).
- Samples: `sector_test_360k.img/.imd/.td0` (fluxfox, synthetic sector patterns, the .img is not
  a FAT volume). A clean FAT12 floppy: archive.org "floppy-144-288-393" (CC0, blank images; boot
  sector may be Microsoft's [unverified]); FreeDOS 1.4 Floppy Edition by URL and hash only (mixed
  package licenses).

#### ZX Spectrum

| Format | Docs | Level | Notes | Effort |
|---|---|---|---|---|
| TAP | https://sinclair.wiki.zxnet.co.uk/wiki/TAP_format | byte | header blocks: type, 10-char name, length, autostart line | 0.5 |
| TZX (and CDT) | https://worldofspectrum.net/TZXformat.html | mostly byte | 0x10/0x11/0x14 carry bytes; 0x15/0x18/0x19 are signal level (list); 0x30 text, 0x32 archive info (title, publisher, author, year, language) are knowledge | 3 |
| CSW | Ramsoft spec v2.00 | pulse | ROM loader only | 3 |
| TRD (TR-DOS), SCL | Sinclair wiki | sector / byte | | 2 |
| +3 DSK + +3DOS | CPCEMU DSK spec (below) | sector | | 2 |
| MDR (Microdrive) | mdr.txt (zxsp mirror) | sector | 254 sectors of 543 bytes | 1.5 |
| Sinclair BASIC | ZX Spectrum manual token table | data | skip the 5-byte hidden number after 0x0E; `zxnumber` 0.1.0 (MIT OR Apache-2.0) decodes it | 1.5 |

Crates: `rtzx` 0.5.0 (MIT) is CLI-oriented with audio dependencies (read only). `spectrusty`
(LGPL-3) and `format198x-tzx` etc. (GPL-2.0+): excluded. tzxtools (GPL-3), fuse/libspectrum
(GPL-2 [unverified]), skoolkit (GPL-3): do not read. Clock Signal ZXSpectrumTAP and TZX (MIT):
readable. Sample: `10print.tap` (MIT homebrew; first block is a BASIC header "Loader").

#### Amstrad CPC and CP/M

| Format | Docs | Notes | Effort |
|---|---|---|---|
| CPCEMU DSK / EXTENDED DSK | https://simonowen.com/mirror/extdsk.html (cpcwiki returns 403 to scripts) | EDSK carries FDC status and weak sectors | 2 |
| AMSDOS | https://www.seasip.info/Cpm/amsform.html | file headers with type, load and exec address | 1.5 |
| CP/M 2.2/3 on raw images | https://www.seasip.info/Cpm/format22.html | needs a disk parameter block (DPB) per machine; do not copy cpmtools `diskdefs` (GPL-3), rebuild DPBs from manuals; `a2kit` has DPBs for Apple II, Osborne 1, Kaypro, Amstrad PCW (MIT) | 3 + 1 per DPB family |
| Locomotive BASIC | | `cpclib-basic` 0.10.0 (MIT) | 1 |

`cpclib-disc` 0.10.0 (MIT on crates.io; GitHub shows no license detected) reads DSK/EDSK and
AMSDOS; keep its `hxcfe` feature off (GPL-3.0). Sample: `maze.dsk` (MIT homebrew, EXTENDED CPC
DSK).

#### Other tape formats

| Format | Docs | Notes | Effort |
|---|---|---|---|
| UEF (BBC Micro, Electron) | http://electrem.emuunlim.com/UEFSpecs.html; https://beebwiki.mdfs.net/Acorn_cassette_format | gzip-wrapped chunks; 0x0100 bytes, 0x0102 bits, 0x0110-0x0116 tones | 2 |
| CAS (MSX) | no spec; Clock Signal MSXCAS.cpp (MIT) | sync `1F A6 DE BA CC 13 7D 74` at 8-aligned offsets, then type bytes D0/D3/EA and a 6-byte name | 1.5 |
| CAS (TRS-80, CoCo) | MAME trs_cas.cpp; trs-80.com | bit stream at 250/500/1500 baud [byte layout unverified] | 2 |

`.cas` is ambiguous: "FUJI" means Atari, the MSX sync means MSX, otherwise TRS-80 or CoCo by
leader and sync.

### B.3 Bit-level, flux and copy protection

| Class | Formats | What Rezzer does |
|---|---|---|
| Byte level | T64, PRG, P00, CRT, LNX, ZX TAP, SCL, UEF 0x0100, MSX CAS, Atari CAS standard records, MDR, all wrappers | full |
| Sector level | D64/D71/D81, ATR, XFD, DCM, ST, MSA, ADF, HDF, DSK/DO/PO/2MG, IMG, IMD, TD0, CQM, DC42, EDSK, TRD | full (TD0, IMD, CQM and DMS are not flux formats) |
| Sector + timing / weak bits | ATX, STX, EDSK weak sectors, D64 error bytes | read the filesystem where sectors are unique; report protection marks as knowledge |
| GCR or MFM bitstream | G64, NIB, WOZ 1/2, IPF | decode standard tracks; list protected tracks with the reason |
| Pulse level | C64 TAP, CSW, TZX 0x15/0x18/0x19 | decode the standard ROM loader only; "custom loader, not decoded" |
| Flux | P64, WOZ 2.1 FLUX, SCP, KryoFlux, HFE, 86F, MFI, PFI, A2R | list only in waves 1-2; P64 later via the Zlib reference |

---

## C. Recommendation

### C.1 Scope and order

Rank by two factors: (1) does the building block also serve modern formats, and (2) is there a
public spec or a permissive reference. Knowledge per day comes next.

**Wave 0: shared building blocks (about 10-12 pd).** Needed by everything below and by ZIP, TAR,
ISO, EML attachments and OLE embedded objects in the core.

1. Container layer and model extension (6-8 pd, C.2).
2. Code page tables in `textenc` (1-2 pd, C.3).
3. Table-driven BASIC detokenizer (1-2 pd): one engine, token tables per dialect (C64 V2/V7,
   Applesoft, Integer, Atari, Sinclair, Locomotive via cpclib-basic).

**Wave 1: high knowledge, public specs, little license risk (about 75-97 pd).**

| Item | pd | Why |
|---|---|---|
| WordStar 3-7 | 5-8 | official spec, MIT samples, high-profile authors |
| man / mdoc + tbl | 10-15 | very common, ISC reference, semantic macros, cross-reference graph |
| dBase / FoxPro | 3-5 | MIT crate, very common |
| Lotus WKS/WK1/WR1 + formula engine | 6-8 | public-domain spec, formulas feed the existing formula facet |
| SYLK, DIF | 5-8 | small, formulas (SYLK) |
| RTF early variants | 2 | builds on the core reader |
| MacBinary, BinHex, AppleSingle/Double, resource forks | 2-3 | Apache crates; needed for every Mac file |
| D64/D71/D81, T64, PRG, P00, LNX + BASIC V2/V7 + PETSCII | 12-13 | excellent docs; directory/BAM is real knowledge |
| ATR/XFD + DOS 2.x/SpartaDOS + Atari BASIC | 7 | MPL-2.0 Python reference (atrcopy) |
| .ST / MSA, IMG/IMA FAT12, IMD, ISO 9660 | 5-7 | MIT crates, also serves modern ISO and FAT images |
| Apple II DSK/DO/PO/2MG + DOS 3.3/ProDOS/Pascal + Applesoft | 9-11 | Apache-2.0 CiderPress II docs and code |
| ADF/HDF + LhA + DMS | 4-5 | permissive crates |
| ZX TAP + TZX byte blocks + Sinclair BASIC | 5 | clear specs |

**Wave 2: big formats with a permissive or MPL reference (about 120-160 pd).**

- WordPerfect 5.x/6+ (25-40), the biggest single gain in archives. Needs Jordi's go for an
  MPL-2.0 port of libwpd (C.4).
- libwps family: MS Works .wps/.wks/.wdb, Word for DOS, XyWrite, MS Write (about 40), Lotus
  WK3/WK4/.123 (12-18, Apache OpenOffice as reference), Quattro Pro (15-22).
- Excel 2-4 BIFF (9-13), FrameMaker MIF (10-15).
- HFS, MFS, DiskCopy 4.2, NDIF, StuffIt, Compact Pro (8-12 with crates).
- GEOS + geoWrite (6), TD0 + CQM (3-5), CPC DSK + AMSDOS + CP/M (8-10), G64 and NIB/WOZ standard
  tracks (6-8).

**Wave 3: lower return (about 100+ pd).** Mac word processors via libmwaw (45-65), Paradox
(6-9), Ami Pro (5-8), DCA/RFT (10-15), Apple II AppleWorks (6-8), Atari CAS, ATX, STX, DCM, ZX
CSW/TRD/SCL/MDR, UEF, MSX and TRS-80 CAS, C64 TAP, P64, IPF AmigaDOS tracks.

**Detect and say "not readable" (with the reason):** Lotus Word Pro (license), WordStar 2000 and
WordStar for Windows, PFS:Write, Interleaf binary, Framework, binary FrameMaker .fm, flux images
without a decoder.

Totals (rounded): wave 0 + 1 is about 85-110 pd and already covers the formats people most
often find in old archives except WordPerfect. With WordPerfect it is about 110-150 pd.

Charset interim for wave 1: until the 8-bit table license is settled (C.4 item 3), PETSCII,
ATASCII and ZX text maps the ASCII-equivalent range directly and writes a placeholder plus a
loss entry ("graphic glyph, table pending") for the rest. The clean-room tables built from the
machine manuals plus UCD names belong in wave 0, so wave 1 does not wait on the legal question.

### C.2 Container and filesystem layer (shared with ZIP, TAR, ISO)

What exists today in the workspace (read, not changed):

- `rezzer-detect` sniffs ZIP and OLE containers (`container.rs`) with a decompression budget.
- `rezzer-limits` already has `max_depth`, `max_unpacked_bytes`, `max_ratio`, `max_time` and a
  `Budget` with `add_unpacked`. Nested images map onto these directly.
- `textenc` decodes text with BOM, strict UTF-8, chardetng and encoding_rs.
- The `Embedded` facet holds only `kind`, `name`, `detail` and `target`.

**Gap in the model.** `Embedded` has no size, dates, file type, type/creator codes, load
address, forks, flags or raw name, and the model has no concept of a child document. "List the
files and pass each to the readers" is the real design decision. Proposal for the core spec:

- A `Container` trait next to `Reader`: `fn entries(&self, src, budget) -> Vec<Entry>` and
  `fn open(&self, entry) -> Bytes` (lazy, budgeted). Implementations: ZIP, TAR, ISO, OLE
  storages, and every image and archive in part B.
- An `Entry` record: path, raw name bytes plus decoded name and encoding, kind (file, dir,
  link, fork, volume, partition), size (bytes and blocks), dates (created, modified, backup,
  accessed, each optional and typed), type info (CBM type, ProDOS type + aux, Mac type/creator,
  Amiga protection bits, CP/M user area), load address, flags (locked, closed, hidden,
  deleted), comment (Finder, Amiga, GEOS info), forks (data, resource), and a loss if it could
  not be read.
- A `Volume` record: name, ID, DOS type, label, format, geometry, free space, interleave,
  protection marks, comment header (IMD, TD0, WOZ META, TZX archive info).
- Child documents: the pipeline runs detection and the reader on each entry and attaches the
  result as a child `Document` (or a separate bundle file) with provenance
  `parent sha256 + entry path`. The renderer prints the container as a table (the directory
  listing) plus one section or bundle file per child.
- Recursion uses `max_depth`, `max_unpacked_bytes` and `max_ratio`; DMS, TD0, StuffIt, LhA,
  NDIF and gzip-wrapped images (ADZ, UEF) count against the unpacked budget.
- Sector access as a second trait (`BlockDevice`: read sector by track/side/sector or LBA),
  so filesystems (FAT, CBM DOS, Atari DOS, ProDOS, CP/M, HFS, OFS/FFS) are written once and run
  on raw, IMD, TD0, CQM, DC42, 2MG, ATR, MSA, EDSK and decoded GCR images alike.

### C.3 Charset tables

| Charset | Source in Rust | Status |
|---|---|---|
| Mac Roman | `encoding_rs::MACINTOSH` (already a dependency of `textenc`) | ready. Do not vendor unicode.org ROMAN.TXT: it says "all rights reserved" by Apple |
| CP437, CP850 (DOS, XyWrite, RTF `\pc`/`\pca`, FAT names, WordStar) | not in `encoding_rs` (WHATWG has no CP437/850 [src]); `oem_cp` 2.1.2 (MIT) has 437/737/850 and more; `codepage-437` 0.1.0 (MIT) has a graphic-glyph variant | add `oem_cp` behind `textenc` |
| IBM866 | `encoding_rs` | ready |
| True Latin-1 (Amiga) | encoding_rs maps the label to windows-1252; use a direct byte map | trivial |
| EBCDIC (DCA/RFT) | no crate checked [unverified] | own table for CP037/CP500 from IBM docs |
| PETSCII (both cases, video and CHR$), ATASCII, ZX Spectrum, Amstrad CPC, Atari ST, MSX, TRS-80 | Unicode 13+ "Symbols for Legacy Computing" (U+1FB00-1FBFF) and the Supplement (U+1CC00-1CEBF, Unicode 18.0); mapping zip of proposal L2/19-025 (https://www.unicode.org/L2/L2019/19025-aux-mappings.zip, Rebecca Bettencourt, 2018). All code points checked against UCD 18.0: 0 missing, names match. Reverse-video glyphs have no code point: render as a style | **license unclear** (C.4) |
| Atari ST | https://www.unicode.org/Public/MAPPINGS/VENDORS/MISC/ATARIST.TXT has a Unicode, Inc. grant ("freely use ... as long as this notice remains attached") | usable with the notice |
| Apple II high-bit ASCII, ProDOS ASCII | trivial | ready |

Put all of them in `textenc` as named encodings, with a flag for "graphic glyph" vs "control"
interpretation of 0x00-0x1F (CP437 text vs CP437 file names).

### C.4 Licensing blockers and decisions for Jordi

1. **MPL-2.0 port of libwpd, libwps, libmwaw (decision needed).** These are dual "MPL-2.0 /
   LGPL" [hdr]. Choosing the MPL-2.0 arm, a Rust port is a derivative work under MPL-2.0. That
   is compatible with Rezzer's MPL-2.0 license, but the ported files must keep MPL-2.0, carry
   the original copyright notices (or a NOTICE file mapped to them) and be marked as derived.
   AGENTS.md says reading a library's code and rewriting it is still a derivative work; for
   MPL this is allowed, but it is not clean-room. Alternative for formats with a public spec
   (WordStar, WK1, DIF, SYLK, MIF, RTF, DCA/RFT, man): use the spec and the AGENTS.md clean-room
   flow (one agent writes a facts-only format spec, a different one implements), as done for DWG
   in the core spec. Linking the C++ libraries in-process is out: MPL is weak copyleft, not
   permissive, and the rule is C/C++ only if permissive.
2. **Lotus Word Pro.** The only reader (LibreOffice `lotuswordpro/`) is LGPL-2.1 / SISSL-1.1.
   Do not read. Detect only.
3. **Charset tables for 8-bit machines.** The L2/19-025 mapping zip has no license grant; the
   newer kreativekorp/charsets repo has no license at all. Options: ask the author (Rebecca
   Bettencourt) for a permissive license, or build the tables as facts from the C64
   Programmer's Reference Guide (appendices) and UCD names in a clean-room step. Blocks embedding
   the tables, not the design.
4. **CAPS/SPS IPF library**: non-commercial license. Never link or require it; use ipf-flux
   (BSD-3).
5. **Tainted or excluded crates**: `stuffit` (benletchford, likely derived from LGPL
   XADMaster), `unfair-codec-*` (private repo, names Gnumeric as a source), `format198x-*` and
   `play198x-core` (GPL-2.0+), `newtua-*` (LGPL-3.0+), `binhex` (GPL-3.0), `spectrusty`
   (LGPL-3), `zinc64-*`, `rs1541`, `xum1541`, `remanence` (GPL-3.0), `ws2markdown` (CDDL-1.1).
   Add them to the cargo-deny ban list.
6. **WTFPL** (`amiga-lzx`, bw-atari8-tools): not on the allow list. Decide whether to allow it;
   until then use facts only.
7. **Zlib** (P64 reference code in the VICE tree): on the allow list, add to cargo-deny if code
   is reused.
8. **Young crates and pins.** fstool, macfs, sit-rs, amiga-ffs, hadris-*, unarc-rs, ipf-flux,
   cyco crates are all 2026 releases with low download counts; fluxfox's crates.io release is
   two years behind its repo. Put each behind Rezzer's trait, pin exactly, keep fixtures with
   expected output. Provenance of unretro (NDIF/KenCode), sit-rs, adflib and rars vs unRAR is
   [unverified].
9. **Test data.** LibreOffice and Tika test files have no per-file license (repository license
   assumed). CiderPress II TestData is CC-BY-4.0 (attribution in the manifest). No clean
   samples exist for GEOS, G64, P64, STX, MSA, ATX, DMS, HDF, StuffIt, NDIF, DCA/RFT, XyWrite,
   WordStar 2000, Paradox or Framework: plan synthetic fixtures made with permissive tools by
   scripts kept outside the corpus folder, published under MPL-2.0 or CC0. Disk images that
   hold Apple, Commodore, Atari, Microsoft or Berkeley Softworks system software are not
   redistributable.
10. **Never run code.** CRT, PRG machine code, XEX, boot sectors and turbo loaders are listed with
    load address and size only. No emulation, no disassembly. BASIC is detokenized to text, never
    interpreted.

### C.5 Open points

- Does anydoc expose RTF annotations, revisions and `\info`? Check in the core spike.
- WP5 document summary fields, Word for DOS summary sheet fields, WordStar 7 header fields: verify
  against the samples once a reader exists.
- a2kit dependency footprint and fstool footprint: run `cargo tree` in a scratch crate before
  choosing.
- encoding_rs Latin-1 helper (`mem::decode_latin1`) existence in 0.8.42 [unverified].
- MSA header offsets, ATX spec location, TRS-80 CAS byte layout, BASIC load addresses
  [unverified].
