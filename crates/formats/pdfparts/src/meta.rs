// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Document metadata: the Info dictionary, XMP, language, the version and the tagged flag.

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;

use crate::objects::{get, get_date, get_dict, get_name, get_text, reference};
use crate::{Issue, IssueKind, Pdf};

/// A date as written in the file and, when it parses, in ISO 8601.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfDate {
    pub raw: String,
    pub iso: Option<String>,
}

/// The document information dictionary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub created: Option<PdfDate>,
    pub modified: Option<PdfDate>,
}

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const DC: &str = "http://purl.org/dc/elements/1.1/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/";
const PDF: &str = "http://ns.adobe.com/pdf/1.3/";
const PDFAID: &str = "http://www.aiim.org/pdfa/ns/id/";
const PDFUAID: &str = "http://www.aiim.org/pdfua/ns/id/";

/// The XMP properties that are read, in output order.
const KEYS: [(&str, &str, &str); 11] = [
    (DC, "title", "dc:title"),
    (DC, "creator", "dc:creator"),
    (DC, "description", "dc:description"),
    (XMP, "CreateDate", "xmp:CreateDate"),
    (XMP, "ModifyDate", "xmp:ModifyDate"),
    (XMP, "CreatorTool", "xmp:CreatorTool"),
    (PDF, "Producer", "pdf:Producer"),
    (PDF, "Keywords", "pdf:Keywords"),
    (PDFAID, "part", "pdfaid:part"),
    (PDFAID, "conformance", "pdfaid:conformance"),
    (PDFUAID, "part", "pdfuaid:part"),
];

fn key_index(ns: Option<&str>, local: &str) -> Option<usize> {
    let ns = ns?;
    KEYS.iter().position(|(n, l, _)| *n == ns && *l == local)
}

impl Pdf {
    /// The Info dictionary, text decoded and dates turned into ISO 8601 where they parse.
    pub fn info(&self) -> Info {
        let doc = &self.doc;
        let Some(info) = get_dict(doc, &doc.trailer, b"Info") else {
            return Info::default();
        };
        let text = |k: &[u8]| get_text(doc, info, k);
        let date = |k: &[u8]| get_date(doc, info, k);
        Info {
            title: text(b"Title"),
            author: text(b"Author"),
            subject: text(b"Subject"),
            keywords: text(b"Keywords"),
            creator: text(b"Creator"),
            producer: text(b"Producer"),
            created: date(b"CreationDate"),
            modified: date(b"ModDate"),
        }
    }

    /// The natural language of the document (`/Lang` of the catalog), for example `en-US`.
    pub fn language(&self) -> Option<String> {
        get_text(&self.doc, self.catalog()?, b"Lang").filter(|l| !l.trim().is_empty())
    }

    /// The PDF version, for example `1.7`: the header's, or the catalog's `/Version` when that
    /// is later (an incremental save may raise it there).
    pub fn version(&self) -> String {
        let header = self.doc.version.clone();
        let catalog = self
            .catalog()
            .and_then(|c| get_name(&self.doc, c, b"Version"))
            .and_then(|v| version_number(v).map(|n| (n, v)));
        match (catalog, version_number(header.as_bytes())) {
            (Some((n, v)), Some(h)) if n > h => String::from_utf8_lossy(v).into_owned(),
            (Some((_, v)), None) => String::from_utf8_lossy(v).into_owned(),
            _ => header,
        }
    }

    /// The file says its tag tree is reliable (`/MarkInfo /Marked true`).
    pub fn tagged(&self) -> bool {
        let doc = &self.doc;
        self.catalog()
            .and_then(|c| get_dict(doc, c, b"MarkInfo"))
            .and_then(|m| get(doc, m, b"Marked"))
            .is_some_and(|m| matches!(m, lopdf::Object::Boolean(true)))
    }

    /// Selected XMP properties as `(key, value)` pairs: `dc:title`, `dc:creator` (a list,
    /// joined with `; `), `dc:description`, `xmp:CreateDate`, `xmp:ModifyDate`,
    /// `xmp:CreatorTool`, `pdf:Producer`, `pdf:Keywords`, `pdfaid:part`, `pdfaid:conformance`
    /// and `pdfuaid:part`. A DOCTYPE is refused and depth and item counts are limited.
    pub fn xmp(&self) -> Vec<(String, String)> {
        let Some(meta) = self.catalog().and_then(|c| c.get(b"Metadata").ok()) else {
            return Vec::new();
        };
        let Some(id) = reference(meta) else {
            self.note(vec![Issue::new(
                "metadata",
                IssueKind::Damaged,
                "the XMP metadata is not a stream object",
            )]);
            return Vec::new();
        };
        let Some(bytes) = self.plain(id) else {
            self.note(vec![Issue::new(
                "metadata",
                IssueKind::Limit,
                "the XMP stream is not read: it is refused or does not decode",
            )]);
            return Vec::new();
        };
        let mut issues = Vec::new();
        let values = parse_xmp(&bytes, self, &mut issues);
        self.note(issues);
        values
            .into_iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|v| (KEYS[i].2.to_string(), v)))
            .collect()
    }
}

/// A version such as `1.7` as (major, minor); `None` when it is not one.
fn version_number(v: &[u8]) -> Option<(u32, u32)> {
    if v.len() > 8 {
        return None;
    }
    let text = std::str::from_utf8(v).ok()?;
    let (major, minor) = text.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// The property being read and what it has collected.
struct Prop {
    key: usize,
    depth: usize,
    text: String,
    items: Vec<(Option<String>, String)>,
    /// Inside an `rdf:li`: its language and text so far.
    li: Option<(Option<String>, String)>,
    alt: bool,
}

fn predefined(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

fn parse_xmp(bytes: &[u8], pdf: &Pdf, issues: &mut Vec<Issue>) -> Vec<Option<String>> {
    let limits = &pdf.limits;
    let mut values: Vec<Option<String>> = vec![None; KEYS.len()];
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.strip_prefix('\u{feff}').unwrap_or(t),
        Err(e) => {
            issues.push(Issue::new(
                "metadata",
                IssueKind::Damaged,
                format!("the XMP metadata is not UTF-8 text: {e}"),
            ));
            return values;
        }
    };
    let mut reader = NsReader::from_str(text);
    // Namespace and local name of each open element.
    let mut stack: Vec<(Option<String>, String)> = Vec::new();
    let mut prop: Option<Prop> = None;
    let mut items = 0usize;
    loop {
        items += 1;
        if items > limits.max_items {
            issues.push(Issue::new(
                "metadata",
                IssueKind::Limit,
                format!("the XMP metadata has more than {} items", limits.max_items),
            ));
            return values;
        }
        let (ns, event) = match reader.read_resolved_event() {
            Ok((ns, event)) => (namespace(ns), event),
            Err(e) => {
                issues.push(Issue::new(
                    "metadata",
                    IssueKind::Damaged,
                    format!("XMP error: {e}"),
                ));
                return values;
            }
        };
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let empty = matches!(event, Event::Empty(_));
                let local = start.local_name().as_ref().to_string();
                let depth = stack.len() + 1;
                if depth > limits.max_depth {
                    issues.push(Issue::new(
                        "metadata",
                        IssueKind::Limit,
                        format!(
                            "the XMP metadata nests deeper than {} levels",
                            limits.max_depth
                        ),
                    ));
                    return values;
                }
                let in_description = stack
                    .last()
                    .is_some_and(|(n, l)| n.as_deref() == Some(RDF) && l == "Description");
                let is_rdf = ns.as_deref() == Some(RDF);
                if is_rdf && local == "Description" {
                    // Properties written as attributes.
                    for (key, value) in attributes(&reader, start, issues, &mut items) {
                        set(&mut values, key, value);
                    }
                } else if let Some(p) = prop.as_mut() {
                    if is_rdf && local == "Alt" {
                        p.alt = true;
                    } else if is_rdf && local == "li" {
                        let lang = start
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == "xml:lang")
                            .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                            .map(|v| v.into_owned());
                        p.li = Some((lang, String::new()));
                        if empty {
                            p.li = None;
                        }
                    }
                } else if in_description
                    && !empty
                    && let Some(key) = key_index(ns.as_deref(), &local)
                {
                    prop = Some(Prop {
                        key,
                        depth,
                        text: String::new(),
                        items: Vec::new(),
                        li: None,
                        alt: false,
                    });
                }
                if !empty {
                    stack.push((ns, local));
                }
            }
            Event::End(_) => {
                let depth = stack.len();
                let closed = stack.pop();
                if let Some(p) = prop.as_mut() {
                    let is_li = closed
                        .as_ref()
                        .is_some_and(|(n, l)| n.as_deref() == Some(RDF) && l == "li");
                    if is_li && let Some((lang, t)) = p.li.take() {
                        p.items.push((lang, t.trim().to_string()));
                    }
                    if depth == p.depth {
                        let p = prop.take().expect("open property");
                        let value = if p.items.is_empty() {
                            p.text
                        } else if p.alt {
                            p.items
                                .iter()
                                .find(|(l, _)| l.as_deref() == Some("x-default"))
                                .or(p.items.first())
                                .map(|(_, t)| t.clone())
                                .unwrap_or_default()
                        } else {
                            let parts: Vec<&str> = p
                                .items
                                .iter()
                                .map(|(_, t)| t.as_str())
                                .filter(|t| !t.is_empty())
                                .collect();
                            parts.join("; ")
                        };
                        set(&mut values, p.key, value);
                    }
                }
            }
            Event::Text(t) => push_text(&mut prop, &t.xml_content(XmlVersion::Implicit1_0)),
            Event::CData(c) => push_text(&mut prop, &c),
            Event::GeneralRef(r) => {
                let c = if r.is_char_ref() {
                    r.resolve_char_ref().ok().flatten()
                } else {
                    predefined(&r)
                };
                match c {
                    Some(c) => push_text(&mut prop, c.encode_utf8(&mut [0; 4])),
                    None => issues.push(Issue::new(
                        "metadata",
                        IssueKind::Damaged,
                        format!("unknown entity &{};", &*r),
                    )),
                }
            }
            Event::DocType(_) => {
                issues.push(Issue::new(
                    "metadata",
                    IssueKind::Unsupported,
                    "a DOCTYPE is refused: the XMP metadata is not read",
                ));
                return vec![None; KEYS.len()];
            }
            Event::Eof => return values,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
}

/// Keeps the first non-empty value of a property.
fn set(values: &mut [Option<String>], key: usize, v: String) {
    let v = v.trim().to_string();
    if !v.is_empty() && values[key].is_none() {
        values[key] = Some(v);
    }
}

fn namespace(ns: ResolveResult<'_>) -> Option<String> {
    match ns {
        ResolveResult::Bound(n) => Some(n.into_inner().to_string()),
        _ => None,
    }
}

fn push_text(prop: &mut Option<Prop>, s: &str) {
    if let Some(p) = prop.as_mut() {
        match p.li.as_mut() {
            Some((_, t)) => t.push_str(s),
            None => p.text.push_str(s),
        }
    }
}

/// Known properties written as attributes of `rdf:Description`.
fn attributes(
    reader: &NsReader<&[u8]>,
    start: &BytesStart<'_>,
    issues: &mut Vec<Issue>,
    items: &mut usize,
) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for attr in start.attributes() {
        *items += 1;
        let Ok(attr) = attr else {
            issues.push(Issue::new(
                "metadata",
                IssueKind::Damaged,
                "a broken XMP attribute",
            ));
            continue;
        };
        let (ns, local) = reader.resolver().resolve_attribute(attr.key);
        let ns = namespace(ns);
        let local = local.as_ref().to_string();
        if let Some(key) = key_index(ns.as_deref(), &local)
            && let Ok(v) = attr.normalized_value(XmlVersion::Implicit1_0)
        {
            out.push((key, v.into_owned()));
        }
    }
    out
}
