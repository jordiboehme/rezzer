# sheetparts

sheetparts reads the parts of a spreadsheet package that give the cells their meaning. Spreadsheet crates return cells. This crate returns the rest.

- Sheets with their name, visibility (visible, hidden, very hidden) and part name
- Defined names with scope, formula text, hidden flag and comment. An array constant such as `{"A","B"}` is kept whole
- Calculation settings: automatic or manual, and iteration for circular references
- Number formats: the format code of every cell style, plus a formatter that turns a number and a format code into the text a person sees. It also tells the kind (percent, currency, date, ...) and the unit (a currency symbol or literal text such as the `m` in `0.0" m"`)
- Document properties: core, app and typed custom properties. `ooxml::read_doc_props` reads them from any OOXML package, also from XLSB, whose other parts are binary
- External links (workbook, add-in, DDE, OLE) and the people behind threaded comments
- Whether the package carries a VBA project. It is never read or run
- ODF spreadsheets (`odf::read`): hidden sheets, rows and columns, annotations with author and date, validations, number formats built from data styles, named ranges and document metadata. Repeat counts are never expanded

```rust
use sheetparts::numfmt;

let f = numfmt::format("0.0\" m\"", 12.34);
assert_eq!(f.text, "12.3 m");
assert_eq!(f.unit.as_deref(), Some("m"));
assert_eq!(numfmt::format("0%", 0.09).text, "9%");
```

`ooxml::read_workbook` takes a `zipguard::Archive`, limits and a stop function. XML is treated as untrusted: a DOCTYPE is refused, only the five predefined entities are known, and depth and item counts are limited. Nothing panics. Every problem is an `Issue` in the result and the other parts are still read.

License: MPL-2.0
