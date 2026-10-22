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
no piece can be cut. Two blocks that touch only at a corner are two regions. This corner
split is skipped for a very sparse piece, which then stays one region.

Cells inside an Excel table, database range, pivot table or autofilter range belong to that
range. The cells around it are cut apart from it, so no other region holds its cells.

This rule has limits:

- It cannot separate two tables that touch with no empty row or column between them.
- It cannot join one table that has an empty row inside it. A defined name over both parts
  joins them.
- It cuts at most 64 levels deep and gives at most 10,000 regions per sheet. Past either
  limit the rest of the sheet is one grid region, and an issue says so.

A stop function lets the caller end the work early. The cells not yet placed then become one
grid region, and an issue says so.

## Titles

The leading rows of a region that hold one text each are its caption rows, at most 3. Where
styles are known, the run ends at a row with fewer marks (bold, fill) than the row before it.
A region of caption rows only that stands at most 2 empty rows above another region becomes
that region's title.

License: MPL-2.0
