# Formats

One page per format: what Rezzer gets out of it, which crates it builds on, how it shows up in the output and what it can't do yet.

## Status

| Label | Meaning |
|---|---|
| ✅ Available | Works in the development version and is part of 0.1.0, the first release. The page shows real output from the test files and how the format is tested |
| 🟡 Partial | Works, but Rezzer reads only part of what the format holds. The page says what is missing |
| 🗓️ 0.2.0 | Planned for release 0.2.0. Nothing is implemented yet. The page describes what is planned |
| 🔭 Later | Planned for a release after 0.2.0. Nothing is implemented yet |

Release 0.2.0 brings HTML first, then the office formats, then EPUB.

## All formats

| Page | Formats | Status |
|---|---|---|
| [PDF](pdf.md) | PDF | ✅ Available |
| [XLSX](xlsx.md) | XLSX, XLSM | ✅ Available |
| [XLSX](xlsx.md) | XLSB | 🟡 Partial: values, formulas and names, no styles, tables or pivots |
| [ODS](ods.md) | ODS | ✅ Available |
| [CSV](csv.md) | CSV, TSV | ✅ Available |
| [Plain text](text.md) | Plain text in any encoding | ✅ Available |
| [Markdown](markdown.md) | Markdown | ✅ Available |
| [HTML](html.md) | HTML | 🗓️ 0.2.0 |
| [DOCX](docx.md) | DOCX | 🗓️ 0.2.0 |
| [PPTX](pptx.md) | PPTX | 🗓️ 0.2.0 |
| [ODT](odt.md) | ODT | 🗓️ 0.2.0 |
| [ODP](odp.md) | ODP | 🗓️ 0.2.0 |
| [DOC](doc.md) | DOC (Word 97-2003) | 🗓️ 0.2.0 |
| [XLS](xls.md) | XLS (Excel 97-2003) | 🗓️ 0.2.0 |
| [PPT](ppt.md) | PPT (PowerPoint 97-2003) | 🗓️ 0.2.0 |
| [RTF](rtf.md) | RTF | 🗓️ 0.2.0 |
| [EPUB](epub.md) | EPUB 2 and 3 | 🗓️ 0.2.0 |
| [CAD](cad.md) | FreeCAD, STEP, IGES, IFC, DXF, 3MF, STL, OBJ, glTF, G-code, KiCad, Gerber, OpenSCAD; DWG and closed native formats after that | 🔭 Later |
| [PostScript](postscript.md) | PostScript, EPS | 🔭 Later |
| [Mail](mail.md) | EML, MBOX | 🔭 Later |
| [Mathcad](mathcad.md) | Mathcad Prime, Mathcad 12-15, SMath | 🔭 Later |

`rezzer formats` lists what your build can read.

The public test files of the available formats and their licenses are listed in [`../../tests/corpus/MANIFEST.md`](../../tests/corpus/MANIFEST.md). The test files of the first format survey are in the manifests in [`../spike-2026-10-19/`](../spike-2026-10-19/).
