# sheetlayout

sheetlayout finds the tables, titles and blocks inside one spreadsheet sheet.

You give it the cells of a sheet with their type, shown text and style flags. You also give it
the merged cells and the layout facts the file states: Excel tables, database ranges, pivot
tables, autofilters, defined names, frozen panes and print titles. It gives back a list of
regions in reading order. Each region has its exact range, says how it was found and how sure
that is.

Facts from the file decide first. Rules decide only where no fact exists. A style flag that
the file does not give is unknown, never false.

sheetlayout never builds a grid of the used area. It works on the sorted cell list and on
rectangles. A merge over the whole sheet costs as much as one cell.

## Regions by gaps

A region is a rectangle of occupied cells with at least one fully empty row or column between
it and other content. A cell is occupied when it holds a value, a text or a formula, or when a
merged area covers it. A styled cell without a value is not occupied. The sheet is cut at
every run of empty rows, then each band at every run of empty columns, again and again until
no piece can be cut. Two blocks that touch only at a corner are two regions. The corner
split follows the cells and merges, not the area they span. A shape with very many long
merges crossed by many wide merges needs too much work and stays one region, and an issue
says so.

Cells inside an Excel table, database range, pivot table or autofilter range belong to that
range. The cells around it are cut apart from it, so no other region holds its cells.

This rule has limits:

- It cannot separate two tables that touch with no empty row or column between them.
- It cannot join one table that has an empty row inside it. A defined name over both parts
  joins them.
- It cuts at most 64 levels deep. A piece that needs a deeper cut stays one grid region,
  and an issue says so.
- It gives at most 10,000 regions per sheet. Past that limit the rest of the sheet is one
  grid region, and an issue says so.

A stop function lets the caller end the work early. The cells not yet placed then become one
grid region, and an issue says so.

## Titles

The leading rows of a region that hold one text each are its caption rows, at most 3. Where
styles are known, the run ends at a row with fewer marks (bold, fill) than the row before it.
A region of caption rows only that stands at most 2 empty rows above another region becomes
that region's title.

## Headers

`detect` finds the regions and then the header of each one. The header band is the top rows
of a region after its caption rows.

- Values change type: the band is the leading rows that hold only text and years (whole
  numbers from 1900 to 2100), at most 4. The next row that is not a single text must hold a
  number, a date, a bool or a formula. A vertical merge that starts in the band extends it.
- Styles change: where styles are known, the band is the leading rows that are all bold or
  all filled, at most 4, when the next row is not. Rows with a single text are skipped.
- When both rules find a band and the bands differ, sheetlayout does not pick one. The
  region stays a grid, and the reason says why.
- A region of only text gets its first row as header only when a style or a header guess
  says so, the cells fill every column, are all different and do not repeat below.
- Print titles that start at the first row after the captions set the band, up to 10 rows.
  Frozen panes only confirm a band or label columns.
- Excel tables, database ranges, pivot tables and autofilters keep the header the file
  states.

Each column gets a header path: the band cells above it, top to bottom, joined with ` / `. A
horizontal merge or a text centered across selection counts for every column it covers. A
vertical merge counts once. A text above an empty cell that is not merged belongs to its own
column only. A column with no header text gets an empty path.

Up to 3 left columns whose data rows hold text in at least 80 % of the rows are label
columns.

The header rules have limits:

- A row under the header with only one text, such as a single unit `cm`, ends the header. The
  unit is then not part of the path.
- A bold header row over a plain second header row makes the rules disagree. The region
  stays a grid.
- Numbers as column titles are not found as a header, except years.
- A group label row with only one text above the header is taken as a title row.
- A first data row with only text, such as `Ann | n/a`, is taken as part of the header.
- Two columns of labels and numbers, such as `Age: | 31`, read as a table whose header is
  the first row.
- A text formula counts as data, never as a header cell.

Path text is limited per sheet. A region whose paths would pass the limit stays one grid
region, and an issue says so.

## Rows of a table

After the header, `detect` looks at the rows of each table.

- A row with one text in the first column, or one merged area over at least 80 % of the
  width, is a group row when a data row follows it. Several such rows in a row are nested
  groups when a data row follows the last one.
- Rows with one text after the last row with values are notes, merged or not. A row with
  one text between value rows is a data row unless it is a group row.
- Rows can have levels: from the row outline first, then from the indent, then from the
  length of leading spaces or dots. The parent of a row is the nearest row above it with a
  lower level. Group rows can be parents. When no row gets a parent, no levels are given.
- A row is a subtotal or a total only by a fact or a formula: an Excel table totals row, a
  database range footer, a pivot total row, a formula with SUBTOTAL or AGGREGATE, or a SUM
  over the rows above in its own column. A SUM or SUBTOTAL that a later total reads is a
  subtotal. A SUM or SUBTOTAL that no later total reads and that covers the table, from
  its first data row past the last group row, is the total. AGGREGATE always gives a
  subtotal. A SUM that also reads its own row, such as a running balance, marks nothing. A row
  with the word "Total" and no formula stays a data row.
- Header-less tables get the same row rules.

## Report sheets

A region with no header goes through these rules, in this order:

- Section headings: a row with one text in the first column, bold, filled or merged, cuts
  the region when a row follows it that is not a single line of text. Without styles only a merge
  counts. Each part is read again with all the rules.
- Label and value pairs: 2 or 3 columns, at least 2 rows, a different label in the first
  column of every row and at least one value. A third column must hold units.
- Text: one column of text, or rows of one merged text each.
- A region with a number, a date or a bool is a table without a header.
- Anything else stays a grid.

Then two rules join regions. A region with no header right below a table, in the same
columns and at most 1 empty row down, is part of that table. A text region at most 2 empty
rows below a table and inside its columns is a note of that table. Text beside a table stays
its own region.

These rules have limits:

- A label and value list with a number, such as `Age: | 31`, reads as a table, because the
  header rules run first (see the header limits).
- A units row such as `cm` under one header column stays a data row.
- A note within 2 empty rows above another table becomes the title of that table, not a
  note of the table above it.
- A note wider than its table is not joined to it.
- An Excel table, database range or pivot table is never continued by another region.
- Sections that would pass the region limit are not cut, and an issue says so.
- When totals stand below their items, a subtotal row can become the parent of the items
  of the next block.
- A total row above its items is not marked.
- A label alone in the first column with a data row after it reads as a group row, even
  when it is a data row with empty values.
- A data row with only a label at the end of a table, such as `d | | `, is taken as a note.

License: MPL-2.0
