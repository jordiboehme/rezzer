// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A light scan of a page's content streams that counts the text shown in render mode 3,
//! which puts no glyphs on the page. Only operators are read: no text is decoded and nothing
//! is drawn. Form XObjects are followed with the render mode they are called with.

use std::collections::HashMap;

use lopdf::{Dictionary, Object, ObjectId};

use crate::objects::{dict, get, get_dict, get_name, reference};
use crate::{Issue, IssueKind, Pdf, pagetree};

/// How many tokens are read between two calls of the stop callback.
const TOKENS_PER_POLL: usize = 4096;

/// Text shown in render mode 3 on one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InvisibleShows {
    /// Text-showing operators (`Tj`, `TJ`, `'` and `"`) run in render mode 3.
    pub count: u64,
    /// The whole page was scanned. `false` when a limit or the stop callback cut the scan;
    /// `count` is then a lower bound.
    pub complete: bool,
}

struct Scan<'a> {
    pdf: &'a Pdf,
    stop: &'a dyn Fn() -> bool,
    /// Tokens read so far, bounded by `max_items`.
    work: usize,
    /// Counts of forms already scanned, by form and the mode they started in.
    forms: HashMap<(ObjectId, u8), u64>,
    /// Forms being scanned, to end a form that calls itself.
    active: Vec<ObjectId>,
    /// A limit hit; nothing more is read.
    cut: bool,
    stopped: bool,
}

impl Pdf {
    /// Counts the text-showing operators that run in render mode 3 on `page` (1-based),
    /// following Form XObjects. The scan counts its tokens against `max_items` and its
    /// nesting against `max_depth`; a hit is a Limit issue and gives an incomplete count.
    pub fn invisible_text_shows(&self, page: u32, stop: &dyn Fn() -> bool) -> InvisibleShows {
        let Some(node) = usize::try_from(page)
            .ok()
            .and_then(|p| p.checked_sub(1))
            .and_then(|i| self.pages.get(i))
        else {
            return InvisibleShows::default();
        };
        if self.verdict.refused_pages.contains(&page) {
            return InvisibleShows::default();
        }
        if stop() {
            return InvisibleShows {
                count: 0,
                complete: false,
            };
        }
        let doc = &self.doc;
        let resources = pagetree::resources(doc, node).and_then(|r| dict(doc, r));
        let mut scan = Scan {
            pdf: self,
            stop,
            work: 0,
            forms: HashMap::new(),
            active: Vec::new(),
            cut: false,
            stopped: false,
        };
        let mut streams = Vec::new();
        if let Some(contents) = doc
            .objects
            .get(&node.id)
            .and_then(|o| dict(doc, o))
            .and_then(|d| d.get(b"Contents").ok())
        {
            match contents {
                Object::Array(items) => streams.extend(items.iter().filter_map(reference)),
                other => streams.extend(reference(other)),
            }
        }
        // The content streams of a page are one stream cut in pieces: the mode carries over.
        let mut data = Vec::new();
        for id in streams {
            if let Some(bytes) = self.plain(id) {
                data.extend_from_slice(&bytes);
                data.push(b'\n');
            }
        }
        let count = scan.stream(&data, resources, 0, 0);
        if scan.cut {
            self.note(vec![Issue::new(
                &format!("page {page}"),
                IssueKind::Limit,
                format!(
                    "the content scan for invisible text stopped after {} tokens",
                    self.limits.max_items
                ),
            )]);
        }
        InvisibleShows {
            count,
            complete: !scan.cut && !scan.stopped,
        }
    }
}

impl Scan<'_> {
    /// Counts the mode 3 shows of one content stream that starts in render mode `mode`.
    fn stream(
        &mut self,
        data: &[u8],
        resources: Option<&Dictionary>,
        mode: u8,
        depth: usize,
    ) -> u64 {
        let mut count: u64 = 0;
        let mut mode = mode;
        let mut saved: Vec<u8> = Vec::new();
        // The operand just before the operator: a number or a name.
        let mut number: Option<f64> = None;
        let mut name: Option<&[u8]> = None;
        let mut i = 0;
        while i < data.len() {
            if self.cut || self.stopped {
                break;
            }
            let c = data[i];
            if is_space(c) {
                i += 1;
                continue;
            }
            self.work += 1;
            if self.work > self.pdf.limits.max_items {
                self.cut = true;
                break;
            }
            if self.work.is_multiple_of(TOKENS_PER_POLL) && (self.stop)() {
                self.stopped = true;
                break;
            }
            match c {
                b'%' => {
                    while i < data.len() && !matches!(data[i], b'\r' | b'\n') {
                        i += 1;
                    }
                }
                b'(' => {
                    i = skip_string(data, i);
                    number = None;
                    name = None;
                }
                b'<' if data.get(i + 1) == Some(&b'<') => i += 2,
                b'<' => {
                    while i < data.len() && data[i] != b'>' {
                        i += 1;
                    }
                    i += 1;
                    number = None;
                    name = None;
                }
                b'>' | b'[' | b']' | b'{' | b'}' | b')' => i += 1,
                b'/' => {
                    let start = i + 1;
                    i = start;
                    while i < data.len() && is_regular(data[i]) {
                        i += 1;
                    }
                    name = Some(&data[start..i]);
                    number = None;
                }
                _ => {
                    let start = i;
                    while i < data.len() && is_regular(data[i]) {
                        i += 1;
                    }
                    let word = &data[start..i];
                    if let Some(n) = std::str::from_utf8(word)
                        .ok()
                        .and_then(|w| w.parse::<f64>().ok())
                    {
                        number = Some(n);
                        name = None;
                        continue;
                    }
                    match word {
                        b"q" => saved.push(mode),
                        b"Q" => mode = saved.pop().unwrap_or(mode),
                        b"Tr" => {
                            if let Some(n) = number
                                && n.fract() == 0.0
                                && (0.0..=7.0).contains(&n)
                            {
                                mode = n as u8;
                            }
                        }
                        b"Tj" | b"TJ" | b"'" | b"\"" if mode == 3 => count += 1,
                        b"Do" => {
                            if let Some(n) = name {
                                count = count.saturating_add(self.form(n, resources, mode, depth));
                            }
                        }
                        b"BI" => i = skip_inline_image(data, i),
                        _ => {}
                    }
                    number = None;
                    name = None;
                    // A word of delimiters only (for example a lone `)`) moves on.
                    if i == start {
                        i += 1;
                    }
                }
            }
        }
        count
    }

    /// The mode 3 shows of the Form XObject `name` in `resources`, called in `mode`.
    fn form(&mut self, name: &[u8], resources: Option<&Dictionary>, mode: u8, depth: usize) -> u64 {
        let doc = &self.pdf.doc;
        let Some(id) = resources
            .and_then(|r| get_dict(doc, r, b"XObject"))
            .and_then(|x| x.get(name).ok())
            .and_then(reference)
        else {
            return 0;
        };
        if let Some(n) = self.forms.get(&(id, mode)) {
            return *n;
        }
        if self.active.contains(&id) {
            return 0;
        }
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
            return 0;
        };
        if get_name(doc, &stream.dict, b"Subtype") != Some(b"Form".as_slice()) {
            return 0;
        }
        if depth >= self.pdf.limits.max_depth {
            self.cut = true;
            return 0;
        }
        let Some(data) = self.pdf.plain(id) else {
            return 0;
        };
        // A form without its own resources uses the ones it was called with.
        let own = get(doc, &stream.dict, b"Resources").and_then(|r| dict(doc, r));
        self.active.push(id);
        let n = self.stream(&data, own.or(resources), mode, depth + 1);
        self.active.pop();
        if !self.cut && !self.stopped {
            self.forms.insert((id, mode), n);
        }
        n
    }
}

fn is_space(c: u8) -> bool {
    matches!(c, 0 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

fn is_regular(c: u8) -> bool {
    !is_space(c)
        && !matches!(
            c,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

/// The position after the literal string that starts at `i`.
fn skip_string(data: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < data.len() {
        match data[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    data.len()
}

/// The position after the inline image whose `BI` ends at `i`: past its `EI`.
fn skip_inline_image(data: &[u8], i: usize) -> usize {
    let Some(id) = find_word(data, b"ID", i) else {
        return data.len();
    };
    // One white space byte follows `ID`; the data runs to the first `EI` that stands alone.
    let mut j = id + 3;
    while j + 2 <= data.len() {
        if &data[j..j + 2] == b"EI"
            && is_space(data[j - 1])
            && data.get(j + 2).is_none_or(|c| !is_regular(*c))
        {
            return j + 2;
        }
        j += 1;
    }
    data.len()
}

/// The start of `word` standing alone at or after `from`.
fn find_word(data: &[u8], word: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while j + word.len() <= data.len() {
        if &data[j..j + word.len()] == word
            && (j == 0 || !is_regular(data[j - 1]))
            && data.get(j + word.len()).is_none_or(|c| !is_regular(*c))
        {
            return Some(j);
        }
        j += 1;
    }
    None
}
