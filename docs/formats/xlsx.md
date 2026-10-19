# XLSX, XLSM and XLSB

Rezzer turns each sheet into a table of values and shows the calculation behind it: every formula with the cells it reads, inputs, results and the dependencies between them.

Status: planned for the first release. Nothing is implemented yet.

File types: `.xlsx`, `.xlsm`, `.xlsb`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values, cached results, formulas, Excel tables, merged cells, sheet visibility and VBA module names
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 3.2.0 to find the cells, ranges, names and tables each formula reads
- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for the parts no crate exposes well (XLSX and XLSM only)

## What Rezzer extracts

### Structure

- Each sheet as a value table, with merged cells
- Excel tables with their header names
- Hidden and very hidden sheets, hidden rows (marked, not dropped)
- Number formats, so 0.09 shows as 9 %

### Metadata

- Core, app and custom document properties

### Hidden logic

- Every formula with the cells, ranges, names and tables it reads, also across sheets
- Copied formulas folded into one pattern. In one engineering workbook from the test files, 2,133 formulas fold into 10 patterns
- Parameters: constant input cells that formulas read, with label, value and unit when one can be found
- Results: formula cells that no other formula reads, with the cached value
- A dependency graph across sheets and named ranges
- Defined names with their scope, also names used as variables
- Modern formulas: LET, LAMBDA (also as defined names), XLOOKUP and dynamic arrays with their spill range
- What-if data tables with their input cells
- Calculation settings: manual calculation and iterative calculation, which mean the stored results can be old
- Data validation rules and lists
- Conditional formats
- Legacy notes and threaded comments with person, date, replies and resolved state
- Pivot table sources and chart series ranges
- Links to other workbooks and add-ins, with their path
- VBA: module names and the user functions used in formulas. VBA is never run

### Pictures and embedded files

- Charts are described by their type and series ranges and can be drawn as Mermaid

## How it shows up in the output

Each sheet becomes a value table. Below it: parameters, formulas grouped by pattern, results and a Mermaid dependency graph. Large sheets get the graph on group level, not cell level. Hidden logic like user functions, external workbooks and data tables is listed on its own.

Example (not final):

```markdown
## Sheet: Summary

| Item    | 2025   | 2026   | Change |
|---------|--------|--------|--------|
| Costs   | 41,200 | 44,900 | 9.0 %  |
| Income  | 52,000 | 55,300 | 6.3 %  |
| Result  | 10,800 | 10,400 | -3.7 % |

### Formulas

| Cell | Formula              | Reads               |
|------|----------------------|---------------------|
| C2   | `=SUM(Costs!C2:C40)` | sheet Costs, C2:C40 |
| C4   | `=C3-C2`             | C3, C2              |
| D2   | `=C2/B2-1`           | C2, B2              |
```

## Known limitations

- XLSB is a binary format. Rezzer's own XML reader does not apply to it. For XLSB Rezzer gets values, formulas and VBA module names from calamine. Threaded comments, validation, names with scope, calculation settings and data tables are not planned for XLSB yet
- Rezzer does not calculate. It shows the cached results stored in the file. When a value is missing, the output says "not calculated in file"
- User functions from VBA or add-ins can't be evaluated. The output names them and says where they come from
- Units come from labels, number formats and `CONVERT` arguments. They are a guess, and the output says where each unit came from
- calamine 0.37.0 skips data table cells and array cells with an empty formula. Rezzer reads those from the XML
- A formula that fails to parse is shown as text, with a note
- No public test file was found with hidden columns, iterative calculation turned on or a custom percent format. Rezzer reads these attributes, but they are not proven on a real file
- Against hostile files Rezzer uses size, depth, cell and time limits

## Tested with

24 XLSX, 3 XLSM and 2 XLSB public test files (one of the XLSB files is broken on purpose) from Apache POI, Apache Tika, LibreOffice, calamine, IronCalc and two open engineering projects. See [MANIFEST-office.md](../spike-2026-10-19/MANIFEST-office.md).
