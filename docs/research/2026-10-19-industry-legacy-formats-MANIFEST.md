# Industry and legacy formats corpus manifest (research 2026-10-19)

Public sample files for `docs/research/2026-10-19-industry-legacy-formats.md`, one folder per
area. Paths are relative to this folder. Downloaded 2026-10-19. All files are untrusted data:
never execute them, keep scripts outside this folder. Only `file`, `head`, `xxd`, `grep`, `wc`,
`unzip -l`/`-p`, `xmllint --noout --nonet` and `shasum` were used on them (plus `python3 -I`
scripts kept in the session scratchpad that read a file as data, for STDF record counts and an
mzML/ELN peek; the science-data agent also ran `strings` and `fold` on two staged candidates in its scratchpad before choosing). SHA-256 values were recomputed with `shasum -a 256` when merging and match the
values the research agents reported (each hash checked to appear exactly once in this file).

## Automotive

| File | Source URL (pinned) | License | SHA-256 | Size (bytes) | What it tests |
|---|---|---|---|---|---|
| `automotive/opendbc-tesla_powertrain.dbc` | https://raw.githubusercontent.com/commaai/opendbc/48c7f06c6f27c41e7169242acb03ef9775658deb/opendbc/dbc/tesla_powertrain.dbc | MIT, Copyright (c) 2020 Comma.ai, Inc. | d5fe66f2c1888a769ba73fb7b6c43da62ba0789da6193b170483bbcf51967930 | 7716 | DBC: 11 nodes, 6 messages, about 76 signals with signed factor and offset and units, 45 `VAL_` value tables, no `CM_` |
| `automotive/xcp-lite-all_features_demo.a2l` | https://raw.githubusercontent.com/vectorgrp/xcp-lite/86c702ab605c0dabb429d1e80dd6e8e47d17926d/tests/fixtures/all_features_demo.a2l | Apache-2.0 (repo license; crate MIT OR Apache-2.0), Vector Informatik xcp-lite | 133b7b87d43259bb67a6a3ff4c3c79565a2e4d32f76a114de8228664a4601bc2 | 29286 | A2L 1.71 with BOM: 46 MEASUREMENT, 39 CHARACTERISTIC, COMPU_METHOD (LINEAR, TAB_VERB, COMPU_VTAB), RECORD_LAYOUT, AXIS_PTS, CURVE and MAP, TYPEDEF and INSTANCE, IF_DATA XCP; unresolved `/include "XCP_104.aml"` must be reported |
| `automotive/cantools-system-4.2.arxml` | https://raw.githubusercontent.com/cantools/cantools/c6b5ab4aefae7ae62207fb2de7ab31b9a68b0110/tests/files/arxml/system-4.2.arxml | MIT (cantools, Erik Moqvist; origin before 2019 [unverified]) | c6bf5671279bc7ec4f81ebbe48f430d5a7049abd04fbf279c8080a193c973fb2 | 71059 | ARXML r4.0 with BOM: 22 packages, 8 CAN frames, 19 I-signals, 13 system signals, PDU mappings, COMPU-SCALEs with rational coefficients, cross references |
| `automotive/esmini-junction_simple_unordered_road_elements.xodr` | https://raw.githubusercontent.com/esmini/esmini/06065f929a7b7031098f9fa061afac3c7249d0e7/EnvironmentSimulator/Unittest/xodr/junction_simple_unordered_road_elements.xodr | MPL-2.0 (esmini's own unit test, not in its 3rd_party_terms) | c30aa13278c20ce6ee18f0851d48923b758da411232ca0cbc293aec6c626c804 | 6533 | OpenDRIVE 1.5: 5 roads, 1 junction with 4 connections, predecessor and successor links out of order (topology graph) |

## Automation and energy

| File | Source URL (pinned) | License | SHA-256 | Size (bytes) | What it tests |
|---|---|---|---|---|---|
| `automation-energy/comtrade/sample_ascii.cfg` | https://raw.githubusercontent.com/dparrini/python-comtrade/fd8a60fba591f8aebcd849ed6b1585247bdfd95d/tests/sample_files/sample_ascii.cfg | MIT (python-comtrade, dparrini and contributors) | 45ff4ba994bd473f99d20d4d6a10890474600b60393b770a5afe110f4c991c47 | 487 | COMTRADE 2013 cfg: 4 analog channels with a/b conversion and units, 4 digital channels, 60 Hz, sample rate, trigger times, local time code |
| `automation-energy/comtrade/sample_ascii.dat` | https://raw.githubusercontent.com/dparrini/python-comtrade/fd8a60fba591f8aebcd849ed6b1585247bdfd95d/tests/sample_files/sample_ascii.dat | MIT (as above) | 8674c7619591861964fd0e894cbcdc7634547954636cac2eedec869ccf361743 | 1276 | ASCII `.dat` paired with the cfg by stem; generic `.dat` detection must not claim it alone |
| `automation-energy/cgmes/20220615T2230Z__Nordheim_EQ_1.xml` | https://raw.githubusercontent.com/entsoe/relicapgrid/f09bb340795122b43d6499e913c121bb21458859/Instance/Nordheim/Grid/cimxml/20220615T2230Z__Nordheim_EQ_1.xml | CC BY-SA 4.0, (c) ENTSO-E, ReliCapGrid test configuration (reference to ENTSO-E required) | e0b7ef524c560e545732af967c683841c15df127213069d35e2aeb6400b88221 | 11683 | CGMES 3.0 CoreEquipment profile, CIM100 namespace, `md:FullModel` header; two `Model.DependentOn` files not in the corpus (unresolved references must be reported); topology graph |
| `automation-energy/nmea0183/nmea_with_sat_info.log` | https://raw.githubusercontent.com/AeroRust/nmea/32d14a8345775e737657479f06c585099d551adc/tests/data/nmea_with_sat_info.log | Apache-2.0 (Felix Obenhuber 2016, AeroRust 2020) | 6f744aabf7954636cc02d86300805d1c7afe27f48c79755805bb87acb9149195 | 28818 | NMEA 0183, CRLF, mixed talkers (GP, GN, GL, GA, proprietary), GSV satellites, empty no-fix sentences, checksums |

## Aerospace and space

| File | Source URL (pinned) | License | SHA-256 | Size (bytes) | What it tests |
|---|---|---|---|---|---|
| `aerospace-space/xtce/BogusSAT-1.xml` | https://raw.githubusercontent.com/greglucas/space-data-toolkit/ce292f75efb325c4e751cb757bfede3638bb3ebf/tests/BogusSAT-1.xml | Apache-2.0 per the in-file header "Copyright 2015 David Overeem" (hosting repo MIT) | ec2b2a13aee4d73fb461ef973f3ebdec0724dab05cc3316451295f954cf32c66 | 85380 | XTCE 1.2 (`http://www.omg.org/space/xtce`): 11 polynomial and 6 spline calibrators, MathOperation, alarms, 12 MetaCommands, units, container inheritance |
| `aerospace-space/ccsds-ndm/oem_g11.kvn` | https://raw.githubusercontent.com/JochimMaene/ccsds-ndm/cb17f74acf752a20cd2b0c99664fc7ed7d466882/ccsds-ndm/data/kvn/oem_g11.kvn | MPL-2.0 (repo). Content appears to be the CCSDS 502.0-B-3 Annex G example [upstream provenance and CCSDS reproduction terms unverified] | 892a3a3cf8b3a3e045659a195a5e990bcf5ca48e1a126b67e4967c0d128286a6 | 1617 | CCSDS OEM 3.0 KVN: header, two META blocks, COMMENT provenance, ephemeris series |
| `aerospace-space/spice/naif0012.tls` | https://naif.jpl.nasa.gov/pub/naif/generic_kernels/lsk/naif0012.tls (not commit-pinnable; identified by SHA-256 and retrieval date) | NASA/JPL NAIF generic kernel; NAIF rules cover toolkit redistribution, kernel terms not explicit [unverified]; credit NAIF | 678e32bdb5a744117a467cd9601cd6b373f0e9bc9bbde1371d5eee39600a039b | 5257 | SPICE `KPL/LSK`: `\begindata`/`\begintext`, leap second table, history; `file` misdetects it as LaTeX |
| `aerospace-space/pds4/kgrs_calibrated_spectra_per1_LF_VALID.xml` | https://raw.githubusercontent.com/NASA-PDS/validate/10180bc6a89c597eaacf967807d0df94d4bc3dd5/src/test/resources/github292/table_delimited/kgrs_calibrated_spectra_per1_LF_VALID.xml | Apache-2.0 (NASA-PDS/validate); describes Kaguya GRS (JAXA) data [upstream provenance unverified] | a218682efe8170f8c4903236447bbdc9d6a166933d163a31c3bb2ea357fb55b8 | 8166 | PDS4 `Product_Observational`: identification, observation area, `Table_Delimited` fields with units |

## Engineering models and legacy

| File | Source URL (pinned) | License | SHA-256 | Size (bytes) | What it tests |
|---|---|---|---|---|---|
| `models-legacy/simulink-stateflow-RPS_HW_2025.slx` | https://raw.githubusercontent.com/mathworks/Learn-Stateflow-with-Micro-bit/095f5ea0a91d0bf393497813a5990d03a310f47d/Finished%20Models/RPS_HW_2025.slx | BSD-3-Clause, Copyright (c) 2024, The MathWorks, Inc. | e7f2247739af4ec394696235c6f6ba8e4b44a903e25ed63d9424fea2e6cf9e80 | 41905 | OPC zip, 27 parts: block graph (Block SID, Line Src/Dst ports), subsystem, Stateflow chart, coreProperties (creator, modified, revision, R2024b), library references |
| `models-legacy/msl-ChuaCircuit.mo` | https://raw.githubusercontent.com/modelica/ModelicaStandardLibrary/fd2d0af77f7cc0d2b5b2f0975cffe96d71b92b02/Modelica/Electrical/Analog/Examples/ChuaCircuit.mo | BSD-3-Clause, Modelica Standard Library, Modelica Association and contributors | dcfd13bdb7daf5ca39ed5b09e241299223b9f75b06491e912d68211d61afb2c4 | 3624 | Modelica: parameters with start/fixed, connect graph, placement annotations, HTML documentation (libmagic says HTML), gettext `.mo` collision |
| `models-legacy/reffmu-Feedthrough-FMI3.xml` | https://raw.githubusercontent.com/modelica/Reference-FMUs/86ca571ec1ed4bbb4950a9355c2b88ff8ec2f78e/Feedthrough/FMI3.xml | BSD-2-Clause, Copyright (c) 2026 Modelica Association Project "FMI" | 9aa5af79010e8bd76044d74e322071fe9b3678e247dad1e1983ede2f730a9cd2 | 7201 | FMI 3.0 modelDescription: all variable types, causality, variability, initial, ModelStructure output dependencies (graph); no units |
| `models-legacy/cobrix-test1_copybook.cob` | https://raw.githubusercontent.com/AbsaOSS/cobrix/c1716605004b50045d905229566167f640505868/data/test1_copybook.cob | Apache-2.0, Copyright 2018 ABSA Group Limited (header in file) | ab391d156613cdc77a836ac5ec2977dafcc552804fbd9d0bb80d5823cc3c123d | 2270 | Copybook: `PIC S9(4) COMP`, `PIC 9(5) COMP-3`, REDEFINES, `OCCURS 80 DEPENDING ON`, group levels |
| `models-legacy/cobrix-test1-example.bin` | https://raw.githubusercontent.com/AbsaOSS/cobrix/c1716605004b50045d905229566167f640505868/data/test1_data/example.bin | Apache-2.0, ABSA Group Limited (repo license) | 4835d1d96e588d50cad013a0eb25bb0cac331b238418a748afeabf9c9a35c96b | 22020 | EBCDIC data for the copybook above: binary COMP, packed COMP-3, EBCDIC text, variable records from ODO |

## Science

| File | Source URL (pinned) | License | SHA-256 | Size (bytes) | What it tests |
|---|---|---|---|---|---|
| `science-data/ref_nccopy3_subset.nc` | https://github.com/Unidata/netcdf-c/blob/bc148bf800d17525075c758f95c8adbaba964366/ncdump/ref_nccopy3_subset.nc | BSD-3-Clause, Copyright 2025 Unidata | 90ca9c957604ace697c7d94a0d5d36e07752904351636a764220d91c5cc18019 | 65284 | NetCDF classic CDF-1: many dimensions and variables, `Conventions = CF-1.5`, multiline global `history`, char dimensions |
| `science-data/ref_provenance_v1.nc` | https://github.com/Unidata/netcdf-c/blob/bc148bf800d17525075c758f95c8adbaba964366/ncdump/ref_provenance_v1.nc | BSD-3-Clause, Copyright 2025 Unidata | 362a59eec9a5c2e0adaff1ac26d328eac64502732ec811cbdb99b635ec2d0ffc | 1469 | NetCDF-4 / HDF5: `_NCProperties`, subgroup, dimension scale, `_Netcdf4Dimid` (NetCDF-4 vs plain HDF5 detection) |
| `science-data/j94f05bgq_flt.fits` | https://github.com/astropy/astropy/blob/65f4f6b427865da51c59822878f09abaed809312/astropy/wcs/tests/data/j94f05bgq_flt.fits | BSD-3-Clause, Copyright Astropy Developers (HST ACS data, NASA/STScI) | 900038e0d853828140a757e2656934cb268ff9f315c5c6f617de85a632ad526b | 83520 | Multi-extension FITS: primary plus 6 IMAGE extensions, `RA---TAN-SIP` WCS with SIP, `BUNIT`, 41 HISTORY cards |
| `science-data/reference-testdata-headeronly.mseed3` | https://github.com/EarthScope/libmseed/blob/d6e6ad306de5b7bdde072e5a5e3d58ddcc11abd8/test/data/reference-testdata-headeronly.mseed3 | Apache-2.0, EarthScope libmseed | 34d8bd9c92c902ee32a1f33faecddbd88d25f4ee849f35c5d3acdabfe21c7cfb | 789 | miniSEED 3 record with JSON extra headers: clock exceptions, clock model, vendor namespaces |
| `science-lab/BIOMD0000000012.xml` | https://www.ebi.ac.uk/biomodels/model/download/BIOMD0000000012?filename=BIOMD0000000012_url.xml (not commit-pinnable; identified by SHA-256 and retrieval date) | CC0 1.0 (declared in the file; BioModels, Elowitz and Leibler 2000) | d535c84bc5fe68b01728cac5c53f8fed971176e3f0f2367a9ba1e9c043b11a2d | 43901 | SBML L2V3 repressilator: 6 species, 12 reactions with MathML kinetic laws, 16 parameters, 9 rules, units, RDF annotations, notes, SBO terms |
| `science-lab/mzdata-three_test_scans.mzML` | https://raw.githubusercontent.com/mobiusklein/mzdata/bc6148e968d3994efadd0110527294a9528f7501/test/data/three_test_scans.mzML | Apache-2.0 (repo license of mobiusklein/mzdata; separate data license [unverified]) | 62d105570de9620a7d84285487ba6e509d38f41fadfcf176dc04b7759b878ed3 | 237646 | indexedmzML 1.1, PSI-MS CV terms, Q Exactive config, software chain, 1 MS1 and 2 MSn spectra, zlib arrays |
| `science-lab/sciformats-CompoundFile.jdx` | https://raw.githubusercontent.com/devrosch/sciformats/bf3d3ff1fcdea257f560c9a983cc57aa351710e9/lib-rs/tests/jdx/resources/CompoundFile.jdx | MIT, Copyright (c) 2025 Robert Schiwon (synthetic) | 8912d9cbb7db25be16f2cf102fe51780896e3fcf68fee238b41939bfae5ec3b9 | 3981 | JCAMP-DX LINK file: 5 blocks, PAC, AFFN, ASDF NTUPLES, peak table and assignments |
| `science-lab/kadi4mat-records-example.eln` | https://raw.githubusercontent.com/TheELNConsortium/TheELNFileFormat/89265e52d654254396c8bb2cbbcb53131bcd29f8/examples/kadi4mat/records-example.eln | Repo MIT (TheELNConsortium); the dataset node declares CC BY 4.0 (Kadi4Mat, KIT) | aed5c374f100b750d5b9181fc5438dc6533a79e7f5154f3b93b649464f547bf9 | 12761 | ELN zip with RO-Crate 1.1 metadata, dataset with author and dates, PropertyValue extras, embedded JSON, Turtle, CSV and TXT, per-node license |

## Machines and lab equipment

| File | Source URL (pinned) | License | SHA-256 | Size (bytes) | What it tests |
|---|---|---|---|---|---|
| `machines-lab/andi_chrom_valid.cdf` | https://raw.githubusercontent.com/devrosch/sciformats/bf3d3ff1fcdea257f560c9a983cc57aa351710e9/lib-rs/tests/andi/resources/andi_chrom_valid.cdf | MIT (devrosch, sciformats) | 17eca606d92a418d0c26d82734383a052ef5e78826a068dd6208eadef7636a9f | 3384 | ANDI chromatography netCDF classic: `aia_template_revision`, sample, method, detector and peak table variables (synthetic) |
| `machines-lab/stepone_std.rdml` | https://raw.githubusercontent.com/kablag/RDML/a898512fd9429d8ff9ab09a105b102ee45e1c554/inst/extdata/stepone_std.rdml | MIT (Blagodatskikh, Roediger, Burdukiewicz, Spiess; R package RDML) | d05f9b83584e809a6b0cd367da86339a72d988531d03685e26d31cb40cf90f1b | 8740 | RDML 1.0 zip from ABI StepOne v2.3: standard curve, 5 standards, NTC, efficiency, wells A1 to H12 |
| `machines-lab/signalexpress_voltage.tdms` | https://zenodo.org/records/3563513/files/19873%20Voltage.tdms | CC-BY-4.0 (Schokker, Athanasiadis, Visser, Veerkamp et al., "Locomotion Data Breed4Food: Educational Files", Zenodo 3563513) | e5618d909ea0c57aff7c8af26a54c388b98511e6e216268f8eb2c4bd52027f17 | 101466 | NI TDMS v4713 from SignalExpress 2015: file properties, group "Voltage", channel with waveform timing and `unit_string` |
| `machines-lab/rust-stdf_lot2.stdf.gz` | https://raw.githubusercontent.com/noonchen/rust-stdf/5160dc3859ea1ed19c94f008a1a6133299715993/demo_stdf/lot2.stdf.gz | MIT (noonchen, rust-stdf) | 37235c192dfd0005d2aefd7508a5e15407aeeb5b0576a6930028a1ee8845837b | 209178 (4,418,001 unpacked) | gzip-wrapped STDF V4, big-endian, Teradyne header; FAR, MIR, SDR, WIR/WRR, 1,569 PIR/PRR, 52,403 PTR, TSR, HBR/SBR, MRR |

## Candidates not downloaded

Pinned and permissive, left out to stay at about four files per area: odxtools `somersault.pdx`
(MIT), ldfparser `lin22.ldf` (MIT), afibex `fibex1.xml` (Apache-2.0), ecCodes `GRIB2.tmpl`,
`GRIB1.tmpl`, `BUFR4.tmpl` at 8d14bc0b (Apache-2.0), libmseed `reference-testdata-steim2.mseed2`
(Apache-2.0), netcdf-c `nc_test4/ref_hdf5_compat1.nc` (BSD-3), Capella IFE sample (EPL-2.0),
SysML v2 `VehicleDefinitions.sysml` (EPL-2.0), MathWorks `traffic.slx` (BSD-3), more ccsds-ndm
examples (MPL-2.0), OpenReadout's corpus manifest (per-file licenses).

Not used because of license or provenance: asam-ev/ASAM-OpenX-Examples (no license), cantools CDD
test file (copied from a forum), railML examples (CC BY-NC-ND), eLabFTW and PASTA `.eln` examples
(CC BY-NC-SA inside an MIT repo), rustylink test `.slx` files (no license), Yamcs XTCE files
(AGPL), FDSN StationXML and miniSEED3 repo examples (no license), kibook S1000D CSDB (no
license), MATPOWER case files (not BSD), FAA CIFP (over 2 MB, owner decision).
