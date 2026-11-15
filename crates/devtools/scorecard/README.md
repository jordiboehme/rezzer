# scorecard

A development tool of Rezzer. It checks relational facts about a document against Rezzer's
model JSON (`rezzer read --json`) and counts how many hold. It is never linked into the
`rezzer` binary.

## The assertion file

A plain text file, one fact per line. Empty lines are skipped. A line that starts with `#` is
a comment. An assertion file `a/b.pdf.facts` belongs to the read `a/b.pdf.json`.

```text
# a comment
row: A | B@2 | C [table]
same_row: A | B [table]
not_row: A | B [table]
cell_under: HEADER => VALUE [table]
pair: LABEL == VALUE [pair]
heading: TEXT [heading]
heading: TEXT level 2 [heading]
order: A << B [order]
text: TEXT [text]
link: URL [link]
```

### Kinds

| Fact | It holds when |
|---|---|
| `row: A \| B \| C` | one row of a table has a cell for each text, left to right in this order. Other cells may lie between them. A text is the whole text of its cell |
| `same_row: A \| B` | one row of a table has a cell of its own for each text, in any order. For truth that does not state the order of the printed columns |
| `not_row: A \| B` | both texts stand in the document, and no row of any table holds both, in two cells or glued in one. A text that is lost fails the fact |
| `cell_under: HEADER => VALUE` | a cell of a header row holds the header, and a cell in a later row of the same table holds the value, and the columns of that cell all lie within the columns of the header cell. A table that marks no header row has its first row as one |
| `pair: LABEL == VALUE` | a table row has two cells with text, the label then the value. Or a table read as label and value pairs or as totals has the two cells side by side. Or one line of text reads `LABEL: VALUE` (see below). A colon at the end of a label does not count. A pair holds only in the body: a form field listed beside the body is no pair |
| `heading: TEXT` | a heading block has exactly this text. With `level N` at the end it also has this level |
| `order: A << B` | both texts are in the text of the document, and the first place of A is before the first place of B |
| `text: TEXT` | one block or one table cell holds the text |
| `link: URL` | a link has exactly this target |

A fact never holds on a read that lost one of its texts. A fact that stands more than once in
an assertion file counts once.

### A pair in a line of text

The colons of a line part it into segments. A colon between two digits (`10:30`) parts
nothing.

- The label is the text before its colon. At the start of the line it is the whole segment.
  After an earlier colon it is the last words of the segment that hold no digit.
- The value is the text after that colon. After the last colon it is the rest of the line.
  Before another colon it is the segment without the words of the next label.
- `#` matches only the first amount of that value. `~` matches only at the start of that
  value, at word bounds. A line does not always mark where a value ends: a next label with
  no colon is still in the value. An exact text is the whole value.

`Netto: 100,00 Brutto: 119,00` pairs `Netto` with `100,00` and `Brutto` with `119,00`, and
never `Netto` with `119,00`. In `Netto: 100,00 Brutto 119,00` (no colon after `Brutto`)
`pair: Netto == #100` holds, and `pair: Netto == #119` and `pair: Netto == ~Brutto` fail. In `Name: Erika Mustermann Ort: Musterstadt` the line does not
say where the value ends and the next label starts, so no pair is read from it. The fact
fails there. That is the safe side.

### A pair with the value under its label

A label over its value in a table (`TOTAL HT | TOTAL TVA` over the amounts) is no `pair`.
Write it as `cell_under` with the class `[pair]`:

```text
cell_under: TOTAL HT => #95 [pair]
```

### Separators

- `row`, `same_row` and `not_row` part their texts with ` | `.
- `cell_under` parts header and value with ` => `.
- `order` parts its two texts with ` << `.
- `pair` parts label and value with ` == `.

A line is split on the separator only when it has a space on each side. So texts like
`<1080>`, `a=b` and `KL. / CL.` stay whole.

A cell text that holds a literal `|` is written as `\|`.

### Texts

- White space is normalised: every run of white space is one space, and there is none at the
  ends.
- `<br>` is a line break. For the whole text of a cell a line break is white space.
- `A@2` means the text is the 2nd line of its cell. Lines are counted from 1.
- `~TEXT` means the place holds the text among other text. It is found only at word bounds.
- `#1234.5` means the place holds this amount, however it is printed. It is found in
  `1.234,50 EUR` and in `1,234.5`. The amount is written with digits and at most one point.
- The sign counts: `#-5` is found in `-5,00` and in `5,00-`, not in `5,00`. A hyphen after a
  letter or a digit is no sign (`RE-2026`, `1-2`).
- A point and a comma in one number: the last one parts the decimals. One mark more than
  once parts groups of three. A date or a version (`21.11.2020`, `1.2.3`) is no amount.
- `1.234` and `1,234` (one mark, three digits after it, one to three before) can be read
  two ways. The document decides: the mark that more of its amounts part decimals with is
  its decimal mark. In a document with comma decimals `1.234` is 1234 and `1,234` is 1.234.
  In a document with point decimals it is the other way. When the document shows no decimal
  mark, or both as often, the mark parts thousands: `1.234` is 1234.
- `*` alone means any text with a letter and no digit. It stands only for the label of a
  pair. It checks no label, so use it only when the label does not matter.
- A backslash takes the next character as it is. Write it before `|`, `[`, `]`, `@`, `~`,
  `#`, `*` and a backslash when they are text, and before a word that is a separator
  (`\==`, `\=>`, `\<<`).
- The target of a `link` is taken as written.
- Words that Rezzer adds are no text of the file: the page marker, "Found by" lines, picture
  notes and status marks.

### Classes

Every fact ends with its class in brackets: `[table]`, `[pair]`, `[order]`, `[heading]`,
`[text]` or `[link]`. The scorecard counts per class. The class may differ from the kind: a
`row` fact about a totals line can be `[pair]`.

## Commands

```text
scorecard check [--names] [--set <name>] [--tsv <file>] [--regions-tsv <file>] <assertions-dir> <json-dir>
scorecard trace <json> <anchor or text>
scorecard gen tagged <json>
scorecard gen invoice <pdf> <json>
scorecard gen fields <json>
scorecard gen all <pdf> <json>
scorecard strip-tags <in.pdf> <out.pdf>
scorecard half <name>
scorecard summary <title> <scorecard.tsv> <regions.tsv>
```

`check` prints `passed / total` per file and class, then the totals per half. Without
`--names` a file is `#n` and no text of a fact or of a file is printed. Use this for private
files. `#n` is the place of the file's read among all reads of the set, in the order of their
paths. An error uses the same number.

Without `--names` an internal error of any command prints only `internal error`, with
`in file #n` when a read of a set was being checked. With `--names` each file is named and its failing facts are listed. Use
this for public files only.

An assertion file with no read is counted per half (`unread_files` in the table). Its facts
are in no count, so a set with such a file is not whole.

`trace` prints the kind, the certainty, the evidence ids and the bounds of the region at an
anchor, or of the regions that hold a text. It prints no text of the file.

`gen` writes facts from the file itself: from the tags of a tagged PDF, from the invoice XML
a PDF carries (ZUGFeRD, Factur-X) and from filled form fields. The JSON is the read of the
file with its tags. Each fact is written once per file. A `cell_under` fact is written only
for a value that stands once in its table.

## What the truth is worth

- **Tagged truth is a drift check (truth from Rezzer's own tagged read).** The facts come
  from Rezzer's read of the file with its tags, and only facts that read holds are kept. So
  a set read with tags holds 100 % on the day its truth is made. A lower number later shows
  that the tagged read moved. It is no evidence that a read is right.
- **Tagged order facts stand within one flow.** An `order` fact is written for a block and
  the next block of its page that stands over or under it: half the width of the narrower
  one or more is shared. Two blocks side by side that the tags name one after the other
  give a fact only when a sign holds them together: the foot of a column and the head of
  the next column of a like width, which starts above that foot, a heading left of its block
  on the block's rows, a label that ends in a colon and its value on one row, or a sentence
  that runs on into a block that starts with a small letter. A next column may start with a
  heading: the like width is then that of the text under the heading. A heading left of a
  block heads it only when the block is as wide as the heading or wider. Other blocks side
  by side give no fact. A margin note and a line of a form, a head block and a date box, a
  title and a number box or the columns of a footer do not depend on each other. A reader may read either first. The tags' own sign
  of a flow, the structure parent of a block, is not in the read, so these tests stand in
  for it. A caption beside its picture gives no fact: a picture has no text to name. The
  last block of a page and the first of the next page give a fact too. The model does not say how a page is turned, so a page whose text is read
  in a turned frame gives no facts of a flow.
- **Untagged rows are a lower bound.** The same file is read with its tags taken out and
  scored against that truth. Text extraction is shared with the truth, and tag conventions
  that the layout does not show count as failures. The number is a lower bound on
  structure. It is no proof of right values.
- **Invoice truth checks presence, not pairing.** The XML states amounts and names, but not
  how the page prints them. A total and a party name are `text` facts: the data is printed.
  An item line is a `same_row` fact. No fact from the XML says which label an amount stands
  with.
- **Form fields give a pair only when the body shows it.** A filled field becomes a `pair`
  fact only if the body of the read holds the label with the value as a pair.
- **Links are left out of `all`.** They come from annotations, not from layout.

`strip-tags` writes a tagged PDF without its tags. The read of that file is scored against the
facts its tags gave. The exit code is 3 when the file has no tags.

`half` prints `dev` or `held`: the half of a set a file is in, by a hash of its name. For a
file in a folder the name of the first folder decides.

`scripts/measure/scorecard.sh <commit>` runs all of this for the public sets.
