# ODS

Rezzer reads OpenDocument spreadsheets the same way as Excel files: the tables inside each sheet, formulas with the cells they read, annotations and hidden parts.

Status: ✅ available in the development version. It is part of 0.1.0, the first release.

File types: `.ods`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values and formulas (as OpenFormula text)
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 4.1.0, behind Rezzer's own formulakit crate, to find the cells and ranges each formula reads. formulakit turns OpenFormula into the familiar A1 form first
- Rezzer's own reader sheetparts on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for named ranges, annotations, hidden sheets, rows and columns, validation, number formats, merged cells, database ranges, data pilots, cell styles and document properties
- Rezzer's own sheetlayout crate to find the tables, titles and notes inside a sheet

## What Rezzer reads

### Structure

- The tables inside each sheet with their header rows, titles and notes, the same way as for [XLSX](xlsx.md#tables-inside-a-sheet)
- Label and value blocks and text lists of report sheets
- Merged cells
- Database ranges with their header and footer rows, also the unnamed range a sheet keeps for its filter
- Data pilot (pivot table) output ranges
- Hidden sheets, rows and columns (marked, not dropped)
- Number formats from the cell styles

### Metadata

- Document properties from the ODF meta part, also user-defined ones

### Hidden logic

- Every formula with the cells and ranges it reads, also across sheets
- Copied formulas folded into patterns, parameters with labels and units, results and a dependency graph, the same way as for [XLSX](xlsx.md). A parameter in a table is named by its path: title, column header and row label. In the Logic section the shared part is written once as a title over the group
- Named ranges and named expressions, with their scope
- Annotations with author and date, at their cell
- Data validation rules

## How it shows up in the output

The same as for [XLSX](xlsx.md): the tables and blocks of each sheet, each with a layout line that says where it is and how it was found, then the Logic section with parameters, formulas grouped by pattern, results, names and a Mermaid dependency graph. OpenFormula references like `[$Data.A3]` are shown in the familiar `Data!A3` form. The text as stored in the file is kept next to it.

## Tables inside a sheet

Rezzer finds tables in an ODS sheet the same way as in an Excel sheet: see [Tables inside a sheet](xlsx.md#tables-inside-a-sheet). The facts it takes from an ODS file are database ranges (exact range, header and footer rows), data pilot output ranges, print header rows, frozen panes and row groups. Bold and filled cells come from the cell styles. `--table-style records` and `--sheet` work the same way. Two sheets whose names differ only in case are told apart by their exact name first.

## Example

Real output of `rezzer read` for `formula-across-sheets.ods` from the test corpus. The part for the Calculation sheet, cut to four of its eight formula rows:

```markdown
### Calculation

#### Parameters

| Cell | Name | Label | Value | Unit | Unit from | Read by |
| --- | --- | --- | --- | --- | --- | --- |
| G1 |  | Colonne b | 1.4965 |  |  | 3 |
| G2 |  | haut (n) | -0.1459 |  |  | 3 |

#### Formulas

A row with a count above 1 stands for one formula copied over its cells. It shows the first cell.

| Cells | Count | Formula | Reads | Role | Value | Math |
| --- | --- | --- | --- | --- | --- | --- |
| B4, C4, E4 | 3 | `=Data!A3` | Data!A3 | intermediate | 1.4666 |  |
| B5, C5, E5 | 3 | `=(B4-$G$1)/$G$2` | B4, G1, G2 | intermediate | 20.493488691 % | $`\frac{\mathrm{B4}-\mathrm{G1}}{\mathrm{G2}}`$ |
| B6:E6 | 4 | `=B5/(1-B5)` | B5 | intermediate, result | 25.775862069 % | $`\frac{\mathrm{B5}}{1-\mathrm{B5}}`$ |
| B9, D9 | 2 | `=L_nb/(1+Xnb)` | name:L\_nb, name:Xnb | intermediate, result | 9.602467443 | $`\frac{\mathrm{L\_nb}}{1+\mathrm{Xnb}}`$ |
```

## Known limitations

- Rezzer does not calculate. It shows the cached results stored in the file
- Not read yet: conditional formats, charts and calculation settings. The output lists them under "Not read"
- A data pilot is read by its output range only: Rezzer finds its table there, but not its header rows, label columns or grand totals
- The limits of table finding are the same as for [XLSX](xlsx.md#limits)
- ODF has no "very hidden" state. A sheet is hidden through its table style. Rezzer reads the style
- No public ODS test file with validation rules was found. This part is not proven on a real file
- Encrypted ODS files are reported as encrypted and not read

## Damaged and hostile files

- calamine keeps every sheet as a dense grid in memory. Before calamine runs, Rezzer works out how much memory that grid would take. A file that would pass the memory limit (2 GB by default) is not read. The output says why. The test files include an ODS with huge repeat counts that asks for about 3.9 TB this way: Rezzer refuses it in a fraction of a second
- `content.xml` must be markup that calamine and Rezzer read the same way: well formed UTF-8, every namespace declared on the root, no DOCTYPE. Anything else is refused as damaged
- The same limits as for [XLSX](xlsx.md) apply: size, unpacked size, packing ratio, XML depth, time and elements

## How it is tested

The corpus has 13 public ODS test files from LibreOffice and calamine. One of them is broken on purpose. See [MANIFEST.md](../../tests/corpus/MANIFEST.md).

Extreme cases the test suite holds, at the default limits unless a limit is named (the figures are the bounds the tests check):

- Repeat counts that ask for about 3.9 TB of cells: refused before any time check, peak heap under 512 MB
- 100,000 nested spans in one cell: refused for its depth before any parser reads it
- An entity bomb that expands to a billion words: the DOCTYPE is refused, nothing expands

Speed: `formula-across-sheets.ods` (16 KB, 23 formulas) reads in under 0.01 seconds with a peak memory of 4 MiB. Median of 3 runs with `--no-cache`, on an Apple M5 (Mac17,3, 32 GB, load average 4.7), release build (cargo profile release) of Rezzer 0.1.0 at commit be1efc2, 2026-10-26.
