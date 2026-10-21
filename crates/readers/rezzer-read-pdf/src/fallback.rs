// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The fallback parser: the text of a file pdfparts cannot open, or of a page with fonts whose
//! canonical copy gave no text, read with the same hayro text walk from the file itself.
//!
//! This is the only place hayro sees the bytes of the file. Before it does, pdfparts measures
//! every stream of the file with its guard's decoder ([`RawStreams`]): a stream over a cap, a
//! stream whose filters are named indirectly and an encrypted file keep the fallback out. The
//! text walk then charges each stream with that size, under the same caps and kept budget as
//! the canonical read, and hayro's page walk keeps each page tree node once.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::Page;
use hayro_syntax::page::Rotation;
use pdfparts::{FilterParms, OpenError, PageInfo, Pdf, RawStreams, measure_stream};
use rezzer_limits::Budget;
use rezzer_model::{Bytes, LossReason, ReadOptions, ReadOutcome, Severity, Source};

use crate::open;
use crate::quote::Marked;
use crate::text::{self, Caps, Extracted, Origin};
use crate::{Out, lay_out};

/// The bytes of the file, shared with the parser without a copy.
struct Shared(Bytes);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// The file as the fallback parser reads it, with the size of every stream. The password is
/// only handed to the parser; it is never printed.
pub(crate) struct Raw<'b> {
    bytes: Arc<Shared>,
    password: Option<String>,
    streams: RawStreams,
    limits: pdfparts::Limits,
    /// What the text walk may still copy out of the file.
    copy_left: Cell<u64>,
    /// Streams the scan did not find, measured when the walk asks for them.
    measured: RefCell<HashMap<(u32, u16), u64>>,
    /// The number of pages the object layer found, when it opened the file.
    pages: Option<u32>,
    stop: &'b dyn Fn() -> bool,
}

impl<'b> Raw<'b> {
    /// Measures every stream of `bytes`. `Err` says why the fallback must not read the file.
    fn new(
        bytes: &Bytes,
        password: Option<&str>,
        budget: &Budget,
        pages: Option<u32>,
        stop: &'b dyn Fn() -> bool,
    ) -> Result<Raw<'b>, String> {
        let limits = open::limits(budget);
        let streams = panic::catch_unwind(AssertUnwindSafe(|| {
            RawStreams::measure(bytes.as_slice(), &limits, stop)
        }))
        .map_err(|_| "measuring its streams stopped with an internal error".to_string())?
        .map_err(|e| e.to_string())?;
        Ok(Raw {
            bytes: Arc::new(Shared(bytes.clone())),
            password: password.map(str::to_string),
            streams,
            limits,
            copy_left: Cell::new(limits.max_object_bytes),
            measured: RefCell::new(HashMap::new()),
            pages,
            stop,
        })
    }

    pub(crate) fn charge_copy(&self, bytes: u64) -> bool {
        match self.copy_left.get().checked_sub(bytes) {
            Some(left) => {
                self.copy_left.set(left);
                true
            }
            None => {
                self.copy_left.set(0);
                false
            }
        }
    }

    /// Takes `bytes` for memory held for a while, only when they fit; a reservation that
    /// does not fit leaves the pool as it is. [`Raw::release_copy`] gives them back.
    pub(crate) fn reserve_copy(&self, bytes: u64) -> bool {
        match self.copy_left.get().checked_sub(bytes) {
            Some(left) => {
                self.copy_left.set(left);
                true
            }
            None => false,
        }
    }

    pub(crate) fn release_copy(&self, bytes: u64) {
        self.copy_left
            .set(self.copy_left.get().saturating_add(bytes));
    }

    pub(crate) fn page_count(&self) -> Option<u32> {
        self.pages
    }

    pub(crate) fn open(&self) -> Result<hayro_syntax::Pdf, hayro_syntax::LoadPdfError> {
        let data = hayro_syntax::PdfData::from(Arc::clone(&self.bytes));
        hayro_syntax::Pdf::new_with_password(data, self.password.as_deref().unwrap_or(""))
    }

    /// What stream `s` unpacks to: the size the scan measured, or for a stream the scan did
    /// not find, its data measured now through the filters it names. A stream that passes a
    /// cap counts as larger than any page may take.
    pub(crate) fn stream_size(&self, s: &Stream<'_>) -> u64 {
        let id = s.obj_id();
        let key = match (u32::try_from(id.obj_number), u16::try_from(id.gen_number)) {
            (Ok(n), Ok(g)) => (n, g),
            _ => return u64::MAX,
        };
        // A damaged stream counts as more than any page may take: it is never unpacked.
        if self.streams.broken(key.0, key.1) {
            return u64::MAX;
        }
        if let Some(n) = self.streams.size(key.0, key.1) {
            return n;
        }
        if let Some(n) = self.measured.borrow().get(&key) {
            return *n;
        }
        let (names, parms) = filters_of(s.dict());
        let names: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
        let size = measure_stream(
            &s.raw_data(),
            &names,
            &parms,
            &self.limits,
            self.limits.max_total_bytes,
            self.stop,
        )
        .unwrap_or(u64::MAX);
        self.measured.borrow_mut().insert(key, size);
        size
    }
}

/// Objects one page's check for damaged streams may visit.
const REACH_MAX: usize = 100_000;

impl Raw<'_> {
    /// Why the parser must not read `page`: it reaches a stream that stops with a decoding
    /// error, which a lenient decoder may unpack further than was measured. The walk follows
    /// what the text walk unpacks: the page's content and its resources, with all they
    /// reach but parents.
    pub(crate) fn unsafe_page<'p>(&self, page: &Page<'p>) -> Option<String> {
        if !self.streams.any_broken() {
            return None;
        }
        let xref: &'p hayro_syntax::xref::XRef = page.xref();
        // The page's content, and its resources with the ones it inherits.
        let mut todo: Vec<Object<'p>> = Vec::new();
        let res = page.resources();
        for d in [
            &res.ext_g_states,
            &res.fonts,
            &res.color_spaces,
            &res.properties,
            &res.x_objects,
            &res.patterns,
            &res.shadings,
        ] {
            todo.push(Object::Dict(d.clone()));
        }
        let mut seen: HashSet<ObjectIdentifier> = HashSet::new();
        let contents = page.raw().get_raw::<Object<'p>>(b"Contents");
        let mut pending: Vec<MaybeRef<Object<'p>>> = contents.into_iter().collect();
        loop {
            let obj = match (pending.pop(), todo.pop()) {
                (Some(v), back) => {
                    todo.extend(back);
                    match v {
                        MaybeRef::NotRef(o) => o,
                        MaybeRef::Ref(r) => {
                            let (n, g) = (
                                u32::try_from(r.obj_number).unwrap_or(0),
                                u16::try_from(r.gen_number).unwrap_or(0),
                            );
                            if self.streams.broken(n, g) {
                                return Some(format!(
                                    "it uses stream {n} {g}, which stops with a decoding error"
                                ));
                            }
                            let id: ObjectIdentifier = r.into();
                            if !seen.insert(id) {
                                continue;
                            }
                            match xref.get::<Object<'p>>(id) {
                                Some(o) => o,
                                None => continue,
                            }
                        }
                    }
                }
                (None, Some(o)) => o,
                (None, None) => break,
            };
            let mut take = |v: MaybeRef<Object<'p>>,
                            todo: &mut Vec<Object<'p>>|
             -> Option<String> {
                match v {
                    MaybeRef::Ref(r) => {
                        let id: ObjectIdentifier = r.into();
                        let (n, g) = (
                            u32::try_from(r.obj_number).unwrap_or(0),
                            u16::try_from(r.gen_number).unwrap_or(0),
                        );
                        if self.streams.broken(n, g) {
                            return Some(format!(
                                "it uses stream {n} {g}, which stops with a decoding error"
                            ));
                        }
                        if seen.insert(id) {
                            if seen.len() > REACH_MAX {
                                return Some(format!(
                                    "it reaches more than {REACH_MAX} objects, too many to check \
                                     for damaged streams"
                                ));
                            }
                            todo.extend(xref.get::<Object<'p>>(id));
                        }
                        None
                    }
                    MaybeRef::NotRef(o) => {
                        todo.push(o);
                        None
                    }
                }
            };
            let dict = match &obj {
                Object::Dict(d) => Some(d.clone()),
                Object::Stream(s) => Some(s.dict().clone()),
                Object::Array(a) => {
                    for v in a.raw_iter() {
                        if let Some(why) = take(v, &mut todo) {
                            return Some(why);
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some(d) = dict {
                for (k, v) in d.entries() {
                    let k = k.as_ref();
                    if k == b"Parent" || k == b"P" {
                        continue;
                    }
                    if let Some(why) = take(v, &mut todo) {
                        return Some(why);
                    }
                }
            }
        }
        None
    }
}

/// The filter names of a stream dictionary (or an inline image's), in order, with the
/// parameters that change what each unpacks to.
pub(crate) fn filters_of(dict: &Dict<'_>) -> (Vec<Vec<u8>>, Vec<FilterParms>) {
    let names: Vec<Vec<u8>> = match dict
        .get::<Name<'_>>(b"Filter")
        .or_else(|| dict.get::<Name<'_>>(b"F"))
    {
        Some(name) => vec![name.as_ref().to_vec()],
        None => dict
            .get::<Array<'_>>(b"Filter")
            .or_else(|| dict.get::<Array<'_>>(b"F"))
            .map(|a| a.iter::<Name<'_>>().map(|n| n.as_ref().to_vec()).collect())
            .unwrap_or_default(),
    };
    let one = |d: Option<Dict<'_>>| FilterParms {
        early_change: d.and_then(|d| d.get::<i64>(b"EarlyChange")) != Some(0),
    };
    let parms: Vec<FilterParms> = match dict
        .get::<Dict<'_>>(b"DecodeParms")
        .or_else(|| dict.get::<Dict<'_>>(b"DP"))
    {
        Some(d) => vec![one(Some(d))],
        None => dict
            .get::<Array<'_>>(b"DecodeParms")
            .or_else(|| dict.get::<Array<'_>>(b"DP"))
            .map(|a| a.iter::<Object<'_>>().map(|o| one(o.into_dict())).collect())
            .unwrap_or_default(),
    };
    (names, parms)
}

/// The page sizes as the parser sees them, without labels, fonts or pictures.
fn page_infos(doc: &hayro_syntax::Pdf) -> Vec<PageInfo> {
    doc.pages()
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let crop = p.intersected_crop_box();
            PageInfo {
                number: i as u32 + 1,
                label: None,
                width: (crop.x1 - crop.x0) as f32,
                height: (crop.y1 - crop.y0) as f32,
                rotation: match p.rotation() {
                    Rotation::None => 0,
                    Rotation::Horizontal => 90,
                    Rotation::Flipped => 180,
                    Rotation::FlippedHorizontal => 270,
                },
                has_fonts: false,
                images: Arc::from(Vec::new()),
            }
        })
        .collect()
}

/// Reads the text of a file pdfparts could not open (`e`, NotPdf or Damaged) with the
/// fallback parser. The read is partial: the object layer is not read. When the fallback
/// cannot read the file either, the outcome is the failed one, with why.
pub(crate) fn read_file(
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
    e: &OpenError,
    mut out: Out,
    text_stop: &dyn Fn() -> bool,
) -> ReadOutcome {
    let stop = || budget.check_time().is_err();
    let unused = |mut out: Out, why: String| {
        out.loss(
            "document",
            format!("the fallback parser was not used: {why}"),
            LossReason::Unsupported,
            Severity::Info,
        );
        open::unreadable(e, out)
    };
    let raw = match Raw::new(&src.bytes, opts.password.as_deref(), budget, None, &stop) {
        Ok(raw) => raw,
        Err(why) => return unused(out, why),
    };
    if let Err(hit) = budget.add_unpacked(raw.streams.unpacked()) {
        return unused(out, hit.to_string());
    }
    let pages = match panic::catch_unwind(AssertUnwindSafe(|| raw.open().map(|d| page_infos(&d)))) {
        Ok(Ok(pages)) if !pages.is_empty() => pages,
        Ok(Ok(_)) => return unused(out, "it finds no pages either".into()),
        Ok(Err(err)) => return unused(out, format!("it cannot open the file either ({err:?})")),
        Err(_) => return unused(out, "it stopped with an internal error".into()),
    };
    out.loss(
        "document",
        format!(
            "the object layer could not be read ({e}); text read with the fallback parser, \
             annotations, forms, scripts and signatures not read"
        ),
        LossReason::Uncertain,
        Severity::Warning,
    );
    // Whatever the text says, the object layer is missing.
    out.partial = true;
    let caps = Caps::new(
        open::limits(budget).max_items,
        budget.limits().max_memory_bytes,
        0,
    );
    let wanted: Vec<u32> = pages.iter().map(|p| p.number).collect();
    let extracted = text::pages_of(
        Origin::Fallback(&raw),
        &wanted,
        &[],
        &Marked::new(),
        &text::Widgets::new(),
        caps,
        text_stop,
    );
    let (_, _, needs) = lay_out(None, &pages, extracted, &[], budget, &mut out);
    // The renderer reads only pdfparts' canonical copy, and there is none.
    if opts.extract_assets
        && let Some(first) = needs.first()
    {
        let k = needs.len();
        let what = if k == 1 {
            format!("page {} is not rendered", first.page)
        } else {
            format!(
                "{k} pages that need vision are not rendered, from page {} on",
                first.page
            )
        };
        out.loss(
            format!("p{}", first.page),
            format!("{what}: the file is read with the fallback parser"),
            LossReason::Unsupported,
            Severity::Warning,
        );
    }
    out.finish()
}

/// Reads `wanted` pages of `pdf` again from the file itself, for pages with fonts whose
/// canonical copy gave no text. `None` when the fallback must not or cannot read the file.
pub(crate) fn pages(
    pdf: &Pdf,
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
    wanted: &[u32],
    caps: Caps,
    stop: &dyn Fn() -> bool,
) -> Option<Extracted> {
    let raw = Raw::new(
        &src.bytes,
        opts.password.as_deref(),
        budget,
        Some(pdf.page_count()),
        stop,
    )
    .ok()?;
    // What the parser may unpack counts like the first read's streams.
    budget.add_unpacked(raw.streams.unpacked()).ok()?;
    let extracted = text::pages_of(
        Origin::Fallback(&raw),
        wanted,
        &[],
        &Marked::new(),
        &text::Widgets::new(),
        caps,
        stop,
    );
    Some(extracted)
}
