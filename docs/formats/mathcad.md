# Mathcad and SMath

Rezzer reads engineering calculation worksheets without the vendor's software. It shows every definition with its expression, unit and last computed value. It also shows which values are inputs and which are results and how they depend on each other.

Status: planned for a later release.

File types:

- Mathcad Prime: `.mcdx` (Prime 3 to 12 seen in test files)
- Mathcad 12 to 15: `.xmcd`
- SMath Studio: `.sm`
- Mathcad 11 and older: `.mcd` is detected, but not read (see below)

Rezzer detects the format by content, not by extension. A Prime file is a zip package with Mathcad content types (`application/vnd.openxmlformats-officedocument.mathprocessingml.mathcad.*`). An `.xmcd` file has the Mathsoft worksheet namespace on its root element. An SMath file starts with an `application` processing instruction that names SMath Studio. Rezzer also tells a Mathcad `.mcd` file apart from the many other formats that use `.mcd`.

Built on:

- Rezzer's own reader on [zip](https://crates.io/crates/zip) and [quick-xml](https://crates.io/crates/quick-xml). No crate exists for these formats
- The format knowledge comes only from public sample files and public notes under the MIT license. No vendor software, schema files or code are used

## What Rezzer extracts

### Structure

- Text regions in page order, with their position, so a value can be tied to the label next to it
- Headers and footers
- Solve blocks with guess values, constraints and the solver call

### Metadata

- Mathcad Prime: product version, engine version, build date, culture, creator, revision
- Mathcad 12 to 15: title, description, author, company, keywords, document IDs and revision
- SMath: version and the plugins the worksheet uses

### Hidden logic

- A definitions table: name, kind (variable, function, range, matrix, program), the expression as readable text, input or result, the last computed value with its unit, its status and its position
- Inputs and results: a definition with only numbers, units and constants on the right side is an input. A definition that reads other variables is derived
- Status of each result: current, pending (not calculated when the file was saved) or an error with its message
- Units: the stored unit and the display unit the author chose
- A dependency graph. Mathcad reads a sheet top to bottom and left to right and allows a name to be defined again, so each use is linked to the nearest earlier definition, not only by name. Global definitions and local definitions inside programs are handled
- Plots in Mathcad Prime: what is plotted against what, axis units, ranges and the cached data points
- Provenance in Mathcad 12 to 15: where a pasted expression came from. This can hold local paths and user names, so it is marked as possibly sensitive
- Protected areas are read when they are plain XML and marked "protected area (locked by author)"

### Pictures and embedded files

- Pictures are extracted to files and marked in the text
- Plots in Mathcad 12 to 15 are stored in a binary form. Rezzer does not parse them. It extracts the picture of the region that Mathcad stores with it and lists it as "needs vision"

## How it shows up in the output

A definitions table per worksheet, then the results, then a Mermaid dependency graph. Plots from Mathcad Prime can become a data table or a Mermaid chart.

Example (not final):

```markdown
| Name | Kind | Expression | Role | Value | Status |
|---|---|---|---|---|---|
| w_c | variable | 1.3 m | input | 1.3 m | current |
| h_c | variable | z - (FUK + t) | result | 1.92 m | current |
```

## Known limitations

- Binary `.mcd` files from Mathcad 11 and older can't be read. There is no public format description and no open reader. The output says so and points to PTC's converter for legacy worksheets
- Rezzer never calculates a worksheet. It shows the values stored in the file. Worksheets can call DLL functions and write files when they are calculated, so Rezzer does not run anything
- Mathcad Prime stores values in SI base units. Rezzer converts them to the display unit with its own unit table, or shows the SI value and says so
- Passwords are never removed, guessed or bypassed. Encrypted content is reported as unreadable
- Not read and said in the output: binary plot data in Mathcad 12 to 15, plugin regions Rezzer does not know, scripted controls, Excel components, the body of DLL functions and files read by functions like `READPRN`
- Mathcad Prime files contain zip packages inside the zip package. Rezzer applies the same limits on entry count, size, ratio and depth as for office files
- No public test file was found for binary `.mcd`, `.xmcdz`, Mathcad 15, Prime 1 and 2, password protected areas, SMath 1.x or SMath plot regions
- Mathcad and Mathcad Prime are trademarks of PTC. SMath Studio is a product of SMath LLC. Rezzer is not related to either company

## Tested with

10 Mathcad Prime, 5 Mathcad 14 and 3 SMath public sample files from open projects on GitHub under the MIT or CC0 license. See [MANIFEST-mathcad.md](../spike-2026-10-19/MANIFEST-mathcad.md).
