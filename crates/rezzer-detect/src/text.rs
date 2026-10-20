// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Text sniffing: decide whether the bytes are text and which text format they hold.
//!
//! Only the first [`SNIFF_BYTES`] are decoded. Order: Markdown frontmatter, the structured
//! formats, G-code and OpenSCAD, Markdown by syntax, then CSV (structured formats often split
//! into equal field counts too), then the tie-breaks: a declared MIME type first, then the
//! extension. A MIME type also raises the confidence of a matching guess and lowers it when
//! it names another table or Markdown format; it never replaces content evidence.

use rezzer_model::{Confidence, FormatId};

use crate::{Found, clean, extension, found};

/// Bytes decoded for sniffing. The reader decodes the whole file later.
const SNIFF_BYTES: usize = 1024 * 1024;
/// Lines looked at by the line-based checks.
const HEAD_LINES: usize = 200;
/// Records the extension fallback for CSV and TSV looks at.
const CSV_FALLBACK_RECORDS: usize = 200;

/// Returns None when the bytes are not text. `mime` is a normalized MIME type (no
/// parameters, lowercase).
pub(crate) fn detect(name: &str, mime: Option<&str>, bytes: &[u8]) -> Option<Found> {
    let truncated = bytes.len() > SNIFF_BYTES;
    let sample = sample(bytes);
    let decoded = textenc::decode(sample);
    let text = decoded.text.as_str();

    let (chars, bad) = text.chars().fold((0usize, 0usize), |(n, bad), c| {
        let is_bad = (c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0C' | '\x1B'))
            || c == '\u{FFFD}';
        (n + 1, bad + usize::from(is_bad))
    });
    if chars == 0 || bad * 100 >= chars {
        return None;
    }

    // Cut a partial last line, so a cut-off record does not distort the line checks.
    let text = if truncated {
        text.rfind('\n').map_or(text, |i| &text[..=i])
    } else {
        text
    };
    let enc = decoded.encoding;
    let base = if decoded.confident {
        Confidence::Likely
    } else {
        Confidence::Guess
    };
    let hint = mime.and_then(|m| mime_format(m).map(|f| (f, m)));
    Some(adjust(classify(name, hint, text, enc, base), hint))
}

/// The text format a MIME type names, if it names one Rezzer sniffs for. `text/plain` names
/// none: it is the default of data: URIs and says nothing about the content.
fn mime_format(mime: &str) -> Option<FormatId> {
    match mime {
        "text/csv" | "application/csv" => Some(FormatId::Csv),
        "text/tab-separated-values" => Some(FormatId::Tsv),
        "text/markdown" | "text/x-markdown" => Some(FormatId::Markdown),
        _ => None,
    }
}

/// A MIME type that agrees with a guessed table or Markdown format raises it to likely; one
/// that names another of these formats lowers a likely result to a guess. The format itself
/// never changes.
fn adjust(found: Found, hint: Option<(FormatId, &str)>) -> Found {
    let (format, mut detection) = found;
    let hinted = [FormatId::Csv, FormatId::Tsv, FormatId::Markdown];
    if let Some((hint, mime)) = hint
        && hinted.contains(&format)
    {
        if hint == format && detection.confidence == Confidence::Guess {
            detection.confidence = Confidence::Likely;
            detection
                .evidence
                .push_str(&format!("; MIME type {mime} agrees"));
        } else if hint != format && detection.confidence == Confidence::Likely {
            detection.confidence = Confidence::Guess;
            detection
                .evidence
                .push_str(&format!("; MIME type {mime} disagrees"));
        }
    }
    (format, detection)
}

/// Takes the first [`SNIFF_BYTES`] and trims a character cut in half at the end.
fn sample(bytes: &[u8]) -> &[u8] {
    if bytes.len() <= SNIFF_BYTES {
        return bytes;
    }
    let cut = &bytes[..SNIFF_BYTES];
    let utf16 = cut.starts_with(&[0xFF, 0xFE]) || cut.starts_with(&[0xFE, 0xFF]);
    if utf16 {
        return &cut[..cut.len() & !1];
    }
    match std::str::from_utf8(cut) {
        Err(e) if e.error_len().is_none() => &cut[..e.valid_up_to()],
        _ => cut,
    }
}

fn classify(
    name: &str,
    hint: Option<(FormatId, &str)>,
    text: &str,
    enc: &str,
    base: Confidence,
) -> Found {
    let body = text.trim_start_matches('\u{FEFF}');
    let trimmed = body.trim_start();
    let lines: Vec<&str> = body.lines().take(HEAD_LINES).collect();

    if let Some(f) = markdown_frontmatter(&lines) {
        return f;
    }
    if let Some(f) = gltf(trimmed) {
        return f;
    }
    if json_like(trimmed) {
        return found(
            FormatId::Text,
            base,
            format!("text ({enc}), json-like structure (no JSON reader, kept as text)"),
        );
    }
    if let Some(f) = html(trimmed) {
        return f;
    }
    if let Some(f) = eml(&lines) {
        return f;
    }
    if let Some(f) = mbox(&lines) {
        return f;
    }
    if trimmed.starts_with("(kicad_sch") {
        return found(
            FormatId::KicadSch,
            Confidence::Certain,
            "s-expression (kicad_sch",
        );
    }
    if trimmed.starts_with("(kicad_pcb") {
        return found(
            FormatId::KicadPcb,
            Confidence::Certain,
            "s-expression (kicad_pcb",
        );
    }
    if let Some(f) = gerber(&lines) {
        return f;
    }
    if let Some(f) = excellon(&lines) {
        return f;
    }
    if let Some(f) = obj(&lines) {
        return f;
    }
    if let Some(f) = gcode(body) {
        return f;
    }
    if let Some(f) = openscad(body) {
        return f;
    }
    if let Some(f) = markdown_syntax(name, &lines) {
        return f;
    }
    if let Some(d) = csvsniff::sniff(body) {
        let (format, label) = if d.delimiter == b'\t' {
            (FormatId::Tsv, "tab".to_string())
        } else {
            (FormatId::Csv, format!("'{}'", d.delimiter as char))
        };
        return found(
            format,
            base,
            format!(
                "delimited text ({enc}), delimiter {label}, {:.0}% of rows with the same field count",
                f64::from(d.consistent_rows) * 100.0
            ),
        );
    }
    if let Some(f) = hint.and_then(|(format, mime)| csv_by_mime(format, mime, body)) {
        return f;
    }
    if let Some(f) = csv_by_extension(name, body) {
        return f;
    }
    // Markdown is a superset of plain text: with nothing else found, the MIME type and then
    // the extension decide.
    if enc == "UTF-8"
        && let Some((FormatId::Markdown, mime)) = hint
    {
        return found(
            FormatId::Markdown,
            Confidence::Likely,
            format!("text ({enc}) with MIME type {mime}, no other format found"),
        );
    }
    if enc == "UTF-8"
        && let Some(ext) = extension(name).filter(|e| e == "md" || e == "markdown")
    {
        return found(
            FormatId::Markdown,
            Confidence::Guess,
            format!("text ({enc}) with extension .{ext}, no other format found"),
        );
    }
    found(FormatId::Text, base, format!("text ({enc})"))
}

/// glTF JSON: a top-level `"asset"` object holding a `"version"` string. A small scanner
/// instead of a JSON parser, because the sample may be cut and files carry large buffers.
fn gltf(t: &str) -> Option<Found> {
    if !t.starts_with('{') {
        return None;
    }
    let b = t.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    // Inside the top-level "asset" object (depth 2).
    let mut in_asset = false;
    // The last string seen, as (depth, start, end); it is a key when a colon follows.
    let mut pending_key: Option<(usize, usize, usize)> = None;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i + 1;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                let end = i.min(b.len());
                pending_key = Some((depth, start, end));
            }
            b':' => {
                if let Some((d, s, e)) = pending_key.take() {
                    let key = &b[s..e];
                    let value = t[i + 1..].trim_start();
                    if d == 1 && key == b"asset" && value.starts_with('{') {
                        in_asset = true;
                    } else if d == 2 && in_asset && key == b"version" && value.starts_with('"') {
                        let v: String = value[1..]
                            .chars()
                            .take_while(|&c| c != '"')
                            .take(20)
                            .collect();
                        return Some(found(
                            FormatId::Gltf,
                            Confidence::Likely,
                            format!("json with top-level asset.version {}", crate::clean(&v)),
                        ));
                    }
                }
            }
            b'{' | b'[' => {
                depth += 1;
                pending_key = None;
            }
            b'}' | b']' => {
                if depth == 2 {
                    in_asset = false;
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return None;
                }
                pending_key = None;
            }
            b',' => pending_key = None,
            _ => {}
        }
        i += 1;
    }
    None
}

fn json_like(t: &str) -> bool {
    let mut chars = t.chars();
    match chars.next() {
        Some('{') => matches!(chars.find(|c| !c.is_whitespace()), Some('"' | '}')),
        Some('[') => matches!(
            chars.find(|c| !c.is_whitespace()),
            Some('{' | '[' | '"' | ']' | '-' | '0'..='9' | 't' | 'f' | 'n')
        ),
        _ => false,
    }
}

fn html(t: &str) -> Option<Found> {
    let head: String = t
        .chars()
        .take(1024)
        .collect::<String>()
        .to_ascii_lowercase();
    if head.starts_with("<!doctype html") {
        return Some(found(FormatId::Html, Confidence::Certain, "html doctype"));
    }
    if head.starts_with("<html") {
        return Some(found(
            FormatId::Html,
            Confidence::Certain,
            "html root element <html",
        ));
    }
    if head.starts_with('<') && (head.contains("<html") || head.contains("<!doctype html")) {
        return Some(found(
            FormatId::Html,
            Confidence::Likely,
            "markup with an <html element near the start",
        ));
    }
    None
}

/// `Name:` with a printable ASCII name and no space before the colon (RFC 5322 field).
fn header_name(line: &str) -> Option<&str> {
    let colon = line.find(':')?;
    let name = &line[..colon];
    (!name.is_empty()
        && name
            .bytes()
            .all(|b| (b'!'..=b'~').contains(&b) && b != b':'))
    .then_some(name)
}

/// Header names in the block starting at `lines[0]`, or None when a line is no header.
/// The whole block is read: provider mail often has dozens of `Received` and `ARC-*` lines
/// before `From`.
fn header_block<'a>(lines: &[&'a str]) -> Option<Vec<&'a str>> {
    let mut names = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if names.is_empty() {
                return None;
            }
            continue;
        }
        names.push(header_name(line)?);
    }
    Some(names)
}

fn eml(lines: &[&str]) -> Option<Found> {
    let names = header_block(lines)?;
    let has = |n: &str| names.iter().any(|h| h.eq_ignore_ascii_case(n));
    if !has("From") {
        return None;
    }
    let others = [
        "Date",
        "Subject",
        "To",
        "Message-ID",
        "MIME-Version",
        "Received",
    ]
    .into_iter()
    .filter(|n| has(n))
    .count();
    if others < 2 {
        return None;
    }
    let found_names: Vec<&str> = ["From", "Date", "Subject"]
        .into_iter()
        .filter(|n| has(n))
        .collect();
    Some(found(
        FormatId::Eml,
        Confidence::Likely,
        format!("mail header block with {}", found_names.join(", ")),
    ))
}

fn mbox(lines: &[&str]) -> Option<Found> {
    let first = lines.first()?;
    if !first.starts_with("From ") {
        return None;
    }
    let names = header_block(&lines[1..])?;
    if names.is_empty() {
        return None;
    }
    Some(found(
        FormatId::Mbox,
        Confidence::Likely,
        "mbox \"From \" separator line followed by mail headers",
    ))
}

fn gerber(lines: &[&str]) -> Option<Found> {
    let hit = lines
        .iter()
        .take(50)
        .map(|l| l.trim_start())
        .find(|l| l.starts_with("%FS") || l.starts_with("%MO"))?;
    Some(found(
        FormatId::Gerber,
        Confidence::Likely,
        format!("gerber statement {}", &hit[..3]),
    ))
}

/// Excellon drill files: an `M48` header, or a header with units (`METRIC`/`INCH`) and tool
/// definitions (`T<n>C<diameter>`), or a `%` header end with `G05`/`G90`, a `T<n>` tool
/// selection line and bare coordinate lines (Allegro and other writers without `M48`).
fn excellon(lines: &[&str]) -> Option<Found> {
    let head = || lines.iter().take(HEAD_LINES).map(|l| l.trim());
    if head().take(50).any(|l| l == "M48") {
        return Some(found(
            FormatId::Excellon,
            Confidence::Likely,
            "excellon header M48",
        ));
    }
    let units = head().any(|l| l.starts_with("METRIC") || l.starts_with("INCH"));
    let tool_def = head().any(|l| {
        let b = l.as_bytes();
        let digits = b.iter().skip(1).take_while(|c| c.is_ascii_digit()).count();
        b.first() == Some(&b'T') && digits > 0 && l[1 + digits..].contains('C')
    });
    if units && tool_def {
        return Some(found(
            FormatId::Excellon,
            Confidence::Likely,
            "excellon header with units and tool definitions",
        ));
    }
    let percent = head().any(|l| l == "%");
    let mode = head().any(|l| l == "G90" || l == "G05");
    let coords = head()
        .filter(|l| {
            let b = l.as_bytes();
            matches!(b.first(), Some(b'X' | b'Y'))
                && b.get(1).is_some_and(|c| c.is_ascii_digit() || *c == b'-')
        })
        .count();
    let tool_select = head()
        .any(|l| l.len() > 1 && l.starts_with('T') && l[1..].bytes().all(|b| b.is_ascii_digit()));
    if percent && mode && tool_select && coords >= 2 {
        return Some(found(
            FormatId::Excellon,
            Confidence::Likely,
            "excellon program: % header end, G90/G05, tool selection and coordinate lines",
        ));
    }
    None
}

/// Comment lines are skipped: slicers put hundreds of `;` thumbnail lines before the first
/// command. The whole sample is scanned until enough command lines were seen. A leading
/// line number (`N10 G21`) is skipped and the next word decides.
fn gcode(body: &str) -> Option<Found> {
    const ENOUGH: usize = 200;
    let (mut total, mut gm, mut other) = (0usize, 0usize, 0usize);
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if matches!(line.as_bytes()[0], b';' | b'(' | b'%') {
            continue;
        }
        total += 1;
        let mut words = line.split_whitespace();
        let mut word = words.next().unwrap_or("");
        let numbered = |w: &str| w.len() > 1 && w.as_bytes()[1].is_ascii_digit();
        let line_number = word.starts_with('N') && numbered(word);
        if line_number {
            // "N10G21" is written without a space too.
            let digits = word[1..].bytes().take_while(u8::is_ascii_digit).count();
            word = if word.len() > 1 + digits {
                &word[1 + digits..]
            } else {
                words.next().unwrap_or("")
            };
        }
        match word.as_bytes().first() {
            Some(b'G' | b'M') if numbered(word) => gm += 1,
            Some(b'T') if numbered(word) => other += 1,
            _ if line_number => other += 1,
            _ => {}
        }
        if total >= ENOUGH {
            break;
        }
    }
    // Tool and line-number words alone prove nothing; at least two G or M words are needed.
    if total < 3 || gm < 2 || (gm + other) * 2 <= total {
        return None;
    }
    Some(found(
        FormatId::Gcode,
        Confidence::Likely,
        format!(
            "{} of {total} command lines are G/M/T codes or numbered blocks",
            gm + other
        ),
    ))
}

/// OpenSCAD: a `module NAME(` or `function NAME(...) =` definition, an `include <x.scad>` or
/// `use <x.scad>` line, or at least two distinct signs among `include <`, `use <`, `$fn`,
/// primitive calls, `translate([`/`rotate([`, `color(` and `text(`. A Python `def ` line or a fenced code block rules it out (source
/// code in another language, or a document that quotes OpenSCAD).
fn openscad(body: &str) -> Option<Found> {
    let mut end = body.len().min(64 * 1024);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let head = &body[..end];
    let mut signs: Vec<&str> = Vec::new();
    let add = |s: &'static str, signs: &mut Vec<&str>| {
        if !signs.contains(&s) {
            signs.push(s);
        }
    };
    for line in head.lines().map(str::trim_start) {
        if line.starts_with("def ") || line.starts_with("```") || line.starts_with("~~~") {
            return None;
        }
        if let Some(rest) = line.strip_prefix("module ")
            && ident_then_paren(rest)
        {
            return Some(found(
                FormatId::Openscad,
                Confidence::Likely,
                "openscad module definition",
            ));
        }
        for (prefix, sign) in [("include <", "include <"), ("use <", "use <")] {
            if let Some(rest) = line.strip_prefix(prefix) {
                // `include <file.scad>` names an OpenSCAD file: proof on its own (a C
                // include has a `#` in front and another extension).
                if rest
                    .split('>')
                    .next()
                    .is_some_and(|f| f.trim().ends_with(".scad"))
                {
                    return Some(found(
                        FormatId::Openscad,
                        Confidence::Likely,
                        format!("openscad {prefix}...scad>"),
                    ));
                }
                add(sign, &mut signs);
            }
        }
        // `function NAME(args) = expr;` is OpenSCAD's own form (JavaScript and others use
        // a block), so it counts like a module definition.
        if let Some(rest) = line.strip_prefix("function ")
            && ident_then_paren(rest)
            && rest
                .find(')')
                .is_some_and(|i| rest[i + 1..].trim_start().starts_with('='))
        {
            return Some(found(
                FormatId::Openscad,
                Confidence::Likely,
                "openscad function definition",
            ));
        }
    }
    if head.contains("$fn") {
        add("$fn", &mut signs);
    }
    for (call, arg) in calls(head) {
        let sign = match call {
            "cube" => "cube(",
            "sphere" => "sphere(",
            "cylinder" => "cylinder(",
            "polygon" => "polygon(",
            "polyhedron" => "polyhedron(",
            "linear_extrude" => "linear_extrude(",
            "rotate_extrude" => "rotate_extrude(",
            "color" => "color(",
            "text" => "text(",
            "translate" if arg == Some('[') => "translate([",
            "rotate" if arg == Some('[') => "rotate([",
            _ => continue,
        };
        add(sign, &mut signs);
        if signs.len() >= 6 {
            break;
        }
    }
    if signs.len() < 2 {
        return None;
    }
    Some(found(
        FormatId::Openscad,
        Confidence::Likely,
        format!("openscad signs {}", signs.join(", ")),
    ))
}

/// `NAME(` with optional spaces before the parenthesis.
fn ident_then_paren(s: &str) -> bool {
    let n = s
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    n > 0 && s[n..].trim_start().starts_with('(')
}

/// Function calls `name (` in `text` with the first non-space character of the argument.
/// A name right after `.` or another identifier character is a method or part of a longer
/// name and is skipped.
fn calls(text: &str) -> impl Iterator<Item = (&str, Option<char>)> {
    let b = text.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() {
            let start = i;
            if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let prev = start.checked_sub(1).map(|p| b[p]);
                let method = matches!(prev, Some(b'.' | b'$'))
                    || prev.is_some_and(|p| p.is_ascii_alphanumeric() || p == b'_');
                let mut j = i;
                while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                    j += 1;
                }
                if !method && b.get(j) == Some(&b'(') {
                    let arg = text[j + 1..].trim_start().chars().next();
                    return Some((&text[start..i], arg));
                }
            } else {
                i += 1;
            }
        }
        None
    })
}

fn markdown_frontmatter(lines: &[&str]) -> Option<Found> {
    let kind = frontmatter(lines)?;
    Some(found(
        FormatId::Markdown,
        Confidence::Likely,
        format!("{kind} frontmatter"),
    ))
}

fn markdown_syntax(name: &str, lines: &[&str]) -> Option<Found> {
    let mut kinds = Vec::new();
    let is_hash = |l: &str| l.starts_with('#');
    // The closing frontmatter fence counts as a blank line before a heading, but only when
    // the body has no `key:` lines: there a `#` line is a comment in a later YAML document.
    let fence = frontmatter_end(lines)
        .map(|(end, _)| end)
        .filter(|&end| !lines[end + 1..].iter().any(|l| yaml_key_line(l.trim_end())));
    let heading = lines.iter().enumerate().any(|(i, l)| {
        let level = l.bytes().take_while(|&b| b == b'#').count();
        let atx = (1..=6).contains(&level) && l[level..].starts_with(' ');
        // Shell and Python comments come in runs; a heading stands alone.
        let after_fence = i > 0 && fence == Some(i - 1);
        let alone = (i == 0 || lines[i - 1].trim().is_empty() || after_fence)
            && lines.get(i + 1).is_none_or(|n| !is_hash(n));
        atx && alone
    });
    if heading {
        kinds.push("atx headings");
    }
    if lines.iter().any(|l| {
        // Top-level items only: indented dashes are mostly YAML or code.
        l.starts_with("- ") || l.starts_with("* ") || l.starts_with("+ ") || ordered_item(l)
    }) {
        kinds.push("list markers");
    }
    if lines.iter().any(|l| {
        let t = l.trim_start();
        t.starts_with("```") || t.starts_with("~~~")
    }) {
        kinds.push("fenced code");
    }
    // A pipe table (header row, delimiter row) is a strong signal: with a heading, a list or
    // prose it is Markdown, not `|` separated data, and so is a lone table in a .md file.
    if pipe_table(lines) {
        let prose = lines.iter().any(|l| prose_line(l));
        let md_ext = extension(name).is_some_and(|e| e == "md" || e == "markdown");
        if !kinds.is_empty() || prose || md_ext {
            kinds.push("pipe table");
        }
        if kinds.len() == 1 {
            // Only the table: nothing else to agree with it, the extension decided.
            kinds.push("extension");
        }
    }
    if kinds.len() < 2 {
        return None;
    }
    Some(found(
        FormatId::Markdown,
        Confidence::Likely,
        format!("markdown syntax: {}", kinds.join(", ")),
    ))
}

/// A pipe table: a line with `|`, then a delimiter row such as `| --- | :---: |`.
fn pipe_table(lines: &[&str]) -> bool {
    lines
        .windows(2)
        .any(|w| w[0].contains('|') && delimiter_row(w[1]))
}

fn delimiter_row(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('|') || !t.contains('-') {
        return false;
    }
    let cells: Vec<&str> = t.trim_matches('|').split('|').collect();
    cells.iter().all(|c| {
        let c = c.trim().trim_start_matches(':').trim_end_matches(':');
        !c.is_empty() && c.bytes().all(|b| b == b'-')
    })
}

/// A line of prose: at least four words, no `|`, not a heading, list item or code fence.
fn prose_line(line: &str) -> bool {
    let t = line.trim();
    !t.contains('|')
        && !t.starts_with(['#', '-', '*', '+', '`', '~', '>'])
        && t.split_whitespace().count() >= 4
}

fn ordered_item(t: &str) -> bool {
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    (1..=9).contains(&digits) && t[digits..].starts_with(". ")
}

/// YAML (`---`) or TOML (`+++`) frontmatter: a fence on line 1, a mapping (`key: value` or
/// `key = value` lines), a closing fence and a body. A body made only of YAML lines (keys,
/// list items, indented lines, comments, document fences) is a multi-document YAML file,
/// not Markdown; one prose line in the body is enough for Markdown. A `#` line is both a
/// YAML comment and a Markdown heading: in a body without `key:` lines it is a heading.
fn frontmatter(lines: &[&str]) -> Option<&'static str> {
    let (i, kind) = frontmatter_end(lines)?;
    let body: Vec<&str> = lines[i + 1..]
        .iter()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty())
        .collect();
    let keys = body.iter().any(|l| yaml_key_line(l));
    let yaml_only = body.iter().all(|l| {
        l.starts_with([' ', '\t'])
            || l.starts_with("- ")
            || *l == "-"
            || (l.starts_with('#') && (keys || !atx_heading(l)))
            || *l == "---"
            || *l == "..."
            || yaml_key_line(l)
    });
    (!body.is_empty() && !yaml_only).then_some(kind)
}

/// `#` to `######`, a space and text.
fn atx_heading(l: &str) -> bool {
    let level = l.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&level) && l[level..].starts_with(' ') && !l[level..].trim().is_empty()
}

/// The line index of the closing fence of frontmatter that opens on line 1, and its kind.
fn frontmatter_end(lines: &[&str]) -> Option<(usize, &'static str)> {
    let first = lines.first()?.trim_end();
    let (fence, close_alt, kind, sep) = match first {
        "---" => ("---", "...", "yaml", ':'),
        "+++" => ("+++", "+++", "toml", '='),
        _ => return None,
    };
    let mut keys = 0;
    for (i, line) in lines.iter().enumerate().skip(1) {
        let l = line.trim_end();
        if l == fence || l == close_alt {
            return (keys > 0).then_some((i, kind));
        }
        let t = l.trim_start();
        if t.is_empty() || t.starts_with('#') || l.starts_with([' ', '\t']) || t.starts_with("- ") {
            continue;
        }
        let key = t.split(sep).next().unwrap_or("");
        if t.contains(sep) && !key.trim().is_empty() && !key.trim().contains(' ') {
            keys += 1;
        } else if sep == '=' && t.starts_with('[') {
            continue;
        } else {
            return None;
        }
    }
    None
}

/// `key:` followed by a space or the line end, with a key of word characters.
fn yaml_key_line(l: &str) -> bool {
    let n = l
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        .count();
    n > 0 && (l[n..].starts_with(": ") || &l[n..] == ":")
}

/// Wavefront OBJ: most non-comment lines start with an OBJ keyword, and at least one vertex
/// line has three numbers.
fn obj(lines: &[&str]) -> Option<Found> {
    const KEYWORDS: &[&str] = &[
        "v", "vt", "vn", "vp", "f", "l", "p", "g", "o", "s", "usemtl", "mtllib",
    ];
    let (mut total, mut hits, mut vertex) = (0usize, 0usize, false);
    for line in lines.iter().map(|l| l.trim()) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        total += 1;
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or("");
        if KEYWORDS.contains(&key) {
            hits += 1;
        }
        if key == "v" && words.take(3).filter(|w| w.parse::<f64>().is_ok()).count() == 3 {
            vertex = true;
        }
    }
    if total < 3 || !vertex || hits * 2 <= total {
        return None;
    }
    Some(found(
        FormatId::Obj,
        Confidence::Likely,
        format!("wavefront obj: {hits} of {total} lines start with obj keywords"),
    ))
}

/// The MIME fallback: `text/csv` or `text/tab-separated-values` and every one of the first
/// records has the same field count of at least two. Likely: the sender declared the format.
fn csv_by_mime(format: FormatId, mime: &str, body: &str) -> Option<Found> {
    let delimiter = match format {
        FormatId::Csv => b',',
        FormatId::Tsv => b'\t',
        _ => return None,
    };
    let width = uniform_width(body, delimiter)?;
    Some(found(
        format,
        Confidence::Likely,
        format!("MIME type {mime} and {width} fields in every record; content sniffing declined"),
    ))
}

/// The extension fallback: `.csv` or `.tsv` and every one of the first records has the same
/// field count of at least two.
fn csv_by_extension(name: &str, body: &str) -> Option<Found> {
    let ext = extension(name)?;
    let (format, delimiter) = match ext.as_str() {
        "csv" => (FormatId::Csv, b','),
        "tsv" => (FormatId::Tsv, b'\t'),
        _ => return None,
    };
    let width = uniform_width(body, delimiter)?;
    Some(found(
        format,
        Confidence::Guess,
        format!(
            "extension .{} and {width} fields in every record; content sniffing declined",
            clean(&ext)
        ),
    ))
}

/// The field count shared by every one of the first records, if it is at least two.
fn uniform_width(body: &str, delimiter: u8) -> Option<usize> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(body.as_bytes());
    let mut width = None;
    for rec in rdr.records().take(CSV_FALLBACK_RECORDS) {
        let n = rec.ok()?.len();
        match width {
            None if n >= 2 => width = Some(n),
            Some(w) if w == n => {}
            _ => return None,
        }
    }
    width
}
