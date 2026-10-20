# formulakit

formulakit reads a spreadsheet formula and tells you what it uses.

- The cells, ranges, defined names, tables and other workbooks it reads
- The functions it calls and which of them are not built in (VBA, add-ins, functions from another workbook)
- Whether it is volatile (NOW, RAND, OFFSET, INDIRECT and similar)
- The LET and LAMBDA parameter names, which are never reported as defined names
- A display form in Excel A1 syntax without storage prefixes like `_xlfn.`
- A pattern key in R1C1 form, so copies of one formula share a key

It reads Excel formulas as stored in XLSX files and OpenFormula as stored in ODF files (`of:=[.B4]+[$Data.A3]`). OpenFormula is turned into Excel A1 syntax.

```rust
use formulakit::{analyze, anchor_of, CellPos, Dialect};

let a = analyze("=SUM(Data!B2:B9)*rate", Dialect::Excel, CellPos { row: 0, col: 3 }).unwrap();
let anchors: Vec<String> = a.refs.iter().map(|r| anchor_of("Main", r)).collect();
assert_eq!(anchors, ["Data!B2:B9", "name:rate"]);
assert_eq!(a.pattern_key, "=SUM(Data!R[1]C[-2]:R[8]C[-2])*rate");
```

Anchors look like `Sheet!A1`, `'My Sheet'!A1:B9`, `name:gg`, `name:Sheet1!gg`, `table:Table1` and `ext:[1]Sheet1!A1`.

For a whole workbook, `logic::Workbook` takes every formula, constant cell and defined name and tells you:

- The role of each formula: constant (reads nothing), result (nothing reads it) or intermediate
- Pattern groups: copies of one formula on one sheet, with their area (`D2:D100`)
- Parameters: constant cells that formulas or names read, with how many formulas read them
- A dependency graph on group level: groups, single formulas, parameter blocks, names and other workbooks

Ranges are not expanded cell by cell, so `A:A` costs what the sheet holds. Tables added with `add_table` let structured references like `Sales[Price]` and `[@Qty]` resolve to cells. A stop callback ends the work early.

`latex` writes a formula as LaTeX math that KaTeX can show (`=B2*C2/1000` becomes `\frac{\mathrm{B2} \cdot \mathrm{C2}}{1000}`). `worth_math` tells if a formula is more than a sum or a single reference. Text from the file is escaped, so it never becomes a TeX command.

A formula that cannot be read gives a `ParseError` with a short reason. `analyze` does not panic on any input. Very long or deeply nested formulas are refused by the parser limits.

formulakit does not calculate anything and never opens another workbook.

License: MPL-2.0
