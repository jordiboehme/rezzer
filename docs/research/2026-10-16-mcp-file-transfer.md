# MCP file upload and download - state of the spec, proposals, clients (2026-10-16)

Research for Rezzer: when will MCP support uploading files to a server and downloading files
from it, and how do we build now so that only an adapter changes later.

Legend: **[V]** verified on 2026-10-16 against a primary source (spec repo, schema, PR/issue
via GitHub API, official docs). **[U]** unverified: secondary source, inference, or a client
behavior we did not test ourselves. Every date is from the GitHub API or the document itself.

## TL;DR

- The current spec (2026-07-28) and the current `draft` have **no upload concept** at all. The
  `draft/schema.ts` is identical to `2026-07-28/schema.ts` except doc links [V]. Download exists
  only as base64 `blob` in `resources/read` / embedded resources, plus `resource_link` and
  `https://` resource URIs the client may fetch itself [V].
- Upload work now lives in an **experimental extension draft**:
  `modelcontextprotocol/experimental-ext-file-uploads` PR #1, opened 2026-10-14 by Den
  Delimarsky (Anthropic, core maintainer), last activity 2026-10-16, status "experimental
  draft, not yet proposed as a SEP", provisional identifier `io.modelcontextprotocol/files` [V].
- Design (stable core): server marks file arguments with `x-mcp-file`; small files go inline as
  `data:` URIs; large files: client sends `tools/call` with placeholder `mcp-file:pending` and
  metadata (name, mimeType, size, sha-256 digest) in `_meta`; server checks and returns
  `resultType: "upload_required"` with an HTTPS PUT target and an opaque file URI; client PUTs
  the bytes and retries the same call with the file URI and `requestState` (MRTR pattern) [V].
- Downloads, chunked/resumable transfer and uploads in elicitation forms are **Future Work** in
  that draft [V]. A separate Filesystems WG works on resource write operations
  (`resources/stat|create|update|delete`) [V].
- Clients today: no client implements a standard upload. ChatGPT has a proprietary equivalent
  (`_meta["openai/fileParams"]`, tool gets `{download_url, file_id, ...}`) [V]. Claude Code
  passes local paths naturally (stdio), answers `roots/list`, supports form and URL elicitation
  on its 2026-07-28 runtime, reads resources [V]. Claude.ai/Desktop users rely on the code
  sandbox plus a pre-signed URL and `curl` [U].
- **Estimate** (not a fact): official upload extension final around **Q2 2027** (range Q1 to
  Q3 2027), host support in Claude and ChatGPT in **H2 2027**, a standard download target later
  than that, core spec inclusion not before 2028.
- Recommendation: build Rezzer's file input as `path` (stdio) plus `file` (URI string with
  `x-mcp-file`, accepts `data:` and `mcp-file:` handles), an upload-session tool whose input and
  output are exactly the draft's `FileMetadata` and `UploadTarget`, and keep `rezzer://sha256/...`
  as the permanent archive address served by `resources/read`. Then the standard later only
  changes the adapter that decides where the upload target is returned.

## 1. What the current spec offers for binary data

Spec versions in the repo: 2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28, draft
(schema directory and GitHub releases, 2026-07-28 RC was tagged 2026-05-29) [V].

### 1.1 Primitives that carry bytes (2026-07-28) [V]

| Primitive | Direction | Bytes | Notes |
| --- | --- | --- | --- |
| `resources/read` -> `BlobResourceContents` | server to client | base64 `blob` + `mimeType` | Results are `CacheableResult` now: `ttlMs` and `cacheScope` required (SEP-2549) |
| `Resource.size` | metadata | - | "size of the raw resource content, in bytes (before base64 encoding)", for UI and context estimates |
| `EmbeddedResource` (`type: "resource"`) in tool results | server to client | base64 `blob` or `text` | Same content types as resources |
| `ResourceLink` (`type: "resource_link"`) in tool results | server to client | none, a URI | "not guaranteed to appear in resources/list" |
| `https://` resource URIs | server to client | client fetches from the web | Servers SHOULD use it only when the client can fetch without the MCP server |
| `ImageContent`, `AudioContent` | server to client | base64 `data` | |
| Tool `arguments` | client to server | only what fits in JSON | No file type; base64 strings are an ad hoc convention |
| Roots (`roots/list`) | client tells server about `file://` roots | none | **Deprecated** in 2026-07-28 (SEP-2577); suggested migration: "pass directories or files via tool parameters, resource URIs, or server configuration" |

- There is **no size limit, no chunking, no streaming** of resource content in the spec. The
  2026-07-28 release even removed SSE resumability (`Last-Event-ID`); a broken stream loses the
  request and the client re-issues it [V].
- 2026-07-28 changes that matter for transfers [V]:
  - Stateless protocol (SEP-2575): no `initialize`, capabilities travel per request in
    `_meta["io.modelcontextprotocol/clientCapabilities"]`; `server/discover` advertises server
    capabilities. No `Mcp-Session-Id`; cross-call state uses server-minted handles in tool
    arguments (SEP-2567). This is exactly the shape of an upload handle.
  - Multi Round-Trip Requests (SEP-2322): server returns `InputRequiredResult`
    (`resultType: "input_required"`, `inputRequests`, `requestState`), client retries with
    `inputResponses`. `inputRequests` is a closed set (`elicitation/create`,
    `sampling/createMessage`, `roots/list`); issue #2919 (open, last activity 2026-10-03) asks
    to open it for namespaced extension members.
  - `resultType` is an open string; extensions may add result types (Tasks uses `"task"`).
  - `extensions` field in client and server capabilities (SEP-2133 framework, final 2026-01-26).
  - `x-mcp-header` lets tool parameters become HTTP headers (SEP-2243).

### 1.2 Elicitation [V]

- Form mode: flat object schemas, no file field type.
- URL mode (SEP-1036, final, in spec since 2025-11-25): server sends a URL, client shows the
  domain and asks consent, user does the interaction out of band. Data other than the URL is not
  exposed to the client. In 2026-07-28 the `elicitationId` and
  `notifications/elicitation/complete` were removed; the client learns the outcome by retrying
  the original request (MRTR), and the server correlates through `requestState`.
- This makes **URL-mode elicitation a standard way to get a human to upload a file today**: the
  URL points at the server's own upload page.

### 1.3 Tasks [V]

- Tasks moved out of core into the official extension `io.modelcontextprotocol/tasks`
  (SEP-2663, final 2026-05-15): `tasks/get` polling, `tasks/update`, unsolicited task handles.
- The upload draft says a server using Tasks MUST return `upload_required` before it creates a
  task. So: upload first, then the long conversion can run as a task.
- Issue #3237 (open, 2026-08-14): no interoperable surface for partial output of a running task.

### 1.4 MCP Apps (`io.modelcontextprotocol/ui`, SEP-1865 final 2026-01-28) [V]

- `ui/download-file`: an app view asks the host to save a file. `contents` takes an
  `EmbeddedResource` (inline `text` or `blob`) or a `ResourceLink` (host fetches the URI). Host
  capability `downloadFile`. Known gap: not implemented on Claude iOS (ext-apps #574, open).
- No upload method in MCP Apps. Views run in sandboxed iframes; a view can upload to a
  server-issued signed URL if the domain is in `_meta.ui.csp.connectDomains` [V for CSP field,
  U for host behavior].
- SEP-3118 "App-Rendered Elicitations for MCP Apps" (open, 2026-07-30, no sponsor yet) would let
  an app render an elicitation form; it is a possible future path for a file picker UI.

### 1.5 Draft spec

`schema/draft/schema.ts` differs from `schema/2026-07-28/schema.ts` only in documentation links
(0 non-link diff lines on 2026-10-16) [V]. Nothing file-related is merged into the draft.

## 2. Proposals, SEPs, issues and working groups

### 2.1 Active

| Item | Status (2026-10-16) | Last activity | People | Design |
| --- | --- | --- | --- | --- |
| **experimental-ext-file-uploads PR #1** "Add initial file uploads extension draft" | Open PR, experimental draft, not a SEP yet; repo created 2026-10-14 | 2026-10-16 | Editor Den Delimarsky (@localden, Anthropic, core maintainer). CODEOWNERS: file-uploads-wg, core-maintainers | `x-mcp-file` + `data:` inline + placeholder/`_meta` metadata + `upload_required` with PUT target + MRTR retry. Details in 2.3 [V] |
| **File Uploads WG** charter | Active, charter 2026-04-23 (anchored on SEP-2356, never updated) | Meeting 2026-10-02 (discussion #3412) | Lead @localden; sponsors @localden and Nick Cooper (@nickcoai, OpenAI); member Olivier Chafik (@ochafik, Anthropic). Oct 2 attendees also Kurtis Van Gent (@kurtisvg, core maintainer), Thomas Sarlandie (@sarfata), Wil St. Charles, Kryspin Ziemski | Weekly meetings since Oct 2. Decisions: extension first, nothing into core "until it has been validated in use"; metadata before bytes; build on MRTR; v0 is one HTTP PUT (about 250 MB); resumable later behind a capability; auth must allow both bearer token and pre-signed URL [V] |
| **Filesystems WG** charter | Active, charter 2026-08-24 | Repo experimental-ext-filesystems created 2026-10-07; PRs #1-#3 open, 2026-10-14 | Leads Sambhav Kothari, Daniel Temesgen (Bloomberg), Ola Hungerford (Nordstrom) | Make Resources bidirectional: `resources/stat`, `create` (create-if-absent), `update` (`ifMatch`), `delete`; opaque `version` for concurrency; listing reuses SEP-2640 `resources/directory/read`; range reads deferred. Explicitly no parallel `files/*` primitive [V] |
| **PR #3389** merge both WGs into one "Files WG" | **Closed unmerged 2026-10-11**: "we're going to abandon this ... and keep the WGs separate for now" (@localden) | 2026-10-11 | - | The Oct 2 notes still say "One Files WG"; that is outdated. The two groups stay separate [V] |
| **SEP-2532** Resource Streaming for Binary Content Delivery (PR) | Open, label `proposal`, **no sponsor** ("seeking sponsor") | 2026-07-21 (author), cited in WG discussion 2026-10-05 | Patrick Rodgers (Microsoft) | New `resources/stream`: server answers with raw bytes or with a `downloadUrl` the client GETs with the MCP credentials; HTTP only, stdio out of scope. Named as prior art for downloads in the upload draft [V] |
| **Issue #2919** MRTR `inputRequests` is a closed enum | Open | 2026-10-03 | ferentinai; +1 from NimbleBrain | Allow namespaced members in `inputRequests`. Open question 1 of the upload draft depends on it [V] |
| **PR #2527** Recommend clients expose resource read to models | Open, assignee dsp (David Soria Parra) | 2026-09-04 | Peter Alexander (@pja-ant, core maintainer) | SHOULD give the model a way to read a resource by URI. Relevant for `rezzer://` URIs [V] |
| **Issue #155** Passing attachments (resources) to MCP servers | Open since 2025-02-01 | 2025-12-20 | - | The original user request [V] |
| **Issue #527** Base64 in JSON-RPC will not scale | Open since 2025-05-14 | 2025-10-03 | Sam Morrow (GitHub) | Asks for streaming or download URLs [V] |

### 2.2 Closed or superseded

| Item | Status | Closed | Design and why it ended |
| --- | --- | --- | --- |
| SEP-1306 Binary Mode Elicitation for File Uploads (issue, @localden) | draft, closed | 2026-03-12 | Third elicitation mode `binary`. "Superseded by #2356" [V] |
| SEP-2356 File input support for tools and elicitation (PR, @ochafik) | `in-review`, closed unmerged, sponsor @localden | 2026-06-26 | `x-mcp-file` / `FileInputDescriptor {accept, maxSize}`, `fileInputs` client capability, RFC 2397 `data:<mime>;name=<pct-encoded>;base64,...`, error reason `file_too_large`; larger files via URL elicitation. Closed "in favor of #2631". Prototypes: typescript-sdk#1633, python-sdk#2217 [V] |
| SEP-2631 File Objects and Transfer (PR, Casey Chow, OpenAI) | closed unmerged, assignee @localden | 2026-10-14 | Built on 2356: `files/authorizeUpload` and `files/authorizeDownload` methods returning HTTPS transfer descriptors, `FileValue` outputs, `transferModes`, digests. Closed in favor of the extension because bytes moved before the server saw the call (quota, permissions, data residency). File URIs, digests and transfer descriptors were reused [V] |
| SEP-2571 Resource Submission (`resources/create`, `resources/delete`) | closed | 2026-09-22 | Closed because new SEPs must come through a WG; folded into Filesystems WG input [V] |
| SEP-2166 Out-of-Band Resource Access via HTTPS URLs | closed | 2026-08-18 | Optional `httpUrl` and `httpUrlExpiresAt` on `Resource` for sandboxes without an MCP client [V] |
| SEP-2433 Transfer Descriptors - Out-of-Band Data Transfer Negotiation | closed | 2026-06-15 | Multi-protocol negotiation (HTTP, S3, BitTorrent ...) with LLM-generated transfer code [V] |
| SEP-1708 MCP Client-Brokered Filesystem Access (issue) | closed | 2026-01-23 | `files/*` family in the client. A comment from 2026-09-15 notes that with Roots deprecated there is no mechanism for auto-approved filesystem access [V] |
| Issue #189 Resumable uploads via tus | closed | 2025-03-14 | Then: "File uploads are not really a part of MCP" (jspahrsummers) [V] |
| Issue #1049 How to pass images from client to server | closed | 2025-07-31 | Answer: upload to object storage, pass URL [V] |

### 2.3 The upload extension draft in detail [V]

Source: `specification/draft/file-uploads.md` at head `de0661f` of
experimental-ext-file-uploads PR #1.

- **Negotiation**: client declares `{"extensions": {"io.modelcontextprotocol/files": {}}}` in
  per-request `clientCapabilities`; server advertises in `server/discover`, for example
  `{"io.modelcontextprotocol/files": {"maxInlineBytes": 1048576}}`. Clients MUST NOT send a
  placeholder to a server that did not advertise it. Oversized `data:` is a tool error
  (SEP-1303); `MissingRequiredClientCapabilityError` (-32021) is allowed.
- **File argument**: `{"type": "string", "format": "uri", "x-mcp-file": {"accept": [...],
  "maxSize": n}}`, also in array items. `maxSize` applies to every transfer path.
- **Allowed values**: an RFC 2397 `data:` URI; a placeholder starting with `mcp-file:pending`
  (suffix like `mcp-file:pending:2` for arrays); or a file URI the server issued. **Servers MUST
  reject any other value and MUST NOT dereference it.** Clients treat file URIs as opaque.
  Servers SHOULD mint them under `mcp-file:`.
- **Host rules**: SHOULD NOT show `x-mcp-file` properties to a model that emits arguments as
  tokens; bind files only on user selection or host policy; never put bytes, upload URLs or
  issued file URIs in model context.
- **Initial call**: all arguments filled, placeholders in file arguments, and
  `_meta["io.modelcontextprotocol/files"].pending` keyed by JSON Pointer (`/file`, `/files/2`)
  with `FileMetadata {name?, mimeType, size, digest?: {algorithm: "sha-256", value: base64url
  without padding}}`. Server runs all checks that do not need bytes and may reject with
  `isError: true`.
- **`upload_required` result**: `{resultType: "upload_required", uploads: {"/file":
  UploadTarget}, requestState}`; `UploadTarget {uri, url (absolute https), headers?,
  credentials: "omit" | "include"}`. `requestState` binds tool name, arguments and file URIs.
- **Transfer**: one PUT per target, 2xx is success, no redirects, no private addresses, no
  `Authorization`/`Cookie` in `headers`; `include` only for the MCP server's own canonical
  origin and path. On stdio only `omit` targets apply.
- **Retry**: same tool and arguments with placeholders replaced by file URIs, plus
  `requestState`. Server verifies state, confirms bytes at the storage (HEAD or session query),
  checks digest/size/type, finalizes, runs the tool. File URIs are single use.
- **Open questions** (in flux): (1) own result type vs an input request in `InputRequiredResult`
  (needs #2919); (2) mandatory digest; (3) auth methods (separate storage credential, HTTP
  Message Signatures); (4) finalize step; (5) how the typed result reaches the model; (6) stdio;
  (7) whether Resources fit this flow; (8) whether `mcp-file:` is required; (9) multipart and
  POST.
- **Review feedback so far** (2026-10-14 to 2026-10-16): Dropbox asks for POST next to PUT and
  for chunked/resumable sessions; @ochafik suggests a dedicated upload MCP method instead of
  HTTP PUT and listing file inputs in `Tool._meta` because a custom keyword may trip strict
  JSON Schema validators; OpenAI (tsarlandie-oai) suggests the file argument be an object with
  required `uri`, `size`, `digest` and no custom scheme; others ask for `expiresAt` on
  `UploadTarget`, idempotent retries, host-side handles (`files://<id>`) the model can name, and
  support for gateways that hide inner tool schemas (Arcade, Cloudflare Code Mode, Docker).
  So the **shape of the file argument and the result type are not settled**; the flow order is.

## 3. What clients support today

| Client | User file to server | Server file to user | Source |
| --- | --- | --- | --- |
| **Claude Code** | stdio servers read local paths directly. Answers `roots/list` with launch dir plus `--add-dir` dirs and sends `roots/list_changed` [V]. On the 2026-07-28 (v2) runtime it declares `elicitation: {form: {}, url: {}}` [V]. No `x-mcp-file` support [V, not in docs]. | `@server:uri` mentions attach resources; built-in tools list and read MCP resources [V]. Images in tool results shown inline and saved to a file [V]. Text over 50,000 chars (or over `MAX_MCP_OUTPUT_TOKENS`, default 25,000) is saved to a file and replaced by its path [V]. `resource_link` handling: an issue said it was ignored (#53453, closed as stale 2026-05-29, not fixed) [U]. | code.claude.com/docs/en/mcp |
| **Claude.ai / Claude Desktop connectors** | No standard path. Pattern used in the wild: attached file lands in the code-execution sandbox, a tool returns a pre-signed URL, Claude runs `curl -X PUT`; requires the user to allow the domain under "Additional allowed domains" [U, futuresearch.ai blog 2026-02-25/03-30]. | MCP Apps supported, including `ui/download-file` on desktop/web [V matrix, U per surface]. Binary embedded resources from remote connectors failed schema validation (claude-code #94746, closed 2026-10-06 as completed; reports on #93946, open, as late as 2026-09-23 on Desktop) [V issue state, U current behavior]. | MCP extension client matrix; issues |
| **ChatGPT** | Proprietary: tool lists file fields in `_meta["openai/fileParams"]`; each field is an object `{download_url, file_id, mime_type?, file_name?}` (all four declared, first two required); server fetches `download_url` (temporary bearer URL). Widgets: `window.openai.uploadFile`, `selectFiles`, `getFileDownloadUrl` [V]. | MCP Apps supported [V]. `getFileDownloadUrl` also works for "tool file references" [V]. | developers.openai.com/apps-sdk/reference |
| **VS Code (Copilot)** | Roots (workspace folders), elicitation, MCP Apps; no file input type [V]. | Resources with text or binary content, attachable as chat context, live updates, templates [V]. | vscode-docs api/extension-guides/ai/mcp.md |
| **Cursor** | Roots, elicitation, Apps "Supported" [V]. No file input type [U]. | Resources "Supported" [V]. | cursor.com/docs/context/mcp |

### Workarounds servers use today

1. **Local path in a tool argument** (stdio only). Simple, no bytes in context. Claude Code,
   Cursor, VS Code [V for the general mechanism].
2. **Base64 in tool arguments**. Works for small files, but the model re-emits every byte as
   tokens. Real failure: redmine-mcp-server #305, a 3,232-byte PNG arrived with 4,280 instead of
   4,312 base64 characters, still valid base64, stored corrupt (reported on SEP-2631,
   2026-10-02) [V report, U root cause].
3. **Pre-signed upload URL plus a tool**: tool returns `{upload_url, upload_id, expires_in,
   max_size_bytes}`, client or sandbox PUTs, later tools take the ID (futuresearch, redmine
   v2.16.0, sunpeak pattern with `create_upload` / `finalize_upload`) [U].
4. **Host file APIs**: ChatGPT `openai/fileParams` [V]; NimbleBrain `ai.nimblebrain/host-resources`
   (server reads host-owned `files://<id>` by URI) [U].
5. **URL-mode elicitation to the server's own upload page** (standard since 2025-11-25, MRTR
   retry since 2026-07-28) [V spec, U adoption].
6. **Shared folder / server config** (a mounted volume both sides see) [U].
7. **Downloads**: `resources/read` blob, `resource_link`, `https://` resource URIs, MCP Apps
   `ui/download-file`, signed GET URLs in tool results [V spec, U adoption].

## 4. Estimate: when is standardized upload and download likely

**This section is an estimate, not a verified fact.**

Inputs [V]:

- Spec release gaps: 2024-11-05 -> 2025-03-26 (4.6 months) -> 2025-06-18 (2.8) -> 2025-11-25
  (5.2) -> 2026-07-28 (8.1). RCs about two months before the release.
- The extension framework (SEP-2133) lets official extensions ship independently of spec
  releases. From SEP PR to final: Tasks 16 days (2026-04-29 -> 2026-05-15), MCP Apps 2 months
  (2025-11-21 -> 2026-01-28), Skills 5 months (2026-04-23 -> 2026-09-13).
- The upload draft is not yet a SEP; it needs a TypeScript reference implementation, a reference
  server and conformance scenarios first. It has 9 open questions and contested basics (argument
  shape, result type, PUT vs a protocol method).
- The roadmap (last updated 2026-08-22) mentions file uploads in one line under "Improved
  Primitives", not as a priority deliverable. SEP-2356 missed its charter target ("End May").
- Pressure is high: vendors on SEP-2631 asked for it repeatedly (2026-08-19, 09-25, 09-29), and
  both Anthropic and OpenAI people drive it. ChatGPT already has a proprietary version.
- Rust SDK: rmcp 3.5.1 (2026-10-05) implements 2026-07-28 and MRTR.

Estimate:

| Milestone | Likely | Range | Confidence |
| --- | --- | --- | --- |
| Experimental draft merged plus TS reference implementation | Q4 2026 | Nov 2026 - Feb 2027 | medium |
| Extensions Track SEP for uploads filed | Q1 2027 | Dec 2026 - Apr 2027 | medium |
| Upload extension final (official) | **Q2 2027** | Q1 - Q3 2027 | low-medium |
| Claude and ChatGPT hosts support it | **H2 2027** | Q2 2027 - 2028 | low |
| rmcp support | shortly after final; we can implement against the wire format earlier | - | medium |
| Standard download target (mirror of `UploadTarget`) | late 2027 or later | - | low |
| Resource write operations (Filesystems WG) | 2027 | - | low |
| Upload in the core spec | not before 2028 | - | low |

Download is less urgent for Rezzer: `resources/read` with `blob`, `resource_link` and Apps
`ui/download-file` already work in the standard today; only large out-of-band downloads lack a
standard.

## 5. Recommendation for Rezzer

### 5.1 Principles

- **Two URI namespaces, never mixed.**
  - Upload handle: `mcp-file:<ulid>`. Opaque, single use, short-lived, issued by Rezzer. It
    matches the draft's recommended scheme, so it stays valid later.
  - Archive address: `rezzer://sha256/<hex>/<name>`. Permanent, content-addressed, cited in
    frontmatter. Never put it in an `x-mcp-file` argument: the draft says servers MUST reject
    anything that is not `data:`, a placeholder or an issued file URI.
- **Metadata first, then bytes, then confirm, then run** (the settled part of the draft).
- **Upload ends in the archive.** After bytes are confirmed (size, sha-256), Rezzer ingests them
  into the source archive and from then on only the `rezzer://` URI is used.
- Roots are deprecated: the stdio path allowlist comes from Rezzer config. Reading `roots/list`
  where offered is a convenience, never a dependency.

### 5.2 Internal abstraction (works today, maps 1:1 later)

```rust
/// How a source file reaches Rezzer. Resolved once, then everything uses ArchivedSource.
enum SourceInput {
    LocalPath(PathBuf),                  // stdio: `path` argument, checked against config allowlist
    Inline { bytes: Vec<u8>, name: Option<String>, mime: String }, // `data:` URI in `file`
    Staged(UploadHandle),                // `mcp-file:<ulid>` in `file`, bytes already PUT
    HostFetch { url: Url, name: Option<String>, mime: Option<String> }, // ChatGPT fileParams adapter
    Archived(ArchiveUri),                // `source` argument: re-convert an archived file
}

struct FileMetadata {           // exactly the draft's shape
    name: Option<String>,
    mime_type: String,          // "application/octet-stream" if unknown
    size: u64,
    digest: Option<Digest>,     // { algorithm: "sha-256", value: base64url, no padding }
}

struct UploadTarget {           // exactly the draft's shape, plus expiresAt (requested in review)
    uri: String,                // "mcp-file:01JD..."
    url: Url,                   // https PUT target
    headers: BTreeMap<String, String>,
    credentials: Credentials,   // Omit | Include
    expires_at: DateTime<Utc>,
}

trait UploadStore {             // one implementation, three front-ends (tool, elicitation page, upload_required)
    fn authorize(&self, call: &CallContext, meta: &FileMetadata) -> Result<UploadTarget, Reject>;
    fn confirm(&self, handle: &UploadHandle, meta: &FileMetadata) -> Result<StagedFile, Pending>;
    fn consume(&self, handle: &UploadHandle) -> Result<StagedFile, Consumed>; // single use
}
```

The `UploadStore` is the stable core. Only the front-end that hands out `UploadTarget` changes:

| Phase | Front-end | Transport |
| --- | --- | --- |
| Today, stdio | none, `path` argument | stdio |
| Today, HTTP, agent with a sandbox | tool `upload_begin` returns `UploadTarget`; agent PUTs; `convert(file="mcp-file:...")` | streamable HTTP |
| Today, HTTP, human present | `convert` with no file returns `InputRequiredResult` with URL-mode `elicitation/create` to Rezzer's upload page; `requestState` carries the handle; retry confirms | streamable HTTP |
| Today, HTTP, ChatGPT | optional `chatgpt_file` object argument in `openai/fileParams`; server fetches `download_url` (allowlist the host) | streamable HTTP |
| Later, standard | `convert(file="mcp-file:pending")` + `_meta` pending -> `upload_required` from the same `authorize()`; advertise `io.modelcontextprotocol/files` in `server/discover` | streamable HTTP, stdio with `omit` targets |

Hex vs base64url: the archive keys by hex sha-256, the draft uses base64url without padding.
Convert at the boundary; it is the same 32 bytes.

### 5.3 Concrete shapes

**Tool `convert`** (name is a placeholder for Rezzer's main tool):

```json
{
  "name": "convert",
  "inputSchema": {
    "type": "object",
    "properties": {
      "path":   { "type": "string", "description": "Local file path. stdio mode only." },
      "file":   { "type": "string", "format": "uri",
                  "x-mcp-file": { "accept": [".pdf", ".docx", "application/pdf"], "maxSize": 268435456 },
                  "description": "data: URI or an mcp-file: handle from upload_begin." },
      "source": { "type": "string", "format": "uri",
                  "description": "rezzer://sha256/<hash>/<name> of an archived source." }
    }
  }
}
```

- All three optional; exactly one required, checked at runtime (`isError: true` otherwise).
- In HTTP mode do not list `path`. In stdio mode `file` is still allowed (inline `data:`).
- Accept `data:<mime>;name=<pct-encoded>;base64,...` (SEP-2356 form). Cap inline size low (for
  example 1 MiB, same as the draft example `maxInlineBytes`), because model-emitted base64 can
  be corrupted silently.
- Watch the open question about `x-mcp-file` inside `inputSchema`: if strict validators reject
  it, the WG may move it to `Tool._meta`. Keep the list of file arguments in one place in code so
  emitting it in either location is a one-line change.

**Tool `upload_begin`** (workaround today; later used internally by `upload_required`):

- Input: `FileMetadata` (`name`, `mimeType`, `size`, `digest`), plus optional `purpose`.
- Output (`structuredContent`): `UploadTarget` (`uri`, `url`, `headers`, `credentials:
  "omit"`, `expiresAt`), plus `maxSize`.
- HTTP endpoint: `PUT /uploads/<ulid>?sig=...` on Rezzer's own origin, raw body, enforce
  declared size and digest, stage only, TTL 15 minutes, single use. POST with raw body may be
  accepted too (Dropbox review point).
- Never echo the URL into `content` text when a host can do the PUT itself; for sandbox agents
  (Claude.ai) the URL must be visible to the model, which is the main reason this stays a
  workaround.

**Download (`source` tool and resources)**:

- Resource template `rezzer://sha256/{hash}/{name}` in `resources/templates/list`;
  `resources/read` returns `BlobResourceContents {uri, mimeType, blob}`, `ttlMs` long (content is
  immutable), `cacheScope: "private"`. Set `Resource.size`.
- `source` tool modes: `path` (stdio, local archive path), `copy` (stdio, copy to a target path),
  `content` (embedded resource for small files), `link` (return a `resource_link` to the
  `rezzer://` URI). For large files over HTTP add `download`: return a `DownloadTarget`-like
  object `{url, headers, credentials, expiresAt}` mirroring `UploadTarget` (signed GET on
  Rezzer's origin), because the draft names "a download target that mirrors UploadTarget" as
  future work.
- For an MCP Apps UI later: `ui/download-file` with a `resource_link`.

### 5.4 Caveats

- **rmcp 3.5.1 cannot return `upload_required` from a normal tool handler** [V source]:
  `ServerHandler::call_tool` returns `CallToolResponse`, a closed enum `Complete | InputRequired
  | Task`. `ResultType` itself is an open string and `ServerResult` has a `CustomResult(Value)`
  variant, so a lower-level request handler or an rmcp change (new variant) is needed. Capability
  plumbing exists: `ServerCapabilities.extensions` and `ClientCapabilities.extensions`
  (`BTreeMap<String, JsonObject>`), and `RequestContext::client_capabilities()` reads the
  per-request `_meta` [V]. Plan an upstream PR or a thin custom handler when the extension
  stabilizes.
- rmcp open issue #1321 (2026-10-06): on streamable HTTP under 2026-07-28, client responses to
  server-initiated requests are discarded. MRTR (`InputRequiredResult` and retry) is the
  supported path; avoid server-initiated `elicitation/create` [V issue, U impact on Rezzer].
- Do not advertise `io.modelcontextprotocol/files` before the extension is final; the
  identifier is provisional.
- Still in flux, keep behind the adapter: own result type vs MRTR input request; where
  `x-mcp-file` lives and whether the argument becomes an object `{uri, size, digest}`; whether
  `mcp-file:` is required; PUT vs POST vs an upload MCP method; auth methods.

## Sources

Spec and repo (GitHub API, 2026-10-16):

- Spec 2026-07-28 changelog, resources, tools, elicitation, roots; `schema/2026-07-28/schema.ts`,
  `schema/draft/schema.ts` - github.com/modelcontextprotocol/modelcontextprotocol
- Roadmap (2026-08-22): docs/development/roadmap.mdx
- WG charters: docs/community/working-groups/file-uploads.mdx, filesystems.mdx
- Extension client matrix: docs/extensions/client-matrix.mdx
- Discussion #3412 "Files Working Group Meeting - October 2, 2026"
- PRs: #2356, #2631, #2532, #2571, #2166, #2433, #2527, #3118, #3389, #1865, #2663, #2640,
  #2133, #2322, #2575; issues #1306, #1708, #155, #527, #1049, #189, #2919, #3237
- github.com/modelcontextprotocol/experimental-ext-file-uploads PR #1
  (`specification/draft/file-uploads.md` at `de0661f`)
- github.com/modelcontextprotocol/experimental-ext-filesystems PRs #1-#3
- github.com/modelcontextprotocol/ext-apps `specification/draft/apps.mdx`, issue #574
- github.com/modelcontextprotocol/rust-sdk tag rmcp-v3.5.1 (`model/mrtr.rs`,
  `model/capabilities.rs`, `service.rs`), issue #1321

Clients:

- code.claude.com/docs/en/mcp (fetched 2026-10-16)
- developers.openai.com/apps-sdk/reference (markdown, fetched 2026-10-16)
- microsoft/vscode-docs api/extension-guides/ai/mcp.md
- cursor.com/docs/context/mcp
- anthropics/claude-code issues #53453, #93946, #94746

Secondary [U]:

- sunpeak.ai/blogs/file-uploads-mcp-apps/ (2026-09-13)
- futuresearch.ai/blog/mcp-large-dataset-upload/ (2026-02-25, updated 2026-03-30)
