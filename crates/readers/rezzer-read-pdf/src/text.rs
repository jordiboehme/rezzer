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
//! glyphs become runs and lines in reading order with Rezzer's own rules, below.

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
use pdfparts::{LinkTarget, Pdf};
use read_fonts::TableProvider;
use read_fonts::ps::agl;
use read_fonts::ps::cff::CffFontRef;
use skrifa::{FontRef, MetadataProvider};
use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};

use crate::layout::Run;

/// Glyphs and device calls between two questions to the stop callback.
const POLL: u64 = 4096;

/// Bytes a stream unpacks to per unit of work.
const BYTES_PER_UNIT: u64 = 64;

/// Nesting of forms the interpreter follows (hayro-interpret's own bound).
const FORM_DEPTH: usize = 50;

/// Run and link pairs one page may test for a URI link over a run.
const LINK_WORK: usize = 16_000_000;

/// A gap wider than this many font sizes between two glyphs of a line separates two words.
const WORD_GAP: f32 = 0.15;

/// A gap wider than this many font sizes splits a line into pieces, which may stand in
/// different columns.
const PIECE_GAP: f32 = 0.8;

/// A column gutter is at least this many font sizes wide.
const GUTTER: f32 = 0.8;

/// Share of the pieces of a region that may cross a column gutter (titles, notes, footers
/// that span the columns).
const CROSSERS: f32 = 0.15;

/// The lines of a text column hold at least this many characters on median; shorter pieces
/// side by side are a table or a form, read row by row.
const PROSE_CHARS: usize = 15;

/// Above this many pieces in a region, columns are not looked for.
const COLUMN_PIECES: usize = 20_000;

/// Deepest nesting of column splits.
const COLUMN_DEPTH: usize = 8;

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
}

impl PageItems {
    /// Copies the runs into fresh allocations.
    fn compact(&mut self) {
        let copy = |list: &mut Vec<Run>| *list = list.to_vec();
        copy(&mut self.runs);
        copy(&mut self.invisible);
        for (_, list) in &mut self.hidden_layers {
            copy(list);
        }
    }

    /// About the memory the items take once their lists are shrunk to fit.
    fn kept_bytes(&mut self) -> u64 {
        // A link's target is shared by its runs: it counts once.
        let mut links: HashSet<*const u8> = HashSet::new();
        let mut runs = |list: &mut Vec<Run>| -> u64 {
            list.shrink_to_fit();
            list.iter()
                .map(|r| {
                    let link = match &r.uri {
                        Some(u) if links.insert(u.as_ptr()) => u.len() as u64 + 32,
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
        total + 16 * self.images.len() as u64 + 256
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
    /// The caps for a read with `max_items` and `max_memory` bytes: a page's glyphs cost about
    /// 600 bytes each until laid out and may take a quarter of the memory; what the pages
    /// keep for layout may take a sixteenth, as building each page leaves memory behind.
    pub(crate) fn new(max_items: usize, max_memory: u64) -> Caps {
        let glyphs = usize::try_from(max_memory / 4 / 600).unwrap_or(usize::MAX);
        Caps {
            max_items,
            max_glyphs: glyphs.max(1_000),
            max_kept: (max_memory / 16).max(1 << 20),
        }
    }
}

/// Reads the text of `wanted` pages (1-based, ascending, none refused by the guard) from the
/// canonical bytes. `stop` is asked before each page and while a page is read; a page the
/// stop cuts is not kept.
pub(crate) fn pages(pdf: &Pdf, wanted: &[u32], caps: Caps, stop: &dyn Fn() -> bool) -> Extracted {
    let mut out = Extracted::default();
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
    let opened = panic::catch_unwind(AssertUnwindSafe(|| {
        hayro_syntax::Pdf::new(pdf.canonical_shared())
    }));
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
    if found.len() != pdf.page_count() as usize {
        all_fail(
            &mut out,
            format!(
                "it finds {} pages where the object layer finds {}, so it could number them \
                 differently",
                found.len(),
                pdf.page_count()
            ),
            Fail::Damaged,
        );
        return out;
    }
    let links = uri_links(pdf);
    let off = switched_off(&doc);
    let mut reader = PageReader::new(pdf, caps, stop, off);
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
        match reader.page(n, page, on_page) {
            Read::Page(items) => out.pages.push(*items),
            Read::Failed(f) => out.failed.push(f),
            Read::Stopped => {
                out.stopped_at = Some(n);
                break;
            }
        }
    }
    out.kept_left = reader.kept_left;
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
type Link = ([f32; 4], Rc<str>);

/// The URI links of every page, from the object layer. They are never followed.
fn uri_links(pdf: &Pdf) -> HashMap<u32, Vec<Link>> {
    let mut out: HashMap<u32, Vec<Link>> = HashMap::new();
    for link in pdf.links() {
        if let LinkTarget::Uri(uri) = link.target {
            let [a, b, c, d] = link.rect;
            out.entry(link.page)
                .or_default()
                .push(([a.min(c), b.min(d), a.max(c), b.max(d)], Rc::from(uri)));
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
    pdf: &'p Pdf,
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
}

impl<'p, 'a> PageReader<'p, 'a> {
    fn new(
        pdf: &'p Pdf,
        caps: Caps,
        stop: &'p dyn Fn() -> bool,
        off: HashSet<ObjectIdentifier>,
    ) -> Self {
        PageReader {
            pdf,
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
        }
    }

    fn page(&mut self, n: u32, page: &Page<'a>, links: &[Link]) -> Read {
        let fail = |why: String, kind: Fail| Read::Failed(Failure { page: n, why, kind });
        if let Some(why) = &self.kept_spent {
            return fail(why.clone(), Fail::Limit);
        }
        let max_bytes = (self.caps.max_items as u64).saturating_mul(BYTES_PER_UNIT);
        let streams = content_streams(page.raw());
        let size: u64 = streams.iter().map(|s| stream_size(self.pdf, s)).sum();
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
        let (pdf, caps) = (self.pdf, self.caps);
        let known = &mut self.known;
        let walked = panic::catch_unwind(AssertUnwindSafe(|| {
            let mut walk = Walk::new(pdf, caps, n, known);
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
            pdf: self.pdf,
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
                Some(Abort::Cap) => notes.push(Note::Limit(format!(
                    "text of page {n} read only in part: it draws more than {} glyphs; the \
                     rest of the page is not read",
                    dev.cap
                ))),
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
        let mut items = lay_out(n, &dev.glyphs, &texts, links, crop.x0, crop.y0, &layers);
        items.images = std::mem::take(&mut dev.images);
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
        notes.append(&mut items.notes);
        items.notes = notes;
        // What every page keeps until layout counts against one bound for the file. It is
        // measured before anything is copied.
        let cost = items.kept_bytes();
        if cost > self.kept_left {
            let why = format!(
                "the text of the file takes more than the {} bytes it may keep until it is laid \
                 out",
                self.caps.max_kept
            );
            self.kept_spent = Some(why.clone());
            return fail(why, Fail::Limit);
        }
        self.kept_left -= cost;
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
    pdf: &'w Pdf,
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
    fn new(pdf: &'w Pdf, caps: Caps, page: u32, known: &'w mut Known) -> Self {
        Walk {
            pdf,
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
        stream_size(self.pdf, s)
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
            if !self.pdf.charge_copy(a.len() as u64) {
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

/// The unpacked size of a stream, as the guard measured it.
fn stream_size(pdf: &Pdf, s: &Stream<'_>) -> u64 {
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
struct Texts<'t> {
    text: &'t str,
    actuals: &'t [Rc<str>],
}

impl Texts<'_> {
    fn of(&self, g: &Gl) -> &str {
        match g.actual {
            Some(i) => self.actuals.get(i as usize).map_or("", |a| a),
            None => &self.text[g.start as usize..(g.start + g.len) as usize],
        }
    }
}

/// One glyph as the device keeps it. Its text is `text[start..start + len]` of the page, or
/// the ActualText it refers to.
#[derive(Debug, Clone, Copy)]
struct Gl {
    start: u32,
    len: u32,
    /// Origin on the baseline, in the visible page box.
    x: f32,
    y: f32,
    /// Unit vector along the baseline.
    dx: f32,
    dy: f32,
    /// Rendered font size.
    size: f32,
    /// Advance along the baseline, when the font gives it.
    adv: Option<f32>,
    bold: bool,
    italic: bool,
    mcid: Option<u32>,
    invisible: bool,
    /// The switched-off layer the glyph stands in.
    layer: Option<usize>,
    vertical: bool,
    /// The glyph goes on the text before it without a space (inside one ActualText span).
    joined: bool,
    /// The ActualText the glyph stands for, an index into the page's list.
    actual: Option<u32>,
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

/// The device that keeps glyphs and picture boxes. It aborts the page with a panic payload of
/// [`Abort`] when the stop callback says so or the glyph cap is reached.
struct TextDevice<'d> {
    glyphs: Vec<Gl>,
    text: String,
    /// ActualText strings the glyphs refer to.
    actuals: Vec<Rc<str>>,
    images: Vec<[f32; 4]>,
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
    pdf: &'d Pdf,
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
                        if self.pdf.charge_copy(built.bytes()) {
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
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {
        self.tick();
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

/// Text that reads from right to left (Hebrew, Arabic and the like).
fn is_rtl(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

/// A run being built: glyphs of one style next to each other on one baseline.
#[derive(Debug, Clone)]
struct Frag {
    text: String,
    /// Start on the baseline in the frame the run reads along +x.
    x: f32,
    y: f32,
    /// The same start in the page's frame.
    ox: f32,
    oy: f32,
    /// The end of the last glyph along the baseline.
    end: f32,
    size: f32,
    bold: bool,
    italic: bool,
    mcid: Option<u32>,
    chars: usize,
    /// Where the text of each glyph (and each space put in) starts in `text`: the units
    /// right-to-left reordering moves as a whole.
    units: Vec<u32>,
}

/// A frame that turns glyph positions so their text reads along +x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    None,
    /// Text that reads upward (rotated 90 degrees counterclockwise).
    Up,
    /// Text upside down.
    Half,
    /// Text that reads downward.
    Down,
}

impl Turn {
    /// The direction class of a unit vector, `None` when it is not near a quarter turn.
    fn of(dx: f32, dy: f32) -> Option<Turn> {
        const NEAR: f32 = 0.985;
        if dx >= NEAR {
            Some(Turn::None)
        } else if dy >= NEAR {
            Some(Turn::Up)
        } else if dx <= -NEAR {
            Some(Turn::Half)
        } else if dy <= -NEAR {
            Some(Turn::Down)
        } else {
            None
        }
    }

    /// A point in the turned frame.
    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        match self {
            Turn::None => (x, y),
            Turn::Up => (y, -x),
            Turn::Half => (-x, -y),
            Turn::Down => (-y, x),
        }
    }
}

/// Builds the runs of a set of glyphs that all read along +x in the frame `turn`, in content
/// order: a new run at a change of style, line or a jump; a space inside a run at a word gap.
/// Each run also keeps where it starts in the frame `page` the page is read in.
fn fragments(glyphs: &[Gl], text: &Texts<'_>, turn: Turn, page: Turn) -> Vec<Frag> {
    // Sized once: a list grown step by step leaves blocks the allocator does not hand out
    // again for the next page.
    let mut out: Vec<Frag> = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        let (x, y) = turn.apply(g.x, g.y);
        let t = text.of(g);
        let size = g.size.max(0.1);
        let adv = g.adv.unwrap_or(size * 0.5);
        if let Some(f) = out.last_mut() {
            let same_style = f.bold == g.bold
                && f.italic == g.italic
                && f.mcid == g.mcid
                && (f.size - size).abs() <= 0.05 * f.size.max(size);
            let gap = x - f.end;
            let on_line = (y - f.y).abs() <= 0.25 * f.size.max(size);
            if same_style && on_line && gap >= -0.5 * size && gap <= PIECE_GAP * size {
                let glued = g.joined
                    || f.text.ends_with(char::is_whitespace)
                    || t.starts_with(char::is_whitespace);
                if gap > WORD_GAP * size && !glued {
                    f.units.push(f.text.len() as u32);
                    f.text.push(' ');
                }
                if !t.is_empty() {
                    f.units.push(f.text.len() as u32);
                }
                f.text.push_str(t);
                f.chars += t.chars().count();
                f.end = f.end.max(x + adv);
                continue;
            }
        }
        let (ox, oy) = page.apply(g.x, g.y);
        out.push(Frag {
            text: t.to_string(),
            x,
            y,
            ox,
            oy,
            end: x + adv,
            size,
            bold: g.bold,
            italic: g.italic,
            mcid: g.mcid,
            chars: t.chars().count(),
            units: vec![0],
        });
    }
    out
}

/// A row of fragments on one baseline, split where a gap is wide enough to be a column
/// gutter.
#[derive(Debug, Clone)]
struct Piece {
    row: usize,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    size: f32,
    chars: usize,
    frags: Vec<usize>,
}

/// The vertical extent of a fragment: a quarter of its size below the baseline, three
/// quarters above.
fn extent(f: &Frag) -> (f32, f32) {
    (f.y - 0.25 * f.size, f.y + 0.75 * f.size)
}

/// Rows of fragments that share a baseline (by vertical overlap), top to bottom, each split
/// into pieces at wide gaps. Fragments drawn twice in one place are dropped.
fn pieces(frags: &[Frag]) -> Vec<Piece> {
    let mut order: Vec<usize> = (0..frags.len()).collect();
    order.sort_by(|&a, &b| {
        frags[b]
            .y
            .total_cmp(&frags[a].y)
            .then(frags[a].x.total_cmp(&frags[b].x))
    });
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut core = (0.0f32, 0.0f32);
    for i in order {
        let (lo, hi) = extent(&frags[i]);
        if let Some(row) = rows.last_mut() {
            let overlap = hi.min(core.1) - lo.max(core.0);
            let smaller = (hi - lo).min(core.1 - core.0).max(0.01);
            if overlap / smaller >= 0.5 {
                row.push(i);
                continue;
            }
        }
        rows.push(vec![i]);
        core = (lo, hi);
    }
    let mut out = Vec::new();
    for (r, mut row) in rows.into_iter().enumerate() {
        row.sort_by(|&a, &b| frags[a].x.total_cmp(&frags[b].x));
        // The same text drawn again on top of itself (a fake bold) is read once.
        let mut kept: Vec<usize> = Vec::with_capacity(row.len());
        for i in row {
            let dup = kept.iter().rev().take(4).any(|&k| {
                let (a, b) = (&frags[k], &frags[i]);
                a.text == b.text
                    && (a.x - b.x).abs() < 0.3 * a.size
                    && (a.y - b.y).abs() < 0.1 * a.size
            });
            if !dup {
                kept.push(i);
            }
        }
        let mut piece: Option<Piece> = None;
        for i in kept {
            let f = &frags[i];
            let (lo, hi) = extent(f);
            if let Some(p) = piece.as_mut() {
                if f.x - p.x1 <= PIECE_GAP * f.size.max(p.size) {
                    p.x1 = p.x1.max(f.end);
                    p.y0 = p.y0.min(lo);
                    p.y1 = p.y1.max(hi);
                    p.size = p.size.max(f.size);
                    p.chars += f.chars;
                    p.frags.push(i);
                    continue;
                }
                out.push(piece.take().expect("a piece"));
            }
            piece = Some(Piece {
                row: r,
                x0: f.x,
                x1: f.end.max(f.x),
                y0: lo,
                y1: hi,
                size: f.size,
                chars: f.chars,
                frags: vec![i],
            });
        }
        out.extend(piece);
    }
    out
}

/// The pieces in reading order: columns one after the other, with the pieces that cross the
/// gutter (titles, notes across the columns) splitting the columns into bands.
fn reading_order(pieces: &[Piece]) -> Vec<usize> {
    let mut out = Vec::with_capacity(pieces.len());
    region(pieces, (0..pieces.len()).collect(), 0, &mut out);
    out
}

fn region(p: &[Piece], mut ids: Vec<usize>, depth: usize, out: &mut Vec<usize>) {
    if ids.len() > 3
        && depth < COLUMN_DEPTH
        && ids.len() <= COLUMN_PIECES
        && let Some((g0, g1)) = gutter(p, &ids)
    {
        ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
        let mut band: Vec<usize> = Vec::new();
        let flush = |band: &mut Vec<usize>, out: &mut Vec<usize>| {
            if band.is_empty() {
                return;
            }
            let (left, right): (Vec<usize>, Vec<usize>) =
                band.drain(..).partition(|&i| p[i].x1 <= g0 + 0.5);
            region(p, left, depth + 1, out);
            region(p, right, depth + 1, out);
        };
        for id in ids {
            if p[id].x1 > g0 + 0.5 && p[id].x0 < g1 - 0.5 {
                flush(&mut band, out);
                out.push(id);
            } else {
                band.push(id);
            }
        }
        flush(&mut band, out);
        return;
    }
    ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
    // Without a gutter, the region is cut at its widest band of white space across, and each
    // part is looked at again: columns may stand above or below full-width text.
    if ids.len() > 3
        && depth < COLUMN_DEPTH
        && ids.len() <= COLUMN_PIECES
        && let Some(at) = widest_gap(p, &ids)
    {
        let rest = ids.split_off(at);
        region(p, ids, depth + 1, out);
        region(p, rest, depth + 1, out);
        return;
    }
    out.extend(ids);
}

/// Where to cut pieces sorted by row: after the row with the widest white space below it,
/// when that space is at least half a line high. The index is the first piece below the cut.
fn widest_gap(p: &[Piece], ids: &[usize]) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    let mut low = f32::MAX;
    for k in 1..ids.len() {
        let (prev, cur) = (&p[ids[k - 1]], &p[ids[k]]);
        low = low.min(prev.y0);
        if cur.row == prev.row {
            continue;
        }
        // The lowest point of everything above against the top of the next row.
        let top = ids[k..]
            .iter()
            .take_while(|&&i| p[i].row == cur.row)
            .map(|&i| p[i].y1)
            .fold(f32::MIN, f32::max);
        let gap = low - top;
        if gap >= 0.5 * prev.size.max(cur.size) && best.is_none_or(|(g, _)| gap > g) {
            best = Some((gap, k));
        }
    }
    best.map(|(_, k)| k)
}

/// A column gutter in a region: the widest vertical band of white space that only a few
/// pieces cross, with prose columns on both sides that stand side by side.
fn gutter(p: &[Piece], ids: &[usize]) -> Option<(f32, f32)> {
    let lo = ids.iter().map(|&i| p[i].x0).fold(f32::MAX, f32::min);
    let hi = ids.iter().map(|&i| p[i].x1).fold(f32::MIN, f32::max);
    if !(hi - lo).is_finite() || hi - lo < 2.0 {
        return None;
    }
    let width = ((hi - lo).ceil() as usize).min(20_000) + 2;
    let scale = (hi - lo) / (width - 2) as f32;
    let bin = |x: f32| (((x - lo) / scale) as usize).min(width - 1);
    let mut diff = vec![0i32; width + 1];
    for &i in ids {
        diff[bin(p[i].x0)] += 1;
        diff[bin(p[i].x1) + 1] -= 1;
    }
    let allowed = (CROSSERS * ids.len() as f32).floor() as i32;
    let mut sizes: Vec<f32> = ids.iter().map(|&i| p[i].size).collect();
    sizes.sort_by(f32::total_cmp);
    let min_gap = GUTTER * sizes[sizes.len() / 2];
    let mut cover = Vec::with_capacity(width);
    let mut c = 0;
    for d in diff.iter().take(width) {
        c += d;
        cover.push(c);
    }
    // The band covered by the fewest pieces wins: a ragged line end next to the gutter must
    // not widen it. Only bands with text on both sides count.
    let band = |level: i32| {
        let mut best: Option<(f32, f32)> = None;
        let mut start = None;
        for (b, &c) in cover.iter().enumerate() {
            match (c <= level, start) {
                (true, None) => start = Some(b),
                (false, Some(s)) => {
                    let (g0, g1) = (lo + s as f32 * scale, lo + b as f32 * scale);
                    if s > 0 && g1 - g0 >= min_gap && best.is_none_or(|(a, z)| g1 - g0 > z - a) {
                        best = Some((g0, g1));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        best
    };
    let best = (0..=allowed).find_map(band);
    let (g0, g1) = best?;
    let crossing = ids
        .iter()
        .filter(|&&i| p[i].x1 > g0 + 0.5 && p[i].x0 < g1 - 0.5)
        .count();
    if crossing as i32 > allowed {
        return None;
    }
    let left: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| p[i].x1 <= g0 + 0.5)
        .collect();
    let right: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| p[i].x0 >= g1 - 0.5)
        .collect();
    let prose = |side: &[usize]| {
        if side.len() < 2 {
            return false;
        }
        let mut chars: Vec<usize> = side.iter().map(|&i| p[i].chars).collect();
        chars.sort_unstable();
        chars[chars.len() / 2] >= PROSE_CHARS
    };
    if !prose(&left) || !prose(&right) {
        return None;
    }
    // The columns stand side by side: their heights overlap.
    let span = |side: &[usize]| {
        (
            side.iter().map(|&i| p[i].y0).fold(f32::MAX, f32::min),
            side.iter().map(|&i| p[i].y1).fold(f32::MIN, f32::max),
        )
    };
    let (a, b) = (span(&left), span(&right));
    let overlap = a.1.min(b.1) - a.0.max(b.0);
    let smaller = (a.1 - a.0).min(b.1 - b.0).max(0.01);
    (overlap / smaller >= 0.3).then_some((g0, g1))
}

/// Lines of fragments in reading order, as lists of fragment indices.
fn lines_of(frags: &[Frag], columns: bool) -> Vec<Vec<usize>> {
    let ps = pieces(frags);
    let order = if columns {
        reading_order(&ps)
    } else {
        let mut o: Vec<usize> = (0..ps.len()).collect();
        o.sort_by(|&a, &b| {
            ps[a]
                .row
                .cmp(&ps[b].row)
                .then(ps[a].x0.total_cmp(&ps[b].x0))
        });
        o
    };
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut last_row = None;
    for i in order {
        let p = &ps[i];
        if last_row == Some(p.row)
            && let Some(l) = lines.last_mut()
        {
            l.extend(p.frags.iter().copied());
        } else {
            lines.push(p.frags.clone());
        }
        last_row = Some(p.row);
    }
    lines
}

/// The runs of one line, numbered `line`. A run that starts a new word gets a leading space;
/// a line with right-to-left text becomes one run in logical order.
fn line_runs(page: u32, frags: &[Frag], ids: &[usize], line: u32) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::with_capacity(ids.len());
    let mut prev: Option<&Frag> = None;
    for &i in ids {
        let f = &frags[i];
        let mut text = f.text.clone();
        if let Some(p) = prev {
            let gap = f.x - p.end;
            let glued =
                p.text.ends_with(char::is_whitespace) || text.starts_with(char::is_whitespace);
            if !glued && gap > WORD_GAP * f.size.max(p.size) {
                text.insert(0, ' ');
            }
        }
        runs.push(Run {
            page,
            text,
            x: f.x,
            y: f.y,
            w: (f.end - f.x).max(0.0),
            h: f.size,
            size: f.size,
            bold: f.bold,
            italic: f.italic,
            mcid: f.mcid,
            uri: None,
            line,
        });
        prev = Some(f);
    }
    if runs.iter().any(|r| r.text.chars().any(is_rtl)) {
        // The glyphs stand in visual order. Reordering a visual line by the bidi rules gives
        // the logical order back; the text of one glyph (a ligature mapped to several
        // characters) moves as a whole.
        let mut units: Vec<&str> = Vec::new();
        for (r, &i) in runs.iter().zip(ids) {
            let f = &frags[i];
            if r.text.len() > f.text.len() {
                units.push(" ");
            }
            let mut starts: Vec<usize> = f.units.iter().map(|&u| u as usize).collect();
            starts.push(f.text.len());
            for w in starts.windows(2) {
                if w[1] > w[0] {
                    units.push(&f.text[w[0]..w[1]]);
                }
            }
        }
        let logical = logical_order(&units);
        let mut one = runs.swap_remove(0);
        let x0 = runs.iter().map(|r| r.x).fold(one.x, f32::min);
        let x1 = runs.iter().map(|r| r.x + r.w).fold(one.x + one.w, f32::max);
        one.text = logical;
        one.x = x0;
        one.w = x1 - x0;
        runs = vec![one];
    }
    runs
}

/// The units of a visual line in logical order. In the bidi algorithm each unit stands for
/// its first strongly directed character; a unit with text of both directions is already in
/// logical order inside and stands for a space, taking the direction around it.
fn logical_order(units: &[&str]) -> String {
    let stand_in: String = units
        .iter()
        .map(|u| {
            let rtl = u.chars().any(is_rtl);
            let ltr = u.chars().any(|c| bidi_class(c) == BidiClass::L);
            if rtl && ltr {
                return ' ';
            }
            u.chars()
                .find(|&c| matches!(bidi_class(c), BidiClass::L | BidiClass::R | BidiClass::AL))
                .or_else(|| u.chars().next())
                .unwrap_or(' ')
        })
        .collect();
    let rtl = stand_in.chars().filter(|&c| is_rtl(c)).count();
    let ltr = stand_in
        .chars()
        .filter(|&c| bidi_class(c) == BidiClass::L)
        .count();
    let level = if rtl > ltr {
        Level::rtl()
    } else {
        Level::ltr()
    };
    let info = ParagraphBidiInfo::new(&stand_in, Some(level));
    let levels = info.reordered_levels_per_char(0..stand_in.len());
    ParagraphBidiInfo::reorder_visual(&levels)
        .into_iter()
        .filter_map(|i| units.get(i).copied())
        .collect()
}

/// Vertical writing: one run per column, glyphs top to bottom, columns from right to left.
fn vertical_runs(
    page: u32,
    glyphs: &[Gl],
    text: &Texts<'_>,
    turn: Turn,
    first_line: u32,
) -> Vec<Run> {
    let mut cols: Vec<(Run, f32)> = Vec::new();
    for g in glyphs {
        let t = text.of(g);
        let size = g.size.max(0.1);
        let (x, y) = turn.apply(g.x, g.y);
        if let Some((c, bottom)) = cols.last_mut()
            && (x - c.x).abs() <= 0.5 * size
            && y <= *bottom + 0.1 * size
            && *bottom - y <= 2.0 * size
        {
            c.text.push_str(t);
            *bottom = y;
            c.h = c.h.max(size);
            continue;
        }
        cols.push((
            Run {
                page,
                text: t.to_string(),
                x,
                y,
                w: size,
                h: size,
                size,
                bold: g.bold,
                italic: g.italic,
                mcid: g.mcid,
                uri: None,
                line: 0,
            },
            y,
        ));
    }
    let mut runs: Vec<Run> = cols.into_iter().map(|(r, _)| r).collect();
    runs.sort_by(|a, b| b.x.total_cmp(&a.x).then(b.y.total_cmp(&a.y)));
    for (k, r) in runs.iter_mut().enumerate() {
        r.line = first_line + k as u32;
    }
    runs
}

/// Runs in reading order for one set of glyphs (visible or invisible), with the frame turn
/// used: the main text along +x first, then text in other directions, then vertical writing.
fn runs_of(page: u32, glyphs: &[Gl], text: &Texts<'_>, turn: Turn) -> Vec<Run> {
    let mut main = Vec::with_capacity(glyphs.len());
    let mut vertical = Vec::new();
    let mut others: Vec<(Turn, Vec<Gl>)> = Vec::new();
    for g in glyphs {
        if g.vertical {
            vertical.push(*g);
            continue;
        }
        let (dx, dy) = turn.apply(g.dx, g.dy);
        match Turn::of(dx, dy) {
            Some(Turn::None) => main.push(*g),
            dir => {
                // Glyphs at an odd angle read in the frame of the nearest quarter turn.
                let t = dir.unwrap_or(if dx.abs() >= dy.abs() {
                    if dx >= 0.0 { Turn::None } else { Turn::Half }
                } else if dy >= 0.0 {
                    Turn::Up
                } else {
                    Turn::Down
                });
                if t == Turn::None {
                    main.push(*g);
                } else if let Some((_, list)) = others.iter_mut().find(|(d, _)| *d == t) {
                    list.push(*g);
                } else {
                    others.push((t, vec![*g]));
                }
            }
        }
    }
    let mut runs = Vec::with_capacity(main.len());
    let mut line = 0u32;
    let frags = fragments(&main, text, turn, turn);
    for ids in lines_of(&frags, true) {
        runs.extend(line_runs(page, &frags, &ids, line));
        line += 1;
    }
    for (dir, list) in others {
        // Lines are found in the frame where this text reads along +x; the runs keep the
        // position of their first glyph in the page's frame.
        let frame = compose(turn, dir);
        let frags = fragments(&list, text, frame, turn);
        for ids in lines_of(&frags, false) {
            let mut r = line_runs(page, &frags, &ids, line);
            for (run, &i) in r.iter_mut().zip(&ids) {
                (run.x, run.y) = (frags[i].ox, frags[i].oy);
            }
            runs.extend(r);
            line += 1;
        }
    }
    runs.extend(vertical_runs(page, &vertical, text, turn, line));
    runs
}

/// The frame that first turns by `a` and then reads text of direction `b` (in the frame `a`)
/// along +x.
fn compose(a: Turn, b: Turn) -> Turn {
    let q = |t: Turn| match t {
        Turn::None => 0,
        Turn::Up => 1,
        Turn::Half => 2,
        Turn::Down => 3,
    };
    match (q(a) + q(b)) % 4 {
        0 => Turn::None,
        1 => Turn::Up,
        2 => Turn::Half,
        _ => Turn::Down,
    }
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
    let (visible, hidden): (Vec<Gl>, Vec<Gl>) = glyphs.iter().partition(|g| !g.invisible);
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
    items.runs = runs_of(page, &visible, text, turn);
    items.invisible = runs_of(page, &hidden, text, turn);
    for (name, list) in layers.iter().zip(&by_layer) {
        if !list.is_empty() {
            items
                .hidden_layers
                .push((name.clone(), runs_of(page, list, text, turn)));
        }
    }
    if items.turned && !links.is_empty() {
        let k = links.len();
        let s = if k == 1 { "" } else { "s" };
        items.notes.push(Note::Unsupported(format!(
            "{k} URI link{s} on page {page} not attached to their text: the page is read in a \
             turned frame"
        )));
    }
    if !items.turned && !links.is_empty() {
        let cost = items.runs.len().saturating_mul(links.len());
        if cost > LINK_WORK {
            items.links_cut = true;
        } else {
            for r in &mut items.runs {
                let cx = f64::from(r.x + r.w / 2.0) + x0;
                let cy = f64::from(r.y + r.h / 4.0) + y0;
                r.uri = links
                    .iter()
                    .find(|(b, _)| {
                        cx >= f64::from(b[0])
                            && cx <= f64::from(b[2])
                            && cy >= f64::from(b[1])
                            && cy <= f64::from(b[3])
                    })
                    .map(|(_, u)| u.clone());
            }
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
    let found = self::pages(pdf, pages, caps, stop);
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
        let found = pages(&pdf, &all, CAPS, &stop);
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
        let found = pages(&pdf, &[1, 2, 3], caps, &|| false);
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
    fn right_to_left_line_keeps_ligature_glyphs_whole() {
        // "salam" drawn from right to left, with lam and alef in one ligature glyph whose
        // text is already in logical order.
        let visual = ["\u{645}", "\u{644}\u{627}", "\u{633}"];
        assert_eq!(logical_order(&visual), "\u{633}\u{644}\u{627}\u{645}");
        // A Latin word next to it keeps its own order.
        let visual = ["a", "b", " ", "\u{645}", "\u{633}"];
        assert_eq!(logical_order(&visual), "ab \u{633}\u{645}");
    }

    #[test]
    fn ligatures_are_written_out() {
        assert_eq!(clean_bf(BfString::Char('\u{FB01}')).as_deref(), Some("fi"));
        assert_eq!(clean_bf(BfString::Char('\u{FFFD}')), None);
        assert_eq!(clean_bf(BfString::Char('\t')).as_deref(), Some(" "));
    }

    fn frag(text: &str, x: f32, y: f32) -> Frag {
        Frag {
            text: text.into(),
            x,
            y,
            ox: x,
            oy: y,
            end: x + 6.0 * text.chars().count() as f32,
            size: 10.0,
            bold: false,
            italic: false,
            mcid: None,
            chars: text.chars().count(),
            units: vec![0],
        }
    }

    #[test]
    fn two_columns_with_a_title_and_a_footer() {
        let mut frags = vec![frag("A title across both of the columns", 100.0, 800.0)];
        for k in 0..6 {
            let y = 700.0 - 12.0 * k as f32;
            frags.push(frag(&format!("left column line {k} words"), 50.0, y));
            frags.push(frag(&format!("right column line {k} words"), 320.0, y));
        }
        frags.push(frag(
            "A footer note across both of the columns here",
            100.0,
            40.0,
        ));
        let lines = lines_of(&frags, true);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.iter().map(|&i| frags[i].text.clone()).collect())
            .collect();
        assert_eq!(text[0], "A title across both of the columns");
        assert_eq!(text[1], "left column line 0 words");
        assert_eq!(text[6], "left column line 5 words");
        assert_eq!(text[7], "right column line 0 words");
        assert_eq!(text[13], "A footer note across both of the columns here");
    }

    #[test]
    fn label_value_rows_are_read_row_by_row() {
        let mut frags = Vec::new();
        for (k, (label, value)) in [
            ("Name", "Ann"),
            ("Age", "31"),
            ("Town", "Bern"),
            ("Zip", "3000"),
        ]
        .iter()
        .enumerate()
        {
            let y = 700.0 - 14.0 * k as f32;
            frags.push(frag(label, 50.0, y));
            frags.push(frag(value, 300.0, y));
        }
        let lines = lines_of(&frags, true);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], [0, 1]);
    }
}
