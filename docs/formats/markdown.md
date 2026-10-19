# Markdown

Rezzer reads Markdown files, keeps their own frontmatter and adds an outline and a table of the links, including wikilinks.

Status: planned for the first release. Nothing is implemented yet.

File types: `.md`, `.markdown`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [pulldown-cmark](https://crates.io/crates/pulldown-cmark) 0.13.4 for the structure, with source positions, YAML and TOML frontmatter blocks and wikilinks
- Rezzer's own reader for the frontmatter values and the link table

## What Rezzer extracts

### Structure

- Heading outline with the position in the source file
- Lists, tables, code blocks

### Metadata

- The existing frontmatter, YAML (`---`) or TOML (`+++`), kept as it is
- The values in it, so Rezzer can use them

### Hidden logic

- Links by kind: inline, reference, autolink, wikilink; internal, external or anchor
- Whether each link target can be found
- Wikilinks as relations. Wikilinks inside code are ignored
- Frontmatter keys that point to other pages, like aliases or redirects

## How it shows up in the output

The original frontmatter stays as it is. Rezzer's own keys are added after it. A `Links` table lists target, kind and whether the target was found.

## Known limitations

- Reading the frontmatter values needs a YAML and a TOML parser. These are not chosen or tested yet
- Liquid or other template tags in the text are kept as text, not run

## Tested with

3 public test files from GitHub Docs, the Rust blog and Foam. See [MANIFEST-web-text.md](../spike-2026-10-19/MANIFEST-web-text.md).
