// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Decoding a stream's filter chain into counters that keep no data.
//!
//! Every layer of the chain is metered, not only the last one: a layer may not unpack to more
//! than `max_stream_bytes`, and once it passes [`RATIO_FLOOR`] not to more than `max_ratio`
//! times the raw stream either. The final output is also charged against what is left of
//! `max_total_bytes`. A meter stops at the first byte over a cap, so a bomb costs time up to
//! the cap and almost no memory. The stop callback is asked as input is fed and as output
//! passes the meters.

use std::cell::Cell;
use std::io::{self, Write};

use lopdf::{Dictionary, Document, Object};

use crate::Limits;
use crate::objects::{get, resolve};

/// How often, in bytes moved through the chain, the stop callback is asked.
const STOP_EVERY: u64 = 1 << 20;

/// The ratio limit applies to a stream or a layer only once it unpacks past this size. Blank
/// images and opaque masks deflate far past 100:1 and are usually smaller.
pub(crate) const RATIO_FLOOR: u64 = 16 << 20;

/// Size of the scratch buffers of the decoding stages.
const CHUNK: usize = 32 * 1024;

/// One decoding step of a stream's filter chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Filter {
    Flate,
    Lzw {
        early_change: bool,
    },
    Ascii85,
    AsciiHex,
    RunLength,
    Brotli,
    /// Data that stays encoded (images, or a filter nobody here decodes). The chain ends.
    Opaque,
    /// A crypt filter. The objects are already decrypted, so it passes data through.
    Crypt,
}

impl Filter {
    pub(crate) fn from_name(name: &[u8], early_change: bool) -> Filter {
        match name {
            b"FlateDecode" | b"Fl" => Filter::Flate,
            b"LZWDecode" | b"LZW" => Filter::Lzw { early_change },
            b"ASCII85Decode" | b"A85" => Filter::Ascii85,
            b"ASCIIHexDecode" | b"AHx" => Filter::AsciiHex,
            b"RunLengthDecode" | b"RL" => Filter::RunLength,
            b"BrotliDecode" => Filter::Brotli,
            b"Crypt" => Filter::Crypt,
            _ => Filter::Opaque,
        }
    }
}

/// What measuring one stream found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Measure {
    /// The stream unpacks to this many bytes (or this many before it broke off).
    Fits(u64),
    /// The stream unpacks past a cap.
    Over(Cap),
    /// The caller asked to stop.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cap {
    /// More than `max_stream_bytes` (in the output or in a layer).
    Stream,
    /// More than `max_ratio` times the raw size, past [`RATIO_FLOOR`].
    Ratio,
    /// Past what is left of `max_total_bytes`.
    Total,
    /// A Flate layer took [`PROBE_MAX`] bytes of input without any output.
    Stall,
}

struct Shared<'a> {
    /// Bytes of the final output.
    count: Cell<u64>,
    max_stream: u64,
    /// `max_ratio` times the raw length, never below [`RATIO_FLOOR`].
    ratio_cap: u64,
    budget: u64,
    over: Cell<Option<Cap>>,
    stopped: Cell<bool>,
    /// Bytes moved through the chain, input and every layer.
    moved: Cell<u64>,
    next_check: Cell<u64>,
    stop: &'a dyn Fn() -> bool,
}

impl Shared<'_> {
    /// Checks one layer's running total and asks the stop callback now and then.
    fn charge(&self, layer: u64, last: bool, moved: usize) -> io::Result<()> {
        let cap = if layer > self.max_stream {
            Some(Cap::Stream)
        } else if layer > self.ratio_cap {
            Some(Cap::Ratio)
        } else if last && layer > self.budget {
            Some(Cap::Total)
        } else {
            None
        };
        if let Some(cap) = cap {
            self.over.set(Some(cap));
            return Err(io::Error::other("over the cap"));
        }
        self.poll(moved)
    }

    fn poll(&self, moved: usize) -> io::Result<()> {
        let m = self.moved.get().saturating_add(moved as u64);
        self.moved.set(m);
        if m >= self.next_check.get() {
            self.next_check.set(m.saturating_add(STOP_EVERY));
            if (self.stop)() {
                self.stopped.set(true);
                return Err(io::Error::other("stopped"));
            }
        }
        Ok(())
    }
}

/// The end of every chain: counts the final output and keeps none.
struct Sink<'s> {
    shared: &'s Shared<'s>,
}

impl Write for Sink<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s = self.shared;
        let n = s.count.get().saturating_add(buf.len() as u64);
        s.count.set(n);
        s.charge(n, true, buf.len())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Counts the output of one layer before it goes to the next.
struct Meter<'s, W: Write> {
    next: W,
    count: u64,
    shared: &'s Shared<'s>,
}

impl<W: Write> Write for Meter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.count = self.count.saturating_add(buf.len() as u64);
        self.shared.charge(self.count, false, buf.len())?;
        self.next.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.next.flush()
    }
}

/// LZW (ISO 32000-1 7.4.4) as a writer stage.
struct LzwStage<W: Write> {
    decoder: weezl::decode::Decoder,
    next: W,
    buf: Box<[u8]>,
    done: bool,
}

impl<W: Write> LzwStage<W> {
    fn new(next: W, early_change: bool) -> Self {
        use weezl::BitOrder;
        let decoder = if early_change {
            weezl::decode::Decoder::with_tiff_size_switch(BitOrder::Msb, 8)
        } else {
            weezl::decode::Decoder::new(BitOrder::Msb, 8)
        };
        LzwStage {
            decoder,
            next,
            buf: vec![0; CHUNK].into_boxed_slice(),
            done: false,
        }
    }

    fn pump(&mut self, mut input: &[u8]) -> io::Result<()> {
        while !self.done {
            let r = self.decoder.decode_bytes(input, &mut self.buf);
            self.next.write_all(&self.buf[..r.consumed_out])?;
            input = &input[r.consumed_in..];
            match r.status {
                Ok(weezl::LzwStatus::Done) => self.done = true,
                Ok(_) if r.consumed_in == 0 && r.consumed_out == 0 => break,
                Ok(_) => {}
                // lopdf keeps what came out before an error.
                Err(_) => self.done = true,
            }
        }
        Ok(())
    }
}

impl<W: Write> Write for LzwStage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pump(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.pump(&[])?;
        self.next.flush()
    }
}

/// ASCIIHexDecode as a writer stage.
struct HexStage<W: Write> {
    next: W,
    high: Option<u8>,
    done: bool,
}

impl<W: Write> Write for HexStage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = Vec::with_capacity(buf.len() / 2 + 1);
        for &ch in buf {
            if self.done {
                break;
            }
            if ch == b'>' {
                self.done = true;
                break;
            }
            if ch.is_ascii_whitespace() {
                continue;
            }
            let Some(nibble) = (ch as char).to_digit(16) else {
                self.done = true;
                break;
            };
            match self.high.take() {
                Some(h) => out.push((h << 4) | nibble as u8),
                None => self.high = Some(nibble as u8),
            }
        }
        self.next.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(h) = self.high.take() {
            self.next.write_all(&[h << 4])?;
        }
        self.next.flush()
    }
}

/// ASCII85Decode as a writer stage.
struct A85Stage<W: Write> {
    next: W,
    group: u64,
    count: usize,
    done: bool,
}

impl<W: Write> Write for A85Stage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = Vec::with_capacity(buf.len());
        for &ch in buf {
            if self.done {
                break;
            }
            match ch {
                b'~' => self.done = true,
                b'z' if self.count == 0 => out.extend_from_slice(&[0; 4]),
                c if c.is_ascii_whitespace() => {}
                b'!'..=b'u' => {
                    self.group = self.group * 85 + u64::from(ch - b'!');
                    self.count += 1;
                    if self.count == 5 {
                        out.extend_from_slice(&(self.group as u32).to_be_bytes());
                        self.group = 0;
                        self.count = 0;
                    }
                }
                _ => self.done = true,
            }
        }
        self.next.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.count > 1 {
            let mut g = self.group;
            for _ in self.count..5 {
                g = g * 85 + 84;
            }
            let bytes = (g as u32).to_be_bytes();
            self.next.write_all(&bytes[..self.count - 1])?;
        }
        self.count = 0;
        self.next.flush()
    }
}

/// RunLengthDecode as a writer stage.
struct RunStage<W: Write> {
    next: W,
    /// Literal bytes still to copy.
    literal: usize,
    /// A repeat count waiting for its byte.
    repeat: Option<usize>,
    done: bool,
}

impl<W: Write> Write for RunStage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut i = 0;
        while i < buf.len() && !self.done {
            if self.literal > 0 {
                let n = self.literal.min(buf.len() - i);
                self.next.write_all(&buf[i..i + n])?;
                self.literal -= n;
                i += n;
            } else if let Some(n) = self.repeat.take() {
                self.next.write_all(&[buf[i]; 128][..n])?;
                i += 1;
            } else {
                match buf[i] {
                    128 => self.done = true,
                    l @ 0..=127 => self.literal = usize::from(l) + 1,
                    l => self.repeat = Some(257 - usize::from(l)),
                }
                i += 1;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.next.flush()
    }
}

/// Input a Flate stage keeps until its first output byte, so it can replay a stream with a
/// broken zlib header as raw deflate. No output after this much input counts as over the cap:
/// lopdf would keep inflating, and the guard cannot keep buffering.
const PROBE_MAX: usize = 64 * 1024;

/// FlateDecode as a writer stage. Like lopdf, a zlib stream that fails before any output is
/// tried again as raw deflate from its third byte, at any place in the chain.
struct FlateStage<'s, W: Write> {
    inflater: flate2::Decompress,
    zlib: bool,
    next: W,
    buf: Box<[u8]>,
    probe: Option<Vec<u8>>,
    done: bool,
    shared: &'s Shared<'s>,
}

impl<'s, W: Write> FlateStage<'s, W> {
    fn new(next: W, shared: &'s Shared<'s>) -> Self {
        FlateStage {
            inflater: flate2::Decompress::new(true),
            zlib: true,
            next,
            buf: vec![0; CHUNK].into_boxed_slice(),
            probe: Some(Vec::new()),
            done: false,
            shared,
        }
    }

    fn feed(&mut self, mut input: &[u8]) -> io::Result<()> {
        while !self.done {
            let (in0, out0) = (self.inflater.total_in(), self.inflater.total_out());
            let res = self
                .inflater
                .decompress(input, &mut self.buf, flate2::FlushDecompress::None);
            let used = (self.inflater.total_in() - in0) as usize;
            let made = (self.inflater.total_out() - out0) as usize;
            input = &input[used.min(input.len())..];
            if made > 0 {
                self.probe = None;
                self.next.write_all(&self.buf[..made])?;
            }
            match res {
                Ok(flate2::Status::StreamEnd) => self.done = true,
                Ok(_) if used == 0 && made == 0 => break,
                Ok(_) => {}
                Err(_) => {
                    match self.probe.take() {
                        Some(probe) if self.zlib && probe.len() > 2 => {
                            self.zlib = false;
                            self.inflater = flate2::Decompress::new(false);
                            return self.feed(&probe[2..]);
                        }
                        // lopdf keeps what came out before an error.
                        _ => self.done = true,
                    }
                }
            }
        }
        Ok(())
    }
}

impl<W: Write> Write for FlateStage<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(probe) = self.probe.as_mut() {
            probe.extend_from_slice(buf);
            if probe.len() > PROBE_MAX {
                self.shared.over.set(Some(Cap::Stall));
                return Err(io::Error::other("no output from a long deflate stream"));
            }
        }
        self.feed(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.feed(&[])?;
        self.next.flush()
    }
}

/// Passes data on unchanged.
struct Pass<W: Write>(W);

impl<W: Write> Write for Pass<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn chain<'s>(filters: &[Filter], shared: &'s Shared<'s>) -> Box<dyn Write + 's> {
    let mut w: Box<dyn Write + 's> = Box::new(Sink { shared });
    for (i, f) in filters.iter().enumerate().rev() {
        // Every layer but the last writes through a meter.
        if i + 1 < filters.len() {
            w = Box::new(Meter {
                next: w,
                count: 0,
                shared,
            });
        }
        w = match f {
            Filter::Flate => Box::new(FlateStage::new(w, shared)),
            Filter::Lzw { early_change } => Box::new(LzwStage::new(w, *early_change)),
            Filter::Ascii85 => Box::new(A85Stage {
                next: w,
                group: 0,
                count: 0,
                done: false,
            }),
            Filter::AsciiHex => Box::new(HexStage {
                next: w,
                high: None,
                done: false,
            }),
            Filter::RunLength => Box::new(RunStage {
                next: w,
                literal: 0,
                repeat: None,
                done: false,
            }),
            Filter::Brotli => Box::new(brotli_decompressor::DecompressorWriter::new(w, 4096)),
            Filter::Crypt | Filter::Opaque => Box::new(Pass(w)),
        };
    }
    w
}

/// Feeds `raw` through the chain. Decoding errors end a stage quietly, as in lopdf; only the
/// sink (over a cap, or stopped) ends the run early.
fn run<'s>(raw: &[u8], filters: &[Filter], shared: &'s Shared<'s>) {
    let mut w = chain(filters, shared);
    for piece in raw.chunks(CHUNK) {
        if shared.poll(piece.len()).is_err() || w.write_all(piece).is_err() {
            return;
        }
    }
    let _ = w.flush();
}

/// Decodes `raw` through `filters` into a counting sink. `budget` is what is left of the total.
pub(crate) fn measure_bytes(
    raw: &[u8],
    filters: &[Filter],
    limits: &Limits,
    budget: u64,
    stop: &dyn Fn() -> bool,
) -> Measure {
    // The chain ends at the first filter whose output stays encoded.
    let end = filters
        .iter()
        .position(|f| *f == Filter::Opaque)
        .unwrap_or(filters.len());
    let filters = &filters[..end];
    let shared = Shared {
        count: Cell::new(0),
        max_stream: limits.max_stream_bytes,
        ratio_cap: limits
            .max_ratio
            .saturating_mul(raw.len() as u64)
            .max(RATIO_FLOOR),
        budget,
        over: Cell::new(None),
        stopped: Cell::new(false),
        moved: Cell::new(0),
        next_check: Cell::new(STOP_EVERY),
        stop,
    };
    run(raw, filters, &shared);
    if shared.stopped.get() {
        return Measure::Stopped;
    }
    if let Some(cap) = shared.over.get() {
        return Measure::Over(cap);
    }
    Measure::Fits(shared.count.get())
}

pub(crate) fn cap_words(cap: Cap, limits: &Limits) -> String {
    match cap {
        Cap::Stream => format!("unpacks to more than {} bytes", limits.max_stream_bytes),
        Cap::Ratio => format!(
            "unpacks to more than {} times its size and more than {} bytes",
            limits.max_ratio, RATIO_FLOOR
        ),
        Cap::Stall => {
            format!("gives no output for its first {PROBE_MAX} bytes, so it cannot be checked")
        }
        Cap::Total => format!(
            "would take the document past {} unpacked bytes",
            limits.max_total_bytes
        ),
    }
}

/// The filter chain of a loaded stream.
pub(crate) fn stream_filters(doc: &Document, dict: &Dictionary) -> Vec<Filter> {
    let names: Vec<Vec<u8>> = match get(doc, dict, b"Filter") {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|o| match resolve(doc, o) {
                Some(Object::Name(n)) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let params: Vec<Option<&Dictionary>> = match get(doc, dict, b"DecodeParms") {
        Some(Object::Dictionary(d)) => vec![Some(d)],
        Some(Object::Array(a)) => a
            .iter()
            .map(|o| match resolve(doc, o) {
                Some(Object::Dictionary(d)) => Some(d),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let early = params
                .get(i)
                .copied()
                .flatten()
                .and_then(|p| p.get(b"EarlyChange").ok())
                .and_then(|o| o.as_i64().ok())
                .is_none_or(|v| v != 0);
            Filter::from_name(n, early)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn chains_decode_to_the_right_length() {
        let limits = Limits::default();
        let m = |raw: &[u8], f: &[Filter]| measure_bytes(raw, f, &limits, u64::MAX, &never);
        assert_eq!(m(b"hello", &[]), Measure::Fits(5));
        assert_eq!(
            m(&zlib(b"hello world"), &[Filter::Flate]),
            Measure::Fits(11)
        );
        assert_eq!(m(b"68656c6c6f>", &[Filter::AsciiHex]), Measure::Fits(5));
        assert_eq!(m(b"6", &[Filter::AsciiHex]), Measure::Fits(1));
        assert_eq!(
            m(b"87cURD]i,\"Ebo80~>", &[Filter::Ascii85]),
            Measure::Fits(12)
        );
        assert_eq!(m(b"z~>", &[Filter::Ascii85]), Measure::Fits(4));
        // A literal of 3 bytes, then 4 copies of one byte, then the end.
        assert_eq!(
            m(&[2, b'a', b'b', b'c', 253, b'x', 128], &[Filter::RunLength]),
            Measure::Fits(7)
        );
        // Hex wrapped around deflate.
        let hex: String = zlib(b"abcdef").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            m(hex.as_bytes(), &[Filter::AsciiHex, Filter::Flate]),
            Measure::Fits(6)
        );
        // An image codec ends the chain: its input is what is counted.
        assert_eq!(
            m(b"jpegdata", &[Filter::Opaque, Filter::Flate]),
            Measure::Fits(8)
        );
    }

    #[test]
    fn broken_zlib_header_is_retried_as_raw_deflate() {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(&[0u8; 100_000]).unwrap();
        let mut raw = vec![0xFF, 0xFF];
        raw.extend(e.finish().unwrap());
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(&raw, &[Filter::Flate], &limits, u64::MAX, &never),
            Measure::Fits(100_000)
        );
    }

    #[test]
    fn caps_are_told_apart() {
        let packed = zlib(&[0u8; 200_000]);
        let small = Limits {
            max_stream_bytes: 1000,
            max_ratio: 1_000_000,
            ..Limits::default()
        };
        assert_eq!(
            measure_bytes(&packed, &[Filter::Flate], &small, u64::MAX, &never),
            Measure::Over(Cap::Stream)
        );
        // Far over 100:1, but under the 16 MiB floor of the ratio limit.
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(&packed, &[Filter::Flate], &limits, u64::MAX, &never),
            Measure::Fits(200_000)
        );
        let big = zlib(&vec![0u8; 20 << 20]);
        assert_eq!(
            measure_bytes(&big, &[Filter::Flate], &limits, u64::MAX, &never),
            Measure::Over(Cap::Ratio)
        );
        let roomy = Limits {
            max_ratio: 1_000_000,
            ..Limits::default()
        };
        assert_eq!(
            measure_bytes(&packed, &[Filter::Flate], &roomy, 10, &never),
            Measure::Over(Cap::Total)
        );
    }

    #[test]
    fn lzw_counts() {
        // "-----A---B" from ISO 32000-1, 7.4.4.2, with EarlyChange 1.
        let raw = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(
                &raw,
                &[Filter::Lzw { early_change: true }],
                &limits,
                u64::MAX,
                &never
            ),
            Measure::Fits(10)
        );
    }

    #[test]
    fn every_layer_is_metered() {
        // The Flate layer unpacks to 2000 spaces, the hex layer to nothing.
        let packed = zlib(&[b' '; 2000]);
        let small = Limits {
            max_stream_bytes: 1000,
            ..Limits::default()
        };
        assert_eq!(
            measure_bytes(
                &packed,
                &[Filter::Flate, Filter::AsciiHex],
                &small,
                u64::MAX,
                &never
            ),
            Measure::Over(Cap::Stream)
        );
    }

    #[test]
    fn stop_is_asked_while_input_is_fed() {
        // Plain data with no filter still asks the callback.
        let raw = vec![b'a'; 4 << 20];
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(&raw, &[], &limits, u64::MAX, &|| true),
            Measure::Stopped
        );
    }
}
