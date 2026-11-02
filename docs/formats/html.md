# HTML

Rezzer reads web pages as structure, not as a text dump: landmarks, tables with spans, math, page metadata, typed entities from JSON-LD and microdata, forms with their rules and the link graph.

Status: 🗓️ planned for 0.2.0. Nothing is implemented yet.

File types: `.html`, `.htm`. Rezzer detects the format by content, not by extension.

A web page saved as one file (MHTML: `.mhtml`, `.mht`) is detected as MHTML already. Rezzer doesn't read it yet and says so in the output. It is planned together with HTML.

Built on (versions as tested on 2026-10-19):

- [scraper](https://crates.io/crates/scraper) 0.27.0 as the one document tree (html5ever with CSS selectors)
- [dom_smoothie](https://crates.io/crates/dom_smoothie) 0.18.2 to find the main content and article metadata
- Rezzer's own reader for the walk into its document model, entities, forms and the link graph

## What Rezzer extracts

### Structure

- Landmarks (main, nav, aside), sections and headings
- Lists
- Tables with row and column spans and their caption
- Figures with captions
- `details` and `summary` blocks, kept as folds
- Abbreviations with their full form, times with their machine value
- Math (MathML), with the TeX text when the page has it
- Main content marked apart from navigation and other boilerplate

### Metadata

- Title, description, author, language, canonical address
- Open Graph and Twitter card data
- Article data: byline, site name, published and modified time

### Hidden logic

- Entities from JSON-LD and microdata, with their type and relations (an article with its publisher, a blog post with its author)
- Forms: each field with type, label, required flag, pattern, min and max, options, grouped by fieldset
- Link graph: internal and external links, in-page anchors that point nowhere, the most linked external hosts
- Hidden values on tables, like a sort key per cell, when they differ from the visible text
- Scripts: listed with source or size and type, never run

### Pictures and embedded files

- Pictures are listed with their alt text and caption

## How it shows up in the output

The page becomes Markdown with its structure. After it: a `Forms` section with one table per form, an `Entities` section with a table and a Mermaid graph of relations, a `Links` summary and a `Scripts` list marked "not executed".

Example (not final):

```markdown
## Forms

| Field | Type | Label | Required | Constraint |
|---|---|---|---|---|
| plate | text | License plate | yes | pattern `[A-ZÖÄÜ]{1,3} [A-Z]{2,4} [0-9]{1,4}` |
| score | number | Score | no | 0 to 100 |
```

## Known limitations

- The main content finder works with rules. On pages that are not articles it can treat real content as boilerplate. Rezzer only marks the main region and says what it treated as boilerplate. It never drops content silently
- Scripts are listed, never run. Content that a page builds with JavaScript at load time is not there
- No public test file was found with a layout that relies heavily on definition lists (`dl`)

## Tested with

10 public test files from Wikipedia, MDN, GOV.UK, WHATWG, W3C and schema.org. See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
