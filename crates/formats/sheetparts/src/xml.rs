// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The one bounded XML scanner every part reader uses.
//!
//! It refuses a DOCTYPE, knows only the five predefined entities and character references,
//! counts depth and items, asks the stop callback now and then and reports problems as
//! [`Issue`]s. Names are given without their namespace prefix.

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::{Issue, IssueKind, Limits};

/// An element name and its attributes, all without namespace prefixes.
pub(crate) struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Elem {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A boolean attribute the way OOXML writes it: `1` and `true` are yes.
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.attr(key), Some("1" | "true"))
    }
}

/// What the scanner hands to a part reader.
pub(crate) enum Ev<'a> {
    Start(&'a Elem),
    Empty(&'a Elem),
    /// The closing tag of an element.
    End(&'a str),
    /// Text between tags, entities resolved. Never empty.
    Text(&'a str),
}

const STOP_EVERY: usize = 512;

fn local(raw: &str) -> String {
    match raw.rsplit_once(':') {
        Some((_, name)) => name.to_string(),
        None => raw.to_string(),
    }
}

fn element(start: &BytesStart<'_>, part: &str, issues: &mut Vec<Issue>, items: &mut usize) -> Elem {
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        *items += 1;
        match attr {
            Ok(a) => {
                let value = a
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map(|v| v.into_owned())
                    .unwrap_or_else(|e| {
                        issues.push(Issue::new(
                            part,
                            IssueKind::Damaged,
                            format!("attribute value: {e}"),
                        ));
                        String::new()
                    });
                attrs.push((local(a.key.as_ref()), value));
            }
            Err(e) => issues.push(Issue::new(
                part,
                IssueKind::Damaged,
                format!("attribute: {e}"),
            )),
        }
    }
    Elem {
        name: local(start.name().as_ref()),
        attrs,
    }
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

/// Scans `bytes` and calls `on_event(event, depth)`. `depth` is 1 for the root element.
/// Returns false when the scan ended early (an issue says why).
pub(crate) fn scan(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
    issues: &mut Vec<Issue>,
    on_event: &mut dyn FnMut(Ev<'_>, usize),
) -> bool {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.strip_prefix('\u{feff}').unwrap_or(t),
        Err(e) => {
            issues.push(Issue::new(
                part,
                IssueKind::Damaged,
                format!("not UTF-8 text: {e}"),
            ));
            return false;
        }
    };
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut items = 0usize;
    let mut pending = String::new();
    let mut names: Vec<String> = Vec::new();

    macro_rules! flush {
        () => {
            if !pending.is_empty() {
                on_event(Ev::Text(&pending), depth);
                pending.clear();
            }
        };
    }

    loop {
        items += 1;
        if items > limits.max_items {
            issues.push(Issue::new(
                part,
                IssueKind::Limit,
                format!("more than {} items", limits.max_items),
            ));
            return false;
        }
        if items.is_multiple_of(STOP_EVERY) && stop() {
            issues.push(Issue::new(part, IssueKind::Limit, "stopped by the caller"));
            return false;
        }
        let event = match reader.read_event() {
            Ok(e) => e,
            Err(e) => {
                issues.push(Issue::new(
                    part,
                    IssueKind::Damaged,
                    format!("XML error: {e}"),
                ));
                return false;
            }
        };
        match event {
            Event::Start(start) => {
                flush!();
                depth += 1;
                if depth > limits.max_depth {
                    issues.push(Issue::new(
                        part,
                        IssueKind::Limit,
                        format!("nesting deeper than {} levels", limits.max_depth),
                    ));
                    return false;
                }
                let elem = element(&start, part, issues, &mut items);
                names.push(elem.name.clone());
                on_event(Ev::Start(&elem), depth);
            }
            Event::Empty(start) => {
                flush!();
                if depth + 1 > limits.max_depth {
                    issues.push(Issue::new(
                        part,
                        IssueKind::Limit,
                        format!("nesting deeper than {} levels", limits.max_depth),
                    ));
                    return false;
                }
                let elem = element(&start, part, issues, &mut items);
                on_event(Ev::Empty(&elem), depth + 1);
            }
            Event::End(_) => {
                flush!();
                let name = names.pop().unwrap_or_default();
                on_event(Ev::End(&name), depth);
                depth = depth.saturating_sub(1);
            }
            Event::Text(t) => pending.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Event::CData(c) => pending.push_str(&c),
            Event::GeneralRef(r) => {
                if r.is_char_ref() {
                    match r.resolve_char_ref() {
                        Ok(Some(c)) => pending.push(c),
                        _ => issues.push(Issue::new(
                            part,
                            IssueKind::Damaged,
                            format!("bad character reference &{};", &*r),
                        )),
                    }
                } else if let Some(c) = predefined(&r) {
                    pending.push(c);
                } else {
                    issues.push(Issue::new(
                        part,
                        IssueKind::Damaged,
                        format!("unknown entity &{};", &*r),
                    ));
                }
            }
            Event::DocType(_) => {
                issues.push(Issue::new(
                    part,
                    IssueKind::Unsupported,
                    "a DOCTYPE is refused: the part is not read",
                ));
                return false;
            }
            Event::Eof => {
                flush!();
                if depth != 0 {
                    issues.push(Issue::new(
                        part,
                        IssueKind::Damaged,
                        "the part ends inside an element",
                    ));
                    return false;
                }
                return true;
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
}
