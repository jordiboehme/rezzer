# Formats

One page per format: what Rezzer gets out of it, which crates it builds on, how it shows up in the output and what it can't do yet.

## Status

| Label | Meaning |
|---|---|
| ✅ Available | Works in the development version and is part of 0.1.0, the first release. The page shows real output from the test files and how the format is tested |
| 🟡 Partial | Works, but Rezzer reads only part of what the format holds. The page says what is missing |
| 🗓️ 0.2.0 | Planned for release 0.2.0. Nothing is implemented yet. The page describes what is planned |
| 📝 Planned | Planned, but the release is not set yet. Nothing is implemented yet. The page describes what is planned |
| 🔭 Later | Planned for a release after 0.2.0. Nothing is implemented yet |

Release 0.2.0 brings PowerPoint (PPTX) first, then EPUB, then HTML. With it, Rezzer also reads dates and titles in more languages.

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
| [DOCX](docx.md) | DOCX | 📝 Planned |
| [PPTX](pptx.md) | PPTX | 🗓️ 0.2.0 |
| [ODT](odt.md) | ODT | 📝 Planned |
| [ODP](odp.md) | ODP | 📝 Planned |
| [DOC](doc.md) | DOC (Word 97-2003) | 📝 Planned |
| [XLS](xls.md) | XLS (Excel 97-2003) | 📝 Planned |
| [PPT](ppt.md) | PPT (PowerPoint 97-2003) | 📝 Planned |
| [RTF](rtf.md) | RTF | 📝 Planned |
| [EPUB](epub.md) | EPUB 2 and 3 | 🗓️ 0.2.0 |
| [ADF](adf.md) | Atlassian Document Format (Jira and Confluence) | 📝 Planned |
| [CAD](cad.md) | FreeCAD, STEP, IGES, IFC, DXF, 3MF, STL, OBJ, glTF, G-code, KiCad, Gerber, OpenSCAD; DWG and closed native formats after that | 🔭 Later |
| [PostScript](postscript.md) | PostScript, EPS | 🔭 Later |
| [Mail](mail.md) | EML, MBOX, MSG, Outlook PST and OST | 🔭 Later |
| [Mathcad](mathcad.md) | Mathcad Prime, Mathcad 12-15, SMath | 🔭 Later |
| Lotus Notes | NSF mail databases | 🔭 Later |
| Photos and scans | JPEG, PNG, TIFF, HEIC | 🔭 Later |
| Archives | ZIP, 7z, TAR | 🔭 Later |
| Diagrams | draw.io, Visio, Lucidchart, Excalidraw | 🔭 Later |
| Design | Figma | 🔭 Later |
| Automation | PLCopen XML, Rockwell L5X, TIA Portal and TwinCAT exports | 🔭 Later |
| Plant and product data | P&ID (DEXPI), Asset Administration Shell, AutomationML, ReqIF, QIF | 🔭 Later |
| Lab data | ELN exports, ChemDraw, Allotrope, instrument files | 🔭 Later |
| Data science | Parquet, Arrow and Feather, Avro, JSON Lines, compressed CSV, HDF5, NetCDF, NumPy, Zarr | 🔭 Later |
| Research writing | Jupyter notebooks, LaTeX, BibTeX, RIS | 🔭 Later |
| Signals and parameters | DBC, A2L, COMTRADE, NI TDMS, FMI | 🔭 Later |
| Retro | WordStar, WordPerfect, Lotus 1-2-3, dBase, disk and tape images from the C64, Atari and classic Mac | 🔭 Later |

The groups without a page get one when their plan starts.

`rezzer formats` lists what your build can read.

The public test files of the available formats and their licenses are listed in [`../../tests/corpus/MANIFEST.md`](../../tests/corpus/MANIFEST.md). The test files of the first format survey are in the manifests in [`../spike-2026-10-19/`](../spike-2026-10-19/).
