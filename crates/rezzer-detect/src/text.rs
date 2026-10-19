// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Text sniffing: decide whether the bytes are text and which text format they hold.
//!
//! Only the first [`SNIFF_BYTES`] are decoded. Structured formats are checked before CSV,
//! because their lines often split into equal field counts too.

use rezzer_model::{Confidence, FormatId};

use crate::{Found, clean, extension, found};

/// Bytes decoded for sniffing. The reader decodes the whole file later.
const SNIFF_BYTES: usize = 1024 * 1024;
/// Lines looked at by the line-based checks.
const HEAD_LINES: usize = 200;
/// Records the extension fallback for CSV and TSV looks at.
const CSV_FALLBACK_RECORDS: usize = 200;

/// Returns None when the bytes are not text.
pub(crate) fn detect(name: &str, bytes: &[u8]) -> Option<Found> {
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
    Some(classify(name, text, enc, base))
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

fn classify(name: &str, text: &str, enc: &str, base: Confidence) -> Found {
    let body = text.trim_start_matches('\u{FEFF}');
    let trimmed = body.trim_start();
    let lines: Vec<&str> = body.lines().take(HEAD_LINES).collect();

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
    if let Some(f) = gcode(body) {
        return f;
    }
    if let Some(f) = openscad(body) {
        return f;
    }
    if let Some(f) = markdown(&lines) {
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
    if let Some(f) = csv_by_extension(name, body) {
        return f;
    }
    found(FormatId::Text, base, format!("text ({enc})"))
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

fn excellon(lines: &[&str]) -> Option<Found> {
    lines.iter().take(50).any(|l| l.trim() == "M48").then(|| {
        found(
            FormatId::Excellon,
            Confidence::Likely,
            "excellon header M48",
        )
    })
}

/// Comment lines are skipped: slicers put hundreds of `;` thumbnail lines before the first
/// command. The whole sample is scanned until enough command lines were seen.
fn gcode(body: &str) -> Option<Found> {
    const ENOUGH: usize = 200;
    let (mut total, mut gm, mut other) = (0usize, 0usize, 0usize);
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let b = line.as_bytes();
        if matches!(b[0], b';' | b'(' | b'%') {
            continue;
        }
        total += 1;
        let numbered = b.get(1).is_some_and(u8::is_ascii_digit);
        match b[0] {
            b'G' | b'M' if numbered => gm += 1,
            b'T' | b'N' if numbered => other += 1,
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
            "{} of {total} command lines start with G/M/T/N codes",
            gm + other
        ),
    ))
}

fn openscad(body: &str) -> Option<Found> {
    let head: String = body.chars().take(64 * 1024).collect();
    let primitives = [
        "cube(",
        "sphere(",
        "cylinder(",
        "polyhedron(",
        "linear_extrude(",
        "rotate_extrude(",
        "square(",
        "circle(",
    ];
    let structure = [
        "module ",
        "difference()",
        "union()",
        "intersection()",
        "translate([",
    ];
    let prim: Vec<&str> = primitives
        .into_iter()
        .filter(|k| head.contains(k))
        .collect();
    let stru: Vec<&str> = structure.into_iter().filter(|k| head.contains(k)).collect();
    if prim.is_empty() || prim.len() + stru.len() < 2 {
        return None;
    }
    let mut all = stru;
    all.extend(prim);
    Some(found(
        FormatId::Openscad,
        Confidence::Likely,
        format!("openscad keywords {}", all.join(" ").trim()),
    ))
}

fn markdown(lines: &[&str]) -> Option<Found> {
    if let Some(kind) = frontmatter(lines) {
        return Some(found(
            FormatId::Markdown,
            Confidence::Likely,
            format!("{kind} frontmatter"),
        ));
    }
    let mut kinds = Vec::new();
    let is_hash = |l: &str| l.starts_with('#');
    let heading = lines.iter().enumerate().any(|(i, l)| {
        let level = l.bytes().take_while(|&b| b == b'#').count();
        let atx = (1..=6).contains(&level) && l[level..].starts_with(' ');
        // Shell and Python comments come in runs; a heading stands alone.
        let alone = (i == 0 || lines[i - 1].trim().is_empty())
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
    if kinds.len() < 2 {
        return None;
    }
    Some(found(
        FormatId::Markdown,
        Confidence::Likely,
        format!("markdown syntax: {}", kinds.join(", ")),
    ))
}

fn ordered_item(t: &str) -> bool {
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    (1..=9).contains(&digits) && t[digits..].starts_with(". ")
}

/// YAML (`---`) or TOML (`+++`) frontmatter: a fence on line 1, `key: value` or
/// `key = value` lines, and a closing fence.
fn frontmatter(lines: &[&str]) -> Option<&'static str> {
    let first = lines.first()?.trim_end();
    let (fence, close_alt, kind, sep) = match first {
        "---" => ("---", "...", "yaml", ':'),
        "+++" => ("+++", "+++", "toml", '='),
        _ => return None,
    };
    let mut keys = 0;
    for line in lines.iter().skip(1) {
        let l = line.trim_end();
        if l == fence || l == close_alt {
            return (keys > 0).then_some(kind);
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

/// The extension fallback: `.csv` or `.tsv` and every one of the first records has the same
/// field count of at least two.
fn csv_by_extension(name: &str, body: &str) -> Option<Found> {
    let ext = extension(name)?;
    let delimiter = match ext.as_str() {
        "csv" => b',',
        "tsv" => b'\t',
        _ => return None,
    };
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
    let width = width?;
    let format = if delimiter == b'\t' {
        FormatId::Tsv
    } else {
        FormatId::Csv
    };
    Some(found(
        format,
        Confidence::Guess,
        format!(
            "extension .{} and {width} fields in every record; content sniffing declined",
            clean(&ext)
        ),
    ))
}
