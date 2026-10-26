# XLSX, XLSM and XLSB

Rezzer finds the tables, titles and notes inside each sheet and shows the calculation behind them: every formula with the cells it reads, inputs, results and the dependencies between them.

Status: available in the development version (no release yet).

File types: `.xlsx`, `.xlsm`, `.xlsb`. Rezzer detects the format by content, not by extension.

Built on (versions as used in the development version):

- [calamine](https://crates.io/crates/calamine) 0.37.0 for values, cached results, formula texts, merged cells and VBA module names
- [formualizer-parse](https://crates.io/crates/formualizer-parse) 4.0.0, behind Rezzer's own formulakit crate, to find the cells, ranges, names and tables each formula reads
- Rezzer's own readers on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml) for the parts no crate exposes well (XLSX and XLSM): sheetparts for the workbook parts, zipguard to check the package before anything is unpacked

## What Rezzer reads

### Structure

- The tables inside each sheet with their header rows, titles and notes (see [Tables inside a sheet](#tables-inside-a-sheet))
- Label and value blocks and text lists of report sheets
- Merged cells
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

Every parameter gets a path: the parts that name it, from wide to narrow. For a value in a table these are the title of the table, the section headings above it, the header of its column and, when the table has label columns, the label of its row. In a label and value block they are the title, the section headings and the label. A cell outside any table takes its label from the text left of it or from the column header above it, as before.

In the Logic section the parameters of one table stand together. What their paths share is written once as a title above them, like `Parameters in INPUTS:`. Each row then shows the shortest end of its path that is unique in that group, like `2021 / United States`. Where a label stands without its title (in a reference from another sheet and in the JSON model) Rezzer writes the full path. When a page of the output starts inside a sheet's Logic part, the page repeats the heading and the title, marked `(continued)`.

Which text counts as a label does not change. A text on the left that repeats in its column, like a category `blue` in a data table, is a value of the row and not a label. The same is true for any text in a column with at least four texts where at least half of them repeat. A label with a unit in brackets, like `Load [kN]`, always stays the label, with its unit. A bare unit like `kg` or a size like `DN 50 mm` can still be a value of the row. In a list like `Diameter | 10 | m` the name on the left stays the label. A header cell whose own text holds ` / ` gives two parts of the path.

### Units

A parameter gets a unit only when the file says so. The output names the source of each unit:

- the label, like `Load [kN]`, `Excavated diameter (m)` or `Span in mm`
- the cell beside the value, like `10 | m`
- the number format, like `0.0" m"`
- a `CONVERT` call that converts the value from a unit written in the formula
- the column header above a run of values, like `t (m)`

Only units from a fixed list count. One-letter units in round brackets count only for m, g, s, l and L, because `(V)` or `(N)` is more often a variable name. Codes like `N/A` are no units. When two sources disagree, the parameter gets no unit and a note says why.

## Tables inside a sheet

A sheet is often more than one table. It has titles, notes, header rows over two or three lines, side tables, group rows and totals. Rezzer finds these parts and writes each one as what it is.

### What Rezzer finds

- Several tables on one sheet, side by side or one below the other
- Header rows, also over several lines with merged cells. A column gets its full header path, like `Population Estimate (as of July 1) / 2021`
- Titles above a table and notes below it
- Label columns on the left of a table
- Group rows inside a table, like `Year 2024` over its months
- Row levels from row outlines, indents and leading dots, shown as a `Parent` column
- Subtotal and total rows
- Report sheets: section headings, label and value blocks (`Label: value unit`) and text lists. A third column of unit symbols like `m`, `kPa` or `kg/m3` is read as the unit. A count word like `nos` or `pcs` counts too when a unit of two or more characters, like `Pa` or `m/s`, stands in the same column. One-letter codes like `A` or `N` alone and codes like `N/A` stay values

### How it decides

Facts from the file come first. Excel tables, pivot tables and filters give the exact range and header rows. Print titles give the header rows. Frozen panes and defined names over a range help. Only where the file says nothing, Rezzer uses its own rules: empty rows and columns between blocks, a change of type between the header and the data, bold or filled header cells, merged titles. Each table says how it was found.

When the rules do not agree on a header, Rezzer does not pick one. The table is shown as a raw grid with the column letters as header, and "Not read" lists it as unsure.

Totals come from facts and formulas only: a totals row of an Excel table, the grand total row or column of a pivot table or a row with `SUBTOTAL`, `AGGREGATE` or a `SUM` over the rows above it. A word like "Total" or "Summe" is not enough, so a pasted total row without a formula is not marked.

### How it shows up

Each part gets a heading: its title when it has one, else the name of its Excel table or of a defined name over it, else its kind and range, like `### Table A40:C40`. A title right on top of a side table is that table's title. A sheet with only one table and no title gets no extra heading. A layout line under the heading says where the table is and how it was found:

```markdown
Table A1:F68 (data A5:F62), header rows 3 to 4, row labels in A, found by: defined name \_NST01, print titles; rules: empty-row gaps, caption rows 1 to 2, header by type change, bold header.
```

A merged cell keeps its text once. `〃` marks a cell merged from above and `←` a cell merged from the left, with a note that lists the merged ranges. Group, subtotal and total rows are named in a line under the table, and so is the grand total column of a pivot table: `Total column: D (pivot table grand total).` No cell text is changed.

With `--table-style records` (MCP `table_style: "records"`, or `table_style = "records"` under `[read]` in the config file) every table row becomes one list item, with the header path of each value:

```markdown
- Row 18 (Table\_1\_01!A18:P18), group: Year 2024
  - Period: February
  - Generation at Utility Scale Facilities / Coal: 44,137
  - Generation at Utility Scale Facilities / Petroleum Liquids: 740
```

Records repeat the header, so they cost about four times as much as a table line. In return each row stands on its own. Empty cells are left out. A value in the grand total column of a pivot table ends with `(total)`. A raw grid stays a grid. A table without a header row stays a table too, because there is no header to name the fields: its layout line says so. Label and value blocks and text lists look the same in both styles.

`--sheet <name>` (MCP `sheets`) reads only some sheets, by name or by position like `#2`. Give it more than once for more sheets. Rezzer still reads the whole workbook, so formulas that read other sheets stay correct. The overview marks the other sheets as not selected. `--table-style` and `--sheet` change only the Markdown: `--json` gives the full model with all sheets.

### Limits

- Two tables that touch with no empty row or column between them are one table. One table with an empty row inside it can become two (a defined name over the whole range joins them)
- A cross table whose header rows sit above and to the right of its corner labels becomes two tables. Every cell is kept
- A total row without a formula is not marked as a total
- A pivot table with more than one value field can have more than one grand total row or column. Only the last one is marked
- A unit found in a label and value block, like `veh/hr` in `Traffic volume: 1200 veh/hr`, may not show up in the Logic section. There the label of the value cell is read by its own rule
- For XLSB Rezzer reads values only, no styles, Excel tables or filters. It finds the tables from the values and says so under "Not read"

## How it shows up in the output

Each sheet becomes its tables and blocks as above. Then a Logic section follows with one part per sheet: parameters, formulas grouped by pattern, results, rules. After that come the defined names, a Mermaid dependency graph and the hidden logic (user functions, add-ins, other workbooks, data tables, spill ranges, VBA modules, calculation settings). Formulas can also be shown as LaTeX math (`--no-math` turns that off). In the graph a copied formula is one box, not one box per cell.

## Example

Real output of `rezzer read` for `tunnel_design_calculations.xlsx` from the test corpus. The part for the Ventilation sheet, cut to two of its 13 parameters and three of its 14 formula rows:

```markdown
### Ventilation

#### Parameters

Parameters in INPUTS:

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
- XLSB is a binary format, so Rezzer's XML readers don't apply to it. For XLSB Rezzer gets values, cached results, formulas, defined names, document properties and VBA module names. It does not read validation, conditional formats, notes and threads, names with scope, calculation settings, data tables, number formats beyond dates, styles, hidden rows and columns, Excel tables, pivot tables, charts and external links. The output lists this under "Not read"
- User functions from VBA or add-ins can't be evaluated. The output names them and says where they come from
- Copies of a formula fold only when they have the same form relative to their cell. A copied formula that reads a fixed cell without `$` gets one row per cell
- A formula that fails to parse is shown as text, with a note
- Encrypted workbooks are reported as encrypted and not read
- No public test file was found with a very hidden sheet, hidden columns, iterative calculation turned on or a custom percent format. Rezzer reads these, but they are not proven on a real file

## Damaged and hostile files

Rezzer reads every file as untrusted data:

- Each entry is measured before it is unpacked. Defaults: 2 GB per file, 4 GB unpacked, at most 100 times the packed size per entry, XML nesting up to 256 levels, 120 seconds, 2 GB of memory and 2 million elements. The unpacked limit is checked against what the file really unpacks to. Each part counts once, even when Rezzer reads it more than once
- A table shows at most 1,000 columns, and a sheet at most 200,000 cells. The output says what was cut. Its formulas still show up in the Logic section
- A formula longer than 8,192 characters (the Excel maximum) is kept as text and not analyzed
- A DOCTYPE in a part is refused, so entity bombs don't expand
- A package that can be read in two ways is refused as damaged: two main workbook parts, two relationships with one id, two sheets with one name, part paths with `.` or `..`, entries that differ only in case, relationship or sheet markup Excel never writes, control characters in sheet names
- A sheet that breaks halfway keeps the cells before the break. The output says where it stopped

## How it is tested

The corpus has 33 XLSX, 3 XLSM and 2 XLSB public test files from Apache POI, Apache Tika, LibreOffice, calamine, IronCalc, two open engineering projects, the US Census Bureau and the US Energy Information Administration. One XLSX file and one XLSB file are broken on purpose. See [MANIFEST.md](../../tests/corpus/MANIFEST.md).

Extreme cases the test suite holds, at the default limits unless a limit is named (the figures are the bounds the tests check):

- A sheet of 64 MB of spaces, packed far over 100 times: refused before it is unpacked, peak heap under 512 MB
- Two cells, A1 and XFD1048576, 17 billion cells apart: peak heap under 64 MB, two tables, the far formula is still listed
- One merge over the whole sheet, A1:XFD1048576: under 2 seconds of CPU time and 64 MB of heap. The value under the merge is listed as not shown
- 50,000 merges and no values: under 2 seconds of CPU time and 512 MB of heap
- 40,000 single cells in a checkerboard: past 10,000 blocks the rest of the sheet stays one raw grid, and the output says so
- 100,000 nested XML elements after the cells: the read stops at 256 levels and keeps the cells before
- An entity bomb that expands to a billion words: the DOCTYPE is refused, nothing expands

Speed: `ex012.PVT_z_factor.xlsm` (130 KB, 2,133 formulas) reads in 0.02 seconds with a peak memory of 14 MiB. Median of 3 runs with `--no-cache`, on an Apple M5 (Mac17,3, 32 GB, load average 3.2), release build (cargo profile release) of Rezzer 0.1.0 at commit 958df97, 2026-10-21.
