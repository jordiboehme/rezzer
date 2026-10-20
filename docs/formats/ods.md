# ODS

Rezzer reads OpenDocument spreadsheets the same way as Excel files: values, formulas with the cells they read, annotations and hidden parts.

Status: available in the development version (no release yet).

File types: `.ods`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values and formulas (as OpenFormula text)
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 4.0.0, behind Rezzer's own formulakit crate, to find the cells and ranges each formula reads. formulakit turns OpenFormula into the familiar A1 form first
- Rezzer's own reader sheetparts on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for named ranges, annotations, hidden sheets, rows and columns, validation, number formats and document properties

## What Rezzer extracts

### Structure

- Each sheet as a value table
- Hidden sheets, rows and columns (marked, not dropped)
- Number formats from the cell styles

### Metadata

- Document properties from the ODF meta part, also user-defined ones

### Hidden logic

- Every formula with the cells and ranges it reads, also across sheets
- Copied formulas folded into patterns, parameters with labels and units, results and a dependency graph, the same way as for [XLSX](xlsx.md)
- Named ranges and named expressions, with their scope
- Annotations with author and date, at their cell
- Data validation rules

## How it shows up in the output

The same as for [XLSX](xlsx.md): one value table per sheet, then the Logic section with parameters, formulas grouped by pattern, results, names and a Mermaid dependency graph. OpenFormula references like `[$Data.A3]` are shown in the familiar `Data!A3` form. The text as stored in the file is kept next to it.

## Known limitations

- Rezzer does not calculate. It shows the cached results stored in the file
- Not read yet: conditional formats, merged cells, database ranges, pivot tables (data pilot), charts and calculation settings. The output lists them under "Not read"
- ODF has no "very hidden" state. A sheet is hidden through its table style. Rezzer reads the style
- No public ODS test file with validation rules was found. This part is not proven on a real file
- Encrypted ODS files are reported as encrypted and not read

## Damaged and hostile files

- calamine keeps every sheet as a dense grid in memory. Before calamine runs, Rezzer works out how much memory that grid would take. A file that would pass the memory limit (2 GB by default) is not read. The output says why. The test files include an ODS with huge repeat counts that asks for about 3.9 TB this way: Rezzer refuses it in a fraction of a second
- `content.xml` must be markup that calamine and Rezzer read the same way: well formed UTF-8, every namespace declared on the root, no DOCTYPE. Anything else is refused as damaged
- The same limits as for [XLSX](xlsx.md) apply: size, unpacked size, packing ratio, XML depth, time and elements

## Tested with

4 public test files from LibreOffice and calamine, 1 of them broken on purpose. See [MANIFEST.md](../../tests/corpus/MANIFEST.md).
