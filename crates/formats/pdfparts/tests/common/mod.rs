// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Files written by hand for the tests: cross-reference streams, object streams and update
//! sections that lopdf's writer does not produce.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// Writes `n` copies of `byte` into `w` in chunks, so the plain data is never held.
pub fn fill(w: &mut impl Write, byte: u8, n: usize) {
    let chunk = vec![byte; 1 << 20];
    let mut left = n;
    while left > 0 {
        let k = left.min(chunk.len());
        w.write_all(&chunk[..k]).unwrap();
        left -= k;
    }
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

pub fn zlib_fill(byte: u8, n: usize) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    fill(&mut e, byte, n);
    e.finish().unwrap()
}

/// A zlib stream of about 2 MB that inflates to 2 GiB of zeros.
pub fn bomb_2g() -> Vec<u8> {
    zlib_fill(0, 2 << 30)
}

/// Maximum resident set size of the process. It is the figure of one test only under nextest,
/// which runs every test in its own process.
#[cfg(unix)]
pub fn max_rss_bytes() -> u64 {
    // SAFETY: getrusage fills the struct we pass.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        v
    } else {
        v * 1024
    }
}

/// The bound every memory test checks.
pub const RSS_BOUND: u64 = 512 * 1024 * 1024;

#[cfg(unix)]
pub fn assert_rss_bounded() {
    let rss = max_rss_bytes();
    eprintln!("max RSS {} MB", rss / 1_000_000);
    assert!(rss < RSS_BOUND, "max RSS {rss}");
}

#[cfg(not(unix))]
pub fn assert_rss_bounded() {}

/// One cross-reference entry: type, second and third field.
type Entry = (u8, u64, u64);

/// A PDF file written by hand with a cross-reference stream.
pub struct Raw {
    pub out: Vec<u8>,
    entries: BTreeMap<u32, Entry>,
}

impl Default for Raw {
    fn default() -> Self {
        Raw::new()
    }
}

impl Raw {
    pub fn new() -> Raw {
        Raw {
            out: b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            entries: BTreeMap::new(),
        }
    }

    /// Writes object `n` with its own header text (for example `n 0 %c\nobj`).
    pub fn object_with_header(&mut self, header: &str, n: u32, body: &[u8]) {
        self.entries.insert(n, (1, self.out.len() as u64, 0));
        self.out.extend(header.as_bytes());
        self.out.push(b'\n');
        self.out.extend(body);
        self.out.extend(b"\nendobj\n");
    }

    /// Writes object `n` without `endobj`.
    pub fn unclosed(&mut self, n: u32, body: &str) {
        self.entries.insert(n, (1, self.out.len() as u64, 0));
        self.out.extend(format!("{n} 0 obj\n{body}\n").as_bytes());
    }

    pub fn object(&mut self, n: u32, body: &str) {
        self.object_with_header(&format!("{n} 0 obj"), n, body.as_bytes());
    }

    /// Lists `member` as the `index`th member of object stream `container`.
    pub fn member(&mut self, member: u32, container: u32, index: u32) {
        self.entries
            .insert(member, (2, u64::from(container), u64::from(index)));
    }

    /// Writes bytes that are not an object the table points at.
    pub fn junk(&mut self, bytes: &[u8]) {
        self.out.extend(bytes);
    }

    pub fn stream(&mut self, n: u32, dict: &str, data: &[u8]) {
        self.object_with_header(&format!("{n} 0 obj"), n, &stream_body(dict, data));
    }

    /// An object stream `n` holding `members`, deflated.
    pub fn object_stream(&mut self, n: u32, members: &[(u32, &[u8])]) {
        self.object_stream_with_header(&format!("{n} 0 obj"), n, members);
    }

    pub fn object_stream_with_header(&mut self, header: &str, n: u32, members: &[(u32, &[u8])]) {
        let (first, plain) = object_stream_data(members);
        let packed = zlib(&plain);
        self.packed_object_stream(header, n, first, members.len(), &packed);
        for (i, (m, _)) in members.iter().enumerate() {
            self.entries.insert(*m, (2, u64::from(n), i as u64));
        }
    }

    /// An object stream whose data is given already deflated, with one member listed in the
    /// cross-reference stream.
    pub fn bomb_object_stream(&mut self, header: &str, n: u32, member: u32, packed: &[u8]) {
        self.packed_object_stream(header, n, 4, 1, packed);
        self.entries.insert(member, (2, u64::from(n), 0));
    }

    fn packed_object_stream(
        &mut self,
        header: &str,
        n: u32,
        first: usize,
        count: usize,
        packed: &[u8],
    ) {
        let dict = format!("/Type /ObjStm /N {count} /First {first} /Filter /FlateDecode");
        self.object_with_header(header, n, &stream_body(&dict, packed));
    }

    /// Ends the file with a cross-reference stream. `extra` goes into its dictionary.
    pub fn finish(mut self, root: u32, extra: &str) -> Vec<u8> {
        let x = self.entries.keys().max().copied().unwrap_or(0) + 1;
        let pos = self.out.len() as u64;
        self.entries.insert(x, (1, pos, 0));
        let size = x + 1;
        let mut rows = Vec::new();
        for n in 0..size {
            let (t, a, b) = self.entries.get(&n).copied().unwrap_or((0, 0, 65535));
            rows.extend(row(t, a, b));
        }
        let dict = format!("/Type /XRef /Size {size} /W [1 8 2] /Root {root} 0 R {extra}");
        self.out.extend(format!("{x} 0 obj\n").as_bytes());
        self.out.extend(stream_body(&dict, &rows));
        self.out
            .extend(format!("\nendobj\nstartxref\n{pos}\n%%EOF\n").as_bytes());
        self.out
    }
}

fn row(t: u8, a: u64, b: u64) -> Vec<u8> {
    let mut r = vec![t];
    r.extend(a.to_be_bytes());
    r.extend((b as u16).to_be_bytes());
    r
}

pub fn stream_body(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend(data);
    v.extend(b"\nendstream");
    v
}

/// The plain data of an object stream and the offset of its first member.
pub fn object_stream_data(members: &[(u32, &[u8])]) -> (usize, Vec<u8>) {
    let mut head = String::new();
    let mut body = Vec::new();
    for (m, data) in members {
        head.push_str(&format!("{m} {} ", body.len()));
        body.extend(*data);
        body.push(b' ');
    }
    head.push('\n');
    let first = head.len();
    let mut plain = head.into_bytes();
    plain.extend(body);
    (first, plain)
}

/// The `startxref` value of a file.
pub fn startxref_of(bytes: &[u8]) -> u64 {
    let at = bytes
        .windows(9)
        .rposition(|w| w == b"startxref")
        .expect("startxref");
    let rest = &bytes[at + 9..];
    let digits: String = rest
        .iter()
        .skip_while(|c| c.is_ascii_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .map(|&c| c as char)
        .collect();
    digits.parse().unwrap()
}

/// Appends an update section to a file: new plain objects, members of object streams that are
/// already in the file (member, container, index), and a cross-reference stream with `/Prev`
/// pointing at the old section. `root` is the catalog the new section names; `extra` goes into
/// the new trailer (for example `/Encrypt` and `/ID` of an encrypted file).
pub fn append_update(
    bytes: &[u8],
    objects: &[(u32, String)],
    members: &[(u32, u32, u32)],
    root: u32,
    extra: &str,
) -> Vec<u8> {
    let prev = startxref_of(bytes);
    let mut out = bytes.to_vec();
    out.push(b'\n');
    let mut entries: BTreeMap<u32, Entry> = BTreeMap::new();
    for (n, body) in objects {
        entries.insert(*n, (1, out.len() as u64, 0));
        out.extend(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    for (m, c, i) in members {
        entries.insert(*m, (2, u64::from(*c), u64::from(*i)));
    }
    let x = entries.keys().max().copied().unwrap_or(0).max(1000) + 1;
    let pos = out.len() as u64;
    entries.insert(x, (1, pos, 0));
    let mut index = String::new();
    let mut rows = Vec::new();
    for (n, (t, a, b)) in &entries {
        index.push_str(&format!("{n} 1 "));
        rows.extend(row(*t, *a, *b));
    }
    let dict = format!(
        "/Type /XRef /Size {} /W [1 8 2] /Index [{index}] /Prev {prev} /Root {root} 0 R {extra}",
        x + 1
    );
    out.extend(format!("{x} 0 obj\n").as_bytes());
    out.extend(stream_body(&dict, &rows));
    out.extend(format!("\nendobj\nstartxref\n{pos}\n%%EOF\n").as_bytes());
    out
}
