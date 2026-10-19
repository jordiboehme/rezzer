# CAD and engineering formats

For engineering files Rezzer gives the agent what matters to an engineer: units, layers, parts lists, assembly and feature trees, sketch constraints, parameters, materials and print settings. Trees come out as Mermaid.

Status: planned for the first release. The exact list of CAD formats depends on tests that are still running.

This page is an overview. The crates below come from research done on 2026-10-16 and 2026-10-19. None of them has been run on real files yet, so they are candidates, not final choices. Versions and support levels change fast and will be checked again.

File types: see the table below. Rezzer detects the format by content, not by extension. This matters for CAD: `.prt` is used by both PTC Creo and Siemens NX. A Creo file starts with a `#UGC:` line. An NX file is an OLE compound file with Parasolid data inside. A DWG file names its version in the first 6 bytes (for example `AC1032`).

## Formats and candidates

| Format | What an agent gets | Candidates |
|---|---|---|
| DXF | Units, drawing extents, layers, blocks with attribute values (title block fields, part numbers), text, dimensions, layouts | [dxf](https://crates.io/crates/dxf), [acadrust](https://crates.io/crates/acadrust) |
| 3MF (also Bambu Studio, OrcaSlicer and PrusaSlicer files) | Objects and parts, units, bounding box, plates, materials and colors, print settings, print time and filament use | [lib3mf-core](https://crates.io/crates/lib3mf-core), [threemf](https://crates.io/crates/threemf) or Rezzer's own reader on zip and quick-xml |
| FreeCAD `.FCStd` | The parametric feature tree, parameters and expressions, sketches with constraints, spreadsheet cells, document metadata | Rezzer's own reader on zip and quick-xml, [cadmpeg](https://github.com/cadmpeg/cadmpeg) |
| STEP (AP203, AP214, AP242) | Originating system, author, date, units, assembly tree and parts list with quantities, part names, colors and layers, properties, materials, PMI (tolerances, datums, dimensions) | [stepq](https://crates.io/crates/stepq), [step-io](https://crates.io/crates/step-io), [step-p21](https://crates.io/crates/step-p21), [cadmpeg](https://github.com/cadmpeg/cadmpeg) or Rezzer's own Part 21 reader |
| IGES | Start and global sections (sender, system, units, author, date), entity counts, names, levels, notes | Rezzer's own reader, [cadmpeg](https://github.com/cadmpeg/cadmpeg) |
| IFC | Project, site, building, storeys and spaces as a tree, element counts by type, property sets, quantities, materials, classifications | [ifc-lite-core](https://crates.io/crates/ifc-lite-core) or the same Part 21 reader as for STEP |
| STL | Solid name or header text, triangle count, bounding box, surface area, volume, watertight check. STL stores no units, and the output says so | [stl_io](https://crates.io/crates/stl_io) |
| OBJ | Object, group and material names, counts, bounding box, comments | [tobj](https://crates.io/crates/tobj) |
| glTF and GLB | Generator, scene graph with names, meshes, materials, animations, user data | [gltf](https://crates.io/crates/gltf) |
| KiCad | Schematic: title block, sheet tree, parts with value and footprint (parts list), nets. Board: size, layers, footprints, nets, design rules | Rezzer's own s-expression reader, [lexpr](https://crates.io/crates/lexpr) |
| Gerber and Excellon | Layer function, generating software, units, apertures, board size, drill sizes and counts | [gerber_parser](https://crates.io/crates/gerber_parser), [gerberx2](https://crates.io/crates/gerberx2) |
| G-code | Slicer and settings from the comments, print time, filament use, layer count, temperatures, tool changes, thumbnails | Rezzer's own streaming reader, [gcode](https://crates.io/crates/gcode) |
| OpenSCAD | The source, plus modules and functions with parameters, Customizer variables, includes and a call tree | [tree-sitter-openscad-ng](https://crates.io/crates/tree-sitter-openscad-ng) |

## Native CAD formats

[cadmpeg](https://github.com/cadmpeg/cadmpeg) is a clean-room Rust project (Apache-2.0) that reads native CAD formats into one data model with features, parameters, sketches, constraints, assemblies and PMI. Every construct it can't read is reported as a named loss, which fits Rezzer's rule to say what it could not read. Its own support levels on 2026-10-19 (version 0.6.2):

| Format | cadmpeg's own level |
|---|---|
| STEP, IGES | full read and write |
| FreeCAD `.FCStd` | shape complete |
| Fusion 360 `.f3d` | design records: features with their operations, sketches, history |
| Inventor, SolidWorks, Creo, NX, CATIA V5, Rhino, ACIS | opened: sections, versions, assets, external references, some parameters |

These levels are cadmpeg's own scoring and still need to be checked on real files. For closed formats like Creo, SolidWorks, NX or CATIA, Rezzer reports what it can read (version, structure, external references, parameters) and asks for a STEP export (AP242 is best) for the rest.

## What Rezzer extracts

### Structure

- Assembly trees, feature trees, sheet trees and spatial trees (IFC), as Mermaid
- Layers, blocks, objects and parts

### Metadata

- Originating program and version, author, organization, dates, units

### Hidden logic

- Parameters and expressions, sketch constraints with their values, spreadsheet cells in FreeCAD files
- Parts lists with quantities
- PMI: tolerances, datums and dimensions
- Print and slicer settings

### Pictures and embedded files

- Thumbnails and previews stored in the file are extracted and listed as "needs vision"

## How it shows up in the output

A metadata table, then tables for parts, layers, parameters or settings (depending on the format). Then a Mermaid graph for the tree.

## Known limitations

- Most of the Rust crates for CAD are young (many first released in 2026). They will be tested on a public test corpus before Rezzer depends on them
- Exact volume, mass and surface area of STEP and IGES solids need a geometry kernel. Rezzer does not link one. A bounding box from points is possible, but it can be too large
- DWG: young pure-Rust readers exist ([acadrust](https://crates.io/crates/acadrust), [cadkit](https://crates.io/crates/cadkit)). They are not tested yet. Rezzer plans its own DWG reader, written from a format specification, that runs inside the binary
- Some CAD libraries are GPL or AGPL (OpenCascade, LibreDWG, some KiCad and OpenSCAD parsers). Rezzer doesn't use them. Everything runs inside the Rezzer binary, without external programs
- Against hostile files Rezzer uses size, decompression, entity count and time limits

## Tested with

Not tested yet. The CAD tests are still running, and there is no test file list yet.
