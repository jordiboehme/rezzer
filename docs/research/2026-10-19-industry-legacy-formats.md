# Rezzer research: old, uncommon and domain-specific formats in industry and science

Date: 2026-10-19. Question: which old, uncommon or domain-specific file formats are still used in
industry, science and lab work, where an AI agent and an engineer would gain from Rezzer reading
them? Rezzer reads knowledge, not text: structure, metadata, parameters and what drives them,
relations as graphs, signals and their meaning, units, revisions, and it says what it could not
read. Every reader runs in-process, Rezzer never runs code from files and never calls external
programs.

Scope: automotive, automation (legacy delta), energy, rail, marine, medical, geo, avionics,
space, engineering models and tools, generic legacy, science (added by the owner) and machine and
lab equipment output (added by the owner). Formats already covered elsewhere get one reference
line plus the delta only:

- the engineering formats report, `docs/research/2026-10-19-engineering-formats.md` (on branch
  `docs/engineering-formats`, not yet on this branch): AAS, AutomationML, DEXPI, OPC UA NodeSet2,
  PLCopen XML, IODD, IEC 61850 SCL, MTConnect, MTP, ESI, GSDML, ODVA EDS, L5X, SimaticML,
  TwinCAT, CODESYS, GAEB, BMEcat, ETIM, ECLASS, KBL/VEC, ReqIF, STEP AP242 XML, QIF, IPC-2581,
  ODB++, Eagle, Altium, EDIF, SPICE netlists and more
- the CAD spike, [`spike-2026-10-19/cad.md`](../spike-2026-10-19/cad.md): FreeCAD, STEP, IGES,
  DXF, DWG, 3MF, STL, OBJ, glTF, G-code, IFC, KiCad, Gerber, OpenSCAD
- the office, PDF and web-text spikes (office documents, spreadsheets, CSV, HTML, plain text)

Method: seven research agents, one per area, worked to one fixed field schema (detection
signature and name collisions, users with evidence, knowledge held, structure, openness with
quoted terms, Rust crates, reference libraries, samples, effort, fit, recommendation). Crate
versions, licenses, dates and download counts come from the crates.io API on 2026-10-19 (it
throttled late in the run, a few values come from the sparse index or lib.rs and say so).
Repository facts come from `gh api`. License and terms quotes come from the standards bodies' and
vendors' own pages where they could be fetched. Everything else is marked **[unverified]**. No
GPL, AGPL or LGPL source code was read; for such projects only the license is noted. No Rezzer
code was written, nothing from the downloads was executed. 29 small public sample files (about
1.1 MB) are in `spike/corpus/industry/<area>/`, listed with source URL, license and SHA-256 in
`spike/corpus/industry/MANIFEST-industry.md` (hashes recomputed when merging; all match).

Legend (same as the engineering report, plus a formulas facet):

- **Effort:** S under about 300 lines on quick-xml, zip, a text tokenizer or an existing reader;
  M 300 to 1000 lines; L over 1000 lines or a reverse-engineered binary; blocked = spec or license
  not obtainable.
- **Fit P/G/T/F/Tb:** 0 to 3 each for properties, graph (relations, signal flow, topology),
  timeline (revisions, history, time series), formulas (conversions, calibrations, equations,
  kinetic laws, projections) and tables.
- **Rec:** W1 first wave, W2 second wave, later, skip (detect and say what to export instead),
  covered, blocked.

## 1. Summary table

Main candidates from all areas (full tables per area in sections 2 to 8). "Crate" names the best
permissive Rust route, "own" means an own reader on existing dependencies is preferred.

| Format | Area | Structure | Openness | Rust route | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|
| A2L / ASAP2 | automotive | keyword text | ASAM members or buy | a2lfile 3.5.1 | S-M | 3/2/0/3/3 | W1 |
| DBC | automotive | text | Vector, no formal spec | can-dbc 10.0.0 | S | 3/2/0/2/3 | W1 |
| ARXML | automotive | XML | AUTOSAR license clause | autosar-data 0.23.0 | M | 3/3/1/2/3 | W2 (W1 after legal) |
| MDF / MF4 | automotive | binary | ASAM members or buy | falcon_mdf 0.7.1 (young) | M | 3/1/3/3/2 | W2 |
| OpenDRIVE | automotive | XML | free, no schema redistribution | own (xodr young) | S-M | 3/3/0/2/2 | W2 |
| LDF, DCM, ODX/PDX, OpenSCENARIO XML | automotive | text / XML | ISO or ASAM paid / free | own; lin-ldf, autors-dcm | S to M-L | see 2 | W2 |
| A2L + HEX/S19 | automotive | text records | public | ihex, srec + a2lfile | M | 3/1/0/3/3 | W2 |
| COMTRADE | energy | text + CSV/binary | IEEE/IEC paid, widely implemented | own | S | 3/1/3/3/3 | W1 |
| CGMES / CIM | energy | RDF/XML | IEC paid, profiles Apache-2.0 | own or cimoxide (young) | M | 3/3/2/1/2 | W2 |
| PSS/E RAW, MATPOWER | energy | text | vendor / BSD code | powerio 0.11.5 (young) | S-M | 3/3/0/1/3 | W2 |
| SunSpec Modbus models | automation | JSON | Apache-2.0 | own | S | 3/1/0/2/3 | W2 |
| STEP 7 AWL/SCL + symbol tables | automation | text | manuals free | own | S-M | 2/1/1/1/3 | W2 |
| railML | rail | XML | CC BY-NC-ND, registration | - | M | 2/3/1/0/2 | blocked |
| NMEA 0183 / AIS | marine | text | paid spec | nmea, nmea-parser | S | 2/0/3/0/3 | W2 |
| DICOM headers | medical | binary | free | dicom-object 0.10.0 | S | 3/2/1/0/3 | W2 |
| HL7 v2 | medical | pipe text | free after registration | hl7-parser 0.3.0 | S | 3/1/2/0/3 | W2 |
| GPX, KML, GeoJSON, Shapefile/DBF | geo | XML / JSON / binary | free | gpx, kml, geojson, shapefile, dbase | S | 2/0-1/0-3/0/2-3 | W2 |
| ARINC 424 (FAA CIFP) | avionics | fixed-width text | spec paid, data public | arinc424 0.4.0 | M | 3/2/1/0/3 | W2 |
| S1000D | aerospace | XML | spec free, samples unclear | own | M | 3/3/2/0/2 | W2 |
| XTCE | space | XML | free | own (sdtk as reference) | M | 3/2/1/3/3 | W1 |
| CCSDS NDM (OEM, OPM, CDM, ...) | space | KVN / XML | free | ccsds-ndm 0.0.9 or own | S-M | 3/1/3/1/3 | W1 |
| SPICE text kernels | space | text | NAIF rules | own (spicekit, anise) | S | 3/2/2/1/2 | W1 |
| PDS4 labels | space | XML | free | own | M | 3/2/1/0/3 | W1 |
| SCOS-2000 MIB | space | TSV folder | ESA ICD terms unclear | own | M | 3/3/1/3/3 | W2 |
| FMI modelDescription | models | XML in zip | free (BSD-2 XSD) | fmi-schema 0.8.0 or own | S | 3/2/0/1/3 | W1 |
| Simulink SLX + Stateflow | models | OPC zip XML | proprietary, undocumented | own | M | 3/3/1/1/2 | W1 |
| SSP, Modelica, SysML v2, XMI | models | XML / text | free | own; rumoca | S-M to M | see 6 | W2 |
| Excel ICD profile | models / avionics | spreadsheet | n/a | existing reader | S | 3/3/0/2/3 | W2 |
| COBOL copybook + EBCDIC | legacy | text + binary | IBM docs free | own (all crates AGPL/LGPL) | M | 2/1/0/1/3 | W2 |
| MATLAB .mat, .m | models | binary / HDF5 / text | v5 PDF free | own; HDF5 crate | S-M | 2/0-2/0-2/0-2/1-3 | W2 |
| man / mdoc | legacy | macro text | free | libmandoc-rs (young) or own | M | 2/2/1/0/2 | W2 |
| FITS | science | text cards + binary | free (IAU) | own (fitsrs, wcs) | S | 3/1/3/2/3 | W1 |
| NetCDF classic and 4, HDF5 | science | binary / HDF5 | free, BSD-3 C libs | netcdf-reader, hdf5-reader 0.9.2 (young) | S-M | 3/2-3/1-2/1-2/2 | W1 |
| StationXML, miniSEED | science | XML / binary | free | own; miniseed-rs | S-M | 3/2/2/3/3 | W2 |
| GRIB2 (inventory) | science | binary messages | WMO tables MIT | grib 0.20.0 | S-M | 3/1/3/2/3 | W2 |
| NASA CDF, Zarr | science | binary / JSON | free; CDF C lib custom | cdf 0.1.4; zarrs_metadata | M / S | 3/2/2/1/2 | W2 |
| Jupyter .ipynb | science | JSON | BSD-3 | nbformat 3.0.0 or own | S | 2/1/1/1/2 | W1 |
| BibTeX, RIS, EndNote XML, CSL-JSON | science | text / XML / JSON | de facto | biblatex, biblib, citationberg | S | 3/1-2/0/0/3 | W1 |
| LaTeX sources | science | macro text | no issue | own (badness-parser young) | M | 2/3/0/2/2 | W1 |
| SBML | science | XML + MathML | free | own | M | 3/3/1/3/3 | W1 |
| ELN (.eln, RO-Crate) | science | zip + JSON-LD | MIT | own | S-M | 3/3/2/0/2 | W1 |
| JCAMP-DX + Bruker NMR parameters | science | text | IUPAC free | own | M | 3/1/1/1/2 | W2 |
| CIF/mmCIF, GenBank, mzML, CellML | science | text / XML | free | own; gb-io, mzdata | S-M | see 7 | W2 |
| NI TDMS | machines | binary | NI documented | own (tedium as reference) | M | 3/2/3/2/3 | W1 |
| ANDI chromatography netCDF | machines | netCDF-3 | ASTM paid, self-describing | netcdf3 / own | S | 3/1/2/1/3 | W1 |
| RDML (qPCR) | machines | zip + XML | MIT | own | S | 3/2/2/1/3 | W1 |
| Allotrope ASM JSON | machines | JSON | schemas CC-BY-NC/ND | own | S | 3/2/1/2/3 | W1 |
| STDF V4 | machines | binary | open (Teradyne) | rust-stdf 1.1.0 | S | 3/1/2/0/3 | W2 |
| DMIS, Heidenhain Klartext | machines | text | free after form / manuals public | own | M / S-M | 3/2-3/0-1/1-3/3 | W2 |
| Tek, LeCroy, Keysight waveform headers | machines | binary | vendor-documented | own | S-M | 3/0/3/2/2 | W2 |
| OME-TIFF / ImageJ TIFF metadata | machines / science | TIFF + XML | open | tiff 0.11.3 | S | 3/0-2/1/0/1-2 | W2 |
| Thermo RAW | science | binary | vendor SDK, restrictive EULA | - | L | - | skip (detect) |
| Origin .opj/.opju | science | binary | closed, GPL-only prior art | - | blocked | - | skip (detect) |
| ARINC 665, ATA iSpec 2200, S5D, Lotus | various | binary / SGML | paid or undocumented | - | blocked / L | - | later or skip |

### Key findings

1. **Signals with conversion formulas are the common thread.** DBC factor and offset, A2L
   COMPU_METHODs, ARXML COMPU-SCALEs, MDF conversions, XTCE and SCOS-2000 calibrators, COMTRADE
   `a*x + b`, TDMS `NI_Scaling`, oscilloscope `YMULT/YOFF/YZERO`, FITS `BSCALE`/WCS, NetCDF
   `scale_factor`, GRIB packing, StationXML response stages, FMI unit factors, SunSpec scale
   factors and Excel ICD sheets all say "this raw value means that physical quantity in this
   unit". One shared signal and formula model (section 9) turns about twenty formats into the
   same kind of knowledge.
2. **OpenReadout claims to read about 130 lab-instrument formats in Rust for AI agents**
   (Apache-2.0 repo, MIT OR Apache-2.0 crates, MCP server, clean-room policy, created
   2026-10-02). Rezzer should not re-derive vendor lab binaries; it should decide how to relate to
   it (section 10).
3. **Most high-value formats are open text or XML with a permissive or no crate needed.** The
   hard parts are semantics (formulas, graphs, inheritance), multi-file inputs and young crates,
   not syntax.
4. **License surprises in both directions.** Blockers: railML (CC BY-NC-ND, registration and
   certification), ASAM terms forbid sharing schemas (even the free OpenX ones), the AUTOSAR
   "commercial exploitation requires a license" clause, the Thermo RawFileReader EULA, Origin
   (GPL-only prior art), every COBOL copybook crate (AGPL/LGPL), CSPICE bindings. Good news: the
   WMO GRIB and BUFR tables are MIT, libwpd and libwps offer MPL-2.0 (so WordPerfect is a build
   question, not a license one), the HDF5, NetCDF and CFITSIO C libraries are permissive, and
   pure-Rust readers now exist for NetCDF-4 and HDF5.
5. **Many relevant crates are weeks old** (2026 releases with a few hundred downloads). Each must
   sit behind Rezzer's own trait with a hostile-input probe, or Rezzer writes its own S reader.

## 2. Automotive

crates.io figures come from the API on 2026-10-19 ("upd" is crates.io `updated_at`, close to the
last publish). Figures for odx-rs and j1939 come from search output before crates.io started to
throttle. KBL and VEC are covered by the engineering report (first wave there).

### Licensing primary sources

**ASAM license terms** ([asam.net/license](https://www.asam.net/license/), dated 2017-10-26):
"Standards" include "Standard documents, schemata, models, interface-definition files, sample
code, source code and other files". Section 2(1)(c): other licensees "are not entitled to pass
ASAM products on and /or grant sublicenses"; 2(2)(b): "standards may be neither shared nor
sublicensed"; 2(3)(b): source may be shared only as a "marked file" ("This file is distributable
in accordance with the ASAM license terms"). Consequence: **no ASAM spec text or XSD (including
the free OpenX ones) may go into the Rezzer repo or corpus.** Writing a reader is not forbidden by
these terms.

**ASAM per-standard status** (asam.net detail pages, fetched 2026-10-19):

- Free after giving name and email ("This standard is free of charge. Please login as ASAM
  member or provide your details to download the standard"): OpenDRIVE 1.9.0 (2026-05-19),
  OpenSCENARIO XML 1.4.0 (2026-05-19), OpenSCENARIO DSL 2.2.0 (2026-03-19), OSI 3.8.0
  (2026-05-19), OpenLABEL 1.0.0 (2021-11-12).
- Members only, otherwise purchase ("Free of charge for members", non-members "JOIN ASAM or
  BUY", price not shown): MDF 4.3.0 (2025-09-23), MCD-2 MC / A2L 1.7.1 (2018-01-30), CDF 2.1.0
  (2015-07-16), MCD-2 D / ODX 2.2.0 (2008-05-18), MCD-2 NET / FIBEX 4.1.2 (2017-06-02), XIL 3.0.0
  (2024-09-30), OTX Extensions 3.3.0 (2025-06-10). The free "view online" links on these pages are
  only the release presentation, table of contents and known-issues list.

**AUTOSAR** (R24-11 `AUTOSAR_CP_TPS_SystemTemplate.pdf`, verbatim): "The commercial exploitation
of the material contained in this work requires a license to such intellectual property rights.
This work may be utilized or reproduced without any modification, in any form or by any means,
for informational purposes only. For any other purpose, no part of the work may be utilized or
reproduced ... without permission in writing from the publisher." Whether a non-partner may ship
an MPL-2.0 ARXML reader is **an open legal question**, not a yes.

**SAE J1939DA** is a paid spreadsheet; a reseller (SNV) sells single-user licenses with
"reproduction is not permitted" (weak, reseller). **ISO 17987 (LIN), ISO 22901 (ODX), ISO 13209
(OTX)** are paid ISO texts (for example ISO 17987-3:2025 about 228 EUR at a reseller).

### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| DBC | universal CAN | text | Vector proprietary, no formal spec | can-dbc 10.0.0 (5.45M dl) | opendbc MIT (1) | S | 3/2/0/2/3 | W1 |
| A2L / ASAP2 | universal measurement and calibration | keyword text | ASAM members or buy | a2lfile 3.5.1 (159k) | xcp-lite Apache-2.0 (1) | S-M | 3/2/0/3/3 | W1 |
| ARXML | all AUTOSAR projects | XML, multi-file | free to read, "commercial exploitation requires a license" | autosar-data 0.23.0 (55k) | cantools MIT (1) | M (L full) | 3/3/1/2/3 | W2; W1 after legal check |
| MDF / MF4 | universal logging | binary blocks | ASAM members or buy | falcon_mdf 0.7.1, mf4-rs 3.7.0 (young) | none (generate) | M | 3/1/3/3/2 | W2 |
| OpenDRIVE | AD simulation, HD maps | XML | free with registration, no redistribution | xodr 0.8.0 (new), opendrive 0.1.0 (stale) | esmini MPL-2.0 (1) | S-M | 3/3/0/2/2 | W2 |
| LDF | all LIN clusters | text | ISO 17987 paid | lin-ldf 0.0.28 | ldfparser MIT (candidate) | S | 3/2/1/2/3 | W2 |
| DCM | DE calibration | keyword text | ETAS note, no reproduction | autors-dcm 0.1.1 | none | S | 3/1/0/0/3 | W2 |
| ODX / PDX | OEM diagnostics | XML in zip | ASAM members or buy, ISO paid | odx-rs 0.39.0 (provenance unclear) | odxtools MIT (candidate) | M-L | 3/3/1/2/3 | W2 |
| OpenSCENARIO XML | AD simulation | XML | free with registration | none permissive | esmini MPL-2.0 (candidate) | S-M | 3/3/3/1/2 | W2 |
| A2L + HEX/S19 | calibration datasets | text records | public | ihex 3.0.0, srec 0.2.0, bin_file 0.2.1 | synthesize | M | 3/1/0/3/3 | W2 |
| HEX / S19 alone | flashing | text records | public | same | - | S | 1/0/0/0/1 | skip (address ranges only) |
| ASAM CDF (CDFX) | calibration data | XML | ASAM members or buy | autors-cdf 0.1.1 | none | S-M | 3/1/2/0/3 | later |
| FIBEX | legacy FlexRay, SOME/IP, DLT | XML | ASAM members or buy | afibex 0.11.1 (stale) | afibex test (candidate) | M | 3/3/1/2/3 | later |
| OTX | diagnostic sequences | XML | ISO paid | none | none | M | 2/3/0/1/1 | later |
| CDD (CANdela) | Vector diagnostics | XML | proprietary | none | none clean | M | 3/2/0/2/3 | later (suggest ODX) |
| OpenSCENARIO DSL | AD simulation | language | free with registration | none | none | L | 3/3/3/1/1 | later |
| OSI traces | sensor co-simulation | protobuf binary | spec free, protos MPL-2.0 | none (prost) | none | M | 2/1/3/0/2 | later |
| OpenLABEL | annotation | JSON | free with registration | none | none | S | 2/2/2/0/2 | later |
| ISO 26262 work products, GSN | safety | office, ReqIF, tool XML | various | none | none | - | - | covered; GSN later |
| SAE J1939DA | heavy duty, ag, marine | xlsx | SAE paid, no reproduction | none | blocked | S profile | 3/1/1/2/3 | skip (xlsx reader) |
| VDA 4938 | invoicing EDI | EDIFACT | VDA | - | - | - | - | skip |
| ASAM XIL | test benches | API, not a file | members or buy | - | - | - | - | skip |
| KBL / VEC | harness | XML | MIT | - | - | - | - | covered |

### DBC (Vector CAN database, .dbc)

Text, usually `VERSION "` then `NS_ :`, `BS_:`, `BU_:` with `BO_` and `SG_` lines, often cp1252.
`.dbc` collides with the Visual FoxPro database container (binary). The de facto CAN signal
exchange format: commaai/opendbc (MIT, 3,502 stars, pushed 2026-10-19), cantools and canmatrix
use it; Vector, PEAK and Kvaser tools read it [tool list unverified]. Holds nodes, messages (id,
DLC, sender), signals (bit position, byte order, sign, factor and offset, min, max, unit,
receivers), multiplexing, value tables (`VAL_`), comments (`CM_`), attributes (`BA_DEF_`/`BA_`,
for example `GenMsgCycleTime`), signal groups. No formal public spec. Crates: **can-dbc 10.0.0**
(MIT OR Apache-2.0, 2026-07-15, 5,451,579 downloads, Pest grammar, mature), dbc-rs 0.10.0,
can_tools 2.1.2 (MIT); Rust `cantools` 0.2.1 has a non-standard license (exclude). References:
cantools (Python, MIT), canmatrix (BSD-2-Clause). Probe for Pest recursion and huge files.
Effort S. **W1**: the most common automotive file, signals with units and scaling.

### LDF (LIN Description File, .ldf)

First token `LIN_description_file;`; collides with the MS SQL Server transaction log (binary).
Defined by ISO 17987-1 and SAE J2602-3, used for every LIN cluster in body electronics. Protocol
version, baud rate, master and slave nodes, frames and signals with init values, signal
encodings (physical value with scale, offset, unit), schedule tables with delays, diagnostic,
event-triggered and sporadic frames. ISO text paid; LIN 2.1 was published by the LIN consortium,
2.2A (2010) after it closed (lin-cia.org). Crates: lin-ldf 0.0.28 (MIT, 2025-07-08, 22k
downloads, pre-1.0), autors-ldf 0.1.1 (Apache-2.0). Reference: ldfparser (Python, MIT). Sample
candidates in ldfparser (MIT). Effort S. **W2.**

### FIBEX (ASAM MCD-2 NET)

XML root `fx:FIBEX`, namespace `http://www.asam.net/xml/fbx`. Historic FlexRay networks, still
used for SOME/IP and DLT non-verbose descriptions; ARXML mostly replaced it. Clusters, channels,
ECUs, frames, PDUs, signals, codings with COMPU-METHODs, SOME/IP services. Members only. Crate
afibex 0.11.1 (MIT OR Apache-2.0, 2024, stale). Effort M. **Later.**

### ARXML (AUTOSAR XML)

Root `AUTOSAR`, namespace `http://autosar.org/schema/r4.0` (3.x: `http://autosar.org/3.x.x`),
often with a UTF-8 BOM. All Classic and Adaptive AUTOSAR projects; OEMs exchange communication
matrices as system or ECU extracts; R24-11 is current. Holds packages with SHORT-NAME paths and
REF DEST cross references, ECU instances, CAN/LIN/FlexRay/Ethernet frames, PDUs, I-signals,
system signals, COMPU-METHODs (rational coefficients, scales, text tables), units, data types,
SW components, ports, interfaces, runnables, variants and admin data with revisions. Crates by
DanielT (also a2lfile): **autosar-data 0.23.0** (MIT OR Apache-2.0, 2026-08-15, 55k downloads,
multi-file, validating), autosar-data-abstraction 0.11.2, autosar-data-specification 0.23.0
(132k downloads; it embeds a metamodel generated from the AUTOSAR XSDs under MIT/Apache, flag
this in the legal check). References: cantools (MIT), canmatrix (BSD-2). Effort M for the
communication matrix and data types, L for the full SWC architecture. **W2, W1 once the AUTOSAR
legal question is settled.**

### A2L / ASAP2 (ASAM MCD-2 MC)

`.a2l` (A2ML `.aml`); first tokens `ASAP2_VERSION` or `/begin PROJECT`, often a BOM. Used by
every calibration tool (ETAS INCA, Vector CANape, ATI Vision); Vector's xcp-lite generates A2L
(Apache-2.0, pushed 2026-10-14). Holds MEASUREMENT and CHARACTERISTIC with addresses, types,
limits and units; COMPU_METHOD (`RAT_FUNC`, `LINEAR`, `FORM` formulas, `TAB_INTP`, `TAB_VERB`),
RECORD_LAYOUT, AXIS_PTS and AXIS_DESCR (curves and maps), the FUNCTION hierarchy
(DEF/REF/IN/OUT/LOC characteristics and measurements, a real graph), GROUP, TYPEDEF and INSTANCE
(1.7), MOD_COMMON byte order, IF_DATA transport, `/include`. Members only. Crate **a2lfile
3.5.1** (MIT OR Apache-2.0, 2026-09-28, 159k downloads, full 1.7.1, layout-preserving, mature);
probe deep nesting and `/include` loops. pyA2L is GPL-2.0 per its LICENSE and pyXCP LGPL-3.0
(license only). The Apache-2.0 sample has an unresolved `/include "XCP_104.aml"`, which the
reader must report. Effort S-M. **W1:** the richest semantics per byte here (formulas, function
graph, units).

### Intel HEX and Motorola S-record, alone and with A2L

Intel HEX lines start with `:`; S-records with `S0` to `S9`. `.hex` is generic, `.mot` weakly
overloaded. Alone they only give address ranges, start address and the S0 header: **skip**
(detect and report ranges). Paired with A2L they give the physical value of every
CHARACTERISTIC (address + RECORD_LAYOUT + COMPU_METHOD), so calibration values become tables and
maps: **W2**, effort M, because it needs multi-file input (section 9). Crates: ihex 3.0.0
(MIT OR Apache-2.0, 14.6M downloads, old and stable), srec 0.2.0 (MIT), bin_file 0.2.1 (MIT OR
Apache-2.0, 2026). srecord is GPL (license only). Probe overlapping records and huge gaps.

### ASAM CDF (Calibration Data Format, .cdf / .cdfx)

XML with root `MSRSW` and category CDF20 [unverified]. **Name collision** with NASA CDF (binary
magic `CD F3 00 01` or `CD F2 60 02`, section 7) and Microsoft Channel Definition Format.
Parameter values (VALUE, CURVE, MAP, VAL_BLK, AXIS_PTS) with maturity and per-parameter history
(SW-CS-HISTORY), linked to A2L names: a timeline. Members only. Crate autors-cdf 0.1.1
(Apache-2.0, 129 downloads; crates `cdf` and `cdflib` are the NASA format). No licensed sample.
Effort S-M. **Later.**

### DCM (DAMOS calibration text, .dcm)

`KONSERVIERUNG_FORMAT 2.0` or blocks `FESTWERT`, `KENNLINIE`, `KENNFELD`, `FESTWERTEBLOCK`,
`STUETZSTELLENVERTEILUNG`. **Collides with DICOM** `.dcm` (`DICM` at offset 128). German
calibration exchange (INCA, CANape). Values, curves and maps with axes, units, LANGNAME and
function membership. Documented in the ETAS TechNote "DCM File Formats" (2012, copyright ETAS and
Bosch, no reproduction; only a third-party copy found [unverified]). Crate autors-dcm 0.1.1. No
licensed sample. Effort S. **W2**: small and high value next to A2L.

### MDF / MF4 (ASAM MDF 3.x and 4.x)

Bytes 0-7 `MDF     ` (finalized) or `UnFinMF ` (unfinalized), version string at offset 8
(confirmed in mdflib, MIT). **Collides** with MS SQL Server `.mdf` and Alcohol 120% images.
Written by practically every logger and tool (CANape, INCA, CSS Electronics, dSPACE); 21
companies are listed as authors of 4.3.0. Holds header XML metadata, file history (FH), channel
groups and channels with names, units, sources and conversions (linear, rational, algebraic
formula, tables, value to text), events and triggers, attachments (often the DBC or A2L
itself), bus logging and the samples. Members only. Rust crates are young: **falcon_mdf 0.7.1**
(MIT OR Apache-2.0, 2026-09-28, 875 downloads, MDF4 plus MDF3, claims a 1,200-file mutation
sweep checked against asammdf: the best hostile-input story), mf4-rs 3.7.0 (MIT, 36 versions
since 2025-11, subset), mdf4-rs 0.6.0, mdflib 0.2.1 (binds the MIT C++ mdflib). asammdf is
LGPL-3.0 (license only). No small licensed MF4 found; generate one. Effort M (metadata,
conversions as formulas, events, attachments, per-channel statistics; no sample dump). **W2.**

### ODX / PDX (ASAM MCD-2 D, ISO 22901-1)

`.odx`, `.odx-d/-c/-cs/-v/-f/-e/-m`; `.pdx` is a zip with `index.xml`. XML root `ODX`.
**`.pdx` collides** with IPC-2571 PDX (engineering report) and the Acrobat catalog. OEM
diagnostic data for testers and flashing (Vector, Softing, ETAS; Mercedes-Benz odxtools, MIT,
pushed 2026-10-06). Diagnostic layers with inheritance, services, requests and responses, DOPs
with COMPU-METHODs and units, DTCs with texts, communication parameters, flash data, revisions.
Members only and ISO paid. **odx-rs 0.39.0** (MIT, created 2026-08-01, 44 versions in about a
week, repository URL returns 404): do not adopt without review. Reference odxtools (MIT); its
`somersault.pdx` (MIT, 30 KB) is the best sample candidate. Effort M-L (inheritance). **W2.**

### OTX (ISO 13209), CDD (Vector CANdela)

OTX: XML diagnostic test sequences (procedures, branches, loops, diagnostic calls), ISO paid,
no crate, effort M, **later**. CDD: XML with `<!DOCTYPE CANDELA SYSTEM 'candela.dtd'>`,
iso-8859-1, Vector proprietary with no spec; services, DIDs, DTCs, multi-language texts. The
only public sample (cantools) was "Copied from forums.ni.com", so not used. **Later**; detect and
suggest an ODX export.

### SAE J1939 Digital Annex

A paid xlsx with PGNs and SPNs (resolution, offset, range, unit, SLOTs, FMI). Crates `j1939`
0.3.0 and `sae-j1939-rs` are protocol stacks. **Skip** as a dedicated reader: the spreadsheet
reader covers a user's own licensed copy; optionally recognize the header row.

### ISO 26262 work products, GSN, VDA 4938, ASAM XIL

ISO 26262 work products are ordinary documents and models (Word, Excel, ReqIF, SysML): covered.
GSN (SCSC Community Standard v3) has no common file format; tools use their own XML and OMG SACM
gives an XMI metamodel [unverified]. A GSN graph would be high value but there is nothing common
to read: **later**. **VDA 4938 is not an engineering format**: it is Global INVOIC, the VDA
guideline for UN/EDIFACT INVOIC D07A (Part 2 V3.1 of 2025-09-29): **skip**. **ASAM XIL** is an
API standard (C#, UML), only mapping files are XML: **skip**.

### OpenDRIVE (.xodr, .xodrz)

XML root `OpenDRIVE` with `header revMajor/revMinor`. Used by CARLA, esmini (MPL-2.0, pushed
2026-10-19), VTD, IPG and dSPACE; ASAM 1.9.0 of 2026-05. Roads with plan-view geometry (line,
arc, spiral, paramPoly3) and cubic polynomials (`a + b*ds + c*ds^2 + d*ds^3` for elevation and
lane width), lanes and links, junctions and connections (topology graph), signals, objects, geo
reference. Free with registration; the XSD is still an ASAM "standard" (no redistribution).
Crates: xodr 0.8.0 (MIT OR Apache-2.0, 2026-10-18, 78 downloads, very new; lane graph, routing),
opendrive 0.1.0+1.7.0 (MIT, 2023, stale). esmini RoadManager is MPL-2.0. asam-ev/ASAM-OpenX-Examples
has **no license**: not used. Effort S-M (topology and properties, no geometry evaluation).
**W2** (W1 if automated driving is a focus).

### OpenSCENARIO XML (.xosc) and DSL (.osc), OSI, OpenLABEL

OpenSCENARIO XML: root `OpenSCENARIO` with `FileHeader`; entities, catalogs, parameter
declarations and distributions, storyboard (Story, Act, ManeuverGroup, Maneuver, Event, Action),
triggers and conditions as timeline and graph, references to `.xodr`. Free with registration.
**openscenario-rs 0.6.0 is GPL-3.0-only: excluded.** Own quick-xml reader, S-M, **W2**. The DSL
is a Python-like language (L, **later**). OSI traces are length-prefixed protobuf; the `.proto`
files are MPL-2.0 (BMW), separate from the ASAM spec terms; M, **later**. OpenLABEL is JSON
(serde_json is enough), S, **later**.

### Area ranking

1. A2L: formulas, function graph, units; mature crate; clean sample.
2. DBC: most common automotive file; mature crate.
3. ARXML: highest knowledge (full E/E architecture graph), strong crate, but legal check first.
4. MDF/MF4 metadata: universal; conversions, events, attachments; young crates need a probe.
5. OpenDRIVE: free spec, simple XML, road and junction graph.
6. LDF, 7. DCM, 8. ODX/PDX, 9. OpenSCENARIO XML, 10. A2L + HEX/S19 (multi-file).
Then CDF, FIBEX, OTX, OSI, OpenLABEL, CDD.

### Open questions / unverified

- Multi-file input: A2L + HEX, A2L `/include`, multi-file ARXML, PDX, `.xosc` referencing
  `.xodr` (section 9).
- AUTOSAR legal position; autosar-data-specification embedding XSD-derived data.
- Young crates (falcon_mdf, xodr, the `autors-*` family created 2026-08 with two versions each,
  odx-rs) vs own readers.
- Detection collisions: `.mdf`, `.ldf`, `.dcm`, `.cdf`, `.pdx`, `.hex`, `.mot`, `.osc`; BOM before
  A2L and ARXML content.

## 3. Automation (legacy delta), energy, rail, marine, medical, geo

The engineering report (`docs/research/2026-10-19-engineering-formats.md` on branch
`docs/engineering-formats`) already covers IEC 61850 SCL, GSDML, ODVA EDS, ESI, IODD, L5X,
SimaticML, TwinCAT, CODESYS, PLCopen XML, OPC UA NodeSet2 and MTConnect device XML. This section
adds only the delta. GitHub search hit its rate limit late in the run, so the vendor PLC formats
(S5D, L5K, RSLogix 500) rest on web sources only and are marked [unverified].

### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| STEP 5 S5D | legacy S5 plants | binary | undocumented | none | none | L / blocked | 1/1/0/1/1 | skip (detect) |
| STEP 7 AWL/SCL sources + symbol tables | S7-300/400 classic | text | manuals free to read | none | none | S-M | 2/1/1/1/3 | W2 |
| STEP 7 .s7p archive | S7 classic | folder/zip, DBF tables? | undocumented | dbase? | none | M-L | 2/2/1/0/2 | later |
| Rockwell L5K | Studio 5000 | text | manual 1756-RM084 | none | none | M | 2/2/1/1/2 | later |
| RSLogix 500 .RSS | legacy SLC 500 | binary (+ CSV exports) | undocumented | none | none | blocked / S | 1/0/0/0/2 | skip; CSV covered |
| SunSpec Modbus models | DER inverters, meters, batteries | JSON (SMDX XML) | Apache-2.0 | none (protocol only) | repo | S | 3/1/0/2/3 | W2 |
| Vendor Modbus maps, HMI tags and alarms | everyone | xlsx/CSV/XML | vendor layouts | existing readers | none | S-M per profile | 2/1/0/2/3 | later |
| ISA-88 BatchML recipes (B2MML) | MES, batch plants | XML | MESA permissive | none | repo | M | 2/3/1/2/2 | later |
| PROFIBUS GSD | process plants (DP/PA) | INI-like text | PI spec; files public | gsd-parser 0.6.0 | vendor only | S | 3/0/0/1/3 | later |
| CGMES / CIM RDF/XML | EU TSOs (mandatory) | RDF/XML, zipped profiles | IEC paid; ENTSO-E profiles Apache-2.0 | cimoxide 0.3.3, cim-rs 0.3.0, powerio 0.11.5 | 1 | M | 3/3/2/1/2 | W2 |
| COMTRADE (C37.111 / IEC 60255-24) | every fault recorder and relay | text cfg + CSV/binary dat | paid, widely implemented | comtrade 0.2.1 (stale), comtrade-io 0.1.0 | 2 | S | 3/1/3/3/3 | W1 |
| PSS/E RAW (+ RAWX) | transmission planning | sectioned text | manual with product | powerio 0.11.5 | none | S-M | 3/3/0/1/3 | W2 |
| MATPOWER case .m | power systems research | MATLAB literal | code BSD-3, cases not | powerio | none | S | 3/3/0/2/3 | W2 |
| GE PSLF .epc | WECC planning | text | vendor | powerio | none | S | 3/3/0/1/3 | later |
| IEC 62351, IEEE C37.118 | - | not file formats | - | - | - | - | - | skip |
| railML 2.x / 3.x | EU rail | XML | CC BY-NC-ND, registration, certification | none | none | M | 2/3/1/0/2 | blocked |
| NMEA 0183 + AIS | GNSS, marine | text lines | paid copyrighted spec | nmea 0.8.0, nmea-parser 0.11.0, ais 0.12.0 | 1 | S | 2/0/3/0/3 | W2 |
| NMEA 2000 logs | marine | candump/YDWG text, binary | paid; CANboat DB Apache-2.0 | canboat 9.0.0, candump-parse 0.1.0 | none | M | 2/1/3/2/3 | later |
| IHO S-57 / S-101 | ECDIS (SOLAS) | ISO 8211 binary | IHO free; S-63 encrypted | iso8211 0.1.5 | none | M | 2/1/2/0/2 | later |
| IHO S-102/104/111 | hydrography | HDF5 | IHO free | hdf5-metno (links C) | none | M | 2/0/2/0/2 | later |
| DICOM headers | medical imaging | binary tagged | free standard | dicom-object 0.10.0 | none | S | 3/2/1/0/3 | W2 |
| HL7 v2 (ER7) | hospitals | pipe text | free license after registration | hl7-parser 0.3.0 | synthetic only | S | 3/1/2/0/3 | W2 |
| HL7 FHIR | health IT | JSON/XML | CC0 [unverified] | fhir-model, helios-fhir | none | S | 3/2/1/0/2 | later |
| HL7 CDA | health IT | XML | HL7 license | none | none | M | 2/2/1/0/2 | later |
| GPX | outdoor, fleet | XML | free | gpx 0.10.0 | none | S | 2/0/3/0/2 | W2 |
| KML/KMZ | GIS | XML / zip | OGC free | kml 0.14.0 | none | S | 2/1/1/0/2 | W2 |
| GeoJSON | GIS | JSON | RFC 7946 | geojson 1.0.0 | none | S | 2/0/0/0/2 | W2 / generic JSON |
| Shapefile (.dbf/.prj metadata) | GIS | binary + dBase + WKT | Esri whitepaper free | shapefile 0.9.0, dbase 0.8.0 | none | S | 2/0/0/0/3 | W2 |

### Automation delta

**Siemens STEP 5 (.S5D, `*ST.S5D`, `Z0.SEQ` symbols, `XR.INI`).** Binary, no public magic
[unverified]; the `*ST.S5D` name pattern is the best hint. Discontinued long ago but still
running in plants; migration goes through the Siemens "S5 to S7 converter" (STEP 7 Classic 5.5)
per third-party blogs (weak evidence). Holds blocks PB/FB/SB/DB, STL code, comments, symbols.
Undocumented; only commercial tools read it (IBH S5 for Windows [unverified]); no open reader.
Effort L (clean-room reverse engineering) or blocked. **Skip:** detect and say "export as ASCII
STL source or convert to STEP 7 AWL".

**STEP 7 classic sources: AWL/STL (.awl), SCL (.scl), symbol tables (.seq/.asc/.sdf/.dif), .s7p.**
Text (cp1252) starting with `FUNCTION_BLOCK`, `FUNCTION`, `ORGANIZATION_BLOCK` or `DATA_BLOCK`
plus `TITLE =`, `VERSION :`. Name collisions: Siemens SCL vs IEC 61850 SCL (different file
extension, same name), `.sdf` vs SQL Server Compact and the chemistry SD file; detect by
keywords. Symbol export `.seq` has lines like `126,<symbol> <address> <type> <comment>`, `.asc` is
fixed width, `.sdf` quoted CSV [layouts unverified]. Holds block interfaces (VAR_INPUT/OUTPUT/STAT
with types and initial values), networks with comments, block attributes (AUTHOR, FAMILY,
VERSION) and the symbol table as the I/O signal list. The language is documented in free Siemens
manuals. No Rust file crate (`s7` 0.1.9, license "non-standard", and `tokio-s7` 0.1.7 are
protocol clients). awlsim is GPL-2.0+ (license only). No licensed samples found. Effort S for
symbol tables, M for block headers and interfaces (no code semantics). The `.s7p` project may hold
dBase tables [unverified]; then `dbase` 0.8.0 (MIT) applies. **W2** for symbol tables and
AWL/SCL headers, `.s7p` later.

**Rockwell L5K.** ASCII project export next to L5X, CSV and TXT tag exports, listed in the
Studio 5000 v38 help
([Rockwell](https://www.rockwellautomation.com/en-ie/docs/studio-5000-logix-designer/38-00/contents-ditamap/studio-5000-logix-designer/import-and-export.html)).
Starts with `IE_VER := x.x;` and `CONTROLLER <name> (...)` [unverified]. Same knowledge as L5X
in a nested block grammar, documented in the Logix5000 Import/Export Reference Manual 1756-RM084
(current terms unverified). No Rust crate (`l5x` 0.6.0 is L5X only), no samples. Effort M.
**Later**, same priority as L5X; until then detect and say "export as L5X".

**RSLogix 500 (.RSS/.RSP, PLC-5 .PC5).** Binary (OLE compound? [unverified]); CSV/TXT
documentation exports are address/symbol/description tables that the generic CSV reader already
handles. **Skip** the binary, CSV covered.

**Modbus register maps.** No standard file format: vendors ship PDF or Excel tables (address,
function code, type, scale, unit, R/W). The one open machine-readable schema is the SunSpec
Alliance model set (inverters, meters, batteries): JSON (formerly SMDX XML) with point id,
offset, type, units, scale-factor reference and access, in
[sunspec/models](https://github.com/sunspec/models) (Apache-2.0, pushed 2026-08-20). Rust
crates are protocol only (`tokio-modbus` 0.17.0 MIT/Apache, `modbus` 1.1.3 MIT,
`modbus-mapping` 0.4.0 MIT). Scale factors are formulas (`value * 10^sf`). **W2** for SunSpec
JSON (S); vendor Excel maps later as a column profile on the spreadsheet reader.

**HMI exports (WinCC, FactoryTalk), alarm lists.** WinCC (TIA) exports tags and alarms as xlsx,
WinCC flexible and V7 as CSV/TXT, FactoryTalk View tags as CSV and alarms as XML [layouts
unverified]. They hold the signal meaning layer agents need: tag, PLC address, type, linear
scaling (formula), alarm text, class, trigger bit, limits. ISA-18.2 / IEC 62682 define no file
format [unverified]. **Later**, as column profiles once real exports exist.

**Recipes: ISA-88 BatchML (part of B2MML).** XML, XSDs on
[MESAInternational/B2MML-BatchML](https://github.com/MESAInternational/B2MML-BatchML) under a
permissive notice: "Permission to use, copy, modify, or redistribute this Work ... for any
purpose and without fee or royalty is hereby granted provided MESA International is acknowledged
as the originator". Master recipe hierarchy (procedure, unit procedure, operation, phase) as a
graph, formula parameters, equipment requirements. No Rust crate. Effort M. **Later** (belongs
with ISA-95 / MES).

**IEC 61850 SCL, delta.** LF Energy CoMPAS distributes the XSD editions
([com-pas/compas-scl-xsd](https://github.com/com-pas/compas-scl-xsd)): `SCL2003` (Ed1),
`SCL2007B` (Ed2), `SCL2007B4` (Ed2.1); the edition comes from the `version`, `revision` and
`release` attributes on the root [mapping unverified]. XSDs and NSD files (IEC 61850-7-7) are IEC
code components under the IEC "CC-EULA", and the CoMPAS notice says "Unauthorized modifications
are strictly prohibited". Rezzer must not bundle them; reading files without them is fine.

**ODVA EDS.** See the engineering report (paid). New here: the name collides with CANopen EDS
(CiA 306, INI-like) and CCSDS SEDS (section 5).

**PROFIBUS GSD (.gsd, .gse/.gsg/.gsf/.gsi/.gsp/.gss).** INI-like Latin-1 text with a
`#Profibus_DP` line, then `GSD_Revision`, `Vendor_Name`, `Ident_Number`, `Module = ... EndModule`,
`ExtUserPrmData`. Collision: `.gsd` is also the HOOMD-blue trajectory format (binary). Device
identity, modules, user parameters with ranges and enums. Crate `gsd-parser` 0.6.0 (MIT OR
Apache-2.0, 2025-02-23, 6,619 downloads, from rahix/profirust); fuzz before use. Vendor files
have unclear redistribution rights. Effort S. **Later** (W2 if a process user asks).

### Energy

**CIM / CGMES (IEC 61970-552 CIM/XML, ENTSO-E CGMES 2.4.15 and 3.0).** RDF/XML, root `rdf:RDF`
with namespace `http://iec.ch/TC57/CIM100#` (3.0) or `.../2013/CIM-schema-cim16#` (2.4.15), plus
`md:FullModel` with `Model.profile`; often one zipped file per profile (EQ, TP, SSH, SV, DL, GL,
DY, SC). Mandatory for EU TSO grid model exchange (ENTSO-E conformity scheme v3.0.2, approved
2024-08-25,
[ENTSO-E](https://www.entsoe.eu/data/cim/cim-conformity-and-interoperability/)). Holds the
topology graph (ConnectivityNode, Terminal, equipment), equipment parameters (impedances,
ratings, limits), containment (Substation > VoltageLevel > Bay) and a model header (created,
scenarioTime, version, `DependentOn`) as revisions and dependencies. A file is only fully
resolved with its `DependentOn` files, so "unresolved reference" output is essential. IEC 61970
is paid; the ENTSO-E profiles (RDFS/SHACL) are Apache-2.0
([application-profiles-library](https://github.com/entsoe/application-profiles-library)); the
ReliCapGrid test models are CC BY-SA 4.0 with "Any use of the test configurations (models) ...
shall include a reference to ENTSO-E". Rust crates are all months old: `cimoxide-decoder` and
`cimoxide-structs` 0.3.3 (Apache-2.0, 2026-09-18, about 250 downloads; the same author ships
`cimoxide-mcp` 0.6.0, an MCP server for CGMES that overlaps with Rezzer), `cim-rs` 0.3.0 (MIT OR
Apache-2.0, 51 downloads), `powerio` 0.11.5 (MIT OR Apache-2.0, 2026-10-11, 4,310 downloads).
References: pycgmes (Apache-2.0), cim4j (Apache-2.0), cimpyorm (BSD-3), CIMDraw (GPL-3.0,
license only). Effort M for an own quick-xml reader of the EQ/TP graph. **W2.**

**COMTRADE (IEEE C37.111 = IEC 60255-24:2013; .cfg + .dat, optional .hdr/.inf, 2013 single-file
.cff).** The `.cfg` is text: line 1 `station,device,rev_year` (1991/1999/2013), line 2
`total,nA,nD` (for example `8,4A,4D`), data type `ASCII|BINARY|BINARY32|FLOAT32` near the end.
`.cfg` and `.dat` are generic extensions: claim a `.dat` only when a `.cfg` with the same stem
is present. Written by every digital fault recorder and protection relay (SEL, SIPROTEC, ABB).
Holds the channel list (name, phase, circuit component, unit, `a*x + b` conversion,
primary/secondary ratio), digital channels with normal state, nominal frequency, sample rates,
trigger and first-sample time, local time offset, plus the sample series. The standard is paid
but widely implemented. Crates: `comtrade` 0.2.1 (MIT, 2021, stale), `comtrade-io` 0.1.0 family
(MIT, 2026-09, under 110 downloads, sub-crates without repo link). Reference python-comtrade
(MIT). Two MIT samples downloaded. Effort S for an own reader. **W1:** small, very common, and
exactly "signals and their meaning" (channels, units, conversions, trigger time).

**PSS/E RAW (.raw, RAWX JSON for v35, .seq, .dyr).** Text; first line `0, 100.00, 33, ...` (IC,
SBASE, REV), sections end with `0 / END OF BUS DATA, BEGIN LOAD DATA`; layout changes by
revision 32 to 35. `.raw` collides with camera raw, audio and dumps. Siemens PSS/E is the
dominant transmission planning tool in North America (weak evidence). Buses, loads, generators,
branches, transformers with impedances and ratings, areas, owners: a bus/branch graph.
`powerio` 0.11.5 (MIT OR Apache-2.0, created 2026-06, active) reads RAW 32-35, RAWX, GE PSLF
`.epc`, MATPOWER, UCTE-DEF, PowerWorld, OpenDSS, IEEE CDF and pandapower JSON. `surge-io` 0.1.9
is **PolyForm-Noncommercial: cannot link**. pandapower is BSD-3. Effort S-M with powerio behind
a trait. **W2.**

**MATPOWER case (.m).** Text with `function mpc = caseN`, `mpc.version = '2';`,
`mpc.bus = [ ... ];`. `.m` collides with MATLAB scripts, Objective-C and Mathematica. Research
standard for power flow and OPF (MATPOWER, PGLib-OPF). Fixed-column bus/gen/branch/gencost
matrices, cost polynomials (formulas). MATPOWER code is BSD-3, but "The MATPOWER case files
distributed with MATPOWER are not covered by the BSD license". It is MATLAB code: parse only the
matrix literal subset, never execute. **W2** with PSS/E via powerio. GE PSLF comes free with
powerio (**later**).

**IEC 62351** is the security series for power system protocols (TLS profiles, roles, keys), not
a file format; certificates belong to generic X.509 handling. **IEEE C37.118** is a streaming
protocol; archives are CSV, COMTRADE or vendor formats. **Skip** both.

### Rail

**railML 2.x / 3.x.** XML root `railML` (namespaces for 2.x and 3.x [years unverified]). Used by
the railML.org consortium of European infrastructure managers, railway undertakings and vendors.
Holds infrastructure topology (netElements/netRelations in 3.x, based on UIC RailTopoModel),
signals, speed profiles, timetables and rolling stock: a strong graph. **License blocker** from
[railml.org/en/licence](https://www.railml.org/en/licence) (fetched 2026-10-19): schemas are
"restricted Creative Commons Attribution-NonCommercial-NoDerivs" (CC BY-NC-ND 3.0 for 2.2 to
2.5, 4.0 for 3.x); registration is required before implementation work, "import, export, or
transformation programmes ... also for test- or research purposes"; certification is "mandatory
before any productive or commercial use of the interface"; schemas "must only be distributed
through" railML.org; example files are CC BY-NC-ND 4.0. No Rust crate, no sample downloaded.
**Blocked** until Jordi decides on registration. RailTopoModel (UIC IRS 30100) is a conceptual
model serialized through railML 3, so it is covered by the same decision. ERA RINF (EU register
of infrastructure, XML and RDF) is **later** [unverified].

### Marine

**NMEA 0183 (IEC 61162-1) and AIS logs.** Lines `$ttsss,...*hh` with XOR checksum; AIS
`!AIVDM`/`!AIVDO`; optional tag blocks `\s:...,c:...*hh\`. Written by every GNSS receiver,
marine instrument and AIS transceiver. Time series of position, satellites, course, speed,
depth, wind, with talker identity; AIS adds vessels (MMSI, name, type, size) and tracks. The
spec is paid and NMEA says "the 0183 Interface Standard is copyrighted and available only from
NMEA" ([nmea.org](https://www.nmea.org/nmea-0183.html)); readers come from public descriptions
and crates. Crates are mature: `nmea` 0.8.0 (Apache-2.0, 2026-08-08, 204k downloads, AeroRust),
`nmea-parser` 0.11.0 (Apache-2.0, 551k, with AIS), `nmea0183` 0.6.0 (BSD-3), `ais` 0.12.0
(Apache-2.0). One Apache-2.0 sample downloaded. Effort S: time span, bounding box, sentence
counts, talkers, decimated track table. **W2.**

**NMEA 2000 (IEC 61162-3) logs.** candump `(1697712000.123456) can0 09F80103#...`, Yacht Devices
RAW `hh:mm:ss.mmm R 09F80103 xx ...`, Actisense binary [partly unverified]. candump `.log` is also
used for automotive CAN (J1939): tell them apart by PGN ranges and share one candump reader with
section 2. The open PGN database is CANboat (reverse-engineered, Apache-2.0); crates `canboat`
9.0.0 (Apache-2.0, publisher unverified), `korri-n2k` 0.7.1, `nmea2000` 0.2.2, `candump-parse`
0.1.0. Effort M. **Later.**

**IHO S-57 ENC (.000 base cell, .001+ updates, CATALOG.031) and S-101.** ISO 8211 binary.
Official ENCs are carried under IMO SOLAS ECDIS rules; commercial cells are S-63 encrypted
(detect and say so). Metadata only: DSID (edition, update number, issue date, agency, scale),
object class counts, coverage bounding box, update chain. IHO S-57 3.1 PDF is a free download.
Crate `iso8211` 0.1.5 (MIT OR Apache-2.0, 5,261 downloads, immature). GDAL S57 driver is MIT.
Effort M. **Later.** S-102, S-104 and S-111 use HDF5 and depend on the HDF5 decision (section 7).

### Medical

**DICOM Part 10 headers (.dcm or no extension, DICOMDIR).** 128-byte preamble then `DICM` at
offset 128. Universal in medical imaging; the standard is free at dicom.nema.org. Holds the
patient/study/series/instance hierarchy, modality, device, acquisition parameters, dates and
private tags, and personal health data. Crates `dicom-object`, `dicom-core`, `dicom-parser`,
`dicom-dictionary-std` 0.10.0 (MIT OR Apache-2.0, 2026-06-27, about 2.0M downloads each,
Enet4/dicom-rs). Reading can stop before PixelData [API name unverified]. Samples must be
synthetic or MIT (pydicom test data, not downloaded). Effort S. **W2** (W1 if a medical user
exists). PII rule needed (section 9).

**HL7 v2 (ER7, .hl7).** First segment `MSH|^~\&|`, CR separated, MLLP framing on the wire. Still
the dominant hospital interface format (ADT, ORM, ORU; weak evidence). Message type, trigger
event, patient (PII), orders, results (OBX value, units, reference range, abnormal flag),
timestamps. Free license since 2013 under the HL7 IP policy, registration needed to download.
Crates: `hl7-parser` 0.3.0 (Apache-2.0, 2025-02-24), `rust-hl7` 0.5.0 (MIT/Apache, 2021),
`hl7` 0.2.3 / `hl7-2` 0.3.1 (multi-license including MIT; pick MIT and tell cargo-deny),
**`hl7v2` 1.5.0 is AGPL-3.0: cannot link**. Samples must be synthetic. Effort S. **W2.**

**HL7 FHIR** (JSON with `"resourceType"` or XML in `http://hl7.org/fhir`; spec CC0
[unverified]): the generic JSON reader gives most of it; a Bundle reference graph is S.
**Later.** **HL7 CDA** (root `ClinicalDocument` in `urn:hl7-org:v3`): header plus narrative
sections; M, **later**.

### Geo

**GPX** (root `gpx`, namespace `http://www.topografix.com/GPX/1/1`; `gpx` 0.10.0 MIT, 323k
downloads, last release 2023): waypoints, routes, tracks as time series with elevation and
extensions. **KML/KMZ** (OGC, root `kml`; `kml` 0.14.0 MIT OR Apache-2.0, 444k): placemark
folders as a tree, ExtendedData properties. **GeoJSON** (RFC 7946; `geojson` 1.0.0, 11.8M):
property table and bounding box. **Shapefile** (`.shp` magic `00 00 27 0A`; `shapefile` 0.9.0
MIT, `dbase` 0.8.0 MIT): attribute schema and table, CRS from `.prj` WKT, encoding from `.cpg`.
All S, all **W2**; the `dbase` reader also serves legacy DBF files elsewhere.

### Area ranking

1. COMTRADE (S): every relay produces it; channel table, units, conversions, trigger time.
2. DICOM headers (S): mature MIT/Apache crate, clear hierarchy; needs a PII rule.
3. HL7 v2 (S): simple grammar, permissive crate, high volume; PII.
4. NMEA 0183 and AIS (S): mature crates, time series summary.
5. CGMES (M): richest graph here, open profiles, CC BY-SA test models; young crates.
6. PSS/E RAW, MATPOWER, PSLF via powerio (S-M): one young permissive crate, many grid formats.
7. GPX, KML, Shapefile/DBF, GeoJSON (S each).
8. SunSpec models, STEP 7 symbol tables and block headers, PROFIBUS GSD (S each).
9. Later: NMEA 2000, S-57/S-101, FHIR, CDA, L5K, BatchML, HMI exports.
10. Blocked or skip: railML (license), S5D and RSLogix 500 binaries (undocumented), IEC 62351
    and C37.118 (not files).

### Open questions / unverified

- railML: should Jordi register with railML.org to allow a reader, or does Rezzer only detect it?
- STEP 7 `.s7p` internals, S5D layout, RSLogix 500 binary: unverified; a real export is needed.
- powerio and cimoxide are months old (cimoxide-mcp is itself an MCP server): wrap or write own?
- Is the `canboat` crate published by the CANboat maintainer?
- MATPOWER case files are not BSD: no corpus copy without a per-case license.

## 4. Avionics and aerospace

Nothing here overlaps the engineering report. crates.io was rate-limited (HTTP 429) for part of
the run; the sparse index confirmed that no crate named `xtce`, `pds4`, `pds3`, `s1000d`, `aixm`,
`arinc653` or `arinc665` exists on crates.io.

### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| ARINC 424 (FAA CIFP) | FMS data suppliers, simulators | fixed 132-column text | spec paid (SAE ITC); CIFP public | arinc424 0.4.0, arinc424-rs 0.9.1 | none (CIFP zip > 2 MB) | M | 3/2/1/0/3 | W2 |
| S1000D data modules | defense and civil tech pubs | XML (old issues SGML) | spec free; sample terms unclear | none | none | M | 3/3/2/0/2 | W2 |
| AIXM 5.1/5.1.1 | EUROCONTROL, FAA, ICAO SWIM | XML/GML | schemas BSD | traffic-thrust 0.2.2 (partial) | none | M-L | 3/2/3/0/2 | W2 / later |
| ARINC 429 label lists, ICD sheets | avionics integrators | xlsx | spec paid, ICDs private | arinc429 0.1.0 (decode only) | none | S profile | 3/2/1/2/3 | covered + profile (section 6) |
| DO-178C artifacts | certified airborne software | office, PDF, ReqIF | RTCA paid | n/a | none | covered (+ trace graph M) | 2/3/2/0/3 | covered |
| ARINC 653 configuration | IMA platforms | XML (vendor variants) | paid, vendor schemas | a653rs (runtime only) | none | S-M | 2/3/2/0/2 | later |
| ARINC 665 LUH / LUM | all field-loadable avionics software | binary big-endian | paid | none | none | blocked / L | 3/2/1/0/2 | later (detect) |
| ATA iSpec 2200, Spec 100, Spec 2000 | airlines, MRO | SGML/XML | paid (A4A) | none | none | blocked / M | 2/2/2/0/2 | later |
| FACE data model | US DoD avionics | XMI | free with account | none | none | M | 2/3/1/1/2 | later |
| DO-200B | data suppliers | process, no format | paid | - | - | - | - | skip |

### ARINC 424 navigation data

No fixed extension; FAA CIFP ships as `FAACIFP18` in a zip, others use `.dat` or `.pc`. Fixed
132-column ASCII: column 1 record type (S standard, T tailored), then area and section codes;
CIFP starts with `HDR01` headers. Detect by line length and a known section code at the fixed
column. Every FMS database supplier (Jeppesen, Lufthansa Systems, NAVBLUE) and the FAA CIFP,
which the FAA describes as "updated every 28 days", in "ARINC 424 version 18", "raw ARINC data.
It will require additional processing before it can be loaded into an avionics system"
(faa.gov CIFP page). Airports, runways, navaids, waypoints, airways, SID/STAR/approach
procedures as leg sequences with path terminators (a graph), airspace, frequencies, magnetic
variation and the AIRAC cycle. The spec is paid through SAE ITC; CIFP is a US government work
[public-domain statement unverified]. Crates (all young, probe needed): `arinc424` 0.4.0
(Apache-2.0, 2026-02-23, libefb), `arinc424-rs` 0.9.1 (MIT, 2026-07-15), `arinc_424` 0.1.0 (MIT,
2024). References PyARINC424 and perryrob/arinc_424 (MIT). Effort M with a crate behind a trait.
**W2.**

### S1000D data modules

`.xml` with names `DMC-<code>_<issue>_<lang>.XML`, `PMC-`, `DML-`, `ICN-`; root `dmodule`, `pm`,
`dml` with the S1000D schema URL (issue number) in `xsi:noNamespaceSchemaLocation`. Issues 4.x,
5.0 and 6 are XML, 2.x and 3.0 can be SGML. Military and civil aerospace technical publications
(Airbus, defense programs, NATO ministries; ATA Spec 1000BR for civil). Data module code (a
hierarchical index), issue info, change marks and reasons for update (revisions), applicability
(product and serial effectivity: a graph), `dmRef`/`pmRef` references (graph), procedures with
steps, warnings and cautions, spares, tools and consumables tables, BREX rules, graphics
references. s1000d.org: "The specification is publicly available and is free to download from
this web site", while the site terms reserve all rights; the bike sample's redistribution terms
were not found, and A4A Spec 1000BR says "the data receiver is prohibited from any
re-distribution of the BREX data modules to any other party". s1kd-tools is GPL-3.0 (license
only); oxygenxml/S1000D is Apache-2.0. No crate. Effort M. **W2** once a redistributable sample
exists.

### AIXM 5.1 / 5.1.1

XML root `message:AIXMBasicMessage` or features in `http://www.aixm.aero/schema/5.1.1`.
EUROCONTROL EAD, FAA NASR and SWIM NOTAM, European AIP datasets. Aeronautical features with
timeslices (BASELINE, TEMPDELTA with validity periods: a strong timeline), xlink relations,
units. The schemas are BSD ("Copyright (c) 2016, EUROCONTROL & FAA ... Redistribution and use in
source and binary forms, with or without modification, are permitted", "an instance of the BSD
license template"); datasets carry provider terms. `traffic-thrust` 0.2.2 (MIT) parses
EUROCONTROL baselines. Effort M-L (temporality, GML). **W2 / later.**

### ARINC 429 label lists and ICD spreadsheets

No standard file: ICDs live in xlsx (sometimes XML or DOORS). Columns: octal label, SDI, SSM,
BNR/BCD/discrete, bit range, resolution/LSB, range, unit, rate; the same holds for ARINC 664
(AFDX) and MIL-STD-1553 ICDs. Signals with encoding (`value = raw * LSB`), units, transmitter
and receiver LRUs (a graph). Covered by the spreadsheet reader; the **signal ICD profile** of
section 6 turns them into signal records with formulas and a Mermaid graph. `arinc429` 0.1.0 and
`arinc-429` 0.1.5 only decode words.

### DO-178C artifacts, DO-200B

DO-178C work products (PSAC, SDP, SVP, SCMP, SQAP, SAS, trace matrices) are office, PDF, xlsx,
DOORS or ReqIF documents (FAA AC 20-115D recognizes DO-178C): covered by existing readers. The new
piece is a **cross-format trace-ID graph** (HLR to LLR to code to test, Annex A objectives),
M, W2, shared with ISO 26262 (section 2) and ReqIF. DO-200B is a process standard: **skip**.

### ARINC 653, ARINC 665, ATA specs, FACE

- **ARINC 653 configuration XML** (root `ARINC_653_Module` or vendor roots for VxWorks 653,
  PikeOS, Deos, LynxOS-178 [unverified]): partitions, memory, major frame schedule (timeline),
  ports and channels (graph), health monitor tables. Schema in the paid ARINC 653 Part 1. No
  public sample. **Later**; the generic XML reader covers basic reading.
- **ARINC 665 loadable software parts** (`*.LUH`, `FILES.LUM`, `LOADS.LUM`, `BATCHES.LUM`; binary
  big-endian with CRCs; ARINC 838 adds an XML header): part numbers, target hardware IDs, file
  lists with CRCs and lengths, load-file relations. ARINC 665-5 (2019) is paid; no sample, no
  crate. **Later:** detect by file name and report what was found.
- **ATA iSpec 2200, Spec 100, Spec 2000** (A4A, paid): SGML/XML manuals (AMM, IPC, WDM, SRM)
  with ATA chapter numbering, effectivity, revision bars, parts trees; still dominant in civil
  aviation next to S1000D (weak evidence). No Rust SGML parser with DTD support (section 6).
  **Later:** detect, say "export XML or PDF".
- **FACE data model** (Open Group FACE 3.x, Open UDDL; XMI): "all editions are available at no
  cost but require the creation of a free account". Entity model with measurement systems and
  units. M, **later**.

### Area ranking

1. ARINC 424 (public CIFP data, crates exist), 2. S1000D (rich graphs, needs a sample),
3. AIXM, 4. the ICD profile and trace-ID graph (cross-cutting). Later: ARINC 653, 665, ATA, FACE.

## 5. Space

FITS is in section 7 (science). PDS labels are here because they describe mission archives.

### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| CCSDS XTCE (OMG XTCE 1.2/1.3) | Yamcs, LASP, F Prime, OMG/CCSDS | XML | free | none on crates.io; sdtk-xtce (git, MIT) | 1 (Apache-2.0) | M | 3/2/1/3/3 | W1 |
| CCSDS NDM (OEM, OPM, OMM, OCM, TDM, CDM, AEM, APM, ACM) | agencies, Space-Track, TraCSS, Orekit | KVN text / XML | free | ccsds-ndm 0.0.9 (MPL-2.0) | 1 | S-M | 3/1/3/1/3 | W1 |
| SPICE text kernels (LSK, FK, PCK, IK, SCLK, MK) | all planetary missions | text | NAIF rules (custom) | spicekit 0.2.2 (MIT), anise 0.10.6 (MPL-2.0), lox-io | 1 | S | 3/2/2/1/2 | W1 |
| PDS4 labels | NASA PDS (mandatory), ESA PSA, JAXA | XML | free; NASA-PDS tools Apache-2.0 | none | 1 (Apache-2.0) | M | 3/2/1/0/3 | W1 |
| SCOS-2000 MIB | ESA missions | folder of TSV tables | ESA ICD, terms unclear | sdtk-scos (git, MIT) | none | M | 3/3/1/3/3 | W2 |
| SPICE DAF/DAS binary (SPK, CK, PCK, DSK) | planetary missions | binary 1024-byte records | NAIF rules | anise, spicekit, muad-dib 0.3.1; not cspice-* | none | M | 2/1/2/0/2 | W2 |
| PDS3 labels (ODL/PVL) | legacy PDS archives, ISIS | text | free | none | none | S-M | 3/1/1/0/2 | W2 |
| CCSDS SOIS SEDS (876.0-B) | NASA cFS EdsLib, ESA SAVOIR, Gateway | XML | free | none | none | M | 3/2/1/2/2 | W2 / later |
| ECSS-E-TM-10-25 (OCDT, CDP4-COMET) | concurrent design facilities | zip of JSON | free TM; COMET SDK LGPL | none | none | M | 3/3/2/1/3 | later |
| Yamcs spreadsheet MDB, Open MCT JSON | ground systems | xls / JSON | Yamcs AGPL; Open MCT Apache-2.0 | n/a | none | S | 2/1/0/2/3 | later / skip |
| SANA registries | CCSDS users | XML/CSV | public [unverified] | n/a | none | S | 1/0/1/0/3 | skip (lookup data) |
| ECSS documents, PUS | ESA and industry | PDF/Word | ESA copyright | spacepackets (runtime) | none | covered | 1/1/1/0/2 | covered |
| CCSDS SLE | ground stations | protocol | - | - | - | - | - | skip (not files) |

### CCSDS XTCE

`.xml`, root `SpaceSystem`. Detection must accept three namespace variants seen in real files:
`http://www.omg.org/space/xtce` (1.1/1.2), `http://www.omg.org/spec/XTCE/20180204` (1.2/1.3) and
no namespace. Yamcs (native XTCE MDB), LASP space_packet_parser (IMAP, CTIM, JPSS test data),
NASA F Prime (fprime-xtce), OMG and CCSDS (660.0-B); active repos in 2026. Telemetry parameters
and types, units, packet containers with inheritance and restriction criteria (a graph),
commands with arguments, constraints and verifiers, and **calibrators as formulas**: polynomial
terms, spline points, MathOperation in RPN, context calibrators; alarms (watch to severe ranges),
derived parameters, author and history. OMG specs and CCSDS books are free [CCSDS reproduction
terms not fetched]. No crate on crates.io; greglucas/space-data-toolkit (GitHub, MIT, sdtk-xtce
and sdtk-scos) claims full XTCE 1.3 and is a reference or git candidate; xpromache/xtce-rs has
**no license**. References: space_packet_parser (BSD-3), xtcetools and python-xtce (Apache-2.0);
Yamcs is AGPL-3.0, pymdb LGPL-3.0, xtceparser GPL-3.0 (license only). The Apache-2.0 sample
BogusSAT-1 has 11 polynomial and 6 spline calibrators and 12 commands. Effort M on quick-xml.
**W1:** the best formula and signal format in the area.

### CCSDS Navigation Data Messages

Extensions `.oem .opm .omm .ocm .tdm .cdm .aem .apm .acm .kvn .xml`. KVN: first non-COMMENT line
`CCSDS_<TYPE>_VERS = x.y`; XML root `oem`/`opm`/... or combined `ndm`. **Collisions:** `.tdm` is
also NI DIAdem (section 8), `.cdm` a PowerDesigner model. Agencies (NASA, ESA, JAXA DARTS
publishes Geotail orbits as OEM), the US Office of Space Commerce TraCSS (recommends OCM),
Space-Track (CDMs), Orekit, GMAT, STK; ISO 26900 equals ODM. Object metadata (name, ID, center,
frame, time system), state vectors and ephemeris series with units, covariance, maneuvers,
conjunction risk (CDM), interpolation method, comments as provenance. Crate `ccsds-ndm` 0.0.9
(MPL-2.0, KVN and XML, very young, repo with about 40 examples); Orekit (Java, Apache-2.0) is the
full reference; nyx-space is AGPL-3.0. Effort S for KVN, M for XML and all types. **W1.**

### SPICE text kernels

First line `KPL/<type>` (LSK, FK, PCK, IK, SCLK, MK), `\begindata` and `\begintext` blocks.
`file` misreads `naif0012.tls` as LaTeX: key on `KPL/`. `.tf` collides with Terraform. Every NASA
and ESA planetary mission. LSK leap second table; FK frame tree (graph); PCK body constants and
pole polynomials (formulas); IK field of view; SCLK clock coefficients; MK kernel list (file
graph); comments and version history. NAIF rules (custom "SPICE Software License"):
"Redistribution of the unmodified SPICE Toolkit, such as from a mirror site, is prohibited
without prior, written clearance from the NAIF manager", while including toolkit modules in a
customer-built tool "is entirely appropriate"; kernel data terms are not stated [unverified].
Pure Rust: `spicekit` 0.2.2 (MIT, clean-room from the published NAIF specs; ships a body-name
table under NAIF terms), `anise` 0.10.6 (MPL-2.0, 225k downloads), `lox-io`. Own parser is easy.
Effort S. **W1.**

**SPICE binary kernels (DAF `DAF/SPK `, `DAF/CK  `, `DAF/PCK `; DAS `DAS/DSK `).** For Rezzer the
comment area (rich provenance text) and segment summaries (body, center, frame, type, coverage
timeline) matter, not the Chebyshev data. **Do not use** `rust-spice` (default feature pulls
`cspice-sys`), `cspice-sys` or `cspice-rs` (LGPL-3.0, linking CSPICE under the custom NAIF
rules). Use anise, spicekit or `muad-dib` 0.3.1 (MIT). DAF summary pointers can loop: probe.
Effort M. **W2.**

### PDS4 and PDS3 labels

**PDS4:** `.xml`, root `Product_Observational`, `Product_Collection`, `Product_Bundle` in
`http://pds.nasa.gov/pds4/pds/v1`, usually with `<?xml-model` instructions. Mandatory for new
NASA planetary missions; ESA PSA (BepiColombo, ExoMars), JAXA, ISRO. Logical identifier URN,
version, modification history (timeline), observation area (mission, instrument, target, time
range), internal references (graph), file areas with table field definitions (name, type, unit,
description) as a data dictionary. Information model and schemas free; NASA-PDS repos Apache-2.0;
pds4_tools BSD-3-style. No crate. Effort M (label to Markdown with a field table; referenced
tables via CSV, FITS, CDF readers). **W1.**

**PDS3:** `.lbl` or attached labels starting `PDS_VERSION_ID = PDS3` (sometimes after an SFDU
header); ODL OBJECT/GROUP tree, data pointers, columns with units. Cassini, MRO, MER, LRO archives
still served; ISIS cubes use PVL. The crate `odl` is an unrelated downloader. pvl and pdr are
BSD-3. S-M. **W2.**

### SCOS-2000 MIB

A folder of tab-separated `.dat` tables (`pcf.dat` parameters, `pid.dat` packet IDs, `plf.dat`
locations, `cur/caf/cap.dat` numeric calibrations, `mcf.dat` polynomial, `lgf.dat` logarithmic,
`txf/txp.dat` text calibrations, `ocf/ocp.dat` limit checks, `ccf/cdf/cpc.dat` commands; more
than 30 files). Detect by the set of known file names in a folder, never by one `.dat`. ESA/ESOC
mission control systems; Yamcs has a loader (AGPL); sdtk-scos (MIT, GitHub) follows
EGOS-MCS-S2K-ICD-0001 issue 7.0. Parameters with units, packet layouts, calibration formulas,
limit checks, commands, verification stages; table cross-references form a graph. The ICD's
official distribution terms are unclear [unverified]; no public sample found. Effort M. **W2.**

### CCSDS SEDS, ECSS, others

- **CCSDS SOIS Electronic Data Sheets (SEDS, 876.0-B):** XML root `PackageFile` or `DataSheet`
  [namespace unverified]. **Name collision with ODVA EDS and CANopen EDS** (INI text): never treat
  `.eds` as SEDS without the XML namespace. NASA cFS via nasa/EdsLib (Apache-2.0), ESA SAVOIR,
  Lunar Gateway. Interfaces, types with encodings and units, commands, telemetry, state machines.
  M, **W2 / later**.
- **ECSS documents and PUS (ECSS-E-ST-70-41C):** PDF and Word, covered. ecss.nl: "No ECSS document
  may be reproduced in any form without the explicit consent of ESA": never in the corpus. PUS
  packet definitions are expressed in XTCE or the MIB. `spacepackets` 0.18.0 is a runtime crate.
- **ECSS-E-TM-10-25A concurrent design model** (OCDT, RHEA CDP4-COMET; zip of JSON [layout
  unverified]): system decomposition, parameters with units per option and state, iterations.
  COMET SDK is LGPL-3.0 (license only). M, **later**.
- **Yamcs spreadsheet MDB** (xls with fixed sheets): later as a profile. **Open MCT dictionaries**
  (app-specific JSON): skip. **SANA registries:** skip as input; maybe lookup data if licensed.
  **CCSDS SLE** is a ground-station protocol, not a file: **skip**.

### Area ranking

1. XTCE, 2. CCSDS NDM (KVN first), 3. SPICE text kernels, 4. PDS4 labels, 5. SCOS-2000 MIB
(needs a sample and ICD terms), 6. SPICE binary comment and summaries, 7. PDS3, SEDS.
Later: TM-10-25, Yamcs xls MDB.

### Open questions / unverified (sections 4 and 5)

- FAA CIFP: allow one file over 2 MB or a documented excerpt (an excerpt is a modified file).
- A redistributable S1000D sample (s1000d.org bike set terms, Oxygen samples).
- SCOS-2000 MIB sample and ICD terms; NAIF kernel redistribution terms; CCSDS reproduction terms
  (they decide whether Blue Book examples like `oem_g11` may stay in the corpus).
- Detection details: SEDS namespace, LUH layout, ARINC 653 roots, DAF byte offsets.

## 6. Engineering models and tools, generic legacy

None of these formats are in the engineering report. `docs/research/2026-10-16-office.md` has no
hit for dBase, Lotus or WordPerfect; the landscape report advised leaving WordPerfect off because
of LGPL, which this section corrects. The Mathcad report already lists `.mlx` as OPC zip.

### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| FMI modelDescription / FMU | about 250 tools [count unverified] | XML in zip | free; spec CC-BY-SA, XSD BSD-2 | fmi-schema 0.8.0 | 1 (BSD-2) | S | 3/2/0/1/3 | W1 |
| Simulink .slx/.mdl + Stateflow | dominant in automotive and aerospace MBD | OPC zip XML / text | proprietary, undocumented | rustylink 3.0.7 (no LICENSE file) | 1 (BSD-3, MathWorks) | M | 3/3/1/1/2 | W1 |
| Excel ICD profile | aerospace, defense, automotive (weak) | spreadsheet | n/a | existing reader | synthesize | S | 3/3/0/2/3 | W2 |
| SSP (.ssp/.ssd/.ssv/.ssm) | Modelica Association tools | XML in zip | free; CC-BY-SA, XSD BSD-2 | none | none | S-M | 3/3/0/2/2 | W2 |
| Modelica .mo | Dymola, OpenModelica, SimulationX | text language | free; MLS CC-BY-SA, MSL BSD-3 | rumoca 0.7.28 (Apache-2.0) | 1 (BSD-3) | M | 3/3/1/3/2 | W2 |
| COBOL copybook + EBCDIC data | IBM Z shops, migrations | text + binary | IBM docs free | only AGPL/LGPL: own | 1 pair (Apache-2.0) | M | 2/1/0/1/3 | W2 |
| EBCDIC fixed records (alone) | IBM Z shops | binary | CCSID tables public | none usable | Cobrix data | S | 1/0/0/0/3 | W2 |
| dBase .dbf | every Shapefile, FoxPro | binary | docs free | dbase 0.8.0 (mature) | none | S | 1/0/1/0/3 | W2 |
| MATLAB .mat v5 / v7.3 | universal | binary / HDF5 | v5 PDF free | matrw 0.1.4, hdf5 crates | none | M / S on HDF5 | 2/0/2/0/3 | W2 |
| MATLAB .m | universal | text | n/a | tree-sitter-matlab 1.3.1 | none | S | 2/2/0/2/1 | W2 |
| man / mdoc | all Unix | macro text | free | libmandoc-rs 0.12.0 (ISC etc., young) | none | M | 2/2/1/0/2 | W2 |
| LabVIEW .lvproj / .lvlib | NI test and measurement | XML | undocumented | none | none | S | 2/2/1/0/1 | W2 |
| SysML v2 text / API JSON | OMG adopted 2025, early tools | text / JSON | free OMG; pilot EPL-2.0 | sysml-v2-parser 0.58.0 (young) | pinned (EPL-2.0) | M (JSON S) | 3/3/1/2/2 | W2 |
| SysML v1 / UML XMI | Cameo, EA, Papyrus | XML | XMI free; exports vary | none permissive | none | M | 3/3/1/1/2 | W2 |
| EA .qea, Cameo .mdzip | Sparx EA, Cameo | SQLite / zip | proprietary | rusqlite + RE | none | M / L | 3/3/1/1/2 | later / skip |
| Capella | rail, aerospace, defense | EMF XMI | EPL-2.0 tool | none | pinned (EPL-2.0) | M | 2/3/0/0/2 | later |
| LabVIEW .vi metadata | NI | RSRC binary | undocumented | none | none | M (diagram L) | 2/2/1/0/1 | later |
| SGML with DTD (iSpec 2200, EDGAR, OFX) | aerospace manuals, finance | text markup | ISO and iSpec paid | sgmlish 0.2.0 (no DTD) | none | L (OFX S) | 2/1/2/0/2 | later |
| WordPerfect | legal archives | binary | SDK [unverified] | unfair-codec-wp 0.1.0 (MPL-2.0, young) | none | M | 1/0/0/0/1 | later |
| Lotus 1-2-3 | archives | binary | old docs [unverified] | none (libwps MPL option, C++) | none | L | 1/0/0/2/3 | skip (detect) |

### Models and simulation

**FMI / FMU modelDescription.xml.** `.fmu` is a zip with `modelDescription.xml` at the root plus
`binaries/`, `sources/`, `resources/`; root `<fmiModelDescription fmiVersion="2.0|3.0">`.
Distinguish from other zips (`.slx`, `.ssp`, `.aasx`, `.mdzip`) by part names. About 250 tools
per the Modelica Association list (Simulink, Dymola, OpenModelica, Amesim, GT-SUITE, dSPACE)
[count unverified]; FMI 3.0.2 released 2024-11-27. Variables with value reference, causality,
variability, start, description; units as SI base exponents with factor and offset; the
ModelStructure lists outputs, derivatives and initial unknowns with `dependencies`: a real
signal graph. License (fmi-standard LICENSE.txt): "The FMI specification is released under the
CC-BY-SA"; "C-header and XML-schema files ... are released under the BSD 2-Clause license".
`fmi-schema` 0.8.0 (MIT OR Apache-2.0, 2026-07-28, 105k downloads) or about 300 lines on
quick-xml. **Never run the binaries** (`fmu-runner`, `libfmi` execute them). FMPy is BSD-2.
Effort S. **W1:** the most common model exchange format, small and open.

**SSP (System Structure and Parameterization).** `.ssp` zip with `SystemStructure.ssd`; `.ssd`
root `ssd:SystemStructureDescription` [unverified]. **`.ssd` collides with IEC 61850 SSD**: check
the namespace. SSP 2.0.1 (2026-05-05). System graph of FMU components, connectors, connections,
signal dictionaries, parameter sets (`.ssv`) and mappings (`.ssm`) with unit conversions. Same
license pattern as FMI. No crate. S-M. **W2**, right after FMI.

**Simulink `.slx` / `.mdl` with Stateflow.** `.slx` is an OPC zip with
`simulink/blockdiagram.xml`, `simulink/systems/system_*.xml`, `simulink/stateflow/chart_N.xml`,
`metadata/coreProperties.xml` (verified on the sample). `.mdl` is either classic `Model {` text
or text-packaged OPC (`__MWOPC_PACKAGE_BEGIN__`). **`.mdl` collides** with IBM Rational Rose,
NVIDIA MDL and game models. The de facto standard for model-based design in automotive and
aerospace control; SLX is the default since R2012b. Verified in the sample: `<Block BlockType
Name SID>` with `<P Name>` parameters, `<Line>` with `Src "2#out:1"` and `Dst "16#in:1"` (a clean
port-level graph), subsystems, Stateflow states with entry and during actions, transitions, data
with scope and type, and coreProperties (creator, last modified by, dates, revision, release
R2024b). Also solver configuration and `graphicalInterface.json`. MathWorks only says SLX is "a
compressed package that conforms to the OPC interoperability standard" with UTF-8 XML; the
MathWorks license agreement forbids licensees to "disassemble, decompile, or reverse engineer" a
Program (section 3.3 in the R2020a SLA); whether reading model files a user wrote is affected is
an open question (section 10). `rustylink` 3.0.7 has an Apache-2.0 Cargo field but **no LICENSE
file**: write an own reader on zip and quick-xml, keep reference files per MATLAB release.
Effort M. **W1:** the highest-value graph format in this area.

**Modelica (`.mo`, `package.mo`).** Text starting with `within X;` or a class keyword. **Collides
with GNU gettext `.mo`** (magic `0x950412DE`); libmagic reports the MSL sample as HTML because of
`Documentation` annotations, so Rezzer must not trust libmagic here. Dymola, OpenModelica,
SimulationX, Wolfram SystemModeler; MSL 4.1.0 (2025-05-23); energy, buildings (LBNL Buildings
library), automotive. Component declarations with parameters and units, `connect()` (graph),
equations and algorithms (formulas), annotations (placement, documentation, revisions,
experiment), `extends`. Language spec CC-BY-SA-4.0, MSL BSD-3-Clause. Crates: `rumoca` 0.7.28
(Apache-2.0, full compiler, crates.io release lags the active repo), `moparse` 0.1.6 (MPL-2.0,
archived). OpenModelica is OSMC-PL (license only). Effort M. **W2.**

**SysML v2 and KerML (`.sysml`, `.kerml`, API JSON).** OMG adopted SysML v2.0, KerML 1.0 and the
Systems Modeling API (OMG press release 2025-07-21); release repo active, pilot implementation
EPL-2.0. Parts, ports, connections, requirements with constraints, attributes with ISQ/SI units,
allocations, states, actions, calculations. `sysml-v2-parser` 0.58.0 (MIT, 60 versions since
2026-04) is young and churning; `syster-base` has no LICENSE file; **`mw-binding-xmi` is
AGPL-3.0** (and pitches "SysML into OKF", an overlap). Effort M (JSON S). **W2:** strategic,
wait for parser maturity.

**SysML v1 / UML as XMI; EA `.qea`; Cameo `.mdzip`; Capella.** XMI (`<xmi:XMI`) is a free OMG
spec but every tool adds `xmi:Extension` blocks; packages, blocks, ports, connectors,
requirements, stereotypes with tagged values. A generic XMI reader is M, **W2**. Sparx says "A
.qea or .qeax file is a SQLite database, the default type in Enterprise Architect Release 16.0
and later" (schema `t_object`, `t_connector` [unverified]): M, **later**. Cameo `.mdzip`: **skip**,
say "export XMI". Capella (EMF XMI, EPL-2.0, Thales origin; capellambse Apache-2.0 as reference)
reuses the XMI reader: M, **later**.

**Excel-based ICDs.** No signature; a column profile on the spreadsheet reader with header
synonyms (signal, source/producer, destination/consumer, unit, min/max, resolution, scale,
offset, data type, bit position, rate, bus, message, interface id). **Not IEC 61850 `.icd`.**
Programs in aerospace, defense and automotive keep ICDs in Excel (common practice, weak
evidence). Output: a signal table plus a Mermaid producer-consumer graph, `physical = raw *
factor + offset` as formulas. Synthetic corpus needed. Effort S. **W2** (shared with ARINC 429
label lists in section 4 and the table profiles of the engineering report).

### Tool files

**LabVIEW.** `.vi` starts `RSRC\r\n`, type `LVIN`, creator `LBVW` (`.ctl` `LVCC`, `.llb` `LVAR`;
facts from pylabview, MIT). `.lvproj` and `.lvlib` are XML with `LVVersion`. Projects list items,
targets, dependencies and build specs: S, **W2**. `.vi` metadata blocks (`vers`, `LIBN`, linker
info with subVI dependencies, `BDPW` password flag, `STRG` description, connector pane) are M,
**later**; the block diagram heaps are code, L and of unclear legality: say "not read". The NI
EULA may forbid reverse engineering [unverified].

**MATLAB `.m`.** Text; **collides** with Objective-C, Mathematica and Mercury. Function
signatures, H1 help, `arguments` validation blocks, section titles, called functions (graph),
classdef properties. `tree-sitter-matlab` 1.3.1 (MIT, C runtime) or an own scanner. S. **W2**
(overlaps with source code reading). MATPOWER cases are in section 3.

**MATLAB `.mat`.** v5: 128-byte header with text `MATLAB 5.0 MAT-file, Platform: ..., Created
on: ...`, version 0x0100 and endian tag `IM`/`MI`; v7.3 is HDF5 with that text in a 512-byte
user block. MathWorks: "Version 7.3 MAT files use an HDF5 based format". Variable directory
(name, class, dimensions, complex, sparse), struct fields, cell nesting, small values as tables,
statistics for large ones, creation date. Simulink logged signals are MCOS objects, opaque in v5
[unverified]. The MAT-File Format PDF (Level 4 and 5) is free (official URL returns 403 to curl;
a 2004 mirror exists). Crates: `matfile` 0.5.0 (MIT, 2024, stale), `matrw` 0.1.4 (MIT OR
Apache-2.0), `mat5` 0.1.0; v7.3 on the HDF5 crates of section 7. scipy.io.loadmat is BSD-3,
mat73 GPL-3 (license only). Effort M for v5, S for v7.3 on HDF5. **W2.**

### Generic legacy

**COBOL copybooks with data files.** Fixed-format COBOL: columns 1-6 sequence, column 7
indicator, level numbers, `PIC`, `COMP-3`, `REDEFINES`, `OCCURS ... DEPENDING ON`, level 88.
Full programs have `IDENTIFICATION DIVISION`. Used wherever IBM Z data is migrated (banks,
insurers, government, airlines; Cobrix and Spark exist for this). The copybook gives the record
layout as a table (level hierarchy, name, PIC, type: alphanumeric, zoned, packed, binary, float;
sign; implied decimal; offset and length), REDEFINES variants, OCCURS groups, condition names;
the data file then decodes to a value table. IBM Enterprise COBOL Language Reference is free
(numeric PIC uses "9, P, S, and V"); the ISO standard is paid. **No permissive Rust option:**
`copybook*` crates (copybook-rs) and `ironwork-*` are AGPL-3.0, `gnucobol-rs` is LGPL-3.0,
`kobold-data-shim` is Apache-2.0 but depends on gnucobol-rs. Do not read them; write an own
parser and decoder from the IBM docs (M). Cobrix (Scala, Apache-2.0, active) has test copybooks
and data; one Apache-2.0 pair downloaded. **W2:** mainframe data is unreadable without it, and
nobody offers it permissively in Rust.

**EBCDIC fixed-width records.** No magic; heuristic on 0x40 spaces, 0xC1-0xE9 letters, 0xF0-0xF9
digits, no 0x0A; 4-byte RDW prefix for variable records. encoding_rs has no EBCDIC, `oem_cp` says
"EBCDIC encodings will never be supported", `ebcdic` 0.1.2 covers IBM-1047 only, and
`copybook-charset` is AGPL. Rezzer needs its own 256-entry tables (CP037, 500, 1047, 1140 and the
German CP273 and 1141), taken as facts from ICU `.ucm` (Unicode license) or Unicode.org mappings.
S. **W2** with copybooks.

**dBase / FoxPro `.dbf`.** Version byte (0x02, 0x03, 0x30, 0x83, 0x8B, 0xF5 ...), date, record
count, header and record length, header terminator 0x0D; validate the arithmetic. **Collides with
Oracle datafiles.** Every Shapefile carries one, plus legacy FoxPro and Clipper apps. Typed
columns, last-update date, code page byte, deleted-row flags (report them). `dbase` 0.8.0 (MIT,
1.3M downloads, memo support); the `dbf` crate is AGPL (avoid). S. **W2**, shared with Shapefile
(section 3) and possibly STEP 7 projects.

**troff / groff man, mdoc, ms.** `.TH`, `.Dd`/`.Dt`/`.Os`, `.TL`/`.AU`/`.NH`; numeric extensions,
so detect by content. Every Unix man page. NAME, SYNOPSIS, OPTIONS, SEE ALSO (cross-reference
graph); mdoc semantic macros (`.Fl`, `.Ar`, `.Xr`, `.Ev`, `.Er`), tbl tables, `.Dd` date. groff is
GPL (external only); `libmandoc-rs` 0.12.0 vendors the ISC mandoc parser (all permissive, very
young, probe). M. **W2** for man and mdoc, ms/me/mm later.

**SGML with DTD.** `<!DOCTYPE x PUBLIC "-//..."` without `<?xml`, omitted end tags. ATA iSpec
2200 is "still in force today" for most of the Airbus and Boeing fleet (vendor page, medium
evidence; new programs use S1000D); SEC EDGAR and OFX 1.x still use SGML. Tag omission and
entities need the DTD. `sgmlish` 0.2.0 (MIT): "DTDs are not supported" (fine for OFX). OpenSP has
an MIT-style license (linkable, but a large C++ build). L for a DTD-aware parser. **Later:** detect
and say "SGML with DTD X, not read; export XML". OFX could be W2 if finance is in scope.

**WordPerfect** (`\xFFWPC`): libwpd is "MPL 2.0 / LGPLv2.1+" with an "Alternatively" clause, so
the MPL option is allowed under AGENTS.md; the real cost is the C++ build with librevenge and
boost. **Correction to the landscape report: the license is not the blocker.** Pure Rust
`unfair-codec-wp` 0.1.0 (MPL-2.0, 90 downloads) needs a probe. M, **later**. **Lotus 1-2-3**
(`.wk1`, `.wk3`, `.123`; `.wks` collides with MS Works): libwps has the same MPL option; archives
only. **Skip:** detect and say "export to xlsx".

### Area ranking

1. FMI (W1), 2. Simulink SLX with Stateflow (W1), 3. Excel ICD profile, 4. COBOL copybook with
EBCDIC, 5. SSP, 6. Modelica, 7. dBase, 8. MAT v5/v7.3, 9. man/mdoc, 10. LabVIEW projects and
MATLAB `.m`, 11. SysML v2, 12. XMI. Later: Capella, EA `.qea`, LabVIEW `.vi`, SGML, WordPerfect.
Skip: Lotus, Cameo `.mdzip`.

### Open questions / unverified

- MathWorks SLA and NI EULA reverse-engineering clauses vs reading user files.
- Simulink XML drift across releases R2012b to R2026b.
- No LICENSE file: rustylink, syster-base, yore. Confirm before use.
- SGML scope (OFX, EDGAR); ICD column synonyms from Jordi's domain experience.

## 7. Science

Two parts: 7.1 array and observation data (climate, astronomy, space physics, weather,
seismology), 7.2 lab science, biology, chemistry and research authoring. None of these formats
is in the engineering report or the CAD spike. PDS3/PDS4 labels are in section 5. Instrument
output files (chromatography, oscilloscopes, microscopes, qPCR) are in section 8. MATLAB `.mat`
is in section 6. Rezzer reads metadata and structure only, never bulk arrays.

### 7.1 Array and observation data

Two findings change the picture:

- **One pure-Rust crate family reads NetCDF classic, NetCDF-4 and HDF5:** `netcdf-reader` 0.9.2
  and `hdf5-reader` 0.9.2 (roteiro-gis/netcdf-rust, MIT OR Apache-2.0, repo created 2026-03-15,
  release 2026-10-19). They have metadata-only APIs and claim bounds-checked cursors, cycle
  detection and a fuzz suite. The risk is their age.
- **The WMO code tables are MIT on GitHub** (wmo-im/GRIB2, wmo-im/BUFR4, wmo-im/CCT; LICENSE.md
  "The MIT License (MIT) Copyright 2020-2024"). The expected licensing blocker for GRIB and BUFR
  does not exist.

#### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| FITS | all astronomy | text cards + binary | free IAU standard; CFITSIO NASA permissive | fitsrs 0.4.1, wcs 0.4.2, fits-well 0.3.0 (pure) | 1 | S | 3/1/3/2/3 | W1 |
| NetCDF classic (CDF-1/2/5) | climate, ocean, CMIP6, ERA5 | binary header | free; netcdf-c BSD-3 | netcdf-reader 0.9.2, netcdf3 0.6.1 (pure) | 1 | S | 3/2/2/2/2 | W1 |
| NetCDF-4 | same | HDF5 | free | netcdf-reader (pure, young) | 1 | S-M | 3/2/2/2/2 | W1 |
| HDF5 (NeXus, HDF-EOS5, MAT 7.3, NWB) | NASA, synchrotrons, ML, MATLAB | binary container | free; libhdf5 BSD-3 | hdf5-reader 0.9.2, hdf5-pure 0.47.0, rust-hdf5 0.7.5 (pure, 2026) | via NetCDF-4 | S-M | 3/3/1/1/2 | W1 |
| FDSN StationXML | FDSN web services | XML | free; repo unlicensed | stationxml-rs 0.2.4 (young) | none | S-M | 3/2/2/3/3 | W2 (W1 if seismology) |
| GRIB1 / GRIB2 | all NWP centres, WMO | binary messages | WMO tables MIT | grib 0.20.0 (pure, GRIB2), gribberish 1.8.4 | URL only | S-M | 3/1/3/2/3 | W2 |
| miniSEED 2/3 | all seismic networks | binary records (+JSON v3) | free FDSN spec; libmseed Apache-2.0 | miniseed-rs 0.2.2 (pure, young) | 1 | S | 2/1/3/0/3 | W2 |
| Zarr v2/v3 | cloud climate, OME-Zarr | JSON + chunk objects | free | zarrs_metadata 0.7.5 | none | S | 3/2/1/1/2 | W2 |
| NASA CDF | heliophysics (SPDF) | binary records | spec free; C lib custom non-OSI | cdf 0.1.4 (pure, tiny) | none | M | 3/2/2/1/2 | W2 |
| BUFR | WMO observations | table-driven bits | WMO tables MIT | tinybufr 0.1.3 (young) | URL only | M-L | 2/1/2/2/2 | later |
| SAC | seismology research | fixed binary header | manual free | sacio 0.1.0 (stale) | none | S | 3/0/2/0/2 | later |
| Dataless / full SEED | legacy seismology | blockettes | free | none | none | M-L | 3/2/2/3/2 | later (detect) |
| HDF4 / HDF-EOS2 | legacy NASA EOS | binary DD blocks | free; BSD-style lib | none | none | L | 2/2/1/1/2 | later (detect) |

#### FITS (.fits, .fit, .fts, .fz)

First card `SIMPLE  =                    T`, 80-character ASCII cards in 2880-byte blocks. No real
collision (`.fit` is also the Garmin activity format, `.FIT` at offset 8). Used by all of
astronomy (HST, JWST, Chandra, ESO, IVOA) and solar physics; FITS 4.0 is maintained by the IAU
FITS Working Group. Holds header cards with units (`BUNIT`, `TUNITn`), WCS keywords (`CTYPEi`
such as `RA---TAN-SIP`, `CRVALi`, `CRPIXi`, `CDi_j`, SIP `A_p_q`/`B_p_q`: projection formulas),
`BSCALE`/`BZERO` linear conversion, time keywords (`DATE-OBS`, `MJD-OBS`, `TIMESYS`), `HISTORY`
and `COMMENT` cards (the corpus sample has 41 HISTORY cards naming calibration reference files:
a processing timeline), `CHECKSUM`, extensions (IMAGE, TABLE, BINTABLE with column names, units
and formats), `EXTNAME`/`EXTVER`, instrument and observation parameters. CFITSIO license:
"Permission to freely use, copy, modify, and distribute this software and its documentation
without fee is hereby granted". Pure Rust: `fitsrs` 0.4.1 (Apache-2.0 OR MIT, CDS Strasbourg,
keeps card order, COMMENT/HISTORY, CONTINUE), `wcs` 0.4.2 (projections), `fits-well` 0.3.0
(claims full FITS 4.0 and hostile-input safety, released 2026-10-03). `fitsio` links CFITSIO.
**Exclude** `rustronomy-fits` (GPL-3.0+) and the `fits` crate (GPL-2.0+); wcslib is LGPL-3.0
(license only). Own header-card reader is S. **W1:** best value per line in this area.

#### NetCDF classic and NetCDF-4 (.nc, .nc4, .cdf)

Classic: `CDF` plus version byte 0x01, 0x02 or 0x05. NetCDF-4: HDF5 magic
`89 48 44 46 0d 0a 1a 0a` at offset 0 or 512 times a power of two, told apart from plain HDF5 by
`_NCProperties`, `_Netcdf4Dimid` or `DIMENSION_LIST`. **Name collisions:** `.nc` is also a CNC
program (section 8, CAD spike), decide by content; `.cdf` is also NASA CDF, ASAM CDF and ANDI.
Used for CMIP6 climate output, Copernicus ERA5 downloads, NOAA/NCEI, Argo, OceanSITES
(Unidata/netcdf-c active, 607 stars). Holds dimensions (unlimited too), typed variables,
attributes with CF semantics: `units` (UDUNITS), `standard_name`, `long_name`, `coordinates`,
`bounds`, `cell_methods`, `grid_mapping` (CRS), `calendar`, `_FillValue`, `scale_factor` and
`add_offset` (a linear formula), and the global `history` (timestamped processing steps: a
timeline). Relations: variable to dimension to coordinate variable, `ancillary_variables`,
`grid_mapping`. Spec free (Unidata, also an OGC standard), CF conventions free, netcdf-c
BSD-3-Clause. Crates: `netcdf-reader` 0.9.2 (pure Rust, CDF-1/2/5 and NetCDF-4; Rezzer must
disable its `FilesystemExternalLinkResolver` and `FilesystemExternalFileResolver`), `netcdf3`
0.6.1 (pure, classic), `netcdf` 0.12.1 (links netcdf-c and libhdf5, both permissive, but C).
Classic header by hand is S; NetCDF-4 via the crate S-M. **W1.** The same classic reader serves
ANDI chromatography (section 8).

#### HDF5 (.h5, .hdf5, .he5, .nxs, MATLAB 7.3 .mat)

Magic as above. `.hdf` is used for both HDF4 and HDF5; MATLAB v7.3 `.mat` is HDF5 with a 512-byte
userblock. NASA HDF-EOS5, NeXus (synchrotron and neutron), MATLAB 7.3, NWB neurophysiology, model
weights, simulations (HDFGroup/hdf5 active). Holds a group tree with hard, soft and external
links (cycles possible), datasets with type, shape, chunking and filters, attributes on any
object, dimension scales and object references (relations), plus conventions (NeXus `NX_class`,
HDF-EOS `StructMetadata` ODL text, NWB schema). Spec free; libhdf5 LICENSE: "This software library
and utilities is covered by the 3-clause BSD License". Pure-Rust readers are all 2026 projects:
`hdf5-reader` 0.9.2 (read-only, same family as netcdf-reader, no szip, no virtual datasets),
`hdf5-pure` 0.47.0 (read/write, 128k downloads, CI against libhdf5), `rust-hdf5` 0.7.5 (MIT),
`consus-hdf5` 0.3.0 (very broad claims, not ranked). `hdf5-metno` 0.15.0 binds libhdf5 (mature,
C). **Refuse external links and external raw-data files** (they open other paths). Effort S-M.
**W1**, same crate family as NetCDF-4; detect and name NeXus, HDF-EOS5, MATLAB 7.3 and NWB.

**HDF4 / HDF-EOS2** (magic `0e 03 13 01` [unverified]): legacy NASA EOS (MODIS collection 6.1
still HDF4 [unverified]); no Rust crate; own reader L. **Later:** detect and say "convert with
h4toh5 or read with GDAL".

#### NASA CDF (Common Data Format, .cdf)

First u32 `CD F3 00 01` (v3), `CD F2 60 02` (v2.6) or `00 00 FF FF` (v2.0), second u32
`00 00 FF FF` or `CC CC 00 01` (compressed) [from the MIT cdf-rs source, NASA document not
fetched]. **Name collision** with ASAM CDF (XML, section 2), NetCDF classic (`CDF\x01`), ANDI and
Wolfram CDF. Heliophysics: SPDF CDAWeb, MMS, THEMIS, Van Allen Probes [mission list unverified].
ISTP vocabulary: `CATDESC`, `UNITS`, `FIELDNAM`, `DEPEND_0..3` and `LABL_PTR` (relations),
`VAR_TYPE`, `FILLVAL`, `VALIDMIN`/`VALIDMAX`, time types `CDF_EPOCH`, `CDF_EPOCH16`,
`CDF_TIME_TT2000`, global `Logical_source`, `PI_name`, `Data_version`. The internal format
description is free; **the NASA CDF C library has custom non-OSI terms** ("may be copied or
redistributed as long as it is not sold for profit, but it can be incorporated into any other
substantive product ... for profit or non-profit"): do not link. Pure Rust `cdf` 0.1.4 (MIT,
written from the spec, 852 downloads, probe needed). Effort M. **W2.**

#### GRIB edition 1 and 2 (WMO FM 92; .grib, .grb, .grib2)

`GRIB` at message start, edition at byte 7, end `7777`; files are concatenated messages,
sometimes with junk between them. Every NWP centre (ECMWF, NOAA GFS/HRRR, DWD ICON open data,
JMA); mandatory on the WMO WIS/GTS. Per message: centre, reference time, production status,
table versions, grid definition (projection, bounds, points), product definition (parameter
name and unit from code tables, level, forecast time, statistical processing, ensemble member),
packing with `Y = (R + X * 2^E) / 10^D`. A file is a table of fields over forecast steps.
WMO-No. 306 is a free download [copyright not fetched]; tables MIT (wmo-im); ECMWF local tables
in ecCodes (Apache-2.0). Crates: `grib` 0.20.0 (MIT OR Apache-2.0, 105k downloads, since 2020,
builds tables from the MIT wmo-im repos; metadata needs no codec), `gribberish` 1.8.4 (MIT,
GRIB1 module; turn off its default C openjpeg feature), `eccodes` 0.15.1 (binds ecCodes C,
Apache-2.0). Samples: ecCodes templates pinned, not downloaded. Effort S-M. **W2** (W1 if a
weather or energy user appears): inventory only, never decode fields.

**BUFR (WMO FM 94).** `BUFR` at message start, possibly inside GTS bulletin headers. Bit-packed
and table-driven (Table B elements with unit, scale, reference, width; Table D sequences;
operators; delayed replication): it cannot be parsed at all without the tables. Tables MIT
(wmo-im/BUFR4); local centre tables may be missing (say so). `tinybufr` 0.1.3 (young). Effort M-L
for full decoding, M for sections 0-3. **Later.**

#### Seismology

**FDSN StationXML (.xml, .staxml).** Root `FDSNStationXML`, namespace
`http://www.fdsn.org/xml/station/1` (SeisComP SC3ML root `seiscomp`). Returned by fdsnws-station
at every FDSN data centre. Network > Station > Channel tree with epochs (timeline), coordinates,
azimuth, dip, sample rate, equipment with serial numbers and calibration dates, and the
instrument response as a staged chain of formulas: PolesZeros (Laplace transfer function with
normalization), Coefficients, FIR, Polynomial, Decimation, StageGain, InstrumentSensitivity with
input and output units (m/s to counts). The FDSN repo has **no license file**: read files, do
not copy its XSD or examples. `stationxml-rs` 0.2.4 (Apache-2.0, very young); own quick-xml
reader is easy. Effort S-M. **W2**, W1 if seismology is in scope: the knowledge-richest seismology
format.

**miniSEED 2 and 3, SEED.** miniSEED 2: six ASCII digits, quality `D`/`R`/`Q`/`M`, blockette 1000;
miniSEED 3: `MS` plus 0x03, little-endian, CRC-32C, JSON extra headers. All global networks;
miniSEED 3 is an FDSN standard (2023); libmseed is Apache-2.0. Per record: FDSN source identifier,
start time, sample rate, encoding, timing and data quality, publication version, v3 JSON extras
(clock exceptions, vendor metadata); gaps and overlaps as a timeline. `miniseed-rs` 0.2.2
(Apache-2.0, pure, young), `mseed` 0.7.1 (links libmseed); `miniseed` 1.0.1 fetches C code with
git2 in `build.rs` (exclude). S. **W2.** Dataless/full SEED (response blockettes) is M-L:
**later**, detect and recommend StationXML. **SAC** (fixed 632-byte header with station, event,
phase picks; `sacio` stale) is S, **later**. ObsPy is LGPL-3.0 (license only).

#### Zarr (comparison)

Directory or zip store; v2 `.zarray`/`.zgroup`/`.zattrs`, v3 `zarr.json` per node. Cloud climate
(Pangeo, cloud ERA5 and CMIP6) and OME-Zarr microscopy: the NetCDF/HDF5 data model plus CF
attributes in JSON. `zarrs_metadata` 0.7.5 (MIT OR Apache-2.0). S, reuses the CF renderer; needs
directory or zip input. **W2.**

#### Ranking (7.1)

1. FITS, 2. NetCDF classic and 4, 3. HDF5 (one crate family with 2), 4. StationXML, 5. GRIB2
inventory, 6. miniSEED, 7. Zarr, 8. NASA CDF, 9. BUFR (detect plus section 1), 10. SAC and HDF4
(detect).

#### Notes (7.1)

- No hard licensing blocker: libhdf5 (BSD-3), HDF4 (BSD-style), netcdf-c (BSD-3), CFITSIO (NASA
  permissive), ecCodes and libmseed (Apache-2.0). Only the NASA CDF C library has custom terms.
- Unrelated crates: `seed` (web framework), `cdflib` (statistics).
- Every pure-Rust HDF5/NetCDF-4 reader is from 2026: probe truncated, cyclic, huge-dimension and
  external-link files; budget concatenated GRIB/BUFR messages and miniSEED records.
- Unverified: NASA CDF magic against the NASA document, MODIS still in HDF4, GRIB1 table licensing.

### 7.2 Lab science, biology, chemistry, research authoring

None of these formats is researched in `docs/spike-2026-10-19/web-text.md` or the landscape
report; the landscape report only notes that Xberg (formerly Kreuzberg) and pandoc (GPL) read
LaTeX and ipynb. OpenReadout (section 8) also covers mzML, mzXML, Thermo RAW, JCAMP-DX, Bruker
TopSpin, Varian VnmrJ and ImageJ TIFF.

#### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| Jupyter .ipynb | data science, research | JSON | BSD-3 schema | nbformat 3.0.0 (BSD-3) | own fixtures | S | 2/1/1/1/2 | W1 |
| BibTeX / BibLaTeX | LaTeX world, Zotero, JabRef | text | de facto | biblatex 0.12.0 (4.1M dl) | own fixtures | S | 3/2/0/0/3 | W1 |
| RIS, EndNote XML/.enw, PubMed .nbib | database exports, EndNote | text / XML | de facto / proprietary | biblib 0.10.0 | own fixtures | S | 3/1/0/0/3 | W1 |
| CSL-JSON | Zotero, pandoc, Manubot | JSON | MIT schema | citationberg 0.7.0 (json feature) | own fixtures | S | 3/1/0/0/3 | W1 |
| LaTeX sources | maths, physics, CS publishing, arXiv | macro text | LPPL, no issue | badness-parser 0.11.2 (young) | own fixtures | M | 2/3/0/2/2 | W1 |
| SBML | systems biology, BioModels | XML + MathML + RDF | free; libSBML LGPL | rust_sbml 0.7.0 (stale): own | 1 (CC0) | M | 3/3/1/3/3 | W1 |
| ELN (.eln, RO-Crate) | eLabFTW, Kadi4Mat, RSpace, PASTA, SampleDB, ... | zip + JSON-LD | MIT spec, IANA registered | rocrate 0.2.1, ro-crate-rs 0.6.0; own | 1 | S-M | 3/3/2/0/2 | W1 |
| JCAMP-DX | spectroscopy exports | text | IUPAC free | sciformats 0.2.0 (tiny) | 1 (MIT) | M | 3/1/0/1/2 | W2 |
| Bruker NMR folder | NMR labs | JCAMP-style text + binary | no spec | openreadout-nmr 0.4.0, nmr 0.1.1 | none | S after JCAMP | 3/1/2/0/2 | W2 |
| CIF / mmCIF (PDBx) | crystallography, PDB | STAR text | IUCr: conformance only | crystal-cif-io 0.2.0, pdbtbx 0.12.0 | none | M | 3/2/1/0/3 | W2 |
| GenBank / EMBL | NCBI, ENA, plasmid tools | text | free | gb-io 0.9.0 (no EMBL) | none | S / M | 3/2/1/0/3 | W2 |
| mzML | proteomics, metabolomics | XML + base64 | free; PSI-MS CV CC BY 4.0 | mzdata 0.67.4 (Apache-2.0) | 1 (Apache-2.0) | S / M | 3/1/2/1/2 | W2 |
| CellML | physiome, cardiac models | XML + MathML | free [unverified] | none; libcellml Apache-2.0 C++ | none | M | 2/3/0/3/2 | W2 |
| ImageJ TIFF metadata | microscopy | TIFF tags | public domain, informal | tiff 0.11.3, fast-tiff-lib (MPL-2.0) | none | S | 3/0/1/0/1 | W2 (TIFF reader) |
| mzXML | legacy MS | XML | free [unverified] | openreadout-mzml | none | S | 2/1/1/0/1 | later |
| PDB legacy | older bio tools | fixed columns | free, frozen | pdbtbx | none | S | 2/2/1/0/2 | later |
| FASTA / FASTQ | all bioinformatics | text | free | noodles, needletail, bio (mature) | none | S | 1/0/0/0/2 | later (summary) |
| Varian procpar | legacy NMR | text + binary | no spec | openreadout-nmr | none | S | 3/0/1/0/2 | later |
| IDL .sav | astro, solar, remote sensing | tagged binary | unofficial spec | none (scirs2-io as reference) | none | M | 2/1/1/0/2 | later |
| EndNote .enl | EndNote | SQLite + folder | closed | rusqlite + RE | none | L | 3/1/0/0/3 | later / skip |
| Thermo RAW | Thermo MS | binary | vendor SDK, restrictive EULA | thermorawfile 0.1.1, openreadout-thermo (new) | none | L | 3/1/2/0/2 | skip (detect) |
| Origin .opj/.opju | lab plotting | binary | closed; liborigin GPL | none | none | blocked | 2/2/0/2/3 | skip (detect) |

#### Research authoring

**Jupyter notebook (.ipynb).** JSON with `"nbformat": 4` (or 3), `"cells"`, `"metadata"`. Holds
kernelspec and language_info, Markdown cells (to Rezzer's Markdown reader), code cells with
`execution_count` (execution order; out-of-order runs are knowledge), outputs (stream text,
`execute_result` and `display_data` mimebundles, `image/png` to the vision list, errors with
`ename`, `evalue`, traceback), cell ids and tags, attachments, widget state, papermill
parameters. nbformat schema BSD-3 (jupyter/nbformat, active). Crate `nbformat` 3.0.0
(BSD-3-Clause, 2026-04-26, 320k downloads, v3 and v4). **Never execute.** Hostile input: huge
base64 outputs, ANSI escapes, HTML/JS in outputs (data only, strip scripts). Effort S. **W1.**

**Citation family: BibTeX/BibLaTeX, RIS, EndNote XML and `.enw`, PubMed `.nbib`, CSL-JSON.**
Signatures: `@<type>{key,` (`.bib`); `TY  - ` ... `ER  - ` (`.ris`); EndNote XML
`<xml><records><record>` [unverified]; CSL-JSON is an array of objects with `id`, `type`,
`author:[{family,given}]`, `issued:{date-parts}` (heuristic sniff). Universal in LaTeX authoring
and as exports of PubMed, Scopus, Web of Science, Zotero, Mendeley and EndNote. Entries with
type, key, fields, `crossref`/`xdata` inheritance (relations), DOIs. Crates: **biblatex 0.12.0**
(MIT OR Apache-2.0, typst, 4.1M downloads, mature), **biblib 0.10.0** (MIT OR Apache-2.0 on
crates.io, GitHub shows Apache-2.0, 294k downloads; RIS, `.nbib`, PubMed XML, EndNote XML,
`.enw`, BibTeX and CSV into one Citation struct), **citationberg 0.7.0** (MIT OR Apache-2.0,
3.6M downloads, CSL-JSON behind the `json` feature), nom-bibtex 0.6.0 (MIT). Trap: crate `ris`
0.1.0 is an unrelated algorithm library. citeproc-js is CPAL/AGPL [unverified]: license only.
EndNote `.enl` (EndNote 20+ is SQLite per PRONOM fmt/1684, `.enlx` compressed fmt/1685) is
undocumented: **detect and say "export XML or RIS"**. Effort S for the family. **W1** as one
"citations" reader with one model; it feeds the `\cite` graph of LaTeX.

**LaTeX sources (.tex, .ltx; .sty/.cls; .bbl/.aux).** `\documentclass` or `\begin{document}`
(heuristic; plain TeX and ConTeXt differ). The standard of maths, physics and CS publishing and
arXiv sources. Holds document class and packages, title/author/abstract, the sectioning tree, the
`\label`/`\ref`/`\eqref`/`\autoref` graph, `\cite` keys linked to `.bib` entries, the
`\input`/`\include` file graph, `\newcommand` macros as a table, figures with captions and
`\includegraphics` paths (vision list), `tabular` to tables, math kept as LaTeX, TODO comments.
Crate `badness-parser` 0.11.2 (MIT, created 2026-08, lossless CST, semantic model, bib module,
young). texlab is GPL-3.0 (license only). **Never use tectonic** (MIT, but a TeX engine: it
executes the document, which breaks "never run code from files"). Parse statically and list
unexpanded user macros as unresolved. Effort M (about 800 lines). **W1.**

**ELN file format (.eln, TheELNConsortium).** Zip with `<root>/ro-crate-metadata.json`; IANA
`application/vnd.eln+zip`. Import/export list in the spec README: eLabFTW, Kadi4Mat, PASTA,
SampleDB, RSpace, OpenSemanticLab, SciLog, datalab, NOMAD and LinkAhead (import), AI4Green and
others; spec repo active 2026-10-19, MIT. RO-Crate JSON-LD graph: datasets (experiments) with
author, created and modified dates, keywords, status; `variableMeasured` as PropertyValue with
unit; comments with author and date; HowToStep protocol steps; files with sha256 and size; links
between experiments (graph); per-dataset licenses; CreateAction provenance. Crates `rocrate`
0.2.1 and `ro-crate-rs` 0.6.0 are young; an own serde_json graph walk is S. Embedded files go
through Rezzer's other readers recursively. **Check the in-crate license of every example**: the
eLabFTW and PASTA examples declare CC BY-NC-SA 4.0 inside an MIT repo (not kept), PASTA contains
a real e-mail address. Effort S-M. **W1.**

#### Biology models

**SBML.** Root `<sbml xmlns="http://www.sbml.org/sbml/level{1,2,3}/version{n}"`. The standard for
systems biology models (BioModels at EBI, COPASI, tellurium, VCell [tool list unverified]). The
CC0 sample (BIOMD0000000012, the repressilator) has 6 species, 12 reactions with 12 MathML
kinetic laws, 16 parameters, 9 assignment rules, 3 unit definitions, SBO terms, RDF annotations
(bqbiol/bqmodel to identifiers.org, UniProt, GO), notes with the citation and creation and
modification dates. Reactions give a bipartite species-reaction graph for Mermaid; kinetic laws
and rules become formulas. Spec free at sbml.org (license file missing [unverified]); libSBML is
LGPL-2.1 and the `libsbml` crate LGPL-3.0 (excluded). `rust_sbml` 0.7.0 (MIT OR Apache-2.0) is
stale since 2021. Own reader on quick-xml plus an own content-MathML-to-infix renderer (shared
with CellML), effort M (about 700 lines). **W1:** the flagship "knowledge, not text" case: graph
plus formulas plus annotations.

**CellML.** Root `<model xmlns="http://www.cellml.org/cellml/2.0#"`. Physiome Model Repository,
OpenCOR, Chaste, Myokit. Components with variables (units, initial values, interfaces), MathML
ODEs, connections (component graph), encapsulation, imports across files, unit definitions.
libcellml is C++ Apache-2.0. Effort M, mostly shared with SBML. **W2**, right after SBML.

#### Chemistry and spectroscopy

**JCAMP-DX (.jdx, .dx, .jcamp).** First line `##TITLE=`, then `##JCAMP-DX=`. **`.dx` collides**
with Agilent OpenLab CDS. The vendor-neutral spectrum export of almost every IR, Raman, UV, NMR
and MS package, although IUPAC says "active development of JCAMP-DX was halted in 2006". Labelled
data records (`##TITLE`, `##DATA TYPE`, `##ORIGIN`, `##OWNER`, `##XUNITS`, `##XFACTOR`,
`##NPOINTS`), vendor `##$` parameters, `##PEAK TABLE`, `##PEAK ASSIGNMENTS`, LINK blocks with
`##BLOCK_ID` cross references (graph), NTUPLES (2D NMR), compressed forms AFFN, PAC, SQZ, DIF,
DUP with Y-checkpoint checks. IUPAC recommendations are free (Pure Appl. Chem. 71 (1999) 1549,
version 5.01). `sciformats` 0.2.0 (MIT, one author, 428 downloads) also reads ANDI; jcampconverter
(JavaScript, MIT) and nmrglue (BSD-3) are references. Effort M (about 600 lines). **W2**; the same
LDR parser reads Bruker `acqus`/`procs`.

**Bruker NMR (TopSpin experiment folder).** `<expno>/acqus` (`##TITLE=`, `##ORIGIN= Bruker`,
`##$` parameters), `pulseprogram`, `pdata/<procno>/procs`, `title`, `audita.txt` (processing
audit trail [detail unverified]). Acquisition and processing parameters (SFO1, SW, TD, NS,
PULPROG, SOLVENT, TE, probe; SI, LB, phases), pulse program text, title. No public spec; nmrglue
(BSD-3) as reference. Effort S on top of the JCAMP parser. **W2**, needs folder input (section 9).
Varian/Agilent VnmrJ `procpar` (typed parameter lines [layout unverified]) is S, **later**.

**mzML (HUPO-PSI).** Root `<mzML xmlns="http://psi.hupo.org/ms/mzml"` or `<indexedmzML`. The
open MS exchange format written by ProteoWizard msconvert; PRIDE/ProteomeXchange use it
[requirement unverified]. Every metadata item is a cvParam (accession, name, value, unit):
instrument model and serial, components, the software chain (provenance edges), data processing,
source file SHA-1, per-spectrum MS level, scan time, polarity, precursor window; arrays base64.
The PSI-MS CV is CC BY 4.0 ("This work is licensed under the Creative Commons Attribution 4.0
International License") and could be bundled with attribution to resolve terms offline.
`mzdata` 0.67.4 (Apache-2.0, 87k downloads, mature) or an own quick-xml metadata reader. Probe
base64/zlib bombs. Output metadata plus a per-spectrum summary table, never peak lists. **W2.**
mzXML (legacy, no CV) is **later**.

**Thermo RAW (.raw).** Bytes `01 A1` then "Finnigan" in UTF-16LE. Written by every Thermo
Orbitrap, Exploris, Astral and TSQ; dominant in proteomics and metabolomics deposits. Holds the
sequence row, embedded instrument method text, instrument model and serial, logs, per-scan
trailer parameters. The official RawFileReader EULA (thermofisherlsms/RawFileReader License.doc,
fetched 2026-10-19): 2.1 "You may not decompile, disassemble, reverse engineer, use reflection or
modify this Software."; 4.2 "You may not commercially exploit this Software or products that
incorporate this Software without the prior written consent of Licensor."; 4.3 forbids
distribution "with any software that is subject to the General Public License (GPL) or similar
license in a manner that would create a combined work". It also needs .NET 8 in-process
(`thermorawfilereader` 0.8.0 wraps it): **out of scope**. Pure-Rust routes are days old
(`thermorawfile` 0.1.1 MIT OR Apache-2.0; `openreadout-thermo`, whose prior art includes the
CC-BY-SA OpenTFRaw spec). **Skip for now:** detect, report model and version from the header,
point to mzML conversion or OpenReadout.

**CIF and mmCIF / PDBx (.cif, .mmcif, .bcif).** First token `data_<name>` (optionally
`#\#CIF_2.0`); mmCIF has `_atom_site.` and `_entry.id`. **`.cif` collides** with Network Rail CIF
timetables. CIF is "the recommended method for submitting publications to Acta Crystallographica
Section C" and the format of CSD and COD; PDBx/mmCIF is the only format for new PDB entries
("all deposits are processed and distributed as PDBx/mmCIF"), and four-character PDB IDs run out
"before 2028", after which extended IDs have no legacy PDB file (wwPDB FAQ). Key-value items and
loops (tables), cell parameters with standard uncertainties (`7.8783(2)`), space group,
authors, citation, refinement statistics, entities, sequences, `struct_conn` bonds, software,
audit revision history (timeline). IUCr policy: the IP rights "relate solely to the interchange
formats, not to the data contained therein, nor to the software", and software "claiming to
read" CIF "must be able to extract the pertinent data from a file conformant to" the syntax.
Crates: `crystal-cif-io` 0.2.0 (MIT, CIF 1.1 and 2.0, very new), `pdbtbx` 0.12.0 (MIT; "many
metadata features ... are unsupported"); `ustar-parser` and `cif_chomper_core` are LGPL
(excluded); gemmi is MPL-2.0 C++. A generic own STAR tokenizer (about 400 lines) gives all
categories, which fits Rezzer better than atom-focused crates. Effort M. **W2.** Legacy PDB
format (80 columns, frozen since 2012 at v3.30; collides with MS debug `.pdb`) is S, **later**.

**GenBank / EMBL flat files.** GenBank starts `LOCUS`; EMBL `ID   ` with `XX` separators.
NCBI, ENA, plasmid tools (SnapGene and Benchling export GenBank [unverified]). LOCUS (length,
molecule, topology, date), ACCESSION.VERSION (revision), taxonomy, references with PubMed ids,
and the FEATURES table with locations and qualifiers (gene, product, db_xref): properties plus
gene-CDS relations. `gb-io` 0.9.0 (MIT, 70k downloads); no Rust EMBL parser. S for GenBank, M
for EMBL. **W2.** **FASTA/FASTQ** (`>` / `@` with `+`): little knowledge beyond ids and counts;
mature crates (noodles-fasta 0.68.0, needletail 0.7.3, bio 4.2.2, all MIT); summary-only
streaming reader with a cap, **later**.

#### Images and analysis tools

**ImageJ TIFF metadata.** ImageDescription (tag 270) starting `ImageJ=` with `images=`,
`channels=`, `slices=`, `frames=`, `unit=`, `spacing=`, `finterval=`; private tags 50838/50839
with Info text, slice labels, ranges, LUTs, ROIs [keys from tifffile behaviour, unverified].
Fiji/ImageJ is the default open microscopy tool. Dimensions, physical calibration, labels, a free
"Info" dump of instrument metadata, ROIs as positioned annotations. `tiff` 0.11.3 gives raw tags.
S. **W2**, as the metadata section of a generic TIFF reader (pixels to the vision list), shared
with OME-TIFF (section 8).

**IDL SAVE (.sav).** Magic `SR` then `\x00\x04` (or `\x00\x06` compressed). **Name collision
with SPSS `.sav`** (`$FL2`/`$FL3` [unverified]): detect by magic only. Astronomy, solar and space
physics (SolarSoft), remote sensing. Timestamp, version, identification records and named
variables with types, dimensions and nested structures. Only an unofficial spec (Craig
Markwardt, "Unofficial Format Specification of the IDL SAVE File", 2011; he notes RSI never
sanctioned it). scipy.io.readsav (BSD-3) as reference. Effort M. **Later.**

**Origin .opj / .opju.** Proprietary binary (`.opj` magic `CPYA` [unverified]). Worksheets with
column formulas, notes, graphs, analysis recalculation links: high potential value. No spec; the
only reader, liborigin, is GPL-3.0-or-later (license only, never read) and reads `.opj` up to
2017, not `.opju`. **Blocked: skip.** Detect and say "export worksheets to CSV/XLSX and graphs to
SVG/PDF from Origin".

#### Ranking (7.2)

1. Jupyter `.ipynb`; 2. the citation family; 3. LaTeX sources; 4. SBML; 5. `.eln`; 6. JCAMP-DX
plus Bruker parameters; 7. CIF/mmCIF and GenBank; 8. mzML metadata; 9. CellML, ImageJ TIFF.
Later: mzXML, PDB legacy, FASTA/FASTQ summaries, Varian, IDL `.sav`. Skip with detection: Thermo
RAW, Origin, EndNote `.enl`.

## 8. Machines and lab equipment

For each format the openness column says one of: **open spec**, **vendor-documented**,
**reverse-engineered only**, or **vendor SDK** (out of scope for Rezzer, which runs no external
programs and links no vendor runtimes).

### OpenReadout: a direct peer

[openreadout/openreadout](https://github.com/openreadout/openreadout) is a Rust project that
reads lab-instrument files for AI agents. Its description (checked with `gh api` on 2026-10-19):
"OpenReadout is an open-source reader purpose-built for AI agents to work with raw
lab-instrument files. Free, single binary, no vendor software required." Repo license
Apache-2.0, created 2026-10-02, pushed 2026-10-19, 2 stars, one main author. It claims 132
instrument formats (85 high, 36 medium, 11 low confidence) with a "known gaps" list per format,
an MCP server with 28 tools, 180 fuzz targets, and a clean-room policy
(`docs/legal/clean-room-policy.md`): GPL/LGPL readers (rainbow, Bio-Formats, libCZI, readlif,
chromConverter) are only run as oracles, their source is never read, and each format has a
provenance log. Its `corpus/manifest.toml` (about 90k lines) lists URL, license and SHA for each
file and is the best sample source in this area. Crates `openreadout-chrom`, `-qpcr`, `-plate`,
`-lif`, `-czi`, `-tiff`, `-spectro`, `-mzml`, `-nmr` are at 0.4.0 (2026-10-18, 55 to 166
downloads each), MIT OR Apache-2.0, with light dependencies (roxmltree, miniz_oxide,
serde_json). Its output is JSON of signals and metadata; it does not do OKF Markdown, graphs or
provenance in Rezzer's sense.

It covers ANDI, ChemStation, OpenLab CDS, Waters, Shimadzu, Thermo RAW, TDMS, RDML, plate
readers, CZI, LIF, ND2, OME-TIFF, mzML/mzXML, JCAMP-DX, Bruker and Varian NMR. It does not cover
oscilloscopes, logic analyzers, STDF, ATML, DMIS, CMM exports, CNC, laser jobs, AnIML, ASM or ADF.

Flag: its Thermo RAW reader lists the OpenTFRaw spec (CC-BY-SA) and the unfinnigan wiki as prior
art; read `docs/provenance/thermo-raw.md` before depending on that crate.

**Consequence for Rezzer:** do not re-derive vendor lab binaries from scratch. Options (decision
for Jordi, section 10): depend on `openreadout-*` behind Rezzer's own trait after a hostile-input
probe and cargo-deny check (17 days old, one maintainer, heavy AI assistance), use its
Apache-2.0 `docs/formats/*.md` as permissive documentation for own readers, or position Rezzer as
the knowledge layer and point users to OpenReadout for vendor binaries.

### Summary

| Format | Users | Structure | Openness | Rust crate | Samples | Effort | P/G/T/F/Tb | Rec |
|---|---|---|---|---|---|---|---|---|
| ANDI chromatography / MS netCDF (ASTM E1947/E2077) | every CDS vendor, repositories | netCDF-3 binary | ASTM paid; self-describing | netcdf3 0.6.1, netcdf-reader 0.9.2 (pure Rust) | 1 (MIT) | S | 3/1/2/1/3 | W1 |
| NI TDMS | LabVIEW, DAQmx, test stands | segmented binary | vendor-documented, free | tdms 0.3.0, tedium 0.2.0, tdms-rs 2.0.0 | 1 (CC-BY-4.0) | M | 3/2/3/2/3 | W1 |
| RDML (qPCR) | most qPCR software | zip + XML | open spec, MIT | rdml-qpcr 0.1.1 (young) | 1 (MIT) | S | 3/2/2/1/3 | W1 |
| Allotrope ASM JSON | allotropy, TetraScience, LIMS | JSON | schemas CC-BY-NC / CC-BY-ND / members | serde_json | allotropy MIT (candidate) | S | 3/2/1/2/3 | W1 |
| STDF V4 | semiconductor ATE, OSATs | binary records | open spec (Teradyne) | rust-stdf 1.1.0 (MIT) | 1 (MIT) | S | 3/1/2/0/3 | W2 |
| DMIS (ISO 22093) | all CMM software | statement language | free after form | none | none | M | 3/3/1/1/3 | W2 |
| Heidenhain Klartext .h + tables | TNC controls (strong in DE) | text | manuals public | none | none | S-M | 3/2/0/3/3 | W2 |
| Tektronix .isf / .wfm | Tek scopes | text+binary / binary | vendor-documented (personal-use terms) | none | RigolWFM BSD-3 (candidate) | M | 3/0/3/2/2 | W2 |
| LeCroy .trc | LeCroy scopes | binary | vendor-documented | none | RigolWFM | S-M | 3/0/3/2/2 | W2 |
| Keysight .bin / .h5 | Infiniium scopes | binary / HDF5 | vendor-documented | none / HDF5 | RigolWFM | S / M | 3/0/3/2/2 | W2 / later |
| AnIML | BSSN/Merck, some LIMS (weak) | XML | XSD on GitHub without license | quick-xml | none clean | S-M | 3/2/3/1/3 | W2 |
| OME-TIFF / OME-XML | microscopy | TIFF + XML | open spec (BSD-2), samples CC-BY-4.0 | tiff 0.11.3, ome-metadata 0.6.1 | OME (candidate) | S | 3/2/1/0/2 | W2 |
| NI DIAdem .dat / .tdm | automotive testing | text+binary / XML+binary | partly documented | none | none | S / M | 3/2/3/2/2 | W2 |
| Saleae export CSV, .sal meta.json | embedded engineers | zip JSON+binary; CSV | export documented, .sal not | saleae-importer 1.0.1 | none | S / M | 3/1/3/0/3 | W2 (meta, CSV) |
| sigrok .sr | PulseView | zip INI + binary | open spec (wiki) | none (crate is GPL) | none | S | 2/0/2/0/1 | W2 (meta) |
| ChemStation .D / OpenLab .dx | Agilent GC/LC | text + binary; OPC zip | reverse-engineered only | entab 0.3.3 (MIT), openreadout-chrom | entab MIT | S meta / L | 3/2/2/1/3 | W2 (text meta) |
| Waters MassLynx .raw folder | Waters MS | text + binary | vendor SDK / RE | openreadout (repo) | MetaboLights | S / L | 3/1/1/2/2 | later |
| Shimadzu .lcd/.gcd | Shimadzu LC/GC | CFB + vendor streams | reverse-engineered only | openreadout-chrom | Zenodo CC-BY | L | 3/1/2/1/3 | later |
| Rigol .wfm/.bin | low-cost scopes | binary, many variants | reverse-engineered only | none | RigolWFM | L | 2/0/3/2/1 | later |
| Allotrope ADF | Allotrope members | HDF5 + RDF | members only [unverified] | hdf5-pure, hdf5-reader | none | L / blocked | 3/3/2/1/2 | later |
| ATML / IEEE 1636.1 | defense ATE, TestStand | XML | paid; XSD personal use only | none | none | S-M | 3/2/2/1/3 | later |
| ASAM ODS / ATFX | automotive test | XML + binary | ASAM members | none | none | M | 2/2/2/1/2 | later |
| Zeiss Calypso tables | Zeiss CMM (DE) | TSV | undocumented | none | none | S | 3/1/1/0/3 | later |
| MTConnect streams (delta) | US machine shops | XML / SHDR | Apache XSD | none | cppagent | S | 2/1/3/0/3 | later |
| LightBurn .lbrn2 | laser shops | XML | undocumented | none | none | S | 3/1/0/0/3 | later |
| Zeiss CZI, Leica LIF, Nikon ND2 (metadata) | microscopy | binary + XML | confidential / none / SDK | czi-rs 0.2.0, openreadout-lif, nd2 1.0.0 | OpenReadout manifest | S-M | 3/1/1/0/2 | later |
| ASTM E1394 (LIS2-A2), SiLA 2 FDL | clinical analyzers, lab automation | text / XML | E1394 paid; SiLA MIT | none | none | S | 3/2/1/0/2 | later |
| Fanuc .nc | Fanuc CNC | text | ISO paid, manuals closed | gcode 0.7.0 | CAD spike | S delta | 2/2/0/2/1 | covered + W2 delta |
| Prusa .bgcode | Prusa printers | binary blocks | spec in AGPL repo | binarygcode (flag) | CAD spike | - | - | covered |
| PLC DataLog CSV, plate reader exports | everyone | CSV/XLSX | none | csv, calamine | - | - | - | covered (dialect hints later) |
| Thermo RAW | Thermo MS | binary | vendor SDK (EULA, .NET) | see section 7 | - | - | - | skip (section 7) |
| PC-DMIS .PRG, RDWorks .rd, balance printouts | - | binary / scrambled / text | vendor only / RE / text | - | - | - | - | skip |

### Chromatography and spectroscopy

**AIA/ANDI netCDF (ASTM E1947 chromatography, E2077 MS; `.cdf`).** netCDF classic magic
`CDF\x01`/`CDF\x02`; chromatography files carry the global attribute `aia_template_revision` or
variable `ordinate_values`, MS files `ms_template_revision` or `mass_values`. **Name collision**
with NASA CDF (`CD F3 00 01`), ASAM CDF (XML) and any other netCDF. The vendor-neutral export of
Agilent ChemStation/OpenLab, Thermo Chromeleon/Xcalibur, Waters Empower/MassLynx, Shimadzu
LabSolutions and PerkinElmer TotalChrom; found in MetaboLights deposits (MTBLS758, MTBLS1892)
and 35 OpenReadout corpus files. Holds sample (name, id, type, amount, injection volume),
methods, operator, detector name and unit, sampling interval, injection timestamp, the peak
table (retention time, area, height, amount, name), the series, and `source_file_reference` to
the original vendor file. ASTM text is paid, but netCDF is self-describing, so the variable
names come from the file. Pure Rust: `netcdf3` 0.6.1 (MIT OR Apache-2.0, 59k downloads),
`netcdf-reader` 0.9.2 (classic plus netCDF-4, young); avoid the C-binding `netcdf` crate. Own
classic reader about 300 lines. References: scipy `netcdf_file` (BSD-3), cheminfo/netcdf-gcms
(MIT). Effort S. **W1:** the universal chromatography export; peak tables map straight to
Markdown tables. Shares the netCDF classic reader with section 7.

**Allotrope Simple Model (ASM) JSON.** JSON with a manifest key (`$asm.manifest`, unverified on
a sample). Benchling's allotropy (MIT, pushed 2026-10-15) converts many instrument text and
Excel exports to ASM; LIMS/SDMS vendors such as TetraScience produce it. Device and measurement
documents, sample documents, processed data, peak lists, calculated data with data-source links
(a graph). License (ASM repo LICENSE.md, primary): "The ASM is collectively licensed under three
licenses": CC-BY-NC 4.0 for non-commercial use, "CC-BY-ND 4.0. This license permits commercial
use, but restricts modifications" for commercial use, or the Allotrope Commercial License for
members. Reading instances needs no schema: **do not bundle or modify the schemas**. Effort S.
**W1** as a JSON dialect with semantic mapping.

**Allotrope Data Format (ADF, `.adf`).** HDF5 with an RDF quad store and data cubes. Collides
with Amiga disk images and ArcInfo grids. Agilent OpenLab offers ADF export (Agilent 5994-0886EN);
Allotrope now focuses on ASM. Spec and libraries appear to be members only [unverified; the
membership page returned 404]. **Later:** detect HDF5 + `.adf` and say "export ASM JSON".

**AnIML (ASTM E13.15).** XML root `AnIML`, namespace `urn:org:astm:animl:schema:core:draft:0.90`
(still a draft namespace after about 20 years [ASTM status not confirmed]). SampleSet,
ExperimentStepSet with technique, method, results, series with units, audit trail and XML-DSig
signatures. Schemas on [AnIML/schemas](https://github.com/AnIML/schemas) have **no license
file**: read files, do not bundle the XSD. Weak evidence of current uptake. Effort S-M. **W2.**

**Agilent ChemStation / OpenLab `.D` folders and OpenLab CDS `.dx`.** A `.D` directory holds
`*.ch`, `*.uv`, `*.ms`, `RUN.LOG`, `acqmeth.txt`, `SAMPLE.XML` and a method folder; `.dx` is an
OPC zip with ACAML XML (`injection.acmd`). Collisions: MassHunter `.d`, Bruker timsTOF `.d`
(SQLite inside), D source; `.dx` vs JCAMP-DX. Reverse-engineered only. Text and XML metadata
(sample, operator, instrument, method, timestamps, sequence) are S in-house; binary traces via
`entab` 0.3.3 (MIT) or `openreadout-chrom` after a probe. rainbow-api is LGPL-3.0 (license only).
**W2** for the text metadata; needs folder input (section 9).

**Waters MassLynx `.raw` folders** (`_HEADER.TXT` with `$$ Key: value`, `_FUNCnnn.DAT`,
`_extern.inf`) and **Shimadzu `.lcd/.gcd`** (CFB with streams `LC Raw Data`, `File Property`,
`Audit Trail`): text headers and calibration lines are S; binaries are L or vendor SDK.
**Later**; for Shimadzu list the CFB stream names and say what is not decoded.

### Oscilloscopes and logic analyzers

**Tektronix `.isf` and `.wfm`.** `.isf`: ASCII preamble `:WFMPRE:` or `:WFMOUTPRE:` (BYT_NR,
ENCDG, XINCR, XZERO, YMULT, YOFF, YZERO, WFID) then `:CURVE #<n><len>` binary. `.wfm`: byte order
`0F 0F` or `F0 F0`, then `:WFM#001/2/3`. Collides with Rigol and Tek AWG `.wfm`. Holds channel,
scales, offsets, units and the conversion `Y = (raw - YOFF) * YMULT + YZERO`, record length,
trigger position, timestamp, FastFrame frames. Vendor-documented in "Reference Waveform File
Format" 077-0220-xx (free download); the Tek manuals agreement allows "internal or personal use"
and says manuals "may not be reproduced for distribution": implementing from it is fine, copying
it into the repo is not. Permissive prior art: RigolWFM (BSD-3, active) has Kaitai `.ksy` specs
for Tek, Agilent, LeCroy, Rigol, Siglent, R&S and Yokogawa. Effort M. **W2.**

**LeCroy `.trc`** (`WAVEDESC` with template `LECROY_2_3`; gain, offset, interval give
`V = gain * adc - offset`, `t = hoffset + i * dt`; documented through the scope's `TEMPLATE?`
query; leCroyParser MIT) S-M, **W2**. **Keysight `.bin`** (cookie `AG`, documented in Infiniium
help) S, **W2**; `.h5` later with HDF5. **Rigol** variants are reverse-engineered only, L,
**later**.

**Saleae.** `.sal` is a zip with `meta.json` plus binary channels; Saleae advises "not using
those files for any 3rd party tools, since the format is subject to change" (not a legal block).
The CSV and binary exports are documented. `saleae-importer` 1.0.1 (MIT). **W2** for
`meta.json` and analyzer CSV (decoded protocol frames), binaries later.

**sigrok `.sr`.** Zip with `version`, INI-style `metadata` (samplerate, probe names, unitsize)
and packed chunks; IANA `application/vnd.sigrok.session`; open wiki spec. libsigrok and the
`sigrok` crate are GPL-3.0 (excluded). S, **W2** for metadata and channel list.

### Test and measurement

**NI TDMS.** Segment tag `TDSm` (index file `TDSh`), version 4712/4713. Written by LabVIEW,
DAQmx, SignalExpress, FlexLogger, VeriStand and DIAdem across industry and academia. Holds a
file/group/channel hierarchy with properties on every level: names, `unit_string`,
`wf_increment`, `wf_start_time`, `NI_Scaling_*` (linear, polynomial, thermocouple: formulas),
author, title, description, plus the series. NI publishes "TDMS File Format Internal Structure"
freely. Crates: `tdms` 0.3.0 (MIT, 2023, stale), `tedium` 0.2.0 (MIT OR Apache-2.0, 2026-01, by a
LabVIEW consultancy), `tdms-rs` 2.0.0; npTDMS is LGPL-3.0 (license only). Own reader M (about 600
lines); probe segment lengths and DAQmx raw data. One CC-BY-4.0 sample. **W1:** the most common
measurement container in industrial test stands.

**NI DIAdem `.dat` / `.tdm`+`.tdx`.** `.dat` text header with `DIAEXTENDED`,
`#BEGINGLOBALHEADER` and numbered keys (200 name, 202 unit, 220 length, scaling keys
[mapping unverified]); `.tdm` is XML plus binary `.tdx`. Partly documented. S/M, **W2**.

**STDF V4 (semiconductor test).** First record FAR: `00 02 00 0A 01 04` (big-endian) or
`02 00 00 0A 02 04`; often gzipped. Every ATE vendor (Teradyne, Advantest) and OSAT writes it.
MIR (lot, part, tester, program, operator, dates), sites, parametric and functional test results
with limits and units, part and bin results, test statistics, wafer records. Open spec
(Teradyne made it public; widely mirrored, no Teradyne-hosted copy found). `rust-stdf` 1.1.0
(MIT, 2026-08-22, active, V4 and V4-2007). Big-endian MIT sample with 52k PTR records. Effort S.
**W2**: clean spec, maintained crate, narrow but deep industry.

**ATML / IEEE 1671 and IEEE 1636.1 test results.** XML; US defense ATE and NI TestStand. The
XSDs are free "for personal use" with "you may not further copy ... distribute copies of the
Materials ... without prior written permission from IEEE": do not bundle. S-M, **later**.
**ASAM ODS / ATFX**: ASAM members only, **later**.

### Metrology and CNC

**DMIS (ANSI/DMIS 105.3, ISO 22093).** Text statements starting `DMISMN/'name',05.3`; extensions
`.dmi`, `.dms`, `.dmo` [unverified list]. Written and read by Calypso, PC-DMIS, Metrolog, CAPPS,
Modus, PolyWorks. Feature definitions `F()` with nominals, tolerances `T()`, datums `D()`: a
feature-tolerance-datum graph; output files carry actuals and deviations. DMSC offers DMIS 5.3
free after a form (qifstandards.org), a separate page lists it at USD 125 (conflict). Effort M.
**W2**, sibling of QIF (engineering report).

**Heidenhain Klartext (`.h`) and tables (`.t`, `.tch`, `.tab`, `.d`, `.p`).** Programs start
`0 BEGIN PGM <name> MM` and end `END PGM <name> MM`; tables `BEGIN <name>.T MM`, a fixed-width
header, rows, `[END]` [table syntax unverified]. **`.h` collides with C/C++ headers**: sniff the
first line. Heidenhain TNC 640/530 controls are very common in German machine shops. Tool calls
(`TOOL CALL 5 Z S3000 F200`), cycles with Q parameters, Q-parameter formulas (`FN`, `Q1 = ...`),
labels and subprogram calls (a call graph), tool table geometry and wear. User manuals are public
PDFs. Effort S-M. **W2** (high DE relevance).

**Fanuc programs (`.nc`).** `%`, `O1234 (TITLE)`, Macro B (`#100`, `G65 P`, `IF [...] GOTO`,
`WHILE ... DO`). **`.nc` collides with netCDF**: sniff `CDF`/HDF5 first. Covered by the CAD
spike's G-code reader; a small **W2 delta** adds the O-number, macro formulas, subprogram call
graph and the sniff. ISO 6983 is paid, Fanuc manuals are closed. `chipbreaker-gcode` is GPL-3.0
(excluded).

**Zeiss Calypso table files** (`*_chr.txt`, `*_hdr.txt`, `*_fet.txt`, TSV; characteristics with
nominal, actual, tolerance, deviation): undocumented, S, **later** as a TSV dialect.
**Q-DAS DFQ (AQDEF K-fields)** is very common in German automotive metrology next to Calypso and
PC-DMIS and was not researched: follow-up. **PC-DMIS `.PRG`** is proprietary: **skip**, say
"export DMIS, QIF or DFQ". **MTConnect streams** add timestamped Samples, Events and Conditions
(alarms with native code and severity) to the covered device XML: S, **later**.

**Laser and 3D printing.** LightBurn `.lbrn2` is plain XML (`LightBurnProject`) with per-layer
cut settings (speed, power, passes): S, **later**. RDWorks/Ruida `.rd` is scrambled binary,
documented only in a GPL-2.0 project: **skip**, say "export DXF/SVG". Prusa `.bgcode` is covered
by the CAD spike; its prose spec lives in the AGPL-3.0 libbgcode repo, and the MIT crate
`binarygcode` calls itself "A Rust implementation of libbgcode": check provenance before use.

### Microscopy (metadata only; pixels go to the vision path)

**OME-TIFF / OME-XML.** TIFF magic with the first ImageDescription starting `<?xml` in namespace
`http://www.openmicroscopy.org/Schemas/OME/2016-06`; `.ome.tif(f)`, `.companion.ome`. The open
microscopy export of the OME and Bio-Formats ecosystem. Instrument (objectives, detectors,
lasers), Pixels (physical sizes with units, channels, planes with timestamps and positions),
annotations, ROIs, plates and wells. ome-model is BSD-2-Clause, OME sample images CC-BY-4.0.
`tiff` 0.11.3 (MIT, 136M downloads), `ome-metadata` 0.6.1 (MIT OR Apache-2.0). S. **W2**, shared
with the ImageJ TIFF metadata of section 7.

**Zeiss CZI** (`ZISRAWFILE`, metadata segment XML; spec confidential under license [secondary];
libCZI is "LGPL Version 3 as well as Proprietary/Commercial", license only; `czi-rs` 0.2.0 MIT OR
Apache-2.0), **Leica LIF** (UTF-16 XML header, no public spec, readlif GPL-3.0), **Nikon ND2**
(SDK licensed; Python nd2 BSD-3 as prior art; `nd2` 1.0.0 crate days old): metadata S-M,
**later**. The `bioformats` crate 0.1.8 is GPL-2.0-or-later ("translation of Bio-Formats"):
excluded.

### Lab data exchange

**RDML (qPCR).** Zip `.rdml` with `rdml_data.xml` or plain XML, root
`<rdml xmlns="http://www.rdml.org" version="1.x">`; LightCycler 96 `.lc96p` is RDML with extras.
Exported by Bio-Rad CFX, Roche LightCycler, Thermo/ABI StepOne and QuantStudio (MIQE context).
Experimenters, dyes, samples, targets with primers and efficiency, thermal cycling programs,
runs, reactions, amplification and melt data, Cq values and exclusions. Open spec (RDML project
MIT, rdmlpython MIT, R package RDML MIT). `rdml-qpcr` 0.1.1 is young (the crate `rdml` is
unrelated). MIT sample from a StepOne. Effort S. **W1.**

**LIMS and lab integration.** ASTM E1394 / CLSI LIS2-A2 text records (`H|\^&|...`, P, O, R, L)
from clinical analyzers: S, **later**. ASTM E1578 is a LIMS guide, not a format. SiLA 2 Feature
Definition XML (`*.sila.xml`, root `Feature`; sila_base MIT) describes commands, parameters,
units and constraints: S, **later**. HL7 v2 is in section 3. **Plate reader exports** have no
standard (vendor CSV/TXT/XLSX); allotropy and openreadout-plate cover the dialects: **skip** as a
format, tables are covered, ASM is the target. **PLC DataLog CSV** (record-number column plus
header) and **balance printouts** (Mettler Toledo MT-SICS, Sartorius GLP text) are covered by
the CSV and plain-text readers.

### Area ranking

1. NI TDMS (W1): most widespread test-stand format, free spec, properties with units and
   scaling, group/channel graph, time series.
2. ANDI netCDF (W1): universal chromatography export, pure-Rust netCDF-3, peak tables.
3. RDML (W1): open MIT spec, complete experiment model, S.
4. ASM JSON (W1): Allotrope's growing target; pure JSON; only the schemas are restricted.
5. STDF (W2), 6. DMIS and Heidenhain (W2), 7. Tek/LeCroy/Keysight waveform headers (W2),
8. AnIML, OME-XML, DIAdem, Saleae/sigrok metadata, ChemStation text metadata (W2).
Later: vendor binaries (Waters, Shimadzu, Rigol, CZI, LIF, ND2), ADF, ATML, MTConnect streams,
LightBurn, Calypso, SiLA, E1394. Skip: PC-DMIS `.PRG`, RDWorks `.rd`, balance printouts and plate
reader exports as own formats.

### Open questions / unverified

- OpenReadout: depend, document from, or point to (section 10).
- ADF terms for non-members; AnIML publication status and XSD license.
- Q-DAS DFQ/AQDEF openness (not researched).
- Heidenhain table syntax and Fanuc parameter backups: confirm on public manuals; no licensed CNC
  samples yet.
- RigolWFM waveform files: confirm the repo BSD-3 covers the user-contributed captures.

## 9. Findings across areas

### 9.1 One signal and formula model

About twenty formats describe signals the same way: a raw value, an encoding, a conversion to a
physical value, a unit, limits, a producer and consumers. Rezzer should model this once and
render it once (a signal table plus a Mermaid producer-consumer graph plus formulas):

| Conversion kind | Formats |
|---|---|
| linear `a*x + b` | DBC, A2L `LINEAR`, COMTRADE, FMI units, TDMS linear scaling, FITS `BSCALE`/`BZERO`, NetCDF `scale_factor`/`add_offset`, Excel ICDs, ARINC 429 LSB, HMI tag scaling |
| rational / polynomial | A2L `RAT_FUNC`, ARXML COMPU-SCALEs, MDF conversions, XTCE PolynomialCalibrator, SCOS-2000 `mcf`, TDMS polynomial scaling, SPICE PCK pole polynomials |
| table / spline / enum | A2L `TAB_INTP`/`TAB_VERB`, DBC `VAL_`, XTCE SplineCalibrator, SCOS-2000 `caf`/`txf`, MDF value-to-text |
| free formula / RPN | A2L `FORM`, MDF algebraic, XTCE MathOperation (RPN), Modelica equations |
| content MathML | SBML kinetic laws and rules, CellML equations |
| transfer functions | StationXML poles and zeros, FIR, polynomial stages |
| power-of-ten scale factor | SunSpec `value * 10^sf`, GRIB `(R + X*2^E)/10^D` |

A shared formula renderer (infix text plus LaTeX) and a content-MathML converter are S to M
modules that pay off in every wave.

### 9.2 Multi-file and folder inputs

Many formats only make sense as a set: A2L with HEX/S19 and `/include`, multi-file ARXML, PDX,
`.xosc` referencing `.xodr`, COMTRADE `.cfg` + `.dat`, CGMES `DependentOn` profiles, SPICE
meta-kernels, LaTeX with `.bib`, Shapefile sets, Bruker experiment folders, Agilent `.D`, Waters
`.raw`, SCOS-2000 MIB folders, Zarr stores, EndNote `.Data`. Rezzer's `Source` needs a "set" or
"folder" form, and the output must list members that are referenced but missing (the A2L and
CGMES samples test exactly this). This is a design decision for sub-project 1 (section 10).

### 9.3 Detection collisions (content beats extension)

| Extension | Formats sharing it | How to tell |
|---|---|---|
| `.cdf` | NASA CDF, ASAM CDF, ANDI netCDF, Wolfram CDF, MS Channel Definition | `CD F3 00 01` [from cdf-rs, unverified] / `CDF\x01` + `aia_template_revision` / XML `MSRSW` [unverified] |
| `.mdf` | ASAM MDF, MS SQL data file, Alcohol image | `MDF     ` or `UnFinMF ` at 0 |
| `.dcm` | DAMOS DCM, DICOM | `KONSERVIERUNG_FORMAT`/`FESTWERT` vs `DICM` at 128 |
| `.ldf` | LIN LDF, MS SQL log | `LIN_description_file;` |
| `.eds` | ODVA EDS, CANopen EDS, CCSDS SEDS | INI `[File]` vs XML SEDS namespace [unverified] |
| `.sav` | IDL SAVE, SPSS | `SR\x00\x04` vs `$FL2` [unverified] |
| `.nc` | netCDF, CNC programs | `CDF` / HDF5 magic vs `%` + `O1234` |
| `.mo` | Modelica, gettext | text keywords vs `0x950412DE` |
| `.mdl` | Simulink, Rational Rose, NVIDIA MDL, game models | `Model {` / `__MWOPC_PACKAGE_BEGIN__` |
| `.m` | MATLAB, MATPOWER, Objective-C, Mathematica | `function mpc` / `#import` |
| `.h` | Heidenhain Klartext, C header | `0 BEGIN PGM` |
| `.pdx` | ASAM PDX, IPC-2571 PDX, Acrobat catalog | zip with `index.xml` vs XML vs binary |
| `.raw` | Thermo RAW, Waters folder, PSS/E RAW, camera RAW | `01 A1` + "Finnigan", folder, text first line |
| `.d`, `.D` | Agilent ChemStation, MassHunter, Bruker timsTOF | folder contents |
| `.dx` | JCAMP-DX, Agilent OpenLab CDS | `##TITLE=` vs OPC zip |
| `.tdm` | NI DIAdem TDM, CCSDS TDM | XML root vs `CCSDS_TDM_VERS` |
| `.ssd` | SSP, IEC 61850 SSD | namespace |
| `.cif` | crystallographic CIF, Network Rail CIF | `data_` |
| `.pdb` | Protein Data Bank, MS debug symbols | `HEADER`/`ATOM` vs `Microsoft C/C++ MSF` |
| `.dbf` | dBase, Oracle datafile | header arithmetic |
| `.tf`, `.tls` | SPICE kernels, Terraform | `KPL/` (libmagic says LaTeX) |
| `.cfg`, `.dat`, `.log`, `.hex`, `.bin`, `.xml`, `.json` | many | pair by stem, root element, shape sniff |

libmagic also misreads Modelica (HTML) and SPICE kernels (LaTeX), so Rezzer's own detector
(feat/detect on the current branch) must not delegate to it.

### 9.4 Peers and overlaps

- **OpenReadout** (lab instruments, Rust, MCP, Apache-2.0 / MIT OR Apache-2.0): see section 8.
- **cimoxide-mcp** (CGMES as an MCP server, Apache-2.0, months old).
- **mw-binding-xmi** (SysML v1 XMI into OKF, AGPL-3.0: never use or read).
- Python ecosystems (cantools, asammdf, ObsPy, astropy, allotropy) are the reference standard of
  correctness; only the permissive ones may be read.

### 9.5 Shared container decisions

- **HDF5:** NetCDF-4, MATLAB 7.3, Keysight `.h5`, Allotrope ADF, IHO S-102/104/111, NWB, NeXus.
  Pure Rust `hdf5-reader` (with `netcdf-reader`) vs `hdf5-pure`; both 2026 projects. libhdf5 is
  BSD-3 if a C fallback is ever accepted. External links and external data files must be refused.
- **netCDF classic:** one S reader serves NetCDF classic and ANDI chromatography.
- **TIFF metadata:** OME-TIFF and ImageJ in one reader; pixels go to the vision list.
- **dBase:** Shapefile, legacy FoxPro, maybe STEP 7 projects.
- **candump:** NMEA 2000 and automotive CAN/J1939 logs.

### 9.6 Never execute

Jupyter notebooks, FMU binaries (`fmu-runner`, `libfmi`), LaTeX (no tectonic or any TeX engine),
MATPOWER `.m` (parse the matrix literal subset only), Modelica (no simulation), Simulink (no
MATLAB), PLC and CNC programs (read, never interpret macros beyond showing them). Outputs of
notebooks may contain HTML and JavaScript: data only.

### 9.7 Personal data

DICOM and HL7 carry patient data; ELN exports and instrument headers carry operator names and
e-mail addresses. Rezzer needs a rule: redact by default (DICOM PS3.15 profiles as the model) or
flag sensitive fields. Corpus samples for these must be synthetic.

### 9.8 Schemas are read-only knowledge, never bundled

Do not ship ASAM XSDs (including OpenX), IEC 61850 XSD/NSD (IEC CC-EULA, "Unauthorized
modifications are strictly prohibited"), IEEE ATML XSDs ("may not further copy ... distribute"),
Allotrope ASM schemas (CC-BY-NC or CC-BY-ND), railML schemas (CC BY-NC-ND), AnIML and FDSN
schemas (no license). Bundling is fine for MIT/BSD/Apache/CC-BY data with attribution: WMO GRIB
and BUFR tables (MIT), PSI-MS CV (CC BY 4.0), ENTSO-E profiles (Apache-2.0), FMI/SSP XSDs
(BSD-2), AIXM schemas (BSD).

## 10. Recommendation

The W1/W2 labels in sections 2 to 8 rank formats inside their area; this section gives the
overall order across areas, so some area-W1 formats (PDS4, ELN, ASM JSON) land in the overall
second wave.

Rules: open and well specified first, then common (with German and EU weight), then knowledge
density (signals with formulas, graphs, timelines), then low effort through reuse of shared
modules. Formats behind a legal question wait for the owner's decision even when they rank high.

### 10.1 Ranked top 15

| # | Format | Area | Why | Effort |
|---|---|---|---|---|
| 1 | A2L / ASAP2 | automotive | richest signals per byte: COMPU_METHOD formulas, FUNCTION graph, units, curves and maps; mature MIT/Apache crate; clean Apache-2.0 sample | S-M |
| 2 | DBC | automotive | the most common automotive signal file; mature crate (5.4M downloads); factor, offset, units, value tables | S |
| 3 | FMI modelDescription (FMU) | models | the common model exchange format of about 250 tools [count unverified]; open BSD-2 XSD; variables with units and a dependency graph | S |
| 4 | FITS | science | all of astronomy; header cards are trivial to read; HISTORY timeline, WCS formulas, table schemas | S |
| 5 | COMTRADE | energy | every protection relay; channels, units, `a*x + b`, trigger time; S own reader | S |
| 6 | NetCDF classic and 4 + HDF5 metadata (incl. ANDI chromatography) | science, machines | climate, ocean, NASA, synchrotron, chromatography; CF attributes are semantics; one pure-Rust crate family plus a small classic reader | S-M |
| 7 | Jupyter .ipynb | science | huge user base; execution order, errors, outputs; reuses Markdown and vision paths | S |
| 8 | Citation family (BibTeX/BibLaTeX, RIS, EndNote XML/.enw, CSL-JSON) | science | one model, mature permissive crates; feeds the LaTeX `\cite` graph | S |
| 9 | NI TDMS | machines | the common test-stand format; free NI spec; group/channel tree, units, scaling formulas | M |
| 10 | XTCE | space | open standard; calibrators as formulas, container inheritance graph, commands, alarms | M |
| 11 | Simulink SLX with Stateflow | models | de facto model-based design format; port-level block graph, state machines, revisions; clean XML | M |
| 12 | SBML (with the content-MathML renderer) | science | reaction graph plus kinetic laws plus annotations; CC0 BioModels; the renderer serves CellML next | M |
| 13 | LaTeX sources | science | label/ref/cite/include graphs and macro table; static parsing only | M |
| 14 | RDML (qPCR) | machines | open MIT spec, complete experiment model, S | S |
| 15 | CCSDS NDM (OEM/OPM/OMM/CDM, KVN first) and SPICE text kernels | space | free specs, small text formats, ephemeris timelines, frame trees; one S-M package | S-M |

Next in line (W2, roughly in order): ARXML (after the AUTOSAR decision), MDF4 metadata, ELN
`.eln`, Allotrope ASM JSON, PDS4 labels, OpenDRIVE, Excel ICD and trace-ID profiles, COBOL
copybook with EBCDIC, StationXML and miniSEED, GRIB2 inventory, JCAMP-DX with Bruker NMR
parameters, STDF, DMIS, Heidenhain Klartext, waveform headers (Tek, LeCroy, Keysight), DICOM
headers, HL7 v2, NMEA 0183, CGMES, powerio formats, GPX/KML/Shapefile/DBF, SSP, Modelica, CIF,
GenBank, mzML metadata, SCOS-2000 MIB, LDF, DCM, ODX/PDX, OpenSCENARIO XML, A2L + HEX.

### 10.2 Possible releases

| Release | Theme | Formats | Shared modules built here | Rough size |
|---|---|---|---|---|
| R-a | Signals and parameters | DBC, A2L, COMTRADE, FMI, (LDF, DCM) | signal model, formula renderer (linear, rational, table, FORM), signal table + Mermaid producer-consumer graph, "missing include" reporting | 2,500 to 3,500 lines |
| R-b | Science containers | FITS, NetCDF classic/4, HDF5 metadata, ANDI profile, NI TDMS, RDML | HDF5/netCDF behind a trait after a probe, CF and units layer, time-series summary (span, rate, gaps, stats; never a dump) | 3,000 to 4,000 lines plus crate probes |
| R-c | Research authoring | Jupyter, citation family, LaTeX | citation model, cross-file `\cite` resolution (needs the set input of 9.2) | 1,500 to 2,500 lines |
| R-d | Models as graphs and formulas | Simulink SLX + Stateflow, XTCE, SBML, CCSDS NDM, SPICE text kernels | content-MathML converter, state machine to Mermaid, container inheritance graph | 3,500 to 5,000 lines |
| R-e (W2) | Second wave by demand | ARXML, MDF4, ELN, ASM, PDS4, OpenDRIVE, ICD profile, COBOL + EBCDIC, StationXML, GRIB2, JCAMP-DX, STDF, DMIS, Heidenhain, DICOM, HL7 v2 | multi-file/folder `Source`, PII rule, EBCDIC tables | per format |

R-a and R-b only need dependencies Rezzer already plans (quick-xml, zip, calamine) plus a2lfile,
can-dbc and one HDF5/netCDF family. Size estimates come from the per-format effort values and
are rough.

### 10.3 Licensing blockers (clear list)

**Blocked until the owner decides:**

- **railML:** schemas CC BY-NC-ND; registration required even for research implementations;
  certification "mandatory before any productive or commercial use"; examples NC-ND.
- **AUTOSAR ARXML:** "The commercial exploitation of the material contained in this work requires a
  license"; unclear for a non-partner MPL-2.0 reader. `autosar-data-specification` embeds an
  XSD-derived metamodel under MIT/Apache.
- **MathWorks SLA (Simulink, MAT) and NI EULA (LabVIEW):** reverse-engineering clauses bind
  licensees of the Program; reading user files is arguably not affected [unverified legal
  reading].

**Not possible in-process (skip, detect and explain):**

- **Thermo RAW** via RawFileReader: EULA 2.1 "You may not decompile, disassemble, reverse engineer
  ...", 4.2 no commercial exploitation, 4.3 no combination with GPL-like licenses, plus .NET 8.
  Pure-Rust routes are days old and cite CC-BY-SA prior art.
- **Origin .opj/.opju:** no spec; liborigin is GPL-3.0-or-later.
- **Vendor SDK formats:** Waters MassLynx, Nikon ND2 SDK, NI DataPlugins, Fanuc FOCAS, Allotrope
  ADF libraries (members only [unverified]).

**Crates that must not be linked:**

| Crate | License | Area |
|---|---|---|
| copybook, copybook-core, copybook-codec, ironwork-* | AGPL-3.0 | COBOL |
| gnucobol-rs (and kobold-data-shim, which depends on it) | LGPL-3.0 | COBOL |
| hl7v2 | AGPL-3.0 | HL7 |
| surge-io | PolyForm-Noncommercial-1.0.0 | power grids |
| openscenario-rs | GPL-3.0-only | automotive |
| rustronomy-fits, fits | GPL-3.0+, GPL-2.0+ | FITS |
| cspice-sys, cspice-rs, rust-spice (default feature) | LGPL-3.0 / links CSPICE under NAIF rules | SPICE |
| libsbml | LGPL-3.0 | SBML |
| ustar-parser, cif_chomper_core | LGPL-3.0 | CIF |
| mw-binding-xmi | AGPL-3.0 | SysML |
| dbf | AGPL | dBase |
| bioformats, sigrok | GPL-2.0+, GPL-3.0 | microscopy, logic analyzers |
| thermorawfilereader | Apache-2.0 but needs the restricted .NET DLL | Thermo RAW |

Multi-licensed crates that include GPL (`hl7`, `hl7-2`, `fhir`) are fine if Rezzer picks MIT or
Apache in cargo-deny. Crates with no LICENSE file (rustylink, syster-base, yore, xtce-rs) are not
used until that is fixed.

**Paid or restricted specs (Rezzer can still read files from public descriptions and permissive
crates):** ASAM MDF, A2L, CDF, ODX, FIBEX, XIL (members or buy) and the ASAM terms forbid sharing
any schema; ISO 17987, 22901, 13209; SAE J1939DA; ARINC 424, 429, 653, 665; RTCA DO-178C; ATA
iSpec 2200 / Spec 2000 (A4A); IEC 61970, 61850, 60255-24; NMEA 0183/2000 (actively enforced);
ASTM E1947/E2077; IEEE 1671; ECSS ("No ECSS document may be reproduced in any form without the
explicit consent of ESA").

### 10.4 Decisions for Jordi

1. **OpenReadout:** depend on its crates behind a trait (after a probe; 17 days old, one
   maintainer), use its Apache-2.0 format docs as prior art for own readers, or position Rezzer as
   the knowledge layer and point to OpenReadout for vendor lab binaries.
2. **AUTOSAR:** ask AUTOSAR or a lawyer whether an MPL-2.0 ARXML reader by a non-partner is fine.
3. **railML:** register with railML.org (and accept the certification rule) or detect only.
4. **Multi-file and folder input** (9.2): a `Source` set or folder form in sub-project 1.
5. **HDF5 route:** pure-Rust `hdf5-reader`/`netcdf-reader` vs `hdf5-pure`, decided by a corpus
   bake-off with hostile files; C libhdf5 (BSD-3) only as a fallback.
6. **Young crates policy:** a2lfile, can-dbc and dicom-rs are mature (use them behind a trait after the usual hostile-input probe); for falcon_mdf,
   powerio, ccsds-ndm, miniseed-rs, xodr, the openreadout family: own reader unless a probe
   passes.
7. **PII rule** for DICOM, HL7, ELN and instrument headers (redact or flag).
8. **Scope:** are seismology (StationXML, miniSEED), meteorology (GRIB, BUFR), power grids
   (CGMES, PSS/E) and medical (DICOM, HL7) in scope for W2, or only on demand?
9. **Corpus edge cases:** the FAA CIFP zip (over 2 MB) or an excerpt; ENTSO-E ReliCapGrid
   (CC BY-SA: are expected-output files adaptations?); `oem_g11.kvn` (CCSDS reproduction terms
   unknown); `naif0012.tls` (kernel terms not explicit).

### 10.5 Main open questions

- AUTOSAR, MathWorks and NI legal readings (above).
- Q-DAS DFQ/AQDEF (very common in German metrology) was not researched.
- Missing public samples: MDF4, LDF (candidate), ODX (candidate), FAA CIFP, S1000D, SCOS-2000
  MIB, StationXML, NASA CDF, SPICE binary, Heidenhain and Fanuc programs, DICOM and HL7
  (synthetic), Excel ICDs (synthetic), Simulink `.mdl`.
- Detection facts marked [unverified]: NASA CDF magic, SEDS namespace, ARINC 665 LUH layout,
  ARINC 653 roots, DAF offsets, SPSS magic, `.opj` magic, Heidenhain table syntax.
- Engineering report items that touch this one: KBL/VEC (automotive harness), IEC 61850 SCL,
  QIF, MTConnect, PLCopen and L5X are ranked there; this report only adds L5K, STEP 7 sources,
  GSD and MTConnect streams.

## Appendix A: sample files

All 29 files are in `spike/corpus/industry/<area>/` (the `spike/` folder is gitignored), listed
with pinned source URL, license and SHA-256 in `spike/corpus/industry/MANIFEST-industry.md`.

| Area folder | Files |
|---|---|
| `automotive/` | opendbc DBC (MIT), xcp-lite A2L (Apache-2.0), cantools ARXML (MIT), esmini OpenDRIVE (MPL-2.0) |
| `automation-energy/` | COMTRADE cfg + dat (MIT), ReliCapGrid CGMES EQ (CC BY-SA 4.0), AeroRust NMEA log (Apache-2.0) |
| `aerospace-space/` | BogusSAT-1 XTCE (Apache-2.0), CCSDS OEM KVN (MPL-2.0), NAIF LSK (NAIF), PDS4 label (Apache-2.0) |
| `models-legacy/` | MathWorks SLX with Stateflow (BSD-3), MSL Modelica (BSD-3), Reference-FMUs FMI 3 XML (BSD-2), Cobrix copybook + EBCDIC data (Apache-2.0) |
| `science-data/` | netCDF classic and NetCDF-4 (BSD-3), HST FITS (BSD-3), miniSEED 3 (Apache-2.0) |
| `science-lab/` | BioModels SBML (CC0), mzdata mzML (Apache-2.0), sciformats JCAMP-DX (MIT), Kadi4Mat ELN (MIT / CC BY 4.0) |
| `machines-lab/` | ANDI netCDF (MIT), StepOne RDML (MIT), SignalExpress TDMS (CC-BY-4.0), rust-stdf STDF (MIT) |

