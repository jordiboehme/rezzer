// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The inline pictures of a content stream, found the way the renderer finds them, while the
//! stream is read piece by piece.
//!
//! A small lexer follows the content as hayro 0.8.0 reads it: strings with nesting and escapes,
//! hex strings, comments, names and runs of regular characters, so a `BI` in a string or a
//! comment is no picture. Each `BI` operator is handed to hayro's own content parser, which
//! says where the picture's data ends, and the lexer goes on from there, as the renderer does.
//!
//! hayro takes the first `EI` after the data that the content after it confirms (no byte
//! above 127 before the next `EI`, a text operator or 20 operators after it), looking as far
//! as the next inline picture or the end of the stream. The scan gives the parser at least
//! [`WINDOW`] bytes after each `BI`, and checks that what the parser decided does not depend
//! on what lies past them: an end it passed over must have been passed over for a reason
//! inside the window, and the content after the end it took is followed until the next inline
//! picture, the way hayro follows it. When that cannot be told, the page is not rendered.

use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::content::{Instruction, TypedIter, UntypedIter};
use hayro_syntax::object::{Object, Stream};

/// Bytes after a `BI` the parser is given before it decides; an inline picture must fit in
/// them.
pub(super) const WINDOW: usize = 1 << 20;

/// Bytes after a `BI` an inline picture's dictionary must fit in, to tell whether it is one.
const DICT: usize = 16 << 10;

/// Bytes the check of one inline picture's earlier ends may look at.
const WORK: usize = 16 * WINDOW;

/// A regular token longer than this is no `BI`, even with `#` escapes.
const TOKEN: usize = 16;

/// The lexer's state between two bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lex {
    Normal,
    Comment,
    Literal {
        depth: u32,
        escape: bool,
    },
    /// After a `<`: a dictionary or a hex string.
    Less,
    Hex,
}

/// The token being read.
#[derive(Clone, Copy)]
enum Token {
    /// A run of regular characters from this index.
    Run(usize),
    /// A name, or a run too long to be `BI`.
    Other,
}

/// The end hayro took for the last inline picture, while what follows could still make it
/// take another: hayro follows the content until the next inline picture.
struct Pending {
    /// Where to look next.
    at: usize,
    /// A byte above 127 came since the end.
    high: bool,
    /// A text operator or 20 operators follow the end.
    confirmed: bool,
}

/// Scans one content stream for its inline pictures and hands each to `check`.
pub(super) struct InlineScan<'c> {
    buf: Vec<u8>,
    /// What is before `pos` has been lexed.
    pos: usize,
    lex: Lex,
    token: Option<Token>,
    pending: Option<Pending>,
    check: &'c dyn Fn(&Stream<'_>) -> Result<(), String>,
}

/// What came of a `BI`.
enum Bi {
    /// Checked; the content goes on at this index.
    Done(usize),
    /// More of the stream is needed.
    Wait,
}

fn white(c: u8) -> bool {
    matches!(c, 0 | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn delimiter(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn regular(c: u8) -> bool {
    !white(c) && !delimiter(c)
}

/// Whether a run of regular characters reads as the operator `BI`, `#` escapes decoded.
fn is_bi(run: &[u8]) -> bool {
    let mut out = [0u8; 2];
    let mut n = 0;
    let mut i = 0;
    while i < run.len() {
        let b = if run[i] == b'#' {
            let hex = |c: u8| (c as char).to_digit(16);
            match (
                run.get(i + 1).and_then(|c| hex(*c)),
                run.get(i + 2).and_then(|c| hex(*c)),
            ) {
                (Some(h), Some(l)) => {
                    i += 3;
                    (h * 16 + l) as u8
                }
                _ => return false,
            }
        } else {
            i += 1;
            run[i - 1]
        };
        if n == 2 {
            return false;
        }
        out[n] = b;
        n += 1;
    }
    n == 2 && out == *b"BI"
}

/// The first index of `needle` in `hay` at or after `from`.
fn find(hay: &[u8], from: usize, needle: &[u8; 2]) -> Option<usize> {
    hay.get(from..)?
        .windows(2)
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Where `part` starts in `whole`, when it is a part of it.
fn offset(whole: &[u8], part: &[u8]) -> Option<usize> {
    let (w, p) = (whole.as_ptr() as usize, part.as_ptr() as usize);
    (p >= w && p + part.len() <= w + whole.len()).then(|| p - w)
}

/// Where instruction `ins`, read from `data`, ends in it: after the `EI` of an inline picture,
/// else after its operator. `None` when that cannot be told.
fn end_of(data: &[u8], ins: &Instruction<'_, '_>) -> Option<usize> {
    for o in ins.operands() {
        if let Object::Stream(s) = o {
            let raw = s.raw_data();
            return offset(data, &raw).map(|at| at + raw.len() + 2);
        }
    }
    let op: &[u8] = ins.operator;
    offset(data, op).map(|at| at + op.len())
}

/// What hayro's check of the content after a candidate end at `data[0]` gives: a text
/// operator or 20 operators confirm it. `None` when that depends on bytes past `data`.
fn confirmed(data: &[u8]) -> Option<bool> {
    let mut typed = TypedIter::new(data);
    let mut plain = UntypedIter::new(data);
    let mut counter = 0;
    loop {
        // 0: no instruction, 1: a text operator, 2: another one.
        let kind = match typed.next() {
            None => 0,
            Some(
                TypedInstruction::NextLineAndShowText(_)
                | TypedInstruction::ShowText(_)
                | TypedInstruction::ShowTexts(_)
                | TypedInstruction::ShowTextWithParameters(_),
            ) => 1,
            Some(_) => 2,
        };
        // The same instruction, read without types, says where it ends. Where an inline
        // picture ends may itself depend on what lies past `data`.
        let end = match plain.next() {
            Some(ins) if &**ins.operator == b"BI" => return None,
            Some(ins) => end_of(data, &ins),
            None => None,
        };
        // Only a whole instruction is sure: one cut at the end of `data` may read otherwise.
        let whole = end.is_some_and(|e| e < data.len());
        match kind {
            // The typed reader stops where the plain one stops (unsure), or at a path operator
            // it cannot read (sure when whole).
            0 => return whole.then_some(false),
            _ if counter >= 20 || kind == 1 => return whole.then_some(true),
            _ => counter += 1,
        }
    }
}

/// Whether `data` starts with an inline picture's `BI` and dictionary, as hayro tells the
/// next inline picture when it looks for the end of one.
fn starts_picture(data: &[u8]) -> bool {
    let data = &data[..data.len().min(DICT)];
    UntypedIter::new(data)
        .next()
        .is_some_and(|ins| &**ins.operator == b"BI")
}

/// Whether hayro surely tells no inline picture's dictionary after the `BI` that `data` starts
/// with: after white space and comments comes neither a name nor `ID`.
fn no_picture(data: &[u8]) -> bool {
    let mut i = 2;
    while let Some(&c) = data.get(i) {
        if white(c) {
            i += 1;
        } else if c == b'%' {
            while data.get(i).is_some_and(|c| !matches!(c, b'\r' | b'\n')) {
                i += 1;
            }
        } else {
            return c != b'/' && !data[i..].starts_with(b"ID");
        }
    }
    false
}

impl<'c> InlineScan<'c> {
    pub(super) fn new(check: &'c dyn Fn(&Stream<'_>) -> Result<(), String>) -> Self {
        InlineScan {
            buf: Vec::new(),
            pos: 0,
            lex: Lex::Normal,
            token: None,
            pending: None,
            check,
        }
    }

    /// Scans the next piece of the stream.
    pub(super) fn feed(&mut self, piece: &[u8]) -> Result<(), String> {
        self.buf.extend_from_slice(piece);
        self.run(false)
    }

    /// Scans what is left at the end of the stream.
    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.run(true)
    }

    fn run(&mut self, last: bool) -> Result<(), String> {
        while self.pos < self.buf.len() {
            let c = self.buf[self.pos];
            match self.lex {
                Lex::Comment => {
                    if matches!(c, b'\r' | b'\n') {
                        self.lex = Lex::Normal;
                    }
                }
                Lex::Literal { depth, escape } => {
                    self.lex = match c {
                        _ if escape => Lex::Literal {
                            depth,
                            escape: false,
                        },
                        b'\\' => Lex::Literal {
                            depth,
                            escape: true,
                        },
                        b'(' => Lex::Literal {
                            depth: depth + 1,
                            escape: false,
                        },
                        b')' if depth == 1 => Lex::Normal,
                        b')' => Lex::Literal {
                            depth: depth - 1,
                            escape: false,
                        },
                        _ => Lex::Literal { depth, escape },
                    };
                }
                Lex::Less => {
                    if c == b'<' {
                        self.lex = Lex::Normal;
                    } else {
                        // This byte is the hex string's first.
                        self.lex = Lex::Hex;
                        continue;
                    }
                }
                Lex::Hex => {
                    if c == b'>' {
                        self.lex = Lex::Normal;
                    }
                }
                Lex::Normal => {
                    if regular(c) {
                        match self.token {
                            None => self.token = Some(Token::Run(self.pos)),
                            Some(Token::Run(t)) if self.pos - t >= TOKEN => {
                                self.token = Some(Token::Other);
                            }
                            _ => {}
                        }
                        self.pos += 1;
                        continue;
                    }
                    if let Some(next) = self.token_end(last)? {
                        match next {
                            Bi::Done(at) => {
                                self.pos = at;
                                continue;
                            }
                            Bi::Wait => break,
                        }
                    }
                    match c {
                        b'%' => self.lex = Lex::Comment,
                        b'(' => {
                            self.lex = Lex::Literal {
                                depth: 1,
                                escape: false,
                            }
                        }
                        b'<' => self.lex = Lex::Less,
                        b'/' => self.token = Some(Token::Other),
                        _ => {}
                    }
                }
            }
            self.pos += 1;
        }
        if last && self.pos >= self.buf.len() {
            if let Some(Bi::Done(at)) = self.token_end(true)? {
                self.pos = at;
                return self.run(true);
            }
            // The end of the stream settles the last inline picture's end.
            self.follow(self.buf.len(), true)?;
            self.pending = None;
            return Ok(());
        }
        self.follow(self.buf.len(), last)?;
        self.drain();
        Ok(())
    }

    /// Ends the token before `pos`, and handles it when it is `BI`.
    fn token_end(&mut self, last: bool) -> Result<Option<Bi>, String> {
        let Some(Token::Run(t)) = self.token.take() else {
            return Ok(None);
        };
        if !is_bi(&self.buf[t..self.pos]) {
            return Ok(None);
        }
        let done = self.picture(t, last)?;
        if matches!(done, Bi::Wait) {
            // Read the token again once more has come.
            self.pos = t;
        }
        Ok(Some(done))
    }

    /// Checks the inline picture whose `BI` starts at `bi`.
    fn picture(&mut self, bi: usize, last: bool) -> Result<Bi, String> {
        if !last && self.buf.len() - bi < WINDOW {
            return Ok(Bi::Wait);
        }
        // What came before this `BI` is followed first: the `BI` may end it.
        self.follow(bi + 2, last)?;
        if self.pending.as_ref().is_some_and(|p| p.at <= bi) {
            return Ok(Bi::Wait);
        }
        let unreadable = || {
            "it has an inline picture that cannot be read, so its size cannot be checked \
             before a render"
                .to_string()
        };
        let data = &self.buf[bi..];
        let mut iter = UntypedIter::new(data);
        let Some(ins) = iter.next() else {
            return Err(unreadable());
        };
        if &**ins.operator != b"BI" {
            return Err(unreadable());
        }
        let Some(stream) = ins.operands().find_map(|o| match o {
            Object::Stream(s) => Some(s.clone()),
            _ => None,
        }) else {
            return Err(if last {
                unreadable()
            } else {
                format!(
                    "it has an inline picture of more than {WINDOW} bytes, too large to check \
                     before a render"
                )
            });
        };
        let raw = stream.raw_data();
        let Some(start) = offset(data, &raw) else {
            return Err(unreadable());
        };
        let (start, end) = (bi + start, bi + start + raw.len());
        drop(raw);
        (self.check)(&stream)?;
        drop(stream);
        if !last {
            self.settle(start, end)?;
        }
        Ok(Bi::Done(end + 2))
    }

    /// Checks that the end hayro took for data from `start` to `end`, with the window it had,
    /// is the end it takes with the whole stream, as far as the window tells; what follows the
    /// end is then followed with [`Self::follow`].
    fn settle(&mut self, start: usize, end: usize) -> Result<(), String> {
        let unclear =
            || "it has an inline picture whose end cannot be told before a render".to_string();
        let buf = &self.buf;
        let mut p = start;
        let mut work = 0usize;
        // Each earlier `EI` hayro passed over.
        while let Some(e) = find(buf, p, b"EI") {
            if e >= end {
                break;
            }
            if buf.get(e + 2).is_some_and(|c| !white(*c)) {
                p = e + 3;
                continue;
            }
            // Passed over for a reason inside the window: a byte above 127 before a later
            // `EI`, or content after it that is not confirmed, ahead of any next picture.
            let mut checked = None;
            let mut high = false;
            let mut ok = false;
            let mut i = e + 2;
            while i + 1 < buf.len() {
                work += 1;
                if work > WORK {
                    return Err(unclear());
                }
                let c = buf[i];
                if c == b'E' && buf[i + 1] == b'I' {
                    if high
                        || *checked.get_or_insert_with(|| confirmed(&buf[e + 2..])) == Some(false)
                    {
                        ok = true;
                        break;
                    }
                } else if c == b'B' && buf[i + 1] == b'I' && !no_picture(&buf[i..]) {
                    break;
                }
                high |= !c.is_ascii();
                i += 1;
            }
            if !ok {
                return Err(unclear());
            }
            p = e + 2;
        }
        let sure = confirmed(&buf[end + 2..]) == Some(true);
        self.pending = Some(Pending {
            at: end + 2,
            high: false,
            confirmed: sure,
        });
        Ok(())
    }

    /// Follows the content after the end hayro took for the last inline picture up to
    /// `limit`, as hayro does: a later `EI` with a byte above 127 before it, or without the
    /// content after the end confirmed, makes hayro take another end; a next picture or the
    /// end of the stream settles it.
    fn follow(&mut self, limit: usize, last: bool) -> Result<(), String> {
        let Some(p) = self.pending.as_mut() else {
            return Ok(());
        };
        let buf = &self.buf;
        let limit = limit.min(buf.len());
        while p.at + 1 < limit {
            let c = buf[p.at];
            if c == b'E' && buf[p.at + 1] == b'I' {
                if p.high || !p.confirmed {
                    return Err(
                        "it has an inline picture whose end cannot be told before a render"
                            .to_string(),
                    );
                }
            } else if c == b'B' && buf[p.at + 1] == b'I' && !no_picture(&buf[p.at..]) {
                if !last && buf.len() - p.at < DICT {
                    return Ok(());
                }
                if starts_picture(&buf[p.at..]) {
                    self.pending = None;
                    return Ok(());
                }
            }
            p.high |= !c.is_ascii();
            p.at += 1;
        }
        Ok(())
    }

    /// Drops what no later step needs.
    fn drain(&mut self) {
        let mut keep = self.pos;
        if let Some(Token::Run(t)) = self.token {
            keep = keep.min(t);
        }
        if let Some(p) = &self.pending {
            keep = keep.min(p.at);
        }
        let keep = keep.min(self.buf.len());
        if keep == 0 {
            return;
        }
        self.buf.drain(..keep);
        self.pos -= keep;
        if let Some(Token::Run(t)) = &mut self.token {
            *t -= keep;
        }
        if let Some(p) = &mut self.pending {
            p.at -= keep;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// The data lengths of the inline pictures `content` holds, fed in pieces of `piece`
    /// bytes.
    fn pictures(content: &[u8], piece: usize) -> Result<Vec<usize>, String> {
        let found = RefCell::new(Vec::new());
        let check = |s: &Stream<'_>| -> Result<(), String> {
            found.borrow_mut().push(s.raw_data().len());
            Ok(())
        };
        let mut scan = InlineScan::new(&check);
        for p in content.chunks(piece) {
            scan.feed(p)?;
        }
        scan.finish()?;
        Ok(found.into_inner())
    }

    /// The same, fed whole, in small pieces and byte by byte.
    fn all_ways(content: &[u8]) -> Result<Vec<usize>, String> {
        let whole = pictures(content, content.len().max(1));
        assert_eq!(whole, pictures(content, 7));
        assert_eq!(whole, pictures(content, 1));
        whole
    }

    #[test]
    fn bi_in_strings_comments_and_hex_strings_is_no_picture() {
        let c = b"(BI) Tj (a\\) BI (BI) ) Tj % BI /W 1 ID x EI\n<4249> Tj [(BI)] TJ /BI cs \
                  xBI BIx\nBI /W 1 /H 1 /BPC 8 /CS /G ID \x07 EI Q";
        assert_eq!(all_ways(c), Ok(vec![2]));
    }

    #[test]
    fn an_escaped_bi_is_a_picture() {
        let c = b"q B#49 /W 2 /H 2 /BPC 8 /CS /G ID \x01\x02\x03\x04 EI Q";
        assert_eq!(all_ways(c), Ok(vec![5]));
    }

    #[test]
    fn data_holding_ei_ends_where_the_renderer_ends_it() {
        let c = b"BI /W 4 /H 2 /BPC 8 /CS /G ID \x80EI \xff\x01\x02\x03 EI Q 0 0 m (x) Tj";
        assert_eq!(all_ways(c), Ok(vec![9]));
    }

    #[test]
    fn a_picture_that_cannot_be_read_fails() {
        for c in [
            &b"BI /W 8 /H ID \x00 EI Q"[..],
            b"q BI /W 1 /H 1 /BPC 8 /CS /G ID \x00 no end",
            b"q BI",
        ] {
            let got = pictures(c, 3);
            assert!(
                got.as_ref().is_err_and(|e| e.contains("cannot be read")),
                "{got:?}"
            );
        }
    }

    /// A picture, more than a window of drawing, then `tail`.
    fn far(tail: &[u8]) -> Vec<u8> {
        let mut c = b"q BI /W 1 /H 1 /BPC 8 /CS /G ID \x07 EI Q\n".to_vec();
        while c.len() < WINDOW + 200_000 {
            c.extend(b"0 0 m 10 10 l S\n");
        }
        c.extend(tail);
        c
    }

    #[test]
    fn pictures_a_window_apart_are_each_found() {
        let c = far(b"BI /W 2 /H 1 /BPC 8 /CS /G ID \x07\x08 EI (EI) Tj\n");
        assert_eq!(pictures(&c, 65_536), Ok(vec![2, 3]));
        assert_eq!(pictures(&c, 1_000), Ok(vec![2, 3]));
    }

    #[test]
    fn data_holding_ei_ends_where_the_renderer_ends_it_a_window_before_the_end() {
        let mut c = b"BI /W 4 /H 2 /BPC 8 /CS /G ID \x80EI \xff\x01\x02\x03 EI Q\n".to_vec();
        c.extend(far(b"(cafe) Tj\n"));
        assert_eq!(pictures(&c, 65_536), Ok(vec![9, 2]));
    }

    #[test]
    fn a_later_ei_the_renderer_would_take_fails() {
        // hayro takes a later `EI` as the picture's end when a byte above 127 comes before it.
        let got = pictures(&far(b"(caf\xe9) Tj (EI) Tj\n"), 65_536);
        assert!(
            got.as_ref().is_err_and(|e| e.contains("cannot be told")),
            "{got:?}"
        );
        // Without one, the end stays.
        assert_eq!(pictures(&far(b"(cafe) Tj (EI) Tj\n"), 65_536), Ok(vec![2]));
    }
}
