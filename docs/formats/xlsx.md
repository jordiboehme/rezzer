# XLSX, XLSM and XLSB

Rezzer turns each sheet into a table of values and shows the calculation behind it: every formula with the cells it reads, inputs, results and the dependencies between them.

Status: available in the development version (no release yet).

File types: `.xlsx`, `.xlsm`, `.xlsb`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values, cached results, formula texts, merged cells and VBA module names
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 4.0.0, behind Rezzer's own formulakit crate, to find the cells, ranges, names and tables each formula reads
- Rezzer's own readers on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for the parts no crate exposes well (XLSX and XLSM): sheetparts for the workbook parts, zipguard to check the package before anything is unpacked

## What Rezzer extracts

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

Example from a tunnel design workbook in the test files (shortened):

```markdown
### Ventilation

#### Parameters

| Cell | Name | Label | Value | Unit | Unit from | Read by |
| --- | --- | --- | --- | --- | --- | --- |
| B5 |  | Tunnel length, L | 800 | m | cell beside | 2 |
| B6 |  | Tunnel diameter, D | 10 | m | cell beside | 2 |

#### Formulas

| Cells | Count | Formula | Reads | Role | Value |
| --- | --- | --- | --- | --- | --- |
| B20 | 1 | `=PI()*B6^2/4` | B6 | intermediate | 78.53981634 |
| B34 | 1 | `=IF(AND(B33>=1,B33<=10),"OK","CHECK RANGE")` | B33 | result | OK |
```

## Known limitations

- Rezzer does not calculate. It shows the cached results stored in the file. When a value is missing, the output says "not calculated in file"
- XLSB is a binary format, so Rezzer's XML readers don't apply to it. For XLSB Rezzer gets values, cached results, formulas, defined names, document properties and VBA module names. It does not read validation, conditional formats, notes and threads, names with scope, calculation settings, data tables, number formats beyond dates, merged cells, hidden rows and columns, Excel tables, pivot tables, charts and external links. The output lists this under "Not read"
- User functions from VBA or add-ins can't be evaluated. The output names them and says where they come from
- Copies of a formula fold only when they have the same form relative to their cell. A copied formula that reads a fixed cell without `$` gets one row per cell
- A label is taken from the text left of a value or from the column header. In data tables the text on the left is sometimes a value of the row, not a label
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

## Tested with

24 XLSX, 3 XLSM and 2 XLSB public test files (one of the XLSB files is broken on purpose) from Apache POI, Apache Tika, LibreOffice, calamine, IronCalc and two open engineering projects. See [MANIFEST.md](../../tests/corpus/MANIFEST.md).
