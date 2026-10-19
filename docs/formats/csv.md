# CSV and TSV

Rezzer reads delimited data files as tables and describes the data: column types, keys, missing values, units and relations between columns that it can check.

Status: planned for the first release. Nothing is implemented yet.

File types: `.csv`, `.tsv` and delimited data in `.txt` files. Rezzer detects the format by content, not by extension. A `.txt` file is checked for delimited data, and the output says how the format was decided.

Built on (versions as tested on 2026-10-19):

- [csv](https://crates.io/crates/csv) 1.4.0 to read the records (quoting, line breaks in fields, CRLF)
- The same encoding detection as for [plain text](text.md): [encoding_rs](https://crates.io/crates/encoding_rs) 0.8.42 and [chardetng](https://crates.io/crates/chardetng) 1.0.0
- Rezzer's own reader for the delimiter, the header row, column types and the relation check

## What Rezzer extracts

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

The table, then a `Columns` table (name, type, unit, missing, distinct, key) and a `Relations` section with the checked relations.

Example (not final):

```markdown
## Relations

- Total ~ Gas Fuel + Liquid Fuel + Solid Fuel + Cement + Gas Flaring
  (210 of 275 rows within rounding, residual up to 125)
```

## Known limitations

- Header detection on files without a header is a guess. The output says so
- An unterminated quote makes the csv crate read the rest of the file as one field, without an error. Rezzer reports the field count per row so this case is visible
- Encoding detection has no confidence score in chardetng 1.0.0. Short Latin-1 files with few special characters are a weak spot. The output says how the encoding was found
- Relations are only checked on files with at least 10 counted rows. Small files get no relation claims
- qsv-sniffer 0.12.2 was tested for delimiter and header detection. It failed or was wrong on 8 of 13 files and its repository is archived, so Rezzer does its own detection

## Tested with

14 public test files from csv-spectrum, pandas, the German weather service (DWD), GeoNames and the datahub datasets. Two of them were made from public files (one changed to Latin-1 without a header, one broken on purpose). See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
