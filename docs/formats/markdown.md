# Markdown

Rezzer reads Markdown files, keeps their own frontmatter and adds an outline and a graph of the links, including wikilinks.

Status: ✅ available in the development version. It is part of 0.1.0, the first release.

File types: `.md`, `.markdown`. Rezzer detects the format by content, not by extension.

Built on (versions as tested on 2026-10-19):

- [pulldown-cmark](https://crates.io/crates/pulldown-cmark) 0.13.5 for the structure, with source positions, YAML and TOML frontmatter blocks and wikilinks
- Rezzer's own reader for the frontmatter values and the link table

## What Rezzer reads

### Structure

- Heading outline with the position in the source file
- Lists, tables, code blocks

### Metadata

- The existing frontmatter, YAML (`---`) or TOML (`+++`), kept as it is
- The values in it, so Rezzer can use them

### Hidden logic

- Links by kind: inline, reference, autolink, wikilink; internal, external or anchor
- Whether each link target can be found
- Wikilinks as relations. They are kept in the link graph. In the body they are written as plain text, because Rezzer does not know the name of the linked note in the output. In the Markdown file this looks like `\[\[graph-view\]\]`. Wikilinks inside code stay as they are
- Frontmatter keys that point to other pages, like aliases or redirects

## How it shows up in the output

The original frontmatter stays as it is. Rezzer's own keys are added after it. The links of the file show up as a Mermaid graph in the Logic section.

## Example

Real output of `rezzer read` for `md-wikilinks-foam-docs.md` from the test corpus. The overview and the Logic section; the text between them is cut:

````markdown
## Overview

- Graphs: 1
- Links: 5

## Logic

### Graph: links

Kind link, 6 nodes, 5 edges.
```mermaid
flowchart LR
    n_self["Wikilinks"]
    n_graph_view["graph-view"]
    n_block_anchors["block-anchors"]
    n_link_reference_definitions["link-reference-definitions"]
    n_footnotes["footnotes"]
    n_templates["templates"]
    n_self -->|"links_to"| n_graph_view
    n_self -->|"links_to"| n_block_anchors
    n_self -->|"links_to"| n_link_reference_definitions
    n_self -->|"links_to"| n_footnotes
    n_self -->|"links_to"| n_templates
```
````

## Known limitations

- Reading the frontmatter values needs a YAML and a TOML parser. These are not chosen or tested yet
- Liquid or other template tags in the text are kept as text, not run

## How it is tested

The corpus has 3 public Markdown files from GitHub Docs, the Rust blog and Foam. See [MANIFEST.md](../../tests/corpus/MANIFEST.md).

Extreme cases the test suite holds, at the default limits unless a limit is named (the figures are the bounds the tests check):

- 100,000 levels of block quotes: the read stops at the depth limit and says so
- 1.2 million emphasized words: the read stops at 2 million elements and says so
- Bytes that are not valid UTF-8: decoded, the text around them is kept

Speed: `md-toml-frontmatter-rust-blog-traits.md` (18 KB) reads in under 0.01 seconds with a peak memory of 5 MiB. Median of 3 runs with `--no-cache`, on an Apple M5 (Mac17,3, 32 GB, load average 3.2), release build (cargo profile release) of Rezzer 0.1.0 at commit 958df97, 2026-10-21.
