# CSV and TSV

Rezzer reads delimited data files as tables and describes the data: column types, keys, missing values, units and relations between columns that it can check.

Status: available in the development version (no release yet).

File types: `.csv`, `.tsv` and delimited data in `.txt` files. Rezzer detects the format by content, not by extension. A `.txt` file is checked for delimited data, and the output says how the format was decided.

Built on (versions as tested on 2026-10-19):

- [csv](https://crates.io/crates/csv) 1.4.0 to read the records (quoting, line breaks in fields, CRLF)
- The same encoding detection as for [plain text](text.md): [encoding_rs](https://crates.io/crates/encoding_rs) 0.8.42 and [chardetng](https://crates.io/crates/chardetng) 1.0.0
- Rezzer's own reader for the delimiter, the header row, column types and the relation check

## What Rezzer reads

### Structure

- The table, with the detected delimiter (comma, semicolon, tab and others)
- Header row detection. When there is no header, the output says "header assumed" or "no header" with the reason
- Broken rows: a count of rows by number of fields, and fields with line breaks inside

### Metadata

- Encoding and how it was found (byte order mark, valid UTF-8 or a guess)
- Row and column counts

### Hidden logic

- Column types: integer, number, date, yes/no, text
- Columns that look like keys (unique values)
- Missing values per column
- Units from the column header, or from a `datapackage.json` or CSVW file next to the CSV when there is one
- Derived columns: Rezzer checks whether a column is the sum of other columns and reports the result with its error, as an observation, not as a fact

## How it shows up in the output

The table, then a `Columns` table (column, type, unit, missing, unique, key) and a Logic section with the relations that were checked, marked as found in the data and not written in the file.

## Example

Real output of `rezzer read` for `csv-datahub-co2-fossil-global.csv` from the test corpus. The overview and the Logic section; the Properties section, the table of 275 rows and the Columns table between them are cut:

```markdown
## Overview

| Part | Kind | Size | Role | Reads from |
| --- | --- | --- | --- | --- |
| csv-datahub-co2-fossil-global | sheet | 275 rows x 8 columns | result |  |

- Formulas: 1

## Logic

Found in the data, not written as formulas in the file.

| Cells | Formula | Reads | Role | Value | Note |
| --- | --- | --- | --- | --- | --- |
| csv-datahub-co2-fossil-global!B:B | `Total = Gas Fuel + Liquid Fuel + Solid Fuel + Cement + Gas Flaring` | csv-datahub-co2-fossil-global!C:C, csv-datahub-co2-fossil-global!D:D, csv-datahub-co2-fossil-global!E:E, csv-datahub-co2-fossil-global!F:F, csv-datahub-co2-fossil-global!G:G | result |  | holds 210 of 275 rows |
```

## Known limitations

- Header detection on files without a header is a guess. The output says so
- An unterminated quote makes the csv crate read the rest of the file as one field, without an error. Rezzer reports the field count per row so this case is visible
- Encoding detection has no confidence score in chardetng 1.0.0. Short Latin-1 files with few special characters are a weak spot. The output says how the encoding was found
- Relations are only checked on files with at least 10 counted rows. Small files get no relation claims
- A very small file (3 to 9 lines) with a ragged row is read as CSV only when it has a `.csv` or `.tsv` extension and its header row and first record have the same field count. Otherwise it is read as text
- The table shows at most 10,000 rows, 1,000 columns and 200,000 cells (counted after short rows are padded). The Columns table lists at most 500 columns. The output says what is not shown
- qsv-sniffer 0.12.2 was tested for delimiter and header detection. It failed or was wrong on 8 of 13 files and its repository is archived, so Rezzer does its own detection

## How it is tested

The corpus has 12 public files: 11 CSV files (one of them a semicolon file named `.txt`) and 1 TSV file, from csv-spectrum, pandas, the German weather service (DWD), GeoNames and the datahub datasets. Two of them were made from public files (one changed to Latin-1 without a header, one broken on purpose). See [MANIFEST.md](../../tests/corpus/MANIFEST.md).

Extreme cases the test suite holds, at the default limits unless a limit is named (the figures are the bounds the tests check):

- One row of 50 MB without a line break, with a time limit of 1 second: the read ends within the limit plus 2 seconds
- 100 MB of short rows with a time limit of 50 ms: the read stops in time and says so
- 3,000 columns by 200 rows: the table shows 1,000 columns and 200,000 cells, the Columns table 500 columns. The output says what is not shown

Speed: `csv-semicolon-latin1-dwd-stations.txt` (148 KB, 1,091 rows x 7 columns) reads in 0.02 seconds with a peak memory of 9 MiB. Median of 3 runs with `--no-cache`, on an Apple M5 (Mac17,3, 32 GB, load average 3.2), release build (cargo profile release) of Rezzer 0.1.0 at commit 958df97, 2026-10-21.
