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

A formula that cannot be read gives a `ParseError` with a short reason. `analyze` does not panic on any input. Very long or deeply nested formulas are refused by the parser limits.

formulakit does not calculate anything and never opens another workbook.

License: MPL-2.0
