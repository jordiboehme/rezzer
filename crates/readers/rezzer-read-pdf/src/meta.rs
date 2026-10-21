// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Info dictionary and XMP as properties, and the facts of the file as a whole. Every copy
//! is charged like the rest of the object layer ([`Kept`]).

use pdfparts::{Info, Pdf};
use rezzer_model::{LossReason, Severity, Value};

use crate::Out;
use crate::objects::{Kept, given, is_are, iso};

/// The facts of the file as a whole that need no copy.
pub(crate) fn facts(pdf: &Pdf, out: &mut Out) {
    let facts = &mut out.doc.properties.facts;
    facts.pages = Some(pdf.page_count());
    facts.tagged = Some(pdf.tagged());
    facts.encrypted = Some(pdf.encryption().is_some());
}

/// Properties from the Info dictionary, else from XMP; an XMP value that differs from the Info
/// value it stands beside is kept as `xmp_<name>`. Then the language, version, PDF/A and
/// PDF/UA parts and the layers.
pub(crate) fn properties(pdf: &Pdf, kept: &Kept<'_>, out: &mut Out) {
    let info: Info = pdf.info();
    let xmp = pdf.xmp();
    let x = |key: &str| given(xmp.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()));
    // A date in the Info dictionary that does not parse, with no XMP date to stand in.
    for (name, d, key) in [
        ("creation", info.created.as_ref(), "xmp:CreateDate"),
        ("modification", info.modified.as_ref(), "xmp:ModifyDate"),
    ] {
        if let Some(d) = d
            && d.iso.is_none()
            && x(key).is_none()
        {
            out.loss(
                "metadata",
                format!(
                    "the {name} date of the document information is not a PDF date: {:?}",
                    d.raw
                ),
                LossReason::Unsupported,
                Severity::Info,
            );
        }
    }
    let mut left_out: Vec<&str> = Vec::new();
    let mut why: Option<String> = None;
    let props = &mut out.doc.properties;
    // The Info value, else the XMP value at `key`; and the XMP value when it differs.
    let pick = |info: Option<&str>, key: &str| -> (Option<String>, Option<String>) {
        let info = given(info);
        let xmp = x(key);
        match (info, xmp) {
            (Some(i), Some(v)) if i != v => (Some(i.to_string()), Some(v.to_string())),
            (Some(i), _) => (Some(i.to_string()), None),
            (None, v) => (v.map(str::to_string), None),
        }
    };
    let mut keep = |name: &'static str, texts: &[&str]| -> bool {
        match kept.take(1, texts) {
            Ok(()) => true,
            Err(w) => {
                why.get_or_insert(w);
                left_out.push(name);
                false
            }
        }
    };
    let mut custom: Vec<(&str, String)> = Vec::new();
    let (title, xmp_title) = pick(info.title.as_deref(), "dc:title");
    if let Some(t) = title
        && keep("title", &[&t])
    {
        props.title = Some(t);
    }
    // The Info author is one name; the XMP creators are a list, joined with "; ".
    let (author, xmp_author) = pick(info.author.as_deref(), "dc:creator");
    if let Some(a) = author {
        let names: Vec<String> = if given(info.author.as_deref()).is_some() {
            vec![a]
        } else {
            a.split("; ")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect()
        };
        let texts: Vec<&str> = names.iter().map(String::as_str).collect();
        if keep("authors", &texts) {
            props.authors = names;
        }
    }
    let (keywords, xmp_keywords) = pick(info.keywords.as_deref(), "pdf:Keywords");
    if let Some(k) = keywords {
        // Some producers write the whole list in quotes.
        let list = k
            .strip_prefix('"')
            .and_then(|k| k.strip_suffix('"'))
            .unwrap_or(&k);
        let words: Vec<String> = list
            .split([',', ';'])
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        let texts: Vec<&str> = words.iter().map(String::as_str).collect();
        if keep("keywords", &texts) {
            props.keywords = words;
        }
    }
    let dates = [
        ("created", iso(info.created.as_ref()), "xmp:CreateDate"),
        ("modified", iso(info.modified.as_ref()), "xmp:ModifyDate"),
    ];
    for (name, from_info, key) in dates {
        let (date, xmp_date) = pick(from_info, key);
        if let Some(d) = date
            && keep(name, &[&d])
        {
            let value = Some(Value::DateTime { value: d });
            match name {
                "created" => props.created = value,
                _ => props.modified = value,
            }
        }
        if let Some(v) = xmp_date {
            custom.push((
                if name == "created" {
                    "xmp_create_date"
                } else {
                    "xmp_modify_date"
                },
                v,
            ));
        }
    }
    let pairs = [
        (
            info.subject.as_deref(),
            "dc:description",
            "subject",
            "xmp_description",
        ),
        (
            info.creator.as_deref(),
            "xmp:CreatorTool",
            "creator",
            "xmp_creator_tool",
        ),
        (
            info.producer.as_deref(),
            "pdf:Producer",
            "producer",
            "xmp_producer",
        ),
    ];
    for (from_info, key, name, xmp_name) in pairs {
        let (v, xv) = pick(from_info, key);
        if let Some(v) = v {
            custom.push((name, v));
        }
        if let Some(v) = xv {
            custom.push((xmp_name, v));
        }
    }
    for (name, v) in [
        ("xmp_title", xmp_title),
        ("xmp_creator", xmp_author),
        ("xmp_keywords", xmp_keywords),
    ] {
        if let Some(v) = v {
            custom.push((name, v));
        }
    }
    if let Some(lang) = pdf.language()
        && keep("language", &[&lang])
    {
        props.language = Some(lang);
    }
    custom.push(("pdf_version", pdf.version()));
    if let Some(part) = x("pdfaid:part") {
        let conformance = x("pdfaid:conformance").unwrap_or_default().to_lowercase();
        custom.push(("pdfa", format!("PDF/A-{part}{conformance}")));
    }
    if let Some(part) = x("pdfuaid:part") {
        custom.push(("pdfua", format!("PDF/UA-{part}")));
    }
    let layers = pdf.layers();
    if !layers.is_empty() {
        props.facts.layers = Some(u32::try_from(layers.len()).unwrap_or(u32::MAX));
        let list: Vec<String> = layers
            .iter()
            .map(|l| {
                let name = given(Some(&l.name)).unwrap_or("unnamed layer");
                let shown = if l.visible { "shown" } else { "hidden" };
                format!("{name} ({shown})")
            })
            .collect();
        custom.push(("layers", list.join(", ")));
    }
    for (name, value) in custom {
        if keep(name, &[name, &value]) {
            props.custom.insert(name.to_string(), Value::Text { value });
        }
    }
    if let Some(why) = why {
        out.loss(
            "metadata",
            format!(
                "{} {} not kept: {why}",
                left_out.join(", "),
                is_are(left_out.len())
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}
