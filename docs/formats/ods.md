# ODS

Rezzer reads OpenDocument spreadsheets the same way as Excel files: values, formulas with the cells they read, annotations and hidden parts.

Status: planned for the first release. Nothing is implemented yet.

File types: `.ods`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values, formulas (as OpenFormula text) and names
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 3.2.0 to find the cells and ranges each formula reads. It parses OpenFormula too
- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for annotations, hidden sheets, rows and columns, validation and styles

## What Rezzer extracts

### Structure

- Each sheet as a value table
- Hidden sheets, rows and columns (marked, not dropped)
- Cell styles, like number formats

### Metadata

- Document properties from the ODF meta part

### Hidden logic

- Every formula with the cells and ranges it reads, also across sheets
- Copied formulas folded into patterns, parameters, results and a dependency graph, the same way as for [XLSX](xlsx.md)
- Named ranges
- Annotations with author and date, at their cell
- Data validation rules

## How it shows up in the output

The same as for [XLSX](xlsx.md): one value table per sheet, then parameters, formulas grouped by pattern, results and a Mermaid dependency graph. OpenFormula references like `[.B4]` are shown in the familiar `B4` form.

## Known limitations

- calamine 0.37.0 returns ODS formulas as raw OpenFormula text (`of:=[$Data.A3]`). Rezzer turns them into readable form
- In ODS a sheet is hidden through its table style. Rezzer reads the style to find hidden sheets
- No public ODS test file with validation rules was found. This part is not proven on a real file
- Rezzer does not calculate. It shows the cached results stored in the file
- Against hostile files Rezzer uses size, cell and time limits. The test files include an ODS with huge repeat counts that made another ODS crate use 5.5 GB of memory

## Tested with

4 public test files from LibreOffice and calamine, 1 of them broken on purpose. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
