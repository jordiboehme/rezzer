# Retro corpus manifest (research 2026-10-19)

Small public test files for the retro formats research
(`docs/research/2026-10-19-retro-formats.md`). One folder per family: `wordproc/`, `sheet-db/`,
`apple/`, `mac/`, `pc/`, `c64/`, `atari/`, `zx/`, `cpc/`. Paths in the tables are relative to
this folder.

All files are untrusted data. They were downloaded with curl and inspected only with `xxd` and
`shasum`. Nothing was executed, no emulator was run, and no script lives in this folder.

License notes:

- Files from the LibreOffice core repository are in an MPL-2.0 repository (per-file headers of
  the code are MPL-2.0). The test files carry no per-file license and some come from bug
  reports; that the repository license covers them is assumed, not verified.
- Files from Apache Tika, Apache POI and openoffice.org testdocs are in Apache-2.0
  repositories or sites; per-file provenance is not stated.
- CiderPress II TestData is CC-BY-4.0 "unless otherwise noted" (TestData/README.md). Only files
  without Apple system software were taken. Attribution: Andy McFadden, CiderPress II.
- fluxfox sector_test images are synthetic sector patterns in an MIT repository; that the MIT
  license covers the images is assumed, not verified. The `.img` is not a FAT volume (its boot
  sector is zero), it is a sector-pattern test.
- unarc-rs fixtures were made by the unarc-rs author (Apache-2.0 repository, crate MIT OR
  Apache-2.0). The ATR images have zeroed boot sectors (no Atari DOS code).
- Homebrew images (calc64, ZXSpeculator, mazecpc) are the authors' own programs in MIT
  repositories.

Date of download for all files: 2026-10-19.

## wordproc/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| LYING.WS | WordStar 7 (has a footnote) | https://raw.githubusercontent.com/jonmichaels/ctrl-kd/main/samples/LYING.WS | MIT (public-domain text, per samples/README.md) | 14464 | 022fe9d2bfba83038726b8287f2aa2583283b02352623fd588b03e1bd720a0e0 |
| OCAPTAIN.WS | WordStar 4 | https://raw.githubusercontent.com/jonmichaels/ctrl-kd/main/samples/OCAPTAIN.WS | MIT (public-domain text) | 1536 | 4f45d0f57e42f917f217087159874a024d25b98d2dc033a84e70468328d853bf |
| WP5.wp | WordPerfect 5.x | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/writer/libwpd/pass/WP5.wp | MPL-2.0 (repository) | 885 | 0012f0f2088d31238f81353b01fbc8f1cb7600d623172d5ab3895a59a2bbe8b6 |
| WP6.wpd | WordPerfect 6+ | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/writer/libwpd/pass/WP6.wpd | MPL-2.0 (repository) | 3587 | d5fde49f38d0dd73c65c0d19cb132defcc037dc09c8c405a2dfbaba457d65150 |
| testWordPerfect_5_1.wp | WordPerfect 5.1 | https://raw.githubusercontent.com/apache/tika/main/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-miscoffice-module/src/test/resources/test-documents/testWordPerfect_5_1.wp | Apache-2.0 (repository) | 18267 | c5b3c18d2c1ba2c99c2fbb1aababcbee15b744f2539605927e589c3c7eeb6bdc |
| Word_5.0_DOS.doc | Word for DOS 5.0 (magic 31 BE) | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/writer/libwps/pass/Word_5.0_DOS.doc | MPL-2.0 (repository) | 7168 | 1e7958c271080b34d0064c9e900dd7689e72c631b39868269a2441442cc2e62b |
| Works_3.0.wps | MS Works 3.0 WP (OLE container) | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/writer/libwps/pass/Works_3.0.wps | MPL-2.0 (repository) | 3584 | 1947092acdf91100d664af66cbc20d7247c2eaa334a03f7dccdaaf4498833055 |
| MacWrite_4.5 | MacWrite 4.5 (data fork only) | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/writer/libmwaw/pass/MacWrite_4.5 | MPL-2.0 (repository) | 5174 | 4ab74befea106f743baa766daf9a938d0e96815b52d0bc24c25374758a43de70 |
| testMIF.mif | FrameMaker MIF 5.5 | https://raw.githubusercontent.com/apache/tika/main/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-miscoffice-module/src/test/resources/test-documents/testMIF.mif | Apache-2.0 (repository) | 12477 | 36ebb0e062d3e1799095fe9b2f239a3ab8f2bd57c7f158485b16931335396843 |
| testComment.rtf | RTF with annotations | https://raw.githubusercontent.com/apache/tika/main/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testComment.rtf | Apache-2.0 (repository) | 33481 | 742ec05361e5c9bfc128ae3f1885acd558ef1a48a9b3a92bb843d7d54cfc3844 |
| mdoc.7 | mdoc(7) man page source | https://cvsweb.bsd.lv/~checkout~/mandoc/mdoc.7 | ISC (header in file) | 79808 | e9267579976a7101da8147f1c830c115cc2ef84729d705bcbb3134671169a83a |
| man.7 | man(7) man page source | https://cvsweb.bsd.lv/~checkout~/mandoc/man.7 | ISC (header in file) | 18911 | e30e12a8455caa77b95f2043958603cd692d72d2f1be34d0604b6771ace26648 |

## sheet-db/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| Lotus.wk1 | Lotus 1-2-3 WK1 | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/calc/libwps/pass/Lotus.wk1 | MPL-2.0 (repository) | 1503 | 33186f0ee623faaeaf770ca7674f6e1fea50239ca5a07549d4ad6b4b54d3bb19 |
| testLotus123.wk3 | Lotus 1-2-3 WK3 | https://raw.githubusercontent.com/apache/tika/main/tika-parsers/tika-parsers-standard/tika-parsers-standard-integration-tests/src/test/resources/test-documents/testLotus123.wk3 | Apache-2.0 (repository) | 18635 | 9741c1393cf98ff660d7faf50f12143608c5a5d662280163bdd1d82c03b7c6a0 |
| Lotus123_98.123 | Lotus 1-2-3 97/98 | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/calc/libwps/pass/Lotus123_98.123 | MPL-2.0 (repository) | 7616 | bc30c228b4b8627d4ef19cd7cadb216cebd126a1914bda53c36599f57a552ab7 |
| QuattroPro.wq1 | Quattro Pro DOS WQ1 | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/calc/libwps/pass/QuattroPro.wq1 | MPL-2.0 (repository) | 3540 | 1f70cc6ea80e5288f57c90d236e27a761cc886784f38e1ade30444862f07ee33 |
| Works_2.0.wks | MS Works 2.0 spreadsheet | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/calc/libwps/pass/Works_2.0.wks | MPL-2.0 (repository) | 1281 | 708c608b6fa5dcd0f1d11486045119cb3921d2ea0f98e2b0cbe2c17a2d9ff018 |
| Multiplan_3 | Multiplan 3 (DOS binary) | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/calc/libwps/pass/Multiplan_3 | MPL-2.0 (repository) | 2612 | 3cdd2543fb558be16485bdaef885109037003f29dbe98477ef2069c239832e12 |
| ClarisWorks_DB_5.0.cwk | ClarisWorks 5 database | https://raw.githubusercontent.com/LibreOffice/core/master/writerperfect/qa/unit/data/calc/libmwaw/pass/ClarisWorks_DB_5.0.cwk | MPL-2.0 (repository) | 10691 | dd5a44486da9ab420e79324bcb7c14baa1017263ed127b63298f69d77e0c6f02 |
| ooo80484-1.slk | SYLK | https://raw.githubusercontent.com/LibreOffice/core/master/sc/qa/unit/data/slk/pass/ooo80484-1.slk | MPL-2.0 (repository) | 2581 | bcfaae8f2ce55e393c100dcd95c37301129b9121af0429586de75b3ec99df420 |
| universal-content.dif | DIF (spreadsheet) | https://raw.githubusercontent.com/LibreOffice/core/master/sc/qa/unit/data/dif/universal-content.dif | MPL-2.0 (repository) | 339 | 82d79fc2a03f8e106356282ee062bb0d41bc6cb61a9e7b3b36c0eafb01d40c82 |
| formula_import_biff2.xls | Excel 2 BIFF2 (raw BOF 0x0009, no OLE) | https://www.openoffice.org/sc/testdocs/formula_import_biff2.xls | Apache-2.0 (openoffice.org site) | 92165 | 942fd07037255ece6d3db5773f1b0dcfe3b3a6e90ae82b0b0b6e71b843b0e4e0 |
| memotest.dbf | FoxPro DBF with memo | https://raw.githubusercontent.com/olemb/dbfread/master/tests/cases/memotest.dbf | MIT (repository) | 480 | e6fc0c840d59e58fd6086b6f32b1e6954401074e818d3b0ff45f53a16b76c650 |
| memotest.FPT | FoxPro memo file | https://raw.githubusercontent.com/olemb/dbfread/master/tests/cases/memotest.FPT | MIT (repository) | 2560 | 0d58778f08d2257860d136cba4dcd8e6df3d30fe8cc2e151bdc1b48f4e896e7e |

## apple/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| simple-dir-test.po | ProDOS 140K, ProDOS order | https://raw.githubusercontent.com/fadden/CiderPress2/master/TestData/prodos/simple-dir-test.po | CC-BY-4.0 (CiderPress II TestData) | 143360 | 356d5605692c845d0ca0d5c8364bd6fe3441a353c7c4a483ace3764c31dfc5ff |
| prodos-disk.2mg | 2IMG wrapper around ProDOS | https://raw.githubusercontent.com/fadden/CiderPress2/master/TestData/2img/prodos-disk.2mg | CC-BY-4.0 (CiderPress II TestData) | 143515 | 9354281295494995bd7244a1e068debe23bdbac61a362f63b8af91b96d1bd002 |

## mac/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| simple-linux.po | HFS volume (800K, made on Linux) | https://raw.githubusercontent.com/fadden/CiderPress2/master/TestData/hfs/simple-linux.po | CC-BY-4.0 (CiderPress II TestData) | 819200 | 65cb1386bb7742a1a11b75ff6e26c86aa0af0529b2823665967498edfc9388b9 |
| hello__.as | AppleSingle | https://raw.githubusercontent.com/fadden/CiderPress2/master/TestData/as/hello__.as | CC-BY-4.0 (CiderPress II TestData) | 167 | 562bb4a8eebcc0902d3c08265303664cba680e63c76c99fc67797171aee64228 |

## pc/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| sector_test_360k.img | raw 360K sector image (not FAT) | https://raw.githubusercontent.com/dbalsom/fluxfox/main/tests/images/sector_test/sector_test_360k.img | MIT (repository, coverage of images assumed) | 368640 | 0e61e0e0a01d799f87566621a96882d1020b6e9445af0096949a03e31d457668 |
| sector_test_360k.imd | ImageDisk | https://raw.githubusercontent.com/dbalsom/fluxfox/main/tests/images/sector_test/sector_test_360k.imd | MIT (repository, coverage of images assumed) | 2622 | 67a27f44d5fd82b836f5913bb73f71c1aa4047f28f17152fb63eb977423d086b |
| sector_test_360k.td0 | Teledisk | https://raw.githubusercontent.com/dbalsom/fluxfox/main/tests/images/sector_test/sector_test_360k.td0 | MIT (repository, coverage of images assumed) | 4680 | 49b5547ca52f9d24a322716b5e4f3e403de8bbcfc68ccf3a4d5f196001a88d05 |

## c64/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| test.d64 | D64 with PRG, SEQ, USR, REL, DEL entries | https://raw.githubusercontent.com/mkrueger/unarc-rs/main/crates/unarc-rs/tests/d64/test.d64 | Apache-2.0 (repository) | 174848 | e2ee2a00964f00394cc547c4e6ceafbffc18de40bba8c2b09bbf3198fbe06bf4 |
| calc64.d64 | D64 homebrew (disk name CALC64, ID CX) | https://raw.githubusercontent.com/davervw/calc64/main/calc64.d64 | MIT (repository) | 174848 | 85cb9f490933062aead5d2b074ba1d059c774776d3a17a9599fe8a3d2e4be5b4 |
| test.t64 | T64 tape archive | https://raw.githubusercontent.com/mkrueger/unarc-rs/main/crates/unarc-rs/tests/t64/test.t64 | Apache-2.0 (repository) | 2528 | b8e944a9015ca9010a49a9fb1b8dd61a2b1a4995cee5a157a4c43096a902976c |
| basic.lnx | Lynx archive with BASIC | https://raw.githubusercontent.com/mkrueger/unarc-rs/main/crates/unarc-rs/tests/lynx/basic.lnx | Apache-2.0 (repository) | 3573 | 827ba11bd68efd90a74f3347ea6a570f297472e25c840d5ed9913724ef2fa28e |

## atari/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| dos25.atr | ATR, Atari DOS 2.5 layout (zeroed boot) | https://raw.githubusercontent.com/mkrueger/unarc-rs/main/crates/unarc-rs/tests/atr/dos25.atr | Apache-2.0 (repository) | 133136 | c246840b9d7c0e833aa38ea8b0d5cdb9f0c8c441a343856f61e3667f3cbb905b |
| atari.st | Atari ST raw FAT12 | https://raw.githubusercontent.com/mkrueger/unarc-rs/main/crates/unarc-rs/tests/fat/atari.st | Apache-2.0 (repository) | 368640 | cc4d8e993a89f169b32b68172f2569fd26a6f4fe814a78dd87d68c69133662be |

## zx/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| 10print.tap | ZX Spectrum TAP (BASIC loader) | https://raw.githubusercontent.com/deanthecoder/ZXSpeculator/main/Experiments/10print/10print.tap | MIT (repository) | 5758 | 029c87005e8672f36fb69dd118519de6c145ce3bb0fa52473d5fac9a27c8a954 |

## cpc/

| File | Format | Source URL | License | Bytes | SHA-256 |
|---|---|---|---|---|---|
| maze.dsk | Amstrad CPC Extended DSK | https://raw.githubusercontent.com/ssg/mazecpc/main/maze.dsk | MIT (repository) | 194816 | a3584fab7b7ed71846c6f56abf13e26f297097fb2f3c2bb5c8b243baeb5d6d26 |

## Not collected

No clean sample was found for: Amiga ADF/DMS/HDF (Fred Fish disks are "freely distributable"
per program, not a clear license; ADFlib dumps sit in an LGPL repo with no data license), GEOS,
G64, P64, STX, MSA, ATX, C64 TAP, TZX with turbo blocks, CSW, UEF, StuffIt, Compact Pro,
DiskCopy 4.2 without Apple software, NDIF, WordStar 2000, DCA/RFT, PFS:Write, Interleaf,
XyWrite, Framework, VisiCalc, Paradox (BSD-3 Java repo with unstated provenance, not taken).
Plan: synthetic fixtures made by our own generator scripts (kept outside this folder) with
permissive tools, published under MPL-2.0 or CC0.
