# XLSX, XLSM and XLSB

Rezzer turns each sheet into a table of values and shows the calculation behind it: every formula with the cells it reads, inputs, results and the dependencies between them.

Status: available in the development version (no release yet).

File types: `.xlsx`, `.xlsm`, `.xlsb`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values, cached results, formula texts, merged cells and VBA module names
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 4.0.0, behind Rezzer's own formulakit crate, to find the cells, ranges, names and tables each formula reads
- Rezzer's own readers on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for the parts no crate exposes well (XLSX and XLSM): sheetparts for the workbook parts, zipguard to check the package before anything is unpacked

## What Rezzer reads

### Structure

- Each sheet as a value table, with merged cells
- Excel tables with their range and column names
- Hidden and very hidden sheets, hidden rows and columns (marked, not dropped)
- Number formats, so 0.09 shows as 9 %

### Metadata

- Core, app and custom document properties, with their type
- Calculation settings

### Hidden logic

- Every formula with the cells, ranges, names and tables it reads, also across sheets and with 3D references like `Sheet1:Sheet3!A1`
- Copied formulas folded into one pattern. In one engineering workbook from the test files, 2,133 formulas fold into 10 rows: 8 patterns and 2 single formulas
- Parameters: constant input cells that formulas read, with label, value and unit when one can be found
- Results: formula cells that no other formula reads, with the cached value
- A dependency graph across sheets and named ranges
- Defined names with their scope, also names used as variables
- Modern formulas: LET, LAMBDA (also as defined names, with their parameters), XLOOKUP and dynamic arrays with their spill range
- Array formulas with their range
- What-if data tables with their input cells
- Calculation settings that mean the stored results can be old: manual calculation, iterative calculation, recalculation on load
- Data validation rules and lists
- Conditional formats
- Legacy notes and threaded comments with person, date, replies and resolved state
- Pivot tables with their location and source range
- Charts with their type, title and series ranges
- Links to other workbooks and add-ins, with their path
- VBA: module names and the user functions used in formulas. VBA is never run
- Volatile functions like `RAND` or `OFFSET`

### Labels

A parameter takes its label from the text left of it or from the column header above it. A text on the left that repeats in its column, like a category `blue` in a data table, is a value of the row and not a label. The same is true for any text in a column with at least four texts where at least half of them repeat. Then the column header is the label. A label with a unit in brackets, like `Load [kN]`, always stays the label, with its unit. A bare unit like `kg` or a size like `DN 50 mm` can still be a value of the row. In a list like `Diameter | 10 | m` the name on the left stays the label.

### Units

A parameter gets a unit only when the file says so. The output names the source of each unit:

- the label, like `Load [kN]`, `Excavated diameter (m)` or `Span in mm`
- the cell beside the value, like `10 | m`
- the number format, like `0.0" m"`
- a `CONVERT` call that converts the value from a unit written in the formula
- the column header above a run of values, like `t (m)`

Only units from a fixed list count. One-letter units in round brackets count only for m, g, s, l and L, because `(V)` or `(N)` is more often a variable name. Codes like `N/A` are no units. When two sources disagree, the parameter gets no unit and a note says why.

## How it shows up in the output

Each sheet becomes a value table. Then a Logic section follows with one part per sheet: parameters, formulas grouped by pattern, results, rules. After that come the defined names, a Mermaid dependency graph and the hidden logic (user functions, add-ins, other workbooks, data tables, spill ranges, VBA modules, calculation settings). Formulas can also be shown as LaTeX math (`--no-math` turns that off). In the graph a copied formula is one box, not one box per cell.

## Example

Real output of `rezzer read` for `tunnel_design_calculations.xlsx` from the test corpus. The part for the Ventilation sheet, cut to two of its 13 parameters and three of its 14 formula rows:

```markdown
### Ventilation

#### Parameters

| Cell | Name | Label | Value | Unit | Unit from | Read by |
| --- | --- | --- | --- | --- | --- | --- |
| B5 |  | Tunnel length, L | 800 | m | cell beside | 2 |
| B6 |  | Tunnel diameter, D | 10 | m | cell beside | 2 |

#### Formulas

| Cells | Count | Formula | Reads | Role | Value | Math |
| --- | --- | --- | --- | --- | --- | --- |
| B20 | 1 | `=PI()*B6^2/4` | B6 | intermediate | 78.53981634 | $`\frac{\pi \cdot \mathrm{B6}^{2}}{4}`$ |
| B33 | 1 | `=MAX(B29,B32)` | B29, B32 | intermediate | 2.184635247 | $`\operatorname{MAX}(\mathrm{B29}, \mathrm{B32})`$ |
| B34 | 1 | `=IF(AND(B33>=1,B33<=10),"OK","CHECK RANGE")` | B33 | result | OK | $`\operatorname{IF}(\operatorname{AND}(\mathrm{B33} \geq 1, \mathrm{B33} \leq 10), \text{"OK"}, \text{"CHECK RANGE"})`$ |
```

## Known limitations

- Rezzer does not calculate. It shows the cached results stored in the file. When a value is missing, the output says "not calculated in file"
- XLSB is a binary format, so Rezzer's XML readers don't apply to it. For XLSB Rezzer gets values, cached results, formulas, defined names, document properties and VBA module names. It does not read validation, conditional formats, notes and threads, names with scope, calculation settings, data tables, number formats beyond dates, merged cells, hidden rows and columns, Excel tables, pivot tables, charts and external links. The output lists this under "Not read"
- User functions from VBA or add-ins can't be evaluated. The output names them and says where they come from
- Copies of a formula fold only when they have the same form relative to their cell. A copied formula that reads a fixed cell without `$` gets one row per cell
- A formula that fails to parse is shown as text, with a note
- Encrypted workbooks are reported as encrypted and not read
- No public test file was found with a very hidden sheet, hidden columns, iterative calculation turned on or a custom percent format. Rezzer reads these, but they are not proven on a real file

## Damaged and hostile files

Rezzer reads every file as untrusted data:

- Each entry is measured before it is unpacked. Defaults: 2 GB per file, 4 GB unpacked, at most 100 times the packed size per entry, XML nesting up to 256 levels, 120 seconds, 2 GB of memory and 2 million elements. The unpacked limit is checked against what the file really unpacks to. Each part counts once, even when Rezzer reads it more than once
- A value table shows at most 1,000 columns and 200,000 cells. The output says what was cut. Its formulas still show up in the Logic section
- A formula longer than 8,192 characters (the Excel maximum) is kept as text and not analyzed
- A DOCTYPE in a part is refused, so entity bombs don't expand
- A package that can be read in two ways is refused as damaged: two main workbook parts, two relationships with one id, two sheets with one name, part paths with `.` or `..`, entries that differ only in case, relationship or sheet markup Excel never writes, control characters in sheet names
- A sheet that breaks halfway keeps the cells before the break. The output says where it stopped

## How it is tested

The corpus has 24 XLSX, 3 XLSM and 2 XLSB public test files from Apache POI, Apache Tika, LibreOffice, calamine, IronCalc and two open engineering projects. One XLSB file is broken on purpose. See [MANIFEST.md](../../tests/corpus/MANIFEST.md).

Extreme cases that were held, at the default limits:

- A sheet of 64 MB of spaces, packed far over 100 times: refused before it is unpacked, peak heap under 512 MB
- Two cells, A1 and XFD1048576, 17 billion cells apart: peak heap under 64 MB, the far formula is still listed
- 100,000 nested XML elements after the cells: the read stops at 256 levels and keeps the cells before
- An entity bomb that expands to a billion words: the DOCTYPE is refused, nothing expands

Speed: `ex012.PVT_z_factor.xlsm` (130 KB, 2,133 formulas) reads in 0.07 seconds with a peak memory of 13 MiB. Measured on an Apple M5 (Mac17,3, 32 GB) with the release build of the development version on 2026-10-21.
