# XLS (Excel 97-2003)

Rezzer reads old binary Excel workbooks: values, formulas with the cells they read and the cached results.

Status: 📝 planned. The release is not set yet. Nothing is implemented yet.

File types: `.xls`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values, formulas and cached results. It was the only tested crate that gives formulas for binary Excel files
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 4.1.0 to find the cells and ranges each formula reads
- [cfb](https://crates.io/crates/cfb) for document properties
- Rezzer's own checks for names and 3D references (see below)

## What Rezzer extracts

### Structure

- Each sheet as a value table

### Metadata

- Document properties, also custom properties

### Hidden logic

- Every formula with the cells and ranges it reads, also across sheets
- Copied formulas folded into patterns, parameters, results and a dependency graph, the same way as for [XLSX](xlsx.md)
- Defined names (see the limitations below)

## How it shows up in the output

The same as for [XLSX](xlsx.md): one value table per sheet, then parameters, formulas grouped by pattern, results and a Mermaid dependency graph.

## Known limitations

- calamine 0.37.0 has bugs with XLS that Rezzer must work around:
  - in one test file a 3D range comes back wrong, as `SUM(Sheet1!$BTRM$1:$BTRM$4)`
  - built-in names like the print area come back as control characters
  - sheet names with spaces are not quoted, so the formula text is not valid any more
- Cell notes: calamine 0.37.0 does not expose them. Only one public test file with a note was found. Notes in XLS are not proven yet
- Validation, conditional formats and the other parts Rezzer reads from XLSX are not planned for XLS yet
- Rezzer does not calculate. It shows the cached results stored in the file
- Against hostile files Rezzer uses size, depth, cell and time limits. The test files include an XLS that caused an out-of-memory allocation and a truncated XLS

## Tested with

5 public test files from Apache POI, Apache Tika and calamine, 2 of them broken on purpose. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
