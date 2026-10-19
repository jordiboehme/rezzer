// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Frontmatter (YAML between `---`, TOML between `+++`) mapped onto document properties.

use std::collections::{HashMap, HashSet};

use rezzer_limits::Budget;
use rezzer_model::{Properties, Value};
use saphyr_parser::{Event, Parser, ScalarStyle};

/// Deepest nesting that is flattened into `a.b.c` keys; deeper values are kept as JSON text.
const MAX_FLATTEN_DEPTH: usize = 32;

/// Integers beyond this cannot be held exactly by an `f64`; they are kept as text.
const MAX_EXACT_INT: i128 = 1 << 53;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Syntax {
    Yaml,
    Toml,
}

/// A format-neutral frontmatter value.
#[derive(Debug, Clone)]
enum Fm {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    /// A TOML date or date-time, already printed in ISO 8601.
    Date(String),
    List(Vec<Fm>),
    Map(Vec<(String, Fm)>),
}

/// The deepest nesting a YAML block may have.
const MAX_YAML_DEPTH: usize = 128;

/// The most work a YAML block may cost, counted in nodes. A node is a scalar, an alias, a
/// sequence or a mapping; every node an alias copies is counted again, and long scalars count
/// one more for every 32 bytes, so expanding aliases cannot take much time or memory.
const MAX_YAML_NODES: usize = 100_000;

/// How many YAML events are read between two time checks.
const CHECK_EVERY: usize = 1000;

/// Why frontmatter could not be read.
#[derive(Debug)]
pub(crate) enum FmError {
    /// The text is not valid or is over a budget of the reader.
    Damaged(String),
    /// The time limit ran out.
    Limit(String),
}

/// Parses `text` and fills `props`. On an error nothing is changed and the message is returned.
///
/// Values keep their data but not their YAML tags (`!foo bar` reads as `bar`). A key with
/// no value is kept as an empty text.
pub(crate) fn apply(
    syntax: Syntax,
    text: &str,
    props: &mut Properties,
    budget: &Budget,
) -> Result<(), FmError> {
    let root = match syntax {
        Syntax::Yaml => parse_yaml(text, budget)?,
        Syntax::Toml => {
            let t: toml::Table =
                toml::from_str(text).map_err(|e| FmError::Damaged(e.to_string()))?;
            from_toml(toml::Value::Table(t))
        }
    };
    match root {
        Fm::Map(entries) => {
            map_properties(entries, props);
            Ok(())
        }
        // An empty YAML block reads as null.
        Fm::Null => Ok(()),
        _ => Err(FmError::Damaged(
            "frontmatter is not a mapping of keys to values".into(),
        )),
    }
}

/// Frontmatter found at the very start of a file.
pub(crate) struct Split<'a> {
    pub(crate) syntax: Syntax,
    pub(crate) raw: &'a str,
    /// Byte offset where the Markdown body starts, after the closing line.
    pub(crate) body_start: usize,
}

/// Splits frontmatter off the start of `text`: a first line of `---` (YAML) or `+++` (TOML) and a
/// closing line of `---` or `...` (YAML) or `+++` (TOML). Without a closing line there is none.
pub(crate) fn split(text: &str) -> Option<Split<'_>> {
    let (syntax, open) = if text.starts_with("---") {
        (Syntax::Yaml, "---")
    } else if text.starts_with("+++") {
        (Syntax::Toml, "+++")
    } else {
        return None;
    };
    let first_end = text.find('\n').unwrap_or(text.len());
    if text[..first_end].trim_end() != open {
        return None;
    }
    let raw_start = (first_end + 1).min(text.len());
    // As in CommonMark tools: the line after the opening one may not be blank or a closer.
    let second = text[raw_start..]
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end();
    if second.is_empty() || second == open || (syntax == Syntax::Yaml && second == "...") {
        return None;
    }
    let mut pos = raw_start;
    while pos < text.len() {
        let end = text[pos..].find('\n').map_or(text.len(), |i| pos + i);
        let line = text[pos..end].trim_end();
        let closes = line == open || (syntax == Syntax::Yaml && line == "...");
        if closes {
            return Some(Split {
                syntax,
                raw: &text[raw_start..pos],
                body_start: (end + 1).min(text.len()),
            });
        }
        pos = end + 1;
    }
    None
}

/// An open sequence or mapping while the events are read.
enum Open {
    Seq {
        anchor: usize,
        items: Vec<Fm>,
        weight: usize,
    },
    Map {
        anchor: usize,
        entries: Vec<(String, Fm)>,
        keys: HashSet<String>,
        key: Option<Fm>,
        weight: usize,
    },
}

/// Builds Rezzer's own values from the events of the first YAML document, with hard budgets
/// on depth, node count (alias expansion included) and time. Reading stops at the first
/// budget that is hit. The tree is never built by a serde deserializer, so aliases cannot
/// replay a large value unnoticed.
fn parse_yaml(text: &str, budget: &Budget) -> Result<Fm, FmError> {
    let damaged = |m: String| FmError::Damaged(m);
    let mut stack: Vec<Open> = Vec::new();
    let mut anchors: HashMap<usize, (Fm, usize)> = HashMap::new();
    let mut spent = 0usize;
    let mut root: Option<Fm> = None;
    let mut in_document = false;
    let mut charge = |units: usize| -> Result<(), FmError> {
        spent = spent.saturating_add(units);
        if spent > MAX_YAML_NODES {
            Err(FmError::Damaged(format!(
                "more than {MAX_YAML_NODES} nodes (aliases expanded)"
            )))
        } else {
            Ok(())
        }
    };

    for (n, item) in Parser::new_from_str(text).enumerate() {
        if n % CHECK_EVERY == CHECK_EVERY - 1 && budget.check_time().is_err() {
            return Err(FmError::Limit("time limit reached".into()));
        }
        let (event, _) = item.map_err(|e| damaged(e.to_string()))?;
        // The finished value of a node, its weight and its anchor, when one just ended.
        let done: Option<(Fm, usize, usize)> = match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {
                if matches!(event, Event::StreamEnd) {
                    break;
                }
                if matches!(event, Event::DocumentEnd) {
                    // Only the first document is read.
                    break;
                }
                None
            }
            Event::DocumentStart(_) => {
                if in_document {
                    return Err(damaged("more than one YAML document".into()));
                }
                in_document = true;
                None
            }
            Event::Scalar(value, style, anchor, tag) => {
                let weight = 1 + value.len() / 32;
                charge(weight)?;
                let forced_str = tag.as_ref().is_some_and(|t| t.suffix == "str");
                let fm = if forced_str || !matches!(style, ScalarStyle::Plain) {
                    Fm::Str(value.into_owned())
                } else {
                    resolve_plain(&value)
                };
                Some((fm, weight, anchor))
            }
            Event::Alias(id) => {
                let Some((value, weight)) = anchors.get(&id) else {
                    return Err(damaged("alias without a finished anchor".into()));
                };
                charge(*weight)?;
                Some((value.clone(), *weight, 0))
            }
            Event::SequenceStart(anchor, _) => {
                charge(1)?;
                if stack.len() >= MAX_YAML_DEPTH {
                    return Err(damaged(format!(
                        "nesting deeper than the {MAX_YAML_DEPTH} levels allowed"
                    )));
                }
                stack.push(Open::Seq {
                    anchor,
                    items: Vec::new(),
                    weight: 1,
                });
                None
            }
            Event::MappingStart(anchor, _) => {
                charge(1)?;
                if stack.len() >= MAX_YAML_DEPTH {
                    return Err(damaged(format!(
                        "nesting deeper than the {MAX_YAML_DEPTH} levels allowed"
                    )));
                }
                stack.push(Open::Map {
                    anchor,
                    entries: Vec::new(),
                    keys: HashSet::new(),
                    key: None,
                    weight: 1,
                });
                None
            }
            Event::SequenceEnd => match stack.pop() {
                Some(Open::Seq {
                    anchor,
                    items,
                    weight,
                }) => Some((Fm::List(items), weight, anchor)),
                _ => return Err(damaged("unbalanced YAML".into())),
            },
            Event::MappingEnd => match stack.pop() {
                Some(Open::Map {
                    anchor,
                    entries,
                    weight,
                    ..
                }) => Some((Fm::Map(entries), weight, anchor)),
                _ => return Err(damaged("unbalanced YAML".into())),
            },
        };
        let Some((value, weight, anchor)) = done else {
            continue;
        };
        if anchor != 0 {
            // Keeping the copy for later aliases costs as much as the value.
            charge(weight)?;
            anchors.insert(anchor, (value.clone(), weight));
        }
        match stack.last_mut() {
            None => root = Some(value),
            Some(Open::Seq {
                items, weight: w, ..
            }) => {
                *w += weight;
                items.push(value);
            }
            Some(Open::Map {
                entries,
                keys,
                key,
                weight: w,
                ..
            }) => {
                *w += weight;
                match key.take() {
                    None => *key = Some(value),
                    Some(k) => {
                        let name = key_text(&k);
                        if !keys.insert(name.clone()) {
                            return Err(damaged(format!("duplicate key `{name}`")));
                        }
                        entries.push((name, value));
                    }
                }
            }
        }
    }
    Ok(root.unwrap_or(Fm::Null))
}

fn key_text(k: &Fm) -> String {
    match scalar_text(k) {
        Some(s) => s,
        None if matches!(k, Fm::Null) => "null".into(),
        None => to_json(k).to_string(),
    }
}

/// The type of a plain scalar by the YAML 1.2 core schema.
fn resolve_plain(s: &str) -> Fm {
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Fm::Null,
        "true" | "True" | "TRUE" => return Fm::Bool(true),
        "false" | "False" | "FALSE" => return Fm::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Fm::Float(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => return Fm::Float(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Fm::Float(f64::NAN),
        _ => {}
    }
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return match s.parse::<i128>() {
            Ok(i) => Fm::Int(i),
            Err(_) => Fm::Str(s.to_string()),
        };
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8)] {
        if let Some(rest) = s.strip_prefix(prefix)
            && !rest.is_empty()
            && let Ok(i) = i128::from_str_radix(rest, radix)
        {
            return Fm::Int(i);
        }
    }
    let float_like = {
        let body = digits;
        let (mantissa, exp) = match body.find(['e', 'E']) {
            Some(i) => (&body[..i], Some(&body[i + 1..])),
            None => (body, None),
        };
        let exp_ok = exp.is_none_or(|e| {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
        });
        let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let digits_only = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
        exp_ok
            && digits_only(int)
            && digits_only(frac)
            && !(int.is_empty() && frac.is_empty())
            && (mantissa.contains('.') || exp.is_some())
    };
    if float_like && let Ok(f) = s.parse::<f64>() {
        return Fm::Float(f);
    }
    Fm::Str(s.to_string())
}

fn from_toml(v: toml::Value) -> Fm {
    use toml::Value as T;
    match v {
        T::String(s) => Fm::Str(s),
        T::Integer(i) => Fm::Int(i128::from(i)),
        T::Float(f) => Fm::Float(f),
        T::Boolean(b) => Fm::Bool(b),
        T::Datetime(dt) => {
            if dt.date.is_some() {
                Fm::Date(dt.to_string())
            } else {
                // A time of day alone is not a date.
                Fm::Str(dt.to_string())
            }
        }
        T::Array(items) => Fm::List(items.into_iter().map(from_toml).collect()),
        T::Table(t) => Fm::Map(t.into_iter().map(|(k, v)| (k, from_toml(v))).collect()),
    }
}

fn map_properties(entries: Vec<(String, Fm)>, props: &mut Properties) {
    for (key, value) in entries {
        match key.as_str() {
            "title" if props.title.is_none() => match scalar_text(&value) {
                Some(s) => props.title = Some(s),
                None => put_custom(props, key, value),
            },
            "description" if props.description.is_none() => match scalar_text(&value) {
                Some(s) => props.description = Some(s),
                None => put_custom(props, key, value),
            },
            "lang" | "language" if props.language.is_none() => match scalar_text(&value) {
                Some(s) => props.language = Some(s),
                None => put_custom(props, key, value),
            },
            "date" if props.created.is_none() && matches!(value, Fm::Str(_) | Fm::Date(_)) => {
                match to_value(&value) {
                    Some(v) => props.created = Some(v),
                    None => put_custom(props, key, value),
                }
            }
            "author" | "authors" => match string_list(&value) {
                Some(list) => props.authors.extend(list),
                None => put_custom(props, key, value),
            },
            "tags" | "keywords" => match string_list(&value) {
                Some(list) => props.keywords.extend(list),
                None => put_custom(props, key, value),
            },
            _ => put_custom(props, key, value),
        }
    }
}

/// A string, or a number or boolean printed as text. Anything else is not a scalar text.
fn scalar_text(v: &Fm) -> Option<String> {
    match v {
        Fm::Str(s) | Fm::Date(s) => Some(s.clone()),
        Fm::Int(i) => Some(i.to_string()),
        Fm::Float(f) => Some(f.to_string()),
        Fm::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A list of scalars, or one scalar (a comma separated string is split). `None` for maps,
/// nested lists and null, so the caller keeps the value in `custom`.
fn string_list(v: &Fm) -> Option<Vec<String>> {
    match v {
        Fm::List(items) => items.iter().map(scalar_text).collect(),
        Fm::Str(s) => Some(
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect(),
        ),
        Fm::Int(_) | Fm::Float(_) | Fm::Bool(_) | Fm::Date(_) => scalar_text(v).map(|s| vec![s]),
        Fm::Null | Fm::Map(_) => None,
    }
}

fn put_custom(props: &mut Properties, key: String, value: Fm) {
    flatten(props, key, value, 0);
}

fn flatten(props: &mut Properties, key: String, value: Fm, depth: usize) {
    match value {
        Fm::Null => {
            props.custom.insert(
                key,
                Value::Text {
                    value: String::new(),
                },
            );
        }
        Fm::Map(entries) if depth < MAX_FLATTEN_DEPTH => {
            for (k, v) in entries {
                flatten(props, format!("{key}.{k}"), v, depth + 1);
            }
        }
        other => {
            if let Some(v) = to_value(&other) {
                props.custom.insert(key, v);
            }
        }
    }
}

/// A typed value; maps (and null) inside lists become JSON text. `None` for null.
fn to_value(v: &Fm) -> Option<Value> {
    Some(match v {
        Fm::Null => return None,
        Fm::Bool(b) => Value::Bool { value: *b },
        Fm::Int(i) if i.abs() <= MAX_EXACT_INT => Value::number(*i as f64, None),
        Fm::Int(i) => Value::Text {
            value: i.to_string(),
        },
        Fm::Float(f) => Value::number(*f, None),
        Fm::Str(s) => classify(s),
        Fm::Date(s) => classify(s),
        Fm::List(items) => Value::List {
            value: items
                .iter()
                .map(|i| {
                    to_value(i).unwrap_or_else(|| Value::Text {
                        value: "null".into(),
                    })
                })
                .collect(),
        },
        Fm::Map(_) => Value::Text {
            value: to_json(v).to_string(),
        },
    })
}

fn to_json(v: &Fm) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Fm::Null => J::Null,
        Fm::Bool(b) => J::Bool(*b),
        Fm::Int(i) if i.abs() <= MAX_EXACT_INT => J::from(*i as i64),
        Fm::Int(i) => J::String(i.to_string()),
        Fm::Float(f) => {
            serde_json::Number::from_f64(*f).map_or_else(|| J::String(f.to_string()), J::Number)
        }
        Fm::Str(s) | Fm::Date(s) => J::String(s.clone()),
        Fm::List(items) => J::Array(items.iter().map(to_json).collect()),
        Fm::Map(entries) => J::Object(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), to_json(v)))
                .collect(),
        ),
    }
}

/// Text that is exactly an ISO date becomes a date, an ISO date with a time a date-time.
fn classify(s: &str) -> Value {
    if is_iso_date(s) {
        Value::Date { value: s.into() }
    } else if is_iso_datetime(s) {
        Value::DateTime { value: s.into() }
    } else {
        Value::Text { value: s.into() }
    }
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_iso_datetime(s: &str) -> bool {
    // `YYYY-MM-DD`, `T` or space, `HH:MM`, then anything an ISO time may carry.
    let Some(date) = s.get(..10) else {
        return false;
    };
    let rest = &s.as_bytes()[10..];
    is_iso_date(date)
        && rest.len() >= 6
        && (rest[0] == b'T' || rest[0] == b't' || rest[0] == b' ')
        && rest[1..3].iter().all(u8::is_ascii_digit)
        && rest[3] == b':'
        && rest[4..6].iter().all(u8::is_ascii_digit)
        && rest[6..]
            .iter()
            .all(|c| c.is_ascii_digit() || b":.+-zZ".contains(c))
}
