// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tokens and values of ISO 32000-1, 7.2 and 7.3: white space and comments, numbers, names
//! with `#` escapes, literal and hex strings, arrays, dictionaries, references and the
//! keywords `true`, `false` and `null`. Every value is charged to the [`Budget`] before it is
//! kept, and nesting is bounded.

use lopdf::{Dictionary, Object, StringFormat};

use super::{Budget, Fail};

/// White space (7.2.3).
pub(crate) fn is_space(c: u8) -> bool {
    matches!(c, 0 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

/// Delimiters (7.2.3).
pub(crate) fn is_delimiter(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(c: u8) -> bool {
    !is_space(c) && !is_delimiter(c)
}

/// Reals are kept within this size, so lopdf reads back the number lopdf's writer prints.
const REAL_MAX: f64 = 1e15;

/// The deepest nesting of arrays and dictionaries that is kept. lopdf reads at most 100
/// levels, so anything kept here must stay well below that.
pub(crate) const NESTING_MAX: usize = 64;

/// A cursor over PDF bytes.
#[derive(Debug, Clone)]
pub(crate) struct Lexer<'a> {
    pub b: &'a [u8],
    pub i: usize,
    /// Bytes of white space and comments skipped so far.
    pub skipped: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(b: &'a [u8], i: usize) -> Lexer<'a> {
        Lexer { b, i, skipped: 0 }
    }

    fn peek(&self) -> Option<u8> {
        crate::work::add(1);
        self.b.get(self.i).copied()
    }

    /// Skips white space and comments.
    pub fn skip(&mut self) {
        let from = self.i;
        self.skip_inner();
        self.skipped += self.i - from;
    }

    fn skip_inner(&mut self) {
        while let Some(c) = self.peek() {
            if is_space(c) {
                self.i += 1;
            } else if c == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The run of regular characters at the cursor, which moves past it.
    pub fn regular(&mut self) -> &'a [u8] {
        let start = self.i;
        while self.peek().is_some_and(is_regular) {
            self.i += 1;
        }
        &self.b[start..self.i]
    }

    /// Skips white space and comments, then reads `word` as a whole token.
    pub fn keyword(&mut self, word: &[u8]) -> bool {
        let save = self.i;
        self.skip();
        if self.regular() == word {
            return true;
        }
        self.i = save;
        false
    }

    /// Skips white space and comments, then reads an unsigned integer token. Leading zeros
    /// are allowed.
    pub fn uint(&mut self) -> Option<u64> {
        let save = self.i;
        self.skip();
        let run = self.regular();
        if !run.is_empty() && run.len() <= 19 && run.iter().all(u8::is_ascii_digit) {
            // At most 19 digits, so the number fits.
            return Some(run.iter().fold(0u64, |v, d| v * 10 + u64::from(d - b'0')));
        }
        self.i = save;
        None
    }

    /// Reads an `N G obj` header.
    pub fn header(&mut self) -> Option<(u32, u16)> {
        let save = self.i;
        let n = self.uint().and_then(|n| u32::try_from(n).ok());
        let g = self.uint().and_then(|g| u16::try_from(g).ok());
        if let (Some(n), Some(g)) = (n, g)
            && self.keyword(b"obj")
        {
            return Some((n, g));
        }
        self.i = save;
        None
    }

    /// Reads one value.
    pub fn value(&mut self, budget: &mut Budget, depth: usize) -> Result<Object, Fail> {
        if depth > budget.max_depth.min(NESTING_MAX) {
            return Err(Fail::Bad("values are nested too deeply"));
        }
        self.skip();
        let Some(c) = self.peek() else {
            return Err(Fail::Bad("the file ends inside an object"));
        };
        match c {
            b'/' => {
                self.i += 1;
                let name = self.name();
                budget.value(name.len())?;
                Ok(Object::Name(name))
            }
            b'(' => {
                let (s, deep) = self.literal()?;
                budget.value(s.len())?;
                // lopdf's writer keeps balanced parentheses as they are, and lopdf reads at
                // most 100 levels of them, so a deep string is written as hex.
                let format = if deep {
                    StringFormat::Hexadecimal
                } else {
                    StringFormat::Literal
                };
                Ok(Object::String(s, format))
            }
            b'<' if self.b.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                budget.value(0)?;
                self.dictionary(budget, depth).map(Object::Dictionary)
            }
            b'<' => {
                let s = self.hex()?;
                budget.value(s.len())?;
                Ok(Object::String(s, StringFormat::Hexadecimal))
            }
            b'[' => {
                self.i += 1;
                budget.value(0)?;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    match self.peek() {
                        Some(b']') => {
                            self.i += 1;
                            // Parsed objects are kept until the file is closed: no spare room.
                            items.shrink_to_fit();
                            return Ok(Object::Array(items));
                        }
                        None => return Err(Fail::Bad("an array is not closed")),
                        _ => {
                            if items.len() >= budget.limits.max_items {
                                return Err(too_many(budget));
                            }
                            items.push(self.value(budget, depth + 1)?);
                        }
                    }
                }
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let o = self.number()?;
                budget.value(0)?;
                Ok(o)
            }
            _ => {
                let word = self.regular();
                let o = match word {
                    b"true" => Object::Boolean(true),
                    b"false" => Object::Boolean(false),
                    b"null" => Object::Null,
                    _ => return Err(Fail::Bad("an object holds an unknown keyword")),
                };
                budget.value(0)?;
                Ok(o)
            }
        }
    }

    /// The rest of a dictionary after `<<`.
    pub fn dictionary(&mut self, budget: &mut Budget, depth: usize) -> Result<Dictionary, Fail> {
        let mut dict = Dictionary::new();
        loop {
            self.skip();
            match self.peek() {
                Some(b'>') if self.b.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    // A map grown entry by entry keeps room for about twice its entries.
                    dict.as_hashmap_mut().shrink_to_fit();
                    return Ok(dict);
                }
                Some(b'/') => {
                    self.i += 1;
                    let key = self.name();
                    budget.value(key.len())?;
                    self.skip();
                    // A key without a value right before the end is dropped.
                    if self.b[self.i..].starts_with(b">>") {
                        continue;
                    }
                    let v = self.value(budget, depth + 1)?;
                    if dict.len() >= budget.limits.max_items {
                        return Err(too_many(budget));
                    }
                    dict.set(key, v);
                }
                None => return Err(Fail::Bad("a dictionary is not closed")),
                _ => return Err(Fail::Bad("a dictionary key is not a name")),
            }
        }
    }

    /// A name after its `/`, with `#xx` escapes decoded (7.3.5).
    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            if !is_regular(c) {
                break;
            }
            if c == b'#'
                && let (Some(h), Some(l)) = (
                    self.b.get(self.i + 1).and_then(|c| hex_digit(*c)),
                    self.b.get(self.i + 2).and_then(|c| hex_digit(*c)),
                )
            {
                out.push((h << 4) | l);
                self.i += 3;
                continue;
            }
            out.push(c);
            self.i += 1;
        }
        out
    }

    /// A literal string (7.3.4.2): nested parentheses, escapes, line ends as `\n`. The flag
    /// says the parentheses nest deeper than [`NESTING_MAX`].
    fn literal(&mut self) -> Result<(Vec<u8>, bool), Fail> {
        self.i += 1;
        let mut out = Vec::new();
        let mut depth = 1usize;
        let mut deep = false;
        loop {
            let Some(c) = self.peek() else {
                return Err(Fail::Bad("a string is not closed"));
            };
            self.i += 1;
            match c {
                b'(' => {
                    depth += 1;
                    deep |= depth > NESTING_MAX;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((out, deep));
                    }
                    out.push(c);
                }
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.i += 1;
                    }
                    out.push(b'\n');
                }
                b'\\' => self.escape(&mut out),
                _ => out.push(c),
            }
        }
    }

    fn escape(&mut self, out: &mut Vec<u8>) {
        let Some(c) = self.peek() else {
            return;
        };
        self.i += 1;
        match c {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'0'..=b'7' => {
                let mut v = u32::from(c - b'0');
                for _ in 0..2 {
                    match self.peek() {
                        Some(d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            self.i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            // A backslash at the end of a line continues the string on the next one.
            b'\r' => {
                if self.peek() == Some(b'\n') {
                    self.i += 1;
                }
            }
            b'\n' => {}
            // `\(`, `\)`, `\\`, and an unknown escape, which stands for the character.
            _ => out.push(c),
        }
    }

    /// A hex string (7.3.4.3). An odd last digit counts as followed by 0.
    fn hex(&mut self) -> Result<Vec<u8>, Fail> {
        self.i += 1;
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(c) = self.peek() else {
                return Err(Fail::Bad("a hex string is not closed"));
            };
            self.i += 1;
            if c == b'>' {
                if let Some(h) = high {
                    out.push(h << 4);
                }
                return Ok(out);
            }
            if is_space(c) {
                continue;
            }
            let Some(d) = hex_digit(c) else {
                return Err(Fail::Bad("a hex string holds a character that is not hex"));
            };
            match high.take() {
                Some(h) => out.push((h << 4) | d),
                None => high = Some(d),
            }
        }
    }

    /// A number, or a reference `N G R` when an unsigned integer is followed by one.
    fn number(&mut self) -> Result<Object, Fail> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| matches!(c, b'+' | b'-' | b'.' | b'0'..=b'9'))
        {
            self.i += 1;
        }
        let run = &self.b[start..self.i];
        // Some writers repeat the sign; the last one counts.
        let signs = run.iter().take_while(|c| matches!(c, b'+' | b'-')).count();
        let negative = signs > 0 && run[signs - 1] == b'-';
        let body = &run[signs..];
        if body.is_empty() || body.iter().filter(|c| **c == b'.').count() > 1 {
            return Err(Fail::Bad("a number is malformed"));
        }
        if body.iter().any(|c| matches!(c, b'+' | b'-')) {
            return Err(Fail::Bad("a number is malformed"));
        }
        if !body.contains(&b'.') {
            if signs == 0
                && let Some(r) = self.reference(body)
            {
                return Ok(r);
            }
            let digits = std::str::from_utf8(body).unwrap_or("0");
            if let Ok(v) = digits.parse::<i64>() {
                return Ok(Object::Integer(if negative { -v } else { v }));
            }
        }
        let text = std::str::from_utf8(body).unwrap_or("0");
        let text = if text == "." { "0" } else { text };
        let v: f64 = text.parse().unwrap_or(0.0);
        let v = v.min(REAL_MAX);
        Ok(Object::Real(if negative { -v } else { v } as f32))
    }

    /// Reads ` G R` after the digits of an object number, or moves nowhere.
    fn reference(&mut self, digits: &[u8]) -> Option<Object> {
        let n: u32 = std::str::from_utf8(digits).ok()?.parse().ok()?;
        let save = self.i;
        if let Some(g) = self.uint().and_then(|g| u16::try_from(g).ok())
            && self.keyword(b"R")
        {
            return Some(Object::Reference((n, g)));
        }
        self.i = save;
        None
    }

    /// After a stream dictionary: the start of the stream data, if the keyword `stream` follows.
    pub fn stream_start(&mut self) -> Option<usize> {
        let save = self.i;
        if !self.keyword(b"stream") {
            self.i = save;
            return None;
        }
        // The keyword is followed by CRLF or LF; a lone CR is accepted too.
        match (self.peek(), self.b.get(self.i + 1)) {
            (Some(b'\r'), Some(b'\n')) => self.i += 2,
            (Some(b'\r' | b'\n'), _) => self.i += 1,
            _ => {}
        }
        Some(self.i)
    }
}

fn too_many(budget: &Budget) -> Fail {
    Fail::Stop(crate::OpenError::Limit(format!(
        "an array or dictionary has more than {} items",
        budget.limits.max_items
    )))
}

fn hex_digit(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Finds `needle` in `hay` from `from`.
pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    let at = hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from);
    crate::work::add(
        at.map_or(hay.len(), |p| p + needle.len())
            .saturating_sub(from) as u64,
    );
    at
}

/// Finds the last `needle` in `hay`.
pub(crate) fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let at = hay.windows(needle.len()).rposition(|w| w == needle);
    crate::work::add(hay.len().saturating_sub(at.unwrap_or(0)) as u64);
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    fn parse(text: &[u8]) -> Result<Object, Fail> {
        let limits = Limits::default();
        let mut budget = Budget::new(&limits, &|| false, text.len());
        Lexer::new(text, 0).value(&mut budget, 0)
    }

    #[test]
    fn values_are_read() {
        assert_eq!(
            parse(b"/Obj#53tm").unwrap(),
            Object::Name(b"ObjStm".to_vec())
        );
        assert_eq!(parse(b"/A#2").unwrap(), Object::Name(b"A#2".to_vec()));
        assert_eq!(
            parse(b"(a(b)c\\)\\n\\101\\\r\nd)").unwrap(),
            Object::String(b"a(b)c)\nAd".to_vec(), StringFormat::Literal)
        );
        assert_eq!(
            parse(b"<41 42 4>").unwrap(),
            Object::String(b"AB@".to_vec(), StringFormat::Hexadecimal)
        );
        assert_eq!(parse(b"-12").unwrap(), Object::Integer(-12));
        assert_eq!(parse(b"--3").unwrap(), Object::Integer(-3));
        assert_eq!(parse(b".5").unwrap(), Object::Real(0.5));
        assert_eq!(parse(b"007 0 %c\nR").unwrap(), Object::Reference((7, 0)));
        assert_eq!(parse(b"7 0 obj").unwrap(), Object::Integer(7));
        assert!(matches!(parse(b"foo"), Err(Fail::Bad(_))));
        let d = parse(b"<< /A [1 2 0 R (x)] /B << /C null >> /D >>").unwrap();
        let d = d.as_dict().unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(
            d.get(b"A").unwrap().as_array().unwrap()[1],
            Object::Reference((2, 0))
        );
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = vec![b'['; 10_000];
        assert!(matches!(parse(&deep), Err(Fail::Bad(_))));
    }

    #[test]
    fn headers_allow_comments_and_zeros() {
        let mut l = Lexer::new(b" 0010 %x\n 00 %y\r obj <<>>", 0);
        assert_eq!(l.header(), Some((10, 0)));
        let mut l = Lexer::new(b"10 0 objx", 0);
        assert_eq!(l.header(), None);
        assert_eq!(l.i, 0);
    }
}
