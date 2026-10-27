# csvsniff

csvsniff looks at CSV text and tells you what is in it.

- The dialect: delimiter, quote character and whether there is a header row. Short title lines above the table are skipped and counted
- Per column: the type (integer, decimal, date, bool or text), a unit from the header, missing values and whether the values are unique
- The class of one cell value (`cell_class`): empty, text, number, date or bool
- Rows with an unexpected field count and quotes that are not closed
- Derived columns: a column that is the sum of other columns, with how many rows match

The input is text that is already decoded. Call `sniff` to find the dialect, then `analyze` for the column facts.

```rust
let text = "id,name,mass (kg)\n1,a,1.5\n2,b,2.5\n";
let dialect = csvsniff::sniff(text).unwrap();
let analysis = csvsniff::analyze(text, &dialect, 100_000);
assert_eq!(analysis.columns[2].unit.as_deref(), Some("kg"));
```

`analyze_until` takes a stop function, so a caller can end a long analysis early. It asks the function every 64 KiB of text, also inside one long record. The result then says it is cut.

The header row and the types are a guess from the data. Read them as hints, not as facts.

License: MPL-2.0
