# layoutregions

layoutregions is the region engine for pages that have text runs, rules, boxes and pictures.

A format reader hands over one page. The page lists its text runs, rules, boxes, pictures and
codes. It can also list regions that the file states itself. The crate gives back regions. Each
region has a kind, its bounds, the pieces it holds and the signals that decided its kind.

The engine reads the layout to find meaning. It does not copy the layout. A wrong value is
worse than an honest fallback, so a region without clear evidence is not typed.

## What it does

- File facts: before any page is laid out, `file::learn` reads what the file says as a whole.
  These are the body size, the heading sizes, the running headers and footers and the
  footnote legend of each page.
- The cut: `segment::segment_page` builds one tree of candidate regions for a page. It nests
  the regions the file states, the boxes and grids the drawn lines make, the text between two
  rules, the whitespace cut (column gutters and bands of white space across) and steps of
  style (size, weight, colour). A segment says what parted it, not what it is.
- The typers: `typers` has one function for each kind. It scores a segment from named signals,
  so a test can ask why a region got its kind. A typer answers only on evidence for its own
  kind. The score is the sum of the weights of its signals, at most 1000. The signals are
  stated relative to the sizes of the segment, so a page at another scale gets the same
  scores.
- The decision: `arbitrate` picks one kind for each segment. The best kind wins only with a
  signal, a least score (300 by default) and a lead over the next kind (150 by default). The
  kinds that lost are kept as alternatives. A region the file states keeps its kind.
- Every piece of the page is in exactly one region.

The kinds are header band, footer band, title block, heading, paragraph, list, table, label
and value pairs, totals, figure, code, footnote legend and spatial.

## Label and value pairs

`extract::pairs` reads the text of a drawn box or of a cell between rules as fields. A field
is a label and its value, or a label alone (an empty field). The typer for pairs also takes
`Label: value` lines, a label over its value and a row of labels over the row of their
values. A pair whose value is not clear is never guessed: the caller gets the reason and the
runs.

## Columns and reading order

`order` puts the regions of a page in reading order in one pass. Header bands come first.
Footnote legends and footer bands come last. A wide drawn rule parts the body into above and
below. A gutter parts it into columns, read left to right, each down to its end. A region
across the columns (a title, a figure, a table) parts them into bands. Lengths are measured
in the body size of the page, so the order is the same at every scale. `order_by_source` is
for a file whose own order is its reading order: only what has no place there is placed.

`extract::text` reads the lines of a region as headings, list items and paragraphs with
their line breaks. `extract::table` reads the cells of a table.

## The spatial fallback

A segment without clear evidence gets the kind spatial, with certainty unsure.
`extract::spatial` reads it line by line: each line in pieces, each piece at its character
column. It keeps every text run of the region exactly once, in reading order. The caller can
write it as plain lines or in its columns, and can say that it was unsure.

## Limits and work steps

Every limit has a default (`Limits`): 10,000 regions per page, 64 levels of the cut, 4
alternatives per segment and 100,000 candidates per page. The caller can also pass a stop
callback, for example for a time limit. When a limit is hit or the caller stops, the part
stays whole and the crate says so in an issue. It never drops something without a word.

`work_steps` counts the work done, in debug builds only. The count is the same on every
machine for the same input, so a test can bound the work without measuring time. A release
build does not count.

The crate has no dependencies. It does not know any file format.

License: MPL-2.0
