# Rezzer research: engineering formats for BOMs, parts, product and plant structure

Date: 2026-10-19. Scope: the formats engineers use for parts lists (BOM), parts data, product and
plant structure, properties with units, relations (wiring, signals, pipes, nets) and
classifications, in the owner's area order: 1 automation, 2 construction, 3 electrical,
4 mechanical, 5 electronics. Formats already covered by the CAD spike
([`spike-2026-10-19/cad.md`](../spike-2026-10-19/cad.md): FreeCAD, STEP via stepq, IFC, IGES,
DXF, 3MF, STL, OBJ, glTF, G-code, KiCad, Gerber X2/X3 and Excellon, OpenSCAD) are only treated
where they have gaps (ifcXML, IDS, STEP AP242 XML, Gerber X3 sample).

Method: five research agents, one per area, worked to one fixed field schema. Crate versions,
dates, licenses and download counts come from the crates.io API today (a sample of nine was
re-checked when merging: ipc2581, async-opcua-xml, plcopen, rvt, openbim-gaeb, reqrs, aas,
sevenz-rust2, cfb). Repository licenses, stars and push dates come from `gh api` or the GitLab
API. Openness and terms come from the standards bodies' own pages and PDFs where they could be
fetched. Everything else is marked **[unverified]**. No GPL, AGPL or LGPL source code was read;
for such projects only the license is noted. No Rezzer code was written, nothing was executed.
30 small public sample files plus 4 license texts (about 5 MB) were downloaded into
`spike/corpus/engineering/`, see [Appendix A](#appendix-a-sample-files) and
`spike/corpus/engineering/MANIFEST-engineering.md`.
All SHA-256 values were recomputed when merging and match.

Legend:

- **Effort:** S under about 300 lines on zip + quick-xml or an existing reader, M 300 to 1000
  lines, L over 1000 lines or reverse-engineered binary, blocked = spec or license not
  obtainable. Reuse of existing Rezzer readers (OPC zip walk from OOXML/3MF, calamine and CSV,
  Part 21 tokenizer, s-expression reader, BOM renderer from STEP) is already credited.
- **Fit P/G/T/Tb:** 0 to 3 each for the facets properties, graph, timeline (revisions, change
  records, dated issues, document versions, time series) and tables (BOM, terminal lists, cost
  lines, parts lists).
- **Recommendation:** W1 first wave, W2 second wave, later, skip, covered.
- **Commonness** evidence is labeled in the area sections (tool support list, members, legal
  mandate, GitHub code search count, vendor claim). GitHub counts are weak evidence.

## 1. Summary table

| Format | Area | Openness | Structure | Commonness | Crates | Samples | Effort | Fit P/G/T/Tb | Rec. |
|---|---|---|---|---|---|---|---|---|---|
| Asset Administration Shell (AASX, AAS XML/JSON, IDTA submodels) | automation | open: specs, schemas and templates CC-BY-4.0 | ZIP-OPC + XML or JSON | high in DE/EU (Industrie 4.0, DPP candidate [unverified mandate]) | aas 0.4.0 (types), basyx-rs 0.2.1 (JSON only); none reads AASX XML | 2 (CC-BY-4.0) | M | 3/2/2/3 | W1 |
| AutomationML / CAEX (.aml, .amlx) | automation | XSDs in MIT repos, whitepapers free, IEC 62714 text paid | XML; .amlx = OPC zip | medium-high in DE (EPLAN, TIA, MTP base) | none (`aml` is ACPI) | 1 (MIT, synthetic) | M | 3/3/1/2 | W1 |
| DEXPI 1.x (Proteus) and 2.0 (DEXPI XML) | automation | spec PDF free, test cases CC-BY-4.0 | XML | medium (process industry, DE chemical) | none | 1 (CC-BY-4.0) | M | 3/3/1/3 | W1 |
| OPC UA NodeSet2 + companion specs | automation | MIT per file (SPDX header) | XML | high (about 72 companion specs, many VDMA) | async-opcua-xml 0.19.0 (MPL-2.0) | 1 (MIT) | M | 3/3/1/1 | W2 |
| PLCopen XML (IEC 61131-10) | automation | schema free [unverified link], IEC text paid | XML | medium (CODESYS export, Beremiz) | plcopen 0.3.1 (MIT) | 1 (BSD-3-Clause-LBNL) | M | 2/3/1/2 | W2 |
| IO-Link IODD | automation | spec terms [unverified], vendor files without license | ZIP of XML | high (20,000+ devices, 2021, weak) | ioddengine 0.1.3 (license mismatch) | none | S-M | 3/0/1/3 | W2 |
| IEC 61850 SCL (.scd/.icd/.cid) | automation | IEC text paid, light XSD under IEC licence [unverified] | XML | high in power utilities | iec61850-scl 0.2.0 (very new) | none (Apache-2.0 candidates) | M | 3/3/2/2 | W2 |
| MTP (VDI/VDE/NAMUR 2658) | automation | guideline paid, AML based [unverified] | ZIP + AML [unverified] | medium in DE process industry | none | none | S on AML | 2/3/0/2 | later |
| EtherCAT ESI | automation | ETG members only, no mirroring | XML | high in DE machine building | ethercat-esi 0.2.0, taktora-ethercat-esi 0.5.3 | none | S-M | 3/1/1/3 | later |
| PROFINET GSDML | automation | PI members only | XML (+ GSDX zip) | high in DE | none | none | S-M | 3/1/1/3 | later |
| EtherNet/IP EDS (ODVA) | automation | paid subscription | INI-like text | high in US | none | none | S | 3/0/1/2 | later |
| MTConnect | automation | free, XSD Apache-2.0 | XML | low in DE | none | none | S | 2/2/2/1 | later |
| Rockwell L5X, TIA SimaticML, TwinCAT 3 | automation | vendor docs or undocumented | XML | high US / DE / DE | l5x 0.6.0 (L5X only) | none | M | 2/2/1/2 | later |
| CODESYS native, NAMUR NE 100 | automation | closed / paid | binary / ECLASS XML | high / low | - | none | blocked / M | - | skip (use PLCopen, AAS) |
| GAEB DA XML 3.x (X31 to X97) | construction | XSD free download, no license text: read, do not bundle | XML | DE tendering default; state rules (Niedersachsen SBN 2024) | openbim-gaeb 0.1.2 (MIT, item summary only) | 2 (MIT) | M | 2/1/2/3 | W1 |
| BCF-XML 2.1 / 3.0 | construction | spec and XSD CC-BY-ND-4.0 | ZIP + XML + PNG | broad BIM coordination support | only AGPL crates (excluded) | 1 (CC-BY-ND-4.0) | S | 2/2/3/2 | W1 |
| IDS 1.0 | construction | XSD CC-BY-ND-4.0 | XML | buildingSMART standard since 2024-06 | openbim-ids 0.2.2 (MIT core, never enable audit features) | 1 (CC-BY-ND-4.0) | S | 3/1/1/3 | W1 |
| DIN 276 / ONORM B 1801 codes, bSDD URIs | construction | codes are facts, names are paid text | codes inside GAEB, IFC, IDS, sheets | universal in DE/AT cost planning | none | no DIN 276 code in the corpus; one bSDD URI in the IDS sample | S | 1/1/0/2 | W1 (enrichment) |
| CityGML 1.0 / 2.0 / 3.0 | construction | OGC, free | GML XML | all DE states publish LoD2 (NRW 35,022 tiles, CityGML 1.0) | ecitygml-io alpha (3.0), cadkit-gml (2.0 only) | 1 (MIT, 3.0) | M | 3/2/2/1 | W2 |
| XPlanung (XPlanGML) | construction | XSD free, no license text found | GML 3.2 (+ ZIP archive) | legally binding in DE land-use planning (secondary sources) | scaffolds only | none (test data unlicensed) | M on GML layer | 3/2/1/1 | W2 |
| LandXML 1.2 | construction | free XSD | XML | road, rail, survey tools | ifc-lite-landxml (terrain only) | 1 (MIT) | S-M | 2/2/1/3 | W2 |
| COBie 2.4 | construction | NIBS, free [unverified license] | XLSX (also IFC, XML) | UK/US; low in DE | none | none clean | S on calamine | 3/2/1/3 | W2 |
| ifcXML | construction | XSD free [unverified for 4.3] | XML | rare next to Part 21 | ifc-xml (AGPL, excluded) | none | M (IFC mapping reused) | 3/3/2/2 | W2 |
| Revit .rvt/.rfa | construction | closed, OLE CFB | binary | dominant BIM tool | rvt 0.6.0 (Apache-2.0, clean-room claim), cfb 0.15.0 | none | S metadata / L elements | 2/1/1/1 | W2 metadata, later elements |
| GAEB 90 / 2000, OKSTRA, XBau, ONORM A 2063, CPIXML, ICDD/MMC, IFC5, CityJSON | construction | spec missing, paid or license unclear | text / XML / GML / JSON | legacy or niche | AGPL or scaffolds | none | M to L or blocked | varies | later |
| ArchiCAD .pln, SIA 451 / CRBX, DIN SPEC 91400 | construction | closed / not public / withdrawn | - | - | none | none | blocked | - | skip |
| IEC 81346 / IEC 61355 designations | electrical | standards paid, syntax documented | string grammar inside any file | every ECAD plan in Europe | none | none in corpus (the uncopied EPDZ sample has them) | S | 2/3/0/2 | W1 (shared) |
| Wiring lists, terminal plans, EPLAN reports | electrical | no standard, tool layouts | CSV / XLSX | very high in panel building | calamine, csv (in Rezzer) | none (self-made later) | S | 2/3/0/3 | W1 (table profile) |
| BMEcat 2005 / 2005.2 | electrical | spec and XSD free, free right of use | XML | very high in DE B2B catalogs | none | 1 (MIT); a real ECLASS Advanced catalog was not kept (provenance) | S-M | 3/1/1/3 | W1 |
| ETIM xChange 2.0 | electrical | Apache-2.0 | JSON + JSON Schema | rising, ETIM International standard | none | 1 (official) | S | 3/2/2/3 | W1 |
| ECLASS / ETIM / IEC CDD codes in files | electrical | in-file codes readable; ECLASS dictionary licensed; ETIM EN model ODC-BY | IRDIs and codes inside host files | very high in DE/EU | none | in 4 samples (2 AAS, BMEcat, xChange) | S | 3/1/1/1 | W1 (code layer) |
| DATANORM 4 / 5 | electrical | no free spec [unverified owner] | text records, CP850 | very high in DE trade | none (oem_cp for CP850) | 2 (MIT, flagged) | S | 2/1/1/3 | W1 |
| EPLAN .epdz (Smart Production) | electrical | undocumented, plain containers | 7z + SQLite + SVG + AML | EPLAN leading ECAD in DE [weak] | sevenz-rust2 0.24.1, rusqlite or turso_core | 1 public, not copied (6.5 MB) | M | 3/3/1/2 | W2 |
| EPLAN .edz and part XML | electrical | undocumented, content under manufacturer terms | 7z + XML | very high among EPLAN users | sevenz-rust2 | none (redistribution forbidden) | M | 3/1/1/2 | W2 |
| ELDANORM | electrical | no spec found | text [unverified] | legacy | none | none | blocked | 2/0/0/2 | later |
| KBL 2.x / VEC 2.x wire harness (prostep ivip / VDA) | electrical | spec, XSDs, wiki and samples MIT | XML | standard OEM-supplier harness exchange in DE automotive | none in Rust (Java harness-model, MIT) | 2 (MIT, official) | KBL S-M, VEC M | 3/3/1/3 | W1 |
| Zuken E3.series, WSCAD, EPLAN native; OCI, IDS-Connect, ELBRIDGE, UGL, IEC 62683, RAMI, VDE-AR | electrical | closed, or not file formats | - | - | - | - | - | - | skip (read exports or codes) |
| ReqIF 1.2 (.reqif, .reqifz) | mechanical | OMG spec and XSD free, RF-Limited IPR | XML / zip | high in DE automotive (OEM to supplier) | reqrs 0.2.2 (Apache-2.0, young); AGPL crates excluded | 2 (Apache-2.0, origin flagged) | S-M | 3/3/2/2 | W1 |
| CAD/PDM BOM exports (SolidWorks, Inventor, Creo, Windchill) | mechanical | vendor output, no spec | CSV / XLS / XLSX | very high | calamine (in Rezzer) | none (synthetic needed) | S | 2/2/1/3 | W1 (table profile) |
| STEP AP242 XML (10303-3001 / -4442) | mechanical | XSDs free at standards.iso.org, ISO text paid | XML (.stpx, .stpxZ) | medium (DE automotive PDM exchange) | none decodes (cadmpeg detects only) | 1 (Apache-2.0) | M | 3/3/2/3 | W1 |
| QIF 3.0 (ISO 23952) | mechanical | ANSI QIF free after a form, XSDs BSL-1.0 | XML | medium (metrology, NIST) | none (`qif*` crates are Quicken) | 2 (BSL-1.0) | M | 3/2/2/3 | W1 |
| IPC-2571 PDX | mechanical | IPC text paid, DTD inside each file | XML / zip | low in DE | none (odx-rs is ASAM PDX) | 1 (Apache-2.0, synthetic) | S | 3/2/2/3 | W2 |
| SAP BOM (IDoc BOMMAT XML) | mechanical | IDoc docs behind SAP login | XML (flat IDoc later) | high in DE, rare as file | none | none | S | 2/2/1/3 | W2 |
| PLM XML (Siemens) | mechanical | "open, published", no license text found | XML | high in Teamcenter/NX shops | none | none licensed | M | 3/3/2/3 | W2 |
| JT (ISO 14306) | mechanical | ISO paid, royalty-free patent grant | binary, LZMA/zlib | high in automotive viz | none | none | L | 2/2/1/2 | later |
| 3D XML (Dassault) | mechanical | docs internal use only: blocks the docs route | ZIP + XML | medium | none | none | M (clean-room from files) | 2/2/1/2 | later |
| OMG PLM Services, OSLC, VDA 4965, VDA 4953, VDA-FS, SolidWorks PDM XML | mechanical | API, not a format, or geometry only | - | - | - | - | - | - | skip |
| IPC-2581 rev C (IPC-DPMX) | electronics | spec paid, XSD public in practice, license [unverified] | XML (often zipped) | about 140 consortium members; Cadence, Siemens, Zuken, Altium, KiCad export | ipc2581 0.3.3 (MIT/Apache, young, same repo as gerberx2) | 1 (MIT) | M | 3/3/2/3 | W1 |
| Eagle XML (.sch/.brd/.lbr) | electronics | DTD shipped, no terms | XML | large legacy corpus; EOL 2026-06-07 | none | 1 (MIT) | M | 3/3/1/3 | W1 |
| Altium SchDoc + PrjPcb | electronics | closed, permissive reverse-engineering docs (WTFPL, Apache, MIT) | OLE CFB + pipe-delimited `KEY=VALUE` records; INI | leading professional PCB tool [vendor claim] | cfb 0.15.0; altium 0.2.1 (stale); altium-format GPL (no) | 1 (MIT) | M | 3/3/1/3 | W1 |
| Electronics BOM and pick-and-place CSV | electronics | conventions | CSV / XLSX | every assembly order | reuse | 1 (MIT) | S | 2/1/0/3 | W1 (table profile) |
| IPC-D-356 netlist | electronics | spec paid, layout widely documented | fixed-column text | standard part of fab packages | none | none kept | S | 1/3/0/2 | W1 |
| ODB++ | electronics | free after sign-up, but PDF says confidential, Siemens UCA | tgz directory, text records, `.Z` LZW | >70 software partners, fabs | none | 1 (CERN-OHL-W-2.0) | M | 3/3/1/3 | W2 (owner decision) |
| prostep IDX / EDMD (PSI 5) | electronics | docs free, XSD "all rights reserved" | XML | Cadence, Siemens, PTC, Dassault, Zuken | none | 1 (CERN-OHL-W-2.0) | M | 3/2/3/2 | W2 |
| SPICE netlists, LTspice .asc | electronics | free manuals | text | common | thevenin-types 0.5.0 and others | none | M | 2/3/0/1 | W2 |
| Altium PcbDoc | electronics | reverse-engineered | CFB binary records | as SchDoc | none | candidate URL only | L | 3/3/1/3 | W2 |
| EDIF, OrCAD DSN, IBIS, HyperLynx, Fritzing, IPC-7351 name decoder | electronics | paid, closed or no spec | text / CFB / ZIP | niche or legacy | none | none | S to L | varies | later |
| Allegro .brd, Zuken CR-8000 native | electronics | closed | binary | common | none | none | blocked | via exports | skip |
| Gerber X3 component layer | electronics | Ucamco spec free | text | KiCad export | own reader exists | still no real file | covered | 2/0/0/3 | covered, add sample |

## 2. Automation


Date: 2026-10-19. Crate data from the crates.io API today, repo data from `gh api` / GitLab API
today. Samples and hashes: Appendix A. Everything not checked
against a primary source is marked [unverified]. No GPL/AGPL/LGPL source was read (beremiz GPL-3.0,
PLC-lang/rusty LGPL-3.0, pyDEXPI AGPL-3.0, DEXPI2graphML LGPL-3.0: license noted only).

Common pattern: almost every format here is XML (sometimes inside an OPC/zip package), so nearly all
readers sit on the existing `zip` + `quick-xml` base and the OPC relationship handling from the
OOXML/3MF readers. The work is the semantic mapping (class libraries, references, units,
dictionaries), not the syntax. Many files carry dictionary IDs (ECLASS IRDIs, POSC Caesar RDL URIs,
UNECE unit codes, IEC CDD). Rezzer can always show the ID; shipping the dictionary text is a
separate license question (ECLASS is licensed, not free).

### Summary

| Format | Openness | Structure | Commonness | Crates | Samples | Effort | Fit (P/G/T/Tb) | Recommendation |
|---|---|---|---|---|---|---|---|---|
| Asset Administration Shell (AASX, AAS XML/JSON) | open: specs and schemas CC-BY-4.0; ECLASS dictionary licensed | ZIP-OPC + XML or JSON | high in DE/EU, policy driven (DPP) | `aas` 0.4.0 (types), `basyx-rs` 0.2.1 (JSON only, 2023) | 2 downloaded (CC-BY-4.0) | M | 3/2/2/3 | first wave |
| AutomationML / CAEX (.aml, .amlx) | XSD MIT, whitepapers free, IEC 62714 paid | XML, ZIP-OPC (.amlx) | medium-high in DE | none (`aml` is ACPI) | 1 downloaded (MIT, synthetic) | M | 3/3/1/2 | first wave |
| DEXPI (P&ID, 1.x Proteus, 2.0 DEXPI XML) | test cases CC-BY-4.0, spec free PDF; RDL license separate | XML | medium (process industry, DE chemical) | none | 1 downloaded (CC-BY-4.0) | M | 3/3/1/3 | first wave |
| OPC UA NodeSet2 + companion specs | NodeSets MIT (SPDX per file), spec free to read | XML | high (79 folders in UA-Nodeset) | `async-opcua-xml` 0.19.0 (MPL-2.0) | 1 downloaded (MIT) | M | 3/3/1/1 | second wave |
| PLCopen XML (IEC 61131-10, TC6 v2.01) | schema free from PLCopen [unverified link], IEC 61131-10 paid | XML | medium (CODESYS, Beremiz exports) | `plcopen` 0.3.1 (MIT) | 1 downloaded (BSD-3-Clause-LBNL) | M | 2/3/1/2 | second wave |
| IO-Link IODD | spec zip on io-link.com redirects to download page [unverified terms]; vendor files no license | ZIP of XML + images | high (>20,000 devices, 2021, weak) | `ioddengine` 0.1.3 (license mismatch) | none (no license) | S-M | 3/0/1/3 | second wave |
| IEC 61850 SCL (.scd/.icd/.cid/.ssd) | IEC 61850-6 paid; light XSD free under IEC license [unverified] | XML | high in power utilities, EU TSOs | `iec61850-scl` 0.2.0 | none yet (open-scd Apache-2.0 candidates) | M | 3/3/2/2 | second wave |
| MTConnect device XML | spec free with license; XSD Apache-2.0 | XML | medium in US, low in DE | none | none yet (cppagent Apache-2.0 candidate) | S | 2/2/2/1 | later |
| MTP (VDI/VDE/NAMUR 2658) | guideline paid (VDI/DIN Media) [unverified]; AML based | ZIP-OPC with AML [unverified] | medium in DE process industry | none | none | S on top of AML | 2/3/0/2 | later (after AML) |
| EtherCAT ESI | spec and XSD ETG members only, no mirroring | XML | high in DE machine building | `ethercat-esi` 0.2.0, `taktora-ethercat-esi` 0.5.3 | none (ETG terms) | S-M | 3/1/1/3 | later |
| PROFINET GSDML (+GSDX) | spec PI members only | XML (+ZIP GSDX) | high in DE (every PROFINET device) | none | none | S-M | 3/1/1/3 | later |
| EtherNet/IP EDS (ODVA) | spec paid subscription | text INI-like | high in US, medium EU | none (`zencan-eds` is CANopen) | none | S | 3/0/1/2 | later |
| Rockwell L5X (added) | free Rockwell manual [unverified] | XML | high in US | `l5x` 0.6.0 (MIT) | none | M | 2/2/1/2 | later |
| TIA Portal SimaticML / Openness XML | Siemens manual, proprietary, XSD with TIA [unverified] | XML | high in DE | none | none | M | 2/2/1/2 | later |
| TwinCAT 3 (.tsproj, .TcPOU, .plcproj) | undocumented proprietary XML [unverified] | XML | medium-high in DE | none for files (`twincat` = ADS protocol) | none | M | 2/2/1/1 | later |
| CODESYS (.project native; PLCopen export) | native binary closed; export = PLCopen XML | binary / XML | high in DE | via `plcopen` | none | blocked native / covered by PLCopen | 2/3/1/2 | skip native, use PLCopen |
| NAMUR NE 100 / PROLIST | NE paid for non-members [unverified], lists via ECLASS | XML (ECLASS Advanced) [unverified] | low-medium (DE process) | none | none | M | 3/0/0/3 | skip for now |


### Asset Administration Shell (AAS, AASX)

- Defining body: Industrial Digital Twin Association (IDTA). Part 1 Metamodel IDTA-01001 v3.1.1
  (2025-08) and v3.2 (2026-07, https://industrialdigitaltwin.org/wp-content/uploads/2026/07/IDTA-01001-3-2_AAS-Specification_Part1_Metamodel.pdf);
  Part 5 AASX package IDTA-01005 (https://industrialdigitaltwin.org/wp-content/uploads/2025/06/IDTA_01005-25-01_AAS-Specification_Part5_AASXPackageFileFormat.pdf).
  IEC 63278-1 is the IEC track [unverified mapping].
- Openness: specs free PDF. Schemas and metamodel repo `admin-shell-io/aas-specs` CC-BY-4.0 (78
  stars, pushed 2026-10-19). Submodel templates `admin-shell-io/submodel-templates` CC-BY-4.0 (94
  stars, 55 published template folders at f3790038). No registration. File format fully readable.
  Dictionary content is separate: the Technical Data sample carries 39 distinct ECLASS IRDIs
  (`0173-1#...`), Handover Documentation 37. Rezzer shows IRDIs and the preferred names that the
  file itself embeds (ConceptDescriptions with IEC 61360 data specs); it must not ship the ECLASS
  dictionary.
- Container: AASX = OPC package (ECMA-376 Part 2): `_rels/.rels` -> `aasx/aasx-origin` -> spec part
  (`*.aas.xml` or `*.aas.json`) -> supplementary files (PDF, images, STEP). Namespace
  `https://admin-shell.io/aas/3/0`; older v2 files (namespace `.../aas/2/0`) exist in the
  deprecated template folders. Key elements: `assetAdministrationShell`, `assetInformation`
  (globalAssetId, assetKind), `submodel` (semanticId, idShort), `submodelElementCollection` /
  `submodelElementList`, `property` (valueType, value), `multiLanguageProperty`, `file`,
  `referenceElement`, `relationshipElement`, `entity` (BoM trees), `conceptDescription`
  (unit, unitId, definition).
- Knowledge carried: properties with units (via ConceptDescriptions; sample: mm, degC), BOM and
  product structure as Entity trees with HasPart relations (Hierarchical Structures enabling BoM,
  IDTA 02011), classifications (ECLASS/IEC CDD class IDs, ProductClassifications in Technical Data),
  references to documents (Handover Documentation IDTA 02004: DocumentId, DocumentVersion,
  StatusValue, StatusSetDate, DigitalFiles to PDFs inside the package), revisions (administration
  version/revision per submodel, document versions), Carbon Footprint (IDTA 02023) values per
  life-cycle phase, Digital Nameplate (IDTA 02006 v3.0) manufacturer, serial, markings.
- Commonness: policy evidence: AAS is the German Plattform Industrie 4.0 digital twin and a
  candidate carrier for the EU Digital Product Passport (ESPR, Battery Regulation) [unverified
  mandate status]. Tool evidence: Eclipse BaSyx (Java, Python MIT), Eclipse AAS4J (Apache-2.0),
  AASX Package Explorer (eclipse-aaspe, 129 stars). IDTA published 55 submodel template families
  (verified count). Germany/EU relevance: very high.
- Rust crates: `aas` 0.4.0 (2026-01-21, MIT, 266 downloads, no repo; README: types for Part 1 v3.1,
  JSON de/serialisation, XML and AASX planned). `basyx-rs` 0.2.1 (2023-06-05, MIT, 8,825 downloads,
  eclipse-basyx/rust-sdk 6 stars; README: metamodel v3.0, "only JSON (de-)serialization is
  supported"). `dpp-aas` 0.22.0 (Apache-2.0) is a projection for one product, not a reader.
  Other OSS: basyx-python-sdk (Python, MIT, 105 stars), aas4j (Java, Apache-2.0, 51 stars),
  aas-core3.0 generated SDKs (license NOASSERTION on GitHub [unverified]).
- Public samples: submodel-templates (CC-BY-4.0, many `.aasx` and `.json`), basyx-java-server-sdk
  examples (MIT, e.g. `examples/BaSyxMinimal/aas/ExampleV3.aasx` 123 KB). Downloaded: Technical
  Data 2.0 sample and Handover Documentation 2.0 example.
- Effort: M. OPC package walk reuses the OOXML/3MF code; XML and JSON serialisations both needed
  (one model, two front ends with `quick-xml` and `serde_json`); v2 vs v3 namespace; semanticId
  resolution to the embedded ConceptDescriptions for names and units; known submodel templates
  (Nameplate, TechnicalData, BoM, Handover, CarbonFootprint) mapped by semanticId to tables and trees.
  Own reader recommended; no crate reads XML + AASX.
- Fit: properties 3, graph 2 (Entity/relationship trees, references), timeline 2 (document
  versions and status dates, submodel versions, time series submodel), tables 3 (BoM, tech data,
  document lists).
- Recommendation: first wave. Open license, German/EU policy weight, and it maps directly onto
  Rezzer's model (properties with units, BoM, documents).

### AutomationML / CAEX

- Defining body: AutomationML e.V.; IEC 62714 (Part 1 Ed. 2 2018 architecture, paid, about 145 EUR
  at resellers), data format CAEX IEC 62424:2016 (CAEX 3.0; 2.15 still common). Whitepaper
  "AutomationML Edition 2.1" (https://www.automationml.org/about-automationml/specifications/).
- Openness: whitepapers and libraries downloadable without registration (page shows direct
  download links; license text not shown). CAEX XSDs in MIT repos (AutomationML/AML-UA-XSLT,
  automationml-python). AMLEngine2.1 (C#) MIT. File format readable; role/interface libraries
  are part of the whitepapers. Linked documents (COLLADA geometry, PLCopen XML logic) are separate
  formats.
- Container: XML `CAEXFile` (namespace `http://www.dke.de/CAEX` for 3.0, none for 2.15) with
  `InstanceHierarchy` > `InternalElement` (tree), `Attribute` (Unit, AttributeDataType, nested),
  `ExternalInterface`, `InternalLink` (RefPartnerSideA/B), `RoleRequirements`, class libraries
  (`SystemUnitClassLib`, `RoleClassLib`, `InterfaceClassLib`, `AttributeTypeLib`),
  `SourceDocumentInformation`. `.amlx` = AutomationML container, an OPC package
  (BPR "AutomationML Container"; also used by OPC UA Part 83 descriptors).
- Knowledge carried: plant/product structure tree, properties with units, connections as graph
  (InternalLink between interfaces), classifications (role classes, ECLASS mapping whitepaper),
  references to external documents (ExternalDataConnector to COLLADA, PLCopen XML, PDFs),
  change mode and writer header (version, writing date). Sample: 21 InternalElement, 26 Attribute
  with units s, kg, degC, 79 ExternalInterface, 18 InternalLink.
- Commonness: weak to medium evidence. Organisation founded by Daimler, Siemens, ABB, KUKA and
  others [unverified]; used as base of MTP and in TIA Portal AML export [unverified]; OPC UA has an
  AML companion spec (`UA-Nodeset/AML`, verified folder). Germany relevance high.
- Rust crates: none on crates.io for AutomationML/CAEX (searched "automationml", "caex"; `aml`
  0.16.4 is ACPI Machine Language, repository rust-osdev/acpi). Other OSS: AMLEngine2.1 (C#, MIT, 31 stars),
  automationml-python (Python, MIT, 1 star, new 2026).
- Public samples: AutomationML/AML-UA-XSLT unit tests (MIT, 14 small files), automationml-python
  validation suite (MIT), AMLToolkit `MasterDatei.aml` (MIT, CAEX 2.15, German names). Downloaded:
  `big-valid-packaging-line.aml` (synthetic).
- Effort: M. Syntax is S on `quick-xml`; class inheritance resolution (RefBaseSystemUnitPath,
  RefBaseClassPath, attribute inheritance), link resolution by ID and path, CAEX 2.15 vs 3.0, and
  `.amlx` via the shared OPC code push it to about 600 lines.
- Fit: properties 3, graph 3, timeline 1, tables 2.
- Recommendation: first wave. Open, German, rich graph, and needed for MTP.

### DEXPI (P&ID exchange)

- Defining body: DEXPI initiative (DECHEMA/ProcessNet with NAMUR [unverified]); ISO 15926 based
  reference data (POSC Caesar RDL). DEXPI Specification 2.0.0 (2025-10-10) and 2.0.1 fix release
  (2026-09-30) on https://gitlab.com/dexpi/Specification/-/releases ; 1.3 (2021) and 1.4 (2024)
  earlier. 2.0 replaces the Proteus schema with a standardized "DEXPI XML" serialisation and adds
  PFD/BFD.
- Openness: spec PDF free (https://dexpi.org/wp-content/uploads/2025/10/DEXPI_Specification_2.0.pdf);
  Specification repo has no license set on GitLab [unverified terms]. Test cases
  `dexpi/TrainingTestCases` CC-BY-4.0 (GitLab license field, LICENSE file). RDL URIs
  (`http://data.posccaesar.org/rdl/...`, `http://sandbox.dexpi.org/rdl/...`) in files; RDL content
  license separate [unverified].
- Container: XML. 1.x: `PlantModel` > `PlantInformation` (OriginatingSystem, SchemaVersion,
  units), `MetaData` (title block GenericAttributes), `Equipment` > `Nozzle`,
  `PipingNetworkSystem` > `PipingNetworkSegment` > `PipingComponent` + `Connection`
  (FromID/ToID), `ProcessInstrumentationFunction`, `InstrumentationLoopFunction`,
  `SignalConveyingFunction`, `GenericAttributes` (Name, Value, Units, AttributeURI), plus Proteus
  graphics (Position, PolyLine, Text). 2.0: new DEXPI XML, no public sample yet.
- Knowledge carried: equipment list with design data and units (sample: Bar, DegreeCelsius,
  Kilowatt, MetreCubedPerHour), pipe network as graph (nozzle to segment to component), instrument
  loops and signal lines, tag names, title block (approver, checker, dates, archive number,
  revision), RDL classification URIs. Sample: 21 Equipment, 21 Nozzle, 27 PipingComponent,
  23 PipingNetworkSegment, 29 Connection, 807 GenericAttribute.
- Commonness: weak to medium. Test cases contain uploads from several P&ID tools (file code
  suffixes AVV, HEX, SAG, ING, AUD, VER per vendor [unverified mapping]); exporter in sample is
  P&ID Toolbox (pnb, Aachen). OPC UA DEXPI companion spec (OPC 30250) and IDTA DEXPI submodel
  template exist (verified folders). German chemical industry (BASF C02 and Covestro C08 test cases; Equinor, Norway, C03).
- Rust crates: none. Other OSS: pyDEXPI (Python, AGPL-3.0, 235 stars, not read),
  DEXPI2graphML (LGPL-3.0), DexpiProcessDotNet (C#, MIT, 3 stars), equinor NOAKADEXPI (no license).
- Public samples: TrainingTestCases (CC-BY-4.0, 1.2 and 1.3 only, about 630 files). Downloaded:
  C01V04 reference P&ID (DEXPI 1.3).
- Effort: M. Two dialects (Proteus 1.x and DEXPI XML 2.0); graph build from Connection elements;
  skip graphics except labels. About 700 lines.
- Fit: properties 3, graph 3, timeline 1, tables 3 (equipment, line, instrument lists).
- Recommendation: first wave. Open samples, rich graph, process industry has no other open P&ID
  format.

### OPC UA NodeSet2 and companion specifications

- Defining body: OPC Foundation (OPC Federation AISBL per 2026 file headers); IEC 62541. Core
  model 1.05.07 (2026-07-30, from RequiredModel in the Machinery NodeSet). Spec readable free at
  https://reference.opcfoundation.org.
- Openness: `OPCFoundation/UA-Nodeset` (286 stars, pushed 2026-10-18) has no repo LICENSE; each
  file carries a license header: `SPDX-License-Identifier: MIT` (2026 files) or "OPC Foundation
  MIT License 1.00" (older files, e.g. Schema/Opc.Ua.NodeSet2.xml, DEXPI, IOLink). So NodeSets
  can be bundled. Member-only drafts in a separate repo.
- Container: XML `UANodeSet` with `NamespaceUris`, `Models/Model` (ModelUri, Version,
  PublicationDate, RequiredModel), `Aliases`, `UAObjectType`, `UAVariableType`, `UADataType`,
  `UAReferenceType`, `UAObject`, `UAVariable`, `UAMethod`, each with `References`. Companion
  files: `.NodeIds.csv`, `.Types.xsd`.
- Knowledge carried: type and instance model as typed graph (HasComponent, HasProperty,
  Organizes, HasTypeDefinition), properties with EUInformation units (UNECE codes), model
  versions and publication dates, documentation links per type. Sample: 62 UAObject, 390
  UAVariable, 60 UAMethod, 2019 References, 9 namespaces.
- Commonness: high. 79 folders in UA-Nodeset (about 72 companion specs after removing Schema,
  XML, DotNet, AnsiC, DemoModel, TestModel, OpenApi), incl. Machinery, MachineTool, PackML,
  Robotics, IOLink, PROFINET, IEC61850, AML, DEXPI, Weihenstephan, Woodworking (VDMA
  specs, strong DE relevance).
- Rust crates: `async-opcua-xml` 0.19.0 (2026-07-18, MPL-2.0, 7,371 downloads, freeopcua/async-opcua
  163 stars): "loading of NodeSet2 XML files", API not inspected in depth; `async-opcua-nodes`
  0.19.0 (2026-07-18, MPL-2.0, 295,389 downloads) node representation. MPL-2.0 is allowed. (`nodeset` 0.6.0, fdiakh/nodeset-rs, "NodeSet management library", appears to be HPC cluster
  node sets, not OPC UA [unverified].) Other OSS: UA-ModelCompiler (C#, MIT), open62541 nodeset compiler
  (Python, MPL-2.0 [unverified]).
- Public samples: all of UA-Nodeset (MIT per file). Downloaded: Machinery Examples NodeSet2.
- Effort: M. Parse is S; readable output needs the base `Opc.Ua.NodeSet2.xml` (numeric
  ReferenceTypes like i=46, i=47, i=40 and DataTypes are only named there) and the RequiredModel
  chain; bundle the MIT base NodeSet (about 1 to 2 MB, compressible) or a generated name table.
  Could use `async-opcua-xml` behind a trait or own `quick-xml` reader (about 500 lines).
- Fit: properties 3, graph 3, timeline 1 (model versions only), tables 1.
- Recommendation: second wave. Clean and open, but files are mostly type models for developers,
  less often project knowledge.

### PLCopen XML (IEC 61131-10, TC6 XML v2.01)

- Defining body: PLCopen TC6; standardized as IEC 61131-10:2019 (Ed. 1, paid,
  https://webstore.iec.ch/publication/33034). Technical doc v2.01 PDF at
  https://www.plcopen.org/system/files/downloads/tc6_xml_v201_technical_doc.pdf returned 404
  today; schema `tc6_xml_v201.xsd` described as downloadable from PLCopen [unverified].
- Openness: schema published by PLCopen free of charge [unverified]; no known patents. File
  format readable.
- Container: XML `project` (namespace `http://www.plcopen.org/xml/tc6_0201`): `fileHeader`,
  `contentHeader`, `types/dataTypes`, `types/pous/pou` (pouType, `interface` var sections,
  `body` in ST/IL/FBD/LD/SFC), `instances/configurations/resource/task`, `addData` for vendor
  extensions.
- Knowledge carried: program structure (POUs, call graph), variables with types and initial
  values (I/O lists), FBD/LD connection graphs, SFC steps and transitions, tasks and cycle times,
  creation/modification dates. Sample: 6 POUs, 20 FBD blocks, 49 connections, 4 ST bodies.
- Commonness: medium. Export from CODESYS (and so WAGO, Festo, Bosch Rexroth, Schneider M-series
  [unverified]), Beremiz/OpenPLC native, B&R [unverified]. GitHub code search for the namespace
  shows mainly Beremiz/OpenPLC projects and research repos (weak).
- Rust crates: `plcopen` 0.3.1 (2025-12-14, MIT, 801 downloads, radevgit/plc 14 stars): typed
  serde model generated from the v2.01 schema, extracts FBD/LD/SFC elements, ST via `iec61131`
  0.7.0 (2025-12-11, MIT, 930 downloads). Same author: `l5x` 0.6.0, `plceye`, `plcviz`. `ironplc` (MIT, 125 stars) is an
  IEC 61131-3 compiler with PLCopen XML tests; crate `ironplc-parser` 0.1.1 (2023-01-06, MIT, 3,413 downloads) is stale on crates.io.
  Other OSS: beremiz (GPL-3.0, not read), PLC-lang/rusty (LGPL-3.0, not read).
- Public samples: lbl-srg/cdl-plc (BSD-3-Clause-LBNL), mniedermaier/CybICS (MIT, OpenPLC
  `plc.xml`), ironplc test files (MIT, tiny). Downloaded: cdl-plc PID.
- Effort: M. `plcopen` crate is a real model and could sit behind a trait (young, single author,
  serde strictness on vendor `addData` untested); own reader about 600 lines with graph build.
- Fit: properties 2, graph 3, timeline 1, tables 2 (variable and I/O lists).
- Recommendation: second wave. Only open vendor-neutral PLC code format; also covers CODESYS.

### IO-Link IODD

- Defining body: IO-Link Community (under PI); IODD V1.1 spec (current V1.1.x [unverified]);
  IEC 61131-9 for IO-Link itself.
- Openness: spec zip URL `io-link.com/share/Downloads/Spec-IODD/IO_Device_Description_V1.1_Specification.zip`
  now redirects to https://io-link.com/downloads (307 checked today); terms not seen [unverified].
  IODD files are vendor documents; IODDfinder license terms unknown [unverified]. File format
  readable; standard definitions file (IODD-StandardDefinitions) ships with the spec.
- Container: zip with main `<Vendor>-<Device>-<date>-IODD1.1.xml`, language files, images, and
  manufacturer declaration (io-link.com/technology/iodd). XML: `DeviceIdentity`,
  `ProfileBody/DeviceFunction/VariableCollection` (Variable with Datatype, index, access,
  units via UnitCode), `ProcessDataCollection`, `UserInterface` menus, `ExternalTextCollection`.
- Knowledge carried: device identity (vendor, device ID, product variants), parameter list with
  types, ranges, default values and units, process data layout (bit offsets), menus,
  multilingual text, release date and version in file name and DocumentInfo.
- Commonness: weak, dated: IODDfinder had IODDs for over 20,000 devices from over 100
  manufacturers (2021 press, https://io-link.com/en/Global/newsmeldung.php?showId=892 and
  others). Strong in DE sensor makers (ifm, SICK, Balluff, Pepperl+Fuchs).
- Rust crates: `ioddengine` 0.1.3 (2024-04-16, crates.io says MIT, README says Apache 2.0,
  1,508 downloads, uneidel/ioddengine 1 star). Weak.
- Public samples: none with a clear license found. Not downloaded.
- Effort: S to M (about 400 lines; text references resolve through the language files).
- Fit: properties 3, graph 0, timeline 1, tables 3 (parameter and process data tables).
- Recommendation: second wave. Very common, simple, but samples need a vendor license or a
  self-written test IODD.

### IEC 61850 SCL (.scd, .icd, .cid, .iid, .ssd, .sed)

- Defining body: IEC TC57; IEC 61850-6 Ed. 2.1 (2018) plus amendments [unverified current edition].
- Openness: standard paid. SCL XSD is an IEC code component; a "light" version is free to
  download from https://www.iec.ch/tc57/supportdocuments, use under IEC code component licence
  [unverified, page returned 403 today]. File format readable.
- Container: XML `SCL` with `Header` (History/Hitem revisions), `Substation` (VoltageLevel, Bay,
  ConductingEquipment, ConnectivityNode: single-line diagram topology), `Communication`
  (SubNetwork, ConnectedAP, addresses), `IED` (AccessPoint, LDevice, LN, DOI/DAI values, DataSet,
  ReportControl, GSEControl, Inputs/ExtRef), `DataTypeTemplates`.
- Knowledge carried: substation single-line topology as graph, IED list with data model,
  signal bindings (ExtRef: GOOSE/SV subscriptions), network addressing tables, revision history
  (Header/History with version, revision, when, who, why).
- Commonness: high in power utilities; LF Energy CoMPAS (com-pas, EU TSOs) and OpenSCD
  (Apache-2.0, 144 stars, archived) show EU adoption (weak, tool evidence).
- Rust crates: `iec61850-scl` 0.2.0 (2026-09-05, MIT OR Apache-2.0, 68 downloads,
  csp0924/iec61850-rust 1 star): two-stage parser into an IedModel for a server stack;
  focuses on the IED model, Substation section coverage not checked. Very new.
- Public samples: open-scd test files (Apache-2.0, e.g. `comm-map.scd` 30 KB). Not downloaded.
- Effort: M (about 700 lines; type template resolution and ExtRef graph).
- Fit: properties 3, graph 3, timeline 2 (History items), tables 2.
- Recommendation: second wave. Well structured and open samples exist; audience is energy, not
  core factory automation.

### MTConnect

- Defining body: MTConnect Institute (AMT, US); v2.5 (2025-02), ANSI/MTC1.4-2018
  (https://www.mtconnect.org/standard-download20181).
- Openness: free with a non-exclusive copyright license to reproduce and redistribute in received
  form (Part 2 2.5.0 PDF notice). XSDs in `mtconnect/schema` Apache-2.0 (17 stars). Royalty free.
- Container: XML `MTConnectDevices` (device model: Device > Components > DataItems with units,
  Compositions, Configuration), `MTConnectStreams` (time-stamped observations), `MTConnectAssets`
  (CuttingTool, etc.).
- Knowledge carried: machine component tree, data items with units and types, time series in
  Streams documents (timeline), cutting tool assets with life values.
- Commonness: medium in US machine tools; in DE/EU OPC UA MachineTool/umati dominates (weak).
- Rust crates: none. Other OSS: cppagent (C++, Apache-2.0, 184 stars), dot_net_sdk (Apache-2.0).
- Public samples: cppagent `demo/agent/Devices.xml` (Apache-2.0, 59 KB). Not downloaded.
- Effort: S (about 300 lines).
- Fit: properties 2, graph 2, timeline 2, tables 1.
- Recommendation: later. Low EU relevance.

### MTP Module Type Package (VDI/VDE/NAMUR 2658)

- Defining body: VDI/VDE/NAMUR 2658 parts 1 to 5+ (paid via DIN Media [unverified]); NAMUR NE 148;
  IEC 63280 in progress [unverified]; driven by NAMUR, ZVEI, PI (MTP roadmap 2024:
  https://se.profibus.com/fileadmin/user_upload/MTP_Status_Roadmap_2024-06-11.pdf).
- Openness: guideline paid; file is AutomationML with MTP role class libraries [unverified
  details]; an IDTA MTP submodel template exists (`submodel-templates/published/MTP`, verified).
- Container: `.mtp` zip with an AML manifest (CAEX InstanceHierarchy `ModuleTypePackage`)
  [unverified, no primary source found today].
- Knowledge carried: module services and procedures, HMI pictures (topology of the module),
  data assemblies (tags with ranges and units), OPC UA communication endpoints.
- Commonness: modular process plants in DE chemical/pharma; vendors Beckhoff, WAGO, Siemens,
  ABB export MTP (vendor pages, weak).
- Rust crates: none. Public samples: none found with a license.
- Effort: S on top of the AutomationML reader (role class mapping).
- Fit: properties 2, graph 3, timeline 0, tables 2.
- Recommendation: later, right after AutomationML lands.

### EtherCAT ESI

- Defining body: EtherCAT Technology Group, ETG.2000 v1.0.18 (2025-05-26), ESI schema v1.21.
- Openness: spec and XSD members only; ETG page: "You may not copy, distribute or 'mirror' the
  files ... without permission in writing from the ETG" (ethercat.org downloads page, checked
  today). Vendor ESI files are freely downloadable from vendors, redistribution terms vary.
  Reading the format from vendor files is possible; spec access needs membership (free for
  members [unverified]).
- Container: XML `EtherCATInfo` > `Vendor`, `Descriptions/Groups`, `Devices/Device` (Type,
  ProductCode, RevisionNo, Sm, Pdo/Entry, Dc, Profile/Dictionary with CoE objects).
- Knowledge carried: device identity and revisions, PDO mapping (process data tables), object
  dictionary with data types, multilingual names.
- Commonness: very common in DE machine building (Beckhoff home market) (weak, no numbers).
- Rust crates: `ethercat-esi` 0.2.0 (2020-11-03, MIT/Apache-2.0, 17,816 downloads, 20 stars,
  last push 2023); `taktora-ethercat-esi` 0.5.3 (2026-07-15, Apache-2.0 OR MIT, 793 downloads):
  "typed IR" for codegen.
- Public samples: none with a clear license. Not downloaded.
- Effort: S to M (about 400 lines on `quick-xml`; CoE dictionary and PDO tables).
- Fit: properties 3, graph 1, timeline 1, tables 3.
- Recommendation: later. Spec terms make a clean-room reader harder to justify.

### PROFINET GSDML

- Defining body: PROFIBUS & PROFINET International (PI); GSDML spec V2.50 (order no. 2.352,
  2026, https://www.profibus.com/gsdml-gsdx-specification-for-profinet). GSDX container since V2.43.
- Openness: spec download members only after accepting a license agreement. Vendor GSDML files
  are public downloads, no license. XSD ships with the spec [unverified].
- Container: XML `ISO15745Profile` > `ProfileBody/DeviceIdentity`, `DeviceFunction`,
  `ApplicationProcess` (DeviceAccessPointList, ModuleList, SubmoduleList, ValueList,
  ExternalTextList), GSDX = zip with GSDML and attachments.
- Knowledge carried: device and module catalog, IO data layout, parameter records with ranges,
  diagnosis texts, multilingual texts; version and date in the file name.
- Commonness: every PROFINET device ships one (spec requirement); PROFINET strong in DE.
- Rust crates: none (`pnio` 0.1.0 and `profidcp` 1.0.3 are protocol stacks, not GSDML readers).
- Public samples: none with a license. Not downloaded.
- Effort: S to M (about 400 lines; ExternalTextList resolution, GSDX zip).
- Fit: properties 3, graph 1, timeline 1, tables 3.
- Recommendation: later (spec access members only).

### EtherNet/IP EDS (ODVA)

- Defining body: ODVA; CIP Volume 1 chapter 7 (Volume 1 edition 3.40, 2026-04).
- Openness: spec sold as yearly subscription (https://www.odva.org/subscriptions-services/).
  2026 change list adds EDS requirements for interpreting applications. Vendor EDS files public.
- Container: ASCII INI-like text: `[File]` (DescText, CreateDate, Revision), `[Device]`
  (VendCode, ProdCode, MajRev), `[Params]`, `[Assembly]`, `[Connection Manager]`, `[Port]`.
  Same `.eds` extension and INI style as CANopen EDS (CiA 306): detection must tell them apart
  (CiA 306 has `[FileInfo]`, `[DeviceInfo]`, hex object index sections).
- Knowledge carried: identity, parameter list with ranges and units, assemblies, revisions and
  dates in `[File]`.
- Commonness: Rockwell ecosystem, strong in US (weak).
- Rust crates: none for ODVA EDS (`zencan-eds` 0.0.0, 2025-05-23, MPL-2.0, 554 downloads, is CANopen; `eds` 0.5.1 is "Easy data separator").
- Public samples: none with a license. Not downloaded.
- Effort: S (about 250 lines).
- Fit: properties 3, graph 0, timeline 1, tables 2.
- Recommendation: later; a CANopen EDS reader could share the INI tokenizer.

### Rockwell Studio 5000 L5X (added)

- Defining body: Rockwell Automation, "Logix 5000 Controllers Import/Export Reference Manual"
  (publication number unclear, 1756-RM084 or similar [unverified]).
- Openness: manual free [unverified]; proprietary XML, no known reader restrictions [unverified].
- Container: XML `RSLogix5000Content` > `Controller` > `DataTypes`, `Modules`, `Tags`,
  `Programs/Routines` (RLL, ST, FBD, SFC), `Tasks` [unverified element names].
- Knowledge carried: I/O module tree, tags with descriptions, logic, export date and software
  revision.
- Commonness: dominant PLC platform in North America (market claim, weak); low in DE.
- Rust crates: `l5x` 0.6.0 (2026-03-09, MIT, 620 downloads, radevgit/plc 14 stars).
- Public samples: none checked. Not downloaded.
- Effort: M.
- Fit: properties 2, graph 2, timeline 1, tables 2.
- Recommendation: later (low EU relevance).

### Siemens TIA Portal Openness XML / SimaticML / AML export

- Defining body: Siemens; documented in the "TIA Portal Openness" system manual [unverified,
  only third-party sources seen today]. TIA also exports AutomationML (CAx export) for hardware
  configuration [unverified].
- Openness: proprietary, documented by the vendor, schema version tied to TIA version (third-party
  blog claim). Readable as XML; no redistribution terms checked.
- Container: XML `Document` > `Engineering version`, `SW.Blocks.FB/FC/OB/DB` with `Interface`
  (Sections, Member with Datatype and comments), `ObjectList/CompileUnit` with `NetworkSource`
  (FlgNet LAD/FBD graph, StructuredText) and multilingual titles [unverified element names].
- Knowledge carried: block structure, interfaces, networks as graphs, comments in several
  languages.
- Commonness: Siemens S7 is the leading PLC in DE (market claim, weak).
- Rust crates: none.
- Public samples: none with a license found. Not downloaded.
- Effort: M.
- Fit: properties 2, graph 2, timeline 1, tables 2.
- Recommendation: later; the CAx export path is covered once the AutomationML reader exists.

### Beckhoff TwinCAT 3 (.tsproj, .plcproj, .TcPOU, .TcDUT, .TcGVL, .xti)

- Defining body: Beckhoff; no public format spec found (community sources only:
  https://twincat-tools.readthedocs.io/stable/pages/tools.html) [unverified].
- Openness: proprietary but plain XML; reading own project files is unproblematic, but no spec.
- Container: XML `TcPlcObject` > `POU` with `Declaration` (CDATA ST declarations) and
  `Implementation/ST` (CDATA), methods and properties; `.tsproj` holds I/O tree and tasks.
- Knowledge carried: POU structure, ST code, I/O configuration tree (EtherCAT devices), task
  setup.
- Commonness: common in DE machine building (weak).
- Rust crates: none for files (`twincat` 0.5.1 and `tcads-*` speak the ADS protocol).
- Public samples: none with a license checked. Not downloaded.
- Effort: M (needs the ST parser shared with PLCopen).
- Fit: properties 2, graph 2, timeline 1, tables 1.
- Recommendation: later (no public spec).

### CODESYS

- Defining body: CODESYS GmbH (Kempten, DE); IEC 61131-3 IDE used by many PLC vendors.
- Openness: native `.project`/`.library` is a closed binary format, no public spec [unverified];
  PLCopen XML export is the open path.
- Container: native binary; export XML per PLCopen TC6 v2.01 with CODESYS `addData` extensions.
- Knowledge carried: same as PLCopen XML (POUs, variables, FBD/LD/SFC graphs, tasks).
- Commonness: very common in DE (WAGO, Festo, Bosch Rexroth, Schneider use CODESYS [unverified]).
- Rust crates: none for native files; export via `plcopen` 0.3.1.
- Public samples: none for native files.
- Effort: blocked for native (no spec); export covered by the PLCopen reader.
- Fit: properties 2, graph 3, timeline 1, tables 2 (via the export).
- Recommendation: skip native; detect it and report "native CODESYS project, export PLCopen XML".

### NAMUR NE 100 / PROLIST

- Defining body: NAMUR (NE 100 property lists), PROLIST e.V.; property lists are delivered
  through ECLASS (ECLASS Advanced XML) [unverified].
- Openness: NE documents paid for non-members [unverified]; dictionary under ECLASS license.
- Container: ECLASS Advanced XML or vendor tool exports [unverified].
- Knowledge carried: device specification sheets (properties with units) for PCT and devices.
- Commonness: DE process industry procurement (weak).
- Rust crates: none.
- Public samples: none.
- Effort: M (ECLASS Advanced XML).
- Fit: properties 3, graph 0, timeline 0, tables 3.
- Recommendation: skip for now; AAS Technical Data covers the same need with open files.

### Area ranking

1. **AAS (AASX XML/JSON)**: CC-BY-4.0 specs and samples, German and EU policy weight, reuses the
   OPC package code, maps directly onto properties, BoM trees and document lists.
2. **AutomationML / CAEX**: open XSD and MIT samples, strong graph (InternalLink), German origin,
   and it unlocks MTP and TIA CAx exports.
3. **DEXPI**: only open P&ID format, CC-BY-4.0 reference P&IDs, best graph and equipment tables
   in the area. Plan for both Proteus 1.x and DEXPI XML 2.0.
4. **OPC UA NodeSet2**: MIT files, about 72 companion specs incl. many VDMA ones; needs the base
   NodeSet bundled for readable output.
5. **PLCopen XML**: vendor-neutral PLC code incl. CODESYS; `plcopen` crate is a usable model.
6. **IO-Link IODD**: very common and simple, waiting for licensed samples.
7. **IEC 61850 SCL**: open Apache-2.0 samples exist, rich graph and history; energy sector.
8. **MTP**: cheap once AutomationML exists.
9. **ESI, GSDML, EDS**: device descriptions with good tables, but specs are member or paid only.
10. **MTConnect, L5X, SimaticML, TwinCAT**: lower EU relevance or no public spec.
11. **NE 100/PROLIST**: skip, covered by AAS Technical Data.

### Open questions / unverified

- AAS: exact relation of IDTA v3.x to IEC 63278-1; EU DPP mandate details for AAS.
- AutomationML: license text of the whitepaper downloads; `.amlx` details live in a BPR, not in
  a free spec.
- MTP: `.mtp` container layout (manifest name, role libraries) not confirmed from a primary source.
- DEXPI: license of the Specification repo and of the RDL; no public DEXPI 2.0 XML sample.
- PLCopen: TC6 v2.01 PDF link returned 404 today; current schema download URL.
- IODD: download terms on io-link.com/downloads and IODDfinder file terms; current spec version.
- IEC 61850: iec.ch support documents page returned 403; code component licence terms.
- GSDML, ESI, EDS: schema access and any implementation terms for a reader (not just the spec).
- TIA SimaticML and TwinCAT: only third-party descriptions; element names above unverified.
- L5X manual publication number.
- `ioddengine` license mismatch (crates.io MIT vs README Apache 2.0).
- AML sample is synthetic; a real tool export (TIA CAx, EPLAN) with a license is still missing.

## 3. Construction


Date: 2026-10-19. Crate data from the crates.io API today (version, date of that version, license,
total downloads). Repo data from `gh api` today. IFC itself (Part 21, ifcZIP) is covered in
`docs/spike-2026-10-19/cad.md` and only its gaps are treated here. Samples in
`spike/corpus/engineering/construction/` (see Appendix A).

### Summary

| Format | Openness | Structure | Commonness | Crates | Samples | Effort | Fit (P/G/T/Tb) | Recommendation |
|---|---|---|---|---|---|---|---|---|
| GAEB DA XML 3.x (X31, X50 to X52, X80 to X89, X93 to X97) | XSD free download, no registration; no license text on XSDs | XML, one root `GAEB`, phase in `DP` | DE tendering default; VHB Bund form, state rules (SBN 2024, Bayern) | openbim-gaeb 0.1.2 (MIT, item summaries only) | 2 (x83, x89) | M | 2/1/2/3 | first wave |
| GAEB 90 / GAEB 2000 (D8x, P8x) | spec not found online; vendor descriptions only [unverified] | text: 80-col records / keyword tags | legacy, still met in old archives | none | none | blocked (spec); M once obtained | 2/1/1/3 | later |
| BCF-XML 2.1 / 3.0 | spec + XSD on GitHub, CC-BY-ND-4.0 | ZIP + XML (`markup.bcf`, `.bcfv`, PNG) | buildingSMART standard, broad BIM tool support | openbim-bcf (AGPL, excluded) | 1 (v3.0) | S | 2/2/3/2 | first wave |
| IDS 1.0 | XSD on GitHub, CC-BY-ND-4.0 | XML | buildingSMART final standard 2024-06 | openbim-ids 0.2.2 (MIT core) | 1 | S | 3/1/1/3 | first wave |
| ifcXML (ISO 10303-28) | free XSDs from buildingSMART [unverified for 4.3] | XML | rare next to Part 21 | ifc-xml (AGPL, excluded) | none | M (reuses IFC mapping) | 3/3/2/2 | second wave |
| ifcJSON / IFC5 (IFCX) | ifcJSON MIT community repo; IFC5 alpha, no license on repo | JSON | ifcJSON inactive since 2024-04; IFC5 alpha | none usable | none | S / M | 2/2/1/1 | later |
| bSDD references | API public; content license per dictionary | URIs inside IFC / IDS | buildingSMART service | openbim-bsdd (empty scaffold) | inside IDS sample | S (as references) | 1/2/0/0 | first wave (as part of IFC/IDS) |
| COBie 2.4 (spreadsheet) | NIBS spec, free [unverified license text] | XLSX / XML / JSON (MVD of IFC 2x3) | US NBIMS-US V3; UK code of practice BS 1192-4 now withdrawn | none | none (no clearly licensed file found) | S on calamine | 3/2/1/3 | second wave |
| CPIXML (RIB iTWO) | no public spec found | XML [unverified] | Allplan and iTWO users in DE | none | none | blocked (spec) | 0/0/0/0 (blocked, content unknown) | later (ask RIB) |
| LandXML 1.2 | free XSD at landxml.org | XML | road/rail/survey tools, widely exported | ifc-lite-landxml 20.4.0 (MPL-2.0, terrain only) | 1 | S to M | 2/2/1/3 | second wave |
| CityGML 1.0 / 2.0 / 3.0 | OGC, free | XML (GML); CityJSON as JSON | all German states publish LoD2 (NRW: 35,022 tiles, CityGML 1.0 AdV profile) | ecitygml-io (Apache/MIT, alpha), cadkit-gml (2.0, 18 days old) | 1 (3.0 versioning) | M | 3/2/2/1 | second wave |
| XPlanung (XPlanGML) 5.x / 6.x | XSD free (XLeitstelle / GDI-DE); no license text found | XML (GML 3.2) | binding in DE land-use planning (IT-Planungsrat 2017/37, secondary sources) | openbim-xplanung (empty scaffold) | none (test data has no license) | M | 3/2/1/1 | second wave |
| XBau 2.x | XÖV standard, XSD free [license unverified] | XML messages | DE building permits, OZG | openbim-xbau (empty scaffold) | none | M | 2/1/2/1 | later |
| OKSTRA 2.023 | XSD + UML free download; license not stated | XML (GML 3.2.2), CTE text | federal roads (ARS 12/2000) | openbim-okstra (AGPL, excluded) | none | L | 3/2/1/1 | later |
| DIN 276, ONORM B 1801-1 (cost structures) | paid standards; codes are facts, names are DIN/ASI text | codes inside GAEB X50/X51, IFC classifications, spreadsheets | universal in DE/AT cost planning | none | inside GAEB samples (none present) | S (code recognition) | 1/1/0/2 | first wave (as classification hints) |
| DIN SPEC 91400 | withdrawn; catalog was a free ifcXML file | ifcXML | low | none | none | S via ifcXML | 1/0/0/0 | skip |
| Revit .rvt/.rfa | closed; OLE CFB container | binary | dominant BIM authoring tool | rvt 0.6.0 (Apache-2.0, clean-room claim), cfb 0.15.0 (MIT) | none | S (metadata via cfb) / L (elements) | 2/1/1/1 | metadata second wave; elements later |
| ArchiCAD .pln | closed, no spec, no OSS reader found | binary | strong in DE/AT/HU architecture | none | none | blocked | 0/0/0/0 | skip (detect and say so) |
| ONORM A 2063 (AT) | paid standard (Austrian Standards) | XML [unverified] | AT tendering (LB-HB/LB-HT) | none | none | M (if spec bought) | 2/1/2/3 | later |
| SIA 451 / CRBX (CH) | spec not public (CRB) | fixed text / ZIP | CH tendering (NPK) | none | none | blocked | 0/0/0/0 (blocked; tables likely 3 [unverified]) | skip for now |
| CityJSON, ICDD/MMC (others) | CityJSON OGC community std (free); ICDD ISO 21597, MMC DIN SPEC 91350 [unverified] | JSON; ZIP+RDF | CityJSON growing; MMC in DE BIM-Deutschland context [unverified] | cityjson 0.7.2 (MIT/Apache); openbim-mmc (AGPL, excluded) | none | S / M | 2/2/1/1 | later |

### GAEB DA XML

- **Defining body:** GAEB (Gemeinsamer Ausschuss Elektronik im Bauwesen), a main committee of the
  DVA (Deutscher Vergabe- und Vertragsausschuss fuer Bauleistungen) since 2005, not part of DIN
  (https://de.wikipedia.org/wiki/GAEB, secondary). Current: GAEB DA XML 3.3, documentation and
  X31 package 2023-01, other packages 2021-05; 3.4 is beta (2026-03), explicitly "keine derzeit
  eingefuehrte und gueltige Version" (https://www.gaeb.de/service/downloads/gaeb-datenaustausch/).
  Phases (per Wikipedia, 3.3): X31 quantity takeoff; X50 cost catalogue, X51 cost estimation,
  X52 calculation data; X80 BoQ catalogue, X81 specification, X82 cost estimate, X83 request for
  bid, X84 bid, X84P price comparison, X85 alternative bid, X86 award, X87 order confirmation,
  X89 invoice; Z variants for time contracts; X93 to X97 trade (price inquiry, offer, order,
  confirmation).
- **Openness:** XSDs and Fachdokumentation (PDF) downloadable without registration or fee from
  gaeb.de. The download page has no license text for the schemas. A third-party repo that ships
  them states "per GAEB's guidance the schema files must not be modified" and that rights stay
  with GAEB/DIN (https://github.com/bambamboole/gaeb-php/blob/main/docs/gaeb/README.md, weak).
  Result: reading GAEB files is unencumbered; redistributing the XSDs is unclear, so Rezzer
  should not bundle them (it does not need them to read). No patents known.
- **Container:** XML, root `GAEB` with namespace `http://www.gaeb.de/GAEB_DA_XML/DA83/3.3`
  (phase number in the namespace and in `Award/DP`). Key elements: `GAEBInfo` (Version,
  VersDate, Date, ProgName), `PrjInfo` (NamePrj, LblPrj, Cur), `Award/AwardInfo`, `OWN`/`CTR`
  addresses, `BoQ/BoQInfo/BoQBkdn` (outline number scheme), nested `BoQBody/BoQCtgy` (RNoPart,
  LblTx), `Itemlist/Item` (RNoPart, Qty, QU, UP, IT, `Description/CompleteText/DetailTxt` with
  XHTML-like `p`/`span`, `TextComplement` owner/bidder blanks, `OutlineText`, `SubDescr`),
  invoice `Invoice` with progress quantities and VAT. Seen in the two samples.
- **Knowledge carried:** BoQ tree (lots, sections, items) with outline numbers; quantities with
  units; unit and total prices, currency, VAT; parties (owner, contractor); text complements
  (blanks the bidder fills); cost groups (DIN 276 codes in X50/X51 and `CtlgAssign` catalogue
  references [unverified for all phases]); phase chain X83 -> X84 -> X86 -> X89 gives a dated
  timeline of one project; references to catalogues (STLB-Bau) as IDs.
- **Commonness:** Procurement: VHB Bund form 244 says that if offer data is exchanged
  electronically it follows GAEB DA XML (2017 form, conditional, found via evergabe.de search,
  weak). Niedersachsen SBN AVA-Pflichtenheft 2024-01 requires GAEB XML 3.2 exclusively for
  X81 to X86 and forbids 3.0, 3.1 and (still) 3.3
  (https://nlbl.niedersachsen.de/download/208208/AVA-Pflichtenheft_des_SBN_Version_2024-01.pdf).
  Bayern: tender specs must conform to DA 83 (search result, weak). VOB/A itself does not name
  GAEB (no source found that it does). GitHub code search (extension + "GAEB"): x83 76, x81 34,
  x86 20, x84 10 files (weak, counts today). Validator for 2.0 to 3.3 hosted by BVBS. Germany:
  very high; EU outside DE/AT: low.
- **Rust crates:** openbim-gaeb 0.1.2 (2026-08-25, MIT, 106 downloads, 1,555 lines, deps
  quick-xml 0.37 + thiserror only, repo openbimrs/gaeb, 1 star). Detects 3.1 to 3.4, exposes an
  `Item` summary (id, outline number, qty, unit, unit price, total, description, category path)
  and lossless Qty edits; no text complements, parties, invoice, X31. `gaeb` 0.1.2 is a re-export.
  Other OSS: pygaeb (Python, MIT, 21 stars), gaeb-php (PHP, MIT, 1 star), meindonut/gaeb-parser
  (MIT, 10 stars), Dangl AVA (.NET, commercial).
- **Public samples:** gaeb-php `tests/fixtures/*` (MIT, self-authored synthetic, schema-valid
  against 3.3 per its README); downloaded `boq.x83` and `invoice.x89`. Not used: GAEB's own
  Musterdateien (3.1, on gaeb.de, no license) and BVBS Pruefdateien that sit in MIT repos
  (GaebToolboxVersionDiff, meindonut/gaeb-parser): the repo license does not cover these
  third-party files.
- **Effort:** M (about 500 to 800 lines on quick-xml): one reader for all phases (shared BoQ
  structure), formatted text to Rezzer paragraphs, money and quantity typing, X31 and X89 extras.
  Reuses the quick-xml layer of the OOXML/3MF readers.
- **Fit:** properties 2, graph 1 (BoQ tree, party roles, catalogue refs), timeline 2 (phase,
  dates, invoice sequence numbers), tables 3 (BoQ as cost lines).
- **Recommendation:** first wave. The single most important construction format in Germany and
  a near-perfect "tables with context" case.

### GAEB 90 and GAEB 2000

- **Defining body:** GAEB. GAEB 90 (1990) and GAEB 2000 (1999) are the ASCII predecessors of
  DA XML (Wikipedia, secondary).
- **Openness:** No official spec found online today [unverified]. GAEB 90: 80-character fixed
  records, record type in columns 1 to 2, line number in the last 6 columns, e.g. type 25 short
  text, 26 long text (streit-software.de, dangl-it.com, vendor pages, weak). GAEB 2000 (`P8x`):
  keyword-tagged text with formatting and attachments (vendor pages, weak).
- **Container:** text (codepage likely CP437/850 for GAEB 90 [unverified]).
- **Knowledge carried:** same BoQ content as X8x, less metadata.
- **Commonness:** legacy; SBN 2024 forbids anything older than XML 3.2 for new work. Still in
  archives of public owners (weak, no number).
- **Crates/OSS:** none in Rust. GAEB 90 readers exist in commercial AVA tools.
- **Samples:** none; GitHub code search for `extension:d83 GAEB` gave 0.
- **Effort:** blocked (spec not obtainable online today); M each once a spec copy is obtained
  [unverified where GAEB hosts old docs].
- **Fit:** 2/1/1/3. **Recommendation:** later; report "GAEB 90/2000 not read" with the detected
  phase from the extension.

### BCF-XML (BIM Collaboration Format) 2.1 and 3.0

- **Defining body:** buildingSMART International, BCF-XML 3.0 released 2021-06-18, 2.1
  2017-01-16 (GitHub releases of buildingSMART/BCF-XML). BCF-API is the REST sibling. Managed by
  the Open CDE group.
- **Openness:** Spec, XSDs and test cases on GitHub under CC-BY-ND-4.0 (LICENSE at pinned SHA
  bc48611). No registration. Reading and writing files needs no license; the ND clause matters
  only for redistributing modified spec or test files.
- **Container:** ZIP (`.bcf`, 2.0 used `.bcfzip`): `bcf.version`, `project.bcfp`,
  `extensions.xml` (3.0), `documents.xml` (3.0), one folder per topic GUID with `markup.bcf`,
  `Viewpoint_*.bcfv`, `Snapshot_*.png`. Sample has all of these (`unzip -l`).
- **Knowledge carried:** issues (Topic: GUID, status, type, priority, stage/milestone, labels,
  due date, assigned to, title, description), comments with author and date, viewpoints with
  camera and selected/hidden IFC GlobalIds (links to IFC elements), header files with model
  references and dates, related topics, document references. Strong timeline (CreationDate,
  ModifiedDate per topic and comment).
- **Commonness:** buildingSMART standard supported by most BIM coordination tools (Solibri,
  BIMcollab, Revit/ArchiCAD plugins; buildingSMART software list not fetched, site returned 403,
  so [unverified]). BCF-XML repo 262 stars. Requested in German BIM AIA templates [unverified].
- **Rust crates:** openbim-bcf 0.6.1 (2026-10-19, AGPL-3.0-or-later): excluded by license,
  source not read. axioval-bcf 0.7.0 (AGPL): excluded. No permissive crate.
- **Samples:** downloaded `bcf/Milestone.bcf` (v3.0, 61 KB). Many more in the same repo
  (v2.0, v2.1, v3.0: labels, due date, user assignment, document refs, related topics).
- **Effort:** S (about 300 lines on zip + quick-xml, versions 2.0 to 3.0 differ in a few
  elements). Snapshots go to the needs-vision list like other images.
- **Fit:** properties 2, graph 2 (topic -> IFC GlobalIds, related topics, documents),
  timeline 3, tables 2 (issue list).
- **Recommendation:** first wave. Small, open, high knowledge density, and links IFC elements to
  dated discussions, which is exactly the "comments stay attached" principle.

### IDS (Information Delivery Specification) 1.0

- **Defining body:** buildingSMART International; v1.0.0 released 2024-06-03 (GitHub release).
  ISO track [unverified].
- **Openness:** `Schema/ids.xsd` and examples on GitHub, CC-BY-ND-4.0 (pinned SHA 870f9c4).
  No registration.
- **Container:** XML, `ids:ids` with `info` (title, copyright, version, date, milestone, author)
  and `specifications/specification` (ifcVersion, name) each with `applicability` and
  `requirements` facets: entity, attribute, property (with bSDD `uri`), classification,
  material, partOf; values as simple values or `xs:restriction` (enumeration, pattern, bounds).
- **Knowledge carried:** required properties with data types and allowed values, classification
  requirements, part-of relations, cardinality; a milestone and date. It is a requirements table,
  not model data.
- **Commonness:** buildingSMART final standard; repo 325 stars; examples from NL (NBvT,
  BIM-basis-ILS) and DE (Aachen). Adoption in tenders growing [unverified, no number].
- **Rust crates:** openbim-ids 0.2.2 (2026-10-04, MIT, 740 downloads, 7,163 lines). Default
  features depend only on openbim-core (MIT) and roxmltree; features `audit-schema` / `audit`
  pull ifc-schema (AGPL) and ifc-template-catalog (AGPL + CC-BY-ND): never enable them. Repo
  openbimrs/ids, 0 stars, one org, 8 releases in 6 weeks: young.
- **Samples:** downloaded `ids/IDS_wooden-windows.ids` (13 KB; copyright field "NBvT", inside
  the CC-BY-ND repo; has a bSDD property URI, 6 IFC2X3 specifications).
- **Effort:** S (about 250 lines, own reader on quick-xml; or openbim-ids without features
  behind a trait). Own reader preferred: the facet model is small and Rezzer needs it rendered as
  a table, not validated.
- **Fit:** properties 3, graph 1, timeline 1, tables 3.
- **Recommendation:** first wave (cheap, open, clean requirements table).

### ifcXML (ISO 10303-28)

- **Defining body:** ISO 10303-28 (XML representation of EXPRESS data); buildingSMART publishes
  the ifcXML XSDs alongside IFC4 and IFC4.3 (ISO 16739-1) [unverified for the 4.3 URL].
- **Openness:** XSDs free with the IFC documentation; ISO 10303-28 text itself is paid, but the
  XSD is enough to read files [unverified].
- **Container:** XML, root `ifcXML` (IFC4) with entities as elements and references by
  `id`/`ref` attributes [from knowledge, unverified]; same entity graph as Part 21.
- **Knowledge carried:** identical to IFC: spatial tree, elements, psets, quantities,
  materials, classifications, relations, owner history.
- **Commonness:** low next to Part 21 (weak; no count). DIN SPEC 91400 catalogue was shipped as
  ifcXML.
- **Rust crates:** ifc-xml 0.4.1 (2026-10-03, AGPL-3.0-or-later): excluded, source not read.
  openbim-ifc-cli (AGPL) also reads ifcXML: excluded.
- **Public samples:** none downloaded (none in buildingSMART Sample-Test-Files today).
- **Effort:** M (about 400 lines XML front end that produces the same entity records as the
  Part 21 tokenizer, then the existing IFC mapping of 354 + 308 lines).
- **Fit:** 3/3/2/2. **Recommendation:** second wave.

### ifcJSON and IFC5 / IFCX (later)

- **ifcJSON:** community spec, buildingsmart-community/ifcJSON, MIT, 167 stars, last push
  2024-04-08. Not a buildingSMART standard. Later.
- **IFC5 / IFCX:** buildingSMART/IFC5-development, alpha examples, no license on the repo,
  last push 2026-09-08, 236 stars. JSON based. Watch, later. openbim-ifcx-geometry exists but
  the openbimrs IFC crates are AGPL.

### bSDD references

- **Defining body:** buildingSMART Data Dictionary (repo buildingSMART/bSDD, MIT for docs and
  code, 196 stars). Online API; each dictionary (e.g. IFC, ETIM, national ones) has its own
  content license [unverified per dictionary].
- **In files:** URIs such as `http://identifier.buildingsmart.org/uri/buildingsmart/ifc-2x3/prop/acousticrating`
  in IDS (`uri` attribute, seen in sample), IFC `IfcClassificationReference.Location`, and
  property `Specification` [unverified].
- **Rezzer use:** keep URIs as typed references with the dictionary namespace parsed from the
  path; no network lookup (no feature may depend on a remote service by default). Shipping
  dictionary content would need a per-dictionary license check: do not ship.
- **Commonness:** bSDD URIs are the standard way IDS 1.0 points to properties and
  classifications (seen in the sample); IFC4.3 classification references use them [unverified
  share]. Repo 196 stars.
- **Crates:** openbim-bsdd 0.1.0 / bsdd 0.1.0: reserved scaffold, no function.
- **Samples:** no separate file; `ids/IDS_wooden-windows.ids` carries one bSDD URI.
- **Effort:** S. **Fit:** 1/2/0/0. **Recommendation:** first wave, as part of IFC and IDS.

### COBie

- **Defining body:** NIBS / buildingSMART alliance (US); COBie 2.4 in NBIMS-US V3 (2015)
  (Wikipedia, secondary). An MVD of IFC 2x3, carried as spreadsheet (XLSX), IFC, or COBie-lite
  XML/JSON.
- **Openness:** spec freely readable from NIBS; "curated under a Creative Commons License"
  per search result, exact license [unverified].
- **Container:** XLSX with fixed sheets: Contact, Facility, Floor, Space, Zone, Type, Component,
  System, Assembly, Connection, Spare, Resource, Job, Impact, Document, Attribute, Coordinate,
  Issue, PickLists [from knowledge, unverified against the 2.4 text].
- **Knowledge carried:** asset register: facility/floor/space tree, types and components with
  manufacturer, model, warranty, serials; systems and zones (graph); Attribute sheet with units;
  Document links; CreatedBy/CreatedOn per row (timeline, weak); Job sheet for maintenance.
- **Commonness:** UK: BS 1192-4:2014 (COBie code of practice) is withdrawn and superseded by
  BS EN ISO 19650-4; COBie moves to the UK national annex
  (https://knowledge.bsigroup.com/products/collaborative-production-of-information-fulfilling-employer-s-information-exchange-requirements-using-cobie-code-of-practice).
  The 2016 UK "BIM Level 2" mandate required COBie on central government projects [unverified,
  no primary source fetched]. US federal (USACE, GSA) use [unverified]. Germany: low.
- **Crates/OSS:** no Rust crate. xBim (C#, CDDL), opensourceBIM COBie plugins (Java, no license),
  IBM MaximoCOBieParser (EPL-1.0).
- **Samples:** none downloaded. XbimExchange (CDDL) holds Lakeside Restaurant and Duplex COBie
  files, but these come from US Army ERDC / NIBS and the repo license does not cover them; WBDG
  "Common BIM Files" page states no license (https://wbdg.org/node/9165). theNBS/COBie-Test-Files
  has no license. A self-authored COBie file would be the clean option.
- **Effort:** S (about 300 lines) on the existing calamine XLSX reader: sheet and column mapping,
  build tree and system graph. COBie in IFC is just IFC.
- **Fit:** properties 3, graph 2, timeline 1, tables 3. **Recommendation:** second wave (cheap,
  but low DE relevance; write a synthetic sample first).

### CPIXML (RIB iTWO)

- **Defining body:** RIB Software (iTWO). Allplan documents "CPIXML file 2017" export plus older
  2.3 (https://help.allplan.com/Allplan/2020-0/1033/New/254558.htm).
- **Openness:** no public spec or XSD found; RIB developer docs not reachable [unverified].
  Openness unknown.
- **Container:** XML [unverified]; carries model elements with quantities for iTWO 5D costing
  [unverified].
- **Commonness:** iTWO is common with large German contractors (weak, no number).
- **Crates/OSS:** none. **Samples:** none.
- **Effort:** blocked until a spec is available (reverse-engineering XML from samples is possible
  but no licensed sample exists). **Recommendation:** later; ask RIB for the spec.

### LandXML 1.2

- **Defining body:** LandXML.org industry consortium. 1.2 of 2008-07-29 is the last release;
  2.0 is a working draft of 2014-06-20 (http://www.landxml.org/, fetched today).
- **Openness:** XSD and docs free to download; no license text checked [unverified].
- **Container:** XML, root `LandXML` with `Units`, `Application`, `CoordinateSystem`, `Project`,
  `Alignments/Alignment/CoordGeom` (Line, Curve, Spiral), `Profile/ProfAlign`, `Cant`,
  `Surfaces/Surface` (TIN), `Parcels`, `CgPoints`, `PipeNetworks`, `Roadways` [from schema
  knowledge; sample shows Units, Application, Alignments, CoordGeom, Profile, Cant].
- **Knowledge carried:** alignments with stations, geometry elements and lengths (as tables),
  profiles, cant, surfaces (counts and bbox, not geometry), parcels, pipe networks (graph:
  structures and pipes), units and the authoring application.
- **Commonness:** exported by Civil 3D, OpenRoads, 12d, RailCOMPLETE and many survey tools
  (sample headers); GitHub code search `"LandXML-1.2" "<Alignment " extension:xml` = 88 files
  (weak). Used in DE road and rail design [unverified].
- **Rust crates:** ifc-lite-landxml 20.4.0 (2026-10-11, MPL-2.0, 2,454 downloads): terrain
  ingestion for ifc-lite, not a general model. jp-landxml (Rust, no license, 3 stars): not usable.
- **Samples:** downloaded `landxml/railcomplete-005-alignment-with-cant.xml` (RailCOMPLETE AS,
  MIT). Alternative: lekks/tin2dem (MIT), Civil 3D TIN only.
- **Effort:** S to M (about 400 lines): alignments, profiles, parcels, pipe networks as tables
  and graphs; surfaces summarized.
- **Fit:** properties 2, graph 2, timeline 1, tables 3. **Recommendation:** second wave.

### CityGML (1.0, 2.0, 3.0) and CityJSON

- **Defining body:** OGC. CityGML 3.0 Conceptual Model (2021) and GML encoding (2023)
  [dates unverified]; repos opengeospatial/CityGML-3.0CM (MIT, 94 stars) and CityGML-3.0Encodings
  (MIT, 40 stars). CityJSON is an OGC community standard [unverified].
- **Openness:** free spec and XSDs.
- **Container:** XML (GML 3.1/3.2). NRW data (one tile header read) use the AdV CityGML profile
  with CityGML 1.0 namespaces; other states [unverified] (`http://www.opengis.net/citygml/1.0`, AdV schema locations at
  repository.gdi-de.org), seen today in NRW tile `LoD2_32_280_5660_1_NW.gml` (header read only).
- **Knowledge carried:** city object tree (CityModel -> Building -> BuildingPart -> surfaces,
  3.0 adds Storey and BuildingUnit), generic attributes with units, addresses (xAL),
  external references to cadastre IDs, function/usage codes (code lists), 3.0 Versioning
  (Version, VersionTransition, creation/termination dates) and Dynamizer time series.
- **Commonness:** NRW alone publishes 35,022 LoD2 CityGML tiles (opengeodata.nrw.de index,
  counted today); all German states publish LoD2 as open data [unverified for every state;
  NRW license dl-de/zero-2.0 unverified]. Germany: high for planning and energy (Waermeplanung).
- **Rust crates:** ecitygml-io 0.0.2-alpha.7 (2026-10-14, MIT OR Apache-2.0, 6,508 downloads,
  CityGML 3.0 model, version detection, TUM, 7 stars, alpha); cadkit-gml 0.3.4 (2026-10-19, MIT
  OR Apache-2.0, 253 downloads, CityGML 2.0 only, first release 2026-10-01); cityjson 0.7.2
  (2026-04-22, MIT OR Apache-2.0, 295 downloads). openbim-citygml: scaffold, no function.
  Neither ecitygml (3.0 model; "version detection" does not prove it reads 1.0/2.0) nor
  cadkit-gml (2.0 only) is confirmed on the main German case, CityGML 1.0 AdV [unverified]. This
  supports an own GML layer.
- **Samples:** downloaded `citygml/historicalSuccession_CityGML_3.0_LOD2_Versioning.gml`
  (88 KB, OGC repo, MIT). Same repo and SHA also has a 12 KB CityGML 2.0
  `Examples-from-CityGML-2.0/building/CityGMLv2/Building_LOD2-EPSG25832.gml` if 2.0 coverage is
  wanted. A real NRW tile (68 KB) is an option once its license is confirmed.
- **Effort:** M (about 700 lines): one GML-aware reader for 1.0/2.0/3.0 namespaces, object tree,
  attributes, code lists by value, versioning; geometry only as counts and bbox. The GML layer is
  shared with XPlanung and OKSTRA.
- **Fit:** properties 3, graph 2, timeline 2 (3.0 versioning, creation dates), tables 1.
- **Recommendation:** second wave; build the GML layer once for CityGML, XPlanung, OKSTRA.

### XPlanung (XPlanGML)

- **Defining body:** XLeitstelle Planen und Bauen (Hamburg). Versions listed up to 6.1
  (https://xleitstelle.de/xplanung/releases-xplanung); schemas at
  https://repository.gdi-de.org/schemas/de.xleitstelle.xplanung/.
- **Openness:** schemas free; no license text found on xleitstelle.de pages fetched today
  [unverified]. Test data on gitlab.opencode.de/xleitstelle/xplanung/testdaten has no license
  file (README checked), so not downloaded.
- **Container:** GML 3.2 application schema; XPlanArchiv = ZIP with GML plus raster plans (PNG +
  world file) and PDF attachments (testdaten README).
- **Knowledge carried:** plan metadata (plan type BP/FP/RP/LP, legal dates: Aufstellungsbeschluss,
  Satzungsbeschluss, Inkrafttreten), plan areas, zoning objects with legal attributes (GRZ, GFZ,
  number of storeys, use), text clauses (Textabschnitte) linked to objects, references to
  external documents.
- **Commonness:** "gesetzlich verbindlicher Datenstandard" (xleitstelle.de); IT-Planungsrat
  decision 2017/37 made XPlanung and XBau binding, new plans in XPlanung from Feb 2023
  (Stadt Koeln Pflichtenheft, Stadt Muenster page: secondary; decision text not fetched).
- **Crates:** openbim-xplanung 0.1.0 and xplanung 0.1.0: 25-line scaffold, no function.
  openbim-xtrasse same org.
- **Samples:** none (license missing).
- **Effort:** M on the shared GML layer. **Fit:** 3/2/1/1 (legal dates are a weak timeline).
- **Recommendation:** second wave (legal mandate in DE, open schema).

### XBau

- **Defining body:** XLeitstelle; XOeV family; versions on xrepository.de (not fetched).
- **Openness:** XSDs downloadable (e.g. `xbau-kernmodul-basisdatentypen.xsd` on
  xleitstelle.de/download); license not stated [unverified].
- **Container:** XML messages (building permit application, notices, decisions) [message list
  unverified].
- **Knowledge carried:** procedure metadata, parties, dates, attached documents (references).
- **Commonness:** recommended by Bauministerkonferenz 2005; binding with XPlanung via IT-PLR
  2017/37 (secondary). OZG building permit services use it [unverified].
- **Crates:** openbim-xbau 0.1.0: scaffold. **Samples:** none.
- **Effort:** M. **Fit:** 2/1/2/1. **Recommendation:** later (messages between authorities
  are rarely files a user hands to an agent).

### OKSTRA

- **Defining body:** BASt / BMV, maintained by interactive instruments GmbH (site meta tag).
  Version 2.023 adopted 2026-05-27 (https://www.okstra.de/okstra.html).
- **Openness:** XML schema (`okstra.xsd` plus sub-schemas), UML/XMI and docs downloadable from
  https://www.okstra.de/schema.html; license not stated [unverified].
- **Container:** OKSTRA-XML with GML 3.2.2 geometry; also CTE text format (BBSoft page, weak).
- **Knowledge carried:** road network (nodes, sections, stations), road inventory, accidents,
  structures, Schluesseltabellen (code lists).
- **Commonness:** introduced for federal trunk roads by ARS 12/2000, recommended for state and
  municipal roads (Brandenburg VV, bravors.brandenburg.de).
- **Crates:** openbim-okstra 0.1.0 (AGPL, excluded). **Samples:** none.
- **Effort:** L (very large schema). **Fit:** 3/2/1/1. **Recommendation:** later.

### DIN 276 and ONORM B 1801-1 (cost structures)

- **Defining body:** DIN 276:2018-12 (DIN, paid); ONORM B 1801-1:2022-03-01 (Austrian
  Standards, paid; https://austrian-standards.at/en/shop/onorm-b-1801-1-2022-03-01~p2867393).
- **Openness:** the codes (e.g. 300, 330, 331) are facts; the group names and descriptions are
  standard text. Rezzer can recognize and pass through codes and names found in a file, but
  should not ship the name tables.
- **In files:** GAEB X50/X51/X52 cost data and `CtlgAssign` in BoQs, IFC
  `IfcClassificationReference` (Identification "331"), spreadsheets with a code column.
- **Effort:** S (code pattern detection and level grouping: 3-digit DIN 276, letter/number
  ONORM). **Fit:** 1/1/0/2. **Recommendation:** first wave as classification hints inside the
  GAEB and IFC readers, not as a format.

### DIN SPEC 91400

- BIM classification based on STLB-Bau; 2017-02 edition shown as withdrawn on din.de; the
  catalogue was a free ifcXML file from din-bauportal.de
  (https://din.de/en/wdc-beuth:din21:267208318, search summary). STLB-Bau content itself is
  licensed (Dr. Schiller & Partner) [unverified]. **Recommendation:** skip; references to
  STLB-Bau in GAEB items are kept as catalogue IDs.

### Revit (.rvt, .rfa, .rte, .rft)

- **Defining body:** Autodesk, closed.
- **Openness:** no spec. Container is OLE CFB, readable with `cfb` 0.15.0 (2026-09-18, MIT,
  74.5 M downloads).
- **Knowledge in the file (per rvt-rs README, not independently verified):** `BasicFileInfo`
  stream (Revit version, build, GUID, original path, worksharing state, central model path,
  last-saved-by user, save counter), `PartAtom` XML (title, OmniClass code, taxonomies) in
  families, PNG preview, `Formats/Latest` schema; element partitions (walls, doors, storeys,
  materials) decoded for 2023 to 2025, experimental for 2015 to 2022, 2026, 2027.
- **Rust crates:** rvt 0.6.0 (2026-10-19, Apache-2.0, 217 downloads, 5 releases since
  2026-09-27, repo DrunkOnJava/rvt-rs 55 stars, claims clean-room via CLEANROOM.md
  [unverified]). Large and moving fast; deps include clap, js-sys (not needed in a library).
- **Commonness:** dominant BIM authoring tool (vendor market claim, weak; no number fetched).
- **Samples:** none with a clear license (Autodesk sample projects are Autodesk content).
- **Effort:** S for metadata (cfb + UTF-16 BasicFileInfo + PartAtom XML, about 200 lines own
  code); L for elements (only via rvt crate behind a trait, after a spike with its own corpus).
- **Fit:** 2/1/1/1 (metadata only); higher with rvt elements.
- **Recommendation:** metadata reader second wave; element extraction later, after testing rvt
  on real files like the CAD spike did with other crates.

### ArchiCAD (.pln, .pla)

- **Defining body:** Graphisoft (Nemetschek), closed. Help pages describe contents (model,
  views, attributes, library references) but not the layout
  (https://helpcenter.graphisoft.com/user-guide/89256).
- **Openness:** no spec, no OSS reader found (search today). Signature hints only from a
  third-party site (weak).
- **Effort:** blocked. **Recommendation:** skip; detect by extension/signature and report
  "ArchiCAD project, not readable, export IFC".

### ONORM A 2063 (Austria)

- **Defining body:** Austrian Standards, ONORM A 2063-1:2021-03-15 (data exchange in AVA:
  specifications, element catalogue, billing)
  (https://www.austrian-standards.at/en/shop/onorm-a-2063-1-2021-03-15~p2571129). Paid.
- **Openness:** standard paid; whether its XSD is published separately [unverified].
- **Container:** XML (2021 edition) [unverified]; older ONORM B 2063 text format.
- **Commonness:** BMWET standard tender texts (LB-HB, LB-HT) are based on ONORM A 2063-1:2021;
  BVergG 2018 section 105(3) asks to consider ONORMs (search summary, weak). Mandatory for
  electronic quantity proof per a vendor (DATAflor wiki, weak).
- **Crates/samples:** none. **Effort:** M once the spec is bought. **Fit:** 2/1/2/3.
- **Recommendation:** later (AT market; needs a paid spec).

### SIA 451 and CRBX (Switzerland)

- **Defining body:** SIA / CRB (NPK catalogue). IfA18 guideline (since 2018-01) changed SIA 451
  exchange (swissmadesoftware.org, weak). CRBX is a ZIP container (filext.com, weak).
- **Openness:** no public spec found; IfA18 docs likely for licensed software makers [unverified].
- **Recommendation:** skip for now; revisit if a Swiss user asks.

### Others relevant in DE/EU

- **CityJSON:** JSON encoding of CityGML; crate cityjson 0.7.2 (MIT OR Apache-2.0). S on
  serde_json. Later.
- **ICDD (ISO 21597) and Multimodell-Container (DIN SPEC 91350):** ZIP with RDF linksets that
  connect IFC, GAEB, schedules; used in German BIM-Deutschland context [unverified].
  openbim-mmc 0.1.0 is AGPL (excluded). Would make the cross-document graph explicit. Later,
  but worth a spike because it links exactly the formats above.
- **LOIN (EN 17412-3 / ISO 7817-3):** openbim-loin 0.3.2 (MIT). Requirements like IDS. Later.

### Area ranking

1. **GAEB DA XML** - highest DE relevance, open XSD, own reader M, tables with context.
2. **BCF-XML** - S effort, dated issues and comments tied to IFC GlobalIds (timeline + graph).
3. **IDS** - S effort, clean requirements table, bSDD links.
4. **DIN 276 / ONORM B 1801 codes and bSDD URIs** - cheap enrichment inside GAEB/IFC/IDS.
5. **CityGML (with a shared GML layer)** - large public German open data, M.
6. **XPlanung** - legal mandate, reuses the GML layer.
7. **LandXML** - S to M, alignments and pipe networks as tables.
8. **COBie** - S on calamine, but low DE relevance and no licensed sample yet.
9. **ifcXML** - reuses IFC mapping, rare in practice.
10. **Revit metadata** - S via cfb; elements later after a crate test.
11. Later: OKSTRA, XBau, GAEB 90/2000, ONORM A 2063, CPIXML, ICDD/MMC, IFC5, CityJSON.
12. Skip: ArchiCAD, SIA 451/CRBX, DIN SPEC 91400.

### Open questions / unverified

- GAEB XSD redistribution terms (no license text on gaeb.de); GAEB 90/2000 official specs.
- Current VHB Bund wording on GAEB (only a 2017 form was seen); legal basis in VOB/A.
- IT-Planungsrat decision 2017/37 text not fetched (only city documents cite it).
- XPlanung, XBau, OKSTRA, LandXML schema license texts; XPlan test data has no license.
- buildingSMART technical pages returned 403 (tool lists, IDS ISO status, BCF software list).
- COBie 2.4 license text and sheet list; UK 2016 mandate wording; no clean COBie sample.
- CPIXML spec availability (ask RIB / Nemetschek).
- NRW CityGML license (dl-de/zero-2.0 assumed); state-by-state CityGML versions.
- rvt crate clean-room claim and real-file behavior; Revit stream names beyond its README.
- CC-BY-ND-4.0 samples (BCF, IDS): verbatim redistribution is fine, but expected-output
  reference files derived from them may count as adaptations. Decision for Jordi.
- ONORM A 2063 XML schema availability; SIA 451 / CRBX spec.

## 4. Electrical


Date: 2026-10-19. Area slug `electrical`. Corpus: `spike/corpus/engineering/electrical/` (5 files, see
Appendix A). Crate data from the crates.io API today. Everything without a primary
source is marked [unverified].

### Headline findings

1. **Licensing of dictionaries is the key issue, not the file formats.** The file formats in this
   area are open or easy to read. The dictionaries behind the codes are not all open:
   - **ECLASS** (Terms of Use 4.3, 2022-03-22, still the newest on eclass.eu): a recipient of a
     file with ECLASS content ("indirect obtainment", §2.1 (4)) may store it, use it in its own
     processes and convert it into internal software (§4.1.2 (1)-(2)). It may **not** append
     further ECLASS content (§4.1.3). So Rezzer may print IRDIs, class codes and the names that
     are in the file, but must **not** resolve codes against a bundled ECLASS dictionary or the
     paid ECLASS web service (0.05 EUR per IRDI). When ECLASS content is passed on, §5.3 gives a
     notice text: "This content contains ECLASS. The ECLASS Terms of Use apply (www.eclass.eu)."
     Recommendation: Rezzer writes this line into the output whenever it finds ECLASS content.
     Source: https://eclass.eu/fileadmin/Redaktion/pdf-Dateien/Sonstige_Dateien/ECLASS_terms-of-use_4-3.pdf
   - **ETIM**: the English master model is under the Open Data Commons Attribution License 1.0
     (ODC-BY). It could be bundled with attribution, so Rezzer could resolve EC/EF/EV/EU codes to
     English names offline. That is a decision for Jordi. The German translation (de-DE) needs
     national ETIM membership. The ETIM xChange format (schema, code lists) is Apache-2.0.
     Source: https://www.etim-international.com/classification/license-info/
   - **IEC CDD** (IEC 61360 dictionaries, incl. 62683DB): content is IEC copyright. The licence
     page returned HTTP 403 to us. A search excerpt says identifier, version, revision and name
     may be reproduced freely [unverified].
2. **EPLAN has a readable open path.** "Publish > Smart Production" writes a `.epdz`: a 7z archive
   with a SQLite `manifest.db`, SVG pages and an AutomationML file. We checked a public sample
   (see EPLAN below). The AML file carries ECLASS role classes, 81346 designations and connections.
3. **IEC 81346 reference designations are the best value per line in this area.** Strings such as
   `=GB1+A1-U91` and `==EES==Page_macros++Infrastructure&EFS#01/1` (81346 aspects plus an IEC 61355
   document kind) appear in EPLAN pages, AML, terminal lists, Excel exports and DXF block
   attributes. One small parser (S) turns them into a plant tree for all formats.
4. **VDI/VDE 3685 is not a reference designation guideline.** It covers adaptive controllers
   (sheet 1 1990, sheet 2 1992; https://store.accuristech.com/ashrae/subgroups/24669). The link
   in the task is not supported. What was meant is [unverified]; likely DIN EN IEC 81346 itself or
   VGB-S-823 (RDS-PP) [unverified].

### Summary table

| Format | Openness | Structure | Commonness | Crates | Samples | Effort | Fit (P/G/T/Tb) | Recommendation |
|---|---|---|---|---|---|---|---|---|
| BMEcat 2005 / 2005.2 (incl. ECLASS and ETIM guidelines) | spec + XSD free, free right of use | XML | very high in DE B2B catalogs | none in Rust | 1 downloaded (MIT); Weidmueller file not kept | S-M | 3/1/1/3 | first wave |
| ETIM xChange 2.0 (JSON) | Apache-2.0, free download | JSON + JSON Schema | rising, ETIM International standard | none | 1 downloaded (official) | S | 3/2/2/3 | first wave |
| ECLASS codes in files (Basic / Advanced, IRDI) | reading codes in files OK; dictionary licensed | inside BMEcat, AML, xChange, Excel | very high in DE/EU | none | in 2 corpus samples (+ AAS) | S (part of host readers) | 3/1/1/1 | first wave (as a mapping layer) |
| ETIM codes in files (EC/EF/EV/EU/MC) | model ODC-BY (EN), translations gated | inside BMEcat, xChange | very high in electrical wholesale | none | in xChange sample | S | 3/1/1/1 | first wave (mapping layer) |
| DATANORM 4 / 5 | spec not free [unverified owner] | text, records `V/A/B/T/P/C...` | very high in DE trade (electrical, SHK) | none in Rust | 2 downloaded | S | 2/1/1/3 | first wave |
| KBL 2.x / VEC 2.x (wire harness, prostep ivip and VDA) | spec, XSD and wiki MIT, free | XML | standard OEM-supplier harness exchange in DE automotive | none in Rust (Java harness-model, MIT) | 2 downloaded (MIT) | KBL S-M, VEC M | 3/3/1/3 | first wave |
| ELDANORM | no public spec [unverified] | text [unverified] | legacy, electrical wholesale | none | none | blocked | 2/0/0/2 | later |
| IEC 81346 designations (parsing convention) | standard paid; syntax facts widely documented | string grammar | very high (every ECAD plan) | none | in EPDZ sample | S | 2/3/0/2 | first wave |
| IEC 61355 DCC (document kind) | standard paid; public code database | code in titles/metadata | high in plant documentation | none | in EPDZ sample | S | 2/1/0/1 | first wave (with 81346 parser) |
| EPLAN `.epdz` (Smart Production) | undocumented, community notes (MIT) | 7z + SQLite + SVG + AML + E3D | EPLAN is market leader in DE [weak] | sevenz-rust2, rusqlite | 1 public sample (6.5 MB, not copied) | M | 3/3/1/2 | second wave |
| EPLAN `.edz` (Data Portal parts) | undocumented; EDZ export needs EPLAN licence module | 7z + `*.part.xml` + macros | very high among EPLAN users | sevenz-rust2 | none (redistribution forbidden) | M | 3/1/1/2 | second wave |
| EPLAN parts XML (`.part.xml`, parts export) | undocumented | XML | high | quick-xml | none | S | 3/1/0/2 | second wave (with EDZ) |
| EPLAN native project (`.elk`, `.edb`, `.zw1`) | closed | proprietary database [unverified] | high | none | none | blocked | - | skip |
| EPLAN labeling and report exports (Excel/CSV/XML) | user-defined schemes | xlsx/csv/txt/xml | very high | calamine, csv | none | S (on existing readers) | 2/2/0/3 | first wave (header heuristics) |
| Wiring lists and terminal plans (CSV/Excel) | no standard | xlsx/csv | very high | calamine, csv | none | S | 2/3/0/3 | first wave (same heuristics) |
| AutomationML from EPLAN (AR APC, EPL PD) | AML is IEC 62714, free | XML (CAEX) | EPLAN, Zuken, TIA Portal | none | inside EPDZ sample | (see automation) | 3/3/1/1 | per automation area |
| Zuken E3.series exports | native `.e3s` closed; exports KBL, AML, PDF, DXF | various | medium, strong in harness [weak] | none | none | see KBL/VEC | - | later (via KBL/AML/DXF) |
| WSCAD exports | native closed; Excel/CSV lists | xlsx/csv | medium in DE SME [weak] | calamine | none | covered by list heuristics | 2/2/0/3 | via list heuristics |
| IEC 61850 SCL | IEC paid; XSD available | XML | grid automation | iec61850-scl 0.2.0 | none | (see automation) | 3/3/1/1 | per automation area |
| IEC 62683 (switchgear product data) | dictionary in IEC CDD, not a file format | properties via CDD IRDIs | low as such | none | none | n/a | - | skip as format; read its IRDIs as codes |
| IEC CDD / IEC 61360 | copyright IEC, partial free attributes [unverified] | dictionary | reference only | none | none | n/a | - | skip bundling; print IRDIs |
| OCI, IDS-Connect, ELBRIDGE, UGL | protocols / order interfaces | HTTP form posts, not files | high in trade | none | none | n/a | - | skip |
| VDE-AR application rules | rules, not a file format | - | - | - | - | n/a | - | skip |
| RAMI 4.0 (DIN SPEC 91345) | reference model, not a file format | - | - | - | - | n/a | - | skip |
| DXF electrical blocks | covered in cad.md | - | - | - | - | - | - | already covered |

### BMEcat 2005 / 2005.2

- **Defining body:** BME (Bundesverband Materialwirtschaft, Einkauf und Logistik e.V.). Current
  version BMEcat 2005.2 (German and English PDF plus XSD), older 2005.1, 2005, 1.2, 1.01 all on
  https://www.bme.de/services/bmecat/downloads_bmecat/ . Date of 2005.2 [unverified].
- **Openness:** spec and XSD are free downloads, no login seen. The 2005 spec says: "The BME hereby
  grants you the durable, not exclusive, free of charge right to use the BMEcat specification"
  (https://a.storyblok.com/f/104752/x/bef34fe816/bmecat_2005_en.pdf). No patents known. The ECLASS
  repo mirrors the 2005.2 XSD and PDFs (https://github.com/eClassStandard/ECLASS-XML-Schema,
  no licence file). Guidelines on top: ETIM BMEcat guideline 5.0 (2021-06-30, revision 5.0.1
  2021-08-24, announced as the last ETIM BMEcat version), eCl@ss-BMEcat guideline (2005.1 package).
- **Container:** XML (namespace `http://www.bmecat.org/bmecat/2005`, variants `2005+onto` for
  ontology/IRDI use, plus DTD-based 1.2). Key elements: `HEADER/CATALOG`, `SUPPLIER`, `BUYER`,
  `AGREEMENT`; transaction `T_NEW_CATALOG`, `T_UPDATE_PRODUCTS`, `T_UPDATE_PRICES`;
  `CLASSIFICATION_SYSTEM/CLASSIFICATION_GROUPS` (tree via `CLASSIFICATION_GROUP_PARENT_ID`);
  `PRODUCT` with `PRODUCT_DETAILS`, `PRODUCT_FEATURES` (`REFERENCE_FEATURE_SYSTEM_NAME`,
  `REFERENCE_FEATURE_GROUP_ID`, `FEATURE/FNAME|FTEMPLATE/FT_ID, FVALUE, FUNIT, VALUE_IDREF`,
  `FID/FPARENT_ID` for ECLASS Advanced blocks), `PRODUCT_ORDER_DETAILS`, `PRODUCT_PRICE_DETAILS`,
  `MIME_INFO`, `PRODUCT_REFERENCE` (accessory, spare part, followup...), `USER_DEFINED_EXTENSIONS`
  (`UDX.*`).
- **Knowledge carried:** catalog with prices and price validity, product properties with units
  (FUNIT as UN/CEFACT codes, e.g. `VLT`), classification (ECLASS, ETIM, UNSPSC, supplier trees),
  relations between products (`PRODUCT_REFERENCE` types), document references (`MIME` datasheets,
  CAD), dates. In the Weidmueller sample (inspected, not kept in the corpus) the ECLASS Advanced
  features form a tree through
  `FID`/`FPARENT_ID` (52 features, IRDIs like `0173-1#02-AAF040#004`).
- **Commonness:** BME standard used across German B2B procurement (strong, but no member counts
  checked). ETIM International "before recommended the BMEcat standard" for classified product
  data (https://www.etim-international.com/classification/exchange-format/). GitHub code search:
  383 XML files with `BMECAT` (weak). Manufacturers publish BMEcat downloads (Stahlwille, ABB
  Striebel & John, Lapp). DE/EU relevance: very high.
- **Rust crates:** none on crates.io (search "bmecat" empty). OSS: olivere/bmecat (Go, MIT,
  4 stars, pushed 2026-09-17), stephanstapel/BMECat.net (C#, Apache-2.0, 19 stars, 2026-09-09),
  enthus-golang/bmecat-2005 (Go, Apache-2.0), HenrikPilz/BMEcatConverter (Python, BSD-3-Clause,
  8 stars), sveneisenschmidt/bmecat (PHP, MIT). Pagnany/bmecat (Rust, no licence, do not use).
- **Public samples:** olivere/bmecat golden files (MIT), BMECat.net Weidmueller files (Apache-2.0
  repo, third-party data), HenrikPilz test data (BSD-3, mostly nexmart/crossbase tool catalogs,
  not electrical), BME "XML examples" ZIP for 2005.1 (licence of examples [unverified]).
  Downloaded: `electrical/bmecat/olivere-new_catalog-2005.xml`. The Weidmueller ECLASS Advanced
  file was inspected but not kept: it is real vendor catalog data that the repo license does not
  cover (same rule as in construction); its pinned URL and SHA-256 are in the corpus manifest as a
  candidate. No ETIM-coded BMEcat sample with a clean licence was found.
- **Effort:** S-M. Streaming reader on quick-xml (no new deps). About 400-600 lines for header,
  classification trees, products, features (Basic and Advanced trees), references, prices, MIME,
  UDX passthrough. Large catalogs (Weidmueller files up to 2.3 MB, real ones hundreds of MB) need
  streaming and a product-count cap in the output summary.
- **Fit:** properties 3, graph 1 (product references, classification tree), timeline 1 (catalog
  version, generation date, price validity), tables 3 (products, prices, features).
- **Recommendation:** first wave. Most common structured product data format in DE trade, open
  spec, simple XML.

### ETIM xChange 2.0 (and ETIM codes)

- **Defining body:** ETIM International. ETIM xChange V2.0 (schema dated 2025-11-27, package
  updated 2026-04-30 with country guidelines V2.01). Note: the exchange-format page still says
  "Currently, ETIM xChange 1.1 is applicable" (page modified 2024-08-27), while the download is V2.0.
  ETIM model current release 10.0 (Dec 2024), 11.0 planned for 2026-12-01 [unverified, secondary].
- **Openness:** xChange: "Licensed under the Apache License, Version 2.0" (licence page) and the
  schema itself has `"$license": "https://www.apache.org/licenses/LICENSE-2.0"`. ETIM classification
  model: ODC-BY 1.0, English master open to all, de-DE translation "National membership".
  Package: https://www.etim-international.com/wp-content/uploads/2025/11/ETIM-xChange_V2.0-2026-04-30.zip
  (found through the WordPress media API; the download page hides the link).
- **Container:** JSON, JSON Schema draft 2020-12 (`$id https://www.etim-international.com/xChange/20`).
  Package: schema, sample file, code lists ZIP, guideline PDF, element overview XLSX, mapping
  to BMEcat XLSX.
- **Knowledge carried:** catalogue header (validity, buyer/supplier GLN), products with
  `ProductIdentification`, `ProductRelations` (`RelationType` e.g. MANDATORY, with quantity),
  `EtimClassification` (`EtimClassCode` EC011533, `EtimFeatures` EF/EV codes, units),
  `EtimModellingClassCode` with `EtimModellingPorts` (connection types, a port graph),
  `OtherClassifications` (ECLASS 13.0 class `27-14-22-01` in the sample), LCA/EPD values (GWP and
  20 more indicators), attachments with issue and expiry dates, trade items with prices and
  packaging, legislation (REACH date, hazard class).
- **Commonness:** ETIM xChange is ETIM International's own standard since 2024; national bodies may
  still prefer BMEcat or DICO/FAB-DIS. ETIM covers electrical, HVAC, plumbing, building (strong in
  NL, DE, AT, Nordics). 2ba (NL) runs xChange imports. GitHub code search `EtimClassCode`: 22 (weak).
- **Rust crates:** none. Other: DmitryMatv/BMEcat-2-ETIMxChange (Python, MIT, 0 stars) carries
  schema V1.1. serde_json is enough.
- **Public samples:** official sample in the package. Downloaded:
  `electrical/etim-xchange/ETIM-xChange_SampleFile_V2.0-2025-11-27.json` (756 lines; includes
  ETIM, ECLASS, modelling ports, relations, LCA).
- **Effort:** S. serde_json walk with known field names, about 300 lines. Optional: bundle the
  English ETIM model (ODC-BY) to resolve codes to names (adds data size; Jordi decides).
- **Fit:** properties 3, graph 2 (product relations, modelling ports), timeline 2 (validity,
  announcement, obsolescence, attachment expiry, EPD validity), tables 3.
- **Recommendation:** first wave. Apache-2.0, official sample, rich semantics, small effort.

### ECLASS (codes and IRDIs in files)

- **Defining body:** ECLASS e.V. (Cologne). Releases yearly-ish; ECLASS 16.0 (Nov 2025, per
  Wikipedia), 17.0 announced on eclass.eu [unverified date]. Export formats Basic and Advanced
  (XML per ISO 13584-32 OntoML; also CSV).
- **Openness:** dictionary licensed (single, concordance, membership, pay per IRDI). Free for
  "pure research and teaching" on request (§4.3.1). Reading codes in received files: allowed as
  above (§2.1 (4), §4.1.2). Schemas: https://github.com/eClassStandard/ECLASS-XML-Schema (OntoML
  ISO 13584-32, ISO 29002, eClassXML, UnitsML, BMEcat 2005-2 XSD; no licence file, 3 stars,
  pushed 2025-04-29).
- **Container:** not a file format of its own for Rezzer. Codes appear as: 8-digit class codes
  `27141137` or `27-14-11-37`; IRDIs `0173-1#01-ADO458#005` (class), `#02-` property, `#07-` value,
  `#05-` unit [unverified for #05]; `REFERENCE_FEATURE_SYSTEM_NAME` `ECLASS-9.0`; AML role class
  paths `[EclassRoleClassLib]/[EclassClassSpecification]/[27 ...]/[27-25-01-01 Feed-through terminal block]`.
- **Knowledge carried:** classification and property semantics with units; the names in the file
  (FT_NAME, eClassPath) are what Rezzer may show.
- **Commonness:** ECLASS is the de facto product classification in German industry; used in BMEcat,
  AAS submodels, EPLAN AML (429 feed-through terminal role refs in the EPLAN sample). GitHub code
  search `0173-1#02-`: 4,104 hits (moderate).
- **Rust crates:** none for ECLASS. dpp-vocab 0.22.0 (Apache-2.0, 1,643 downloads) is a DPP
  vocabulary register, not an ECLASS reader.
- **Public samples:** codes in the olivere BMEcat and the xChange sample (corpus), the Weidmueller
  BMEcat (not kept), and the AAS samples (automation).
- **Effort:** S as a shared code recognizer (regex for IRDI and class codes, version tag) used by
  BMEcat, AML, xChange, Excel readers.
- **Fit:** properties 3, graph 1, timeline 1 (IRDI version suffix), tables 1.
- **Recommendation:** first wave as a mapping layer. Print codes and in-file names only, write
  the §5.3 notice, never resolve from a dictionary.

### DATANORM 4 / 5

- **Defining body:** Datanorm working group; trade associations named as VSI and GDH in secondary
  sources [unverified]. DATANORM 4 (1994) and 5 (1999) [unverified, secondary].
- **Openness:** no free official spec found. A paid third-party developer guide exists (Digistore24,
  125 EUR). Lapp offers a Datanorm 4.0 ZIP [unverified content]. OSS parser halo/datanorm (Ruby,
  MIT) documents behavior. Format facts are simple enough to derive from public files.
- **Container:** text, records one per line, record type in field 1, semicolon-separated fields
  in both samples. The v4 sample has a fixed-width `V` header record (`V 120825Artikelstammdaten
  Deutschland, ...04EUR`) and semicolon `A` (article: number, short texts, unit `St.`, price
  `28000`, discount group, text key), `B` (extra data), `T` (long text lines keyed by text number)
  records. The v5 sample has a semicolon `V;050;A;20200406;EUR;...` header, `T` and `A` records
  and `C` (service) records. Also `P` (prices), `R`, `S`, `D` records exist per halo test names
  [unverified]. Encoding: v4 sample decodes as CP850 (DOS); v5 sample is ASCII with `?` where
  umlauts were (lossy, cannot test encoding).
- **Knowledge carried:** article master data with prices, units, discount groups, product groups,
  long texts, EAN, set/replacement links [unverified]. No technical properties with units.
- **Commonness:** standard in German trade for crafts (electrical, SHK, building). Manufacturers
  (Lapp, ABB Striebel & John, Wildeboer) and wholesalers publish it. Many crafts ERP systems import
  it (Streit, etc.). DE relevance: very high; outside DACH low.
- **Rust crates:** none ("datanorm" empty). Encoding: encoding_rs has no CP437/CP850; use oem_cp
  2.1.2 (MIT, 3.3 M downloads, 2025-12-24) or yore 2.1.1 (MIT, 2026-07-17). OSS: halo/datanorm
  (Ruby, MIT, 4 stars, 2026-08-11), phamos-eu/DATANORM_integration (Python, licence "other").
- **Public samples:** halo/datanorm test assets (MIT). Downloaded:
  `electrical/datanorm/halo-v4_with_texts.001`, `electrical/datanorm/halo-v5_texts_products_services.001`.
- **Effort:** S. Line reader with record dispatch, text joining, price decimals, CP850; about
  250-350 lines.
- **Fit:** properties 2, graph 1 (article to text, set links), timeline 1 (header date), tables 3.
- **Recommendation:** first wave. Very common in German electrical trade, tiny reader.

### KBL and VEC (wire harness, added when merging)

- **Defining body:** prostep ivip Association and VDA, joint project group VES-WF (Vehicle Electric
  Systems Workflow Forum); tool vendors test in the ECAD Implementor Forum. KBL
  (Kabelbaumliste) is the older single-harness format (KBL 2.3 SR-1, 2.4 SR-1, 2.5 SR-1 of
  2020-06-26); VEC (Vehicle Electric Container) is the successor for the whole vehicle network
  (1.1.3, 1.2.2, 2.0.0 of 2022-03-09, latest 2.2.0). Home: https://ecad-wiki.prostep.org/ .
  The VDA recommendation numbers usually given (VDA 4964 for KBL, VDA 4968 for VEC) were not
  confirmed from a primary source today [unverified].
- **Openness:** ECAD-WIKI: "The VEC, KBL and ECAD-WIKI are published under the MIT License"
  (https://ecad-wiki.prostep.org/post/how-to-contribute/). Repo prostep-ivip-e-V/vec (MIT,
  "Copyright (c) 2025 prostep ivip association", 10 stars, pushed 2026-10-16) holds the schemas,
  the model documentation, implementation guidelines and sample files. Since 2024 VEC is also
  published as an OWL 2 ontology with SHACL shapes. No registration, no patent terms found. This
  is the most open format in the electrical area.
- **Container:** XML. KBL: root `kbl:KBL_container` (namespace
  `http://www.prostep.org/Car_electric_container/KBL2.3/KBLSchema`, `version_id="2.3 SR-1"`),
  part master data (`Part` with `Part_number`, `Company_name`, `Version`, `Mass_information`,
  value and unit components), connector housings with `Cavities`, wires, occurrences, connections,
  harness, modules, topology nodes and segments with Cartesian points or B-splines. VEC: root
  `vec:VecContent` (namespace `http://www.prostep.org/ecad-if/2011/vec`, `VecVersion` 2.1.0 in the
  sample), `DocumentVersion` (CompanyName, DocumentNumber, DocumentType, DocumentVersion,
  ReferencedPart), `Specification` blocks, `Component`, `ContactPoint`, `WireEnd`,
  `WireMounting`, `CavityMounting`, `TopologyNode`, `TopologySegment`, `Routing`, `Path`.
- **Knowledge carried:** harness BOM (harness, harness configurations, modules, assemblies in KBL;
  generic BOM hierarchies in VEC), part master data with mass and units, connectors and cavities,
  wires with cross-section and colour, the connection list (wire end to cavity), harness topology
  as a graph (nodes, segments, routing of wires through segments), variant conditions (logistic
  control strings in KBL, configuration on any element in VEC), placements, document versions
  with numbers and dates. VEC adds E/E components, system schematics and couplings between
  harnesses (whitepaper "KBL vs. VEC", https://ecad-wiki.prostep.org/post/kbl-vs-vec/).
- **Commonness:** created for OEM-supplier harness exchange in German automotive; sample files
  come from Volkswagen (VOBES and VOBES NG); the whitepaper calls KBL still sufficient for the
  OEM-supplier interface and VEC the choice for whole-vehicle models (standards body, medium).
  Tool support list not checked [unverified]; Zuken E3.series exports KBL (see Zuken below).
- **Rust crates:** none (crates.io searches for "kbl" and harness terms find unrelated crates).
  Other OSS: 4Soft-de/harness-model (Java, MIT, 9 stars, pushed 2026-10-19: JAXB models for KBL
  2.4/2.5 and VEC 1.1 to 2.x, KBL to VEC converter, RDF converter, test files).
- **Public samples:** ECAD-WIKI sample section in prostep-ivip-e-V/vec (MIT repo of the standards
  body; Volkswagen "Old Beetle" files are marked "pure testing data", part numbers obfuscated) and
  harness-model test resources (MIT). Downloaded: `electrical/harness/vw-oldbeetle-kbl23-generator-cable.kbl`
  (KBL 2.3 SR-1, 35 Part elements, 14 cavity groups) and `electrical/harness/vec-routing-examples.vec`
  (VEC 2.1.0, topology nodes and segments, routings, wire ends). Larger candidates: the full
  vehicle VEC 1.1.3 (6.7 MB) and a KBL 2.4 main harness (1.8 MB).
- **Effort:** KBL S-M (about 400 lines on quick-xml with an id index, BOM and connection list
  into the shared BOM renderer and graph). VEC M (wide model, 2.x vs 1.x differences, about
  700 to 900 lines). Both reuse the 81346-style designation handling only where OEMs put such
  codes in names.
- **Fit:** properties 3, graph 3 (connection list, topology, routing), timeline 1 (document
  versions and dates), tables 3 (BOM, wire list, connector and cavity table).
- **Recommendation:** first wave (after DATANORM). Fully open under MIT with official samples,
  a real BOM plus wiring graph, and strong German automotive relevance. KBL first, VEC right after.

### ELDANORM

- **Defining body:** electrical wholesale (ZVEI/VEG context) [unverified]. No primary source found.
- **Openness:** no public spec found. ABB offers Striebel & John data in Datanorm and Eldanorm
  (https://new.abb.com/low-voltage/de/artikelstammdaten/direktvertrieb).
- **Container:** text, Datanorm-like [unverified].
- **Knowledge carried:** article master data (order number, EAN, short texts, list prices).
- **Commonness:** legacy, few current mentions (weak).
- **Rust crates / OSS:** none found.
- **Public samples:** none with licence.
- **Effort:** blocked until a spec or a licensed sample exists.
- **Fit:** 2/0/0/2.
- **Recommendation:** later. Detect and report "ELDANORM, not readable yet" if a magic can be found.

### IEC 81346 reference designations (and DIN EN IEC 81346)

- **Defining body:** IEC/ISO. IEC 81346-1:2022 ed2 (basic rules, adds type aspect and relation
  rules; https://webstore.iec.ch/publication/64021), IEC 81346-2:2019 (classes and letter codes),
  EN IEC 81346-2:2020 adopted in Europe (https://www.evs.ee/en/evs-en-iec-81346-2-2020). DIN EN
  IEC adoption date of part 1 [unverified]. VDI/VDE 3685 is unrelated (see headline 4).
- **Openness:** standards are paid. The syntax is widely documented and visible in files:
  prefixes `=` function, `+` location (`++` installation site), `-` product, `%` type (ed2),
  double prefixes (`==`, `++`) in EPLAN for higher-level function and installation site, `:`
  connection point, `&` document kind (IEC 61355), `#` page counter (EPLAN). Class letter codes
  (2019 tables) are in the paid part 2; Rezzer can show codes without names.
- **Container:** string grammar inside any file.
- **Knowledge carried:** plant tree (function, location, product aspects), connection points
  (`-X1:3`), document kind. Evidence from the EPLAN sample: `page_package` rows
  `Sample_Project-4_21|=GB1|+A1|&EFS1`, function names `+-U91`, AML link names
  `+A1-U4_Drill hole*1`, page titles `==EES==Page_macros++Infrastructure++Power_supply++AC400V&EFS#01/1`
  (from ecad-view docs).
- **Commonness:** every EPLAN, WSCAD, Zuken plan in Europe uses it; RDS-PP in power plants. Very
  high (practice evidence, no counts).
- **Rust crates:** none.
- **Public samples:** EPDZ sample (not copied, too big), plus strings in DXF title blocks.
- **Effort:** S, about 200 lines: tokenizer for aspects and levels, normalizer, tree builder,
  Mermaid output, used by every reader that sees designation-like cells.
- **Fit:** properties 2, graph 3 (plant tree, connection points to wiring graph), timeline 0,
  tables 2.
- **Recommendation:** first wave, as shared module. Highest value per line in this area.

### IEC 61355 document kind classification (DCC)

- **Defining body:** IEC, IEC 61355-1:2008 ed2 (https://webstore.iec.ch/publication/5379).
  Successor ISO/IEC 81355 in work [unverified, Wikipedia].
- **Openness:** standard paid; public code database IEC 61355 DB at std.iec.ch (free browse).
- **Container:** code `&` + A1 (technical area, optional) + A2 A3 (main class, sub-class), e.g.
  `&EFS` (circuit diagram, per EPLAN usage), with EPLAN suffix numbers `&EFS1`.
- **Knowledge carried:** document type of a page or file; groups pages into kinds (circuit
  diagram, terminal diagram, parts list).
- **Commonness:** high in plant documentation and in EPLAN page names.
- **Rust crates:** none.
- **Public samples:** EPDZ sample (`&EFS1`).
- **Effort:** S, part of the 81346 parser (about 50 extra lines plus a small code table of the
  public A2/A3 letters if licence allows [unverified]).
- **Fit:** 2/1/0/1.
- **Recommendation:** first wave with the 81346 parser.

### EPLAN `.epdz` (Smart Production publication)

- **Defining body:** EPLAN GmbH & Co. KG. Written by EPLAN Electric P8 2022+ via File > Publish >
  Smart Production. No official spec. Community notes: Canary-ECAD/ecad-view `docs/formats.md`
  (MIT, 5 stars, created 2026-07-13, pin b4edd7777b3ff54a33a674c34523a23b238b1189).
- **Openness:** undocumented, but plain containers. Reading facts from an MIT repo is fine.
- **Container (checked on the sample):** 7z (magic `7z BC AF 27 1C`), listed with macOS bsdtar:
  `manifest.db` (SQLite, `database.version` = `1.0.0`), `packages/pages/items/pagesvg/*.svg` (13),
  `packages/installationspaces/items/installationspacee3d/*.E3d` (7), `packages/automationML/Sample_Project.aml`
  (31.9 MB). Tables: `package` (tree; types: function 3186, mergedconnection 847, location 67,
  interruptionpoint 66, propertydefinition 35, page 10, container 7, installationspace 7, project 1),
  `page_package (packageid, name, ep1140, ep1240, ...)`, `function_package`, `location_package`,
  `mergedconnection_package`, `property (packageid, propname, propid, value)`, `item`,
  `page_functions`, `page_interruptionpoints`. Project properties by EPLAN property IDs (10000
  name, 10011 description, 10015/10016/10017 company, 2000 version).
  AML: `CAEXFile SchemaVersion="2.15"`, `AutomationMLVersion="3.0"`, WriterVendor "EPLAN Software
  and Service", WriterVersion 2025.0.3, recommendation `Document DocumentIdentifier="EPL PD"
  Version="3.0.0"`; 3,318 InternalElement, 306 InternalLink, 427 ExternalInterface, 170,294
  Attribute; role classes from `EPLANProPanelRoleClassLib` and `[EclassRoleClassLib]` (ECLASS
  class paths); attributes refer to EPLAN property IDs via `RefSemantic CorrespondingAttributePath`.
  SVG pages contain `javascript:jumpToFunction(...)` links and an `SVGScripting.js` (never run).
  E3D: binary 3D, versions 1-4 documented in ecad-view notes.
- **Knowledge carried:** project metadata, plant structure (functions, locations, 81346), pages
  with document kinds, cross references, merged connections (graph), panel layout (mounting
  surfaces, drill holes), ECLASS classes of devices, 3D. Report pages (BOM, terminal diagrams)
  only if generated before publishing (ecad-view wiki).
- **Commonness:** EPLAN is the leading ECAD in DE/EU [weak, vendor claim not checked]. EPDZ is
  new (2022+); Rittal/EPLAN Smart Production pushes it [weak].
- **Rust crates:** sevenz-rust2 0.24.1 (Apache-2.0, 2026-10-18, 1.75 M downloads); rusqlite 0.40.2
  (MIT, 2026-08-08, 129 M; `bundled` links SQLite C, public domain, in-process) or turso_core
  0.8.3 (MIT, pure Rust, young); quick-xml 0.42.0 for AML and SVG.
- **Public samples:** `samples/ejemplo.epdz` in ecad-view (MIT repo; content is EPLAN's sample
  project, project properties name EPLAN Software & Service; 6,508,358 bytes, SHA-256
  f6dde7258c52a1999ad080fd6b1399ace935db067c6242bcfe1837d4d2241886,
  https://raw.githubusercontent.com/Canary-ECAD/ecad-view/b4edd7777b3ff54a33a674c34523a23b238b1189/samples/ejemplo.epdz).
  Not copied: over the 5 MB cap and third-party content. Owner decision (section 8.5).
- **Effort:** M. 7z + SQLite + AML mapping + SVG text/titles; about 600-900 lines, AML part shared
  with the automation area.
- **Fit:** properties 3, graph 3, timeline 1, tables 2.
- **Recommendation:** second wave. Rich and readable, but needs two new dependencies (7z, SQLite)
  and the AML mapper first.

### EPLAN `.edz` (Data Portal exchange format) and parts XML

- **Defining body:** EPLAN. Import via Parts management > Extras > Import "Data Portal"; export to
  EDZ needs the licence module "EPLAN EDZ format" (EPLAN help,
  https://eplan.help/en-us/infoportal/content/Plattform/2.8/Content/htm/partsmanagementgui_d_artikelexport.htm).
  API: `IXPartsImportExportEdz` (https://eplan.help/en-us/infoportal/content/api/2.9/IXPartsImportExportEdz.html).
- **Openness:** undocumented. Data Portal content is manufacturer data under the EPLAN Cloud terms
  plus manufacturer terms; e.g. Mitsubishi forbids redistribution of CAD macro data. No sample can
  enter the corpus.
- **Container:** 7z (per the MIT Go importer 91fapiao-cn/EPLAN-EDZ-Import, which uses a pure-Go 7z
  reader and falls back to an external 7zr.exe when that fails; filext says some EDZ are ZIP
  [unverified]). Contents: `manifest.xml`, `*.part.xml` with `<part>` attributes `P_ARTICLE_*`,
  `<variant>` (`P_ARTICLE_VARIANT`), `<functiontemplate>`, `<freeproperty>`,
  `<attributeposition>`, `<accessoryposition>`; manufacturer XML; independent XML kinds
  accessorylist, construction, terminal; macros `.ema` (format [unverified]), images, PDFs.
  The same `<part>` XML is the EPLAN parts management XML export.
- **Knowledge carried:** article master data (order number, manufacturer, ECLASS/ETIM codes in
  `P_ARTICLE_*` [unverified]), function templates (connection points), accessories (relations),
  dimensions, documents.
- **Commonness:** very high among EPLAN users; Data Portal holds data from hundreds of
  manufacturers [unverified count].
- **Rust crates:** sevenz-rust2, quick-xml.
- **Public samples:** none with a licence.
- **Effort:** M (7z plus a lenient part XML mapper, about 400 lines); part XML alone S.
- **Fit:** 3/1/1/2.
- **Recommendation:** second wave, together with EPDZ. Open question: does sevenz-rust2 read all
  EDZ compression methods (LZMA2, BCJ, possibly PPMd)?

### EPLAN native project, labeling and report exports

- **Native project** (`.elk` link file, `.edb` project folder, `.zw1` compressed archive, `.pxf`
  project exchange): closed, no public spec [unverified internals]. Recommendation: skip, detect
  by extension and say "export as EPDZ, AML or Excel".
- **Labeling / reports:** EPLAN writes txt, xls, xlsx and xml from user-defined schemes; columns
  are free per scheme; header modes are none, property name only, or relation plus property name
  (e.g. "Cables / DT (full)") (https://www.eplan.help/en-us/infoportal/content/Plattform/2024/Content/htm/labelgui_k_start.htm,
  https://eplan.help/en-us/Infoportal/Content/Plattform/2.8/Content/htm/xpxfconverterexcel_d_pxfexceleinstellungen.htm).
  There is no column standard. Reader: existing calamine/CSV readers plus header synonyms (see
  wiring lists). Effort S. Fit 2/2/0/3. First wave as heuristics.

### Wiring lists and terminal plans (CSV / Excel conventions)

- **Defining body:** none. Produced by EPLAN, WSCAD, Zuken, and consumed by wire processing
  machines (Komax, Weidmueller, Steinhauer, Phoenix PROJECT interfaces in WSCAD release notes,
  https://wscad.com/en/?p=108232).
- **Openness:** n/a (tool-specific layouts). WSCAD imports "terminal diagrams, material lists and
  connection lists" from other ECAD tools as Excel (https://www.buhl.de/presse/?p=781).
- **Container:** xlsx/csv.
- **Knowledge carried:** wires as graph edges (source device and pin, target device and pin),
  cross-section, color, length, wire end treatment, cable and core, terminal strip and terminal
  number, bridges. Typical header words (DE/EN): BMK/DT/Betriebsmittelkennzeichen, Quelle/Ziel,
  Source/Target, Anschluss/Pin, Querschnitt/Cross-section, Farbe/Color, Laenge/Length,
  Klemmleiste/Terminal strip, Klemme/Terminal, Kabel/Cable, Ader/Core [header list from practice,
  unverified as a set].
- **Commonness:** very high in panel building (practice evidence).
- **Rust crates:** calamine 0.37.0 (MIT), csv 1.4.0 (Unlicense/MIT): already in Rezzer.
- **Public samples:** none with licence found; can be self-made later.
- **Effort:** S: header synonym detection plus 81346 parsing of cells, emitting a wiring graph
  (Mermaid) and terminal tables. About 250 lines on existing readers.
- **Fit:** properties 2, graph 3, timeline 0, tables 3.
- **Recommendation:** first wave, as a "table profile" on the XLSX/CSV readers.

### AutomationML in EPLAN and ECAD (short; see automation for AML)

- EPLAN exports PLC data as AutomationML AR APC (dialog "AutomationML AR APC V1.3.0";
  https://eplan.help/en-us/infoportal/content/Plattform/2025/Content/htm/plcgui_k_amlbusdatenaustausch.htm)
  for TIA Portal exchange, and the EPDZ contains an AML file with recommendation "EPL PD" 3.0.0
  (checked). Zuken E3.series added AML for PLC data. For Rezzer: the AML reader of the automation
  area plus an EPLAN profile (EPLAN property IDs from `RefSemantic`, ECLASS role paths, 81346 names).
  Recommendation: per automation area; add the EPLAN profile in the EPDZ work.

### Zuken E3.series

- **Defining body:** Zuken. Native `.e3s` project format closed, no spec [unverified internals].
- **Exports:** STEP AP212/KBL (KBL 2.3), STEP AP212/ELOG, Harness XML, PLMXML import, AML for PLC
  data, DXF/DWG, PDF, SVG, DGN, CGM, VRML, STEP AP203/214, E3.NCBridge XML
  (https://zuken.com/en/resource/e3-series-2017-delivers-on-productivity-and-third-party-integration,
  https://pcdandf.com/pcdesign/index.php/editorial/menu-new-indust-prod/17604-zuken-rolls-out-e3-series-2023).
  VEC support not confirmed [unverified].
- **Knowledge carried (via KBL/VEC):** harness topology, wires, connectors, cavities, lengths;
  KBL and VEC are the open harness XML formats (see the KBL and VEC section above).
- **Commonness:** medium, strong in harness and rail [weak].
- **Rust crates:** none. **Samples:** none.
- **Effort / Fit:** n/a for native; via KBL/AML/DXF/PDF readers.
- **Recommendation:** later; read its exports, detect `.e3s` and say so.

### WSCAD

- **Defining body:** WSCAD GmbH. Native project format closed [unverified internals].
- **Exports/imports:** Excel/CSV material lists (semicolon CSV for wscaduniverse BOM upload,
  manufacturer in column 1, part number in column 2; https://blog.wscad.com/en/?p=4284),
  terminal and connection lists, production interfaces (Komax, Steinhauer, Phoenix PROJECT).
  AML or BMEcat export not found [unverified].
- **Commonness:** medium in German SMEs [weak].
- **Rust crates / Samples:** none.
- **Effort / Fit:** covered by the wiring list heuristics.
- **Recommendation:** via list heuristics; no own reader.

### IEC 61850 SCL (short; see automation)

- IEC 61850-6 SCL (ICD, CID, SCD, SSD) is XML with substation topology, IEDs, logical nodes and
  communication. Crate iec61850-scl 0.2.0 (MIT OR Apache-2.0, 2026-09-05, 68 downloads, very
  young). Recommendation: per automation area.

### IEC 62683 (product data for low-voltage switchgear)

- IEC 62683-1:2026 ed2 replaces 2017 ed1 (dates from reseller listings: VDE April 2026, EVS EN
  2026-06-15 [unverified]); it reflects the IEC CDD dictionary 62683DB. It is a dictionary of
  classes and properties for catalogs, not a file format. Its IRDIs appear in AAS, BMEcat or
  xChange files. Recommendation: skip as a format; the code recognizer prints IEC CDD IRDIs
  (`0112/2///62683#...` form [unverified]) like ECLASS IRDIs.

### IEC CDD / IEC 61360 and RAMI 4.0

- IEC CDD (https://cdd.iec.ch): dictionaries per IEC 61360; content IEC copyright; access logged;
  licence page not reachable for us (403). Free reuse of identifiers and names per a search excerpt
  [unverified]. Recommendation: never bundle; print IRDIs and in-file names.
- RAMI 4.0 (DIN SPEC 91345): reference architecture, not a file format. Recommendation: skip.

### OCI, IDS-Connect, ELBRIDGE, UGL (trade interfaces)

- **OCI** (SAP Open Catalog Interface, 4.0/5.0): punch-out via HTML form post to `HOOK_URL` with
  `NEW_ITEM-*[n]` fields (DESCRIPTION, UNIT, PRICE, CURRENCY, VENDORMAT, MANUFACTMAT, ...); spec
  copy at https://www.mul.uzh.ch/dam/jcr:10879611-30ea-4ceb-8eed-608a61b08159/OCI_5.0.pdf .
- **IDS-Connect** (2.0 2013, 2.5 2020; ITEK, https://www.itek.de/fileadmin/user_upload/itek-website/Beratung/Standardisierung/IDS_Connect/2.5/20201225_IDS_Schnittstelle_2_5.pdf):
  craftsman software to wholesaler web shop (cart, order), HTTP based.
- **ELBRIDGE**: electrical trade interface between configurators and wholesaler shops (ABB
  ComfortPlaner example); body and spec [unverified].
- **UGL**: order exchange between crafts and wholesale (GC-Gruppe origin, UGL 5.0) [unverified].
- These are protocols, not files at rest. Recommendation: skip. If a saved cart file shows up,
  the XML/CSV readers handle it.

### VDE-AR

- VDE application rules (e.g. VDE-AR-N 4105/4110 grid connection) are rule texts, not file
  formats. Recommendation: skip.

### Area ranking

1. **IEC 81346 / IEC 61355 designation parser** (S): shared by every other reader here and in
   CAD/automation; turns strings into a plant tree and wiring endpoints.
2. **ETIM xChange 2.0** (S): Apache-2.0, official sample, relations, ports, timeline fields.
3. **BMEcat 2005 / 2005.2** (S-M): the most common structured catalog format in DE; ECLASS Basic
   and Advanced trees; open spec.
4. **ECLASS / ETIM code recognizer with licence rules** (S): needed by 2, 3 and AML; must print the
   ECLASS notice and never resolve from a dictionary.
5. **DATANORM 4/5** (S): very common in German electrical trade; needs CP850 (oem_cp).
6. **Wiring list / terminal plan table profile** (S): on calamine/CSV; gives graphs from EPLAN,
   WSCAD and machine exports.
7. **KBL, then VEC** (S-M and M, first wave, added when merging): MIT-licensed harness BOM and
   wiring graph with official samples.
8. **EPLAN EPDZ** (M, second wave): richest ECAD source, needs 7z + SQLite + AML.
9. **EPLAN EDZ / part XML** (M, second wave): no legal sample; build from the format facts.
10. Later: ELDANORM (no spec), Zuken/WSCAD native (closed; use exports).
11. Skip: OCI, IDS-Connect, ELBRIDGE, UGL, VDE-AR, RAMI, IEC 62683 and IEC CDD as formats.

### Open questions / unverified

- ECLASS Terms of Use 4.3 (2022) is the newest found; a newer version may exist with ECLASS 16/17.
  Confirm before release. Also confirm that showing in-file ECLASS names in Rezzer output counts
  as "converting into internal software" for the user (§4.1.2 (2)).
- Bundling the English ETIM model (ODC-BY) to resolve codes: Jordi decides (size, attribution).
- IEC CDD licence text (403); free-attribute clause from a search excerpt only.
- DATANORM owner and spec access (VSI/GDH per secondary sources); full record list beyond V, A, B,
  T, C seen in samples.
- ELDANORM spec and owner; UGL and ELBRIDGE owners and specs.
- Does sevenz-rust2 0.24.1 read every EDZ/EPDZ compression method? The Go importer needed an
  external 7zr fallback, which the owner ruling forbids.
- SQLite choice for EPDZ: rusqlite `bundled` (C, public domain, in-process) vs turso_core (pure
  Rust, young).
- `.ema` macro format (XML or binary), EDZ ZIP variant, EPLAN `P_ARTICLE_*` property list.
- Zuken VEC support, `.e3s` internals; WSCAD native format.
- Licence of the Weidmueller BMEcat sample: Apache-2.0 by repo, but third-party catalog data with
  ECLASS 9.0 content, so not kept in the corpus. The halo DATANORM texts look like real product
  texts behind an anonymized header; kept with a flag in the manifest.
- IEC 81346 prefix list for ed2 (`%` type aspect) and EPLAN double prefixes taken from practice
  and ecad-view notes, not from the standard text.

## 5. Mechanical


Research date 2026-10-19. Scope: PLM/PDM exchange, metrology, BOM exports, change management and
requirements. Already covered by the CAD spike (docs/spike-2026-10-19/cad.md): STEP Part 21
AP203/214/242 via stepq 0.6.0 (header, units, product structure, BOM with roll-up quantities,
placements, user properties, semantic PMI) plus the own Part 21 tokenizer. Everything below is a gap
relative to that, unless noted. Note: stepq and the own tokenizer read only Part 21 text; no reader
covers the XML encodings of STEP (AP242 BO/Domain Model XML, Part 28). cadmpeg-codec-step 0.6.2
(Apache-2.0) only *detects* a BO-Model document (root `Uos` plus namespace); its own docs say
"XML validation and schema selection remain outside this bounded marker check"
(https://github.com/cadmpeg/cadmpeg/blob/main/docs/formats/step.md).

### Summary

| Format | Openness | Structure | Commonness | Crates | Samples | Effort | Fit (P/G/T/Tb) | Recommendation |
|---|---|---|---|---|---|---|---|---|
| QIF 3.0 (ISO 23952) | ANSI QIF 3.0 free after a form; ISO edition paid; XSDs and samples BSL-1.0 on GitHub | XML, one XSD family | medium (metrology vendors, NIST, ISO); growing in automotive | none (`qif*` crates are Quicken) | 2 downloaded (BSL-1.0) | M | 3/2/2/3 | first wave |
| STEP AP242 XML (10303-3001 / -4442) | XSDs free at standards.iso.org; ISO text paid; MBx-IF rec. practices free | XML (`.stpx`, `.stpxZ` zip) | medium in German automotive PDM exchange (prostep ivip) | none that decode (cadmpeg detects only) | 1 downloaded (Apache-2.0) | M | 3/3/2/3 | first wave |
| ReqIF 1.2 (OMG) | spec and XSD free, OMG RF-Limited IPR | XML `.reqif`, zip `.reqifz` | high in German automotive (OEM-supplier) | reqrs 0.2.2 (Apache-2.0, weak); AGPL crates excluded | 2 downloaded (Apache-2.0 repo, origin flagged) | S to M | 3/3/2/2 | first wave |
| BOM CSV/XLSX exports (SolidWorks, Inventor, Creo, Windchill) | vendor output, no spec; user-configurable columns | CSV / XLS / XLSX | very high (every CAD shop) | calamine (reuse) | none (vendor files, no licensed public set) | S | 2/2/1/3 | first wave |
| IPC-2571 PDX | IPC standard paid; DTD embedded in every file | XML (`pdx.xml` in a zip `.pdx`) | low to medium, electronics supply chain (Arena, Agile) | none (`odx-rs` is ASAM PDX, different) | 1 downloaded (Apache-2.0, synthetic) | S | 3/2/2/3 | second wave |
| SAP BOM (IDoc BOMMAT, CSV) | IDoc docs need SAP login; XML form self-describing | IDoc XML, flat fixed-width, CSV | high in German industry (SAP ERP) | none | none | S (XML), M (flat) | 2/2/1/3 | second wave (IDoc XML), CSV via BOM mapper |
| PLM XML (Siemens) | schema "open, published" and docs public; license text not found | XML `.plmxml` / `.xml` | high in Teamcenter/NX shops | none | none licensed found | M | 3/3/2/3 | second wave |
| SolidWorks PDM XML export | no public schema; alias-set driven | XML | medium (SolidWorks PDM to ERP) | none | none | S | 2/2/1/3 | later, generic XML BOM mapper |
| JT (ISO 14306) | ISO paid; Siemens royalty-free patent grant for JT software; reference doc availability [unverified] | binary, segments, LZMA/zlib | high in automotive viz/DMU (VDA JT workflows) | none (0x-jerry/jt_reader no license) | none | L | 2/2/1/2 | later (structure and properties only) |
| 3D XML (Dassault) | docs royalty-free but internal use only | ZIP of XML (`Manifest.xml`, `.3DRep`) | medium (CATIA/3DEXPERIENCE viewers) | none | none | M | 2/2/1/2 | later, clean-room from public files only; docs license blocks use |
| OMG PLM Services 2.1 | spec free (OMG) | WSDL/XMI web service, PLM_container XML | low (PDTnet era) | none | none | M | 2/3/2/2 | skip (covered by AP242 XML) |
| prostep ivip recs: PDM schema, VDA 4953, VDA-FS | recs partly free/paid; VDA-FS is DIN 66301 (1986) | VDA 4953 = drawing guideline, not a format; VDA-FS = text geometry | VDA-FS legacy only | none | none | n/a / M | 0/0/0/0 | skip |
| VDA 4965 ECM (ECR/ECO) | VDA rec paid [unverified]; data via PLM Services / AP242 / EDIFACT | no own file format | medium in German automotive processes | none | none | n/a | (via carrier) | skip as format; map ECR/ECO fields when found in AP242 XML or PDX |
| OSLC (OASIS) | free OASIS standards | REST API, RDF (Turtle, RDF/XML) | medium in ALM tools (IBM ELM) | none | none | M | 2/3/1/0 | skip (API, not files) |

### QIF 3.0 (Quality Information Framework)

- **Defining body:** Digital Metrology Standards Consortium (DMSC). ANSI/DMSC QIF 3.0, December 2018;
  harvested as ISO 23952:2020 (edition 1, August 2020), which ISO lists as reviewed and confirmed in
  2026 (https://www.iso.org/standard/77461.html). A "next version" (3.1 or 4.0) is in planning, no
  release found (https://qifstandards.org/tag/next-version-qif).
- **Openness:** ANSI QIF 3.0 is free, but the download needs a form with name, organization, email and
  newsletter consent (https://qifstandards.org/download/). ISO 23952 is paid. The QIF 3.0 XSDs are also
  in https://github.com/QualityInformationFramework/qif-community under BSL-1.0 (LICENSE.md, checked),
  in `bindings/CPP-Kramer/schema/` (QIFDocument, QIFPlan, QIFResults, QIFProduct, QIFStatistics,
  QIFRules, QIFMeasurementResources, Characteristics, Features, Geometry, Topology, Units,
  Traceability, ...), `bindings/CSharp` and `bindings/Python`. Exception per LICENSE.md: the
  `bindings/CPP-CodeSynthesis` folder is under the CodeSynthesis license. No patent terms found.
- **Container:** XML, root `QIFDocument` in namespace `http://qifstandards.org/xsd/qif3`, attribute
  `versionQIF`. Sections (seen in the NIST sample): `Header`, `FileUnits`, `DatumDefinitions`,
  `DatumReferenceFrames`, `Product` (incl. geometry/topology), `Features`, `Characteristics`
  (Definitions, Nominals, Items), `MeasurementPlan`, `MeasurementsResults`, `Statistics`,
  `MeasurementResources`, `Rules`. Object ids are integer `id` with `idMax`; cross-file links by
  `QPId` UUIDs and `ExternalQIFReferences`.
- **Knowledge carried:** MBD/PMI as semantic characteristics (tolerances, datums, DRFs) linked to
  features and B-rep topology; measurement plans (ordered steps, work instructions); results per
  characteristic with measured value, status and `CharacteristicStatusEnum` (widget sample: 42
  statuses, 26 nominals); measurement devices; statistics (Cpk, GR&R, FAI); traceability
  (inspecting organization, purchase order, serial numbers, `TimeCreated`). Strong graph:
  characteristic -> nominal -> definition -> feature -> geometry.
- **Commonness:** DMSC community hub lists free validation tools, editors and bindings
  (https://qifstandards.org/community-hub/) - weak. Sample headers name exporters (SOLIDWORKS 2016 with
  CheckMate, Mitutoyo) - weak. NIST CTC/FTC test models published as QIF (repo `samples/NISTmodels`).
  ISO status helps EU relevance; German adoption evidence not found [unverified].
- **Rust crates:** none. crates.io `qif`, `qif_parser`, `qif_generator` are Quicken Interchange Format
  (name collision). Other OSS: QIF bindings (C++, C#, Python, BSL-1.0), QIF Rules Editor, QIF
  Resources Editor (repo list on https://qualityinformationframework.github.io/).
- **Public samples:** qif-community `samples/` (Plans, Results, Rules, QIFwidget, NISTmodels, Resources),
  BSL-1.0; NIST models are US government work. Downloaded: `qif3_widget_results.qif` (results with
  statuses and measured values) and `qif3_nist_ctc_01_mbd.qif` (MBD with PMI, 765 KB). Both pass
  `xmllint --noout`.
- **Effort:** M. zip not needed; quick-xml streaming plus an id index (integer ids, ref elements
  `*Id`). The model is wide (many characteristic types); a first slice of header, units,
  characteristics table (designator, type, nominal, tolerance, measured, status) and feature links is
  about 600 to 900 lines. Units reuse the model's property-with-unit type.
- **Fit:** properties 3, graph 2 (feature/characteristic/datum links), timeline 2 (plan vs. results,
  multiple result sets, `TimeCreated`, statistics over samples), tables 3 (inspection report).
- **Recommendation:** first wave - free, open schema, rich semantics, and it pairs with stepq's AP242 PMI
  (same NIST CTC models exist as STEP in the corpus).

### STEP AP242 XML (Business Object Model ISO/TS 10303-3001, Domain Model ISO/TS 10303-4442)

- **Defining body:** ISO TC 184/SC 4; implementation by MBx Interoperability Forum (prostep ivip and
  PDES Inc., formerly CAx-IF and PDM-IF). Schema table from https://www.mbx-if.org/home/mbx/resources/xsd-schemas/ :
  AP242 ed1 (2014) = 3001 ed1; ed1 TC = 3001 ed2 (2018); ed2 (2020) = 4442 ed2; ed3 (2022) = 4442 ed3;
  ed4 (2025) = 4442 ed5. MBx-IF: "The schema versions most common in industry use currently are AP242
  Ed.1 TC (3001 Ed.2) and AP242 Ed.3 (4442 Ed.3)." Current rec. practices: AP242 Domain Model XML
  Product & Assembly Structure v4.0 (2025-10-20), Kinematics v1.3
  (https://www.mbx-if.org/home/cax/recpractices/).
- **Openness:** XSDs free at standards.iso.org (e.g.
  https://standards.iso.org/iso/ts/10303/-4442/ed-3/tech/xml-schema/domain_model/DomainModel.xsd).
  ISO texts paid. MBx-IF recommended practices downloadable free (PDM-IF page lists Product & Assembly
  Structure v3.1 and Configuration Management v1.1, http://www.pdm-if.org/). No patent terms found.
- **Container:** XML, root `Uos` with `Header` and `DataContainer` (xsi:type AP242DataContainer).
  `.stpx` plain, `.stpxZ` zipped (CAx-IF naming convention). Key elements: `Part`, `PartVersion`,
  `PartView`, `ViewOccurrenceRelationship` with `Placement`, `Occurrence`, `Document`,
  `DigitalFile` / `ExternalGeometry` (references to Part 21 or JT files), `ExternalClassSystem`,
  `Classification`, `Approval`, `Organization`, `Person`. Ids: `uid`/`uidRef` local per file.
- **Knowledge carried:** PDM product structure (part, version, view, occurrence, placements), quantities,
  documents and file references (assembly split across files), classifications, approvals and dates,
  configuration management (effectivities), persons and organizations. It is the PDM half that stepq
  covers only through Part 21; the XML form carries more PDM metadata and no geometry.
- **Commonness:** prostep ivip / MBx-IF test rounds; German OEM PDM exchange (strong recommendation
  from prostep ivip) - medium, vendor/forum evidence. GitHub code search for the 3001 namespace finds
  a handful of repos (steptools, GfSE CASCaDE, cadmpeg, PMem) - weak.
- **Rust crates:** none that decode. cadmpeg-codec-step 0.6.2 (2026-10-19, Apache-2.0, 7,506
  downloads) detects BO-Model by root and namespace only. Other OSS: steptools/stp2bom (MIT, C++,
  STEP to BO XML), steptools/bom2stp (Apache-2.0).
- **Public samples:** steptools/bom2stp `as1-oc-214.xml` (Apache-2.0, CAx-IF AS1 assembly, 3001 ed1
  namespace) - downloaded as `ap242_bo_xml_as1.stpx`; it is the same AS1 model as the Part 21 file
  in the mcad corpus, so the BOM can be cross-checked against stepq. GfSE/CASCaDE `Bogie.stpx`
  (Apache-2.0) is a second candidate. No ed2/ed3 (4442) sample found; that is the most used schema.
- **Effort:** M. quick-xml, uid index, map Part/PartVersion/PartView/occurrence to the existing BOM and
  tree model used for stepq (reuse renderer and roll-up). 3001 and 4442 differ in namespace and some
  element names; one mapping table per edition. About 500 to 900 lines.
- **Fit:** properties 3, graph 3 (occurrence tree, document links, cross-file references), timeline 2
  (versions, approvals with dates, effectivities), tables 3 (BOM).
- **Recommendation:** first wave - fills the only gap left by stepq (XML encoding) with free XSDs and
  shared BOM rendering.

### ReqIF (Requirements Interchange Format)

- **Defining body:** OMG, ReqIF 1.2 formal/16-07-01, July 2016 (earlier 1.1 2013, 1.0.1 2011). Started
  by HIS (Audi, BMW, Daimler, VW, Porsche) and prostep ivip; contributors on the OMG page include Audi,
  BMW, Bosch, Continental, Daimler, VW, PROSTEP (https://www.omg.org/spec/ReqIF/).
- **Openness:** spec PDF and XSDs free (`reqif.xsd` dtc/11-04-05, `driver.xsd` for XHTML). OMG IPR mode
  "RF-Limited" (royalty-free, limited). XHTML content is a W3C subset.
- **Container:** XML, root `REQ-IF` namespace `http://www.omg.org/spec/ReqIF/20110401/reqif.xsd`
  (older files use `20101201`, as in the RMF sample). `.reqifz` = zip with one or more `.reqif` plus
  attachments (images, OLE objects referenced from XHTML `<object>`). Key elements: `THE-HEADER`,
  `DATATYPES` (string, integer, real, date, boolean, enumeration, XHTML), `SPEC-TYPES`,
  `SPEC-OBJECTS`, `SPEC-RELATIONS`, `SPECIFICATIONS` with nested `SPEC-HIERARCHY`, `RELATION-GROUPS`.
  Tool extensions in `TOOL-EXTENSIONS` (e.g. DOORS, Polarion).
- **Knowledge carried:** requirement objects with typed attributes, document hierarchy (chapters),
  typed relations (satisfies, refines, links across documents), `LAST-CHANGE` timestamps per object and
  definition, rich text as XHTML, attachments. Graph is first class.
- **Commonness:** standard exchange between OEM and supplier in German automotive; tool support from
  Jama (with DOORS, DOORS Next, Codebeamer, Polarion, Simulink,
  https://www.jamasoftware.com/media/2024/06/jama-connect-interchange-for-ReqIF-datasheet.pdf),
  MathWorks Requirements Toolbox, Capella, Enterprise Architect - vendor docs, medium. prostep ivip
  ReqIF Implementor Forum (older tested tool list on formalmind.com, dated) - medium.
- **Rust crates:** reqrs 0.2.2 (2026-07-27, Apache-2.0, 179 downloads, 1 star, port of strictdoc reqif,
  claims byte-identical round trip) - young, weak. reqif-rs 0.1.2 and doorstop-reqif 1.0.0 are
  AGPL-3.0-or-later: cannot be linked. Other OSS: strictdoc-project/reqif (Python, Apache-2.0, 59
  stars, pushed 2026-08), STARIONGROUP/reqifsharp (C#, Apache-2.0, 34 stars), pyreqif (BSD-2), Eclipse
  Capella requirements VP (EPL-2.0).
- **Public samples:** strictdoc-project/reqif test corpus (Apache-2.0 repo; includes files from Eclipse
  RMF, DOORS, Polarion, ReqIF Studio and the ci.eclipse.org implementor test cases TC1000 to TC1800,
  original licenses vary). Downloaded `reqif_rmf_spec_relations.reqif` (2 objects, 1 typed relation,
  nested hierarchy) and `reqifz_with_attachment.reqifz` (zip with nested folder, SVG attachment and
  macOS junk entries; a realistic container test).
- **Effort:** S to M. quick-xml with an identifier index; reuse the zip reader for `.reqifz`; XHTML to
  Markdown via the existing HTML/XHTML path if there is one, else a small converter. About 400 to 700
  lines. Own reader preferred over reqrs (young, 179 downloads), but reqrs is linkable if it holds up.
- **Fit:** properties 3, graph 3 (spec relations, cross-document links), timeline 2 (LAST-CHANGE per
  object), tables 2 (attribute tables per specification).
- **Recommendation:** first wave - free spec, very common in German automotive, and it is exactly the
  "relations between documents" knowledge Rezzer is about.

### BOM exports from CAD and PDM (SolidWorks, Inventor, Creo, Windchill)

- **Defining body:** none; vendor output (Dassault SolidWorks, Autodesk Inventor, PTC Creo/Windchill).
- **Openness:** no spec; the file format is plain CSV/XLS/XLSX, readable. Column names are user
  templates.
- **Container:** CSV, XLS (Inventor iLogic export supports `.xls` only, not `.xlsx`, per
  https://help.autodesk.com/cloudhelp/2022/ENU/Inventor-iLogic/files/GUID-5BEAA369-CFC0-482B-B01E-76914AAA750C.htm),
  XLSX, SolidWorks BOM table save-as (CSV/XLS) [unverified columns: ITEM NO., PART NUMBER,
  DESCRIPTION, QTY.]. Inventor API sample exports "Structured" and "Parts Only" views with columns
  Item, QTY, REV, Description, Part Number
  (https://help.autodesk.com/cloudhelp/2025/ENU/Inventor-API/files/BOMView_Export_Sample.htm).
  Windchill BOM reports (single or multi level) export from the report page; column names include
  Quantity, "Quantity with Units", Reference Designator (https://www.ptc.com/en/support/article/CS268059).
  Creo `.bom.N` text format: no doc found [unverified].
- **Knowledge carried:** BOM with quantities and units, item numbers, hierarchy via indented item numbers
  (1, 1.1, 1.1.2) or a level column, revisions, descriptions, materials, mass. Structured vs. Parts Only
  changes the meaning of QTY: a forum report says Structured shows per-parent quantity, not rolled up
  (https://forums.autodesk.com/t5/inventor-programming-forum/bom-export/m-p/9823074) - weak.
- **Commonness:** every mechanical CAD shop produces these; strongest real-world source of BOMs - vendor
  docs, strong by ubiquity but no counts.
- **Rust crates:** calamine 0.37.0 (2026-10-18, MIT, 15.6 M downloads) and the existing CSV reader -
  reuse.
- **Public samples:** none with a clear license found; synthetic CC0 files can be generated in the
  layout of the vendor docs.
- **Effort:** S. A column-role mapper on top of the existing table readers: detect item/level, quantity,
  unit, part number, revision, description columns (English and German headers: Pos., Menge,
  Benennung, Sachnummer, Teilenummer), derive the tree from item numbers or level, then reuse the stepq
  BOM renderer and roll-up. About 200 to 300 lines. It must say in the output when the tree was inferred
  from item numbers.
- **Fit:** properties 2, graph 2 (tree), timeline 1 (revision column), tables 3.
- **Recommendation:** first wave - cheap, reuses existing readers, very common.

#### SolidWorks PDM XML export

- PDM Professional export rules write BOM variables to XML for ERP, with names from alias sets, triggered
  by workflow transitions (https://help.solidworks.com/2015/English/EnterprisePDM/Admin/t_Creating_an_Export_Rule.htm).
  No public XSD found; element layout [unverified]. SolidWorks Manage lets users define tag names freely
  (GoEngineer blog). Effort S as a generic "XML BOM" mapper once a real sample exists. Recommendation:
  later.

### IPC-2571 PDX (Product Data eXchange)

- **Defining body:** IPC, 2570 series (2570 framework, 2571 generic requirements), Revision 0, November
  2001, ANSI approved; task group 2-15a (https://shop.electronics.org/ipc-2571/ipc-2571-standard-only).
- **Openness:** standard text sold by IPC. The DTD ("-//IPC//DTD 2571 200111//EN") is embedded in each
  file's DOCTYPE, so a reader can be written from files plus vendor notes
  (https://buyplm.com/pdxplorer/ipc-2571-pdx-technical). IPC copyright on the DTD text [unverified].
- **Container:** `.pdx` = zip with `pdx.xml` plus attached files; XML root `ProductDataeXchangePackage`.
  Elements: `Items/Item` (itemIdentifier, revisionIdentifier, globalLifeCyclePhaseCode,
  category, revisionReleasedDate), `BillOfMaterial/BillOfMaterialItem` (quantity,
  `ReferenceDesignators`), `ApprovedManufacturerList/ApprovedManufacturerListItem` (manufacturedBy,
  manufacturerPartIdentifier, preferred status), `Attachments`, `Changes` (ECR, ECO, deviations with
  `AffectedItems`), `History`, `AdditionalAttributes` (name, value, dimension, dataType).
- **Knowledge carried:** BOM with reference designators, AML (approved manufacturers and their part
  numbers), lifecycle phase, revisions with release dates, change orders, attachments, typed attributes
  with dimension (unit).
- **Commonness:** electronics supply chain (Arena PLM, Oracle Agile export) - vendor claims, weak. pypdx
  author: "there does not seem to be any sample PDX files that you can download" - weak, supports low
  public visibility. Germany/EU relevance low.
- **Rust crates:** none (`odx-rs` 0.39.0 reads ASAM ODX `.pdx`, a different format; `pdx` crate is a
  reserved name). Other OSS: sid5432/pypdx (Python, Apache-2.0, 4 stars).
- **Public samples:** pypdx `pypdx/data/pdx-example.xml` (Apache-2.0 repo, synthetic, "obviously not
  for a real product") - downloaded as `pdx_ipc2571_example.xml` (100 items, 108 BOM items, 120 AML
  items, 94 reference designators, 3,916 additional attributes). No zipped `.pdx` sample found.
- **Effort:** S. quick-xml (DTD must not be fetched or expanded; internal DTD is ignored), zip reuse.
  About 250 to 350 lines; BOM renderer reused.
- **Fit:** properties 3, graph 2 (BOM tree, item to manufacturer part), timeline 2 (changes, release
  dates, history), tables 3 (BOM, AML).
- **Recommendation:** second wave - small and rich, but rare in the German mechanical audience.

### SAP BOM (IDoc BOMMAT, CSV)

- **Defining body:** SAP SE. IDoc basic type BOMMAT0x (message BOMMAT), ALE distribution of material
  BOMs.
- **Openness:** IDoc type documentation (WE60) is in SAP systems and help.sap.com; segment field lists
  need an SAP login for the authoritative version. IDoc XML is self-describing (segment names as
  elements). No license restriction on reading files.
- **Container:** IDoc XML (`<BOMMAT05><IDOC><EDI_DC40>...`), flat fixed-width IDoc (control record plus
  data records, segment layouts needed), or CSV/XLSX from SE16/CS11/CS12 reports. Segments per community
  posts: `E1STZUM` (BOM header), `E1MASTM` (material allocation, MATNR), `E1STPOM` (items, IDNRK
  component, MENGE quantity), `E1STASM` (https://community.sap.com/t5/technology-q-a/idoc-segement/qaq-p/7705153,
  https://userapps.support.sap.com/sap/support/knowledge/en/3666446) - weak, [unverified] for BOMMAT05.
- **Knowledge carried:** single-level BOM per material with quantities, units, item category, valid-from
  dates and change numbers (Änderungsnummer), plant and usage.
- **Commonness:** SAP ERP dominates German industry - strong by market, but IDoc files rarely reach
  document stores; CSV reports from CS11/CS12 are more likely - weak.
- **Rust crates:** none.
- **Public samples:** none licensed found.
- **Effort:** S for IDoc XML (quick-xml, field mapping table); M for flat IDoc (needs segment layouts).
  CSV goes through the BOM column mapper above.
- **Fit:** properties 2, graph 2 (multi-level via linked BOMs), timeline 1 (change number, valid-from),
  tables 3.
- **Recommendation:** second wave for IDoc XML; flat IDoc later, only with a real file.

### PLM XML (Siemens)

- **Defining body:** Siemens Digital Industries Software; schema namespace
  `http://www.plmxml.org/Schemas/PLMXMLSchema`, documented versions up to v7.0.3
  (https://media.plm.automation.siemens.com/open/plm-xml/docs/v7.0.3/documentation/plmxml.htm).
- **Openness:** Siemens: "PLM XML Schema is open, published and compliant with the W3C XML schema
  recommendation" (https://www.siemens.com/en-us/products/plm-components/plm-xml/). Schema docs are
  public. No license text for the XSD found; the SDK is a separate product [unverified terms].
- **Container:** XML, root `PLMXML`. Elements: `ProductRevision`, `ProductView`, `Occurrence`
  (instancedRef, transform, `occurrenceRefs`), `ProductInstance`, `Representation` (links to JT/XT
  files), `UserData/UserValue` (title, value, type), `Form`, `AssociatedAttachment`, `DataSet`,
  `ExternalFile`, `Site`, `RevisionRule`.
- **Knowledge carried:** Teamcenter product structure (BOM views, occurrences, transforms), item
  revisions, forms with properties, datasets and file references (JT, PDF, native CAD), release status.
- **Commonness:** standard export from Teamcenter and NX; tool list: Teamcenter, NX, Tecnomatix,
  third-party viewers (PLMXMLViewer, MIT, 5 stars). GitHub repos: JAGSSoftware/plmxmldoc (MIT),
  rschrem/plmxml (no license), plmxml2xlsx tools - weak. Siemens strong in German automotive and
  machinery.
- **Rust crates:** none.
- **Public samples:** no licensed sample found (hlrs-vis/covise and GitNet0815/PLMXML-Deserializer have
  no license file; Siemens community thread asks for samples without answer). Not downloaded.
- **Effort:** M. quick-xml plus id/href resolution (`#id` and external `.plmxml` files), map to the BOM
  tree model; 500 to 800 lines. Needs a real Teamcenter export to test.
- **Fit:** properties 3, graph 3, timeline 2 (revisions, release status), tables 3.
- **Recommendation:** second wave - valuable but blocked on a licensed sample and unclear schema terms.

### JT (ISO 14306)

- **Defining body:** ISO TC 184/SC 4, ISO 14306:2017 (edition 2, adds STEP B-rep segment), paid
  (https://www.iso.org/standard/62770.html). Siemens JT File Format Reference 10.5 (2019) cited by the
  Library of Congress (https://loc.gov/preservation/digital/formats/fdd/fdd000562.shtml); 10.6 (2024)
  per Wikipedia [unverified].
- **Openness:** ISO text paid. Siemens JT Data Format Reference Patent Clarification Notice grants a
  royalty-free, non-exclusive license under two US patents for software that produces, consumes or
  interprets JT files
  (https://www.plm.automation.siemens.com/media/global/it/Siemens-PLM-JT-Data-Format-Reference-Patent-Clarification-Notice-mi-X1_tcm56-58013.pdf).
  Free download of the 10.x reference [unverified]. JT Open Program (150+ members) gives the toolkit
  under membership (https://www.siemens.com/en-us/products/plm-components/jt/jt-open-program/).
- **Container:** binary, file header plus TOC, segments (LSG scene graph, property tables, shapes,
  meta data, PMI, XT or STEP B-rep), compressed with zlib or LZMA (10.x).
- **Knowledge carried:** assembly tree (LSG), per-node properties (name/value meta data), PMI, LODs,
  materials. For Rezzer: tree and properties; geometry not needed.
- **Commonness:** automotive visualization and DMU (VDA JT workflows, Siemens JT2Go) - vendor claims,
  medium. Strong German automotive relevance (VDA 4966 JT supplier exchange [unverified]).
- **Rust crates:** none on crates.io. Other OSS: Open CASCADE JT Assistant (GPL-2.0, license only, not
  read), 0x-jerry/jt_reader (Rust, no license), ian-wigley/JT-Reader (C#, MIT, 1 star).
- **Public samples:** not searched beyond GitHub; no licensed one found.
- **Effort:** L. Binary, LZMA, element type GUIDs, version differences 8/9/10. A first slice reading TOC,
  LSG tree and property tables is still about 1,000+ lines and needs a spec copy.
- **Fit:** properties 2, graph 2 (tree), timeline 1, tables 2 (derived BOM).
- **Recommendation:** later - high value in automotive, but spec access and binary effort; first release
  identifies JT and asks for STEP AP242 or PLM XML.

### 3D XML (Dassault Systèmes)

- **Defining body:** Dassault Systèmes, proprietary. Versions 3.x/4.x [unverified current].
- **Openness:** documentation offered with a yearly royalty-free license for internal business use
  only, per Wikipedia (https://en.wikipedia.org/wiki/3DXML) [license text not read]. Internal-use terms
  conflict with an MPL-2.0 open-source reader built from those docs: blocked for the docs route.
- **Container:** ZIP; `Manifest.xml` names the root; product structure `.3dxml` XML
  (`Reference3D`, `Instance3D`, `ReferenceRep`, `InstanceRep`), representations `.3DRep` (XML or
  binary, tessellated) (https://xeokit.github.io/sdk/docs/api/documents/formats.threedxml.3DXML_Format_Guide.html).
- **Knowledge carried:** assembly tree with instances and transforms, names, some attributes; geometry
  as meshes.
- **Commonness:** CATIA / 3DEXPERIENCE / SolidWorks viewers - vendor, medium. Wikipedia: only supported
  by the Dassault product line.
- **Rust crates:** none. Other OSS: Terrev/3DXML-to-OBJ (C#, MIT, 35 stars), xeokit-convert (AGPL,
  license only, not read), ArasLabs viewer (MIT).
- **Public samples:** none licensed found.
- **Effort:** M for tree only on zip + quick-xml, if done clean-room from public files.
- **Fit:** properties 2, graph 2, timeline 1, tables 2.
- **Recommendation:** later - only clean-room from observed files; ask Jordi before using any Dassault
  documentation.

### OMG PLM Services 2.1

- **Defining body:** OMG, PLM Services 2.1 formal/11-05-03, May 2011 (2.0 2009); information model from
  ISO 10303-214, functional model from PDM Enablers 1.3 (https://www.omg.org/spec/PLM).
- **Openness:** free spec, WSDL and XMI.
- **Container:** web service (SOAP/WSDL); `PLM_container` XML documents (PDTnet heritage).
- **Knowledge carried:** same PDM concepts as AP214 (items, versions, BOM, documents, ECM messages).
- **Commonness:** low today; replaced by AP242 XML in prostep ivip work - weak.
- **Rust crates / samples:** none.
- **Effort:** M if ever needed; mapping overlaps AP242 XML.
- **Fit:** properties 2, graph 3, timeline 2, tables 2.
- **Recommendation:** skip - API rather than files; AP242 XML covers the content.

### prostep ivip formats: PDM schema recs, VDA 4953, VDA-FS

- **Defining body:** prostep ivip and VDA. PDM exchange recommendations now via MBx-IF AP242 XML (see
  above). VDA 4953 is a guideline for simplified/3D-based drawings, not a file format. VDA-FS: VDA CAD/CAM
  working group, DIN 66301 (July 1986), surface geometry only
  (https://en.wikipedia.org/wiki/VDA-FS, KIT publication https://publikationen.bibliothek.kit.edu/270031180/7707471).
- **Openness:** prostep recommendations partly free for members/paid [unverified]; DIN 66301 paid.
- **Container:** VDA-FS is fixed-format text (80-column cards); VDA 4953 n/a.
- **Knowledge carried:** VDA-FS: surfaces, curves, points, names only; no structure or metadata.
- **Commonness:** VDA-FS legacy, superseded by STEP; still importable in HiCAD, AutoCAD - weak.
- **Rust crates / samples:** none.
- **Effort:** VDA-FS M (text, but geometry only); VDA 4953 n/a.
- **Fit:** 0/0/0/0 for knowledge (geometry only).
- **Recommendation:** skip - VDA-FS: identify by header and say it carries geometry only.

### VDA 4965 ECM (Engineering Change Management: ECR, ECO)

- **Defining body:** VDA with prostep ivip project group ECM; VDA 4965-1 (ECR), prostep ivip PSI 3
  (ECO recommendation) (https://www.prostep.org/en/medialibrary/detail?ai[action]=detail&ai[controller]=Catalog&ai[d_name]=ecr_ig).
- **Openness:** VDA recommendation sold by VDA [unverified]; prostep implementation guides partly free.
- **Container:** no own file format; messages carried via PLM Services, STEP/AP242 XML or XML defined in
  the implementation guide (odette presentation: "using existing standards like PLM Services, STEP and
  XML") - EDIFACT per a secondary glossary [unverified].
- **Knowledge carried:** change request and order ids, status, affected parts, dates, participants.
- **Commonness:** OEM-supplier change processes in German automotive - medium, but files rarely leave
  the PLM integration layer.
- **Rust crates / samples:** none.
- **Effort:** n/a as a reader; map ECR/ECO fields when they appear inside AP242 XML or PDX `Changes`.
- **Fit:** via carrier (timeline 3 when present).
- **Recommendation:** skip as a format; note in the AP242 XML mapping.

### OSLC (Open Services for Lifecycle Collaboration)

- **Defining body:** OASIS OSLC Open Project. OSLC Core 3.0 and Query 3.0 OASIS Standards (August 2021);
  Requirements Management 2.1 OASIS Standard (June 2021); Change Management 3.0
  (https://www.oasis-open.org/?p=7646).
- **Openness:** free specs, OASIS IPR.
- **Container:** HTTP REST with RDF (Turtle, RDF/XML, JSON-LD); resource shapes. Not a file format.
- **Knowledge carried:** links between requirements, change requests, tests, models (typed relations).
- **Commonness:** IBM ELM, some ALM tools - medium; OSLC OP repos (oslc4net EPL-1.0, pyoslc BSD-3).
- **Rust crates:** none.
- **Public samples:** n/a.
- **Effort:** M (RDF parsing; an RDF crate would be needed).
- **Fit:** properties 2, graph 3, timeline 1, tables 0.
- **Recommendation:** skip - Rezzer reads files; an exported Turtle file could later go through a generic
  RDF reader if one exists in another area.

### Area ranking

1. **ReqIF** - free spec, common in German automotive, small reader, graph-first content (relations
   across documents). Two samples ready.
2. **BOM CSV/XLSX exports** - cheapest win, reuses calamine and the stepq BOM renderer, the most common
   BOM source in practice. Needs synthetic CC0 samples.
3. **STEP AP242 XML** - the only STEP gap left after stepq; free XSDs; AS1 sample cross-checks stepq.
   Find a 4442 ed3 sample next.
4. **QIF 3.0** - free schema under BSL-1.0, rich PMI and inspection results, NIST pairs with existing
   STEP corpus; wider model makes it M.
5. **IPC-2571 PDX** - small, rich BOM/AML/changes, but rare in the target audience.
6. **SAP IDoc XML** - common source, small reader, but no sample and no public segment doc.
7. **PLM XML** - valuable for Teamcenter users; wait for a licensed sample and clear schema terms.
8. **JT** then **3D XML** - later; binary/spec access (JT) and docs license (3D XML) block a clean path.
9. Skip: OMG PLM Services, OSLC, VDA 4965 (no own format), VDA 4953 (not a format), VDA-FS (geometry
   only), SolidWorks PDM XML until a real sample exists.

### Open questions / unverified

- QIF XSD license: the Kramer, C# and Python XSD copies fall under the repo's BSL-1.0; only the
  CodeSynthesis bindings folder is excepted. The XSDs in the DMSC form download may carry other terms
  [unverified].
- Provenance of samples: `reqif_rmf_spec_relations.reqif` and the reqif inside the reqifz come from
  Eclipse RMF test data (originally EPL likely) in an Apache-2.0 repo; `ap242_bo_xml_as1.stpx` is the
  CAx-IF AS1 model published by STEP Tools under Apache-2.0; the PDX file embeds the IPC DTD text.
  Flag for the corpus license check.
- No AP242 Domain Model XML (4442 ed3) sample found, although MBx-IF names it as one of the two most
  used schemas.
- PLM XML schema license text not found; no licensed PLM XML sample.
- JT reference 10.5/10.6 free availability and VDA JT workflow recommendation number [unverified].
- 3D XML license text not read directly (Wikipedia summary only).
- SAP BOMMAT05 segment layout and SolidWorks/Creo default BOM column names [unverified].
- VDA 4965 carrier formats (EDIFACT vs. XML) [unverified]; prostep recommendation pricing [unverified].

## 6. Electronics


Research date 2026-10-19. Versions, prices and counts were checked today; re-check before relying on
them. Already covered by Rezzer readers from the CAD spike (docs/spike-2026-10-19/cad.md lines 46-47,
108, 461): KiCad 5 to 10 (own s-expression reader with netlist pass), Gerber X2/X3 incl. X3 component
attributes (from source, no real X3 sample yet), Excellon, Gerber job file. Nothing here repeats them.

GitHub counts below are weak evidence: code search only indexes text files under 384 KB, `total_count`
is approximate, and topic tags undercount. They show order of magnitude only.

### Summary

| Format | Openness | Structure | Commonness | Crates | Samples | Effort | Fit (P/G/T/Tb) | Recommendation |
|---|---|---|---|---|---|---|---|---|
| IPC-2581 (DPMX) rev C | spec paid, XSD public in practice (license [unverified]) | XML | about 140 consortium members; Cadence, Siemens, Zuken, Altium, KiCad export | `ipc2581` 0.3.3 (MIT/Apache, young) | downloaded (KiCad 10, MIT) | M | 3/3/2/3 | first wave |
| ODB++ (Design) v8.1 | spec free after sign-up, but notice says confidential, UCA terms | dir / tgz / zip, text records, `.Z` LZW | "more than 70 software partners" (Siemens) | none | downloaded (PADS 2014, CERN-OHL-W) | M to L | 3/3/1/3 | second wave, owner decision on spec terms |
| IPC-D-356 / 356A | spec paid ($155), format widely documented | fixed-column text | standard fab-package companion; 374 `.d356` files on GitHub (weak) | none | not kept (pointer only) | S | 1/3/0/2 | first wave |
| Eagle XML .sch/.brd/.lbr | DTD shipped with Eagle, no spec license | XML | 25,792 `.sch` / 45,568 `.brd` hits with "eagle version" (weak); EOL 2026-06-07 | none | downloaded (MIT) | M | 3/3/1/3 | first wave |
| Altium SchDoc/PcbDoc/PrjPcb | closed, reverse-engineered docs (WTFPL, Apache, MIT) | OLE CFB + pipe-delimited `KEY=VALUE` records; PrjPcb INI | market leader in pro PCB (vendor claims); 476+430 topic repos | `altium` 0.2.1 (Apache, stale); `altium-format` GPL (no) | SchDoc + Altium centroid downloaded (MIT) | SchDoc M, PcbDoc L | 3/3/1/3 | SchDoc first wave, PcbDoc second |
| Pick-and-place / centroid | conventions only, no standard | CSV / text | every assembly order | none needed | Altium file downloaded (MIT) | S | 2/0/0/3 | first wave |
| BOM CSV conventions | conventions only | CSV / XLSX | every assembly order | none needed (calamine/CSV) | (BOM inside IPC-2581 sample) | S | 2/1/0/3 | first wave |
| ProSTEP iViP IDX (EDMD) | PSI 5 v4.5 docs free download; XSD "all rights reserved" | XML | Cadence, Siemens, PTC, Dassault, Zuken implement; German body | none | downloaded (Expedition 2023, CERN-OHL-W) | M | 3/2/3/2 | second wave |
| IPC-7351 names / footprint libs | IPC-7351B paid (2010); C never published | naming convention | ubiquitous in footprint names | `vcad-ecad-package` (generator, not reader) | in Altium and IPC-2581 samples | S | 2/0/0/1 | later (helper) |
| Gerber X3 component layer | Ucamco spec free | text | KiCad, Altium export [unverified for Altium] | (covered) | none real (gap) | covered | 2/0/0/3 | covered, add a sample |
| EDIF 2 0 0 netlist | EIA-548-1988 / IEC 61690 paid | s-expression text | 7,312 `.edf` hits with `edifVersion` (weak, mostly FPGA) | none | no | S to M | 2/3/0/1 | later |
| OrCAD Capture / Allegro | closed binary | CFB (DSN) / proprietary (brd) | very common in industry (no hard number) | none | no | DSN L, brd blocked | 3/3/1/3 via exports | later (DSN); Allegro via IPC-2581/ODB++ |
| SPICE netlists (+ LTspice .asc) | no defining body; tool manuals free | text | 51,456 `.asc` with "SHEET" (weak) | `thevenin-types`, `netlist-db`, `ngspice-rs` | no | M (dialects) | 2/3/0/1 | second wave |
| IBIS | IBIS Open Forum, v8.0 (2025-12-05), spec PDF public | keyword text | standard for SI models; 391 `.ibs` hits (weak) | none | no | M | 2/1/1/2 | later |
| Zuken CR-8000 | closed native | n/a | strong in DE/JP (weak) | none | no | blocked native | via exports | skip native |
| HyperLynx .hyp | no public spec | text | 521 `.hyp` with BOARD (weak) | none (`hyp2mat` GPL) | no | M, blocked-ish | 2/3/0/1 | later |
| Fritzing .fzz | app GPL-3.0, parts CC-BY-SA-3.0, no spec | ZIP + XML | education/maker; 225 topic repos | none | no | S | 2/3/0/2 | later |

### IPC-2581 (IPC-DPMX)

- **Defining body:** IPC, 2-16 DPMX subcommittee; promoted by the IPC-2581 Consortium. Current
  published revision: IPC-2581C, published 2020-11-01 (reseller listing,
  https://www.technickenormy.cz/en/ipc-2581-revision-c/). The standard is being rebranded IPC-DPMX
  (Digital Product Model Exchange) (https://www.electronics.org/committee-detail/2-16). Revision D:
  not found as of 2026-10-19 [unverified]; a podcast mentions a new revision for November 2024 without
  a letter (https://upmg.podbean.com/e/pcb-chat-episode-66-the-ipc-2581-consortium-on-electronics-data-transfer).
- **Openness:** spec is paid. IPC shop lists the 2581B kit (hard copy plus schema download) at $172
  non-member / $95 member (https://shop.ipc.org/IPC-2581B-WAM1-English-X); Rev C about 170 EUR at a
  reseller. The consortium site gives no spec or XSD download (https://www.ipc2581.com/support/). The
  XSD is public in practice: `IPC-2581C.xsd` is vendored in MIT-licensed diodeinc/pcb
  (`crates/ipc2581/IPC-2581C.xsd`) and in KiCad's qa data; it carries no license header, only the
  comment "Schema File for IPC-2581 revision C". The official host webstds.ipc.org/2581 returns 403.
  XSD license: [unverified]; do not vendor it without checking with IPC. No royalty or patent terms
  found; consortium members call it "open, 100% un-encrypted, public, neutrally maintained" (members page).
  Reading the format is not a problem; owning the spec text is.
- **Container:** single XML file (often zipped). Root `<IPC-2581 revision="C" xmlns="http://webstds.ipc.org/2581">`.
  Sections: `Content` (FunctionMode, StepRef, LayerRef, BomRef, AvlRef, dictionaries), `LogisticHeader`
  (Enterprise, Person, roles), `HistoryRecord` (FileRevision, SoftwarePackage, origination and
  lastChange timestamps), `Bom` (BomItem with quantity, RefDes list with `populate`, Characteristics
  with Textual name/value pairs), `Ecad` (CadHeader with `Spec`, CadData: Layer, Stackup/StackupLayer
  with thickness and tolerances, Step with Package, Component, LogicalNet or net attributes on `Set`,
  PhyNet), `Avl` (approved vendor list: MPN, vendor).
- **Knowledge carried:** BOM with quantities and DNP (`populate="false"`), part properties (value,
  LCSC number, manufacturer fields as Textual characteristics), layer stackup with materials and
  thickness, components with placement, package and pins, netlist (pins to nets), drill and
  impedance specs, approved vendors, revision history with timestamps and software, owner/enterprise.
  In the sample: 72 BomItems, 216 Components, 43 Packages, 13 StackupLayers, 233 distinct `net=`
  values on 3,087 `Set` elements, 2,838 PinRefs, 1 HistoryRecord, 0 `LogicalNet` (KiCad puts nets
  on `Set`; Allegro or Altium exports may use `LogicalNet` / `PhyNet` instead: sample gap).
- **Commonness:** consortium members page lists about 140 companies (Altium, Cadence, Siemens/Mentor,
  Autodesk, Zuken via test cases, Pulsonix, fabs like AT&S, Sanmina, TTM, OEMs like Cisco, Ericsson,
  Huawei, Lockheed Martin) (https://www.ipc2581.com/members-showcase/, tool-support list on
  https://www.ipc2581.com/support/). KiCad exports Rev C since KiCad 8 (sample made with kicad-cli
  10.0.7). 349 GitHub code hits for `webstds.ipc.org` in `.xml` (weak). EU: AT&S (Austria) and
  European fabs are members; no legal mandate found.
- **Rust crates:** `ipc2581` 0.3.3 (2026-05-18, MIT OR Apache-2.0, 122 downloads,
  https://github.com/diodeinc/pcb, 464 stars, pushed today): typed model, BOM, stackup, XSD
  validation. Same monorepo as `gerberx2`, which used 18.6 GB on a 297-byte input in the CAD spike;
  it needs the same hostile-input probe before adoption. `pcb-ir` 0.3.82 (same repo). `weft-boardchecker`
  0.0.2 (MIT, 18 downloads) reads IPC-2581 and IPC-D-356A netlists. Other OSS: IPC-2581_Serializer.NET
  (C#, no license), sjgallagher2/ipc2581 (Python, MIT, experiments), midub/boardui (AGPL-3.0, license
  noted only), KiCad exporter (GPL-3.0, not read).
- **Public samples:** consortium test cases Rev A/B/C (Cadence Allegro 16.6, Zuken CR5000, Fujitsu)
  at https://www.ipc2581.com/a-test-cases/ with no license terms stated: not redistributable as far
  as known. Downloaded: `electronics/ipc2581/de10_interface.ipc2581.zip` (KiCad 10.0.7, Rev C,
  4 layers, MIT per author's SOURCE.txt). Zip ratio 11x (1.0 MB to 11.7 MB): use the shared zip
  size caps.
- **Effort:** M. Streaming quick-xml over one big file, zip reader reused. Mapping BOM, AVL, stackup,
  components, nets and history is a few hundred lines; geometry is not needed for knowledge.
- **Fit:** P 3 (characteristics, stackup, specs), G 3 (nets, component to package to pin), T 2
  (HistoryRecord, FileRevision; usually one record), Tb 3 (BOM, AVL, stackup table).
- **Recommendation:** first wave. The richest open manufacturing format, XML, one file, exported by
  all big tools and by KiCad.

### ODB++ (ODB++Design)

- **Defining body:** Siemens (ODB++ Solutions Alliance). Current: ODB++Design Format Specification
  Release 8.1 Update 4, August 2024 (PDF revised 2024-07-16); Siemens names v8.1.2 as current
  release (https://odbplusplus.com/design/). Family also has ODB++Process and ODB++Manufacturing.
- **Openness:** two sides. Siemens says the spec "is available free for anyone to download", after
  sign-up, with "no restrictions on who can access the specification"
  (https://www.siemens.com/en-us/products/pcb/odb-plus-plus/resources/). But the PDF's own notice
  (page 2, extracted today) says: "This Documentation contains trade secrets or otherwise confidential
  information ... Access to and use of this Documentation is strictly limited as set forth in
  Customer's applicable agreement(s) with Siemens ... may not be copied, distributed", and without a
  signed agreement use is subject to the Siemens Universal Customer Agreement. Site terms forbid
  copying site content (https://odbplusplus.com/design/terms-of-use/). So the official sample
  `designodb_rigidflex.tgz` is not redistributable. Whether an MPL-2.0 reader may be written from
  this spec is [unverified]: owner decision under the AGENTS.md clean-room rule (a spec writer reads
  the PDF, an implementer writes code). Permissive readers exist (ODBPy Apache-2.0, pcb2svg BSD-3)
  and KiCad ships a GPL exporter: weak evidence that implementing is tolerated, not permission.
- **Container:** directory tree, shipped as `.tgz` or `.zip`. `matrix/matrix` (steps and layers as
  structured text `LAYER { ROW= TYPE= NAME= }`), `misc/info` (ODB version, source tool, dates,
  user), `misc/attrlist`, `steps/<step>/eda/data` (packages, nets, pins), `steps/<step>/netlists/cadnet/netlist`,
  `steps/<step>/layers/comp_+_top|comp_+_bot/components` (placement, refdes, part, BOM properties),
  `layers/<name>/features`, `drill/tools`, `stackups/`, v8: `misc/metadata.xml`, `impedance.xml`.
  Many files are Unix `compress` (`.Z`, LZW) compressed; tar + flate2 + an LZW `.Z` decoder needed
  (`newtua-lzw-z` 0.1.0, MIT OR Apache-2.0, 2,061 downloads, young; or 100 lines of own code).
- **Knowledge carried:** components with part names and attributes (BOM), nets and pin connections
  (eda/data, cadnet), layer matrix and stackup, drill tools, system attributes by class (product,
  DFM, fab, assy, test), save dates and app. Sample: ODB 7.0 from PADS PowerPCB 9.5 via
  "ODB++ Inside 9.2.0", created 2014-09-25, 132 entries, `eda/data.Z`, `comp_+_top/components.Z`,
  `netlists/cadnet/netlist`.
- **Commonness:** "supported by more than 70 software partners" (odbplusplus.com); default fab
  hand-off of Siemens Xpedition/PADS and accepted by nearly all fabs (vendor claim). KiCad exports
  ODB++ (KiCad 9+ [unverified version]). GitHub topic `odbplusplus`: 3 (weak, binary archives are not
  indexed).
- **Rust crates:** none on crates.io (`odb`, `odbpp` searches return git object DBs only). OSS:
  OdbDesign (C++, AGPL-3.0, 85 stars, license noted only), ODBPy (Python, Apache-2.0, 42 stars,
  stale 2019), pcb2svg (C++, BSD-3-Clause, 19 stars).
- **Public samples:** official rigid-flex sample (not redistributable, see above). Downloaded:
  `electronics/odbpp/lcore_5.tgz` (IIT iCub LCORE_5, CERN-OHL-W-2.0).
- **Effort:** M for components, nets, matrix, attributes (text records, own tokenizer); L if
  features and symbols were needed (they are not for knowledge).
- **Fit:** P 3, G 3, T 1 (save dates only), Tb 3.
- **Recommendation:** second wave, after the owner decides on the spec terms. IPC-2581 carries the
  same knowledge without the legal question.

### IPC-D-356 (356A / 356B) netlist

- **Defining body:** IPC. IPC-D-356B, October 2002 (supersedes 356A, January 1998), about 61 pages
  (https://shop.ipc.org/general-electronics/standards/d356-0-b-english).
- **Openness:** spec paid ($155 non-member / $109 member, DRM PDF). Table of contents public
  (https://www.ipc.org/TOC/IPC-D-356B.pdf). The record layout is widely described in public tools.
  No patents known.
- **Container:** fixed-column ASCII. `P` parameter lines (`P  JOB`, `P  UNITS CUST 0`), `C` comments,
  `317` through-hole test point, `327` SMD access, `367`/`378` outlines and conductors (356B),
  `999` end. Columns: net name, refdes, pin, hole size, access side, X/Y, pad size, rotation, soldermask.
  Verified on a KiCad 10 export (`317NET-(P3-P1)      C1    -1    D0394PA00X+055750Y-039250...`).
- **Knowledge carried:** netlist as net to (refdes, pin, position), test access side, drilled vs SMD,
  unconnected pads (`CTED-` names in KiCad). No BOM values.
- **Commonness:** part of most fab packages for electrical test; KiCad, Altium, Allegro, PADS export
  it. 374 `.d356` files on GitHub (weak, `.ipc` and `.net` extensions also used).
- **Rust crates:** none; `weft-boardchecker` 0.0.2 (MIT) reads 356A as input. OSS: gerbonara (Python,
  Apache-2.0, 22 stars) has an IPC-356 reader and writer (permissive reference).
- **Public samples:** KiCad demo exports in hanzamzamy/kairoq testdata (CC-BY-SA-4.0, share-alike);
  mesh-iit/electronics-boards-public has `.ipc` files (CERN-OHL-W-2.0). Not downloaded (file cap).
- **Effort:** S (fixed columns, < 300 lines). Pairs with the Gerber reader for a full fab package.
- **Fit:** P 1, G 3, T 0, Tb 2 (pin/net table).
- **Recommendation:** first wave. Cheap, and it gives the net graph for Gerber-only packages.

### Eagle XML (.sch, .brd, .lbr)

- **Defining body:** Autodesk (formerly CadSoft). XML since EAGLE 6 (2012); schema is `eagle.dtd`
  shipped with the program. EAGLE reached end of life on 2026-06-07; Fusion Electronics is the
  successor and still opens .sch/.brd (https://forums.autodesk.com/t5/eagle-forum/eagle-dying-on-june-7-2026/td-p/12017987).
- **Openness:** no formal spec; the DTD is readable and the files are plain XML. No license terms on
  the format found. Reading is unproblematic.
- **Container:** XML, root `<eagle version="6.5.0">`, `<drawing>` with `<settings>`, `<layers>`,
  then `<schematic>` (`libraries/library/devicesets/deviceset/devices/device/connects`, `parts/part`
  with `attribute`, `sheets/sheet/instances`, `nets/net/segment/pinref`, `variantdefs`), or `<board>`
  (`elements/element` with x/y/rot and attributes, `signals/signal/contactref`, `designrules`),
  or `<library>`.
- **Knowledge carried:** parts with value, device, package, library, user attributes (MPN etc.),
  variants (DNP per variant), nets with pin references (explicit netlist, no geometry pass needed),
  sheets, descriptions, design rules, placement on boards. Sample 555.sch: Eagle 6.5.0, 22 parts,
  11 nets, 44 pinrefs, 8 libraries embedded.
- **Commonness:** 25,792 GitHub `.sch` and 45,568 `.brd` files containing "eagle version" (weak);
  SparkFun, Adafruit and much open hardware of 2010 to 2020 are Eagle. EOL means large legacy archives
  that still need reading: raises the value of a reader. GitHub topics eagle 567, eagle-cad 275.
- **Rust crates:** none. OSS: InteractiveHtmlBom (Python, MIT, 4,606 stars, reads Eagle/Fusion),
  EagleSharp (C#, MIT), Swoop (Python, license not detected), KiCad importer (GPL, not read).
- **Public samples:** GanzYouthWorkshop/GYW-Electro-Curriculum (MIT, about 40 .sch/.brd pairs).
  Downloaded: `electronics/eagle/555.sch`. SparkFun/Adafruit files are mostly CC-BY-SA (flag).
- **Effort:** M (quick-xml; schematic netlist is explicit, about 400 to 600 lines with boards and
  libraries).
- **Fit:** P 3, G 3, T 1 (no history; version attribute only), Tb 3 (BOM from parts).
- **Recommendation:** first wave. Plain XML, explicit nets, big legacy corpus orphaned by the EOL.

### Altium Designer (SchDoc, PcbDoc, PrjPcb, SchLib, PcbLib)

- **Defining body:** Altium (Renesas subsidiary since 2024 [unverified]). Closed format, no spec.
- **Openness:** reverse-engineered public documentation: vadmium/python-altium `format.md` (WTFPL,
  195 stars), pluots/altium book (Apache-2.0), AltiumSharp (Apache-2.0), altium.js (MIT),
  universal-netlist (Apache-2.0). GPL projects noted only: altium2kicad (GPL-2.0), PyAltium (GPL-3.0),
  KiCad importer (GPL-3.0), `altium-format` crate (GPL-3.0-only, cannot be linked). Since the
  permissive docs exist, no clean-room split is needed for them; keep away from the GPL ones.
- **Container:** OLE Compound File (CFB). SchDoc streams: `FileHeader` (here 196,354 bytes),
  `Storage`, `Additional`. Records are length-prefixed pipe-delimited `KEY=VALUE` property lists:
  `HEADER=Protel for Windows - Schematic Capture Binary File Version 5.0`, then RECORD=31 sheet,
  1 component, 2 pin, 4 label, 17 power port, 25 net label, 27 wire, 29 junction, 34 designator,
  41 parameter, 44/45/46/48 implementation (footprint, sim, IBIS models). Record numbers checked
  against python-altium format.md. PcbDoc: many storages (components, nets, pads, tracks, polygons,
  rules, each with `Data` streams) [unverified detail], mixed property-list and binary records.
  PrjPcb: INI text (documents, variants, parameters, output jobs).
- **Knowledge carried:** components with parameters (value, MPN, supplier, description), footprints
  and models, designators, wires/net labels/power ports/ports/sheet entries (netlist needs a
  connectivity pass like the KiCad one), sheet hierarchy via PrjPcb, variants (not fitted, alternate
  parts), title block. Sample counts: RECORD 41 x535, 2 x86, 45 x72, 4 x53, 27 x36, 17 x32, 25 x30,
  1 x24.
- **Commonness:** Altium is the leading professional PCB tool in Europe and the US (vendor claims,
  no hard number). GitHub: 16,768 `.PrjPcb` code hits containing "Design" (weak, PrjPcb is text);
  topics altium-designer 476, altium 430. Many open hardware projects (Luxonis OAK, LimeSDR,
  AlbertaSat) are Altium.
- **Rust crates:** `altium` 0.2.1 (2023-10-23, Apache-2.0 per LICENSE file although GitHub shows
  NOASSERTION, 5,259 downloads, pluots/altium 13 stars): SchLib good, SchDoc list items, PcbDoc not
  done; stale. `altium-format` 0.1.7 (GPL-3.0-only, no). `altium-cli` (no license). `cfb` 0.15.0
  (2026-09-18, MIT, 74.5 M downloads) for the container. OSS: universal-netlist (TypeScript,
  Apache-2.0, 56 stars) builds netlists from SchDoc and OrCAD DSN and runs as an MCP server: closest
  existing product to what Rezzer would do here, good reference.
- **Public samples:** luxonis/oak-hardware (MIT, 387 SchDoc, 44 PcbDoc, pick-place, BOM .xls);
  AlbertaSat (Apache-2.0); myriadrf LimeSDR (CC-BY-3.0 per gerbonara notes). Downloaded:
  `electronics/altium/BG0249_R1M1E1_BW235_IMX378.SchDoc`. Not downloaded (file cap): the matching
  PcbDoc, 4,208,128 bytes,
  https://raw.githubusercontent.com/luxonis/oak-hardware/7d569e3ccdff30014a498dc6c64a2e0dcad6964c/BG0249_DepthAI_RGB_Camera/PCB/BG0249_R1M1E1.PcbDoc
  and the PrjPcb next to it.
- **Effort:** SchDoc M (cfb + record tokenizer + mapping + connectivity pass, reuse the KiCad net
  pass design). PrjPcb S (INI). PcbDoc L (binary records, reverse-engineered).
- **Fit:** P 3, G 3, T 1 (no history in file), Tb 3.
- **Recommendation:** SchDoc and PrjPcb first wave (very common, permissive docs, CFB is easy),
  PcbDoc second wave (or ask for IPC-2581/ODB++ export).

### Pick-and-place / centroid / XY files

- **Defining body:** none; tool conventions. Gerber X3 is the standardized alternative.
- **Openness:** conventions are public.
- **Container and columns:**
  - KiCad `.pos` CSV: `Ref,Val,Package,PosX,PosY,Rot,Side` (verified on a KiCad 10 export); ASCII
    variant with `### Module positions`, `## Unit = mm, Angle = deg.` header lines and `# Ref Val
    Package PosX PosY Rot Side` [unverified exact header].
  - Altium: free-text header block ("Altium Designer Pick and Place Locations", file path, date,
    revision, variant, "Units used: mm"), then quoted CSV; columns are user-configurable, sample:
    `"Designator","Center-X(mm)","Center-Y(mm)","Rotation","Layer","Manufacturer Part Number 1",
    "Footprint","Footprint Description","Description","Comment"`, values with unit suffix `-10.3250mm`,
    layer `TopLayer`/`BottomLayer`. Older default: Designator, Footprint, Mid X, Mid Y, Ref X, Ref Y,
    Pad X, Pad Y, Layer, Rotation, Comment [unverified].
  - Eagle `mountsmd.ulp`: `.mnt`/`.mnb`, whitespace columns name x y rotation value package [unverified].
  - JLCPCB CPL: Designator, Mid X, Mid Y, Layer, Rotation, millimetres
    (https://jlcpcb.com/help/article/how-to-generate-the-bom-and-centroid-file-from-kicad).
- **Knowledge carried:** placement table per refdes (x, y, rotation, side), often value, footprint,
  MPN; design revision and variant in the Altium header.
- **Commonness:** every assembly order; 2,320 GitHub `.pos` files with "Module positions" (weak).
  oak-hardware alone has 48 CSVs.
- **Rust crates:** not needed; Rezzer's CSV reader plus a header profile.
- **Public samples:** downloaded `electronics/centroid/altium-pick-place-BG0249_R1M1E1.csv` (MIT,
  24 rows, top only).
- **Effort:** S (header detection, unit suffix parsing, side normalization).
- **Fit:** P 2, G 0, T 0 (date/revision in Altium header only), Tb 3.
- **Recommendation:** first wave, as a CSV profile together with BOM.

### BOM CSV conventions (KiCad, JLCPCB, Octopart/Nexar)

- **Defining body:** none. JLCPCB requires Comment, Designator, Footprint, "LCSC Part #" (CSV, XLS
  or XLSX) (same JLCPCB article). KiCad 8+ BOM export: user-defined fields, default preset with
  Reference, Value, Footprint, Qty, DNP [unverified]. Octopart BOM tool / Nexar: upload maps
  arbitrary columns to MPN, Manufacturer, Quantity, refdes [unverified].
- **Openness:** public conventions.
- **Container:** CSV/TSV/XLSX; grouped rows (one line per value with refdes list) or flat rows.
- **Knowledge carried:** quantities, refdes lists, values, footprints, MPN, manufacturer, supplier
  numbers (LCSC, Digi-Key), DNP.
- **Commonness:** universal (every board order). ember-pcb and oak-hardware ship BOM xlsx/xls.
- **Rust crates:** calamine and CSV reader already in Rezzer.
- **Public samples:** BOM inside the IPC-2581 sample; MIT BOMs in akashlevy/ember-pcb (not downloaded).
- **Effort:** S: header synonyms table (Designator/Reference/RefDes, Qty/Quantity, MPN/Manufacturer
  Part Number), refdes range expansion (`R1-R4`), quantity check against refdes count.
- **Fit:** P 2, G 1 (refdes to part), T 0, Tb 3.
- **Recommendation:** first wave (with centroid): turns generic spreadsheets into a BOM table with
  checked quantities.

### ProSTEP iViP IDX / EDMD (ECAD-MCAD collaboration)

- **Defining body:** prostep ivip Association (Darmstadt, Germany), Recommendation PSI 5 "ECAD/MCAD
  Collaboration". Released document set: version 4.5, 2020-09-01 (Developer Recommendation, User
  Recommendation, Implementation Guidelines, schema, examples). IDX V6 schema "completed and released
  in December 2025" (https://www.prostep.org/en/projects/ecad/mcad-collaboration-implementor-forum);
  v6 download location [unverified].
- **Openness:** PSI 5 v4.5 package is a free download without registration
  (https://www.prostep.org/fileadmin/prod-download/PSI5_IDXv4.5_release.zip, 16 MB, fetched to a
  temporary folder only, not kept). Every page and XSD says "Copyright (C) prostep ivip Association 2006-2019, all
  rights reserved": read freely, do not vendor the XSD or the examples. Working-group participation
  needs membership. No patents mentioned.
- **Container:** XML. Root `foundation:EDMDDataSet` with namespaces foundation, pdm, property,
  geometry.d2, administration, computational, grouping, text, material (EDMD 3.0 in the sample, 4.0
  in v4.5) plus vendor `idx:` namespace (PTC 1.2.0 in the sample). `Header` (CreatorCompany,
  CreatorSystem, Creator, GlobalUnitLength, CreationDateTime), `Body` (Items with ItemType assembly
  or single, ItemInstances with 3x4 Transformation, Shapes, ShapeElements, UserProperty key/value,
  Identifier with Number, Version, Revision, Sequence), `ProcessInstruction` (SendInformation =
  baseline, SendChanges = proposal, with accept/reject and history entries).
- **Knowledge carried:** board outline and cutouts, component placement in 3D with refdes and part
  numbers, keep-outs/keep-ins, holes, layer info, properties, baseline vs incremental change
  proposals with acceptance and history entries (EDMDHistoryEntry types since v1.2), ownership.
  Sample: MGC Expedition, 2023-01-25, UNIT_MM, 64 ItemInstances, 155 UserProperties, 62 versioned items.
- **Commonness:** implemented by Cadence (Allegro X, OrCAD X), Siemens (Xpedition, PADS Pro, NX,
  Solid Edge), PTC (Creo, Onshape), Dassault (SolidWorks CircuitWorks, CATIA), Zuken (Design Force
  via EMD Collaborator), Autodesk pending (https://www.ecad-mcad.org/). Strong German automotive
  relevance (prostep is a German association) [weak: no mandate found]. GitHub: 6 code hits (weak).
- **Rust crates:** none. No OSS reader found.
- **Public samples:** prostep examples (all rights reserved, not redistributable); mesh-iit
  electronics-boards-public has 10 IDX files (CERN-OHL-W-2.0). Downloaded:
  `electronics/idx/1674666597_Baseline_LCORE_5_6775_B.idx`.
- **Effort:** M (quick-xml; xsi:type dispatch, id references across the set, transforms).
- **Fit:** P 3, G 2 (item/instance tree, references), T 3 (baseline, change proposals, history,
  versions), Tb 2 (component list).
- **Recommendation:** second wave. Unique timeline content and German relevance, but files are
  rarer outside ECAD/MCAD tool pairs.

### IPC-7351 land patterns and footprint libraries

- **Defining body:** IPC. IPC-7351B, June 2010, is the last published revision; 7351C was drafted
  but never published; IPC-7352 was planned as successor, status disputed (forum sources only:
  https://www.pcblibraries.com/forum/topic3150.html) [unverified].
- **Openness:** standard paid; the naming convention (e.g. `CAPC1005X55N`, `SOIC127P600X175-8N`,
  density suffix L/N/M) is widely published.
- **Container:** not a file format. Footprint libraries are tool formats: KiCad `.kicad_mod`
  (covered), Altium SchLib/PcbLib (CFB, see Altium), Eagle `.lbr` (see Eagle).
- **Knowledge carried:** decoding a footprint name gives package family, pitch, body size, height,
  pin count, density level: properties with units. The Altium sample uses `CAPC0402(1005)70_N`.
- **Commonness:** footprint names in most BOMs, centroid files and IPC-2581 packages.
- **Rust crates:** `vcad-ecad-package` 0.10.0 (MIT) generates IPC-7351 packages; no name decoder.
- **Effort:** S (regex table).
- **Fit:** P 2, G 0, T 0, Tb 1.
- **Recommendation:** later, as a helper used by BOM/centroid/IPC-2581 readers.

### Gerber X3 component layer

- Already covered by the own Gerber reader (cad.md lines 47, 461: X3 component characteristics
  graded E from source). Ucamco spec is free. Knowledge: refdes, MPN, manufacturer, value,
  footprint, rotation, mount type per component, which equals a centroid file plus BOM fields.
- **Gap:** no real X3 component-layer file in the corpus (cad.md line 108). A KiCad 8+ export of an
  Apache-2.0 antmicro board would close it; none found pre-exported under a permissive license
  in this session.
- **Fit:** P 2, G 0, T 0, Tb 3. **Recommendation:** covered; add a sample.

### EDIF netlist

- **Defining body:** EIA. EDIF 2 0 0 = ANSI/EIA-548-1988; 3 0 0 = EIA-618; 4 0 0 = IEC 61690-1/-2
  (2000) (https://en.wikipedia.org/wiki/EDIF). In practice only the 2 0 0 netlist view is used.
- **Openness:** standards paid (IEC via VDE); no free official copy found [unverified]. No patents.
- **Container:** s-expression text (`(edif name (edifVersion 2 0 0) (library ... (cell ... (view
  ... (interface (port ...)) (contents (instance ...) (net ... (joined (portRef ...))))))))`).
  Rezzer's iterative s-expression reader from KiCad can be reused.
- **Knowledge carried:** hierarchical netlist (cells, instances, ports, nets), properties, renames.
- **Commonness:** FPGA tools (Xilinx/AMD, Lattice) and older PCB flows (OrCAD, Mentor); 7,312
  `.edf` code hits with `edifVersion` (weak, mostly FPGA netlists).
- **Rust crates:** none for EDIF (`reticle` is a VHDL/Verilog compiler, not EDIF).
- **Public samples:** not searched in depth.
- **Effort:** S to M (s-expr reuse plus EDIF semantics, renames, arrays).
- **Fit:** P 2, G 3, T 0, Tb 1.
- **Recommendation:** later. Cheap with the s-expr reader, but low demand in PCB hand-offs today.

### OrCAD Capture / Allegro (Cadence)

- **Defining body:** Cadence. Closed formats: Capture `.DSN`/`.OLB` (OLE CFB with proprietary binary
  streams), Allegro `.brd`/`.dra` (proprietary binary).
- **Openness:** no spec. Capture can export XML (`dsn.xsd` ships with the tool, per OpenOrCadParser
  README). Public reverse-engineering: OpenOrCadParser (C++, MIT, 69 stars, docs on the container);
  universal-netlist (Apache-2.0) reads DSN including CIS variants and Do Not Stuff flags. Allegro: no
  permissive reverse-engineering found.
- **Exports:** Allegro writes IPC-2581 (Cadence produced consortium test cases 2 and 3 with Allegro
  16.6), ODB++ (via ODB++ Inside), IPC-D-356, Gerber, IDX (Allegro X MCAD collaboration), IDF.
  Capture writes EDIF, XML, Allegro netlists (`pstxnet.dat` etc.) [unverified file names].
- **Knowledge carried (DSN):** parts with properties, CIS database fields, variants, nets, hierarchy.
- **Commonness:** very common in industry and in the IIT mesh repo (64 `.dsn` files); topics orcad
  40, allegro 304 (weak, "allegro" includes a game library).
- **Rust crates:** none.
- **Public samples:** mesh-iit/electronics-boards-public orcad folders (CERN-OHL-W-2.0). Not downloaded.
- **Effort:** DSN L (CFB plus reverse-engineered streams, using MIT/Apache docs). Allegro `.brd`
  blocked (no spec, no permissive docs).
- **Fit (via exports):** P 3, G 3, T 1, Tb 3.
- **Recommendation:** later for DSN; Allegro skip, ask for IPC-2581 or ODB++ export and say so in the
  output.

### SPICE netlists (and LTspice .asc)

- **Defining body:** none; Berkeley SPICE 2/3 heritage, dialects ngspice, Xyce, HSPICE, PSpice,
  LTspice. Manuals are free (ngspice manual).
- **Openness:** format readable; no license issues for netlists. Vendor model libraries in files
  can carry their own licenses (note, do not ship).
- **Container:** line-based text: element lines (`R1 n1 n2 10k`, `X1 ... subckt`), `.subckt`/`.ends`,
  `.model`, `.param`, `.include`/`.lib`, analysis cards (`.tran`, `.ac`), continuation `+`.
  LTspice `.asc` is its own text schematic (`SHEET`, `WIRE`, `SYMBOL`, `SYMATTR`).
- **Knowledge carried:** components with values and SI suffixes (properties with units), net
  graph, subcircuit hierarchy (tree), models, parameters, simulation setup.
- **Commonness:** standard simulation interchange; 51,456 `.asc` hits with "SHEET" (weak; `.asc` is
  also other formats); 247,808 `.cir` hits for ".end" (very noisy). Topics: spice 594, ltspice 369,
  ngspice 251.
- **Rust crates:** `thevenin-types` 0.5.0 (BSD-3-Clause, 1,067 downloads, ngspice dialect parser);
  `netlist-db` 0.4.10 (MIT, 7,613 downloads, HSPICE, "under building"); `ngspice-rs` 0.4.0
  (BSD-3-Clause, 82 downloads, full simulator, young); `hauksbee-ir` 0.1.0-beta.6 (Apache-2.0,
  SPICE front-end, beta). `general-spice-core` 0.1.0 is AGPL-3.0-or-later: license noted only.
- **Public samples:** not downloaded.
- **Effort:** M (dialect handling, includes resolved inside the input folder only, SI suffix parsing).
- **Fit:** P 2, G 3, T 0, Tb 1.
- **Recommendation:** second wave (useful graph knowledge for analog designs; dialects need care).

### IBIS

- **Defining body:** IBIS Open Forum (SAE ITC). IBIS 8.0 ratified 2025-12-05; 7.2 on 2023-01-27
  (https://ibis.org/ver8.0/). Spec PDF linked publicly from that page (no registration seen); license
  terms on the spec text not stated [unverified]. IBISCHK golden parser license [unverified].
- **Container:** keyword text (`[IBIS Ver]`, `[Component]`, `[Manufacturer]`, `[Package]`, `[Pin]`,
  `[Model]`, I-V and V-T tables, `[Model Selector]`), plus EBD, ICM, Touchstone siblings.
- **Knowledge carried:** component pin table with signal names and model names, package RLC with
  min/typ/max, model types, voltage ranges, revision and date fields (`[File Rev]`, `[Date]`).
- **Commonness:** standard for signal-integrity models from every IC vendor; 391 `.ibs` hits (weak).
- **Rust crates:** none (`ibis` 0.1.0 on crates.io is a "WIP" placeholder, 8 downloads).
- **Effort:** M (keyword sections, tables; can skip I-V curves).
- **Fit:** P 2, G 1, T 1, Tb 2.
- **Recommendation:** later. Niche for a knowledge pipeline; pin tables are the useful part.

### Zuken CR-8000 (and CR-5000)

- **Defining body:** Zuken. Native Design Force / Design Gateway files closed [unverified detail].
- **Exports:** IPC-2581 (Zuken CR5000 produced consortium test cases 4, 7 and 8), IDX via EMD
  Collaborator (ecad-mcad.org), ODB++, IDF, Gerber [unverified for ODB++/IDF].
- **Commonness:** strong in Japan and in German automotive (weak, vendor reputation; Zuken E3.series
  belongs to the electrical area).
- **Rust crates / OSS:** none.
- **Effort:** native blocked. **Fit:** via exports. **Recommendation:** skip native; IPC-2581 and IDX
  readers cover it. Say so in the output when a native file arrives.

### HyperLynx (.hyp)

- **Defining body:** Siemens (formerly Mentor/Innoveda). ASCII BoardSim format; no public spec found,
  Siemens docs need a login [unverified]. Altium documents a .HYP export; other exporters [unverified].
- **Knowledge carried:** stackup with dielectric constants and copper thickness, nets, components
  with values, placement (openEMS hyp2mat tutorial).
- **OSS:** hyp2mat (C++, GPL-3.0, license noted only). No Rust crates.
- **Commonness:** 521 `.hyp` code hits with BOARD (weak).
- **Effort:** M, and spec-blocked unless a clean-room spec is written from exports.
- **Fit:** P 2, G 3, T 0, Tb 1. **Recommendation:** later.

### Fritzing (.fzz)

- **Defining body:** Fritzing project (Friends-of-Fritzing). No formal spec.
- **Openness:** app source GPL-3.0, parts and docs CC-BY-SA-3.0 (fritzing-app README). Format is
  plain XML in a ZIP; reading it does not need their code.
- **Container:** `.fzz` = ZIP with `.fz` XML (instances with moduleIdRef, properties, views with
  geometry, connectors with `connects`) plus custom `.fzp` part files and SVGs.
- **Knowledge carried:** parts with properties, breadboard/schematic/PCB connection graph, notes.
- **Commonness:** education and maker; topic fritzing 225, 91 `.fzz` code hits (weak). Low
  engineering relevance.
- **Rust crates:** none. **Samples:** fritzing-app examples are CC-BY-SA (flag); not downloaded.
- **Effort:** S (zip + quick-xml reuse).
- **Fit:** P 2, G 3, T 0, Tb 2. **Recommendation:** later (cheap, but low demand).

### Area ranking

1. **IPC-2581** - richest open hand-off (BOM, AVL, stackup, nets, history) in one XML file, exported
   by Cadence, Siemens, Zuken, Altium and KiCad. M effort on quick-xml.
2. **Altium SchDoc + PrjPcb** - the most common professional schematic source; permissive
   reverse-engineering docs; `cfb` crate is mature. Reuse the KiCad connectivity pass.
3. **Eagle XML** - plain XML with explicit nets; large legacy corpus orphaned by the 2026-06-07 EOL.
4. **BOM and centroid CSV profiles** - S effort on the existing CSV/XLSX readers; every assembly
   package has them. Do together.
5. **IPC-D-356** - S effort, adds the net graph to Gerber-only fab packages next to the Gerber reader.
6. **ODB++** - same knowledge as IPC-2581, very common at fabs, but the spec's confidentiality notice
   needs an owner decision first; text records plus `.Z` decoding.
7. **IDX** - unique timeline (baseline and change proposals), German body; M effort.
8. **SPICE netlists** - graph and values for analog designs; dialects make it M.
9. **Altium PcbDoc** - L effort; IPC-2581/ODB++ exports cover most of it.
10. Later: EDIF, OrCAD DSN, IBIS, HyperLynx, Fritzing, IPC-7351 name decoder. Skip native: Allegro,
    Zuken (point the user to IPC-2581/ODB++/IDX exports).

### Open questions / unverified

- IPC-2581 Revision D existence and date; license of `IPC-2581C.xsd` (vendored widely, no header,
  IPC copyright assumed). Ask IPC before shipping the XSD.
- ODB++: may an MPL-2.0 project implement a reader from a spec marked "trade secrets ... confidential"
  under the Siemens UCA? Owner decision; consider asking Siemens / the Solutions Alliance in writing.
- IDX v6 schema download and terms; whether v6 changes namespaces from EDMD 4.0.
- Altium PcbDoc stream list, Altium ASCII save format, and Altium ownership date (Renesas).
- KiCad version that added ODB++ export; KiCad BOM default columns; KiCad ASCII `.pos` header text.
- Eagle `mountsmd.ulp` column order; Octopart/Nexar BOM column mapping.
- IBIS spec text license and IBISCHK license.
- Zuken native format names and exact export list; Capture netlist file names.
- Samples still missing: real Gerber X3 component layer, Allegro or Altium IPC-2581 (to test
  `LogicalNet`/`PhyNet`), Altium PcbDoc (URL above), OrCAD DSN (mesh-iit), IPC-D-356 (mesh-iit `.ipc`),
  SPICE, IBIS, Fritzing.
- Two samples come from CERN-OHL-W-2.0 (weakly reciprocal: redistribution fine, modifications of the
  covered files must stay under the license). Flagged like CC-BY-SA in the CAD manifest.
- The IDX sample header names a creator (`MBorgagni`) as published by IIT; it is public data, but a
  renderer will print it as author.

## 7. Findings across areas

1. **The syntax is easy, the semantics is the work.** Almost every recommended format is XML,
   sometimes inside an OPC or plain zip package, or a simple text format. All of them sit on the
   existing zip + quick-xml base, calamine or the CSV reader. No crate gives the knowledge layer:
   for AAS, AutomationML, DEXPI, GAEB, BMEcat, ReqIF, AP242 XML, QIF, IPC-2581 and Eagle there is
   no usable Rust reader at all, or only a young one with partial coverage. This matches the CAD
   spike: own readers, crates only for containers (zip, cfb, 7z, SQLite).
2. **Dictionary licenses matter more than format licenses.** Files carry ECLASS IRDIs, ETIM codes,
   IEC CDD IRDIs, POSC Caesar RDL URIs, bSDD URIs, DIN 276 codes and UNECE unit codes. Rezzer can
   always show the code and the names that the file itself carries. ECLASS Terms of Use 4.3
   (2022, newest found) allow a recipient to store, use and convert ECLASS content found in a
   file (sections 2.1 (4) and 4.1.2) but forbid adding further ECLASS content (4.1.3). So Rezzer
   must **never resolve ECLASS codes from a bundled dictionary or the paid web service**, and
   should print the notice of section 5.3 ("This content contains ECLASS. The ECLASS Terms of Use
   apply (www.eclass.eu).") when it finds ECLASS content. The English ETIM model is ODC-BY and
   could be bundled with attribution (owner decision). DIN 276 and ONORM group names are paid
   standard text: show codes, do not ship name tables.
3. **Several shared modules pay off across areas:**
   - an IEC 81346 / 61355 designation parser (S) that turns `=GB1+A1-U91:3` and `&EFS` strings
     into plant trees and wiring end points, for EPLAN, AML, terminal lists, DXF title blocks
   - a classification code layer (S) for the codes in point 2, with the license rules
   - table profiles on calamine/CSV (S each): mechanical BOM exports, electronics BOM and
     centroid files, wiring lists and terminal plans. One header-synonym mechanism (English and
     German headers), tree from item numbers or level column, refdes range expansion, quantity
     check, and the output says when a tree or role was inferred
   - one CAEX reader for AutomationML, MTP, EPLAN EPDZ and TIA CAx exports
   - one GML layer for CityGML, XPlanung and OKSTRA
   - the OPC package walk (exists) for AASX, AMLX, IPC-2581 zips; `cfb` for Altium, Revit and
     OrCAD; the s-expression reader for EDIF
4. **Licensing blockers** (details in the area sections):
   - GPL/AGPL crates that must not be linked: openbimrs bcf, ifc-xml, okstra, mmc and the
     ifc-schema audit features of openbim-ids, axioval-bcf, reqif-rs, doorstop-reqif,
     altium-format, general-spice-core. Permissive alternatives or own readers exist for each
   - member-only or paid specs: ESI (ETG forbids mirroring), GSDML (PI members), ODVA EDS
     (subscription), JT and IPC texts (paid), ONORM A 2063 (paid), SIA 451 and CPIXML (not public)
   - documentation terms that block the docs route: 3D XML (internal business use only), ODB++
     (PDF marked confidential under the Siemens UCA, although Siemens says it is free to download)
   - schemas that may be read but should not be bundled: GAEB XSDs (no license text), IPC-2581C
     XSD (no license header), prostep IDX XSD ("all rights reserved"). Rezzer does not need them
     to read files
   - manufacturer data that cannot enter the corpus: EPLAN Data Portal EDZ, vendor IODD, GSDML,
     ESI and EDS files
5. **Correction to the task:** VDI/VDE 3685 is about adaptive controllers (1990/1992), not
   reference designations. The relevant standard is DIN EN IEC 81346 (and VGB-S-823 RDS-PP for
   power plants [unverified]).
6. **German mandates found:** GAEB DA XML is required by state building administrations (for
   example Niedersachsen SBN 2024 requires GAEB XML 3.2 and forbids 3.3 for now); the VHB Bund
   form only says electronic offer data follows GAEB DA XML (2017 form seen); VOB/A does not name
   GAEB. XPlanung and XBau are binding by IT-Planungsrat decision 2017/37 (secondary sources,
   decision text not fetched). OKSTRA is introduced for federal trunk roads (ARS 12/2000). No
   mandate was found for AAS (EU Digital Product Passport carrier status [unverified]).

## 8. Recommended order for Rezzer

Rules used: open and well specified first, then common (with German and EU weight), then rich
structure (BOM, tree, graph, timeline), then low effort through reuse. The owner's area order
breaks ties. Where the order leaves the area order, the reason is given.

### 8.1 Shared modules (built together with the first reader that needs them)

| Module | Used by | Effort |
|---|---|---|
| Classification code layer: ECLASS, ETIM, IEC CDD IRDIs, POSC Caesar RDL, bSDD URIs, DIN 276 / ONORM B 1801 codes, UNECE units; never resolves ECLASS from a dictionary; prints the ECLASS notice | AAS, AML, DEXPI, BMEcat, xChange, GAEB, IDS, IFC, EPLAN | S |
| IEC 81346 / 61355 designation parser with tree builder and Mermaid output | AML, EPLAN, wiring lists, DXF title blocks, DEXPI tags | S |
| Table profiles on calamine / CSV: BOM (mechanical and electronics), pick-and-place, wiring list and terminal plan | CAD/PDM exports, KiCad/JLCPCB/Altium CSV, EPLAN and WSCAD reports | S each |
| CAEX reader (2.15 and 3.0, class inheritance, links) | AutomationML, MTP, EPLAN EPDZ, TIA CAx | part of AML |

### 8.2 First wave, in this order

| # | Format | Area | Reason |
|---|---|---|---|
| 1 | Asset Administration Shell (AASX XML/JSON) | automation | CC-BY-4.0 specs and samples, German and EU policy weight, maps directly onto properties with units, BOM entity trees and document lists with versions; reuses the OPC package walk |
| 2 | AutomationML / CAEX (.aml, .amlx) | automation | open XSDs, German origin, plant tree plus link graph; the CAEX reader is reused by MTP, EPLAN EPDZ and TIA exports |
| 3 | GAEB DA XML 3.x (all phases) | construction | the most important construction format in Germany, required by state administrations, open XSD, BoQ tree with quantities and prices ("tables with context") |
| 4 | DEXPI 1.x (Proteus) | automation | the only open P&ID format, CC-BY-4.0 reference P&IDs, best pipe and signal graph of all formats here; DEXPI XML 2.0 added once a public sample exists. Placed after GAEB because it is less common |
| 5 | BMEcat 2005 / 2005.2 + ETIM xChange 2.0 | electrical | open spec (BMEcat) and Apache-2.0 (xChange), the common structured product catalogs in DE trade, properties with units and classifications; S to S-M |
| 6 | Wiring list / terminal plan, CAD BOM and electronics BOM / centroid table profiles | electrical, mechanical, electronics | leaves the area order on purpose: one S mechanism on existing readers covers the most common real BOM and wiring sources of three areas |
| 7 | BCF-XML 2.1 / 3.0 and IDS 1.0 | construction | S each, open; BCF ties dated issues and comments to IFC GlobalIds (timeline + graph, the "comments stay attached" principle); IDS is a clean requirements table |
| 8 | DATANORM 4 / 5 | electrical | S, very common in German electrical and SHK trade; needs CP850 decoding (oem_cp, MIT) |
| 9 | KBL, then VEC (wire harness) | electrical | MIT-licensed spec, XSDs and official samples from the standards body; a real BOM plus connection list and topology graph; German automotive OEM-supplier standard. Added when merging |
| 10 | ReqIF 1.2 | mechanical | free OMG spec, standard OEM-supplier exchange in German automotive, typed relations between documents with change dates; S-M |
| 11 | STEP AP242 XML (3001 ed1/ed2, 4442 ed3) | mechanical | free XSDs, the only STEP gap left after stepq; reuses the stepq BOM model and renderer; AS1 sample cross-checks stepq |
| 12 | QIF 3.0 | mechanical | free BSL-1.0 XSDs, PMI plus inspection results (timeline), pairs with the NIST STEP files already in the corpus |
| 13 | IPC-2581 rev C | electronics | richest open-in-practice PCB hand-off (BOM, AVL, stackup, nets, history) in one XML file, exported by all big tools and KiCad; own reader preferred, ipc2581 crate only after a hostile-input probe |
| 14 | Eagle XML | electronics | plain XML with explicit nets; large legacy corpus orphaned by the 2026-06-07 end of life |
| 15 | Altium SchDoc + PrjPcb | electronics | most common professional schematic source; permissive reverse-engineering docs; cfb is mature; reuses the KiCad connectivity pass design |
| 16 | IPC-D-356 | electronics | S; adds the net graph to Gerber-only fab packages next to the existing Gerber reader |

Rough size of the first wave from the per-format estimates: about 9,000 to 12,000 lines of own
reader code, all on existing dependencies except `cfb` 0.15.0 (MIT) for Altium and `oem_cp`
2.1.2 (MIT) for the CP850 text of DATANORM.

### 8.3 Second wave

In this order: OPC UA NodeSet2 (bundle the MIT base NodeSet for names), PLCopen XML (covers
CODESYS exports), IEC 61850 SCL, EPLAN EPDZ and EDZ (after AML; new dependencies sevenz-rust2 and
SQLite), CityGML and XPlanung (shared GML layer, CityGML 1.0 AdV profile first), LandXML, IO-Link
IODD (once a licensed or self-written sample exists), MTP, ifcXML, COBie, Revit metadata via cfb,
IPC-2571 PDX, SAP IDoc XML, PLM XML (needs a licensed sample), ODB++ (after the owner decision),
prostep IDX, SPICE netlists, Altium PcbDoc.

### 8.4 Later and skip

- Later: ESI, GSDML, EDS (member or paid specs), MTConnect, L5X, SimaticML, TwinCAT, OKSTRA, XBau,
  GAEB 90/2000 (spec missing), ONORM A 2063, CPIXML, ICDD/MMC, IFC5, CityJSON, ELDANORM, JT,
  3D XML (clean-room from files only), EDIF, OrCAD DSN, IBIS, HyperLynx, Fritzing, IPC-7351 name
  decoder, Revit elements via the rvt crate after a spike on real files.
- Skip (detect and say what to export instead): CODESYS native (export PLCopen XML), ArchiCAD
  (IFC), EPLAN native project (EPDZ, AML or Excel), Zuken and WSCAD native (their exports),
  Allegro and Zuken CR-8000 native (IPC-2581, ODB++ or IDX), SIA 451/CRBX, DIN SPEC 91400,
  NE 100, OMG PLM Services, OSLC, VDA 4965, VDA 4953, VDA-FS, OCI, IDS-Connect, ELBRIDGE, UGL,
  IEC 62683 and IEC CDD as formats (their IRDIs go through the code layer), RAMI 4.0, VDE-AR.

### 8.5 Decisions for Jordi

1. ECLASS: confirm the rule "show in-file codes and names, never resolve, print the section 5.3
   notice". Check for a newer ECLASS Terms of Use than 4.3 before release.
2. ETIM: bundle the English ETIM model (ODC-BY, with attribution) to resolve codes offline, or not.
3. ODB++: may a clean-room reader be written from a spec PDF marked confidential under the
   Siemens UCA (spec writer and implementer split per AGENTS.md)? Or ask Siemens in writing.
4. CC-BY-ND-4.0 test files (BCF, IDS): unmodified redistribution is fine, but expected-output
   reference files made from them may count as adaptations.
5. New container dependencies for EPLAN: sevenz-rust2 (Apache-2.0) and either rusqlite with
   bundled SQLite (C, public domain, linked in-process) or turso_core (pure Rust, young). Open:
   does sevenz-rust2 read every EDZ/EPDZ compression method?
6. Corpus provenance: two candidates were not copied and need a decision: the Weidmueller
   BMEcat (the only public ECLASS Advanced catalog with IRDIs, but real vendor data in an
   Apache-2.0 repo without stated permission, so it failed the rule that a repo license only
   covers what the repo owner wrote) and the 6.5 MB public EPDZ (EPLAN's sample project in an MIT
   repo). Kept with a flag: halo DATANORM texts (look like real product texts), ReqIF samples
   (Eclipse RMF origin, EPL likely), PDX sample (embeds the IPC DTD text), KBL sample (Volkswagen
   test data published by prostep ivip under MIT), ODB++ and IDX samples (CERN-OHL-W-2.0, weakly
   reciprocal).

### 8.6 Main open questions

- Schema license texts: GAEB, XPlanung, XBau, OKSTRA, LandXML, PLM XML, IPC-2581C, IEC SCL, IODD.
- No public samples yet: DEXPI 2.0 XML, AP242 4442 ed3, PLM XML, COBie (clean license), a real
  AML tool export (TIA or EPLAN with a license), IODD, real Gerber X3 component layer, wiring
  lists and CAD BOM exports (to be written as CC0 synthetic files).
- MTP container layout, TIA SimaticML and TwinCAT element names: only third-party sources.
- KBL and VEC: the VDA recommendation numbers (4964, 4968) and a tool support list were not
  confirmed from a primary source; the check was short (added when merging).

## Appendix A: sample files

All files are in `spike/corpus/engineering/<area>/` (the `spike/` folder is gitignored), listed
with source URL, license and SHA-256 in `spike/corpus/engineering/MANIFEST-engineering.md`. The
table below is the short form. All files are untrusted data and were only inspected with `head`,
`grep`, `xmllint --noout --nonet`, `unzip -l`, `tar t` and an OLE stream listing.

| File | License | Size (bytes) |
|---|---|---|
| `automation/automationml/big-valid-packaging-line.aml` | MIT | 29535 |
| `automation/opcua/Opc.Ua.Machinery.Examples.NodeSet2.xml` | MIT | 483329 |
| `automation/aas/IDTA_02003_Sample_TechnicalData.aasx` | CC-BY-4.0 | 574574 |
| `automation/aas/IDTA_02004-2-0_Example_HandoverDocumentation.aasx` | CC-BY-4.0 | 124294 |
| `automation/dexpi/C01V04-VER.EX01.xml` | CC-BY-4.0 | 445726 |
| `automation/plcopen/cdl-plc_PID_plc.xml` | BSD-3-Clause-LBNL | 73334 |
| `construction/gaeb/boq.x83` | MIT | 5903 |
| `construction/gaeb/invoice.x89` | MIT | 4371 |
| `construction/bcf/Milestone.bcf` | CC-BY-ND-4.0 | 61468 |
| `construction/ids/IDS_wooden-windows.ids` | CC-BY-ND-4.0 | 13132 |
| `construction/citygml/historicalSuccession_CityGML_3.0_LOD2_Versioning.gml` | MIT | 88607 |
| `construction/landxml/railcomplete-005-alignment-with-cant.xml` | MIT | 3565 |
| `electrical/bmecat/olivere-new_catalog-2005.xml` | MIT | 7322 |
| `electrical/etim-xchange/ETIM-xChange_SampleFile_V2.0-2025-11-27.json` | Apache-2.0 | 27705 |
| `electrical/datanorm/halo-v4_with_texts.001` | MIT | 2724 |
| `electrical/datanorm/halo-v5_texts_products_services.001` | MIT | 88484 |
| `electrical/harness/vw-oldbeetle-kbl23-generator-cable.kbl` | MIT | 34901 |
| `electrical/harness/vec-routing-examples.vec` | MIT | 20552 |
| `mechanical/qif3_widget_results.qif` | BSL-1.0 | 70383 |
| `mechanical/qif3_nist_ctc_01_mbd.qif` | BSL-1.0 | 764631 |
| `mechanical/ap242_bo_xml_as1.stpx` | Apache-2.0 | 38810 |
| `mechanical/reqif_rmf_spec_relations.reqif` | Apache-2.0 | 5654 |
| `mechanical/reqifz_with_attachment.reqifz` | Apache-2.0 | 4108 |
| `mechanical/pdx_ipc2571_example.xml` | Apache-2.0 | 495105 |
| `electronics/ipc2581/de10_interface.ipc2581.zip` | MIT per author's testdata/de10_interface/SOURCE.txt | 1047802 |
| `electronics/odbpp/lcore_5.tgz` | CERN-OHL-W-2.0 | 64677 |
| `electronics/idx/1674666597_Baseline_LCORE_5_6775_B.idx` | CERN-OHL-W-2.0 | 235532 |
| `electronics/altium/BG0249_R1M1E1_BW235_IMX378.SchDoc` | MIT | 200704 |
| `electronics/centroid/altium-pick-place-BG0249_R1M1E1.csv` | MIT | 5324 |
| `electronics/eagle/555.sch` | MIT | 314602 |
| `electronics/licenses/kairoq-de10_interface-SOURCE.txt` | MIT statement | 497 |
| `electronics/licenses/mesh-iit-LICENSE.txt` | CERN-OHL-W-2.0 text | 14540 |
| `electronics/licenses/luxonis-oak-hardware-LICENSE.txt` | MIT text | 1064 |
| `electronics/licenses/GYW-Electro-Curriculum-LICENSE.txt` | MIT text | 1076 |
