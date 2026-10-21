// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The text of the pages with positions, read by hayro-interpret from pdfparts' canonical bytes,
//! never from the file itself. This is the only file that reads text with hayro.
//!
//! Each page is read in two passes. A walk of its content operators (forms included, the way
//! the interpreter runs them) counts the work the page takes, collects the marked content and
//! the fonts the page uses, and finds where the page has to be cut. Then hayro-interpret runs
//! the page, cut there, into a text [`Device`] that keeps every glyph with its position. The
//! glyphs become runs and lines in reading order with Rezzer's own rules, in `order.rs`.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;

use hayro_interpret::font::{Glyph, GlyphRun};
use hayro_interpret::hayro_cmap::{self, BfString, CMap, CidFamily};
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret,
};
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::content::{TypedIter, UntypedIter};
use hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream, dict_or_stream};
use hayro_syntax::page::{Page, Resources};
use kurbo::{Affine, BezPath, Point};
use pdfparts::{Link as PdfLink, LinkTarget, Pdf};
use read_fonts::TableProvider;
use read_fonts::ps::agl;
use read_fonts::ps::cff::CffFontRef;
use skrifa::{FontRef, MetadataProvider};

use crate::fallback::Raw;
use crate::layout::{Run, UriTarget};
use crate::order::{Turn, runs_of};
use crate::quote::{Marked, Quote, Quoting};
use crate::tables::{Rule, RuleKind};

/// Glyphs and device calls between two questions to the stop callback.
const POLL: u64 = 4096;

/// Bytes a stream unpacks to per unit of work.
const BYTES_PER_UNIT: u64 = 64;

/// Nesting of forms the interpreter follows (hayro-interpret's own bound).
const FORM_DEPTH: usize = 50;

/// Glyph and link pairs one page may test for a URI link over a glyph.
const LINK_WORK: usize = 16_000_000;

/// What the text parser found on one page. Any text layer that fills it must keep this
/// contract, because the layout has no reading order logic of its own:
///
/// - `runs` come in reading order, one column after the other: a column is read to its end
///   before the next one starts. Text that does not read along the x axis (margin notes,
///   stamps, vertical writing) comes after the rest of the page.
/// - The runs of one visual line are next to each other and share [`Run::line`]; line numbers
///   grow in reading order on the page. `invisible` runs number their lines the same way, in
///   their own order.
/// - Inside a line, a run that starts a new word begins with a space; a run that goes on a
///   word (a kerning split, a style change inside a word) does not. The first run of a line
///   needs no space.
/// - Coordinates are points in the visible page box, origin at its lower left corner, `y`
///   upward, `/Rotate` not applied: `y` lies between 0 and the page height, which running
///   header and footer detection relies on. When the frame was turned to make the text read
///   left to right, `turned` is set and the coordinates are not page coordinates.
/// - Text that puts no glyphs on the page (render mode 3 or 7) is never in `runs`.
#[derive(Debug, Default)]
pub(crate) struct PageItems {
    pub page: u32,
    /// Visible text in reading order; the runs of one line are next to each other.
    pub runs: Vec<Run>,
    /// Boxes of the pictures the page draws: x0, y0, x1, y1.
    pub images: Vec<[f32; 4]>,
    /// For each of `images`, whether its pixels could be a barcode or QR code: 1-bit and at
    /// least 21 on a side.
    pub images_code: Vec<bool>,
    /// Straight lines and rectangles the page draws, in the frame of the page box (not
    /// turned), for finding ruled tables.
    pub rules: Vec<Rule>,
    /// The lower left corner of the visible page box in user space: what moves boxes of the
    /// object layer (links, form fields) into the frame of the runs.
    pub origin: (f32, f32),
    /// Text that puts no glyphs on the page (render mode 3 or 7), in reading order, on a page
    /// that also shows visible text.
    pub invisible: Vec<Run>,
    /// The lines of the invisible text of a page without visible text, as an OCR program
    /// writes it behind a scan. When it is filled, `runs` and `invisible` are empty.
    pub hidden_layer: Vec<String>,
    /// The page has more URI links than were matched to its runs.
    pub links_cut: bool,
    /// The frame was turned a quarter or half so the text reads left to right; the
    /// coordinates are not page coordinates then, and no running header or footer is looked
    /// for.
    pub turned: bool,
    /// Text in optional content the file switches off, by layer name, in reading order.
    pub hidden_layers: Vec<(String, Vec<Run>)>,
    /// Visible glyphs with a Unicode mapping.
    pub mapped: u64,
    /// Glyphs without a Unicode mapping; each stands as U+FFFD in the text.
    pub unmapped: u64,
    /// Visible glyphs without a Unicode mapping.
    pub unmapped_visible: u64,
    /// What on this page was not read, and why.
    pub notes: Vec<Note>,
    /// A limit cut the page's text: what is missing was not read, it is not missing in the
    /// file.
    pub in_part: bool,
    /// What the page was charged against the bytes the pages may keep until laid out.
    pub kept: u64,
    /// The text under each annotation of the page that marks text, quoted from its glyphs.
    /// The quotes are charged to the bytes the file may keep for good: they go into the
    /// model as they are.
    pub quotes: Vec<Quote>,
    /// The text was read from the file itself by the fallback parser.
    pub fallback: bool,
}

impl PageItems {
    /// Whether the page gave any text, visible or not.
    pub(crate) fn has_text(&self) -> bool {
        !self.runs.is_empty()
            || !self.invisible.is_empty()
            || !self.hidden_layer.is_empty()
            || !self.hidden_layers.is_empty()
            || self.mapped > 0
            || self.unmapped > 0
    }

    /// Copies the runs into fresh allocations.
    fn compact(&mut self) {
        let copy = |list: &mut Vec<Run>| *list = list.to_vec();
        copy(&mut self.runs);
        copy(&mut self.invisible);
        self.rules = self.rules.to_vec();
        for (_, list) in &mut self.hidden_layers {
            copy(list);
        }
    }

    /// About the memory the items take once their lists are shrunk to fit.
    fn kept_bytes(&mut self) -> u64 {
        // A link's target is shared by its runs: it counts once.
        let mut links: HashSet<*const UriTarget> = HashSet::new();
        let mut runs = |list: &mut Vec<Run>| -> u64 {
            list.shrink_to_fit();
            list.iter()
                .map(|r| {
                    let link = match &r.uri {
                        Some(u) if links.insert(Rc::as_ptr(u)) => u.uri.len() as u64 + 64,
                        _ => 0,
                    };
                    (std::mem::size_of::<Run>() + 32 + r.text.len()) as u64 + link
                })
                .sum()
        };
        let mut total = runs(&mut self.runs) + runs(&mut self.invisible);
        for (name, list) in &mut self.hidden_layers {
            total += name.len() as u64 + runs(list);
        }
        total += self
            .hidden_layer
            .iter()
            .map(|l| l.len() as u64 + 32)
            .sum::<u64>();
        self.rules.shrink_to_fit();
        total
            + 17 * self.images.len() as u64
            + (std::mem::size_of::<Rule>() * self.rules.len()) as u64
            + 256
    }
}

/// A part of a page that was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Note {
    /// A limit cut the page.
    Limit(String),
    /// A part of the page is broken.
    Damaged(String),
    /// A part is not read on purpose.
    Unsupported(String),
}

/// Why a page was not read at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fail {
    Damaged,
    Limit,
    /// The text parser stopped with an internal error.
    Internal,
}

/// A page whose text was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub page: u32,
    pub why: String,
    pub kind: Fail,
}

/// The pages read, the pages that failed and where a stop cut the reading.
#[derive(Debug, Default)]
pub(crate) struct Extracted {
    pub pages: Vec<PageItems>,
    pub failed: Vec<Failure>,
    /// The first page not read because the stop callback said so.
    pub stopped_at: Option<u32>,
    /// What is left of the bytes the pages may keep ([`Caps::max_kept`]): the room the model's
    /// copies of link targets may take.
    pub kept_left: u64,
    /// The quotes left out, if any.
    pub quotes_cut: Option<crate::quote::Cut>,
    /// The bound on glyph visits for quotes.
    pub quote_visits: usize,
}

/// Bounds for reading one page.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caps {
    /// Work one page may take: one unit per content operator, per byte of shown text and
    /// ActualText, and per 64 bytes a stream unpacks to, forms counted each time they run.
    pub max_items: usize,
    /// Glyphs one page may build: the interpreter builds every glyph of a show before the
    /// device sees it, so the walk counts shown bytes against this bound first.
    pub max_glyphs: usize,
    /// Bytes the pages read so far may keep together until they are laid out: runs, their
    /// text and the other items. Past it, the later pages are not read.
    pub max_kept: u64,
}

impl Caps {
    /// The caps for a read with `max_items` and `max_memory` bytes, of which the object layer
    /// took `objects` at open: a page's glyphs cost about 600 bytes each until laid out and
    /// may take a quarter of the memory, less half of what the objects take, but never less
    /// than a sixteenth; what the pages keep for layout may take a sixteenth, as building
    /// each page leaves memory behind.
    pub(crate) fn new(max_items: usize, max_memory: u64, objects: u64) -> Caps {
        let for_glyphs = (max_memory / 4)
            .saturating_sub(objects / 2)
            .max(max_memory / 16);
        let glyphs = usize::try_from(for_glyphs / 600).unwrap_or(usize::MAX);
        Caps {
            max_items,
            max_glyphs: glyphs.max(1_000),
            max_kept: (max_memory / 16).max(1 << 20),
        }
    }
}

/// What the text parser reads: pdfparts' canonical bytes, whose streams the guard measured,
/// or, for the fallback parser, the file itself, whose streams [`Raw`] measured.
#[derive(Clone, Copy)]
pub(crate) enum Origin<'p> {
    Canonical(&'p Pdf),
    Fallback(&'p Raw<'p>),
}

impl Origin<'_> {
    /// Takes `bytes` from the pool for what the readers copy out of the document; false when
    /// they do not fit.
    fn charge_copy(self, bytes: u64) -> bool {
        match self {
            Origin::Canonical(pdf) => pdf.charge_copy(bytes),
            Origin::Fallback(raw) => raw.charge_copy(bytes),
        }
    }

    /// The number of pages the parser's page walk must find, if it is known.
    fn page_count(self) -> Option<u32> {
        match self {
            Origin::Canonical(pdf) => Some(pdf.page_count()),
            Origin::Fallback(raw) => raw.page_count(),
        }
    }

    fn open(self) -> Result<hayro_syntax::Pdf, hayro_syntax::LoadPdfError> {
        match self {
            Origin::Canonical(pdf) => hayro_syntax::Pdf::new(pdf.canonical_shared()),
            Origin::Fallback(raw) => raw.open(),
        }
    }
}

/// Reads the text of `wanted` pages (1-based, ascending, none refused by the guard) from the
/// canonical bytes, and quotes the text under the annotations of `marked`. `stop` is asked
/// before each page and while a page is read; a page the stop cuts is not kept.
pub(crate) fn pages(
    pdf: &Pdf,
    wanted: &[u32],
    links: &[PdfLink],
    marked: &Marked<'_>,
    caps: Caps,
    stop: &dyn Fn() -> bool,
) -> Extracted {
    pages_of(Origin::Canonical(pdf), wanted, links, marked, caps, stop)
}

/// [`pages`] from either origin.
pub(crate) fn pages_of(
    origin: Origin<'_>,
    wanted: &[u32],
    links: &[PdfLink],
    marked: &Marked<'_>,
    caps: Caps,
    stop: &dyn Fn() -> bool,
) -> Extracted {
    // Pages that are not read keep nothing: what they may keep stays for the rest.
    let mut out = Extracted {
        kept_left: caps.max_kept,
        quote_visits: caps.max_items,
        ..Extracted::default()
    };
    if wanted.is_empty() {
        return out;
    }
    let all_fail = |out: &mut Extracted, why: String, kind: Fail| {
        out.failed.extend(wanted.iter().map(|&page| Failure {
            page,
            why: why.clone(),
            kind,
        }));
    };
    let opened = panic::catch_unwind(AssertUnwindSafe(|| origin.open()));
    let doc = match opened {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => {
            all_fail(
                &mut out,
                format!("it cannot open the file ({e:?})"),
                Fail::Damaged,
            );
            return out;
        }
        Err(_) => {
            all_fail(&mut out, "opening the file".into(), Fail::Internal);
            return out;
        }
    };
    let found = match panic::catch_unwind(AssertUnwindSafe(|| doc.pages())) {
        Ok(found) => found,
        Err(_) => {
            all_fail(&mut out, "walking the pages".into(), Fail::Internal);
            return out;
        }
    };
    if let Some(count) = origin.page_count()
        && found.len() != count as usize
    {
        all_fail(
            &mut out,
            format!(
                "it finds {} pages where the object layer finds {count}, so it could number \
                 them differently",
                found.len(),
            ),
            Fail::Damaged,
        );
        return out;
    }
    let links = uri_links(links);
    let off = switched_off(&doc);
    let mut reader = PageReader::new(origin, caps, stop, off);
    for &n in wanted {
        if stop() {
            out.stopped_at = Some(n);
            break;
        }
        let Some(page) = usize::try_from(n)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| found.get(i))
        else {
            continue;
        };
        let on_page = links.get(&n).map_or(&[][..], Vec::as_slice);
        let marks = marked.get(&n).map_or(&[][..], Vec::as_slice);
        match reader.page(n, page, on_page, marks) {
            Read::Page(items) => out.pages.push(*items),
            Read::Failed(f) => out.failed.push(f),
            Read::Stopped => {
                out.stopped_at = Some(n);
                break;
            }
        }
    }
    out.kept_left = reader.kept_left;
    out.quote_visits = reader.quoting.max();
    out.quotes_cut = reader.quoting.cut;
    out
}

/// The optional content groups the default configuration switches off, read the way the
/// interpreter reads them.
fn switched_off(doc: &hayro_syntax::Pdf) -> HashSet<ObjectIdentifier> {
    let mut off = HashSet::new();
    let xref = doc.xref();
    let Some(props) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|c| c.get::<Dict<'_>>(b"OCProperties"))
    else {
        return off;
    };
    let Some(config) = props.get::<Dict<'_>>(b"D") else {
        return off;
    };
    let refs = |a: Array<'_>| -> Vec<ObjectIdentifier> {
        a.raw_iter()
            .filter_map(|i| i.as_obj_ref())
            .map(Into::into)
            .collect()
    };
    if config.get::<Name<'_>>(b"BaseState").as_deref() == Some(b"OFF")
        && let Some(all) = props.get::<Array<'_>>(b"OCGs")
    {
        off.extend(refs(all));
    }
    if let Some(on) = config.get::<Array<'_>>(b"ON") {
        for id in refs(on) {
            off.remove(&id);
        }
    }
    if let Some(list) = config.get::<Array<'_>>(b"OFF") {
        off.extend(refs(list));
    }
    off
}

/// A URI link box (x0, y0, x1, y1 in user space) and its target.
pub(crate) type Link = ([f32; 4], Rc<UriTarget>);

/// The URI links of every page, from the object layer. They are never followed. Links that
/// share one address share one target.
fn uri_links(links: &[PdfLink]) -> HashMap<u32, Vec<Link>> {
    let mut out: HashMap<u32, Vec<Link>> = HashMap::new();
    let mut targets: HashMap<*const u8, Rc<UriTarget>> = HashMap::new();
    for link in links {
        if let LinkTarget::Uri(uri) = &link.target {
            let target = targets
                .entry(uri.as_ptr())
                .or_insert_with(|| UriTarget::new(uri));
            let [a, b, c, d] = link.rect;
            out.entry(link.page)
                .or_default()
                .push(([a.min(c), b.min(d), a.max(c), b.max(d)], Rc::clone(target)));
        }
    }
    out
}

/// What reading one page gave.
enum Read {
    Page(Box<PageItems>),
    Failed(Failure),
    Stopped,
}

/// Why the device ended a page early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Abort {
    Stop,
    Cap,
}

/// What the walk learned about a font, keyed by hayro's cache key of the font dictionary.
#[derive(Debug, Clone, Default)]
struct FontInfo {
    bold: bool,
    italic: bool,
    /// A Type0 font that writes top to bottom.
    vertical: bool,
    /// The font program is in the file, so its own tables describe its glyphs.
    embedded: bool,
    /// The embedded program and its unpacked size: a program of another size in use is a
    /// stand-in for one that did not load.
    program: Option<(ObjectIdentifier, u64)>,
    /// The CID collection of a CID font whose glyph ids are its CIDs (a TrueType CID font with
    /// the identity CIDToGIDMap), or whose CFF program is CID-keyed.
    collection: Option<CidFamily>,
}

/// A marked content sequence the walk met, in the order the interpreter meets it.
#[derive(Debug, Clone)]
struct Mark {
    /// The tag and MCID the interpreter reports for it, to match the two.
    tag: u64,
    mcid: Option<i32>,
    /// The MCID of the sequence in the file, kept when the walk rewrote it.
    own_mcid: Option<i32>,
    actual: Option<Rc<str>>,
    /// The switched-off layer this sequence opens, when the walk rewrote it to be drawn.
    layer: Option<usize>,
}

/// The glyph tables of embedded font programs, shared by every font dictionary that uses a
/// program, and charged to the copy budget.
#[derive(Default)]
struct Programs {
    /// Font dictionary to the program in use, `None` for a stand-in.
    of_font: HashMap<u128, Option<ObjectIdentifier>>,
    tables: HashMap<ObjectIdentifier, Rc<GlyphNames>>,
    collections: HashMap<Vec<u8>, Option<Rc<CMap>>>,
    /// The copy budget ran out; no more tables are read.
    spent: bool,
}

/// Reads pages one after the other, sharing hayro's caches and what is known of the fonts.
struct PageReader<'p, 'a> {
    origin: Origin<'p>,
    caps: Caps,
    stop: &'p dyn Fn() -> bool,
    settings: InterpreterSettings,
    cache: InterpreterCache<'a>,
    /// Font program bytes the cache holds; past the work bound it starts again empty.
    cache_bytes: u64,
    known: Known,
    programs: Programs,
    /// The device's glyph and text buffers, reused from page to page.
    glyph_buf: Vec<Gl>,
    text_buf: String,
    /// Bytes the kept pages may still take.
    kept_left: u64,
    /// Why the later pages are not read, once the kept pages took all they may.
    kept_spent: Option<String>,
    /// What quoting the text under annotations may still visit.
    quoting: Quoting,
}

impl<'p, 'a> PageReader<'p, 'a> {
    fn new(
        origin: Origin<'p>,
        caps: Caps,
        stop: &'p dyn Fn() -> bool,
        off: HashSet<ObjectIdentifier>,
    ) -> Self {
        PageReader {
            origin,
            caps,
            stop,
            settings: InterpreterSettings {
                // Annotation appearances are not page text: form values and notes come from
                // the object layer.
                render_annotations: false,
                ..InterpreterSettings::default()
            },
            cache: InterpreterCache::new(),
            cache_bytes: 0,
            known: Known {
                off,
                ..Known::default()
            },
            programs: Programs::default(),
            glyph_buf: Vec::new(),
            text_buf: String::new(),
            kept_left: caps.max_kept,
            kept_spent: None,
            quoting: Quoting::new(caps.max_items),
        }
    }

    fn page(
        &mut self,
        n: u32,
        page: &Page<'a>,
        links: &[Link],
        marked: &[(usize, &[[f32; 8]])],
    ) -> Read {
        let fail = |why: String, kind: Fail| Read::Failed(Failure { page: n, why, kind });
        if let Some(why) = &self.kept_spent {
            return fail(why.clone(), Fail::Limit);
        }
        if let Origin::Fallback(raw) = self.origin
            && let Some(why) = raw.unsafe_page(page)
        {
            return fail(why, Fail::Damaged);
        }
        let max_bytes = (self.caps.max_items as u64).saturating_mul(BYTES_PER_UNIT);
        let streams = content_streams(page.raw());
        let size: u64 = streams.iter().map(|s| stream_size(self.origin, s)).sum();
        if size > max_bytes {
            return fail(
                format!(
                    "its content unpacks to {size} bytes, more than the {max_bytes} bytes one \
                     page may take"
                ),
                Fail::Limit,
            );
        }
        let mut notes = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        for s in &streams {
            match s.decoded() {
                Ok(d) => {
                    if streams.len() == 1 {
                        data = d.into_owned();
                    } else {
                        data.extend_from_slice(&d);
                    }
                    // The streams of a page are one stream cut in pieces.
                    data.push(b' ');
                }
                Err(_) => notes.push(Note::Damaged(format!(
                    "a content stream of page {n} does not unpack; the text after it may \
                     be missing"
                ))),
            }
        }
        if self.cache_bytes > max_bytes {
            self.cache = InterpreterCache::new();
            self.cache_bytes = 0;
        }
        let (origin, caps) = (self.origin, self.caps);
        let known = &mut self.known;
        let walked = panic::catch_unwind(AssertUnwindSafe(|| {
            let mut walk = Walk::new(origin, caps, n, known);
            walk.charge(size / BYTES_PER_UNIT);
            let cut = walk.top(&data, page.resources());
            walk.finish(cut)
        }));
        let Ok(walked) = walked else {
            return fail("walking the content".into(), Fail::Internal);
        };
        let Walked {
            cut,
            marks,
            rewrites,
            layers,
            notes: walk_notes,
            unread_hidden,
            clip_in_forms,
            font_bytes,
        } = walked;
        self.cache_bytes = self.cache_bytes.saturating_add(font_bytes);
        let (placed, failed) = edit(&mut data, cut.as_ref().map(|(at, _)| *at), &rewrites);
        let mut in_part = cut.is_some();
        if let Some((_, why)) = &cut {
            if placed {
                notes.push(Note::Limit(format!(
                    "text of page {n} read only in part: {why}; the rest of the page is not \
                     read"
                )));
            } else {
                data.clear();
                notes.push(Note::Limit(format!("text of page {n} not read: {why}")));
            }
        }
        for i in failed {
            match rewrites.get(&i) {
                Some(Rewrite::Layer(k)) => notes.push(Note::Unsupported(format!(
                    "text in hidden layer {} on page {n} not read: its marked content could \
                     not be opened",
                    layers[*k]
                ))),
                _ => notes.push(Note::Unsupported(format!(
                    "a render mode of page {n} could not be changed; text set to clip may be \
                     missing"
                ))),
            }
        }
        notes.extend(walk_notes);
        for (name, count) in unread_hidden {
            let s = if count == 1 { "" } else { "s" };
            notes.push(Note::Unsupported(format!(
                "{count} text operator{s} in hidden layer {name} on page {n} not read: the \
                 interpreter does not run them"
            )));
        }
        if clip_in_forms > 0 {
            let s = if clip_in_forms == 1 { "" } else { "s" };
            notes.push(Note::Unsupported(format!(
                "{clip_in_forms} text operator{s} in render mode 7 (clip only) inside forms on \
                 page {n} not read"
            )));
        }
        let use_marks = marks.iter().any(|m| m.actual.is_some()) || !layers.is_empty();
        let crop = page.intersected_crop_box();
        let origin = Affine::translate((-crop.x0, -crop.y0));
        let bbox = kurbo::Rect::new(0.0, 0.0, crop.x1 - crop.x0, crop.y1 - crop.y0);
        let mut ctx = Context::new(
            origin,
            bbox,
            &self.cache,
            page.xref(),
            self.settings.clone(),
        );
        let mut dev = TextDevice {
            glyphs: std::mem::take(&mut self.glyph_buf),
            text: std::mem::take(&mut self.text_buf),
            actuals: Vec::new(),
            images: Vec::new(),
            images_code: Vec::new(),
            rules: Vec::new(),
            rules_cut: false,
            open: Vec::new(),
            marks: if use_marks { &marks } else { &[] },
            next_mark: 0,
            out_of_step: 0,
            layers_out_of_step: 0,
            layer_tags: marks
                .iter()
                .filter(|m| m.layer.is_some())
                .map(|m| m.tag)
                .collect(),
            fonts: &self.known.fonts,
            origin: self.origin,
            programs: &mut self.programs,
            stand_ins: HashSet::new(),
            tables_cut: false,
            stop: self.stop,
            calls: 0,
            cap: self.caps.max_glyphs.max(1),
            last: None,
            unmapped: 0,
            unmapped_visible: 0,
            mapped: 0,
        };
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            interpret(TypedIter::new(&data), page.resources(), &mut ctx, &mut dev);
        }));
        drop(ctx);
        if let Err(payload) = run {
            match payload.downcast_ref::<Abort>() {
                Some(Abort::Stop) => return Read::Stopped,
                Some(Abort::Cap) => {
                    in_part = true;
                    notes.push(Note::Limit(format!(
                        "text of page {n} read only in part: it draws more than {} glyphs; the \
                         rest of the page is not read",
                        dev.cap
                    )))
                }
                None => {
                    // Whatever the interpreter cached may be half built.
                    self.cache = InterpreterCache::new();
                    self.cache_bytes = 0;
                    return fail("interpreting the content".into(), Fail::Internal);
                }
            }
        }
        if !dev.stand_ins.is_empty() {
            let k = dev.stand_ins.len();
            let (s, them) = if k == 1 { ("", "its") } else { ("s", "their") };
            notes.push(Note::Damaged(format!(
                "{k} embedded font program{s} of page {n} could not be read; {them} glyphs \
                 without a ToUnicode mapping count as unmapped"
            )));
        }
        if dev.layers_out_of_step > 0 {
            let k = dev.layers_out_of_step;
            let (s, did, them) = if k == 1 {
                ("", "does", "its")
            } else {
                ("s", "do", "their")
            };
            notes.push(Note::Unsupported(format!(
                "{k} sequence{s} of hidden layers on page {n} {did} not match the content \
                 walk; {them} text may be in the body"
            )));
        }
        if dev.out_of_step > 0 {
            let k = dev.out_of_step;
            let (s, it) = if k == 1 {
                ("", "the sequence does")
            } else {
                ("s", "the sequences do")
            };
            notes.push(Note::Unsupported(format!(
                "the ActualText of {k} marked content sequence{s} on page {n} is not used: \
                 {it} not match the content walk"
            )));
        }
        if dev.tables_cut {
            notes.push(Note::Limit(format!(
                "glyph tables of embedded fonts on page {n} not read: they take more than the \
                 object limit allows; their glyphs count as unmapped"
            )));
        }
        let texts = Texts {
            text: &dev.text,
            actuals: &dev.actuals,
        };
        // The quotes are taken from the glyphs while they are at hand; no glyph box is kept.
        let kept_before = self.kept_left;
        let quotes = if marked.is_empty() {
            Vec::new()
        } else {
            crate::quote::quote_page(
                n,
                &dev.glyphs,
                &texts,
                marked,
                (crop.x0 as f32, crop.y0 as f32),
                &mut self.quoting,
                &mut self.kept_left,
                self.stop,
            )
        };
        let mut items = lay_out(n, &dev.glyphs, &texts, links, crop.x0, crop.y0, &layers);
        items.quotes = quotes;
        items.images = std::mem::take(&mut dev.images);
        items.images_code = std::mem::take(&mut dev.images_code);
        items.rules = std::mem::take(&mut dev.rules);
        items.origin = (crop.x0 as f32, crop.y0 as f32);
        if dev.rules_cut {
            notes.push(Note::Limit(format!(
                "drawn lines of page {n} past the first {} are not used to find tables: the \
                 page draws more lines than it may keep",
                dev.cap
            )));
        }
        items.mapped = dev.mapped;
        items.unmapped = dev.unmapped;
        items.unmapped_visible = dev.unmapped_visible;
        // The glyph buffers are kept for the next page: built again for each page, their
        // memory would not be used again.
        let (mut glyphs, mut text) = (
            std::mem::take(&mut dev.glyphs),
            std::mem::take(&mut dev.text),
        );
        drop(dev);
        glyphs.clear();
        text.clear();
        self.glyph_buf = glyphs;
        self.text_buf = text;
        items.in_part = in_part;
        notes.append(&mut items.notes);
        items.notes = notes;
        // What every page keeps until layout counts against one bound for the file. It is
        // measured before anything is copied.
        let cost = items.kept_bytes();
        if cost > self.kept_left {
            // The page is not kept, so neither are its quotes.
            self.kept_left = kept_before;
            let why = format!(
                "the text of the file takes more than the {} bytes it may keep until it is laid \
                 out",
                self.caps.max_kept
            );
            self.kept_spent = Some(why.clone());
            return fail(why, Fail::Limit);
        }
        self.kept_left -= cost;
        items.kept = cost;
        // The page's runs were built among many short-lived allocations; copied now (within
        // the bound just charged), the kept text sits together and the memory of the build
        // can be used again.
        items.compact();
        Read::Page(Box::new(items))
    }
}

/// The content streams of a page, in order.
fn content_streams<'a>(page: &Dict<'a>) -> Vec<Stream<'a>> {
    if let Some(s) = page.get::<Stream<'a>>(b"Contents") {
        return vec![s];
    }
    page.get::<Array<'a>>(b"Contents")
        .map(|a| a.iter::<Stream<'a>>().collect())
        .unwrap_or_default()
}

/// A change [`edit`] makes to one page-level operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rewrite {
    /// The operand of a `Tr` becomes this render mode: the clipping modes 4 to 7 are read as
    /// 0 to 3, so the interpreter builds no glyph outlines for a clip and draws mode 7 text.
    Mode(u8),
    /// A `BDC` that switches optional content off becomes `/Rz<k> BMC`: the interpreter draws
    /// the text in it, and the device knows it belongs to hidden layer `k`.
    Layer(usize),
}

/// The tag a switched-off `BDC` of the page gets.
fn layer_tag(k: usize) -> String {
    format!("Rz{k}")
}

/// Edits the page content in place: cuts it before operator `cut` and applies `rewrites`
/// (keyed by operator index). Returns whether the cut could be placed, and the rewrites that
/// did not fit.
fn edit(
    data: &mut Vec<u8>,
    cut: Option<usize>,
    rewrites: &HashMap<usize, Rewrite>,
) -> (bool, Vec<usize>) {
    let base = data.as_ptr() as usize;
    let len = data.len();
    let mut cut_at = None;
    let mut fixes: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut failed = Vec::new();
    {
        let mut ops = UntypedIter::new(data);
        let mut prev_end = 0usize;
        let mut i = 0usize;
        while let Some(op) = ops.next() {
            if cut == Some(i) {
                cut_at = Some(prev_end);
                break;
            }
            let start = (op.operator.as_ptr() as usize).wrapping_sub(base);
            if start > len {
                // The operator was copied out of the data; no offset for it.
                if rewrites.contains_key(&i) {
                    failed.push(i);
                }
                i += 1;
                continue;
            }
            let end = start + op.operator.len();
            match rewrites.get(&i) {
                Some(Rewrite::Mode(m)) => {
                    let region = &data[prev_end..start];
                    match region.iter().position(|b| b.is_ascii_digit() || *b == b'+') {
                        Some(at) => {
                            let mut new = vec![b' '; region.len()];
                            new[at] = b'0' + m;
                            fixes.push((prev_end, new));
                        }
                        None => failed.push(i),
                    }
                }
                Some(Rewrite::Layer(k)) => {
                    let new = format!("/{} BMC", layer_tag(*k));
                    if new.len() < end - prev_end {
                        let mut bytes = vec![b' '; end - prev_end];
                        bytes[1..=new.len()].copy_from_slice(new.as_bytes());
                        fixes.push((prev_end, bytes));
                    } else {
                        failed.push(i);
                    }
                }
                None => {}
            }
            prev_end = end;
            i += 1;
        }
    }
    for (at, bytes) in fixes {
        data[at..at + bytes.len()].copy_from_slice(&bytes);
    }
    let placed = match (cut, cut_at) {
        (Some(_), Some(at)) => {
            data.truncate(at);
            true
        }
        (Some(_), None) => false,
        _ => true,
    };
    (placed, failed)
}

/// What the walk of a page gives.
struct Walked {
    cut: Option<(usize, String)>,
    marks: Vec<Mark>,
    rewrites: HashMap<usize, Rewrite>,
    layers: Vec<String>,
    notes: Vec<Note>,
    unread_hidden: Vec<(String, u64)>,
    clip_in_forms: u64,
    font_bytes: u64,
}

/// What the walk keeps for the whole file: fonts, the ActualText of named properties and
/// the optional content groups that are off.
#[derive(Default)]
struct Known {
    fonts: HashMap<u128, FontInfo>,
    properties: HashMap<ObjectIdentifier, Option<Rc<str>>>,
    off: HashSet<ObjectIdentifier>,
}

/// The parts of the graphics state the walk follows.
#[derive(Debug, Clone, Copy, Default)]
struct Gs {
    mode: i64,
    type3: bool,
}

/// Glyphs of Type3 fonts and glyphs in a clipping render mode inside forms cost the
/// interpreter about this many plain glyphs each.
const HEAVY_GLYPH: u64 = 8;

/// The walk of a page's content operators. It follows forms the way the interpreter runs
/// them, with the same optional content visibility, and stops once the page's work or glyphs
/// pass their bounds.
struct Walk<'w> {
    origin: Origin<'w>,
    caps: Caps,
    page: u32,
    known: &'w mut Known,
    work: usize,
    glyphs: u64,
    /// Marked content sequences in the order the interpreter meets them.
    marks: Vec<Mark>,
    rewrites: HashMap<usize, Rewrite>,
    /// Names of the switched-off layers met, by index.
    layers: Vec<String>,
    notes: Vec<Note>,
    /// Text operators the interpreter does not draw because their layer is off, by layer.
    unread_hidden: Vec<(String, u64)>,
    /// Text operators in render mode 7 inside forms, which the interpreter does not draw.
    clip_in_forms: u64,
    /// Fonts already charged on this page.
    charged: HashSet<u128>,
    font_bytes: u64,
    /// Forms being walked: the unpacked bytes they hold.
    held: u64,
    depth: usize,
    gs: Vec<Gs>,
    /// Whether the interpreter draws what is shown here.
    visible: Vec<bool>,
    /// The switched-off layer the content stands in.
    layer: Vec<Option<usize>>,
    /// Forms the interpreter does not run are walked to count their text only; their marked
    /// content is not met.
    skipping: usize,
    /// Why the walk stopped.
    over: Option<String>,
}

impl<'w> Walk<'w> {
    fn new(origin: Origin<'w>, caps: Caps, page: u32, known: &'w mut Known) -> Self {
        Walk {
            origin,
            caps,
            page,
            known,
            work: 0,
            glyphs: 0,
            marks: Vec::new(),
            rewrites: HashMap::new(),
            layers: Vec::new(),
            notes: Vec::new(),
            unread_hidden: Vec::new(),
            clip_in_forms: 0,
            charged: HashSet::new(),
            font_bytes: 0,
            held: 0,
            depth: 0,
            gs: vec![Gs::default()],
            visible: Vec::new(),
            layer: Vec::new(),
            skipping: 0,
            over: None,
        }
    }

    fn finish(self, cut: Option<(usize, String)>) -> Walked {
        Walked {
            cut,
            marks: self.marks,
            rewrites: self.rewrites,
            layers: self.layers,
            notes: self.notes,
            unread_hidden: self.unread_hidden,
            clip_in_forms: self.clip_in_forms,
            font_bytes: self.font_bytes,
        }
    }

    /// Adds work; false once the page is over its bound.
    fn charge(&mut self, units: u64) -> bool {
        self.work = self
            .work
            .saturating_add(usize::try_from(units).unwrap_or(usize::MAX));
        if self.work > self.caps.max_items && self.over.is_none() {
            self.over = Some(format!(
                "it takes more than {} units of work (content operators, shown text and \
                 unpacked bytes)",
                self.caps.max_items
            ));
        }
        self.over.is_none()
    }

    /// Charges a text show of `bytes` bytes: its glyphs when the interpreter draws them, a
    /// count of unread text when its layer is off.
    fn show(&mut self, bytes: usize, top: bool) -> bool {
        let bytes = bytes as u64;
        if !self.charge(bytes) {
            return false;
        }
        if !self.is_visible() {
            let name = self
                .layer
                .iter()
                .rev()
                .find_map(|l| *l)
                .and_then(|k| self.layers.get(k).cloned())
                .unwrap_or_else(|| "without a name".to_string());
            match self.unread_hidden.iter_mut().find(|(n, _)| *n == name) {
                Some((_, count)) => *count += 1,
                None => self.unread_hidden.push((name, 1)),
            }
            return true;
        }
        let gs = self.gs.last().copied().unwrap_or_default();
        if !top && gs.mode == 7 {
            self.clip_in_forms += 1;
        }
        let heavy = gs.type3 || (!top && (4..=7).contains(&gs.mode));
        let cost = if heavy { bytes * HEAVY_GLYPH } else { bytes };
        self.glyphs = self.glyphs.saturating_add(cost);
        if self.glyphs > self.caps.max_glyphs as u64 && self.over.is_none() {
            self.over = Some(format!(
                "it shows more than {} glyphs (glyphs of Type3 fonts and glyphs that clip \
                 count {HEAVY_GLYPH} times)",
                self.caps.max_glyphs
            ));
        }
        self.over.is_none()
    }

    fn note(&mut self, note: Note) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn size(&self, s: &Stream<'_>) -> u64 {
        stream_size(self.origin, s)
    }

    fn is_visible(&self) -> bool {
        self.visible.last().copied().unwrap_or(true)
    }

    /// Whether optional content `d` (a group, or a membership dictionary) is on, the way the
    /// interpreter decides it from the default configuration.
    fn oc_on(&self, d: &Dict<'_>, id: Option<ObjectIdentifier>) -> bool {
        if d.get::<Name<'_>>(b"Type").as_deref() != Some(b"OCMD") {
            return id.is_none_or(|id| !self.known.off.contains(&id));
        }
        let mut groups: Vec<ObjectIdentifier> = Vec::new();
        if let Some(a) = d.get::<Array<'_>>(b"OCGs") {
            groups.extend(
                a.raw_iter()
                    .filter_map(|i| i.as_obj_ref())
                    .map(ObjectIdentifier::from),
            );
        } else if let Some(r) = d.get_ref(b"OCGs") {
            groups.push(r.into());
        }
        if groups.is_empty() {
            return true;
        }
        let off = |g: &ObjectIdentifier| self.known.off.contains(g);
        match d.get::<Name<'_>>(b"P").as_deref() {
            Some(b"AllOn") => groups.iter().all(|g| !off(g)),
            Some(b"AnyOff") => groups.iter().any(off),
            Some(b"AllOff") => groups.iter().all(off),
            _ => groups.iter().any(|g| !off(g)),
        }
    }

    /// The name of a layer: the group's `/Name`, or the names of a membership dictionary's
    /// groups.
    fn oc_name(&self, d: &Dict<'_>) -> String {
        let name = |g: &Dict<'_>| {
            g.get::<hayro_syntax::object::String<'_>>(b"Name")
                .map(|s| pdfparts::objects::text_string(s.as_bytes()))
        };
        if d.get::<Name<'_>>(b"Type").as_deref() == Some(b"OCMD") {
            let names: Vec<String> = match d.get::<Array<'_>>(b"OCGs") {
                Some(a) => a.iter::<Dict<'_>>().filter_map(|g| name(&g)).collect(),
                None => d
                    .get::<Dict<'_>>(b"OCGs")
                    .and_then(|g| name(&g))
                    .into_iter()
                    .collect(),
            };
            if !names.is_empty() {
                return names.join(", ");
            }
        }
        name(d).unwrap_or_else(|| "without a name".to_string())
    }

    /// The layer index for a switched-off group.
    fn layer_index(&mut self, name: String) -> usize {
        match self.layers.iter().position(|n| *n == name) {
            Some(k) => k,
            None => {
                self.layers.push(name);
                self.layers.len() - 1
            }
        }
    }

    /// Walks the page's own content. Returns the index of the first operator that does not
    /// fit, with the reason.
    fn top(&mut self, data: &[u8], resources: &Resources<'_>) -> Option<(usize, String)> {
        let mut ops = TypedIter::new(data);
        let mut i = 0usize;
        while let Some(op) = ops.next() {
            let marks = self.marks.len();
            if !self.op(&op, resources, Some(i)) {
                // The operator that broke the bound is not run, and neither is what it marks.
                self.marks.truncate(marks);
                return Some((i, self.over.clone().unwrap_or_default()));
            }
            i += 1;
        }
        if stops_early(data, i) {
            let page = self.page;
            self.note(Note::Damaged(format!(
                "the content of page {page} stops at a broken operator; the rest of the page \
                 is not read"
            )));
        }
        None
    }

    /// Walks the content of a form.
    fn form(&mut self, data: &[u8], resources: &Resources<'_>) -> bool {
        let mut ops = TypedIter::new(data);
        let mut i = 0usize;
        while let Some(op) = ops.next() {
            if !self.op(&op, resources, None) {
                return false;
            }
            i += 1;
        }
        if self.skipping == 0 && stops_early(data, i) {
            let page = self.page;
            self.note(Note::Damaged(format!(
                "a form on page {page} stops at a broken operator; the rest of the form is \
                 not read"
            )));
        }
        true
    }

    /// One operator; `top` is its index on the page, `None` inside a form.
    fn op(
        &mut self,
        op: &TypedInstruction<'_, '_>,
        res: &Resources<'_>,
        top: Option<usize>,
    ) -> bool {
        if !self.charge(1) {
            return false;
        }
        let page_level = top.is_some();
        match op {
            TypedInstruction::ShowText(s) => self.show(s.0.as_bytes().len(), page_level),
            TypedInstruction::NextLineAndShowText(s) => self.show(s.0.as_bytes().len(), page_level),
            TypedInstruction::ShowTextWithParameters(s) => {
                self.show(s.2.as_bytes().len(), page_level)
            }
            TypedInstruction::ShowTexts(a) => {
                let bytes: usize =
                    a.0.iter::<Object<'_>>()
                        .map(|o| match o {
                            Object::String(s) => s.as_bytes().len(),
                            _ => 1,
                        })
                        .sum();
                self.show(bytes, page_level)
            }
            TypedInstruction::SaveState(_) => {
                let gs = self.gs.last().copied().unwrap_or_default();
                self.gs.push(gs);
                true
            }
            TypedInstruction::RestoreState(_) => {
                if self.gs.len() > 1 {
                    self.gs.pop();
                }
                true
            }
            TypedInstruction::TextRenderingMode(m) => {
                let mut mode = m.0.as_i64();
                if let (Some(i), 4..=7) = (top, mode) {
                    mode -= 4;
                    self.rewrites.insert(i, Rewrite::Mode(mode as u8));
                }
                if let Some(gs) = self.gs.last_mut() {
                    gs.mode = mode;
                }
                true
            }
            TypedInstruction::TextFont(f) => match res.get_font(f.0) {
                Some(font) => self.set_font(&font),
                None => true,
            },
            TypedInstruction::SetGraphicsState(g) => {
                // An ExtGState may set the font too: `/Font [font size]`.
                let font = res
                    .get_ext_g_state(g.0)
                    .and_then(|d| d.get::<Array<'_>>(b"Font"))
                    .and_then(|a| a.iter::<Dict<'_>>().next());
                match font {
                    Some(font) => self.set_font(&font),
                    None => true,
                }
            }
            TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                self.bdc(bdc.0, bdc.1, res, top)
            }
            TypedInstruction::BeginMarkedContent(bmc) => {
                let outer = self.is_visible();
                self.visible.push(outer);
                let layer = self.layer.last().copied().flatten();
                self.layer.push(layer);
                if self.skipping == 0 {
                    self.marks.push(Mark {
                        tag: tag_hash(bmc.0),
                        mcid: None,
                        own_mcid: None,
                        actual: None,
                        layer: None,
                    });
                }
                true
            }
            TypedInstruction::EndMarkedContent(_) => {
                self.visible.pop();
                self.layer.pop();
                true
            }
            TypedInstruction::XObject(x) => {
                let Some(s) = res.get_x_object(x.0) else {
                    return true;
                };
                let dict = s.dict().clone();
                match dict.get::<Name<'_>>(b"Subtype").as_deref() {
                    Some(b"Form") => self.run_form(&s, &dict, res),
                    Some(b"Image") => {
                        let cs = dict.get::<Object<'_>>(b"ColorSpace");
                        let bytes = cs.map_or(0, |o| self.streams_under(&o, 3));
                        self.charge(bytes / BYTES_PER_UNIT)
                    }
                    _ => true,
                }
            }
            TypedInstruction::InlineImage(i) => {
                self.charge(i.0.raw_data().len() as u64 / BYTES_PER_UNIT)
            }
            TypedInstruction::Shading(s) => {
                let bytes = res
                    .get_shading(s.0)
                    .map_or(0, |o| self.streams_under(&o, 3));
                self.charge(bytes / BYTES_PER_UNIT)
            }
            TypedInstruction::NonStrokeColorNamed(c) => {
                let bytes =
                    c.1.and_then(|n| res.get_pattern(n))
                        .map_or(0, |o| self.streams_under(&o, 3));
                self.charge(bytes / BYTES_PER_UNIT)
            }
            TypedInstruction::StrokeColorNamed(c) => {
                let bytes =
                    c.1.and_then(|n| res.get_pattern(n))
                        .map_or(0, |o| self.streams_under(&o, 3));
                self.charge(bytes / BYTES_PER_UNIT)
            }
            TypedInstruction::ColorSpaceStroke(c) => self.charge_color_space(c.0, res),
            TypedInstruction::ColorSpaceNonStroke(c) => self.charge_color_space(c.0, res),
            _ => true,
        }
    }

    /// A marked content sequence with properties: its optional content as the interpreter
    /// reads it, its MCID and its ActualText.
    fn bdc(
        &mut self,
        tag: &Name<'_>,
        props: &Object<'_>,
        res: &Resources<'_>,
        top: Option<usize>,
    ) -> bool {
        let outer = self.is_visible();
        let oc = props
            .clone()
            .into_name()
            .and_then(|n| {
                let r = res.properties.get_ref(n.as_ref())?;
                let d = res.properties.get::<Dict<'_>>(n).unwrap_or_default();
                Some((d, ObjectIdentifier::from(r)))
            })
            .or_else(|| {
                let (d, _) = dict_or_stream(props)?;
                let r = d.get_ref(b"OC")?;
                let oc = d.get::<Dict<'_>>(b"OC").unwrap_or_default();
                Some((oc, ObjectIdentifier::from(r)))
            });
        let mut layer = self.layer.last().copied().flatten();
        let mut visible = outer;
        let mut tag_bytes: Vec<u8> = tag.as_ref().to_vec();
        if let Some((d, id)) = oc
            && !self.oc_on(&d, Some(id))
        {
            let k = self.layer_index(self.oc_name(&d));
            layer = Some(k);
            match top {
                // The page's own content is changed so the text is drawn and read as hidden.
                Some(i) if outer && self.skipping == 0 => {
                    self.rewrites.insert(i, Rewrite::Layer(k));
                    tag_bytes = layer_tag(k).into_bytes();
                }
                _ => visible = false,
            }
        }
        self.visible.push(visible);
        self.layer.push(layer);
        if self.skipping > 0 {
            return true;
        }
        let rewritten = top.is_some_and(|i| self.rewrites.contains_key(&i));
        // A rewritten sequence reaches the interpreter as `BMC`, without its MCID; the walk
        // keeps the MCID and the ActualText for it.
        let own_mcid = dict_or_stream(props).and_then(|(d, _)| d.get::<i32>(b"MCID"));
        let actual = self.actual_text(props, res);
        if let Some(a) = &actual {
            // The device copies the text once for the sequence.
            if !self.charge(a.len() as u64) {
                return false;
            }
            if !self.origin.charge_copy(a.len() as u64) {
                self.over = Some(
                    "its ActualText copies take more than the object limit allows".to_string(),
                );
                return false;
            }
        }
        self.marks.push(Mark {
            tag: tag_hash(&tag_bytes),
            mcid: if rewritten { None } else { own_mcid },
            own_mcid,
            actual,
            layer: if rewritten { layer } else { None },
        });
        true
    }

    /// The ActualText of a sequence's properties: inline, or a named entry of `/Properties`,
    /// read once for the file.
    fn actual_text(&mut self, props: &Object<'_>, res: &Resources<'_>) -> Option<Rc<str>> {
        let read = |d: &Dict<'_>| {
            d.get::<hayro_syntax::object::String<'_>>(b"ActualText")
                .map(|s| Rc::from(pdfparts::objects::text_string(s.as_bytes())))
        };
        if let Some((d, _)) = dict_or_stream(props) {
            return read(d);
        }
        let name = props.clone().into_name()?;
        let id: ObjectIdentifier = res.properties.get_ref(name.as_ref())?.into();
        if let Some(found) = self.known.properties.get(&id) {
            return found.clone();
        }
        let found = res.properties.get::<Dict<'_>>(name).and_then(|d| read(&d));
        self.known.properties.insert(id, found.clone());
        found
    }

    /// A font becomes current (through `Tf` or an ExtGState): its kind decides what its
    /// glyphs cost, and its streams are charged once per page.
    fn set_font(&mut self, font: &Dict<'_>) -> bool {
        let key = font.cache_key();
        if !self.known.fonts.contains_key(&key) {
            let mut info = font_info(font);
            info.program = font_program(font).map(|s| (s.obj_id(), self.size(&s)));
            self.known.fonts.insert(key, info);
        }
        let type3 = font.get::<Name<'_>>(b"Subtype").as_deref() == Some(b"Type3");
        if let Some(gs) = self.gs.last_mut() {
            gs.type3 = type3;
        }
        if self.charged.insert(key) {
            let bytes = self.font_streams(font);
            self.font_bytes = self.font_bytes.saturating_add(bytes);
            return self.charge(bytes / BYTES_PER_UNIT);
        }
        true
    }

    /// A color space the interpreter builds may unpack an ICC profile or a lookup table.
    fn charge_color_space(&mut self, name: &Name<'_>, res: &Resources<'_>) -> bool {
        let bytes = res
            .get_color_space(name)
            .map_or(0, |o| self.streams_under(&o, 3));
        self.charge(bytes / BYTES_PER_UNIT)
    }

    /// Walks a form the way the interpreter runs it: its own resources, else the caller's,
    /// and not at all when its optional content is off, its bounding box is missing or it is
    /// nested too deep. Each of these is said.
    fn run_form(&mut self, s: &Stream<'_>, dict: &Dict<'_>, res: &Resources<'_>) -> bool {
        let page = self.page;
        if self.skipping == 0 && dict.get::<[f32; 4]>(b"BBox").is_none() {
            self.note(Note::Damaged(format!(
                "a form on page {page} has no bounding box and is not drawn; its text is not \
                 read"
            )));
            return true;
        }
        if self.depth >= FORM_DEPTH {
            if self.skipping == 0 {
                self.note(Note::Limit(format!(
                    "forms nested deeper than {FORM_DEPTH} on page {page} are not drawn; \
                     their text is not read"
                )));
            }
            return true;
        }
        let size = self.size(s);
        if !self.charge(size / BYTES_PER_UNIT) {
            return false;
        }
        let max_bytes = (self.caps.max_items as u64).saturating_mul(BYTES_PER_UNIT);
        if self.held.saturating_add(size) > max_bytes {
            self.over = Some(format!(
                "its nested forms hold more than {max_bytes} unpacked bytes at once"
            ));
            return false;
        }
        let Ok(data) = s.decoded() else {
            if self.skipping == 0 {
                self.note(Note::Damaged(format!(
                    "a form on page {page} does not unpack and is not drawn; its text is not \
                     read"
                )));
            }
            return true;
        };
        // The interpreter does not run a form in hidden content or with its own optional
        // content off; it is walked only to count the text in it.
        let mut hidden = !self.is_visible();
        let mut layer = self.layer.last().copied().flatten();
        if let Some(oc) = dict.get::<Dict<'_>>(b"OC") {
            let id = dict.get_ref(b"OC").map(ObjectIdentifier::from);
            let on = match id {
                Some(id) => self.oc_on(&oc, Some(id)),
                None => self.oc_on(&oc, None),
            };
            if !on {
                hidden = true;
                layer = Some(self.layer_index(self.oc_name(&oc)));
            }
        }
        let own = dict.get::<Dict<'_>>(b"Resources").map(Resources::new);
        self.held += size;
        self.depth += 1;
        let gs = self.gs.last().copied().unwrap_or_default();
        self.gs.push(gs);
        self.visible.push(!hidden);
        self.layer.push(layer);
        if hidden {
            self.skipping += 1;
        }
        let (visible_depth, layer_depth) = (self.visible.len(), self.layer.len());
        let ok = self.form(&data, own.as_ref().unwrap_or(res));
        // A form's unbalanced marked content ends with the form.
        self.visible.truncate(visible_depth - 1);
        self.layer.truncate(layer_depth - 1);
        if hidden {
            self.skipping -= 1;
        }
        self.gs.pop();
        self.depth -= 1;
        self.held -= size;
        ok
    }

    /// Unpacked bytes of the streams a font reads when it is loaded: the program, ToUnicode
    /// and an encoding CMap.
    fn font_streams(&self, font: &Dict<'_>) -> u64 {
        let mut total = 0u64;
        let mut add = |d: &Dict<'_>, key: &[u8]| {
            if let Some(s) = d.get::<Stream<'_>>(key) {
                total = total.saturating_add(self.size(&s));
            }
        };
        add(font, b"ToUnicode");
        add(font, b"Encoding");
        let mut descriptors = vec![font.get::<Dict<'_>>(b"FontDescriptor")];
        if let Some(d) = font
            .get::<Array<'_>>(b"DescendantFonts")
            .and_then(|a| a.iter::<Dict<'_>>().next())
        {
            descriptors.push(d.get::<Dict<'_>>(b"FontDescriptor"));
        }
        for desc in descriptors.into_iter().flatten() {
            for key in [&b"FontFile"[..], b"FontFile2", b"FontFile3"] {
                add(&desc, key);
            }
        }
        total
    }

    /// Unpacked bytes of the streams within `depth` levels of `obj` (a shading's functions, a
    /// pattern's shading, an ICC profile). At most 64 objects are looked at.
    fn streams_under(&self, obj: &Object<'_>, depth: usize) -> u64 {
        let mut total = 0u64;
        let mut todo = vec![(obj.clone(), depth)];
        let mut seen = 0;
        while let Some((o, d)) = todo.pop() {
            seen += 1;
            if seen > 64 {
                break;
            }
            let children: Vec<Object<'_>> = match &o {
                Object::Stream(s) => {
                    total = total.saturating_add(self.size(s));
                    let dict = s.dict();
                    dict.keys()
                        .filter(|k| k.as_ref() != b"Parent")
                        .filter_map(|k| dict.get::<Object<'_>>(k))
                        .collect()
                }
                Object::Dict(dict) => dict
                    .keys()
                    .filter(|k| k.as_ref() != b"Parent")
                    .filter_map(|k| dict.get::<Object<'_>>(k))
                    .collect(),
                Object::Array(a) => a.iter::<Object<'_>>().take(64).collect(),
                _ => Vec::new(),
            };
            if d > 0 {
                todo.extend(children.into_iter().map(|c| (c, d - 1)));
            }
        }
        total
    }
}

/// Whether the content stops before its end: the typed operators end after `typed`, but
/// more operators follow (a path operator with operands the interpreter cannot read ends the
/// stream for it).
fn stops_early(data: &[u8], typed: usize) -> bool {
    let mut ops = UntypedIter::new(data);
    for _ in 0..typed {
        if ops.next().is_none() {
            return false;
        }
    }
    ops.next().is_some()
}

/// The unpacked size of a stream, as the guard measured it: pdfparts' guard for the
/// canonical bytes, the fallback's for the file itself.
fn stream_size(origin: Origin<'_>, s: &Stream<'_>) -> u64 {
    let pdf = match origin {
        Origin::Canonical(pdf) => pdf,
        Origin::Fallback(raw) => return raw.stream_size(s),
    };
    let id = s.obj_id();
    match (u32::try_from(id.obj_number), u16::try_from(id.gen_number)) {
        (Ok(n), Ok(g)) => pdf.unpacked_size(n, g),
        _ => None,
    }
    .unwrap_or_else(|| s.raw_data().len() as u64)
}

/// The embedded program of a font: its own descriptor's, or its descendant's for a Type0
/// font.
fn font_program<'a>(font: &Dict<'a>) -> Option<Stream<'a>> {
    let desc = font.get::<Dict<'a>>(b"FontDescriptor").or_else(|| {
        font.get::<Array<'a>>(b"DescendantFonts")
            .and_then(|a| a.iter::<Dict<'a>>().next())
            .and_then(|d| d.get::<Dict<'a>>(b"FontDescriptor"))
    })?;
    [&b"FontFile2"[..], b"FontFile3", b"FontFile"]
        .iter()
        .find_map(|k| desc.get::<Stream<'a>>(*k))
}

fn tag_hash(tag: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    tag.hash(&mut h);
    h.finish()
}

/// Style and kind of a font from its dictionary: the descriptor's flags, weight and angle and
/// the font's name.
fn font_info(font: &Dict<'_>) -> FontInfo {
    let subtype = font.get::<Name<'_>>(b"Subtype");
    let type0 = subtype.as_deref() == Some(b"Type0");
    let descendant = font
        .get::<Array<'_>>(b"DescendantFonts")
        .and_then(|a| a.iter::<Dict<'_>>().next());
    let desc = if type0 {
        descendant
            .as_ref()
            .and_then(|d| d.get::<Dict<'_>>(b"FontDescriptor"))
    } else {
        font.get::<Dict<'_>>(b"FontDescriptor")
    };
    let name = font
        .get::<Name<'_>>(b"BaseFont")
        .map(|n| String::from_utf8_lossy(&n).to_lowercase())
        .unwrap_or_default();
    // A subset prefix (six capitals and a plus) says nothing about the style.
    let name = match name.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 => rest.to_string(),
        _ => name,
    };
    let flags = desc
        .as_ref()
        .and_then(|d| d.get::<i64>(b"Flags"))
        .unwrap_or(0);
    let weight = desc
        .as_ref()
        .and_then(|d| d.get::<f32>(b"FontWeight"))
        .unwrap_or(0.0);
    let angle = desc
        .as_ref()
        .and_then(|d| d.get::<f32>(b"ItalicAngle"))
        .unwrap_or(0.0);
    let embedded = desc.as_ref().is_some_and(|d| {
        d.contains_key(b"FontFile") || d.contains_key(b"FontFile2") || d.contains_key(b"FontFile3")
    });
    let bold = flags & (1 << 18) != 0
        || weight >= 600.0
        || ["bold", "black", "heavy", "semibold", "demi"]
            .iter()
            .any(|w| name.contains(w));
    let italic = flags & (1 << 6) != 0
        || angle != 0.0
        || ["italic", "oblique"].iter().any(|w| name.contains(w));
    let vertical = type0
        && match font.get::<Object<'_>>(b"Encoding") {
            Some(Object::Name(n)) => n.ends_with(b"-V"),
            Some(Object::Stream(s)) => s.dict().get::<i64>(b"WMode") == Some(1),
            _ => false,
        };
    let collection = descendant.as_ref().and_then(|d| {
        let info = d.get::<Dict<'_>>(b"CIDSystemInfo")?;
        let registry = info.get::<hayro_syntax::object::String<'_>>(b"Registry")?;
        let ordering = info.get::<hayro_syntax::object::String<'_>>(b"Ordering")?;
        let family = CidFamily::from_registry_ordering(registry.as_bytes(), ordering.as_bytes());
        family.ucs2_cmap()?;
        let kind = d.get::<Name<'_>>(b"Subtype");
        let identity = match d.get::<Object<'_>>(b"CIDToGIDMap") {
            None => true,
            Some(Object::Name(n)) => n.as_ref() == b"Identity",
            _ => false,
        };
        match kind.as_deref() {
            // The glyph id of a TrueType CID font is its CID through the identity map; a
            // CID-keyed CFF program says the CID of each glyph itself.
            Some(b"CIDFontType2") if identity => Some(family),
            Some(b"CIDFontType0") => Some(family),
            _ => None,
        }
    });
    FontInfo {
        bold,
        italic,
        vertical,
        embedded,
        program: None,
        collection,
    }
}

/// The characters of the glyphs of one embedded font program, from its own tables.
#[derive(Debug, Default)]
struct GlyphNames {
    by_gid: HashMap<u32, String>,
}

impl GlyphNames {
    /// About the memory the table takes.
    fn bytes(&self) -> u64 {
        self.by_gid.values().map(|t| 56 + t.len() as u64).sum()
    }
}

impl GlyphNames {
    /// Reads the reverse of the font's cmap, its glyph names and, for a CID font of a known
    /// collection, the collection's CID to Unicode table.
    fn read(data: &[u8], collection: Option<&CMap>) -> GlyphNames {
        let mut by_gid: HashMap<u32, String> = HashMap::new();
        if let Ok(font) = FontRef::new(data) {
            let mut best: HashMap<u32, u32> = HashMap::new();
            for (cp, gid) in font.charmap().mappings() {
                if is_private(cp) {
                    continue;
                }
                let e = best.entry(gid.to_u32()).or_insert(cp);
                *e = (*e).min(cp);
            }
            for (gid, cp) in best {
                if let Some(c) = char::from_u32(cp) {
                    by_gid.insert(gid, c.to_string());
                }
            }
            if let Some(cmap) = collection {
                // A TrueType CID font of the collection: glyph id and CID are the same.
                let count = font.maxp().map(|m| u32::from(m.num_glyphs())).unwrap_or(0);
                for gid in 0..count {
                    if !by_gid.contains_key(&gid)
                        && let Some(t) = cmap.lookup_bf_string(gid).and_then(clean_bf)
                    {
                        by_gid.insert(gid, t);
                    }
                }
            }
            for (gid, name) in font.glyph_names().iter() {
                if name.is_synthesized() || by_gid.contains_key(&gid.to_u32()) {
                    continue;
                }
                if let Some(t) = name_text(name.as_str()) {
                    by_gid.insert(gid.to_u32(), t);
                }
            }
        } else if let Ok(cff) = CffFontRef::new_cff(data, 0, None) {
            let cid = cff.is_cid();
            if let Some(charset) = cff.charset() {
                for (gid, sid) in charset.iter() {
                    let text = if cid {
                        collection.and_then(|c| {
                            c.lookup_bf_string(u32::from(sid.to_u16()))
                                .and_then(clean_bf)
                        })
                    } else {
                        cff.string(sid)
                            .and_then(|n| std::str::from_utf8(n).ok())
                            .and_then(name_text)
                    };
                    if let Some(t) = text {
                        by_gid.insert(gid.to_u32(), t);
                    }
                }
            }
        }
        GlyphNames { by_gid }
    }
}

/// A Unicode private use code point: no meaning outside the font that uses it.
fn is_private(cp: u32) -> bool {
    (0xE000..=0xF8FF).contains(&cp) || cp >= 0xF0000
}

/// The text of a glyph name: an AGL name, `uniXXXX` sequences or `uXXXX[XX]`, ligature parts
/// joined with `_`, variants after a dot left out.
fn name_text(name: &str) -> Option<String> {
    let base = agl::split_variant(name).0;
    let mut out = String::new();
    for part in base.split('_') {
        let mut found = false;
        for c in agl::name_to_chars(part) {
            if is_private(c as u32) {
                return None;
            }
            out.push(c);
            found = true;
        }
        if !found {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The text a Unicode mapping gives, cleaned: ligatures written out, white space control
/// characters as spaces. `None` when nothing usable is left (U+FFFD, noncharacters, other
/// control characters).
fn clean_bf(bf: BfString) -> Option<String> {
    let mut out = String::new();
    let mut push = |c: char| match c {
        '\u{FB00}' => out.push_str("ff"),
        '\u{FB01}' => out.push_str("fi"),
        '\u{FB02}' => out.push_str("fl"),
        '\u{FB03}' => out.push_str("ffi"),
        '\u{FB04}' => out.push_str("ffl"),
        '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
        '\t' | '\n' | '\r' => out.push(' '),
        '\u{FFFD}' | '\u{FFFE}' | '\u{FFFF}' => {}
        c if c.is_control() => {}
        c => out.push(c),
    };
    match bf {
        BfString::Char(c) => push(c),
        BfString::String(s) => s.chars().for_each(&mut push),
    }
    (!out.is_empty()).then_some(out)
}

/// The text of a page's glyphs: the device's text buffer, and the ActualText strings the
/// glyphs refer to, kept once each.
pub(crate) struct Texts<'t> {
    pub text: &'t str,
    pub actuals: &'t [Rc<str>],
}

impl Texts<'_> {
    pub(crate) fn of(&self, g: &Gl) -> &str {
        match g.actual {
            Some(i) => self.actuals.get(i as usize).map_or("", |a| a),
            None => &self.text[g.start as usize..(g.start + g.len) as usize],
        }
    }
}

/// One glyph as the device keeps it. Its text is `text[start..start + len]` of the page, or
/// the ActualText it refers to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gl {
    pub start: u32,
    pub len: u32,
    /// Origin on the baseline, in the visible page box.
    pub x: f32,
    pub y: f32,
    /// Unit vector along the baseline.
    pub dx: f32,
    pub dy: f32,
    /// Rendered font size.
    pub size: f32,
    /// Advance along the baseline, when the font gives it.
    pub adv: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub mcid: Option<u32>,
    pub invisible: bool,
    /// The switched-off layer the glyph stands in.
    pub layer: Option<usize>,
    pub vertical: bool,
    /// The glyph goes on the text before it without a space (inside one ActualText span).
    pub joined: bool,
    /// The ActualText the glyph stands for, an index into the page's list.
    pub actual: Option<u32>,
    /// The URI link whose box holds the glyph's middle, an index into the page's links.
    pub link: Option<u32>,
}

/// How a sequence the interpreter opens matches the marks of the walk.
#[derive(Debug, PartialEq, Eq)]
enum MarkMatch {
    /// The mark at `at` is the sequence's own; `skipped_actual` marks with ActualText were
    /// passed over to reach it.
    Found { at: usize, skipped_actual: u64 },
    /// A mark further on looks the same but is not trusted; `actual` says it has ActualText.
    Refused { actual: bool },
    /// No mark from `from` on looks the same.
    None,
}

/// The mark from `from` on that a sequence with tag hash `want` and `mcid` is. The walk met
/// the sequences in the order the interpreter meets them, so the next mark is trusted; a
/// match further on is trusted only with an MCID or for a sequence the walk rewrote for a
/// switched-off layer, which carries the walk's own tag.
fn match_mark(marks: &[Mark], from: usize, want: u64, mcid: Option<i32>) -> MarkMatch {
    let from = from.min(marks.len());
    let Some(k) = marks[from..]
        .iter()
        .position(|m| m.tag == want && m.mcid == mcid)
    else {
        return MarkMatch::None;
    };
    let at = from + k;
    let m = &marks[at];
    if k == 0 || mcid.is_some() || m.layer.is_some() {
        let skipped_actual = marks[from..at]
            .iter()
            .filter(|m| m.actual.is_some())
            .count() as u64;
        MarkMatch::Found { at, skipped_actual }
    } else {
        MarkMatch::Refused {
            actual: m.actual.is_some(),
        }
    }
}

/// An open marked content sequence on the device.
struct Open {
    mcid: Option<u32>,
    layer: Option<usize>,
    actual: Option<Rc<str>>,
    /// The ActualText was written out with the first glyph of the sequence.
    used: bool,
}

/// The fewest pixels a side of a QR code has.
const CODE_PIXELS: u32 = 21;

/// Whether a picture's pixels could be a barcode or QR code, from its dictionary alone:
/// 1-bit pixels (a stencil mask, one bit per component, or a JBIG2 or CCITT fax filter) and
/// at least [`CODE_PIXELS`] on each side.
fn code_pixels(image: &Image<'_, '_>) -> bool {
    if image.width() < CODE_PIXELS || image.height() < CODE_PIXELS {
        return false;
    }
    let Image::Raster(r) = image else {
        return true;
    };
    let dict = r.stream().dict();
    let bits = dict
        .get::<u32>(b"BitsPerComponent")
        .or_else(|| dict.get::<u32>(b"BPC"));
    let one_bit = |n: &[u8]| matches!(n, b"JBIG2Decode" | b"CCITTFaxDecode" | b"CCF");
    let filter = match dict
        .get::<Object<'_>>(b"Filter")
        .or_else(|| dict.get::<Object<'_>>(b"F"))
    {
        Some(Object::Name(n)) => one_bit(n.as_ref()),
        Some(Object::Array(a)) => a.iter::<Name<'_>>().any(|n| one_bit(n.as_ref())),
        _ => false,
    };
    bits == Some(1) || filter
}

/// The device that keeps glyphs and picture boxes. It aborts the page with a panic payload of
/// [`Abort`] when the stop callback says so or the glyph cap is reached.
struct TextDevice<'d> {
    glyphs: Vec<Gl>,
    text: String,
    /// ActualText strings the glyphs refer to.
    actuals: Vec<Rc<str>>,
    images: Vec<[f32; 4]>,
    images_code: Vec<bool>,
    rules: Vec<Rule>,
    /// Lines were drawn past the cap and are not kept.
    rules_cut: bool,
    open: Vec<Open>,
    marks: &'d [Mark],
    next_mark: usize,
    /// Sequences whose ActualText was not used because they did not match the walk.
    out_of_step: u64,
    /// Sequences of switched-off layers that did not match the walk.
    layers_out_of_step: u64,
    /// The tags the walk gave rewritten switched-off sequences.
    layer_tags: HashSet<u64>,
    fonts: &'d HashMap<u128, FontInfo>,
    origin: Origin<'d>,
    programs: &'d mut Programs,
    /// Fonts whose embedded program did not load and a stand-in is in use.
    stand_ins: HashSet<u128>,
    /// Glyph tables were not read on this page: the copy budget is spent.
    tables_cut: bool,
    stop: &'d dyn Fn() -> bool,
    calls: u64,
    cap: usize,
    /// The glyph run drawn last, to read a run that is filled and then stroked once.
    last: Option<u64>,
    unmapped: u64,
    unmapped_visible: u64,
    mapped: u64,
}

impl TextDevice<'_> {
    fn tick(&mut self) {
        self.calls += 1;
        if self.calls.is_multiple_of(POLL) && (self.stop)() {
            panic::resume_unwind(Box::new(Abort::Stop));
        }
    }

    /// The text of a glyph from the font's own tables, for an embedded font whose program
    /// loaded. The tables are read once per program and charged to the copy budget.
    fn fallback(&mut self, g: &Glyph<'_>) -> Option<String> {
        let Glyph::Outline(o) = g else {
            return None;
        };
        let key = o.font_cache_key();
        let info = self.fonts.get(&key)?;
        if !info.embedded {
            // The program in use is a stand-in; its tables say nothing about this font.
            return None;
        }
        let program = match self.programs.of_font.get(&key) {
            Some(p) => *p,
            None => {
                let data = o.font_data();
                // A program of another size than the embedded one is a stand-in for a
                // program that did not load.
                let size_in_use = data.as_ref().map(|d| d.data.as_ref().as_ref().len() as u64);
                let p = match (info.program, size_in_use) {
                    (Some((id, size)), Some(used)) if used == size => Some(id),
                    (Some(_), Some(_)) => {
                        self.stand_ins.insert(key);
                        None
                    }
                    // The interpreter gives no program data for this font kind.
                    _ => None,
                };
                self.programs.of_font.insert(key, p);
                if let Some(id) = p
                    && !self.programs.tables.contains_key(&id)
                {
                    let tables = if self.programs.spent {
                        self.tables_cut = true;
                        GlyphNames::default()
                    } else {
                        let cmap = match &info.collection {
                            Some(family) => collection_cmap(&mut self.programs.collections, family),
                            None => None,
                        };
                        let built = o
                            .font_data()
                            .map(|d| GlyphNames::read(d.data.as_ref().as_ref(), cmap.as_deref()))
                            .unwrap_or_default();
                        if self.origin.charge_copy(built.bytes()) {
                            built
                        } else {
                            self.programs.spent = true;
                            self.tables_cut = true;
                            GlyphNames::default()
                        }
                    };
                    self.programs.tables.insert(id, Rc::new(tables));
                }
                p
            }
        };
        let tables = self.programs.tables.get(&program?)?;
        tables.by_gid.get(&o.glyph_id().to_u32()).cloned()
    }

    fn push_glyph(&mut self, text: &str, g: Gl) {
        let start = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        self.text.push_str(text);
        let len = u32::try_from(text.len()).unwrap_or(0);
        self.glyphs.push(Gl { start, len, ..g });
    }
}

/// A filled rectangle at most this many points thick is a rule, not a shaded area.
const THIN: f32 = 2.0;

/// Straight segments shorter than this many points are not rules.
const SHORTEST: f32 = 1.0;

/// The rules a path draws once `transform` maps it into the page box: each straight
/// horizontal or vertical segment of a stroked path is a line, a filled axis-aligned
/// rectangle is a line when thin and a rectangle otherwise. Curves and slanted segments are
/// not rules.
fn rules_of(path: &BezPath, transform: Affine, stroke: bool, keep: &mut dyn FnMut(Rule)) {
    use kurbo::PathEl;
    // The first points of the subpath being drawn (a rectangle has at most five) and how
    // many it has, and whether it holds a curve.
    let mut corners: [Point; 5] = [Point::ZERO; 5];
    let mut count = 0usize;
    let mut curved = false;
    let mut start = Point::ZERO;
    let mut last = Point::ZERO;
    // A filled subpath that ends is a rule when it is a rectangle.
    let filled = |corners: &[Point; 5], count: usize, curved: bool| -> Option<Rule> {
        if stroke || curved || !(4..=5).contains(&count) {
            return None;
        }
        rectangle(&corners[..count])
    };
    let segment = |a: Point, b: Point| -> Option<Rule> {
        let (dx, dy) = ((b.x - a.x).abs() as f32, (b.y - a.y).abs() as f32);
        let line = |x0: f64, y0: f64, x1: f64, y1: f64| Rule {
            x0: x0 as f32,
            y0: y0 as f32,
            x1: x1 as f32,
            y1: y1 as f32,
            kind: RuleKind::Line,
        };
        if dy <= 0.5 && dx >= SHORTEST {
            let y = (a.y + b.y) / 2.0;
            Some(line(a.x.min(b.x), y, a.x.max(b.x), y))
        } else if dx <= 0.5 && dy >= SHORTEST {
            let x = (a.x + b.x) / 2.0;
            Some(line(x, a.y.min(b.y), x, a.y.max(b.y)))
        } else {
            None
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if let Some(r) = filled(&corners, count, curved) {
                    keep(r);
                }
                let p = transform * p;
                (start, last, count, curved) = (p, p, 1, false);
                corners[0] = p;
            }
            PathEl::LineTo(p) => {
                let p = transform * p;
                if stroke && let Some(r) = segment(last, p) {
                    keep(r);
                }
                if count < corners.len() {
                    corners[count] = p;
                }
                count += 1;
                last = p;
            }
            PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                curved = true;
                last = transform * p;
                count += 1;
            }
            PathEl::ClosePath => {
                if stroke && let Some(r) = segment(last, start) {
                    keep(r);
                }
                last = start;
            }
        }
    }
    if let Some(r) = filled(&corners, count, curved) {
        keep(r);
    }
}

/// The rule a filled four-cornered path is when its sides are horizontal and vertical.
fn rectangle(points: &[Point]) -> Option<Rule> {
    let mut points = points.to_vec();
    if points.len() == 5 {
        let (a, b) = (points[0], points[4]);
        if (a.x - b.x).abs() > 0.5 || (a.y - b.y).abs() > 0.5 {
            return None;
        }
        points.pop();
    }
    for k in 0..4 {
        let (a, b) = (points[k], points[(k + 1) % 4]);
        if (a.x - b.x).abs() > 0.5 && (a.y - b.y).abs() > 0.5 {
            return None;
        }
    }
    let x0 = points.iter().map(|p| p.x).fold(f64::MAX, f64::min) as f32;
    let x1 = points.iter().map(|p| p.x).fold(f64::MIN, f64::max) as f32;
    let y0 = points.iter().map(|p| p.y).fold(f64::MAX, f64::min) as f32;
    let y1 = points.iter().map(|p| p.y).fold(f64::MIN, f64::max) as f32;
    let (w, h) = (x1 - x0, y1 - y0);
    if w.max(h) < SHORTEST {
        return None;
    }
    Some(if h <= THIN {
        let y = (y0 + y1) / 2.0;
        Rule {
            x0,
            y0: y,
            x1,
            y1: y,
            kind: RuleKind::Line,
        }
    } else if w <= THIN {
        let x = (x0 + x1) / 2.0;
        Rule {
            x0: x,
            y0,
            x1: x,
            y1,
            kind: RuleKind::Line,
        }
    } else {
        Rule {
            x0,
            y0,
            x1,
            y1,
            kind: RuleKind::Rect,
        }
    })
}

/// Glyphs of one text show whose font tells no advance (Type3) get the distance to the next
/// glyph along the baseline; the last one gets the advance of the one before it.
fn advances_from_positions(run: &mut [Gl]) {
    for i in 0..run.len() {
        if run[i].adv.is_some() {
            continue;
        }
        let g = run[i];
        let next = run
            .get(i + 1)
            .map(|n| (n.x - g.x) * g.dx + (n.y - g.y) * g.dy);
        run[i].adv = match next {
            Some(d) if d > 0.0 => Some(d),
            _ if i > 0 => run[i - 1].adv,
            _ => None,
        };
    }
}

/// The CID to Unicode table of a CID collection, parsed once.
fn collection_cmap(
    cache: &mut HashMap<Vec<u8>, Option<Rc<CMap>>>,
    family: &CidFamily,
) -> Option<Rc<CMap>> {
    let name = family.ucs2_cmap()?;
    let key = name.to_bytes().to_vec();
    cache
        .entry(key)
        .or_insert_with(|| {
            let data = hayro_cmap::load_embedded(name)?;
            CMap::parse(data, hayro_cmap::load_embedded).map(Rc::new)
        })
        .clone()
}

impl<'a> Device<'a> for TextDevice<'_> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        self.tick();
        // Lines in a switched-off layer are not on the page.
        if matches!(mode, DrawMode::Invisible) || self.open.iter().any(|o| o.layer.is_some()) {
            return;
        }
        let stroke = !matches!(mode, DrawMode::Fill(_));
        let (rules, cap) = (&mut self.rules, self.cap);
        let mut cut = false;
        rules_of(path, props.transform, stroke, &mut |r| {
            if rules.len() < cap {
                rules.push(r);
            } else {
                cut = true;
            }
        });
        self.rules_cut |= cut;
    }

    fn push_clip_path(&mut self, _: &ClipPath) {
        // Every form the interpreter runs starts with a clip; forms can be costly.
        self.calls += 1;
        if (self.stop)() {
            panic::resume_unwind(Box::new(Abort::Stop));
        }
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.tick();
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        self.tick();
        let glyphs = run.glyphs();
        let Some(first) = glyphs.first() else {
            return;
        };
        let mut h = DefaultHasher::new();
        glyphs.len().hash(&mut h);
        first.transform().as_coeffs().map(f64::to_bits).hash(&mut h);
        for g in glyphs {
            match &**g {
                Glyph::Outline(o) => o.glyph_id().to_u32().hash(&mut h),
                Glyph::Type3(t) => t.cache_key().hash(&mut h),
            }
        }
        let key = h.finish();
        if self.last == Some(key) {
            // Fill and stroke of one text show, or the same glyphs drawn twice in one place.
            return;
        }
        self.last = Some(key);
        let layer = self.open.iter().rev().find_map(|o| o.layer);
        let invisible = matches!(mode, DrawMode::Invisible) || layer.is_some();
        let mcid = self.open.iter().rev().find_map(|o| o.mcid);
        let from = self.glyphs.len();
        for g in glyphs {
            if self.glyphs.len() >= self.cap {
                panic::resume_unwind(Box::new(Abort::Cap));
            }
            if (self.glyphs.len() as u64 + 1).is_multiple_of(POLL) && (self.stop)() {
                panic::resume_unwind(Box::new(Abort::Stop));
            }
            let t = (props.transform * g.transform()).as_coeffs();
            let along = (t[0] * 1000.0, t[1] * 1000.0);
            let up = (t[2] * 1000.0, t[3] * 1000.0);
            let along_len = along.0.hypot(along.1);
            let up_len = up.0.hypot(up.1);
            let size = if up_len > 0.0 { up_len } else { along_len } as f32;
            let (dx, dy) = if along_len > 0.0 {
                ((along.0 / along_len) as f32, (along.1 / along_len) as f32)
            } else {
                (1.0, 0.0)
            };
            let (adv, info) = match &**g {
                Glyph::Outline(o) => {
                    let adv = o
                        .advance_width()
                        .filter(|w| *w > 0.0)
                        .map(|w| (f64::from(w) / 1000.0 * along_len) as f32);
                    (adv, self.fonts.get(&o.font_cache_key()).cloned())
                }
                // A Type3 glyph tells no advance and no font style here.
                Glyph::Type3(_) => (None, None),
            };
            let info = info.unwrap_or_default();
            let mut gl = Gl {
                start: 0,
                len: 0,
                x: t[4] as f32,
                y: t[5] as f32,
                dx,
                dy,
                size,
                adv,
                bold: info.bold,
                italic: info.italic,
                mcid,
                invisible,
                layer,
                vertical: info.vertical,
                joined: false,
                actual: None,
                link: None,
            };
            // ActualText of the outermost sequence that has one replaces the glyphs in it.
            if let Some(o) = self.open.iter_mut().find(|o| o.actual.is_some()) {
                if o.used {
                    gl.joined = true;
                    self.push_glyph("", gl);
                } else {
                    o.used = true;
                    // The text is referred to, not copied.
                    if let Some(a) = o.actual.clone() {
                        gl.actual = u32::try_from(self.actuals.len()).ok();
                        self.actuals.push(a);
                    }
                    self.push_glyph("", gl);
                }
                continue;
            }
            let text = g
                .as_unicode()
                .and_then(clean_bf)
                .or_else(|| self.fallback(g));
            match text {
                Some(t) => {
                    if !invisible {
                        self.mapped += 1;
                    }
                    self.push_glyph(&t, gl);
                }
                None => {
                    self.unmapped += 1;
                    if !invisible {
                        self.unmapped_visible += 1;
                    }
                    self.push_glyph("\u{FFFD}", gl);
                }
            }
        }
        advances_from_positions(&mut self.glyphs[from..]);
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        self.tick();
        // The transform maps the image's pixel grid onto the page.
        let t = props.transform;
        let (w, h) = (f64::from(image.width()), f64::from(image.height()));
        let corners = [
            t * Point::new(0.0, 0.0),
            t * Point::new(w, 0.0),
            t * Point::new(0.0, h),
            t * Point::new(w, h),
        ];
        let xs = corners.map(|p| p.x as f32);
        let ys = corners.map(|p| p.y as f32);
        let min = |v: [f32; 4]| v.into_iter().fold(f32::MAX, f32::min);
        let max = |v: [f32; 4]| v.into_iter().fold(f32::MIN, f32::max);
        self.images.push([min(xs), min(ys), max(xs), max(ys)]);
        self.images_code.push(code_pixels(&image));
    }

    fn pop_clip(&mut self) {
        self.tick();
    }

    fn pop_transparency_group(&mut self) {
        self.tick();
    }

    fn begin_marked_content(&mut self, tag: &[u8], mcid: Option<i32>) {
        self.tick();
        // The walk met the sequences in the order the interpreter meets them. A match further
        // on is trusted only with an MCID; otherwise the glyphs are read.
        let want = tag_hash(tag);
        let mut actual = None;
        let mut layer = None;
        let mut own_mcid = mcid;
        match match_mark(self.marks, self.next_mark, want, mcid) {
            MarkMatch::Found { at, skipped_actual } => {
                let m = &self.marks[at];
                actual = m.actual.clone();
                // A layer comes only from the sequence the walk rewrote, never from a tag
                // that only looks the same.
                layer = m.layer;
                own_mcid = own_mcid.or(m.own_mcid);
                self.next_mark = at + 1;
                // The marks passed over are never matched again: their ActualText is not used.
                self.out_of_step += skipped_actual;
            }
            MarkMatch::Refused { actual: true } => self.out_of_step += 1,
            MarkMatch::Refused { actual: false } => {}
            MarkMatch::None => {
                if self.layer_tags.contains(&want) {
                    self.layers_out_of_step += 1;
                }
            }
        }
        self.open.push(Open {
            mcid: own_mcid.and_then(|m| u32::try_from(m).ok()),
            layer,
            actual,
            used: false,
        });
    }

    fn end_marked_content(&mut self) {
        self.tick();
        self.open.pop();
    }
}

/// Marks each glyph with the first of `links` whose box holds the glyph's middle (half its
/// advance along the baseline, a quarter of its size above it), so a run ends where a link
/// does. `x0`, `y0` move the glyphs into the links' user space. The boxes are looked up by
/// height; past [`LINK_WORK`] tests no glyph keeps a link and the result is false. A word
/// that a box edge cuts takes the link whole: the box is drawn around words.
fn link_glyphs(glyphs: &mut [Gl], text: &Texts<'_>, links: &[Link], x0: f64, y0: f64) -> bool {
    let mut order: Vec<usize> = (0..links.len()).collect();
    order.sort_by(|&a, &b| links[a].0[1].total_cmp(&links[b].0[1]));
    let bottoms: Vec<f64> = order.iter().map(|&k| f64::from(links[k].0[1])).collect();
    let tallest = links
        .iter()
        .map(|(b, _)| f64::from(b[3] - b[1]))
        .fold(0.0f64, f64::max);
    let mut work = 0usize;
    for g in glyphs.iter_mut() {
        let size = f64::from(g.size.max(0.1));
        let half = f64::from(g.adv.unwrap_or(g.size.max(0.1) * 0.5)) / 2.0;
        let (dx, dy) = (f64::from(g.dx), f64::from(g.dy));
        let cx = f64::from(g.x) + dx * half - dy * size / 4.0 + x0;
        let cy = f64::from(g.y) + dy * half + dx * size / 4.0 + y0;
        let lo = bottoms.partition_point(|b| *b < cy - tallest);
        let hi = bottoms.partition_point(|b| *b <= cy);
        work = work.saturating_add(hi.saturating_sub(lo));
        if work > LINK_WORK {
            glyphs.iter_mut().for_each(|g| g.link = None);
            return false;
        }
        g.link = order[lo.min(hi)..hi]
            .iter()
            .copied()
            .filter(|&k| {
                let b = &links[k].0;
                cx >= f64::from(b[0])
                    && cx <= f64::from(b[2])
                    && cy >= f64::from(b[1])
                    && cy <= f64::from(b[3])
            })
            .min()
            .and_then(|k| u32::try_from(k).ok());
    }
    // One ActualText span is one unit: its text stands on its first glyph, the others hold
    // none. A link on any glyph of the span goes to all of them.
    let mut k = 0;
    while k < glyphs.len() {
        if glyphs[k].actual.is_none() {
            k += 1;
            continue;
        }
        let mut e = k + 1;
        while e < glyphs.len() && glyphs[e].joined && glyphs[e].actual.is_none() {
            e += 1;
        }
        let span = &mut glyphs[k..e];
        let mut found = span.iter().filter_map(|g| g.link);
        if let Some(first) = found.next()
            && found.all(|l| l == first)
        {
            span.iter_mut().for_each(|g| g.link = Some(first));
        }
        k = e;
    }
    // Words: glyphs with ink that follow each other along one baseline without a gap.
    let ink = |g: &Gl| {
        let t = text.of(g);
        !t.is_empty() && !t.chars().all(char::is_whitespace)
    };
    // `b` stands right after `a` along the baseline.
    let follows = |a: &Gl, b: &Gl| {
        let size = a.size.max(b.size).max(0.1);
        let adv = a.adv.unwrap_or(a.size.max(0.1) * 0.5);
        let (ex, ey) = (a.x + a.dx * adv, a.y + a.dy * adv);
        let along = (b.x - ex) * a.dx + (b.y - ey) * a.dy;
        let across = -(b.x - ex) * a.dy + (b.y - ey) * a.dx;
        (a.dx - b.dx).abs() < 0.01
            && (a.dy - b.dy).abs() < 0.01
            && along.abs() <= 0.15 * size
            && across.abs() <= 0.25 * size
    };
    // Glyphs of one word follow each other in content order, also when a line is drawn from
    // right to left.
    let joins = |a: &Gl, b: &Gl| follows(a, b) || follows(b, a);
    let mut k = 0;
    while k < glyphs.len() {
        if !ink(&glyphs[k]) {
            k += 1;
            continue;
        }
        let mut e = k + 1;
        while e < glyphs.len() && ink(&glyphs[e]) && joins(&glyphs[e - 1], &glyphs[e]) {
            e += 1;
        }
        // A box edge that cuts inside the word: the link grows to the word's edges, but not
        // over punctuation at an edge (a full stop after an address, a bracket before it). A
        // box that ends where letters meet punctuation (an apostrophe, a hyphen) stays.
        let linked: Vec<usize> = (k..e).filter(|&i| glyphs[i].link.is_some()).collect();
        if let (Some(&a), Some(&z)) = (linked.first(), linked.last())
            && linked.iter().all(|&i| glyphs[i].link == glyphs[a].link)
        {
            let link = glyphs[a].link;
            let letter = |g: &Gl| text.of(g).chars().all(char::is_alphanumeric);
            let exact = |inside: usize, outside: usize| {
                letter(&glyphs[inside]) && !letter(&glyphs[outside])
            };
            let mut from = a;
            if a > k && !exact(a, a - 1) {
                from = k;
                while from < a && !letter(&glyphs[from]) {
                    from += 1;
                }
            }
            let mut to = z + 1;
            if z + 1 < e && !exact(z, z + 1) {
                to = e;
                while to > z + 1 && !letter(&glyphs[to - 1]) {
                    to -= 1;
                }
            }
            glyphs[from..to].iter_mut().for_each(|g| g.link = link);
        }
        k = e;
    }
    true
}

/// The targets of the links `glyphs` carry that no run with text carries, each once, in the
/// order of the links. A link's target is cut to 200 characters for the loss.
fn unplaced_links(glyphs: &[Gl], runs: &[Run], links: &[Link]) -> Vec<String> {
    let mut on_glyphs = vec![false; links.len()];
    for g in glyphs {
        if let Some(k) = g.link.and_then(|k| on_glyphs.get_mut(k as usize)) {
            *k = true;
        }
    }
    let placed: HashSet<*const UriTarget> = runs
        .iter()
        .filter(|r| !r.text.trim().is_empty())
        .filter_map(|r| r.uri.as_ref().map(Rc::as_ptr))
        .collect();
    let mut said: HashSet<*const UriTarget> = HashSet::new();
    links
        .iter()
        .zip(on_glyphs)
        .filter(|((_, u), on)| *on && !placed.contains(&Rc::as_ptr(u)))
        .filter(|((_, u), _)| said.insert(Rc::as_ptr(u)))
        .map(|((_, u), _)| u.uri.chars().take(200).collect())
        .collect()
}

/// From the glyphs of a page to its items. Coordinates are already in the visible page box;
/// `x0`, `y0` move user space links into it.
fn lay_out(
    page: u32,
    glyphs: &[Gl],
    text: &Texts<'_>,
    links: &[Link],
    x0: f64,
    y0: f64,
    layers: &[String],
) -> PageItems {
    let mut by_layer: Vec<Vec<Gl>> = vec![Vec::new(); layers.len()];
    let mut rest: Vec<Gl> = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        match g.layer {
            Some(k) if k < by_layer.len() => by_layer[k].push(*g),
            _ => rest.push(*g),
        }
    }
    let glyphs = &rest[..];
    let (mut visible, hidden): (Vec<Gl>, Vec<Gl>) = glyphs.iter().partition(|g| !g.invisible);
    // When most of the text reads in one other direction, the page is read in that frame.
    let mut by_dir = [0usize; 4];
    for g in &visible {
        if g.vertical {
            continue;
        }
        if let Some(t) = Turn::of(g.dx, g.dy) {
            by_dir[t as usize] += g.len as usize;
        }
    }
    let total: usize = by_dir.iter().sum();
    let turn = [Turn::Up, Turn::Half, Turn::Down]
        .into_iter()
        .find(|&t| total > 0 && by_dir[t as usize] * 2 > total)
        .unwrap_or(Turn::None);
    let mut items = PageItems {
        page,
        turned: turn != Turn::None,
        ..PageItems::default()
    };
    if items.turned && !links.is_empty() {
        let k = links.len();
        let s = if k == 1 { "" } else { "s" };
        items.notes.push(Note::Unsupported(format!(
            "{k} URI link{s} on page {page} not attached to their text: the page is read in a \
             turned frame"
        )));
    }
    if !items.turned && !links.is_empty() {
        items.links_cut = !link_glyphs(&mut visible, text, links, x0, y0);
    }
    let linked = !items.turned && !items.links_cut && !links.is_empty();
    items.runs = runs_of(page, &visible, text, turn, links);
    if linked {
        for uri in unplaced_links(&visible, &items.runs, links) {
            items.notes.push(Note::Unsupported(format!(
                "a URI link on page {page} to {uri} is not placed on its words: its glyphs \
                 are in no run with text"
            )));
        }
    }
    items.invisible = runs_of(page, &hidden, text, turn, links);
    for (name, list) in layers.iter().zip(&by_layer) {
        if !list.is_empty() {
            items
                .hidden_layers
                .push((name.clone(), runs_of(page, list, text, turn, links)));
        }
    }
    if items.runs.is_empty() && !items.invisible.is_empty() {
        // A page that is only an invisible text layer.
        let mut lines: Vec<String> = Vec::new();
        let mut last = None;
        for r in std::mem::take(&mut items.invisible) {
            if last == Some(r.line) {
                if let Some(l) = lines.last_mut() {
                    l.push_str(&r.text);
                }
            } else {
                lines.push(r.text.trim_start().to_string());
            }
            last = Some(r.line);
        }
        items.hidden_layer = lines
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();
    }
    items
}

/// Pages that need OCR or a look at the picture, with the reasons. Only `pages` are read.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn needs_ocr(
    pdf: &Pdf,
    pages: &[u32],
    caps: Caps,
    stop: &dyn Fn() -> bool,
) -> Vec<(u32, Vec<String>)> {
    let found = self::pages(pdf, pages, &[], &Marked::new(), caps, stop);
    let sizes: HashMap<u32, (f32, f32)> = pdf
        .pages(stop)
        .into_iter()
        .map(|p| (p.number, (p.width, p.height)))
        .collect();
    found
        .pages
        .iter()
        .filter_map(|items| {
            let (w, h) = sizes.get(&items.page).copied()?;
            let reasons = ocr_reasons(items, w, h);
            (!reasons.is_empty()).then_some((items.page, reasons))
        })
        .collect()
}

/// Why a page needs OCR: no text but pictures over half of it, glyphs that all lack a
/// Unicode mapping, or only invisible text over a picture (an OCR layer).
pub(crate) fn ocr_reasons(items: &PageItems, width: f32, height: f32) -> Vec<String> {
    let area = (width * height).max(1.0);
    let covered: f32 = items
        .images
        .iter()
        .map(|b| {
            let w = (b[2].min(width) - b[0].max(0.0)).max(0.0);
            let h = (b[3].min(height) - b[1].max(0.0)).max(0.0);
            w * h
        })
        .sum::<f32>()
        .min(area);
    let picture = covered / area > 0.5;
    let mut reasons = Vec::new();
    let visible_text = items.mapped > 0;
    let invisible_text = !items.hidden_layer.is_empty() || !items.invisible.is_empty();
    if !visible_text && !invisible_text && picture {
        reasons.push("no text layer: a picture covers most of the page".to_string());
    }
    if items.mapped == 0 && items.unmapped_visible > 0 {
        reasons.push("the glyphs on the page have no Unicode mapping".to_string());
    }
    if !visible_text && invisible_text && picture {
        reasons.push("only an invisible text layer over a picture".to_string());
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_on_glyphs_but_on_no_run_is_named() {
        let a = UriTarget::new("https://example.org/a");
        let b = UriTarget::new("https://example.org/b");
        let links: Vec<Link> = vec![([0.0; 4], a.clone()), ([0.0; 4], b.clone())];
        let glyph = |link: Option<u32>| Gl {
            start: 0,
            len: 0,
            x: 0.0,
            y: 0.0,
            dx: 1.0,
            dy: 0.0,
            size: 10.0,
            adv: None,
            bold: false,
            italic: false,
            mcid: None,
            invisible: false,
            layer: None,
            vertical: false,
            joined: false,
            actual: None,
            link,
        };
        let run = |text: &str, uri: Option<Rc<UriTarget>>| Run {
            page: 1,
            text: text.into(),
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 10.0,
            size: 10.0,
            bold: false,
            italic: false,
            mcid: None,
            uri,
            line: 0,
            off_axis: false,
        };
        // Link 0 is on a run with text; link 1 only on a run of white space.
        let glyphs = [glyph(Some(0)), glyph(Some(1)), glyph(None)];
        let runs = [run("word", Some(a)), run(" ", Some(b))];
        assert_eq!(
            unplaced_links(&glyphs, &runs, &links),
            ["https://example.org/b"]
        );
    }
    use crate::fixtures::{Page as FixturePage, pdf, show};

    fn open(bytes: &[u8]) -> Pdf {
        Pdf::open(bytes, None, &pdfparts::Limits::default(), &|| false).expect("opens")
    }

    fn corpus(name: &str) -> Pdf {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/corpus/pdf")
            .join(name);
        open(&std::fs::read(path).expect("corpus file"))
    }

    const CAPS: Caps = Caps {
        max_items: 2_000_000,
        max_glyphs: 1_000_000,
        max_kept: 1 << 30,
    };

    /// The rules of page 1 of a file whose page draws `content`, with `/Fm1` a form that
    /// draws `form`.
    fn rules_of_page(content: &str, form: &str, caps: Caps) -> PageItems {
        let mut b = crate::fixtures::Builder::default();
        let catalog = b.reserve();
        let tree = b.reserve();
        let font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        let fm = b.stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 595 842]",
            form.as_bytes(),
        );
        let cs = b.stream("", content.as_bytes());
        let kid = b.add(format!(
            "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 {font} 0 R >> /XObject << /Fm1 {fm} 0 R >> >> \
             /Contents {cs} 0 R >>"
        ));
        b.set(
            tree,
            format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"),
        );
        b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
        let pdf = open(&b.finish(catalog));
        let mut found = pages(&pdf, &[1], &pdf.links(), &Marked::new(), caps, &|| false);
        assert!(found.failed.is_empty(), "{:?}", found.failed);
        found.pages.remove(0)
    }

    #[test]
    fn rules_from_content_stream() {
        let items = rules_of_page(
            "q 2 0 0 2 0 0 cm 10 10 100 50 re S 100 350 m 250 350 l S Q \
             q 1 0 0 1 20 0 cm /Fm1 Do Q BT /F1 12 Tf 72 780 Td (Text) Tj ET",
            "50 100 200 0.5 re f 60 300 m 60 400 l S 80 500 m 150 560 l S",
            CAPS,
        );
        let line = |x0: f32, y0: f32, x1: f32, y1: f32| (x0, y0, x1, y1, RuleKind::Line);
        let mut got: Vec<(f32, f32, f32, f32, RuleKind)> = items
            .rules
            .iter()
            .map(|r| {
                let k = |v: f32| (v * 4.0).round() / 4.0;
                (k(r.x0), k(r.y0), k(r.x1), k(r.y1), r.kind)
            })
            .collect();
        got.sort_by(|a, b| a.partial_cmp(b).expect("numbers"));
        let mut want = vec![
            // The rectangle stroked under a scale of two: its four sides.
            line(20.0, 20.0, 220.0, 20.0),
            line(220.0, 20.0, 220.0, 120.0),
            line(20.0, 120.0, 220.0, 120.0),
            line(20.0, 20.0, 20.0, 120.0),
            line(200.0, 700.0, 500.0, 700.0),
            // In the form, moved 20 points: a thin filled bar and a stroked line. The slanted
            // line is no rule.
            line(70.0, 100.25, 270.0, 100.25),
            line(80.0, 300.0, 80.0, 400.0),
        ];
        want.sort_by(|a, b| a.partial_cmp(b).expect("numbers"));
        assert_eq!(got, want);
        assert_eq!(
            items
                .runs
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            "Text"
        );
    }

    #[test]
    fn a_filled_box_is_a_rectangle_rule() {
        let items = rules_of_page("1 0 0 rg 100 100 50 30 re f", "", CAPS);
        assert_eq!(
            items.rules,
            [Rule {
                x0: 100.0,
                y0: 100.0,
                x1: 150.0,
                y1: 130.0,
                kind: RuleKind::Rect
            }]
        );
    }

    #[test]
    fn rules_bounded() {
        // More lines than a page may keep: the first ones are kept and a loss says the rest
        // are not used.
        let caps = Caps {
            max_glyphs: 1_000,
            ..CAPS
        };
        let items = rules_of_page(&"10 10 m 100 10 l S\n".repeat(5_000), "", caps);
        assert_eq!(items.rules.len(), 1_000);
        assert!(
            items
                .notes
                .iter()
                .any(|n| matches!(n, Note::Limit(w) if w.contains("drawn lines of page 1"))),
            "{:?}",
            items.notes
        );
        // An operator bomb inside the object limit: the walk cuts it with a loss, and the
        // rules kept stay under the cap.
        let start = std::time::Instant::now();
        let items = rules_of_page(&"10 10 m 100 10 l S\n".repeat(1_000_000), "", CAPS);
        assert!(start.elapsed().as_secs() < 30, "{:?}", start.elapsed());
        assert!(items.rules.len() <= CAPS.max_glyphs);
        assert!(
            items.notes.iter().any(|n| matches!(n, Note::Limit(_))),
            "{:?}",
            items.notes
        );
    }

    #[test]
    fn marks_passed_over_count_their_actual_text() {
        let mark = |tag: &[u8], mcid: Option<i32>, actual: Option<&str>| Mark {
            tag: tag_hash(tag),
            mcid,
            own_mcid: mcid,
            actual: actual.map(Rc::from),
            layer: None,
        };
        let marks = [
            mark(b"Span", None, Some("first")),
            mark(b"Span", None, None),
            mark(b"Span", None, Some("third")),
            mark(b"P", Some(7), Some("own")),
        ];
        // The sequence with MCID 7 is trusted further on; two of the marks passed over had
        // ActualText.
        assert_eq!(
            match_mark(&marks, 0, tag_hash(b"P"), Some(7)),
            MarkMatch::Found {
                at: 3,
                skipped_actual: 2
            }
        );
        // The next mark is its own, with nothing passed over.
        assert_eq!(
            match_mark(&marks, 0, tag_hash(b"Span"), None),
            MarkMatch::Found {
                at: 0,
                skipped_actual: 0
            }
        );
        // Without an MCID a match further on is refused.
        let refused = [mark(b"P", None, None), mark(b"Span", None, Some("x"))];
        assert_eq!(
            match_mark(&refused, 0, tag_hash(b"Span"), None),
            MarkMatch::Refused { actual: true }
        );
        assert_eq!(match_mark(&marks, 0, tag_hash(b"Q"), None), MarkMatch::None);
    }

    #[test]
    fn scanned_page_needs_ocr() {
        let found = needs_ocr(&corpus("scan-truman-letter-nara.pdf"), &[1], CAPS, &|| {
            false
        });
        assert!(
            found.iter().any(|(p, why)| *p == 1 && !why.is_empty()),
            "{found:?}"
        );
    }

    #[test]
    fn text_page_needs_no_ocr() {
        let found = needs_ocr(&corpus("pypdf-multicolumn.pdf"), &[1], CAPS, &|| false);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn unmapped_invisible_glyphs_do_not_ask_for_ocr() {
        let items = PageItems {
            unmapped: 5,
            unmapped_visible: 0,
            ..PageItems::default()
        };
        assert!(ocr_reasons(&items, 595.0, 842.0).is_empty());
        let items = PageItems {
            unmapped: 5,
            unmapped_visible: 5,
            ..PageItems::default()
        };
        assert_eq!(
            ocr_reasons(&items, 595.0, 842.0),
            ["the glyphs on the page have no Unicode mapping"]
        );
    }

    #[test]
    fn stop_is_asked_before_each_page() {
        let page = FixturePage::text(show("F1", 12.0, 72.0, 700.0, "Some words."));
        let pdf = open(&pdf(&vec![page; 10], ""));
        let asked = std::cell::Cell::new(0);
        // Yes from the fifth question on: four pages are read.
        let stop = || {
            asked.set(asked.get() + 1);
            asked.get() > 4
        };
        let all: Vec<u32> = (1..=10).collect();
        let found = pages(&pdf, &all, &pdf.links(), &Marked::new(), CAPS, &stop);
        assert_eq!(found.pages.len(), 4);
        assert_eq!(found.stopped_at, Some(5));
        assert_eq!(asked.get(), 5);
    }

    #[test]
    fn a_failing_page_keeps_its_neighbours() {
        // Page 2's content takes more work than a page may: it alone is cut.
        let words = |n: u32| show("F1", 12.0, 72.0, 700.0, &format!("Page {n} words."));
        let flood = format!("{}{}", words(2), "0 0 m\n".repeat(5_000));
        let pages_in = [
            FixturePage::text(words(1)),
            FixturePage::text(flood),
            FixturePage::text(words(3)),
        ];
        let pdf = open(&pdf(&pages_in, ""));
        let caps = Caps {
            max_items: 1_000,
            max_glyphs: 1_000,
            max_kept: 1 << 30,
        };
        let found = pages(
            &pdf,
            &[1, 2, 3],
            &pdf.links(),
            &Marked::new(),
            caps,
            &|| false,
        );
        let read: Vec<u32> = found.pages.iter().map(|p| p.page).collect();
        assert_eq!(read, [1, 2, 3]);
        assert!(found.failed.is_empty(), "{:?}", found.failed);
        let text = |n: usize| -> String {
            found.pages[n]
                .runs
                .iter()
                .map(|r| r.text.as_str())
                .collect()
        };
        assert_eq!(text(0), "Page 1 words.");
        assert_eq!(text(1), "Page 2 words.");
        assert_eq!(text(2), "Page 3 words.");
        assert!(found.pages[0].notes.is_empty());
        assert!(
            matches!(found.pages[1].notes.as_slice(), [Note::Limit(_)]),
            "{:?}",
            found.pages[1].notes
        );
        assert!(found.pages[2].notes.is_empty());
    }

    #[test]
    fn render_modes_and_layers_are_rewritten_in_place() {
        let mut data =
            b"BT /F1 12 Tf 7 Tr 72 650 Td (x) Tj 4.0 Tr 0 Tr ET /OC /L1 BDC EMC".to_vec();
        let rewrites = HashMap::from([
            (2, Rewrite::Mode(3)),
            (5, Rewrite::Mode(0)),
            (8, Rewrite::Layer(0)),
        ]);
        assert_eq!(edit(&mut data, None, &rewrites), (true, vec![]));
        assert_eq!(
            String::from_utf8_lossy(&data),
            "BT /F1 12 Tf 3 Tr 72 650 Td (x) Tj 0   Tr 0 Tr ET /Rz0 BMC    EMC"
        );
        let mut data = b"q 1 0 0 1 0 0 cm Q BT ET".to_vec();
        assert_eq!(edit(&mut data, Some(3), &HashMap::new()), (true, vec![]));
        assert_eq!(String::from_utf8_lossy(&data), "q 1 0 0 1 0 0 cm Q");
    }

    #[test]
    fn glyph_names_give_text() {
        assert_eq!(name_text("a").as_deref(), Some("a"));
        assert_eq!(name_text("f_i").as_deref(), Some("fi"));
        assert_eq!(name_text("uni00410042").as_deref(), Some("AB"));
        assert_eq!(name_text("A.swash").as_deref(), Some("A"));
        assert_eq!(name_text("square"), None);
        assert_eq!(name_text("g12"), None);
    }

    #[test]
    fn ligatures_are_written_out() {
        assert_eq!(clean_bf(BfString::Char('\u{FB01}')).as_deref(), Some("fi"));
        assert_eq!(clean_bf(BfString::Char('\u{FFFD}')), None);
        assert_eq!(clean_bf(BfString::Char('\t')).as_deref(), Some(" "));
    }
}
