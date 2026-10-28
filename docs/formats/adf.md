# ADF

Rezzer reads Atlassian Document Format, the JSON tree that Jira Cloud and Confluence Cloud use for rich text. An agent that gets a Jira issue or a Confluence page through the API gets this tree. Rezzer turns it into Markdown with its structure and metadata.

Status: 📝 planned. The release is not set yet. Nothing is implemented yet. The request is [issue 22](https://github.com/jordiboehme/rezzer/issues/22).

File types: `.json` files that hold an ADF document. Rezzer detects the format by content: a JSON object with `"type": "doc"`, a `version` and a `content` list. A single node or a list of comments needs a wrapper rule, which the spec decides.

Built on: Rezzer's own reader. ADF has a public JSON schema, and the reader maps each node type into Rezzer's document model.

## Where the files come from

- Jira Cloud REST API v3: issue descriptions, comments and worklog comments
- Confluence Cloud REST API v2: page bodies requested as `atlas_doc_format`
- Jira webhooks and Jira Cloud JSON exports
- Atlassian MCP servers and other API wrappers that pass the raw ADF on

## What Rezzer will extract

- Headings, paragraphs, lists, task lists, quotes and rules
- Tables, with header cells and merged cells
- Code blocks with their language
- Panels (info, note, warning, error, success) as callouts
- Links, mentions, emoji, dates and status labels
- Expand sections and layouts
- Media nodes: each attachment is listed with its id, name and place in the text
- Comment author and date, when they come with the ADF body

## Limits

- ADF holds no media files, only their ids. Downloading attachments through the Atlassian API needs credentials and network access. Whether and how Rezzer does that is decided in the spec for this format. Credentials never go on the command line.
- Node types Rezzer does not know are listed under "Not read" with their type. They are never dropped silently.
