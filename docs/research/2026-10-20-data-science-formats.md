# Rezzer research: data science formats (columnar files and scientific arrays)

Date: 2026-10-20. Scope (set by Jordi during the research): 1 columnar and tabular files
(Parquet, Arrow IPC and Feather, Avro, ORC, JSON Lines, compressed CSV) and 2 scientific arrays
(HDF5, NetCDF, Zarr, NumPy `.npy`/`.npz`). Notebooks, statistics packages, ML model files and
pickle get a short "later" section. Folder table formats (Delta Lake, Iceberg, Hudi) and
databases as files (SQLite, DuckDB) are out of scope.

The industry report ([`2026-10-19-industry-legacy-formats.md`](2026-10-19-industry-legacy-formats.md),
section 7.1) already covers HDF5, NetCDF, FITS, Zarr and MATLAB `.mat` for science. This report
does not repeat that. It goes deeper only where data science needs more: pandas and AnnData
conventions in HDF5, pickled objects inside arrays and the comparison with Python.

Method:

- Crate versions, dates, licenses, MSRV and downloads come from the crates.io API today
- Transitive weight comes from `cargo tree -e normal` in throwaway projects. Binary size comes
  from a small release binary per crate that reads one test file. Build settings: cargo 1.97.0,
  macOS arm64, `strip = true`, thin LTO, one codegen unit. The scratch projects do not see
  Rezzer's `rust-toolchain.toml` (1.99.0). Sizes are deltas over a 339 KB baseline binary. The
  deltas overlap: serde, zip and flate2 are already in Rezzer
- 47 small public test files (252 KB) were downloaded (each into its own empty folder) and read
  by these Rust probes. One Python step ran with `-I`: `pickletools.genops` lists pickle opcodes
  and never unpickles. Nothing from a file was executed
- PyPI downloads (pypistats.org, last month) are used as a rough proxy for how often an agent
  meets a format. CI inflates them. Read them as an order of magnitude
- Anything not checked today is marked **[unverified]**

Legend: effort **S** under about 300 lines, **M** 300 to 1000, **L** over 1000. **R1** first data
science release, **R2** second, **later**, **detect** (name the format and say why it is not read).

## 1. Summary

| Format | How common (proxy) | What Rezzer can prove without reading the data | Crate or own reader | Weight (delta) | C code | Effort | Rec. |
|---|---|---|---|---|---|---|---|
| Parquet | very high: pyarrow 335 M, polars 51 M, fastparquet 9.9 M downloads a month | schema, exact row count, row groups, per-chunk min/max/null count, codecs, encodings, writer, key-value metadata (pandas, Spark, Arrow, Avro, GeoParquet) | `parquet` 60.0.0 without default features | +255 KB footer; +2.05 MB with row sample (no zstd); +5.05 MB with arrow and all codecs | none for footer; zstd is C | M | R1 |
| CSV / TSV compressed (.gz .bz2 .xz .zst .zip) | very high (pandas 547 M) | everything Rezzer already gets from CSV, plus codec and ratio | flate2 and zip (in Rezzer), bzip2 0.6.1, lzma-rust2 0.23.0, ruzstd 0.9.1 | small [sizes not measured] | none | S | R1 |
| JSON Lines / NDJSON | very high (logs, LLM datasets, HF exports) | record count, key set per record, inferred types, nulls, nesting | own on serde_json (in Rezzer) | 0 | none | S | R1 |
| Arrow IPC file, Feather v2 | high (pyarrow, polars `.arrow`, `.feather`) | schema with nullability, field and schema metadata, extension types, dictionaries, batch count, body codec | `arrow-ipc` 60.0.0 | +1.74 MB alone; shared with Parquet arrow path | none for schema; zstd body is C | S | R1 |
| NumPy .npy / .npz | very high (numpy 802 M) | dtype, shape, order, byte size, pickled object arrays (by opcode scan) | own header parser (S) or `npyz` 0.9.1 | +302 KB with npyz | none | S | R1 |
| HDF5 (.h5, .hdf5, .h5ad, pandas HDFStore) | high (h5py 26 M, tables 1.6 M, anndata 1.3 M) | group tree, datasets with type, shape, chunks, filters, attributes, pandas and AnnData conventions | `hdf5-reader` 0.9.2 | +308 KB | none | S-M | R1 |
| NetCDF classic and 4 | medium-high (xarray 10 M, netCDF4 3.1 M, h5netcdf 2.8 M) | dimensions, variables, CF attributes, history | `netcdf-reader` 0.9.2 | +494 KB | none | S | R1 |
| Avro object container | medium-high (fastavro 30 M, avro 12 M; Kafka and data lakes) | schema JSON, codec, exact record count from block headers, sync markers | own header reader (S); `apache-avro` 0.22.0 for rows | +624 KB without codecs | zstandard and xz features are C | S, M with rows | R1 header, R2 rows |
| Zarr v2 / v3 | medium (zarr 4.9 M) | arrays, groups, shapes, chunks, codecs, attributes, xarray dimension names | own serde or `zarrs_metadata` 0.7.5 | +187 KB | none | S | R2 (single-file forms in R1) |
| ORC | low outside Hive and Spark (pyorc 0.17 M) | schema, row count, stripes, column statistics, writer version | `orc-rust` 0.9.0 | +7.86 MB, 101 crates | none | M | R2 or detect |
| Feather v1, Lance, Vortex | low | format name and version | none suitable | - | - | S | detect |

For comparison, the engines: `polars` 0.55.2 (parquet, ipc, lazy) is 260 crates and +14.5 MB;
`datafusion` 55.2.0 is 241 crates and +65.4 MB. Neither is needed to read what an agent needs.

## 2. Rezzer versus "write a Python script"

Without Rezzer an agent that meets one of these files has to run Python. That means a working
environment with the right wheels (pyarrow alone is about 40 MB [unverified]), code that the
agent writes and runs, often loading all data into memory and the trap that some readers run
code from the file. With Rezzer the agent calls one tool and gets bounded Markdown that also says
what could not be read.

| Format | What the agent would script | Cost and risk of the script | What Rezzer returns in one call |
|---|---|---|---|
| Parquet | `pq.ParquetFile(p).metadata`, `.schema_arrow`, `.row_group(i).column(j).statistics`, then `pd.read_parquet(p).head()` | needs pyarrow; `read_parquet` loads all columns; pyarrow 0.14 to 14.0.0 ran pickled `PyExtensionType` metadata (CVE-2023-47248, CVSS 9.8) | schema as a table, exact rows, stats per column folded over row groups, writer, pandas/Spark/GeoParquet metadata decoded, first rows, "not proven" notes for missing or inexact stats |
| Arrow IPC / Feather | `pa.ipc.open_file(p).schema`, `pd.read_feather(p)` | same CVE path; `read_feather` loads all | schema, metadata, extension types named, pickled Python types flagged and not decoded |
| Avro | `fastavro.reader(f).writer_schema`, then iterate to count | must decode every record to count rows | schema, codec, exact record count from block headers, first records |
| ORC | `pyarrow.orc.ORCFile(p).schema`, `.nrows`, `.nstripes` | needs pyarrow with ORC | schema, rows, stripes, column stats |
| JSON Lines | `pd.read_json(p, lines=True)` | loads all; mixed records become object columns | key set, types and nulls per key, nested shapes, broken lines with line numbers |
| Compressed CSV | `pd.read_csv(p, compression="infer")` | infers by extension; a `.csv` that is really TSV (see the pandas sample) goes wrong silently | codec found by magic bytes, ratio, then the normal CSV analysis |
| NumPy | `np.load(p)`; `allow_pickle=True` for object arrays | `allow_pickle=True` runs code from the file | dtype, shape, order, size; object arrays listed with the imports their pickle names, never loaded |
| HDF5 | `h5py.File(p).visititems(...)`, `pd.HDFStore(p).info()`, `anndata.read_h5ad(p)` | pandas HDFStore and PyTables unpickle object columns [unverified on a file]; `read_h5ad` loads the matrix | tree with types, shapes, chunks, filters and attributes; pandas frame layout and AnnData layout named |
| NetCDF | `xr.open_dataset(p)` or `ncdump -h` | needs netCDF4 or h5netcdf; `ncdump` is an external program | the `ncdump -h` view plus CF meaning (units, coordinates, packing, calendars) |
| Zarr | `zarr.open_consolidated(p).tree()`, `xr.open_zarr(p)` | a folder or a URL; v2 object arrays may use a pickle codec | tree, shapes, chunks, codecs, attributes; pickle codec flagged |

Python can still do things Rezzer will not: run arbitrary statistics over all values, join files
and plot. Rezzer reports what the file says and marks what it computed itself (for example from a
sample) as computed, not as stored.

## 3. Columnar and tabular files

### 3.1 Parquet (.parquet, .pq, also inside datasets folders)

**Detect.** `PAR1` at start and end; encrypted footer: `PARE` at the end. The footer is a Thrift
`FileMetaData` block before the last 8 bytes and is never compressed.

**What the footer proves** (seen today in the corpus, through `ParquetMetaDataReader`):

- Physical and logical schema with repetition (REQUIRED, OPTIONAL, REPEATED = nullability and
  lists), logical types (STRING, DECIMAL, TIMESTAMP with unit and UTC flag, TIME, UUID, JSON,
  BSON, FLOAT16, GEOMETRY, GEOGRAPHY, VARIANT). `crs-default.parquet` shows a GEOMETRY column
- Exact row count (`num_rows`, required by the spec) and row groups with rows and bytes
- Per column chunk: codec, encodings, sizes and statistics (min, max, null count, rarely distinct
  count) with the flags `min_value_exact`, `max_value_exact` and the deprecated old fields
- `created_by`: the writer and version, for example `impala version 1.3.0-INTERNAL` or
  `parquet-mr version 1.13.0-SNAPSHOT`. This is provenance
- Key-value metadata, which holds the knowledge:
  - `pandas`: JSON with index columns, column names, pandas dtypes, categorical levels
    [shape from pandas docs, no pandas-written file in the corpus]
  - `ARROW:schema`: base64 Arrow IPC schema (the true Arrow types, time zones, dictionary flags)
  - `org.apache.spark.sql.parquet.row.metadata`: Spark schema JSON (seen in
    `nested_lists.snappy.parquet`)
  - `parquet.avro.schema` with field docs (seen: `"doc":"Type inferred from 'Hello'"`) and
    `writer.model.name`
  - `geo`: GeoParquet JSON with version, primary column, encoding, CRS and bbox [unverified on a file]
  - `huggingface`: datasets features JSON [unverified]
- Optional page index (column index with per-page min/max, offset index) and bloom filters.
  These let Rezzer say "sorted by" or "clustered" only when the page stats prove it

**Statistics need care.** `nan_in_stats.parquet` stores max = NaN. Stats can be missing (the
`column_chunk_key_value_metadata.parquet` and the GEOMETRY column have none), inexact
(truncated strings) or from old writers with a broken sort order. Rezzer writes "the writer
recorded min 1.0, max NaN" and gives a file-wide range only when every row group has exact stats.

**What a row sample adds.** Real values, the true format of strings (dates as text, codes), JSON
inside strings, and what nested lists look like. It does not prove anything about the whole file.
Say "first N rows of row group 0", not "sample". A random sample costs one read per row group.

**Crate.** `parquet` 60.0.0 (Apache-2.0, 2026-09-15, MSRV 1.88, 85 M downloads, apache/arrow-rs):

| Build | Crates | Size delta | Reads |
|---|---|---|---|
| no default features | 25 | +255 KB | footer, schema, stats, key-value metadata, page index |
| `snap,lz4,flate2-zlib-rs,brotli` (no arrow, no zstd) | 33 | +2.05 MB | plus rows through `SerializedFileReader::get_row_iter` (tested: 10 rows of `alltypes_plain`, nested lists, NaN) |
| `arrow` plus all codecs (about the default) | 47 | +5.05 MB | plus Arrow record batches, `ARROW:schema` decoding |
| `encryption` | 28 | +322 KB | pulls `ring` 0.17.14 (C and assembly); not needed, see below |

Findings from the corpus:

- `PARQUET-1481.parquet` (bad data) fails with "Unexpected Type -7": a clean error, no panic
- `encrypt_columns_and_footer` (PARE) fails as expected. Rezzer says "encrypted footer, no key"
- `encrypt_columns_plaintext_footer` failed with `PageIndexPolicy::Optional` ("Expected list
  element type of Binary but got Byte", and "Cannot decrypt column index" with `encryption`
  on). With `PageIndexPolicy::Skip` the footer reads fine without the `encryption` feature:
  50 rows and the full schema. So Rezzer reads the footer with Skip first and loads the page index
  in a second, optional step
- The `zstd` feature maps to `zstd` 0.14.3 and `zstd-sys` (bundled C, BSD-3-Clause). Rezzer
  links no C on macOS and Linux today (`cc` in Cargo.lock is only for the Haiku target). zstd
  matters: polars writes zstd by default (docs.pola.rs `write_parquet`, `compression='zstd'`).
  The parquet crate has no option for the pure-Rust `ruzstd`. Without zstd Rezzer still reads
  the full footer of a zstd file and says "rows not shown: zstd codec not built in"
- Variant (`variant_experimental`) and `geospatial` are experimental features. Rezzer names the
  logical type and does not decode it in R1

Effort M: footer to model mapping, stats folding with the exact flags, key-value decoders for
pandas, Arrow, Spark, Avro and GeoParquet, optional sample. **R1.**

### 3.2 Arrow IPC file and stream, Feather v2 (.arrow, .feather, .arrows, .ipc)

**Detect.** File: `ARROW1` plus 2 padding bytes at start, `ARROW1` at end. Stream: starts with
continuation `FF FF FF FF` and a Schema message (seen in `generated_custom_metadata.stream`).
Feather v2 is the IPC file. Feather v1 starts with `FEA1` and arrow-rs has no reader
[unverified]: detect only.

**Knowledge.** Full Arrow schema with nullability, field metadata and schema metadata (seen:
`"pandas"` key, many custom keys), extension types (`arrow.uuid`, unregistered ones), dictionary
fields with index type, batch count, body compression (LZ4 frame or zstd). The IPC file footer
lists all batches, so counting rows needs one small read per batch header, not the data.

**Safety.** pyarrow before 14.0.1 rebuilt legacy `PyExtensionType` types by unpickling their
extension metadata (CVE-2023-47248; the extension name is `arrow.py_extension_type` [unverified]).
Rezzer shows such a field as "Python pickled extension type, not decoded" with the imports from an
opcode scan (section 5).

**Crate.** `arrow-ipc` 60.0.0 (Apache-2.0, MSRV 1.88): 32 crates, +1.74 MB alone, read the schema
of all four corpus files. When Parquet takes the arrow path, IPC comes almost free. Body zstd is
the same C question. Effort S. **R1.**

### 3.3 Avro object container (.avro)

**Detect.** `Obj` plus 0x01. Then a plain Avro map with `avro.schema` (JSON) and `avro.codec`
(null, deflate, snappy, zstandard, bzip2, xz), a 16-byte sync marker and blocks. Each block header
has the object count and byte size, before the compressed data.

**Knowledge.** The schema is the documentation: records with namespaces, field docs, defaults,
unions with null (nullability), enums with symbols, logical types (decimal, date, timestamp-millis,
uuid, duration). The exact record count is the sum of block counts.

**Finding.** `apache-avro` 0.22.0 (Apache-2.0, 2026-08-10, MSRV 1.88, 56 crates, +624 KB) has no
codecs by default and fails on `Reader::new` for the snappy and zstandard files, before it gives
back the schema. Its `zstandard` feature pulls the C zstd and `xz` pulls `liblzma` (C). So:

- own header and block-count reader on the existing Rezzer code: S, no codec needed
- rows later through `apache-avro` with `snappy` only, or own decoding: M

**R1** for header and count, **R2** for rows. `arrow-avro` 60.0.0 exists too (Apache-2.0), only
useful if Rezzer takes the arrow path.

### 3.4 ORC (.orc)

**Detect.** `ORC` at start; the last byte is the PostScript length. Only the PostScript is plain;
the footer and metadata are compressed with the file codec (zlib, snappy, lzo, lz4, zstd), so even
the schema needs decoders.

**Knowledge.** Type tree, row count, stripes, file and stripe column statistics (min, max, sum,
has null), writer version, user metadata, encryption with masked columns. `orc-rust` read all
three corpus files, including stats on 11,000 rows and the encrypted `sample2.orc` schema (ssn,
email, name). What it shows about the masked columns was not checked [unverified].

**Crate.** `orc-rust` 0.9.0 (Apache-2.0, 2026-09-02, MSRV 1.73): 101 crates and +7.86 MB because
it builds on arrow. An own reader needs protobuf (`prost` 0.14.4) and the codecs: M. ORC is rare
outside Hive and older Spark lakes. **R2** with the crate if arrow is in anyway, else **detect**.

### 3.5 JSON Lines, NDJSON (.jsonl, .ndjson, .json)

**Detect.** One JSON value per line; also inside `.json` (arrow-json's test files are NDJSON
with a `.json` name). Tell apart from one JSON document by parsing the first lines.

**Knowledge.** It has no schema, so Rezzer infers one and says so: key set with how many records
have each key, types per key (the corpus has `b` as int and float, `d` as string and object),
nulls, nested shapes, keys that look like ids, timestamps as text. The pandas `items.jsonl` has
the same keys in a different order. Common uses: logs, LLM fine-tuning sets (`messages`,
`prompt`/`completion`), Hugging Face exports.

**Reader.** Own, on `serde_json` 1.0.154 (in Rezzer), reusing the CSV column profiler. Bounded:
line length, depth and record limits. Effort S. **R1.**

### 3.6 Compressed CSV and JSONL (.gz .bz2 .xz .zst .zip .tar.gz)

**Detect.** By magic: gzip `1F 8B`, bzip2 `BZh`, xz `FD 37 7A 58 5A 00`, zstd `28 B5 2F FD`, zip
`PK`. The pandas `salaries.csv.*` files are tab-separated despite `.csv`: the decompressed stream
goes through normal content detection.

**Crates.** All pure Rust:

| Codec | Crate | Version (date) | License |
|---|---|---|---|
| gzip | `flate2` (in Rezzer) | 1.1.10 | MIT OR Apache-2.0 |
| zip | `zip` (in Rezzer) | 9.0.5 | MIT |
| bzip2 | `bzip2` with default `libbz2-rs-sys` | 0.6.1 (2025-10-16) / 0.2.5 | MIT OR Apache-2.0 / `bzip2-1.0.6` |
| xz | `lzma-rust2` or `xz4rust` | 0.23.0 (2026-10-18) / 0.2.3 | Apache-2.0 / MIT |
| zstd | `ruzstd` | 0.9.1 (2026-10-11), MSRV 1.87 | MIT |

`bzip2-1.0.6` is not in Rezzer's `deny.toml` allow list (it is a BSD-style license): an exception
is needed. Limits: decompressed size and ratio (zip bombs), one level of nesting. Effort S. **R1.**

## 4. Scientific arrays

### 4.1 HDF5 for data science (.h5, .hdf5, .h5ad, .keras h5)

The industry report covers HDF5 in general. What data science adds:

- **pandas HDFStore** (PyTables): attributes `pandas_type`, `pandas_version`, `encoding`, `ndim`,
  `nblocks`, `blockN_items`, `blockN_values`, `axis0`, `axis1`. Seen in the pandas 3.0.0
  `fixed.h5`: 15 blocks, one per dtype, item names in `blockN_items`. Rezzer can rebuild the frame
  layout: column names, dtypes, row count, index. Object columns stored as PyTables `ObjectAtom`
  are pickles [unverified on a file]: report and refuse
- **AnnData `.h5ad`** (single-cell biology): `encoding-type` and `encoding-version` on every
  group, `obs`, `var`, `obsm`, `obsp`, `layers`, `uns`, `_index`, `column-order`. Seen in the
  anndata v0.11.4 archive. Rezzer reports cells by genes, annotations and layers
- **Keras HDF5 models**: `model_config` JSON attribute. A `Lambda` layer holds marshalled
  Python bytecode [unverified]: name the layer, never decode
- Plain h5py files: dimension scales, attributes as units

**Crate.** `hdf5-reader` 0.9.2 (MIT OR Apache-2.0, 2026-10-19, MSRV 1.81, pure Rust, no build
script): 33 crates without default features, +308 KB. It read all four corpus files. Notes:

- `OpenOptions` leaves `external_file_resolver` and `external_link_resolver` at `None` by default.
  Rezzer keeps that default and never installs the filesystem resolvers. External links and
  external raw files are listed, not followed
- `ref_szip.h5`: the tree and chunk shape read, but the szip filter is not supported. Output:
  "data compressed with szip, not readable"
- The crate is from 2026 (repo created 2026-03-15). It claims bounds-checked cursors, cycle
  detection and fuzzing. Keep it behind a Rezzer trait

Effort S-M on top of the industry report's HDF5 work. **R1.**

### 4.2 NetCDF

Covered in the industry report. Crate `netcdf-reader` 0.9.2 (same family, `netcdf4` feature
only): 43 crates, +494 KB. Corpus: two classic files from xarray (CF `units` and `long_name`,
unlimited `time`), `ref_tst_dims.nc` with a dimension of 4,500,000,000 read as metadata only
(good hostile-input evidence) and `ref_provenance_v1.nc`. **R1**, shared renderer with HDF5.

### 4.3 Zarr v2 and v3

**Layout.** A folder or a zip. v2: `.zarray`, `.zgroup`, `.zattrs`, consolidated `.zmetadata`;
v3: `zarr.json` per node, optionally with consolidated metadata in the root.

**Knowledge.** Shape, chunks, sharding, data type, fill value, codec chain (seen: `bytes` plus
`gzip` inside `sharding_indexed` with a `crc32c` index; v2 `blosc` with zstd), dimension names
(v3 `dimension_names`, v2 xarray `_ARRAY_DIMENSIONS`), attributes (CF, OME-Zarr `multiscales`
[unverified on a file]).

**Single-file forms** that need no folder source: a consolidated `.zmetadata` or root `zarr.json`
given alone, and `.zarr.zip` read with Rezzer's zip crate. The full folder waits for the planned
set or folder source.

**Safety.** v2 object arrays use an `object_codec`; numcodecs has a `pickle` codec [unverified on
a file]. Rezzer names it and never decodes chunks with it.

**Crate.** `zarrs_metadata` 0.7.5 (MIT OR Apache-2.0, 2026-05-13, MSRV 1.77): 33 crates,
+187 KB, or plain serde structs (S). Chunk data needs `zarrs` 0.23.14 and blosc: not in scope.
**R1** for single-file metadata, **R2** for folders.

### 4.4 NumPy .npy and .npz

**Detect.** `\x93NUMPY`, version bytes, header length, then a Python dict literal:
`{'descr': '<i4', 'fortran_order': False, 'shape': (2, 3), }`. `.npz` is a zip of `.npy` members.

**Knowledge.** dtype with byte order (seen: `>f8` big-endian, Fortran order, shape 2x3x4),
structured dtypes with field names, shape and size. For `.npz`: member names (the variable names
from `np.savez`), each with dtype and shape. Small arrays: values or min, max and mean computed
by Rezzer and marked as computed.

**Safety.** `descr` `|O` means the payload is a pickle (`py3-objarr.npy`). An opcode scan of that
file, without unpickling, found `GLOBAL numpy._core.multiarray _reconstruct`, `numpy ndarray`,
`numpy dtype`, `builtins range` and 2 `REDUCE` calls. Rezzer reports exactly that and loads
nothing.

**Reader.** The header literal must be parsed by a tiny own parser (strings, bools, ints, tuples,
lists, dicts), never evaluated. `npyz` 0.9.1 (MIT, 2026-05-02, 22 crates, +302 KB) parses it and
has `uses_pickled_array()`; it read all three `.npy` files. Own reader S, or npyz. **R1.**

## 5. Code inside files: what to read and what to refuse

| Where | What carries code | Rezzer reads | Rezzer refuses |
|---|---|---|---|
| `.npy` / `.npz` object dtype | pickle | header, shape, opcode scan with imports | values |
| Arrow / Parquet `arrow.py_extension_type` | pickle in extension metadata | field name, storage type | the Python type |
| pandas HDFStore object columns | PyTables pickle [unverified] | layout, other blocks | that block |
| Zarr v2 `pickle` codec | pickle | metadata | chunks |
| Keras h5 `Lambda` | marshalled bytecode [unverified] | layer list | the function |

The opcode scan is a small own reader of the pickle opcode stream (the opcode list is documented
in CPython's `pickletools`). It lists `GLOBAL`, `STACK_GLOBAL`, `INST`, `REDUCE`, `BUILD`,
`NEWOBJ` and the module names they use. It never builds objects. The same scanner serves the
later ML formats.

## 6. Later (short)

- **Notebooks (.ipynb, R Markdown, Quarto):** `nbformat` 3.0.0 (BSD-3-Clause, 40 crates) or own
  serde; cells, execution order, outputs, images to the vision list, HTML and JS outputs as data.
  Already W1 in the industry report. `.Rmd` and `.qmd` are Markdown with YAML and code chunks: S
- **Statistics packages:** Stata `dta` 0.6.0 (MIT, 7 crates), SAS `sas7bdat` 0.9.1 (MIT, 25
  crates), SAS transport `sas_xport` 0.4.1 (MIT, 8 crates), SPSS `ambers` 0.4.6 (MIT, 68 crates),
  R `.rds` `rds2rust` 0.3.0 (MIT, 37 crates). Variable labels, value labels and formats are the
  knowledge. `polars-readstat-rs` 0.24.2 is 268 crates: avoid. R `.RData` can hold closures and
  environments; Rezzer lists them and never evaluates
- **ML model files:** `safetensors` 0.8.0 (Apache-2.0, 26 crates; JSON header with tensor names,
  dtypes and shapes) and GGUF (`gguf-rs-lib` 0.3.2, MIT, 14 crates, or own: key-value metadata
  with architecture, tokenizer and license) are safe and S. ONNX is protobuf (`onnx-ir` 0.22.0,
  99 crates; own prost reader M). PyTorch `.pt` is a zip with `data.pkl`: opcode scan only;
  `pytorch-reader` 0.22.0 (burn) claims to read without Python [unverified how]
- **Pickle and joblib:** opcode scan only, never unpickle. Report imports and refuse the rest

## 7. Test files

47 files, 252 KB, listed with source, license and SHA-256 in
[`2026-10-20-data-science-formats-MANIFEST.md`](2026-10-20-data-science-formats-MANIFEST.md).
Sources (pinned commits): apache/parquet-testing, apache/arrow-testing, apache/orc (Apache-2.0),
apache/arrow-rs (Apache-2.0), pandas-dev/pandas (BSD-3-Clause), pydata/xarray (Apache-2.0),
Unidata/netcdf-c (BSD-3-Clause), scverse/anndata (BSD-3-Clause), zarrs/zarrs (MIT OR
Apache-2.0), jturner314/ndarray-npy (MIT OR Apache-2.0), numpy/numpy (BSD-3-Clause). Gaps: no
pandas-written Parquet with the `pandas` key, no GeoParquet `geo` key, no Feather v1, no Zarr zip,
no HDFStore with pickled objects, no Keras h5. These can be made locally from public data later.

## 8. Recommendation

**R1 (first data science release):**

1. Parquet footer reader on `parquet` 60.0.0 without default features (M), with key-value
   decoders and folded statistics
2. Compressed CSV and JSONL by magic bytes with pure-Rust codecs (S), JSON Lines profiler (S)
3. NumPy `.npy`/`.npz` own header parser plus the pickle opcode scanner (S)
4. HDF5 and NetCDF on `hdf5-reader`/`netcdf-reader` 0.9.2 with pandas, AnnData and CF
   conventions (S-M)
5. Arrow IPC schema (S) and Avro header with exact count (S)
6. Zarr single-file metadata (S)

**R2:** Parquet and Avro sample rows, Zarr folders (after the folder source), ORC.
**Detect only:** Feather v1, Lance, Vortex, encrypted Parquet footers, szip data.

Rough total for R1: 2 M and 6 S, about 3,000 to 4,000 lines. Binary growth: about 1.1 MB for
Parquet footer, HDF5 and NetCDF, plus 1.74 MB for `arrow-ipc` (about 2.8 MB in all; a sum of
separate deltas with overlaps not removed). If Arrow IPC waits for the arrow decision, R1 adds
about 1.1 MB.

## 9. Open decisions for Jordi

1. **Sample rows for Parquet:** none in R1, the row API without arrow (+2.05 MB, no zstd) or the
   arrow path (+5.05 MB, also gives Arrow IPC and ORC later)
2. **zstd in C:** allow `zstd-sys` (bundled C, BSD-3-Clause, first C code in Rezzer on macOS and
   Linux) so zstd Parquet, Arrow and Avro rows are readable, or stay pure Rust and report "zstd
   rows not shown". Polars writes zstd by default
3. **License exception** for `libbz2-rs-sys` (`bzip2-1.0.6`) in `deny.toml`, or skip bzip2
4. **ORC:** crate with arrow in R2, own reader or detect only
5. **Sample size and kind:** first N rows or one row per row group; N for the output limit
6. **Computed statistics:** may Rezzer compute min, max and nulls when the file has none, on how
   many rows, and how is that marked in the output
7. **Zarr folders:** wait for the folder source, or accept a folder path for Zarr only
