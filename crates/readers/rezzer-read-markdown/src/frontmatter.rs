// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Frontmatter (YAML between `---`, TOML between `+++`) mapped onto document properties.

use rezzer_model::{Properties, Value};

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

/// The deepest nesting a YAML block may have. This is also the recursion limit of the parser,
/// but its running time is quadratic in flow nesting, so the depth is checked up front.
const MAX_YAML_DEPTH: usize = 128;

/// Parses `text` and fills `props`. On an error nothing is changed and the message is returned.
///
/// Values keep their data but not their YAML tags (`!foo bar` reads as `bar`). A key with
/// no value is kept as an empty text.
pub(crate) fn apply(syntax: Syntax, text: &str, props: &mut Properties) -> Result<(), String> {
    let root = match syntax {
        Syntax::Yaml => {
            if let Some(depth) = yaml_depth_over_limit(text) {
                return Err(format!(
                    "nesting of {depth} levels is deeper than the {MAX_YAML_DEPTH} allowed"
                ));
            }
            let v: serde_yaml_ng::Value =
                serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
            from_yaml(v)
        }
        Syntax::Toml => {
            let t: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
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
        _ => Err("frontmatter is not a mapping of keys to values".into()),
    }
}

/// One linear pass over the text: the depth of flow brackets (`[` and `{`, quotes ignored so
/// the count can only be too high, never too low) and of block indentation, including compact
/// `- - - x` nesting. Returns the depth when it is over the limit.
fn yaml_depth_over_limit(text: &str) -> Option<usize> {
    let mut flow: usize = 0;
    let mut indents: Vec<usize> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start_matches(' ');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if flow == 0 {
            while indents.last().is_some_and(|&i| i >= indent) {
                indents.pop();
            }
            indents.push(indent);
            // Compact nesting: `- - - x`.
            let mut rest = trimmed;
            let mut extra = 0usize;
            while let Some(r) = rest
                .strip_prefix("- ")
                .or_else(|| rest.strip_prefix("? "))
                .map(|r| r.trim_start_matches(' '))
            {
                extra += 1;
                rest = r;
            }
            if indents.len() + extra > MAX_YAML_DEPTH {
                return Some(indents.len() + extra);
            }
        }
        for b in line.bytes() {
            match b {
                b'[' | b'{' => {
                    flow += 1;
                    if flow + indents.len() > MAX_YAML_DEPTH {
                        return Some(flow + indents.len());
                    }
                }
                b']' | b'}' => flow = flow.saturating_sub(1),
                _ => {}
            }
        }
    }
    None
}

fn from_yaml(v: serde_yaml_ng::Value) -> Fm {
    use serde_yaml_ng::Value as Y;
    match v {
        Y::Null => Fm::Null,
        Y::Bool(b) => Fm::Bool(b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                Fm::Int(i128::from(i))
            } else if let Some(u) = n.as_u64() {
                Fm::Int(i128::from(u))
            } else {
                Fm::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Y::String(s) => Fm::Str(s),
        Y::Sequence(items) => Fm::List(items.into_iter().map(from_yaml).collect()),
        Y::Mapping(m) => Fm::Map(
            m.into_iter()
                .map(|(k, v)| (yaml_key(k), from_yaml(v)))
                .collect(),
        ),
        Y::Tagged(t) => from_yaml(t.value),
    }
}

fn yaml_key(k: serde_yaml_ng::Value) -> String {
    use serde_yaml_ng::Value as Y;
    match k {
        Y::String(s) => s,
        Y::Bool(b) => b.to_string(),
        Y::Number(n) => n.to_string(),
        Y::Null => "null".into(),
        other => serde_yaml_ng::to_string(&other)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
    }
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
