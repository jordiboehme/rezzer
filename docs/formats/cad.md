# CAD and engineering formats

For engineering files Rezzer gives the agent what matters to an engineer: units, layers, parts lists, assembly and feature trees, sketch constraints, parameters and what drives them, materials, nets and print settings. Trees and dependencies come out as Mermaid.

Status: planned for the first release, except DWG and the closed native formats (see below). Nothing is implemented yet.

File types: see the table. Rezzer detects the format by content, not by extension. This matters for CAD: `.prt` is used by both PTC Creo and Siemens NX. A Creo file starts with a `#UGC:` line. An NX file is an OLE compound file. A DWG file names its version in the first 6 bytes (for example `AC1032`).

Everything runs inside the Rezzer binary. No CAD program and no external tool is needed.

## Formats

Tested on 2026-10-19 with about 390 public test files. Most formats get Rezzer's own reader: in the tests the crates either missed the knowledge layer, refused real files or failed on hostile files.

| Format | What an agent gets | Built on | Release |
|---|---|---|---|
| FreeCAD `.FCStd` | Feature tree, expressions and spreadsheet aliases as "drives" links (also across documents), sketch constraints with values, placements, links, assembly joints, drawings, metadata | own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) | first |
| STEP (AP203, AP214, AP242) | Header, units, product structure and parts list with total quantities, placements, properties, PMI (tolerances, datums, dimensions), colors, layers, saved views | [stepq](https://crates.io/crates/stepq) and own extras | first |
| IGES | Start text, sender, system, units, author, dates, levels, names, notes, views | own reader | first |
| IFC 2X3, 4, 4X3, ifcZIP | Project, site, building, storeys and spaces as a tree, elements, property sets, quantities, materials, classifications, types, systems, zones, documents | own reader, schema tables from [ifc-lite-core](https://crates.io/crates/ifc-lite-core) | first |
| DXF | Header and units, layers, blocks, attribute values (title block fields, part numbers), text, dimensions, leaders, tables, xrefs, images, groups | own reader | first |
| 3MF (also Bambu Studio, OrcaSlicer, PrusaSlicer, Cura) | Objects and parts, units, plates, materials and colors, all slicer settings, print time and filament | own reader on zip, quick-xml and [serde_json](https://crates.io/crates/serde_json) | first |
| STL | Header text, solids, counts, bounding box, area, volume, watertight check, colors. STL stores no units, and the output says so | own reader | first |
| OBJ and MTL | Objects and groups, comments, materials, texture references (listed, never followed) | own reader | first |
| glTF and GLB | Generator, node tree with names, user data, extensions, variants, external references | own reader on serde_json | first |
| G-code, Prusa `.bgcode` | Slicer and settings, print time, filament, temperatures, tools, thumbnails, CNC tools, feeds and offsets | own reader | first |
| KiCad 5 to 10 | Title block, sheet tree, parts with fields (parts list), not-fitted parts, notes, netlist, board stackup, footprints, design rules, holes | own reader | first |
| Gerber X2 and X3, Excellon, job file | Layer function, generating software, attributes, nets from copper layers, drill tools, plating, hit counts | own reader | first |
| OpenSCAD | Customizer parameters, modules and functions, includes, call graph, checks | own reader | first |
| DWG R13 to R2018 | Same as DXF, plus summary info and preview | own reader, written from a format specification | later |
| Fusion 360, SolidWorks, Creo, NX, CATIA, Inventor, Rhino, ACIS | Feature trees, parameters and metadata | own readers, written from the [cadmpeg format documentation](https://github.com/cadmpeg/cadmpeg/tree/main/docs/formats) (CC BY 4.0) | later |

## Until the later formats arrive

For DWG and the closed native formats the first release detects the format and its version, says what it can't read yet and asks for an export the agent can use: DXF for drawings, STEP AP242 for models.

## Known limitations

- stepq is young (one maintainer, first released in 2026-09). Rezzer pins it, keeps it behind its own STEP interface and uses the NIST test models as reference files
- cadmpeg reads real feature trees and parameters from SolidWorks, Creo, NX and Fusion files, far above its own "opened" label. But design data needs a full geometry decode (44 s and 3.1 GB on a 20.6 MB FreeCAD file), it adds about 72 MB to the binary and it refuses a whole file over one unusual record. That's why Rezzer doesn't link it. Rezzer writes its own readers for these formats from cadmpeg's format documentation, and reads only what an agent needs, without decoding all the geometry
- Exact volume and mass of STEP and IGES solids need a geometry kernel. Rezzer doesn't link one. Bounding boxes come from points and can be too large
- Native assemblies (SolidWorks, Inventor, Creo) are weak in every tested reader
- The G-code body inside `.bgcode` files is compressed and comes later; its metadata is read in the first release
- Some CAD libraries are GPL or AGPL (OpenCascade, LibreDWG, some KiCad and OpenSCAD parsers). Rezzer doesn't use them
- Against hostile files Rezzer uses limits on size, unpacking, depth, entity count and time. Referenced files (textures, buffers, xrefs) are listed and never opened outside the input folder
- Not covered by public test files: IGES notes, Fusion user parameters, DWG and DXF xrefs, a real Cura 3MF, Gerber X3 component layers, ifcXML, native assemblies

## Tested with

About 390 public test files (168 MB) from the FreeCAD, KiCad, cadmpeg, NIST, buildingSMART, ACadSharp and other public test suites. See [MANIFEST-cad.md](../spike-2026-10-19/MANIFEST-cad.md).
