// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The assertion file: one relational fact about a document per line.
//!
//! ```text
//! # a comment
//! row: A | B@2 | C [table]
//! same_row: A | B [table]
//! not_row: A | B [table]
//! cell_under: HEADER => VALUE [table]
//! pair: LABEL == VALUE [pair]
//! heading: TEXT [heading]
//! heading: TEXT level 2 [heading]
//! order: A << B [order]
//! text: TEXT [text]
//! link: URL [link]
//! ```
//!
//! Every fact ends with its class in brackets. The texts of a fact are parted by ` | `, ` => `,
//! ` << ` or ` == `, and only where the separator has a space on each side: `<1080>`, `a=b`
//! and `KL. / CL.` stay whole. A text that holds a literal `|` writes it as `\|`.
//!
//! A text may start with `~` (the place holds the text, at word bounds, among other text),
//! with `#` (the place holds this amount, however it is printed: `#1234.5` is found in
//! `1.234,50 EUR`) or be `*` alone (any text; for the label of a pair whose source states no
//! label). `@N` at its end names the N-th line of a table cell. A backslash takes the next
//! character as it is, for a text that holds one of `| [ ] @ ~ # *` or a backslash, or a word
//! that is a separator (`\==`). White space is normalised and `<br>` is a line break. The
//! target of a `link` is taken as written. The README of the crate has the full rules.

use std::fmt;

use crate::text::{Number, norm};

/// What a fact counts for in the scorecard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Table,
    Pair,
    Order,
    Heading,
    Text,
    Link,
}

impl Class {
    /// Every class, in the order of the scorecard.
    pub const ALL: [Class; 6] = [
        Class::Table,
        Class::Pair,
        Class::Order,
        Class::Heading,
        Class::Text,
        Class::Link,
    ];

    /// The name in the brackets of a fact line.
    pub fn name(self) -> &'static str {
        match self {
            Class::Table => "table",
            Class::Pair => "pair",
            Class::Order => "order",
            Class::Heading => "heading",
            Class::Text => "text",
            Class::Link => "link",
        }
    }

    fn of(name: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// How a text of a fact is compared with a place of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The place holds exactly this text.
    Exact,
    /// The place holds this text at word bounds, among other text.
    Contains,
    /// The place holds this amount.
    Number(Number),
    /// The place holds any text.
    Any,
}

/// One text of a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub mode: Mode,
    /// The text, normalised. Empty for [`Mode::Number`] and [`Mode::Any`].
    pub text: String,
    /// The 1-based line of the cell the text is, when the fact names one.
    pub line: Option<usize>,
}

impl Term {
    /// The place holds exactly `text`.
    pub fn exact(text: &str) -> Term {
        Term {
            mode: Mode::Exact,
            text: norm(text),
            line: None,
        }
    }

    /// The place holds `text` among other text.
    pub fn contains(text: &str) -> Term {
        Term {
            mode: Mode::Contains,
            text: norm(text),
            line: None,
        }
    }

    /// The place holds the amount `number`.
    pub fn number(number: Number) -> Term {
        Term {
            mode: Mode::Number(number),
            text: String::new(),
            line: None,
        }
    }

    /// The place holds any text.
    pub fn any() -> Term {
        Term {
            mode: Mode::Any,
            text: String::new(),
            line: None,
        }
    }

    /// The same text as the `line`-th line (from 1) of its cell.
    pub fn at_line(mut self, line: usize) -> Term {
        self.line = Some(line);
        self
    }

    /// Whether the term names no text at all: an empty exact or contained text.
    fn is_empty(&self) -> bool {
        matches!(self.mode, Mode::Exact | Mode::Contains) && self.text.is_empty()
    }
}

/// The characters a backslash is written before.
const SPECIAL: [char; 8] = ['\\', '|', '[', ']', '@', '~', '#', '*'];

/// The separators of the texts of a fact. Each parts texts only with a space on each side.
const SEPARATORS: [&str; 4] = ["|", "=>", "<<", "=="];

/// `text` as a fact line writes it: a backslash before every special character and before a
/// word that is a separator.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, word) in text.split(' ').enumerate() {
        if n > 0 {
            out.push(' ');
        }
        if SEPARATORS.contains(&word) && word != "|" {
            out.push('\\');
        }
        for c in word.chars() {
            if SPECIAL.contains(&c) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.mode {
            Mode::Exact => write!(f, "{}", escape(&self.text))?,
            Mode::Contains => write!(f, "~{}", escape(&self.text))?,
            Mode::Number(n) => write!(f, "#{n}")?,
            Mode::Any => write!(f, "*")?,
        }
        if let Some(line) = self.line {
            write!(f, "@{line}")?;
        }
        Ok(())
    }
}

/// What a fact says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The texts stand in one table row, in this column order; other cells may lie between.
    Row(Vec<Term>),
    /// The texts stand in one table row, each in a cell of its own, in any order. For truth
    /// that does not state the order of the printed columns (the data of an invoice).
    SameRow(Vec<Term>),
    /// The two texts never stand in one table row.
    NotRow(Term, Term),
    /// The value is in the column whose header cell holds the header.
    CellUnder { header: Term, value: Term },
    /// A label and its value.
    Pair { label: Term, value: Term },
    /// The text is a heading, of this level when one is given.
    Heading { text: Term, level: Option<u8> },
    /// The first text comes before the second in reading order.
    Order(Term, Term),
    /// The text is in the output, in one block.
    Text(Term),
    /// One link has this target.
    Link(String),
}

/// One fact of an assertion file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub kind: Kind,
    pub class: Class,
}

impl Fact {
    pub fn row(terms: Vec<Term>) -> Fact {
        Fact {
            kind: Kind::Row(terms),
            class: Class::Table,
        }
    }

    pub fn same_row(terms: Vec<Term>) -> Fact {
        Fact {
            kind: Kind::SameRow(terms),
            class: Class::Table,
        }
    }

    pub fn not_row(a: Term, b: Term) -> Fact {
        Fact {
            kind: Kind::NotRow(a, b),
            class: Class::Table,
        }
    }

    pub fn cell_under(header: Term, value: Term) -> Fact {
        Fact {
            kind: Kind::CellUnder { header, value },
            class: Class::Table,
        }
    }

    pub fn pair(label: Term, value: Term) -> Fact {
        Fact {
            kind: Kind::Pair { label, value },
            class: Class::Pair,
        }
    }

    pub fn heading(text: &str, level: Option<u8>) -> Fact {
        Fact {
            kind: Kind::Heading {
                text: Term::exact(text),
                level,
            },
            class: Class::Heading,
        }
    }

    pub fn order(a: Term, b: Term) -> Fact {
        Fact {
            kind: Kind::Order(a, b),
            class: Class::Order,
        }
    }

    pub fn text(text: Term) -> Fact {
        Fact {
            kind: Kind::Text(text),
            class: Class::Text,
        }
    }

    pub fn link(target: &str) -> Fact {
        Fact {
            kind: Kind::Link(target.trim().to_string()),
            class: Class::Link,
        }
    }

    /// The texts of the fact, in the order it names them.
    pub fn terms(&self) -> Vec<&Term> {
        match &self.kind {
            Kind::Row(t) | Kind::SameRow(t) => t.iter().collect(),
            Kind::NotRow(a, b) | Kind::Order(a, b) => vec![a, b],
            Kind::CellUnder { header, value } => vec![header, value],
            Kind::Pair { label, value } => vec![label, value],
            Kind::Heading { text, .. } | Kind::Text(text) => vec![text],
            Kind::Link(_) => Vec::new(),
        }
    }
}

/// `terms` joined by `separator`.
fn joined(terms: &[Term], separator: &str) -> String {
    let parts: Vec<String> = terms.iter().map(Term::to_string).collect();
    parts.join(separator)
}

impl fmt::Display for Fact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Row(t) => write!(f, "row: {}", joined(t, " | "))?,
            Kind::SameRow(t) => write!(f, "same_row: {}", joined(t, " | "))?,
            Kind::NotRow(a, b) => write!(f, "not_row: {a} | {b}")?,
            Kind::CellUnder { header, value } => write!(f, "cell_under: {header} => {value}")?,
            Kind::Pair { label, value } => write!(f, "pair: {label} == {value}")?,
            Kind::Heading { text, level } => {
                // A text that ends as a level does is written with a backslash in it.
                let written = text.to_string();
                match level_of(&written) {
                    Some((head, _)) => {
                        write!(f, "heading: {head} \\level{}", &written[head.len() + 6..])?
                    }
                    None => write!(f, "heading: {written}")?,
                }
                if let Some(level) = level {
                    write!(f, " level {level}")?;
                }
            }
            Kind::Order(a, b) => write!(f, "order: {a} << {b}")?,
            Kind::Text(t) => write!(f, "text: {t}")?,
            Kind::Link(target) => write!(f, "link: {target}")?,
        }
        write!(f, " [{}]", self.class.name())
    }
}

/// `text` without a ` level N` at its end, and that level.
fn level_of(text: &str) -> Option<(&str, u8)> {
    let digit = text.chars().next_back().filter(char::is_ascii_digit)?;
    let level = u8::try_from(digit.to_digit(10)?)
        .ok()
        .filter(|n| (1..=6).contains(n))?;
    Some((text[..text.len() - 1].strip_suffix(" level ")?, level))
}

/// The parts of `text` between the places where `separator` stands with a space on each
/// side. A separator with a backslash before it has no space there, so it is text.
fn split<'a>(text: &'a str, separator: &str) -> Vec<&'a str> {
    text.split(&format!(" {separator} ")).collect()
}

/// One text of a fact line.
fn term(raw: &str) -> Result<Term, String> {
    let raw = raw.trim();
    if raw == "*" {
        return Ok(Term::any());
    }
    // A line number at the end: `@` with no backslash before it, then digits.
    let mut body = raw;
    let mut line = None;
    if let Some(at) = raw.rfind('@') {
        let digits = &raw[at + 1..];
        let escaped = raw[..at].chars().rev().take_while(|c| *c == '\\').count() % 2 == 1;
        if !escaped && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            let n: usize = digits.parse().map_err(|_| "a line number is too large")?;
            if n == 0 {
                return Err("lines are counted from 1".into());
            }
            line = Some(n);
            body = &raw[..at];
        }
    }
    let (mode, body) = match body.chars().next() {
        Some('~') => (Mode::Contains, &body[1..]),
        Some('#') => {
            let number = Number::parse(&body[1..])
                .ok_or("an amount is digits with at most one point, as in #1234.5")?;
            (Mode::Number(number), "")
        }
        _ => (Mode::Exact, body),
    };
    let mut text = String::with_capacity(body.len());
    let mut escaped = false;
    for c in body.chars() {
        if escaped || c != '\\' {
            text.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    let term = Term {
        mode,
        text: norm(&text),
        line,
    };
    if term.is_empty() {
        return Err("a text is empty".into());
    }
    Ok(term)
}

/// The texts of `body` parted by `separator`: at least `least`, at most `most`.
fn terms(body: &str, separator: &str, least: usize, most: usize) -> Result<Vec<Term>, String> {
    let parts = split(body, separator);
    if parts.len() < least || parts.len() > most {
        return Err(format!(
            "the fact takes {least} to {most} texts parted by ` {separator} `"
        ));
    }
    parts.into_iter().map(term).collect()
}

/// The two texts of `body` parted by `separator`.
fn two(body: &str, separator: &str) -> Result<(Term, Term), String> {
    let mut both = terms(body, separator, 2, 2)?.into_iter();
    match (both.next(), both.next()) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err("the fact takes two texts".into()),
    }
}

/// The fact of one line that is no comment. The error says what is wrong and never repeats
/// the line's text.
fn fact(line: &str) -> Result<Fact, String> {
    let line = line.trim();
    let open = line.rfind(" [").ok_or("the class in brackets is missing")?;
    let class = line[open + 2..]
        .strip_suffix(']')
        .and_then(Class::of)
        .ok_or("the class is none of table, pair, order, heading, text, link")?;
    let (name, body) = line[..open]
        .split_once(':')
        .ok_or("the kind of the fact is missing")?;
    let body = body.trim();
    const MANY: usize = 64;
    let kind = match name.trim() {
        "row" => Kind::Row(terms(body, "|", 1, MANY)?),
        "same_row" => Kind::SameRow(terms(body, "|", 2, 8)?),
        "not_row" => {
            let (a, b) = two(body, "|")?;
            Kind::NotRow(a, b)
        }
        "cell_under" => {
            let (header, value) = two(body, "=>")?;
            Kind::CellUnder { header, value }
        }
        "pair" => {
            let (label, value) = two(body, "==")?;
            Kind::Pair { label, value }
        }
        "heading" => {
            let (text, level) = match level_of(body) {
                Some((text, level)) => (text, Some(level)),
                None => (body, None),
            };
            Kind::Heading {
                text: term(text)?,
                level,
            }
        }
        "order" => {
            let (a, b) = two(body, "<<")?;
            Kind::Order(a, b)
        }
        "text" => Kind::Text(term(body)?),
        "link" if body.is_empty() => return Err("the target is empty".into()),
        "link" => Kind::Link(body.to_string()),
        _ => return Err("the kind of the fact is unknown".into()),
    };
    let fact = Fact { kind, class };
    // `*` stands for the label of a pair and nothing else.
    let any = |t: &Term| t.mode == Mode::Any;
    let allowed = match &fact.kind {
        Kind::Pair { value, .. } => !any(value),
        _ => !fact.terms().into_iter().any(any),
    };
    if !allowed {
        return Err("`*` stands only for the label of a pair".into());
    }
    Ok(fact)
}

/// A fact with the line it was written as, for the list of failing facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub fact: Fact,
    /// The 1-based line of the file.
    pub number: usize,
    pub line: String,
}

/// The facts of an assertion file. Empty lines and lines that start with `#` are skipped. An
/// error names the line by its number and never repeats its text.
pub fn parse(text: &str) -> Result<Vec<Written>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fact = fact(trimmed).map_err(|why| format!("line {}: {why}", n + 1))?;
        out.push(Written {
            fact,
            number: n + 1,
            line: trimmed.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Fact {
        parse(line).unwrap().remove(0).fact
    }

    #[test]
    fn every_kind_parses() {
        let text = "# facts\n\nrow: A | B@2 | C [table]\nsame_row: ~A | #1.50 [table]\n\
                    not_row: A | B [table]\ncell_under: H => V [table]\npair: L == V [pair]\n\
                    pair: * == #10 [pair]\nheading: T [heading]\nheading: T level 2 [heading]\n\
                    order: A << B [order]\ntext: T [text]\nlink: https://example.org/a?b=c [link]\n";
        let facts = parse(text).unwrap();
        assert_eq!(facts.len(), 11);
        assert_eq!(facts[0].number, 3);
        assert_eq!(facts[0].line, "row: A | B@2 | C [table]");
        assert_eq!(
            facts[0].fact,
            Fact::row(vec![
                Term::exact("A"),
                Term::exact("B").at_line(2),
                Term::exact("C")
            ])
        );
        assert_eq!(
            facts[1].fact,
            Fact::same_row(vec![
                Term::contains("A"),
                Term::number(Number::parse("1.5").unwrap())
            ])
        );
        assert_eq!(facts[7].fact, Fact::heading("T", Some(2)));
        assert_eq!(facts[10].fact, Fact::link("https://example.org/a?b=c"));
    }

    #[test]
    fn written_facts_read_back_the_same() {
        let hard = "a | b == c << d => e = f < g / h [i] @2 ~j #k * \\ level 3";
        let facts = [
            Fact::row(vec![Term::exact(hard), Term::contains("~x").at_line(3)]),
            Fact::pair(Term::any(), Term::number(Number::parse("12.30").unwrap())),
            Fact::pair(Term::exact("E-Mail:"), Term::exact("a@b.example")),
            Fact::heading(hard, None),
            Fact::heading(hard, Some(4)),
            Fact::heading("Stufe level 2", Some(1)),
            Fact::heading("Geb\u{fc}hr 5 \u{20ac}", None),
            Fact::heading("\u{20ac}", Some(6)),
            Fact::order(Term::contains("a<b"), Term::contains("c")),
            Fact::cell_under(Term::exact("km/h"), Term::exact("1/2")),
            Fact::not_row(Term::exact("*"), Term::exact("#1")),
            Fact::text(Term::contains("x  y")),
            Fact::link("https://example.org/[1]"),
        ];
        for f in facts {
            let line = f.to_string();
            assert_eq!(one(&line), f, "{line}");
        }
    }

    #[test]
    fn a_separator_parts_texts_only_with_a_space_on_each_side() {
        assert_eq!(
            one("order: a<br>b <1080> << c<<d [order]"),
            Fact::order(Term::exact("a b <1080>"), Term::exact("c<<d"))
        );
        assert_eq!(
            one("pair: a=b == c==d [pair]"),
            Fact::pair(Term::exact("a=b"), Term::exact("c==d"))
        );
        assert_eq!(
            one("cell_under: KL. / CL. => 1/2 [table]"),
            Fact::cell_under(Term::exact("KL. / CL."), Term::exact("1/2"))
        );
        assert_eq!(
            one("row: a|b | c \\| d | e [table]"),
            Fact::row(vec![
                Term::exact("a|b"),
                Term::exact("c | d"),
                Term::exact("e")
            ])
        );
        // A text that holds a separator as a word is written with a backslash.
        let fact = Fact::pair(Term::exact("a == b"), Term::exact("== c <<"));
        assert_eq!(fact.to_string(), "pair: a \\== b == \\== c \\<< [pair]");
        assert_eq!(one(&fact.to_string()), fact);
    }

    #[test]
    fn a_class_may_differ_from_the_kind() {
        assert_eq!(one("row: Summe | 10,00 [pair]").class, Class::Pair);
    }

    #[test]
    fn errors_name_the_line_and_not_its_text() {
        for (text, part) in [
            ("row: Geheim | Wert", "class in brackets is missing"),
            ("row: Geheim | Wert [tables]", "class is none of"),
            ("rows: Geheim | Wert [table]", "kind of the fact is unknown"),
            ("Geheim [table]", "kind of the fact is missing"),
            ("pair: Geheim [pair]", "2 to 2 texts"),
            ("pair: == Geheim [pair]", "2 to 2 texts"),
            ("row: Geheim |  | Wert [table]", "a text is empty"),
            ("row: Geheim@0 [table]", "counted from 1"),
            ("same_row: #1,5 | Geheim [table]", "an amount is digits"),
            ("row: * | Geheim [table]", "only for the label"),
            ("pair: Geheim == * [pair]", "only for the label"),
            ("link: [link]", "target is empty"),
        ] {
            let err = parse(&format!("# first\n{text}\n")).unwrap_err();
            assert!(err.starts_with("line 2: "), "{err}");
            assert!(err.contains(part), "{err}");
            assert!(!err.contains("Geheim"), "{err}");
        }
    }
}
