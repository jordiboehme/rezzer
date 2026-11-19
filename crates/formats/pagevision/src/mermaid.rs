// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small validity check for the Mermaid a model writes: flowcharts and sequence diagrams.
//! It is not a renderer. Every other kind is named as not checked.

/// Checks one Mermaid source. Supported:
/// - `graph` and `flowchart` with a direction (TD, TB, BT, LR, RL): node ids with `[]`, `()`,
///   `{}` labels and their nested forms, the edges `-->`, `---`, `<-->`, `-.->`, `==>` and their
///   longer forms, `|label|` and `-- label -->`, `&` between nodes, `;` between statements,
///   `%%` comments and the `style`, `classDef`, `class`, `linkStyle`, `click`, `subgraph` and
///   `end` lines;
/// - `sequenceDiagram` with `participant` and `actor` (with `as`), the messages `->>`, `-->>`,
///   `->` and `-->` (and the `x` and `)` ends), `Note`, `activate`, `deactivate`, `autonumber`,
///   `title` and the blocks `loop`, `alt`, `else`, `opt`, `par`, `and`, `rect`, `critical`,
///   `break` and `end`.
///
/// Every other kind is `Err("mermaid kind X is not checked")`.
pub fn check(src: &str) -> Result<(), String> {
    let mut lines = src
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let Some(first) = lines.next() else {
        return Err("mermaid kind (empty) is not checked".to_string());
    };
    let mut pieces = split_statements(first).into_iter();
    let header = pieces.next().unwrap_or_default();
    let mut words = header.split_whitespace();
    let kind = words.next().unwrap_or("");
    let rest: Vec<String> = pieces
        .chain(lines.flat_map(split_statements))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with("%%"))
        .collect();
    match kind {
        "graph" | "flowchart" => {
            match words.next() {
                Some(d) if DIRECTIONS.contains(&d) => {}
                Some(d) => return Err(format!("{kind} has the unknown direction {d:?}")),
                None => return Err(format!("{kind} needs a direction")),
            }
            if let Some(extra) = words.next() {
                return Err(format!("unexpected {extra:?} after the direction"));
            }
            rest.iter().try_for_each(|s| flow_statement(s))
        }
        "sequenceDiagram" => {
            if let Some(extra) = words.next() {
                return Err(format!("unexpected {extra:?} after sequenceDiagram"));
            }
            rest.iter().try_for_each(|s| sequence_statement(s))
        }
        other => Err(format!("mermaid kind {other} is not checked")),
    }
}

const DIRECTIONS: [&str; 5] = ["TD", "TB", "BT", "LR", "RL"];

/// Splits a line at `;`, outside quotes and brackets.
fn split_statements(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut cur) = (0usize, false, String::new());
    for c in line.chars() {
        match c {
            '"' => quote = !quote,
            '[' | '(' | '{' if !quote => depth += 1,
            ']' | ')' | '}' if !quote => depth = depth.saturating_sub(1),
            ';' if !quote && depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

// ---- flowcharts ----

/// The first words of flowchart lines that hold no nodes or edges to check.
const FLOW_SKIP: [&str; 8] = [
    "style",
    "classDef",
    "class",
    "linkStyle",
    "click",
    "subgraph",
    "end",
    "direction",
];

/// A cursor over the characters of one statement.
struct Scan<'a> {
    s: &'a str,
    chars: Vec<char>,
    i: usize,
}

impl Scan<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn at(&self, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(k, c)| self.chars.get(self.i + k) == Some(&c))
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn done(&self) -> bool {
        self.i >= self.chars.len()
    }

    fn err(&self, what: &str) -> String {
        format!("{what} in {:?}", self.s)
    }

    /// Finds `text` from the cursor on, outside quotes; returns its index.
    fn find(&self, text: &str) -> Option<usize> {
        let want: Vec<char> = text.chars().collect();
        let mut quote = false;
        let mut k = self.i;
        while k + want.len() <= self.chars.len() {
            if self.chars[k] == '"' {
                quote = !quote;
            } else if !quote && self.chars[k..k + want.len()] == want[..] {
                return Some(k);
            }
            k += 1;
        }
        None
    }
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

fn flow_statement(stmt: &str) -> Result<(), String> {
    let first = stmt.split_whitespace().next().unwrap_or("");
    if FLOW_SKIP.contains(&first) {
        return Ok(());
    }
    let mut sc = Scan {
        s: stmt,
        chars: stmt.chars().collect(),
        i: 0,
    };
    node_group(&mut sc)?;
    loop {
        sc.skip_ws();
        if sc.done() {
            return Ok(());
        }
        edge(&mut sc)?;
        sc.skip_ws();
        if sc.peek() == Some('|') {
            sc.i += 1;
            let close = sc.find("|").ok_or_else(|| sc.err("unclosed arrow label"))?;
            sc.i = close + 1;
        }
        sc.skip_ws();
        if sc.done() {
            return Err(sc.err("an arrow without a target"));
        }
        node_group(&mut sc)?;
    }
}

/// `A`, `A[label]` or `A & B & C`.
fn node_group(sc: &mut Scan) -> Result<(), String> {
    node(sc)?;
    loop {
        let save = sc.i;
        sc.skip_ws();
        if sc.peek() == Some('&') {
            sc.i += 1;
            sc.skip_ws();
            node(sc)?;
        } else {
            sc.i = save;
            return Ok(());
        }
    }
}

fn node(sc: &mut Scan) -> Result<(), String> {
    sc.skip_ws();
    let start = sc.i;
    while sc.peek().is_some_and(is_id_char) {
        sc.i += 1;
    }
    if sc.i == start {
        return Err(sc.err("expected a node"));
    }
    if sc
        .peek()
        .is_some_and(|c| matches!(c, '[' | '(' | '{' | '>'))
    {
        shape(sc)?;
    }
    if sc.at(":::") {
        sc.i += 3;
        while sc.peek().is_some_and(|c| is_id_char(c) || c == '-') {
            sc.i += 1;
        }
    }
    Ok(())
}

/// A node label: `[x]`, `(x)`, `{x}`, their nested forms like `((x))`, `[(x)]` or `{{x}}`, and
/// `>x]`. The label may be quoted.
fn shape(sc: &mut Scan) -> Result<(), String> {
    let close: String = if sc.peek() == Some('>') {
        sc.i += 1;
        "]".to_string()
    } else {
        let mut close = String::new();
        while let Some(c) = sc.peek().filter(|c| matches!(c, '[' | '(' | '{')) {
            if close.len() == 3 {
                break;
            }
            close.insert(
                0,
                match c {
                    '[' => ']',
                    '(' => ')',
                    _ => '}',
                },
            );
            sc.i += 1;
        }
        close
    };
    let end = sc
        .find(&close)
        .ok_or_else(|| sc.err(&format!("unclosed label, no {close:?}")))?;
    sc.i = end + close.chars().count();
    Ok(())
}

/// One arrow: a solid (`-->`, `---`), thick (`==>`, `===`) or dotted (`-.->`, `-.-`) line of any
/// length, an optional `<` start for both heads, a `>`, `x` or `o` head, or the text forms
/// `-- text -->`, `== text ==>` and `-. text .->`.
fn edge(sc: &mut Scan) -> Result<(), String> {
    let start = sc.i;
    let both = sc.peek() == Some('<');
    if both {
        sc.i += 1;
    }
    let run_start = sc.i;
    while sc.peek().is_some_and(|c| matches!(c, '-' | '=' | '.')) {
        sc.i += 1;
    }
    let run: String = sc.chars[run_start..sc.i].iter().collect();
    // A text arrow: `--`, `==` or `-.` then text, closed by an arrow.
    if !both
        && matches!(run.as_str(), "--" | "==" | "-.")
        && sc.peek().is_some_and(char::is_whitespace)
    {
        let closers: &[&str] = match run.as_str() {
            "--" => &["-->", "---"],
            "==" => &["==>", "==="],
            _ => &[".->", ".-"],
        };
        let text_start = sc.i;
        let found = closers
            .iter()
            .filter_map(|c| sc.find(c).map(|k| (k, c.chars().count())))
            .min();
        let Some((k, len)) = found else {
            return Err(sc.err("unclosed arrow text"));
        };
        let text: String = sc.chars[text_start..k].iter().collect();
        if text.trim().is_empty() {
            return Err(sc.err("empty arrow text"));
        }
        sc.i = k + len;
        // Longer closers: `--->`.
        while sc.peek().is_some_and(|c| matches!(c, '-' | '=')) {
            sc.i += 1;
        }
        if sc.peek() == Some('>') {
            sc.i += 1;
        }
        return Ok(());
    }
    let head = match sc.peek() {
        Some('>') => {
            sc.i += 1;
            true
        }
        Some('x' | 'o') if sc.chars.get(sc.i + 1).is_none_or(|c| c.is_whitespace()) => {
            sc.i += 1;
            true
        }
        _ => false,
    };
    let solid = run.len() >= 2 && run.chars().all(|c| c == '-');
    let thick = run.len() >= 2 && run.chars().all(|c| c == '=');
    let dotted = run.len() >= 3
        && run.starts_with('-')
        && run.ends_with('-')
        && run[1..run.len() - 1].chars().all(|c| c == '.');
    let ok = if head {
        solid || thick || dotted
    } else {
        !both && ((solid || thick) && run.len() >= 3 || dotted)
    };
    if ok {
        Ok(())
    } else {
        let text: String = sc.chars[start..sc.i.max(start + 1).min(sc.chars.len())]
            .iter()
            .collect();
        Err(sc.err(&format!("expected an arrow, found {text:?}")))
    }
}

// ---- sequence diagrams ----

/// Message arrows, longest first.
const SEQ_ARROWS: [&str; 8] = ["-->>", "->>", "--x", "--)", "-->", "-x", "-)", "->"];

/// Lines that open or close a block, or set an option.
const SEQ_WORDS: [&str; 14] = [
    "autonumber",
    "title",
    "end",
    "else",
    "and",
    "box",
    "loop",
    "alt",
    "opt",
    "par",
    "rect",
    "critical",
    "break",
    "option",
];

fn sequence_statement(stmt: &str) -> Result<(), String> {
    let mut words = stmt.splitn(2, char::is_whitespace);
    let first = words.next().unwrap_or("");
    let rest = words.next().unwrap_or("").trim();
    match first.to_lowercase().as_str() {
        "participant" | "actor" | "activate" | "deactivate" => {
            return if rest.is_empty() {
                Err(format!("{first} without a name in {stmt:?}"))
            } else {
                Ok(())
            };
        }
        "note" => {
            return if stmt.contains(':') {
                Ok(())
            } else {
                Err(format!("a note without text in {stmt:?}"))
            };
        }
        w if SEQ_WORDS.contains(&w) => return Ok(()),
        _ => {}
    }
    let found = stmt.match_indices('-').find_map(|(d, _)| {
        let left = &stmt[..d];
        if left.trim().is_empty() || left.contains(':') {
            return None;
        }
        SEQ_ARROWS
            .iter()
            .find(|a| stmt[d..].starts_with(**a))
            .map(|a| (d, *a))
    });
    let Some((dash, arrow)) = found else {
        return Err(format!("not a message in {stmt:?}"));
    };
    let right = stmt[dash + arrow.len()..].trim_start();
    let right = right.strip_prefix(['+', '-']).unwrap_or(right);
    let target = right.split(':').next().unwrap_or("").trim();
    if target.is_empty() {
        return Err(format!("a message without a target in {stmt:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edges of `tests/corpus/vision/test2.edges`, written as a flowchart.
    const TEST2_TRUTH: &str = "flowchart TD\n    search --> Learn\n    Learn --> teach\n    teach --> search\n    App --> DB\n    MCP <--> DB\n";

    #[test]
    fn mermaid_check_accepts_test2_truth() {
        assert_eq!(check(TEST2_TRUTH), Ok(()));
    }

    #[test]
    fn mermaid_check_rejects_unclosed_label() {
        let err = check("graph TD\n    A[Start --> B").unwrap_err();
        assert!(err.contains("unclosed"), "{err}");
        let err = check("graph TD\n    A((Start) --> B").unwrap_err();
        assert!(err.contains("unclosed"), "{err}");
        let err = check("graph TD\n    A -->|yes B").unwrap_err();
        assert!(err.contains("unclosed"), "{err}");
    }

    #[test]
    fn mermaid_check_names_unchecked_kinds() {
        assert_eq!(
            check("pie title Pets\n  \"Dogs\" : 5"),
            Err("mermaid kind pie is not checked".to_string())
        );
        assert_eq!(
            check("%% c\nclassDiagram\n  A <|-- B"),
            Err("mermaid kind classDiagram is not checked".to_string())
        );
        assert_eq!(
            check(""),
            Err("mermaid kind (empty) is not checked".to_string())
        );
    }

    /// Forms a model writes: the Mermaid of the harness tests, the Chandra fixture and common
    /// variants.
    #[test]
    fn mermaid_check_accepts_common_forms() {
        let ok = [
            "graph TD\n    search --> Learn\n    Learn --> teach\n    teach --> search",
            "graph LR\n    App --> DB\n    DB <--> MCP",
            "graph TD; search((search)) --> teach((teach)); teach --> learn((learn)); learn --> search;",
            "graph LR; App((App)) --> DB[(DB)]; DB --> MCP[MCP];",
            "graph TD; A[Notedown<br/>CLI] --> B[(Server)];",
            "graph LR\n  A[md] --> B[mcp] --> C[claude ai agent]",
            "graph LR\n  A -- uses --> B",
            "graph LR\n  A -- e-mail --> B",
            "graph LR\n  A[Notedown] --> B[OCR model (qwen)]",
            "graph TD; A --- B --> C",
            "graph TD\n  A --- B\n  B <--> C\n  C -.-> D\n  D ==> E\n  A[First]\n  endpoint --> A\n",
            "flowchart LR\n  %% a comment\n  style A fill:#fff\n  subgraph S\n  A -->|via| B\n  end",
            "flowchart TB\n  A[\"Quoted [label]\"] --> B{Decision}\n  B -->|yes| C([Stadium])\n  B -- no --> D[[Sub]]\n  D --> E{{Hex}}\n  E --> F>Flag]\n  F --> G[/Para/]",
            "flowchart BT\n  A & B --> C\n  C ---> D\n  D ===> E\n  E -. dotted .-> F\n  F == thick ==> G\n  G --x H\n  H --o I",
            "graph RL\n  classDef big font-size:20px\n  class A big\n  linkStyle 0 stroke:#f00\n  A:::big --> B\n  click A callback",
            "graph TD\n  Notedown <--> SN[Super Note]\n  Notedown --> OCR[OCR Model] --> MD[.md]\n  MD -->|mcp| Claude[Claude AI Agent] <--> Crystalline",
            "graph TD\n  A[\"UI (macOS menu bar item)\"] --> B[notedown CLI]\n  B --> C[notedown server]\n  C --> D[OCR]\n  C --> E[Filesystem Files]",
            "sequenceDiagram; participant U as User; U->>UI: click; UI-->>U: done",
            "sequenceDiagram\n  Web-Server->>DB: query",
            "sequenceDiagram\n    autonumber\n    actor User\n    participant UI\n    participant S as Service\n    User->>UI: click\n    activate UI\n    UI->>+S: call\n    S-->>-UI: answer\n    Note right of UI: wait\n    loop every minute\n      UI-)S: ping\n    end\n    alt ok\n      UI->User: show\n    else error\n      UI--xUser: fail\n    end\n    deactivate UI",
        ];
        for src in ok {
            assert_eq!(check(src), Ok(()), "{src}");
        }
    }

    #[test]
    fn mermaid_check_rejects_broken_statements() {
        let bad = [
            "graph",
            "graph XY\n  A --> B",
            "graph TD\n  A -->",
            "graph TD\n  --> B",
            "graph TD\n  A --> B C",
            "graph TD\n  A -> B",
            "graph TD\n  A[x] y --> B",
            "graph TD\n  A -- text B",
            "sequenceDiagram\n  A->>: hi",
            "sequenceDiagram\n  just words",
        ];
        for src in bad {
            assert!(check(src).is_err(), "{src}");
        }
    }
}
