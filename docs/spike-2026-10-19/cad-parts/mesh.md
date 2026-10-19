# CAD spike part C: mesh, 3D print and manufacturing formats

Date: 2026-10-19. Throwaway spike, no Rezzer code. Group `mesh`.
Probes: `spike/cad-test/mesh/` (one crate per probe, `run.sh` for all pairs, `huge.sh` for big
generated files, `gen/gen_synthetic.py` for hostile files). Corpus: `spike/corpus/cad/mesh/` (84 files,
55 MB, see `MANIFEST-part.md`). Raw probe output was throwaway (scratchpad) and is not kept.
Versions checked today; re-check before relying on them.

Legend: **E** exposed in the crate's public API, **P** partial (see note), **N** not exposed,
**-** not supported. `(src)` = scored from the crate source only. A crate that only prints text with
no field in its model scores P at best.

Course correction relayed by the coordinator during the spike (from the project owner): every reader runs in-process in the one
binary, no external programs. This report recommends no slicer or other program as a helper.
PrusaSlicer 2.9.6 was only used on this machine to generate test files.

## 1. Recommendation in short

| Format | Use | Read ourselves | First release? |
|---|---|---|---|
| 3MF (+ Bambu/Orca, Prusa, Cura) | **own reader** on `zip` + `quick-xml` + `serde_json` (570 lines incl. all slicer configs, 0.09 to 0.16 s for the 20 MB XML of `orca_sliced.3mf`) | everything; use `lib3mf-core` source only as a reference for Bambu config field names | yes |
| STL | **own reader** (110 lines, std only, streaming) | everything; `stl_io` loses header, solid names, colors, drops data silently | yes |
| OBJ + MTL | **own line reader** (85 lines, std only); `tobj` only behind `load_obj_buf` if a mesh is ever needed | comments, o vs g, smoothing groups, MTL with texture options, external refs | yes |
| glTF / GLB | **own reader** on `serde_json` (124 lines, GLB JSON chunk only) | everything; `gltf` is a fine typed model but rejects any file with an unknown required extension (Draco, meshopt) and `gltf::import` follows `../` and `file://` URIs | yes |
| G-code (text) | **own streaming line reader** (207 lines incl. bgcode, constant memory, 160 MB/s) | everything; all three crates either fail the whole file, drop comments or need the file in memory | yes |
| Prusa binary G-code (.bgcode) | **own block reader** (about 60 lines, `miniz_oxide` for deflate) for metadata and thumbnails; G-code blocks (heatshrink/MeatPack) not needed for knowledge | metadata blocks, thumbnails | yes (metadata), later (G-code body) |

Reasons in short:

1. The 3MF crates are validators first. All four reject at least one real file outright (threemf fails
   on the BambuStudio root model; the other three reject more):
   duplicate metadata names (OrcaSlicer writes `OrcaSlicer` twice, 3MF Consortium sample
   `dodeca_chain_loop_color.3mf` has two `Description`), a relationship to a thumbnail that is not in
   the package (PrusaSlicer CLI and OrcaSlicer files), a production sub-model without `<build>`
   (BambuStudio), Bambu namespaces and relationship types. A knowledge reader must read these files
   and report the defect as a loss, not refuse them.
2. Only `lib3mf-core` understands slicer settings, and only Bambu/Orca. It has no Prusa or Cura
   support, keeps metadata in a `HashMap` (order lost), never computes `is_manifold`, overflows the
   stack on a component cycle (process abort) and reads any zip entry fully into memory (6.1 GB RSS on
   a 2 MB zip bomb in a config part).
3. Mesh formats carry their knowledge in the parts the crates throw away: STL header text, solid
   names and facet colors; OBJ comments, object vs group, smoothing groups, MTL texture options;
   glTF extensions the crate does not know; G-code comments and slicer settings blocks. Reading
   them is short, std-only code.
4. Untrusted references: `tobj::load_obj` opens `mtllib ../obj/cornell_box.mtl` outside the
   folder (proven), `gltf::import` reads `../../../../etc/hosts` as a buffer (proven, 216 bytes
   loaded). Rezzer must list references and never follow them outside the input folder.

## 2. Corpus

84 files (55 MB), details in `spike/corpus/cad/mesh/MANIFEST-part.md`.

| Group | Files | Source and license | Covers |
|---|---|---|---|
| 3MF core/extensions | 12 | 3MF Consortium 3mf-samples (BSD-2), telecos/lib3mf_rust (MIT) | units, metadata, names, components, build transforms, base materials, color groups, vertex colors, textures, production UUIDs and object thumbnails, beam lattice, slice stack |
| 3MF slicer projects | 6 (incl. `orca_sliced.3mf`, 20 MB model XML) | BambuStudio, OrcaSlicer, PrusaSlicer repos (**AGPL-3.0 data, flagged**), bambox fixture (AGPL-3.0, exported by BambuStudio 2.5) | Bambu/Orca `project_settings.config` (JSON, 322 to 536 keys), `model_settings.config` (objects, parts, plates), `slice_info.config` (print time, weight, filament use, warnings), `plate_1.gcode`, 5 plate pictures; Prusa `Slic3r_PE.config` (376 keys INI), `Slic3r_PE_model.config` (volumes, negative volume, layer ranges), custom G-code per Z |
| 3MF generated | 1 | PrusaSlicer 2.9.6 CLI from BSD-2 input | Prusa CLI 3MF with a dangling thumbnail relationship |
| STL | 3 | stl_io (MIT), medo64/TmpUsb (MIT) | binary, ASCII with solid name, real watertight part |
| OBJ/MTL | 5 | tobj (MIT), generated | multi-object, two MTL libraries, inline MTL comment, texture names with spaces, vertex colors, exporter OBJ without names |
| glTF/GLB | 22 (9 models) | Khronos glTF-Sample-Assets: CC0 (AnimatedCube, SimpleSkin, Cameras, SimpleInstancing, ToyCar) and CC-BY-4.0 (Box, BoxTextured, CesiumMan, MaterialsVariantsShoe) | asset copyright and generator, node tree, skins, animations, cameras, external buffers and textures, GLB, Draco (required extension), KHR_materials_variants, clearcoat/sheen/transmission, texture transform, GPU instancing, extras |
| G-code | 9 | PrusaSlicer (generated, and AGPL test data), Bambu plate G-code (AGPL), Cura 5.6 (medo64/TmpUsb, MIT), ForgeX fixture (Apache-2.0), gcode-preview CNC files (MIT) | Prusa settings footer, PNG thumbnail, MMU tool changes, objects_info; Bambu HEADER/CONFIG blocks and proprietary M-codes; Cura `;FLAVOR ;TIME ;MINX`, `;MESH:` names, `;SETTING_3`; bgcode; CNC router (G21, S12000) and Mach3 plasma (G20, paren comments) |
| Synthetic hostile | 26 | generated, CC0 | see section 9 |

Gaps (no redistributable file found in time):
- **Real Cura 3MF project** (`Cura/*.cfg`): only a synthetic file written from memory of the Cura 5
  writer (flagged in the manifest). The INI part is trivial; the structure is unverified.
- **OrcaSlicer G-code** and an Orca sliced `.gcode.3mf`: Orca writes the same format as BambuStudio
  (shared code base), so the Bambu files stand in.
- **G-code with QOI/JPG thumbnails**: the only QOI sample found (gcode-preview 3DBenchy) has an
  unclear model license; the PrusaSlicer CLI does not render thumbnails. PNG thumbnails are covered.
- **bgcode thumbnail block**: same reason (CLI); the block layout is read in the own probe from the
  spec, not proven on a file.
- **STL with VisCAM/SolidView colors from a real exporter**, and a real exporter header text: only
  synthetic.
- **Real CNC files from a CAM system with tool table comments** (Fusion, FreeCAD Path LGPL): one
  synthetic mill program plus two MIT demo files.
- 3MF from 3MF test_suites (BSD-2, 2,271 files) not taken; the examples repo covers the extensions.

## 3. Method

- One probe binary per crate, own target dir `spike/cad-test/mesh/target`. Probes print what the
  crate's model exposes. Own readers: `p-own3mf`, `p-ownstl`, `p-ownobj`, `p-owngltf`, `p-owngcode`.
- `run.sh` runs every (probe, file) pair in its own process with a 20 s alarm, records exit code,
  wall time and peak RSS (`/usr/bin/time -l`). `huge.sh` generates 150 to 255 MB files into the
  scratchpad, runs them with a 60 s alarm and deletes them.
- APIs scored from `~/.cargo/registry` sources first, then confirmed on corpus output.
- Licenses from `cargo metadata` over the full resolved tree of every probe.
- Build time and binary size are left to the coordinator.

## 4. Candidates

| Crate | Version (date) | License | MSRV | Activity | Transitive tree (resolved) |
|---|---|---|---|---|---|
| lib3mf-core | 0.4.0 (2026-03-04) | BSD-2-Clause | 1.88 | 12 stars, 0 commits in 90 d, last push 2026-06, 4 releases | 56 crates, all permissive (default features off; `crypto` adds rsa, aes-gcm, x509) |
| threemf | 0.8.0 (2026-03-05) | 0BSD | 1.56 | 49 stars, 2 commits in 90 d, 9 releases | 25, permissive |
| lib3mf (telecos/lib3mf_rust) | 0.1.6 (2026-03-13) | MIT | none | 5 stars, 40 commits in 90 d but no release since March | 18 with `default-features = false`; defaults pull parry3d, nalgebra, clipper2 (88 crates) |
| threemf2 | 0.4.1 (2026-08-17) | MIT OR Apache-2.0 | 1.89 | 3 stars, 6 commits in 90 d | 41, permissive |
| stl_io | 0.11.0 (2026-03-15) | MIT | 1.85 | 64 stars, 0 commits in 90 d, 5.1 M downloads | 1 (no deps) |
| tobj | 4.0.5 (2026-08-02) | MIT | none | 275 stars, 8 commits in 90 d, 3 M downloads | 16, permissive (ahash default) |
| obj-rs | 0.7.4 (2024-09-28) | Apache-2.0 OR MIT | none | 96 stars, 0 commits in 90 d | 10 |
| gltf (+ gltf-json) | 1.4.1 (2024-05-10) | MIT OR Apache-2.0 | 1.61 | 644 stars, 0 commits in 90 d, 91 open issues, no release since 2024 | 40 with `import` (pulls `image`), fewer without |
| gcode | 0.7.0 (2026-04-02) | MIT OR Apache-2.0 | 1.85 | 109 stars, 4 commits in 90 d | 16 |
| g-code | 0.6.0 (2026-03-04) | MIT OR Apache-2.0 | none | 14 stars, 0 commits in 90 d | 26 (peg, rust_decimal, codespan) |
| gcode-nom | 0.7.0 (2026-04-11) | MIT | **1.94** | 8 stars, 3 commits in 90 d | 33 (nom, inflate, heatshrink) |
| own readers | - | - | - | zip 9.0.5, quick-xml 0.42.0, serde_json 1.0.154, miniz_oxide 0.8.9 | 3MF 25, glTF 11, G-code 13, STL/OBJ 0 |

All trees: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, 0BSD, Unlicense, Unicode-3.0 only. `r-efi`
(MIT OR Apache-2.0 OR LGPL-2.1) appears via getrandom on UEFI only; MIT applies.

## 5. 3MF

### 5.1 Applicable items

Units, metadata (Title, Designer, Application, CreationDate, License/Copyright), objects with names,
part numbers and types, component hierarchy and build items with transforms (assembly), triangle
counts and bbox, base materials and color groups, textures, extensions (production, materials, beam
lattice, slice) and required extensions, thumbnails (pictures for vision), slicer settings per
vendor, losses. Not applicable: parameters, constraints, PMI.

### 5.2 Scoring

| Item | lib3mf-core | threemf | lib3mf (telecos) | threemf2 | own reader |
|---|---|---|---|---|---|
| Opens real slicer files | P (rejects Orca: duplicate metadata; Orca badge and Prusa CLI: dangling thumbnail rel; consortium dodeca_chain_loop_color: duplicate Description) | P (rejects Bambu root model) | N (rejects all Bambu/Orca files, 2 of 3 Prusa files and 2 consortium samples) | P (rejects Bambu sub-model without build, Orca badge, Prusa CLI) | E (all 19 real 3MF files) |
| Unit | E | E | E (string) | E | E |
| Metadata in document order | P (HashMap, order and duplicates lost) | E | E | E | E |
| Object name, type, part number, UUID | E | P (no type, no UUID) | E | E | E |
| Components with transform and `p:path` (assembly) | E | P (no path) | E | E | E (tree walk with cycle and depth guard) |
| Build items with transform, printable, part number | E | P | P (no printable) | E | E |
| Triangle/vertex counts | E | E | E | E | E (streamed, no mesh kept) |
| Bbox (world, through transforms) | E (`compute_stats`) | N | N | N | E |
| Area, volume | E (`compute_stats`) | N | N (needs `mesh-ops`) | N | N (not needed for 3MF; STL reader has it) |
| Manifold check | P (field exists, never computed: always `false`) | N | N (src: validator has it) | N | N |
| Base materials, color groups | E | P (base materials only) | E | E | E |
| Textures and texture groups | E | N | E | E | E |
| Beam lattice | E | N | E | E (src) | P (beam count) |
| Slice stack | E | N | E | E | P (count and part path) |
| Thumbnails as pictures | P (stats flag, object thumbnail path) | N | E (package thumbnail) | E (handles) | E (all PNG/JPG parts listed for vision) |
| Bambu/Orca `project_settings.config` | E (typed subset + `extras` map) | N | N | N | E (JSON Value, curated keys) |
| Bambu/Orca `model_settings.config` (objects, parts, plates) | E | N | N | N | E (generic key/value) |
| Bambu `slice_info.config` (time, weight, filaments, warnings) | E | N | N | N | E (generic key/value) |
| Embedded `plate_N.gcode` header | P (path only) | N | N | N | E (header block parsed) |
| PrusaSlicer `Slic3r_PE.config` + `Slic3r_PE_model.config` | N | N | N | N | E |
| Prusa layer ranges, custom G-code per Z | N | N | N | N | E |
| Cura `Cura/*.cfg` stacks | N | N | N | N | E (synthetic file only) |
| Losses reported (cycles, dangling refs, limits) | P (errors abort) | N | P (errors abort) | P (errors abort) | E |

### 5.3 Evidence

```
# lib3mf-core, bambu_reference.gcode.3mf (typed Bambu parsers)
bambu Metadata/slice_info.config: SliceInfoData { client_type: Some("slicer"), client_version: Some("02.05.00.66"),
  plates: [SlicePlateInfo { id: 1, prediction: Some(7745), weight: Some(32.12), filaments: [SliceFilamentUsage { id: 1,
  type_: Some("PLA"), color: Some("#FFFFFF"), used_m: Some(7.85), used_g: Some(23.43) }, ...],
  warnings: [SlicerWarning { msg: "bed_temperature_too_high_than_filament", level: Some("3"), error_code: Some("1000C001") }] ...
bambu Metadata/project_settings.config: printer_model=Some("Bambu Lab P1S") layer_height=Some(0.2)
  filament_type=["PLA", "ASA", "PETG-CF", "PLA", "TPU"] wall_loops=Some(2) infill=Some("15%") support=Some("tree(auto)") extras=520

# lib3mf-core, bambu_pa_pattern.3mf
ERROR compute_stats: Validation failed: Missing required <build> element. Every 3MF model must contain a <build> element
# lib3mf-core / lib3mf, orca_sliced.3mf (20 MB XML)
ERROR parse_model: Validation failed: Duplicate metadata name 'OrcaSlicer'. Each metadata name must be unique
# lib3mf (telecos), prusa_fdm_roundtrip1.3mf
ERROR [E2003] Invalid XML structure: Unexpected attribute 'slic3rpe:fuzzy_skin' in triangle element.
# lib3mf (telecos), sphere_logo.3mf (3MF Consortium sample)
ERROR [E2004] Invalid 3MF format: Invalid content type 'application/vnd.ms-package.3dmanufacturing-3dmodeltexture' for PNG extension

# own reader, prusa_fdm_roundtrip1.3mf
Metadata/Slic3r_PE_model.config: 88 entries
  config/object[1] name = Shape-Box
  config/object[1]/volume name = Generic-Box
  config/object[1]/volume volume_type = NegativeVolume
  config/object[1]/volume source_file = apply_test_object.3mf
Metadata/Prusa_Slicer_custom_gcode_per_print_z.xml:
  custom_gcodes_per_print_z/code [("print_z", "13.399999618530273"), ("type", "1"), ("extra", "pause"), ("gcode", "M601")]
  custom_gcodes_per_print_z/code [("print_z", "15.050000190734863"), ("extruder", "2"), ("color", "#DB5182"), ("gcode", "M600")]
prusa Slic3r_PE.config: 376 keys
  filament_type = PVA;PVA;PVA;PVA;PVA
  fill_density = 15%

# own reader, bambu_pa_pattern.3mf (production sub-model, Bambu parts and plates)
[3D/3dmodel.model] object id=2 ... comps=["/3D/Objects/Cube_1.model#1 @(0.00,0.00,0.00)"]
build item 3D/3dmodel.model#2 printable=Some("1") at (82.79,86.07,0.42)
build: triangles(instanced)=12 bbox=Some(([80.29, 83.57, 0.0], [85.29, 88.57, 0.85]))
  config/object[2]/part[1] [("id", "1"), ("subtype", "normal_part")]
  config/plate thumbnail_file = Metadata/plate_1.png

# own reader, syn_component_cycle.3mf
losses: ["component cycle: [(\"3D/3dmodel.model\", \"1\"), (\"3D/3dmodel.model\", \"2\")] -> 3D/3dmodel.model#1"]
```

### 5.4 Bugs found

- lib3mf-core 0.4.0: `compute_stats` recurses through components without depth limit or visited set:
  a component cycle aborts the process with a stack overflow (`max_depth` exists only in
  `PartResolver::resolve_meshes`). `ZipArchiver::read_entry` reads entries with `read_to_end`, no size
  cap: 2.0 GB RSS on the model bomb, 6.1 GB RSS on a bomb in `project_settings.config`, which
  `compute_stats` reads whenever `Application` contains "Bambu" or "Orca". `is_manifold` is never set.
  Rejects duplicate metadata names and a thumbnail relationship to a missing part (whole file lost).
  Vendor parsing is chosen by the `Application` string and parse errors are swallowed (`if let Ok`).
  `ModelStats.system_info` fingerprints the host (CPU count, SIMD), irrelevant for a document.
- lib3mf (telecos) 0.1.6: the zip bomb is read through `take(1 GiB + 1)`, so memory stays low, but
  the cut-off stream then fails with "Model must contain a <build> element" instead of a size error.
  Strict validation rejects all slicer files in the corpus and the
  consortium sample `sphere_logo.3mf`; validation of a 5,000-level chain takes 13.9 s. Has a 1 GiB
  per-part cap and a circular component check (good).
- threemf 0.8.0: reads every `*.model` entry regardless of relationships; no `Debug` on model types;
  a truncated zip reports "Error writing ZIP file"; 6.1 GB RSS on the zip bomb.
- threemf2 0.4.1: `MissingValue("Model::build")` on Bambu sub-models; `Zip(FileNotFound)` for a
  dangling thumbnail relationship; 2.0 GB RSS on the zip bomb.
- Own reader gaps (fixable): does not yet flag a relationship to a missing part as a loss; entity
  references in metadata text (`&amp;`) arrive as separate quick-xml 0.42 `GeneralRef` events and
  need handling; a truncated zip (no central directory) fails like all others; a local-header scan
  could recover parts.

### 5.5 Recommendation

Own reader. It is the only candidate that opens every real file, and it is short: generic model
scanner, OPC relationships, size and ratio limits per part, assembly walk with cycle and depth
guard, plus four small vendor readers (Bambu/Orca JSON + XML key/value, Prusa INI + XML, Cura INI,
plate G-code header). Use lib3mf-core's `bambu_config.rs` only as documentation of field meanings
(BSD-2, reading is fine). Ready for the first release.

### 5.6 Presentation ideas

- Frontmatter: unit, application, slicer and version, printer model, object count, plate count,
  estimated time, filament weight.
- Tables: objects (name, part number, type, triangles, bbox size, material/color, extruder, parts
  with subtype: part, negative volume, modifier, support blocker); build/plate items with position;
  filaments (type, color, grams, meters); print settings (curated 20 to 40 keys, full list in a
  collapsible block); per-object overrides and layer ranges; custom G-code per height (pause, color
  change); slicer warnings.
- Mermaid: assembly tree `build -> object -> component (sub-model path)`, plates -> objects.
- Pictures: plate and object thumbnails listed as needing vision with role (plate, top, pick, cover).
- Losses: rejected parts (limits), cycles, dangling references, unknown required extensions.

## 6. STL

### 6.1 Applicable items

Binary vs ASCII, header text (exporter), solid names, triangle count (header vs real), bbox and size,
area, volume, watertight check, colors (VisCAM/SolidView facet bits, Materialise header), units not
stored (say so).

### 6.2 Scoring

| Item | stl_io | own reader |
|---|---|---|
| Binary vs ASCII detection | P (by "solid " prefix; bare `solid` line read as binary and fails) | E (size check 84 + 50 n first) |
| Header text / exporter | N (read and dropped) | E |
| Solid names, multi-solid | N (stops after first `endsolid`, rest dropped silently) | E |
| Triangle count, header vs real | P (trusts header: too high = hard error, too low = data dropped silently) | E (real count, mismatch reported) |
| Bbox, area, volume | P (mesh only, computed by caller) | E |
| Watertight check | E (`IndexedMesh::validate`) | E (directed edge balance) |
| Colors (facet attribute, Materialise header) | N | E |
| Streaming, constant memory | P (iterator exists, `read_stl` indexes everything) | E for counts and bbox; watertight table costs memory (see 6.5) |

### 6.3 Evidence

```
# stl_io, syn_ascii_noname.stl ("solid" without a name)
ERROR failed to fill whole buffer
# stl_io, syn_ascii_multisolid.stl (lid 12 + base 6 facets)
faces=12 ...                      <- second solid silently dropped
# stl_io, syn_count_too_low.stl (header 6, file holds 12)
faces=6 ... validate: Err("did not find facing edge for face #1, edge #v1 -> #v2")
# own, syn_count_too_low.stl
notes: ["triangle count in header 6 but file holds 12 (remainder 0 bytes); using 12"]
# own, syn_color_materialise.stl
facet attribute values (top 5): ["0x7c00x6", "0x001fx6"]
notes: ["Materialise header color RGBA [255, 128, 0, 255]", "Materialise header material [...]"]
# own, tmpusb_case_top.stl
units: not stored in STL (usually mm, unknown)
triangles=772 size=[19.0, 28.5, 7.0] area=2121.688 volume=1147.436
watertight (every edge used once in each direction): true (unbalanced edges: 0)
```

### 6.4 Bugs found

stl_io 0.11.0: bare `solid` first line misdetected as binary; only the first ASCII solid is read;
header triangle count trusted (data loss or failure). The binary-with-`solid`-header case was
handled correctly in this test.

### 6.5 Recommendation

Own reader, ready for the first release. Memory: on a 255 MB ASCII STL with 5.7 M distinct
vertices the own reader peaked at 916 MB because of the edge table (stl_io 870 MB because it indexes
all vertices). Cap the edge table lower (for example 2 M edges) and report "watertight: unknown"
above it, or quantize to compact keys. Counts, bbox, area and volume stay constant memory (binary
238 MB: 3 MB RSS, 0.33 s).

### 6.6 Presentation

One table: format, header text, solids with triangle counts, size x/y/z, area, volume, watertight,
colors. Always the line "units are not stored in STL". Volume only when watertight.

## 7. OBJ + MTL

### 7.1 Applicable items

Objects and groups, materials with properties and texture files (external references), comments
(exporter, units hints), counts, bbox, smoothing groups, vertex colors, free-form statements.

### 7.2 Scoring

| Item | tobj | obj-rs (raw) | own reader |
|---|---|---|---|
| Object (`o`) vs group (`g`) names | P (treated the same; `o Housing` lost when `g` follows) | P (`groups` map; only the last `o` name kept) | E |
| Smoothing groups | N | E | E |
| Comments / exporter | N | N | E |
| Counts, bbox | E (per model) | E | E |
| Vertex colors | E | N (fails on `v x y z r g b`) | E (flag) |
| MTL properties | E (+ `unknown_param` for PBR keys) | P (fails on inline comment and texture options) | E |
| Texture files, options | P (`-bm 0.5 textures/x.png` kept as file name) | P | E (file and options split) |
| External refs listed, not followed | P (`load_obj` follows `../`; `load_obj_buf` gives control) | E (never opens files) | E |
| Free-form statements | E (ignored silently) | N (`unimplemented!()` panic) | E (counted) |

### 7.3 Evidence

```
# tobj load_obj, syn_mtllib_sibling.obj (mtllib ../obj/cornell_box.mtl)
material "red" Kd=Some([1.0, 0.0, 0.0]) ... map_Kd=Some("this diffuse texture has spaces.jpg")   <- read from outside the folder
# tobj, syn_rich.obj
model name="housing_bottom" ... model name="gasket_ring"     <- object names Housing/Gasket gone
material "painted_steel" ... normal=Some("-bm 0.5 textures/paint_normal.png") unknown={"Pm": "1.0", "Pr": "0.4"}
# obj-rs, syn_freeform.obj
thread 'main' panicked at obj-rs-0.7.4/src/raw/object.rs:140:34: not implemented
# own, syn_rich.obj
comments (3): ["Exported by SyntheticExporter 1.0 (rezzer spike)", "Units: millimeters", "Author: spike generator"]
objects (o) with face counts: [("Housing", 4), ("Gasket", 2)]
groups (g): {"gasket_ring": 2, "housing_bottom": 2, "housing_top": 2}
smoothing groups: {"1": 2, "2": 2, "off": 2}
    map_Bump -> texture file "textures/paint_normal.png" options=["-bm", "0.5"] (missing)
    map_Kd -> texture file "C:\\Users\\someone\\Textures\\rubber.jpg" (outside folder or absolute path)
```

### 7.4 Bugs found

tobj 4.0.5: `o` and `g` merged; texture options kept in the file name; `load_obj` follows any
relative or absolute `mtllib`. obj-rs 0.7.4: panics on valid free-form OBJ statements, fails on
vertex colors and on MTL inline comments and texture options; one object name only.

### 7.5 Recommendation

Own line reader, ready for the first release (3 MB RSS and 0.9 s on a 251 MB OBJ; tobj 1.07 GB).

### 7.6 Presentation

Tables: objects with groups and face counts, materials (Kd as hex color, Ns, d, PBR keys), texture
references with status (present, missing, outside folder: not read). Comments block (first lines,
often exporter and units). Units unknown unless a comment says so.

## 8. glTF / GLB

### 8.1 Applicable items

Asset generator, version and copyright; extensionsUsed and extensionsRequired; scene/node tree with
names and transforms; meshes and primitives; accessor min/max bbox; materials (PBR and KHR
extensions); textures and external URIs; animations, skins, cameras; extras at every level;
KHR_materials_variants.

### 8.2 Scoring

| Item | gltf 1.4.1 (`Gltf::open`) | own reader (serde_json) |
|---|---|---|
| Opens files with unknown required extension (Draco, meshopt, vendor) | N (validation error, whole file lost) | E |
| Asset generator, copyright, version | E | E |
| extensionsUsed / Required | E | E |
| Node tree with names, TRS, mesh/skin/camera | E (no cycle check: node cycle passes validation) | E (cycle and depth guard, reported) |
| Primitive count, modes, POSITION min/max | E | E |
| World bbox through transforms | N (caller) | E |
| PBR and known KHR material extensions | E (typed, per feature flag) | E (raw JSON, all extensions) |
| Unknown extensions | P (`extension_value` map with `extensions` feature) | E |
| Extras at every level | E (feature `extras`, raw JSON) | E |
| KHR_materials_variants | E | E |
| Animations, skins, cameras | E | E (counts and names) |
| External URIs listed, not followed | P (`Gltf::open` lists; `import` follows `../` and `file://`) | E |
| Truncated GLB | N (fails) | E (JSON chunk intact: full metadata) |

### 8.3 Evidence

```
# gltf, Box_draco.gltf
ERROR invalid glTF: accessors[0].bufferView: Missing data; ...; extensionsRequired[0] = "KHR_draco_mesh_compression": Unsupported extension;
# own, Box_draco.gltf
mesh 0 "Mesh" primitives=1 materials=[Some(0)] prim_ext=[["KHR_draco_mesh_compression"]]
material 0 "Red" pbr={"baseColorFactor":[0.8,0,0,1],"metallicFactor":0,"roughnessFactor":1}
world bbox: Some(([-0.5005, -0.5005, -0.5005], [0.5005, 0.5005, 0.5005]))
# gltf::import, syn_external_refs_bufonly.gltf (buffer uri "../../../../../../../../../../etc/hosts")
import ok: buffers=[216] images=0
# gltf, syn_node_cycle.gltf: accepted; the probe's guarded walk shows
      node 0 Some("A") ... CYCLE
# gltf, MaterialsVariantsShoe.glb
KHR_materials_variants: ["midnight", "beach", "street"]
mesh 0 ... variant_mappings=[[(Some(0), [0]), (Some(1), [1]), (Some(2), [2])]]
node 2 Some("Shoeobj") ... extras={"name":"Shoe.obj"}
# own, syn_truncated_CesiumMan.glb (half the file)
GLB version=2 declared_length=438044 file_length=219022
counts: scenes=1 nodes=22 meshes=1 ... animations=1 skins=1
```

### 8.4 Bugs and limits

gltf 1.4.1: unknown required extensions make the document invalid (by spec a renderer must refuse,
but a knowledge reader should not); no node cycle check, so a caller's recursive walk loops; `import`
follows `file://` and `../` URIs and decodes images with the `image` crate; no release since 2024-05
(91 open issues). `Gltf::from_slice_without_validation` exists and would avoid the extension error,
but then nothing is validated.

### 8.5 Recommendation

Own reader on `serde_json` (124 lines). It never loads the BIN chunk, keeps every extension and
extras, survives truncation, and guards cycles. Ready for the first release. Optional later: `gltf-json`
types without validation if typed access is wanted.

### 8.6 Presentation

Frontmatter: generator, copyright, glTF version, required extensions. Mermaid scene graph (nodes
with names, mesh/camera/light/skin labels; cap at about 200 nodes, collapse long chains). Tables:
meshes (primitives, material, bbox), materials (base color, metallic, roughness, extensions),
variants, animations, skins, cameras, external files (present, missing, outside folder). Extras as
key/value tables: this is where CAD exporters put part numbers and properties.

## 9. G-code

### 9.1 Applicable items

Slicer header/footer comments and settings, estimated time, filament use, layer count, thumbnails,
temperatures, units, absolute/relative, bbox of moves, tool changes, feed rates and spindle speeds,
work offsets, program comments, M-code inventory, object names, bgcode metadata.

### 9.2 Scoring

| Item | gcode 0.7.0 | g-code 0.6.0 | gcode-nom 0.7.0 | own reader |
|---|---|---|---|---|
| Parses real slicer output | P (whole file is `Err` on any diagnostic: Cura, Bambu, MMU Prusa failed) | P (fails on Bambu and on a CNC `%` end line) | E (line based, never fails) | E |
| Streaming / constant memory | N (`&str` of the whole file; 3.3 GB RSS on 144 MB) | N (8.0 GB RSS on 144 MB) | E (per line; bgcode needs whole file) | E (3 MB RSS) |
| Comments kept | E (`Comment` with kind; paren comment keeps the `)`) | P (bytes only, fields `pub(crate)`) | N (comment lines become `Nop`) | E |
| Slicer settings (`; key = value`, Cura `;SETTING_3`, Bambu header block) | P (raw comments) | P | N | E |
| Estimated time, filament, layers | P (raw comments) | P | N | E |
| Thumbnails as pictures | P (raw comment lines) | P | N | E (PNG/QOI/JPG blocks with size) |
| Temperatures, tools, feeds, spindle, offsets | E (codes with args) | P (Debug only) | P (G0/G1/G2/G3 params only; other codes dropped as `MDrop(n)` without args) | E |
| Units, G90/G91, M82/M83 | E | P | E (G20/G21/G90/G91) | E |
| Bbox of moves | N (caller) | N | P (positions for G0/G1/arcs) | E (all moves and extruding/cutting moves) |
| Object names (`;MESH:`, M486, objects_info, EXCLUDE_OBJECT) | P | P | P (M486 typed) | E |
| CNC program (N-numbers, O-number, paren comments) | E | N | N (all lines `Nop`) | E |
| Prusa bgcode metadata | N | N (not UTF-8) | P (Display text of blocks) | E |

### 9.3 Evidence

```
# gcode 0.7.0, cura56_tmpusb_case_top.gcode
parse returned Err with 97 diagnostics, first: Unexpected { actual: "W", expected: [Number] } line 28
# gcode-nom, syn_cnc_mill.nc
inventory=[("Nop", 37)]
# own, cura56_tmpusb_case_top.gcode
generator: ["FLAVOR:Marlin", "Generated with Cura_SteamEngine 5.6.0"]
  TIME = 370   LAYER_COUNT = 16   TARGET_MACHINE.NAME = Creality Ender-3 S1
  cura SETTING_3 extruder_quality: ["alternate_extra_perimeter = True", "material_print_temperature = 205.0", ...]
  cura SETTING_3 global_quality: ["material_bed_temperature = 63", "support_enable = True"]
objects: {"TmpUsb'H2 Case B.stl"}
# own, bambu_plate_1.gcode
bambu header: ["BambuStudio 02.05.00.66", "model printing time: 2h 1m 52s; total estimated time: 2h 9m 5s",
  "total layer number: 100", "total filament weight [g] : 23.43,8.70", ...]
M inventory: {..., "M620": 54, "M620.1": 106, "M621": 53, "M624": 250, "M625": 250, ...}
# own, prusa_test_multimaterial.gcode
estimated printing time (normal mode) = 8h 35m 49s
thumbnails (pictures, need vision): ["thumbnail 160x120 (28624 base64 chars)"]
tools: {"T0", "T1", "T2", "T3", "T4"} tool_changes=268
# own, gen_prusa_box.bgcode
file_metadata: ["Producer=PrusaSlicer 2.9.6"]
print_metadata (9 keys, compression=1): ["filament used [mm]=1257.10", "estimated printing time (normal mode)=12m 25s"]
slicer_metadata (347 keys, compression=1): [...]
# own, syn_cnc_mill.nc
units: ["G21"] tools: {"T1", "T2"} spindle_S: {"2500", "9000"} feed F min/max: (150.0, 900.0) work offsets: {"G54", "G55"}
program comments: ["BRACKET OP1 - ...", "MATERIAL: AL6061-T6", "T1 D=10.00 FLAT ENDMILL 3FL", "T2 D=6.80 DRILL 118DEG"]
```

### 9.4 Bugs and limits

gcode 0.7.0: one unexpected token anywhere and `parse` returns no program at all; parenthesis
comment values keep the closing `)`. g-code 0.6.0: all token fields are `pub(crate)`, so the model is
only reachable through `Debug` or bytes; fails on a valid `%` program end. gcode-nom 0.7.0: drops
comment text and the arguments of every M-code except M486; numbered CNC lines become `Nop`; MSRV
1.94. Own reader: Bambu T-codes such as `T1000`/`T255` are firmware specials, not tools; counting
`T` and `M6` together double counts on CNC; dialect tables are needed (Marlin, Klipper, Bambu,
Fanuc-style CNC).

### 9.5 Recommendation

Own streaming reader plus a small bgcode metadata reader (deflate via `miniz_oxide`, MIT/Zlib/
Apache). Ready for the first release. Decoding heatshrink/MeatPack G-code blocks in bgcode is only
needed for the bbox and is later work.

### 9.6 Presentation

Frontmatter: kind (3D print or CNC), generator, flavor, printer, estimated time, filament
(length, weight per tool), layer count, units. Tables: print settings (curated keys, full list
collapsible), temperatures, tools with change count, objects, CNC tool list from comments with
spindle and feed ranges and work offsets, G/M code inventory with a short meaning per code
(dialect aware). Thumbnails as pictures needing vision. Bbox of extruding/cutting moves.

## 10. Robustness

Each pair ran in its own process (20 s alarm; huge files 60 s). RSS is peak resident.

| File | lib3mf-core | threemf | lib3mf | threemf2 | own |
|---|---|---|---|---|---|
| syn_component_cycle.3mf | **abort, stack overflow** | ok (no cycle concept) | error, cycle detected | ok (no cycle concept) | ok, cycle reported as loss |
| syn_deep_chain_5000.3mf | ok, 17 MB | ok | ok, **13.9 s** | ok | ok, depth over 64 reported |
| syn_zipbomb_2g.3mf (2 MB zip, 2 GiB model) | **2.0 GB RSS**, 0.8 s | **6.1 GB RSS**, 2.6 s | stops at its 1 GiB read cap, then misleading error "Model must contain a <build> element", 0.36 s, 3 MB | **2.0 GB RSS**, 2.7 s | rejected (declared size over 256 MiB), 2 MB |
| syn_zipbomb_config.3mf (bomb in project_settings) | **6.1 GB RSS**, 1.3 s | ok (not read) | ok (not read) | ok (not read) | model read, config rejected as loss |
| syn_truncated_cube_gears.3mf | error | error ("Error writing ZIP file") | error | error | error (no EOCD) |

| File | crate | own |
|---|---|---|
| big ASCII STL 255 MB, 1.9 M facets | stl_io: ok, 2.0 s, 870 MB | 1.05 s, 916 MB (edge table) |
| big binary STL 238 MB, 5 M facets | stl_io: ok, 0.56 s, 194 MB | 0.33 s, 3 MB |
| syn_count_too_high.stl | stl_io: error, no data | ok, mismatch reported |
| big OBJ 251 MB | tobj 1.11 s, 1.07 GB; obj-rs 1.37 s, 193 MB | 0.92 s, 3 MB |
| syn_freeform.obj | tobj ok; **obj-rs panic** | ok |
| syn_node_cycle.gltf | gltf: accepted (no check) | cycle reported |
| syn_truncated_CesiumMan.glb | gltf: error | full metadata |
| syn_glb_bad_chunklen.glb (2 GiB chunk length) | gltf: error (no huge allocation) | error, limit |
| big G-code 144 MB (10 x real Prusa file) | gcode 1.34 s, **3.3 GB**; g-code 2.27 s, **8.0 GB**; gcode-nom 2.62 s, 2 MB | 0.94 s, 3 MB |

Parse speed on the biggest real files: own 3MF reader 0.09 to 0.16 s for `orca_sliced.3mf` (20 MB
XML, 256,568 triangles, 24 MB RSS; threemf 0.21 s, threemf2 0.14 s; lib3mf-core and lib3mf reject it); own G-code reader 0.09 s for the 15 MB PrusaSlicer file
(592,476 lines, about 160 MB/s); own glTF reader 0 ms on the 7.8 MB shoe GLB (JSON chunk only),
gltf 1.4.1 also fast but 17.5 MB RSS (reads the whole file).

## 11. Licenses

- All probe trees are permissive (see section 4). `lib3mf-core` `crypto` feature (rsa, aes-gcm,
  x509-parser) not measured; its docs claim about 300 crates with all features and about 154 minimal,
  but the measured minimal tree is 56, so the docs' counts are not reliable. Not needed. telecos `lib3mf` defaults pull parry3d, nalgebra and
  clipper2; turn defaults off.
- `gltf` with `import` pulls `image`; not needed for knowledge extraction.
- Corpus: slicer repository files are AGPL-3.0 data (flagged in the manifest). Khronos models are
  CC0 or CC-BY-4.0 per model; CC-BY needs attribution in the corpus manifest (done). Cesium logo in
  BoxTextured and CesiumMan carries a trademark note. 3DBenchy and CC BY-NC-SA files were excluded.
- No GPL/LGPL/AGPL crate was added to any Cargo.toml.

## 12. How Rezzer should present this group's knowledge

- One section "Manufacturing" for 3MF and G-code that answers what an engineer asks first: what is
  printed (objects, parts, assembly), with what (printer, nozzle, filaments with colors and grams),
  how (curated settings, per-object overrides, layer ranges, pauses and color changes), how long
  (estimated time), and what the slicer warned about.
- One section "Geometry summary" for all mesh formats: unit (or "not stored"), triangle count, bbox
  size, area and volume when watertight, watertight yes/no/unknown.
- Mermaid: 3MF assembly tree and plate to object mapping; glTF scene graph. Keep diagrams capped and
  say when they are truncated.
- Pictures: plate thumbnails, object thumbnails, textures, G-code thumbnails are extracted and listed
  as needing vision with their role.
- Losses, always explicit: rejected parts by size or ratio limit, cycles, dangling references,
  references outside the folder (listed, not read), unknown required extensions, header and real
  count mismatches, truncated packages.
