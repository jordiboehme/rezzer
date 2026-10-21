// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pages that need vision: a page without a text layer, or whose pictures cover a part of
//! it, gets an asset `page-<N>` the calling agent should look at, and a `Media` block at the
//! reading position of its first picture. With `extract_assets` the page is rendered to PNG
//! with hayro from pdfparts' canonical bytes, one page at a time.
//!
//! A render is bounded before it starts, because hayro cannot be stopped inside one, and an
//! allocation that fails inside it ends the process:
//! - the guard refused the pages whose streams break a limit, and a stream that stops with a
//!   decoding error holds only what the guard measured;
//! - a page whose text was cut by a limit is not rendered;
//! - the page pixmap has at most [`MAX_PIXELS`], and fewer when its PNG would not fit the
//!   render room;
//! - `cost.rs` works out every buffer the renderer would build from the file's values
//!   (pictures, tiling pattern cells, shading textures, soft masks, Type3 glyphs, annotation
//!   appearances) by running the page through the interpreter into a device that draws
//!   nothing, with what vello keeps for each draw call; a page over the render room (a
//!   quarter of the memory limit) is not rendered, and neither is one that reaches a stencil
//!   mask whose size cannot be known;
//! - the time is checked before each render, and a file renders at most [`MAX_RENDERS`]
//!   pages.
//!
//! One render can still run past the time limit: it cannot stop in the middle of a page.

mod cost;
mod headers;
mod inline;
mod stencils;

use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{self, Pixmap, RenderContext};
use hayro::{RenderCache, RenderSettings};
use hayro_interpret::InterpreterSettings;
use hayro_interpret::util::TransformExt;
use kurbo::Affine;
use pdfparts::{PageInfo, Pdf};
use rezzer_limits::Budget;
use rezzer_model::{Anchor, Asset, Barcode, Blob, Block, LossReason, ReadOptions, Severity};
use sha2::{Digest, Sha256};

use crate::Out;
use crate::order::Turn;
use crate::text::PageItems;

/// Pixels per point: 144 dpi.
const SCALE: f64 = 2.0;

/// The most pixels one render may have; a larger page is rendered at a smaller scale.
pub(crate) const MAX_PIXELS: u64 = 16_000_000;

/// The most pages one file renders.
pub(crate) const MAX_RENDERS: usize = 1_000;

/// Pictures that cover at least this part of the page make it need vision.
const PICTURE_SHARE: f32 = 0.02;

/// A picture that repeats on many pages and covers less than this part of the page is page
/// furniture.
const FURNITURE_SHARE: f32 = 0.25;

/// A picture of at least this many square points (1 cm2) is listed even when it does not make
/// its page need vision.
const SMALL_PICTURE: f32 = 803.6;

/// Small pictures whose box the alt text names; past them it says how many more there are.
const SMALL_LISTED: usize = 10;

/// Pictures that cover more than this part of a page without text make it a scan.
const SCAN_SHARE: f32 = 0.5;

/// Why a page needs vision, and the box of its first picture.
pub(crate) struct Vision {
    /// The asset's alt text: `no text layer`, `3 pictures`.
    pub alt: String,
    pub first: Option<[f32; 4]>,
    /// Parts of the page rendered again on their own and read for codes: pictures at a finer
    /// scale than the page's render, codes drawn as paths.
    pub regions: Vec<Region>,
}

/// A part of a page that is rendered on its own to be read for codes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Region {
    /// Its box in the visible page box (`PageItems::images`).
    pub b: [f32; 4],
    pub how: How,
}

/// At what scale a part of a page is rendered to be read for codes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum How {
    /// A picture of this many pixels: at about its own resolution, at least 3 render pixels
    /// to one of its pixels when it is small (a code of one pixel a module), at most 300 dpi.
    Picture(u32, u32),
    /// A code drawn as paths, modules of this many points: whole pixels a module (3), the
    /// region's corner on a whole pixel. 300 dpi when the module size is not known (0).
    Modules(f32),
}

/// The most regions of one page that needs vision.
const PAGE_REGIONS: usize = 8;

/// How far the edges of one picture may move between pages, in points, and as a share of its
/// larger side: a logo scaled a little differently on each page is still the same logo.
const SAME_SPOT: f32 = 2.0;
const SAME_SHARE: f32 = 0.02;

/// Pictures a bucket of [`Spots`] compares with; past them a box starts a spot of its own.
const SPOTS_PER_BUCKET: usize = 64;

/// Whether two picture boxes stand at the same place on their pages.
fn same_spot(a: &[f32; 4], b: &[f32; 4]) -> bool {
    let side = (a[2] - a[0]).abs().max((a[3] - a[1]).abs());
    let near = SAME_SPOT.max(SAME_SHARE * side);
    a.iter().zip(b).all(|(p, q)| (p - q).abs() <= near)
}

/// Pictures at their places, found by their lower left corner on a grid of 4 points. A spot
/// is one picture (by its identity, see `text::picture_id`) at one place: different pictures
/// at one place are different spots.
#[derive(Default)]
pub(crate) struct Spots {
    buckets: HashMap<(i32, i32), Vec<usize>>,
    boxes: Vec<([f32; 4], u64)>,
}

impl Spots {
    fn cell(b: &[f32; 4]) -> (i32, i32) {
        ((b[0] / 4.0).floor() as i32, (b[1] / 4.0).floor() as i32)
    }

    /// The spot of picture `id` at `b`, if one is known.
    fn find(&self, b: &[f32; 4], id: u64) -> Option<usize> {
        let (cx, cy) = Self::cell(b);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = self.buckets.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                if let Some(&k) = list
                    .iter()
                    .take(SPOTS_PER_BUCKET)
                    .find(|&&k| self.boxes[k].1 == id && same_spot(&self.boxes[k].0, b))
                {
                    return Some(k);
                }
            }
        }
        None
    }

    /// The spot of picture `id` at `b`, a new one when none is known.
    fn add(&mut self, b: &[f32; 4], id: u64) -> usize {
        if let Some(k) = self.find(b, id) {
            return k;
        }
        let k = self.boxes.len();
        self.boxes.push((*b, id));
        self.buckets.entry(Self::cell(b)).or_default().push(k);
        k
    }

    pub(crate) fn holds(&self, b: &[f32; 4], id: u64) -> bool {
        self.find(b, id).is_some()
    }
}

/// The same picture drawn at the same place (within [`SAME_SPOT`]) on at least three pages
/// and on at least half of the pages read. When they are small, they are page furniture (a logo in the
/// margin, a strip along the edge), like a running header, and do not make a page need vision.
pub(crate) fn furniture<'a>(pages: impl Iterator<Item = &'a PageItems>) -> Spots {
    let mut all = Spots::default();
    // For each spot: the pages it is on and the last page it was counted for.
    let mut seen: Vec<(usize, usize)> = Vec::new();
    let mut read = 0usize;
    for items in pages {
        read += 1;
        for (i, b) in items.images.iter().enumerate() {
            let k = all.add(b, items.images_id.get(i).copied().unwrap_or(0));
            if k == seen.len() {
                seen.push((0, 0));
            }
            if seen[k].1 != read {
                seen[k] = (seen[k].0 + 1, read);
            }
        }
    }
    let mut out = Spots::default();
    for (k, &(n, _)) in seen.iter().enumerate() {
        if n >= 3 && n * 2 >= read {
            let (b, id) = all.boxes[k];
            let i = out.boxes.len();
            out.boxes.push((b, id));
            out.buckets.entry(Spots::cell(&b)).or_default().push(i);
        }
    }
    out
}

/// What a page shows that an agent may need to look at.
pub(crate) enum Seen {
    /// The whole page needs vision.
    Page(Vision),
    /// Only small pictures, each of at least 1 cm2, in the order the page draws them, and how
    /// many more there are past the cap the caller can take.
    Small(Vec<Small>, usize),
}

/// A small picture on a page that does not need vision as a whole.
pub(crate) struct Small {
    /// Its box in the visible page box (`PageItems::images`), where its crop is cut.
    pub b: [f32; 4],
    /// Its box in the frame of the runs (turned when the page is), which the text names.
    pub framed: [f32; 4],
    /// What the picture is (`PageItems::images_id`): equal pictures share one asset.
    pub id: u64,
    /// A logo: it needs no vision.
    pub logo: bool,
    /// The marked content id it was drawn in (`PageItems::images_mcid`).
    pub mcid: Option<u32>,
    /// How it is rendered again to be read for codes when its crop gives none: a likely code
    /// picture at its own resolution, a code drawn as paths at whole pixels a module.
    pub how: Option<How>,
    /// It is not a picture but a region of paths that may be a code: its asset stays only
    /// when a code is read from it.
    pub vector: bool,
}

/// The top or bottom band of a page, as a share of its height, where a small picture is a
/// logo.
const LOGO_BAND: f32 = 0.15;

/// The corner regions of a page, as a share of its width and height, where a small picture
/// is a logo.
const LOGO_CORNER: f32 = 0.2;

/// Whether box `b` lies wholly in the top or bottom band or in a corner region of a page `w`
/// by `h` points (all in one frame).
fn in_logo_place(b: &[f32; 4], w: f32, h: f32) -> bool {
    let (x0, y0) = (b[0].max(0.0), b[1].max(0.0));
    let (x1, y1) = (b[2].min(w), b[3].min(h));
    let band = y0 >= (1.0 - LOGO_BAND) * h || y1 <= LOGO_BAND * h;
    let side = x1 <= LOGO_CORNER * w || x0 >= (1.0 - LOGO_CORNER) * w;
    let end = y1 <= LOGO_CORNER * h || y0 >= (1.0 - LOGO_CORNER) * h;
    band || (side && end)
}

/// Whether picture `k` of a page looks like a barcode or QR code: near square, 1-bit pixels,
/// at least 21 a side.
fn looks_like_code(items: &PageItems, k: usize) -> bool {
    let b = items.images[k];
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let square = (w - h).abs() <= 0.1 * w.max(h);
    square && items.images_code.get(k).copied().unwrap_or(false)
}

/// The pictures (by `PageItems::images_id`) drawn on every page of a document of two pages
/// or more, anywhere on the page. `count` is the document's page count; when fewer pages
/// were read, no picture is known to be on every page.
pub(crate) fn on_every_page<'a>(
    pages: impl Iterator<Item = &'a PageItems>,
    count: usize,
) -> HashSet<u64> {
    let mut read = 0usize;
    let mut found: Option<HashSet<u64>> = None;
    for items in pages {
        read += 1;
        let here: HashSet<u64> = items.images_id.iter().copied().collect();
        found = Some(match found {
            None => here,
            Some(mut all) => {
                all.retain(|id| here.contains(id));
                all
            }
        });
    }
    match found {
        Some(all) if count >= 2 && read == count => all,
        _ => HashSet::new(),
    }
}

/// What on page `info` with the items read from it needs a look. The whole page needs
/// vision when it has no text layer over a picture (a scan), its glyphs have no Unicode
/// mapping, or it is only an invisible text layer over a picture (an OCR layer); or when its
/// pictures, without the page furniture `skip`, cover at least 2 % of it. Otherwise each of
/// its pictures of at least 1 cm2 is a small picture of its own; one that does not look like
/// a code and lies in the top or bottom band or a corner, or is in `everywhere`, is a logo.
/// At most `cap` small pictures are kept, the first the page draws.
pub(crate) fn vision(
    info: &PageInfo,
    items: &PageItems,
    skip: &Spots,
    everywhere: &HashSet<u64>,
    cap: usize,
) -> Option<Seen> {
    let (width, height) = (info.width, info.height);
    let area = (width * height).max(1.0);
    let visible = |b: &[f32; 4]| {
        let w = (b[2].min(width) - b[0].max(0.0)).max(0.0);
        let h = (b[3].min(height) - b[1].max(0.0)).max(0.0);
        w * h
    };
    let drawn: Vec<usize> = (0..items.images.len())
        .filter(|&k| visible(&items.images[k]) > 0.0)
        .collect();
    let covered = (drawn
        .iter()
        .map(|&k| visible(&items.images[k]))
        .sum::<f32>()
        / area)
        .min(1.0);
    // Furniture is small: a picture over a quarter of the page is content wherever it is.
    let shown_at: Vec<usize> = drawn
        .into_iter()
        .filter(|&k| {
            let b = &items.images[k];
            let id = items.images_id.get(k).copied().unwrap_or(0);
            !(skip.holds(b, id) && visible(b) < FURNITURE_SHARE * area)
        })
        .collect();
    let shown: Vec<&[f32; 4]> = shown_at.iter().map(|&k| &items.images[k]).collect();
    let pictured = (shown.iter().map(|b| visible(b)).sum::<f32>() / area).min(1.0);
    let visible_text = items.mapped > 0;
    let invisible_text = !items.hidden_layer.is_empty() || !items.invisible.is_empty();
    let mut words: Vec<String> = Vec::new();
    if !visible_text && !invisible_text && covered > SCAN_SHARE {
        words.push("no text layer".into());
    }
    if items.mapped == 0 && items.unmapped_visible > 0 {
        words.push("glyphs without a Unicode mapping".into());
    }
    if !visible_text && invisible_text && covered > SCAN_SHARE {
        words.push("only an invisible text layer".into());
    }
    let mut first = shown.first().map(|b| **b);
    let large = !shown.is_empty() && pictured >= PICTURE_SHARE;
    if large {
        let k = shown.len();
        words.push(format!(
            "{k} {}",
            if k == 1 { "picture" } else { "pictures" }
        ));
    }
    // Pictures too small to make the page need vision are still not dropped: each one of at
    // least 1 cm2 is named with its box.
    let small: Vec<usize> = if large {
        Vec::new()
    } else {
        shown_at
            .iter()
            .copied()
            .filter(|&k| visible(&items.images[k]) >= SMALL_PICTURE)
            .collect()
    };
    let vectors: Vec<&crate::codes::VectorCode> =
        items.codes.iter().filter(|c| visible(&c.b) > 0.0).collect();
    if !words.is_empty() {
        // The page is rendered whole: its small pictures are named in its alt text and not
        // cut out again.
        if let Some(&k) = small.first() {
            first = Some(items.images[k]);
            words.push(small_pictures(items, &small));
        }
        // Codes drawn as paths first, then pictures that look like codes, then the largest.
        let mut regions: Vec<Region> = vectors
            .iter()
            .map(|c| Region {
                b: c.b,
                how: How::Modules(c.module),
            })
            .collect();
        let mut pictures: Vec<usize> = shown_at.clone();
        pictures.sort_by(|&a, &b| {
            looks_like_code(items, b)
                .cmp(&looks_like_code(items, a))
                .then(visible(&items.images[b]).total_cmp(&visible(&items.images[a])))
        });
        regions.extend(pictures.into_iter().filter_map(|k| {
            let (pw, ph) = items.images_px.get(k).copied()?;
            Some(Region {
                b: items.images[k],
                how: How::Picture(pw, ph),
            })
        }));
        regions.truncate(PAGE_REGIONS);
        return Some(Seen::Page(Vision {
            alt: words.join(", "),
            first,
            regions,
        }));
    }
    if small.is_empty() && vectors.is_empty() {
        return None;
    }
    // Past `cap` the work for each picture is more than the caller may take.
    let more = (small.len() + vectors.len()).saturating_sub(cap);
    let mut small = small;
    small.truncate(cap);
    let (fw, fh) = items.turn.size(width, height);
    let mut list: Vec<Small> = small
        .into_iter()
        .map(|k| {
            let b = items.images[k];
            let framed = items.turn.place_box(b, width, height);
            let id = items.images_id.get(k).copied().unwrap_or(0);
            let code = looks_like_code(items, k);
            let logo = !code && (in_logo_place(&framed, fw, fh) || everywhere.contains(&id));
            let how = items
                .images_px
                .get(k)
                .filter(|_| code)
                .map(|&(pw, ph)| How::Picture(pw, ph));
            Small {
                b,
                framed,
                id,
                logo,
                mcid: items.images_mcid.get(k).copied().flatten(),
                how,
                vector: false,
            }
        })
        .collect();
    let room = cap.saturating_sub(list.len());
    list.extend(vectors.into_iter().take(room).map(|c| Small {
        b: c.b,
        framed: items.turn.place_box(c.b, width, height),
        id: c.id,
        logo: false,
        mcid: None,
        how: Some(How::Modules(c.module)),
        vector: true,
    }));
    if list.is_empty() {
        return None;
    }
    Some(Seen::Small(list, more))
}

/// The alt text for small pictures `small` of a page that needs vision as a whole: how many,
/// and the box of each of the first [`SMALL_LISTED`] in points. Codes read from them are
/// listed with the page's asset.
fn small_pictures(items: &PageItems, small: &[usize]) -> String {
    let n = small.len();
    let mut boxes: Vec<String> = small
        .iter()
        .take(SMALL_LISTED)
        .map(|&k| box_words(&items.images[k]))
        .collect();
    if n > SMALL_LISTED {
        boxes.push(format!("{} more", n - SMALL_LISTED));
    }
    format!(
        "{n} small {} ({})",
        if n == 1 { "picture" } else { "pictures" },
        boxes.join("; ")
    )
}

/// What a picture that looks like a code but gives none is called, after its box.
const UNREAD_WORDS: &str = ", barcode or QR code found, not decoded";

/// `box 40, 760 to 100, 800`: a box in whole points.
fn box_words(b: &[f32; 4]) -> String {
    let r = |v: f32| v.round() as i64;
    format!("box {}, {} to {}, {}", r(b[0]), r(b[1]), r(b[2]), r(b[3]))
}

/// What a small picture is called before it is read for codes: a code drawn as paths, a
/// logo, or a small picture (which needs vision).
fn small_name(logo: bool, vector: bool) -> &'static str {
    if vector {
        "barcode or QR code"
    } else if logo {
        "logo or small picture"
    } else {
        "small picture"
    }
}

/// The words for one place of a small picture: `small picture, box 40, 760 to 100, 800`.
pub(crate) fn place_alt(s: &Small) -> String {
    format!("{}, {}", small_name(s.logo, s.vector), box_words(&s.framed))
}

/// The words of one place `alt` (from [`place_alt`]) once its picture was read for codes:
/// named after the codes read, or marked as a code not decoded.
fn reread_place(alt: &str, read: &Reread) -> String {
    let rest = alt.find(", box ").map_or(alt, |i| &alt[i + 2..]);
    match read {
        Reread::Codes(name) => format!("{name}, {rest}"),
        Reread::Unread => format!("{alt}{UNREAD_WORDS}"),
    }
}

/// What reading a small picture for codes changed.
enum Reread {
    /// The kinds of codes read: `QR Code`, `QR Code and Code 128`.
    Codes(String),
    /// It looks like a code, but none was read.
    Unread,
}

/// The places of one small picture asset: the page and box of each of the first
/// [`SMALL_LISTED`], the pages it is on (at most [`SMALL_LISTED`] named), and how many places
/// there are.
#[derive(Default)]
pub(crate) struct Places {
    listed: Vec<(u32, [f32; 4])>,
    pages: Vec<u32>,
    more_pages: usize,
    count: usize,
    /// The last page a place was added on: places come in page order.
    last: u32,
    /// Every place is a logo: the asset needs no vision.
    pub logo: bool,
    /// It is a region of paths, not a picture.
    vector: bool,
    /// One of its places looks like a code.
    hint: bool,
    /// What reading it for codes changed.
    read: Option<Reread>,
}

impl Places {
    /// The places of the small picture `s`, first seen on page `n`.
    pub(crate) fn new(n: u32, s: &Small) -> Places {
        let mut p = Places {
            logo: true,
            vector: s.vector,
            ..Places::default()
        };
        p.add(n, s);
        p
    }

    /// Adds the place of `s` on page `n`. One place that is not a logo makes the asset need
    /// vision.
    pub(crate) fn add(&mut self, n: u32, s: &Small) {
        if self.listed.len() < SMALL_LISTED {
            self.listed.push((n, s.framed));
        }
        if n != self.last {
            if self.pages.len() < SMALL_LISTED {
                self.pages.push(n);
            } else {
                self.more_pages += 1;
            }
            self.last = n;
        }
        self.count += 1;
        self.logo &= s.logo;
        self.hint |= s.how.is_some();
    }

    /// The asset's alt text: the place's words for one place; for more, the pages and the
    /// box of each place listed, then how many more places there are.
    pub(crate) fn alt(&self) -> String {
        let name = match &self.read {
            Some(Reread::Codes(name)) => name.as_str(),
            _ => small_name(self.logo, self.vector),
        };
        let code = match self.read {
            Some(Reread::Unread) => UNREAD_WORDS,
            _ => "",
        };
        if self.count == 1 {
            let b = self
                .listed
                .first()
                .map(|(_, b)| box_words(b))
                .unwrap_or_default();
            return format!("{name}, {b}{code}");
        }
        let mut pages: Vec<String> = self.pages.iter().map(u32::to_string).collect();
        if self.more_pages > 0 {
            pages.push(format!("{} more", self.more_pages));
        }
        let mut places: Vec<String> = self
            .listed
            .iter()
            .map(|(n, b)| format!("{} on page {n}", box_words(b)))
            .collect();
        if self.count > self.listed.len() {
            let more = self.count - self.listed.len();
            places.push(format!(
                "{more} more {}",
                if more == 1 { "place" } else { "places" }
            ));
        }
        let word = if self.pages.len() + self.more_pages == 1 {
            "page"
        } else {
            "pages"
        };
        format!(
            "{name}{code}, {} places on {word} {}: {}",
            self.count,
            pages.join(", "),
            places.join("; ")
        )
    }
}

/// The top middle of the first picture in the frame of the runs (`turn` of a page `width`
/// by `height`), where the run nearest to it is looked for.
pub(crate) fn picture_top(v: &Vision, turn: Turn, width: f32, height: f32) -> Option<(f32, f32)> {
    let b = turn.place_box(v.first?, width, height);
    Some(((b[0] + b[2]) / 2.0, b[1].max(b[3])))
}

/// The anchor of a block.
fn anchor_of(block: &Block) -> Option<&Anchor> {
    match block {
        Block::Section { anchor, .. }
        | Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Table { anchor, .. }
        | Block::Code { anchor, .. }
        | Block::Quote { anchor, .. }
        | Block::Math { anchor, .. }
        | Block::Media { anchor, .. }
        | Block::Callout { anchor, .. }
        | Block::Rule { anchor, .. }
        | Block::Task { anchor, .. } => anchor.as_ref(),
    }
}

/// The index of the first block with each anchor.
pub(crate) fn anchor_index(blocks: &[Block]) -> HashMap<Anchor, usize> {
    let mut out = HashMap::new();
    for (i, b) in blocks.iter().enumerate() {
        if let Some(a) = anchor_of(b) {
            out.entry(a.clone()).or_insert(i);
        }
    }
    out
}

/// Where a picture goes among `len` blocks whose anchors `anchored` knows: after the block
/// anchored `at` when the run found there stands above the picture, else before it. Without
/// such a block it goes at the end.
pub(crate) fn position(
    anchored: &HashMap<Anchor, usize>,
    len: usize,
    at: Option<&Anchor>,
    after: bool,
) -> usize {
    match at.and_then(|a| anchored.get(a)) {
        Some(&i) if after => i + 1,
        Some(&i) => i,
        None => len,
    }
}

/// `blocks` with `media` put in, each before the block at its index (at the end past the
/// last), in one pass. `media` is sorted by index; pictures at one index keep their order.
pub(crate) fn merge(blocks: Vec<Block>, media: Vec<(usize, Block)>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len() + media.len());
    let mut media = media.into_iter().peekable();
    for (i, b) in blocks.into_iter().enumerate() {
        while let Some((_, m)) = media.next_if(|(at, _)| *at <= i) {
            out.push(m);
        }
        out.push(b);
    }
    out.extend(media.map(|(_, m)| m));
    out
}

/// A page to render: for its own asset when it needs vision, or to cut its small pictures
/// out. Also whether a limit cut its text, and whether its text came from the fallback
/// parser (its content is then missing in the canonical copy).
pub(crate) struct Need {
    pub page: u32,
    /// The index of the page's asset in the document, when the whole page needs vision.
    pub whole: Option<usize>,
    /// The small pictures first seen on this page, in reading order.
    pub crops: Vec<Crop>,
    /// The lower left corner of the visible page box in user space (`PageItems::origin`).
    pub origin: (f32, f32),
    pub cut: bool,
    pub fallback: bool,
    /// Parts of a page that needs vision as a whole, rendered again to be read for codes.
    pub regions: Vec<Region>,
    /// How the frame of the page's text is turned, and the size of the visible page box:
    /// the box of a code read from the page is given in that frame.
    pub turn: Turn,
    pub size: (f32, f32),
    /// The page draws few enough pictures to be rendered only to be read for codes, without
    /// files: at most [`SCAN_PICTURES`].
    pub light: bool,
}

/// The most pictures a page may draw to be rendered only to be read for codes.
pub(crate) const SCAN_PICTURES: usize = 1_000;

/// A small picture to cut out of its page's render.
pub(crate) struct Crop {
    /// The index of its asset in the document.
    pub asset: usize,
    /// Its box in the visible page box.
    pub b: [f32; 4],
    /// How it is rendered again when its crop gives no code (`Small::how`).
    pub how: Option<How>,
}

/// The asset of page `n`, listed without a file until it is rendered.
pub(crate) fn asset(n: u32, alt: String, caption: Option<String>) -> Asset {
    picture_asset(format!("page-{n}"), alt, caption, true)
}

/// A picture asset, listed without a file until it is rendered.
pub(crate) fn picture_asset(
    id: String,
    alt: String,
    caption: Option<String>,
    needs_vision: bool,
) -> Asset {
    Asset {
        id,
        path: None,
        sha256: String::new(),
        mime: "image/png".into(),
        caption,
        alt: Some(alt),
        needs_vision,
        codes: Vec::new(),
        code_unread: false,
    }
}

/// The margin around a small picture's crop, in pixels.
pub(crate) const CROP_MARGIN: u32 = 8;

/// What was not written because a limit was reached: the page it began on, why, and how many
/// pages and small pictures.
struct LeftOut {
    first: u32,
    /// What the first one left out is: `page 3 is not rendered`.
    what: String,
    why: String,
    pages: usize,
    pictures: usize,
    /// Files are written; else pages are rendered only to be read for codes.
    files: bool,
}

impl LeftOut {
    fn page(n: u32, why: String, files: bool) -> LeftOut {
        LeftOut {
            first: n,
            what: format!("page {n} {}", not_done(files)),
            why,
            pages: 1,
            pictures: 0,
            files,
        }
    }

    /// `pictures` small pictures, the first being `id` on page `n`.
    fn pictures(n: u32, id: &str, pictures: usize, why: String, files: bool) -> LeftOut {
        LeftOut {
            first: n,
            what: if files {
                format!("picture {id} is not written")
            } else {
                format!("picture {id} is not scanned for barcodes")
            },
            why,
            pages: 0,
            pictures,
            files,
        }
    }

    fn words(&self) -> String {
        let (n, why) = (self.first, &self.why);
        let s = |k: usize| if k == 1 { "" } else { "s" };
        let done = if self.files {
            "written"
        } else {
            "scanned for barcodes"
        };
        let pages = if self.files {
            "rendered"
        } else {
            "scanned for barcodes"
        };
        match (self.pages, self.pictures) {
            (1, 0) | (0, 1) => format!("{}: {why}", self.what),
            (k, 0) => {
                format!("{k} pages that need vision are not {pages}, from page {n} on: {why}")
            }
            (0, k) => format!("{k} small pictures are not {done}, from page {n} on: {why}"),
            (k, m) => format!(
                "{k} page{} that need vision and {m} small picture{} are not {done}, from page \
                 {n} on: {why}",
                s(k),
                s(m)
            ),
        }
    }
}

/// What a page that is not rendered is called in a loss: with files the page is not
/// rendered; without, it is only not read for codes.
fn not_done(files: bool) -> &'static str {
    if files {
        "is not rendered"
    } else {
        "is not scanned for barcodes"
    }
}

/// The small pictures of `need` not handled on an earlier page and not counted in a loss: an
/// asset shared with earlier pages is cut from the first page that renders.
fn pending<'n>(
    need: &'n Need,
    counted: &HashSet<usize>,
    handled: &HashSet<usize>,
) -> Vec<&'n Crop> {
    need.crops
        .iter()
        .filter(|c| !handled.contains(&c.asset) && !counted.contains(&c.asset))
        .collect()
}

/// Renders the pages of `needs`: a page that needs vision into a blob of its asset (with
/// `extract_assets`), a page with small pictures into one blob for each picture first seen
/// there, cut from the render. Each render is read for barcodes and QR codes on its pixels,
/// with or without `extract_assets`, and so are parts of it rendered again at a finer scale.
/// One render buffer lives at a time, besides a part rendered again. What is not rendered
/// gets a loss: one per page for a page cap or a render that failed, one for all that is left
/// past the render count, the blob room or the time. Returns what the scans read.
pub(crate) fn render_pages(
    pdf: &Pdf,
    needs: &[Need],
    opts: &ReadOptions,
    budget: &Budget,
    out: &mut Out,
) -> Scans {
    let files = opts.extract_assets;
    let mut scans = Scans::new(budget);
    if needs.is_empty() {
        return scans;
    }
    // Without files a page is rendered only to be read for codes: what is not is a gap in
    // that reading, not in the text.
    let severity = if files {
        Severity::Warning
    } else {
        Severity::Info
    };
    // The renderer opens the file again and holds its own page list while it renders.
    let Some(_list) = crate::text::PageList::of(pdf) else {
        let why = format!(
            "the renderer's list of the {} pages would take about {} bytes, more than the \
             object limit leaves after the objects of the file",
            pdf.page_count(),
            crate::text::page_list_bytes(pdf.page_count())
        );
        for need in needs {
            out.loss(
                format!("p{}", need.page),
                format!("page {} {}: {why}", need.page, not_done(files)),
                LossReason::Limit,
                severity,
            );
        }
        return scans;
    };
    let opened = panic::catch_unwind(AssertUnwindSafe(|| {
        hayro_syntax::Pdf::new(pdf.canonical_shared())
    }));
    let doc = match opened {
        Ok(Ok(doc)) if doc.pages().len() == pdf.page_count() as usize => doc,
        found => {
            let why = match found {
                Ok(Ok(doc)) => format!(
                    "the renderer finds {} pages where the object layer finds {}",
                    doc.pages().len(),
                    pdf.page_count()
                ),
                Ok(Err(e)) => format!("the renderer cannot open the file ({e:?})"),
                Err(_) => "the renderer stopped with an internal error".to_string(),
            };
            for need in needs {
                out.loss(
                    format!("p{}", need.page),
                    format!("page {} {}: {why}", need.page, not_done(files)),
                    LossReason::Damaged,
                    severity,
                );
            }
            return scans;
        }
    };
    let limits = crate::open::limits(budget);
    let stop = || budget.check_time().is_err();
    let sizes = |n: u32, g: u16| pdf.unpacked_size(n, g);
    let facts = cost::Facts {
        sizes: &sizes,
        limits: &limits,
        stop: &stop,
    };
    let room = budget.limits().max_memory_bytes / 4;
    let mut pages = Pages {
        doc: &doc,
        budget,
        room,
        facts: &facts,
        seen: HashSet::new(),
        clips: 0,
    };
    // What is left out for the render count, the blob room or the time, and the small
    // pictures it counts.
    let mut left_out: Option<LeftOut> = None;
    let mut counted: HashSet<usize> = HashSet::new();
    // The small pictures cut out (with files) or read for codes (without).
    let mut handled: HashSet<usize> = HashSet::new();
    // Why each page with small pictures was not rendered.
    let mut failed: HashMap<u32, LossReason> = HashMap::new();
    let mut rendered = 0;
    for need in needs {
        let n = need.page;
        // A page with small pictures only is rendered for those not handled yet.
        let todo = pending(need, &counted, &handled);
        if need.whole.is_none() && todo.is_empty() {
            continue;
        }
        // Without files, a page is rendered only to be read for codes: while there is time
        // for that, and when it draws few enough pictures to render fast.
        if !files {
            if !need.light {
                continue;
            }
            if !scans.stopped() && scans.past() {
                scans.halt(n, "barcode scan stopped: time limit".into());
                continue;
            }
            if scans.stopped() {
                scans.skip(n, todo.len().max(1));
                continue;
            }
        }
        let mut fail = |why: LossReason, what: String, out: &mut Out| {
            out.loss(
                format!("p{n}"),
                format!("page {n} {}: {what}", not_done(files)),
                why,
                severity,
            );
            failed.insert(n, why);
        };
        if let Some(left) = &mut left_out {
            match need.whole {
                Some(_) => left.pages += 1,
                None => {
                    left.pictures += todo.len();
                    counted.extend(todo.iter().map(|c| c.asset));
                }
            }
            continue;
        }
        let first_out = |why: String, counted: &mut HashSet<usize>| match need.whole {
            Some(_) => LeftOut::page(n, why, files),
            None => {
                counted.extend(todo.iter().map(|c| c.asset));
                let mut left = LeftOut::page(n, why, files);
                left.pages = 0;
                left.pictures = todo.len();
                left
            }
        };
        if rendered == MAX_RENDERS {
            left_out = Some(first_out(
                format!("a file renders at most {MAX_RENDERS} pages"),
                &mut counted,
            ));
            continue;
        }
        if need.fallback {
            fail(
                LossReason::Unsupported,
                "its text was read with the fallback parser, and the renderer reads only the \
                 object layer's copy, where its content is missing"
                    .into(),
                out,
            );
            continue;
        }
        if need.cut {
            fail(
                LossReason::Limit,
                "a limit cut its text, so its content is more than a render may take".into(),
                out,
            );
            continue;
        }
        let page = match pages.page(n) {
            Ok(page) => page,
            Err(Failed::Limit(why)) => {
                left_out = Some(first_out(why, &mut counted));
                continue;
            }
            Err(Failed::Room(why)) => {
                fail(LossReason::Limit, why, out);
                continue;
            }
            Err(Failed::Damaged(why)) => {
                fail(LossReason::Damaged, why, out);
                continue;
            }
        };
        rendered += 1;
        match need.whole {
            Some(asset) => {
                scans.read(n, asset, &page, Some(need), budget);
                if files {
                    match encode(page.pixmap) {
                        Ok(png) => {
                            if let Err(why) = keep(png, asset, out) {
                                left_out = Some(LeftOut::page(n, why, files));
                            }
                        }
                        Err(why) => fail(LossReason::Damaged, why, out),
                    }
                } else {
                    drop(page);
                }
                for region in &need.regions {
                    if scans.stopped() {
                        break;
                    }
                    if let Some(r) = pages.region(n, need, region, 0, &mut scans) {
                        scans.read(n, asset, &r, Some(need), budget);
                    }
                }
            }
            None => {
                let mut cut = Cut {
                    need,
                    todo: &todo,
                    files,
                    handled: &mut handled,
                };
                left_out = cut.run(page, &mut pages, &mut scans, out);
                if left_out.is_some() {
                    counted.extend(
                        todo.iter()
                            .filter(|c| !handled.contains(&c.asset))
                            .map(|c| c.asset),
                    );
                }
            }
        }
    }
    if let Some(left) = left_out {
        out.loss(
            format!("p{}", left.first),
            left.words(),
            LossReason::Limit,
            severity,
        );
    }
    if files {
        unwritten(needs, &counted, &failed, out);
    }
    scans
}

/// Small pictures most named in the loss for those that have no file.
const UNWRITTEN_NAMED: usize = 10;

/// Says which small pictures have no file because no page they stand on was rendered (each
/// page already says why): one loss for each reason, naming the first pictures with their
/// pages. Those a limit loss already counts are left out.
fn unwritten(
    needs: &[Need],
    counted: &HashSet<usize>,
    failed: &HashMap<u32, LossReason>,
    out: &mut Out,
) {
    // For each asset without a file: its pages, and why its first page was not rendered.
    let mut lost: Vec<(usize, Vec<u32>, LossReason)> = Vec::new();
    let mut at: HashMap<usize, usize> = HashMap::new();
    for need in needs.iter().filter(|n| n.whole.is_none()) {
        for c in &need.crops {
            if !out.doc.assets[c.asset].sha256.is_empty() || counted.contains(&c.asset) {
                continue;
            }
            let why = failed
                .get(&need.page)
                .copied()
                .unwrap_or(LossReason::Damaged);
            let k = *at.entry(c.asset).or_insert_with(|| {
                lost.push((c.asset, Vec::new(), why));
                lost.len() - 1
            });
            lost[k].1.push(need.page);
        }
    }
    for why in [
        LossReason::Limit,
        LossReason::Damaged,
        LossReason::Unsupported,
    ] {
        let these: Vec<&(usize, Vec<u32>, LossReason)> =
            lost.iter().filter(|(_, _, w)| *w == why).collect();
        let Some(first) = these.first().and_then(|(_, pages, _)| pages.first()) else {
            continue;
        };
        let mut named: Vec<String> = these
            .iter()
            .take(UNWRITTEN_NAMED)
            .map(|(asset, pages, _)| {
                let id = &out.doc.assets[*asset].id;
                let shown: Vec<String> = pages
                    .iter()
                    .take(UNWRITTEN_NAMED)
                    .map(u32::to_string)
                    .collect();
                let more = pages.len().saturating_sub(UNWRITTEN_NAMED);
                let more = if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                };
                let word = if pages.len() == 1 { "page" } else { "pages" };
                format!("{id} on {word} {}{more}", shown.join(", "))
            })
            .collect();
        if these.len() > UNWRITTEN_NAMED {
            named.push(format!("{} more", these.len() - UNWRITTEN_NAMED));
        }
        let what = if these.len() == 1 {
            "1 small picture has no file".to_string()
        } else {
            format!("{} small pictures have no file", these.len())
        };
        out.loss(
            format!("p{first}"),
            format!(
                "{what}: {}; no page it stands on was rendered",
                named.join("; ")
            ),
            why,
            Severity::Warning,
        );
    }
}

/// Keeps `png` as the blob of asset `asset`, if it fits the blob room; else says why not.
fn keep(png: Vec<u8>, asset: usize, out: &mut Out) -> Result<(), String> {
    let size = png.len() as u64;
    if size > out.blob_room {
        return Err(format!(
            "its picture takes {size} bytes, more than the {} bytes left for extracted \
             pictures and attachments (an eighth of the memory limit)",
            out.blob_room
        ));
    }
    out.blob_room -= size;
    let sha256: String = Sha256::digest(&png)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let asset = &mut out.doc.assets[asset];
    asset.sha256 = sha256;
    out.blobs.push(Blob {
        asset: asset.id.clone(),
        bytes: png,
    });
    Ok(())
}

/// The small pictures of one page to cut out of its render.
struct Cut<'c> {
    need: &'c Need,
    todo: &'c [&'c Crop],
    files: bool,
    handled: &'c mut HashSet<usize>,
}

impl Cut<'_> {
    /// Cuts the small pictures out of `page`, one at a time: each crop is read for codes,
    /// rendered again at a finer scale when it gives none and may be a code, and written as a
    /// blob of its asset with files. On a page rendered coarser than 144 dpi (a very large
    /// page) the crop is rendered again at 144 dpi. A crop takes its pixels and its PNG next
    /// to the render, which must fit the render room the page was checked against. A picture
    /// not cut here stays for a later page. Returns what is left out when the time or the
    /// blob room runs out.
    fn run(
        &mut self,
        page: Rendered,
        pages: &mut Pages<'_>,
        scans: &mut Scans,
        out: &mut Out,
    ) -> Option<LeftOut> {
        let (need, todo, files) = (self.need, self.todo, self.files);
        let n = need.page;
        let budget = pages.budget;
        let room = pages.room;
        let pixels = |p: &Rendered| u64::from(p.pixmap.width()) * u64::from(p.pixmap.height());
        let held = pixels(&page) * 4;
        let coarse = page.scale < SCALE * 0.99;
        for (i, c) in todo.iter().enumerate() {
            let id = out.doc.assets[c.asset].id.clone();
            let left = |why: String| Some(LeftOut::pictures(n, &id, todo.len() - i, why, files));
            if let Err(hit) = budget.check_time() {
                return left(hit.to_string());
            }
            let user = [
                c.b[0] + need.origin.0,
                c.b[1] + need.origin.1,
                c.b[2] + need.origin.0,
                c.b[3] + need.origin.1,
            ];
            let Some(rect) = crop_rect(&page, user, CROP_MARGIN) else {
                out.loss(
                    format!("p{n}"),
                    format!(
                        "picture {id} is not cut out of page {n}: its box lies outside the \
                         rendered page"
                    ),
                    LossReason::Damaged,
                    Severity::Warning,
                );
                continue;
            };
            // The render's pixels, the crop's and its PNG (twice its pixels).
            let takes = held + rect.pixels() * cost::PNG_BYTES;
            if takes > room {
                out.loss(
                    format!("p{n}"),
                    format!(
                        "picture {id} is not cut out of page {n}: that would take {takes} bytes, \
                         more than the {room} bytes a render may take (a quarter of the memory \
                         limit)"
                    ),
                    LossReason::Limit,
                    Severity::Warning,
                );
                continue;
            }
            let mut picture = Rendered {
                pixmap: crop(&page, rect),
                transform: Affine::translate((-f64::from(rect.x), -f64::from(rect.y)))
                    * page.transform,
                scale: page.scale,
            };
            if coarse {
                let at = Region {
                    b: c.b,
                    how: How::Picture(0, 0),
                };
                if let Some(fine) = pages.region_at(n, need, &at, SCALE, held) {
                    picture = fine;
                }
            }
            let found = scans.read(n, c.asset, &picture, None, budget);
            if found == Some(0)
                && let Some(how) = c.how
            {
                let at = Region { b: c.b, how };
                let again = pages
                    .region(n, need, &at, held, scans)
                    .and_then(|fine| scans.read(n, c.asset, &fine, None, budget));
                if again == Some(0) || again.is_none() {
                    scans.unread(c.asset);
                }
            }
            if !files {
                self.handled.insert(c.asset);
                continue;
            }
            match encode(picture.pixmap) {
                Ok(png) => {
                    if let Err(why) = keep(png, c.asset, out) {
                        return left(why);
                    }
                    self.handled.insert(c.asset);
                }
                Err(why) => out.loss(
                    format!("p{n}"),
                    format!("picture {id} is not cut out of page {n}: {why}"),
                    LossReason::Damaged,
                    Severity::Warning,
                ),
            }
        }
        None
    }
}

/// Why a render did not happen.
enum Failed {
    /// The time ran out: no later page is rendered either.
    Limit(String),
    /// The page would take more than the render room, or what it takes cannot be known.
    Room(String),
    Damaged(String),
}

/// A rendered page or part of one: its pixels, the transform from user space to them, and
/// its pixels per point.
pub(crate) struct Rendered {
    pub pixmap: Pixmap,
    pub transform: Affine,
    pub scale: f64,
}

/// A rectangle of pixels in a render: left, top, width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelRect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl PixelRect {
    fn pixels(&self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }
}

/// The pixels of `page` under box `b` (points in user space), at the render's own scale,
/// with `margin` pixels on each side, clipped to the page. `None` when the box lies outside
/// the page.
pub(crate) fn crop_rect(page: &Rendered, b: [f32; 4], margin: u32) -> Option<PixelRect> {
    let corners = [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])]
        .map(|(x, y)| page.transform * kurbo::Point::new(f64::from(x), f64::from(y)));
    let xs = corners.map(|p| p.x);
    let ys = corners.map(|p| p.y);
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let m = f64::from(margin);
    let (pw, ph) = (
        f64::from(page.pixmap.width()),
        f64::from(page.pixmap.height()),
    );
    // Rounded to a hundredth first, so a box on whole pixels does not grow by one.
    let snap = |v: f64| (v * 100.0).round() / 100.0;
    let x0 = (snap(min(xs)).floor() - m).clamp(0.0, pw);
    let y0 = (snap(min(ys)).floor() - m).clamp(0.0, ph);
    let x1 = (snap(max(xs)).ceil() + m).clamp(0.0, pw);
    let y1 = (snap(max(ys)).ceil() + m).clamp(0.0, ph);
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    Some(PixelRect {
        x: x0 as u16,
        y: y0 as u16,
        w: (x1 - x0) as u16,
        h: (y1 - y0) as u16,
    })
}

/// The pixels of `rect` of `page`, as a pixmap of their own.
pub(crate) fn crop(page: &Rendered, rect: PixelRect) -> Pixmap {
    let mut out = Pixmap::new(rect.w, rect.h);
    let src = page.pixmap.data();
    let width = usize::from(page.pixmap.width());
    let (x, w) = (usize::from(rect.x), usize::from(rect.w));
    for (row, dst) in out.data_mut().chunks_exact_mut(w).enumerate() {
        let at = (usize::from(rect.y) + row) * width + x;
        dst.copy_from_slice(&src[at..at + w]);
    }
    out.set_may_have_transparency(page.pixmap.may_have_transparency());
    out
}

/// Writes `pixmap` as PNG.
fn encode(pixmap: Pixmap) -> Result<Vec<u8>, String> {
    match panic::catch_unwind(AssertUnwindSafe(|| pixmap.into_png())) {
        Ok(Ok(png)) => Ok(png),
        Ok(Err(e)) => Err(format!("the PNG could not be written ({e})")),
        Err(_) => Err("the PNG writer stopped with an internal error".into()),
    }
}

/// The most parts of pages a file renders again to read codes.
const MAX_REGIONS: usize = 500;

/// The scale at most of a part of a page rendered again: 300 dpi.
const REGION_SCALE: f64 = 300.0 / 72.0;

/// The most pixels of a part of a page rendered again.
const REGION_PIXELS: u64 = 16_000_000;

/// Pixels a module of a code drawn as paths takes when it is rendered again.
const MODULE_PIXELS: f64 = 3.0;

/// The margin around a part of a page rendered again, in pixels: the quiet zone.
const REGION_MARGIN: f64 = 12.0;

/// The renderer on the pages of one file, with what it checked already.
struct Pages<'p> {
    doc: &'p hayro_syntax::Pdf,
    budget: &'p Budget,
    /// What one render may take, in bytes.
    room: u64,
    facts: &'p cost::Facts<'p>,
    /// The content streams and stencils checked for earlier pages.
    seen: HashSet<hayro_syntax::object::ObjectIdentifier>,
    /// Parts of pages rendered again so far.
    clips: usize,
}

impl<'p> Pages<'p> {
    /// Renders page `page` (1-based) at 144 dpi, at a smaller scale when that would pass
    /// [`MAX_PIXELS`] or the PNG would not fit the room.
    fn page(&mut self, page: u32) -> Result<Rendered, Failed> {
        self.budget
            .check_time()
            .map_err(|hit| Failed::Limit(hit.to_string()))?;
        let p = self.find(page)?;
        let (w, h) = p.render_dimensions();
        let (w, h) = (f64::from(w), f64::from(h));
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return Err(Failed::Damaged("the page has no size".into()));
        }
        let (px, py) = pixel_size(w, h, MAX_PIXELS.min(self.room / cost::PNG_BYTES));
        let transform = Affine::scale_non_uniform(f64::from(px) / w, f64::from(py) / h)
            * p.initial_transform(true).to_kurbo();
        let pixmap = self.draw(p, transform, (px, py), 0)?;
        Ok(Rendered {
            pixmap,
            transform,
            scale: f64::from(px) / w,
        })
    }

    fn find(&self, page: u32) -> Result<&'p hayro_syntax::page::Page<'p>, Failed> {
        let doc: &'p hayro_syntax::Pdf = self.doc;
        usize::try_from(page)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| doc.pages().get(i))
            .ok_or_else(|| Failed::Damaged("the renderer does not find it".into()))
    }

    /// Renders `region` of page `n` again on its own to be read for codes, at the scale its
    /// kind asks for; `held` bytes of an earlier render are still alive. `None` when it is not
    /// rendered: the regions cap, a scale no finer than the page's, a picture too large to
    /// scan (`scans` counts it), or a render that does not fit or fails.
    fn region(
        &mut self,
        n: u32,
        need: &Need,
        region: &Region,
        held: u64,
        scans: &mut Scans,
    ) -> Option<Rendered> {
        let (bw, bh) = (
            f64::from(region.b[2] - region.b[0]),
            f64::from(region.b[3] - region.b[1]),
        );
        if !(bw > 0.0 && bh > 0.0) {
            return None;
        }
        let scale = match region.how {
            How::Picture(pw, ph) => {
                if u64::from(pw) * u64::from(ph) > barcodes::Limits::default().max_pixels {
                    scans.too_large(n);
                    return None;
                }
                // Its own pixels per point; a small picture three render pixels to each.
                let own = (f64::from(pw) / bw).max(f64::from(ph) / bh);
                let want = if pw.max(ph) <= 400 {
                    own * MODULE_PIXELS
                } else {
                    own
                };
                want.min(REGION_SCALE)
            }
            How::Modules(m) if m > 0.0 => MODULE_PIXELS / f64::from(m),
            How::Modules(_) => REGION_SCALE,
        };
        // Finer than the page's own render, or there is nothing to gain.
        if scale <= SCALE * 1.01 && matches!(region.how, How::Picture(..)) {
            return None;
        }
        self.region_at(n, need, region, scale, held)
    }

    /// Renders `region` of page `n` at `scale` pixels per point, its top left corner
    /// `REGION_MARGIN` pixels in from the corner of the render (so the modules of a code
    /// drawn at the scale's module size fall on whole pixels).
    fn region_at(
        &mut self,
        n: u32,
        need: &Need,
        region: &Region,
        scale: f64,
        held: u64,
    ) -> Option<Rendered> {
        if self.clips >= MAX_REGIONS || self.budget.check_time().is_err() {
            return None;
        }
        let room = self.room.checked_sub(held)?;
        let p = self.find(n).ok()?;
        let init = p.initial_transform(true).to_kurbo();
        let (ox, oy) = (f64::from(need.origin.0), f64::from(need.origin.1));
        let b = region.b.map(f64::from);
        let corners = [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])]
            .map(|(x, y)| init * kurbo::Point::new(x + ox, y + oy));
        let min_x = corners.iter().map(|c| c.x).fold(f64::INFINITY, f64::min);
        let max_x = corners
            .iter()
            .map(|c| c.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = corners.iter().map(|c| c.y).fold(f64::INFINITY, f64::min);
        let max_y = corners
            .iter()
            .map(|c| c.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let size = |s: f64| {
            (
                ((max_x - min_x) * s + 2.0 * REGION_MARGIN).ceil(),
                ((max_y - min_y) * s + 2.0 * REGION_MARGIN).ceil(),
            )
        };
        let max = REGION_PIXELS.min(room / cost::PNG_BYTES) as f64;
        let mut s = scale;
        let (w, h) = size(s);
        if w * h > max {
            s *= (max / (w * h)).sqrt();
        }
        let (w, h) = size(s);
        if !(w >= 1.0 && h >= 1.0 && w <= f64::from(u16::MAX) && h <= f64::from(u16::MAX)) {
            return None;
        }
        let transform = Affine::translate((REGION_MARGIN - s * min_x, REGION_MARGIN - s * min_y))
            * Affine::scale(s)
            * init;
        self.clips += 1;
        let pixmap = self.draw(p, transform, (w as u16, h as u16), held).ok()?;
        Some(Rendered {
            pixmap,
            transform,
            scale: s,
        })
    }

    /// Draws page `p` through `transform` into a pixmap of `px` by `py` pixels, once what
    /// it takes is known to fit the room less `held` bytes.
    fn draw(
        &mut self,
        p: &hayro_syntax::page::Page<'_>,
        transform: Affine,
        (px, py): (u16, u16),
        held: u64,
    ) -> Result<Pixmap, Failed> {
        let room = self.room.saturating_sub(held);
        let settings = InterpreterSettings::default();
        stencils::page_stencils(p, room, self.facts, &mut self.seen).map_err(Failed::Room)?;
        match cost::estimate(p, transform, (px, py), &settings, room, self.facts) {
            Ok(_) => {}
            Err(cost::Unknown::Over(kind, bytes)) => {
                return Err(Failed::Room(format!(
                    "{} would take {bytes} bytes or more to render, more than the {room} bytes a \
                     render may take (a quarter of the memory limit)",
                    kind.words()
                )));
            }
            Err(cost::Unknown::Unbounded(why)) => return Err(Failed::Room(why)),
            Err(cost::Unknown::Stopped) => {
                return Err(Failed::Limit("the time limit was reached".into()));
            }
        }
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            // A cache of its own, so nothing a page decoded stays for the next.
            let cache = RenderCache::new();
            let mut ctx = RenderContext::new(px, py);
            hayro::render_into(
                p,
                &cache,
                &settings,
                &RenderSettings::default(),
                &mut ctx,
                transform,
            );
            ctx.flush();
            let mut pixmap = Pixmap::new(px, py);
            ctx.render_with(
                &mut pixmap,
                &mut vello_cpu::Resources::default(),
                vello_cpu::RasterizerSettings {
                    target_init: vello_cpu::TargetInit::Clear(WHITE),
                    ..Default::default()
                },
            );
            drop(ctx);
            drop(cache);
            pixmap
        }));
        run.map_err(|_| Failed::Damaged("the renderer stopped with an internal error".into()))
    }
}

/// The pixel size of a page of `w` by `h` points: twice its size, scaled down to at most
/// `max` pixels, each side at least 1 and at most 65,535 pixels.
fn pixel_size(w: f64, h: f64, max: u64) -> (u16, u16) {
    let mut scale = SCALE;
    let pixels = w * scale * h * scale;
    if pixels > max as f64 {
        scale *= (max as f64 / pixels).sqrt();
    }
    let side = |v: f64| (v * scale).floor().clamp(1.0, f64::from(u16::MAX)) as u16;
    (side(w), side(h))
}

/// The most codes kept from one picture, and from one file.
const PICTURE_CODES: usize = 50;
const FILE_CODES: usize = 500;

/// What reading the pictures of one asset for codes found.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub codes: Vec<Barcode>,
    /// A picture that looks like a code was read, and gave none.
    pub unread: bool,
    /// It was read.
    pub read: bool,
}

/// The barcode scans of one read: their bounds, what they found and what they could not do.
pub(crate) struct Scans {
    /// When reading stops for the file.
    until: Instant,
    /// The time one picture may take.
    per_picture: Duration,
    tile_pixels: u64,
    /// Codes kept in the file.
    kept: usize,
    /// What each asset holds.
    pub found: HashMap<usize, Found>,
    /// Why reading stopped, and on which page.
    stop: Option<(u32, String)>,
    /// Pictures and pages not read after the stop.
    skipped: usize,
    /// Pages with a picture that holds more codes than are kept, too large a picture, or an
    /// internal error of the decoder; codes whose payload is only counted.
    capped: Vec<u32>,
    too_large: Vec<u32>,
    internal: Vec<u32>,
    large_payloads: usize,
}

#[cfg(test)]
thread_local! {
    /// Decoder runs in this thread, for tests.
    pub(crate) static SCAN_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Scans {
    /// The scans of a read under `budget`: a quarter of the time limit in all, ending an
    /// eighth of it before the end, and at most 10 seconds a picture.
    fn new(budget: &Budget) -> Scans {
        let max = budget.limits().max_time;
        let left = max.saturating_sub(budget.elapsed()).saturating_sub(max / 8);
        let room = budget.limits().max_memory_bytes / 4;
        Scans {
            until: Instant::now() + (max / 4).min(left),
            per_picture: Duration::from_secs(10).min(max / 12),
            // rxing takes about 30 bytes a pixel of a tile while it reads.
            tile_pixels: 4_000_000u64.min(room / 48).max(10_000),
            kept: 0,
            found: HashMap::new(),
            stop: None,
            skipped: 0,
            capped: Vec::new(),
            too_large: Vec::new(),
            internal: Vec::new(),
            large_payloads: 0,
        }
    }

    fn stopped(&self) -> bool {
        self.stop.is_some()
    }

    /// The time for reading codes is over.
    fn past(&self) -> bool {
        Instant::now() >= self.until
    }

    fn halt(&mut self, n: u32, why: String) {
        self.stop.get_or_insert((n, why));
        self.skipped += 1;
    }

    /// `count` pictures of page `n` are not read: reading stopped.
    fn skip(&mut self, _n: u32, count: usize) {
        self.skipped += count;
    }

    fn too_large(&mut self, n: u32) {
        self.too_large.push(n);
    }

    /// The likely code `asset` gave no code.
    fn unread(&mut self, asset: usize) {
        let f = self.found.entry(asset).or_default();
        if f.codes.is_empty() {
            f.unread = true;
        }
    }

    /// Reads `picture` (of page `n`, for asset `asset`) for codes. With `need`, the picture
    /// is the page or a part of it, and each code gets its box on the page. Returns how many
    /// new codes it gave, `None` when it was not read.
    fn read(
        &mut self,
        n: u32,
        asset: usize,
        picture: &Rendered,
        need: Option<&Need>,
        budget: &Budget,
    ) -> Option<usize> {
        if self.stopped() {
            self.skipped += 1;
            return None;
        }
        if self.kept >= FILE_CODES {
            self.halt(
                n,
                format!("the file holds more than {FILE_CODES} codes, the most that are read"),
            );
            return None;
        }
        let now = Instant::now();
        if now >= self.until || budget.check_time().is_err() {
            self.halt(n, "barcode scan stopped: time limit".into());
            return None;
        }
        let limits = barcodes::Limits {
            time: self.per_picture.min(self.until - now),
            tile_pixels: self.tile_pixels,
            max_codes: PICTURE_CODES,
            ..barcodes::Limits::default()
        };
        let p = &picture.pixmap;
        // The render is cleared to white: every pixel is opaque, so premultiplied is plain.
        let scan = barcodes::Picture {
            pixels: barcodes::Pixels::Rgba(p.data_as_u8_slice()),
            width: u32::from(p.width()),
            height: u32::from(p.height()),
        };
        #[cfg(test)]
        SCAN_RUNS.with(|r| r.set(r.get() + 1));
        self.found.entry(asset).or_default().read = true;
        let codes = match barcodes::Worker::global().decode(&scan, &limits) {
            Ok(s) => {
                if s.capped {
                    self.capped.push(n);
                }
                s.codes
            }
            Err(barcodes::Stop::Time(s)) => {
                self.halt(n, "barcode scan stopped: time limit".into());
                s.codes
            }
            Err(barcodes::Stop::TooLarge { .. }) => {
                self.too_large.push(n);
                return None;
            }
            Err(barcodes::Stop::Internal | barcodes::Stop::BadSize) => {
                self.internal.push(n);
                return None;
            }
        };
        let mut added = 0;
        for c in codes {
            if self.kept >= FILE_CODES {
                self.halt(
                    n,
                    format!("the file holds more than {FILE_CODES} codes, the most that are read"),
                );
                break;
            }
            let mut b = barcode(&c);
            if b.text.is_none() && b.base64.is_none() {
                self.large_payloads += 1;
            }
            b.bounds = need.map(|need| page_box(picture, need, c.bounds));
            let found = self.found.entry(asset).or_default();
            if found.codes.iter().any(|o| same_code(o, &b)) {
                continue;
            }
            let size = b.text.as_ref().map_or(0, String::len)
                + b.base64.as_ref().map_or(0, String::len)
                + b.symbology.len()
                + 64;
            if let Err(hit) = budget
                .add_elements(1)
                .and_then(|()| budget.charge_text(size))
            {
                self.halt(
                    n,
                    format!("the codes read take more than the file may keep: {hit}"),
                );
                break;
            }
            found.codes.push(b);
            found.unread = false;
            self.kept += 1;
            added += 1;
        }
        Some(added)
    }

    /// Says in losses what was not read: one for a stop, one for each kind of other gap.
    pub(crate) fn finish(&self, out: &mut Out) {
        let pages = |list: &[u32]| {
            let mut v: Vec<String> = list.iter().take(10).map(u32::to_string).collect();
            if list.len() > 10 {
                v.push(format!("{} more", list.len() - 10));
            }
            v.join(", ")
        };
        if let Some((n, why)) = &self.stop {
            let k = self.skipped;
            let what = if k == 1 {
                "1 picture or page is".to_string()
            } else {
                format!("{k} pictures or pages are")
            };
            out.loss(
                format!("p{n}"),
                format!("{why}; {what} not read for codes, from page {n} on"),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        if let Some(first) = self.capped.first() {
            out.loss(
                format!("p{first}"),
                format!(
                    "pictures on pages {} hold more than {PICTURE_CODES} codes each; the codes                      past the first {PICTURE_CODES} of a picture are not read",
                    pages(&self.capped)
                ),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        if let Some(first) = self.too_large.first() {
            out.loss(
                format!("p{first}"),
                format!(
                    "pictures on pages {} are too large to scan for codes: more than {} pixels",
                    pages(&self.too_large),
                    barcodes::Limits::default().max_pixels
                ),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        if let Some(first) = self.internal.first() {
            out.loss(
                format!("p{first}"),
                format!(
                    "the barcode reader stopped with an internal error on pages {}",
                    pages(&self.internal)
                ),
                LossReason::Internal,
                Severity::Warning,
            );
        }
        if self.large_payloads > 0 {
            out.loss(
                "document",
                format!(
                    "{} codes hold more than {} bytes; only their size is kept",
                    self.large_payloads,
                    barcodes::Limits::default().max_payload
                ),
                LossReason::Limit,
                Severity::Warning,
            );
        }
    }
}

/// Whether two codes read from one asset are the same code: same kind and payload, and boxes
/// that overlap when both have one.
fn same_code(a: &Barcode, b: &Barcode) -> bool {
    a.symbology == b.symbology
        && a.text == b.text
        && a.base64 == b.base64
        && a.bytes == b.bytes
        && match (a.bounds, b.bounds) {
            (Some(p), Some(q)) => p[0] <= q[2] && q[0] <= p[2] && p[1] <= q[3] && q[1] <= p[3],
            _ => true,
        }
}

/// The model's code for `c`: its payload as text, else as base64, else only its size.
fn barcode(c: &barcodes::Code) -> Barcode {
    use base64::Engine;
    let (text, base64, bytes) = match &c.payload {
        barcodes::Payload::Text(t) => (Some(t.clone()), None, t.len()),
        barcodes::Payload::Bytes(b) => (
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(b)),
            b.len(),
        ),
        barcodes::Payload::TooLarge(n) => (None, None, *n),
    };
    Barcode {
        symbology: c.symbology.name().to_string(),
        text,
        base64,
        bytes: bytes as u64,
        bounds: None,
    }
}

/// The box of pixels `px` of `picture` on its page, in points in the frame of the page's
/// text, rounded to a tenth.
fn page_box(picture: &Rendered, need: &Need, px: [u32; 4]) -> [f32; 4] {
    let back = picture.transform.inverse();
    let corners = [
        (px[0], px[1]),
        (px[2], px[1]),
        (px[0], px[3]),
        (px[2], px[3]),
    ]
    .map(|(x, y)| back * kurbo::Point::new(f64::from(x), f64::from(y)));
    let xs = corners.map(|p| p.x as f32 - need.origin.0);
    let ys = corners.map(|p| p.y as f32 - need.origin.1);
    let min = |v: [f32; 4]| v.into_iter().fold(f32::MAX, f32::min);
    let max = |v: [f32; 4]| v.into_iter().fold(f32::MIN, f32::max);
    let b = [min(xs), min(ys), max(xs), max(ys)];
    let b = need.turn.place_box(b, need.size.0, need.size.1);
    b.map(|v| (v * 10.0).round() / 10.0)
}

/// Puts what the scans read into the model: the codes of each asset; a small picture a code
/// was read from needs no vision and is named after its codes at every place; a likely code
/// that gave none says so; a region of paths no code was read from is taken out with its
/// places and file. `places` are the small picture assets with their places; their alt texts
/// are written here.
pub(crate) fn apply_codes(scans: Option<Scans>, places: Vec<(usize, Places)>, out: &mut Out) {
    let mut found = match scans {
        Some(scans) => {
            scans.finish(out);
            scans.found
        }
        None => HashMap::new(),
    };
    // The new words of each place by asset id, or `None` to take it out.
    let mut places_of: HashMap<String, Option<Reread>> = HashMap::new();
    let mut gone: HashSet<usize> = HashSet::new();
    for (asset, mut p) in places {
        let f = found.remove(&asset).unwrap_or_default();
        let a = &mut out.doc.assets[asset];
        a.needs_vision = !p.logo;
        if !f.codes.is_empty() {
            let mut names: Vec<&str> = Vec::new();
            for c in &f.codes {
                if !names.contains(&c.symbology.as_str()) {
                    names.push(&c.symbology);
                }
            }
            let name = names.join(" and ");
            p.read = Some(Reread::Codes(name.clone()));
            places_of.insert(a.id.clone(), Some(Reread::Codes(name)));
            a.needs_vision = false;
            a.codes = f.codes;
        } else if p.vector {
            places_of.insert(a.id.clone(), None);
            gone.insert(asset);
            continue;
        } else if f.unread || (p.hint && f.read) {
            p.read = Some(Reread::Unread);
            places_of.insert(a.id.clone(), Some(Reread::Unread));
            a.code_unread = true;
        }
        a.alt = Some(p.alt());
    }
    // Pages that need vision as a whole keep it, and list their codes.
    for (asset, f) in found {
        if let Some(a) = out.doc.assets.get_mut(asset) {
            a.codes = f.codes;
        }
    }
    if places_of.is_empty() {
        return;
    }
    fn walk(blocks: &mut Vec<Block>, places: &HashMap<String, Option<Reread>>) {
        blocks.retain(|b| match b {
            Block::Media { asset, .. } => !matches!(places.get(asset), Some(None)),
            _ => true,
        });
        for b in blocks.iter_mut() {
            match b {
                Block::Media { asset, alt, .. } => {
                    if let (Some(Some(read)), Some(words)) = (places.get(asset), alt.as_mut()) {
                        *words = reread_place(words, read);
                    }
                }
                Block::Section { blocks, .. }
                | Block::Quote { blocks, .. }
                | Block::Callout { blocks, .. } => walk(blocks, places),
                Block::List { items, .. } => {
                    for item in items {
                        walk(item, places);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&mut out.doc.body, &places_of);
    if !gone.is_empty() {
        let ids: HashSet<String> = gone.iter().map(|&k| out.doc.assets[k].id.clone()).collect();
        let mut k = 0;
        out.doc.assets.retain(|_| {
            k += 1;
            !gone.contains(&(k - 1))
        });
        let mut freed = 0;
        out.blobs.retain(|b| {
            let keep = !ids.contains(&b.asset);
            if !keep {
                freed += b.bytes.len() as u64;
            }
            keep
        });
        out.blob_room += freed;
    }
}

/// A Figure of the tag tree with an alt text: the text, and the marked content ids of its
/// own content on its page.
pub(crate) struct Figure {
    pub alt: String,
    pub mcids: Vec<u32>,
}

/// The Figures with an alt text of each page, in the order of the tag tree.
pub(crate) fn page_figures(pdf: &Pdf) -> HashMap<u32, Vec<Figure>> {
    let mut out: HashMap<u32, Vec<Figure>> = HashMap::new();
    if let Some(tree) = pdf.struct_tree() {
        for e in &tree.elems {
            if let (true, Some(page), Some(alt)) = (e.role == "Figure", e.page, e.alt.as_ref())
                && !alt.trim().is_empty()
            {
                out.entry(page).or_default().push(Figure {
                    alt: alt.clone(),
                    mcids: e.mcids.clone(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_cut_short_never_finds_a_picture_on_every_page() {
        let page = |n: u32| PageItems {
            page: n,
            images: vec![[0.0, 0.0, 30.0, 30.0]],
            images_id: vec![7],
            ..PageItems::default()
        };
        let read = [page(1), page(2)];
        assert_eq!(on_every_page(read.iter(), 2), HashSet::from([7]));
        // Two pages read of three: the third may not have it.
        assert!(on_every_page(read.iter(), 3).is_empty());
        // One page is not every page.
        assert!(on_every_page(read[..1].iter(), 1).is_empty());
    }

    #[test]
    fn pages_stay_under_the_pixel_cap() {
        assert_eq!(pixel_size(595.0, 842.0, MAX_PIXELS), (1190, 1684));
        let (w, h) = pixel_size(200_000.0, 200_000.0, MAX_PIXELS);
        assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS);
        assert_eq!((w, h), (4000, 4000));
        // A long thin page keeps each side inside a pixmap's bounds.
        let (w, h) = pixel_size(1e9, 0.001, MAX_PIXELS);
        assert_eq!((w, h), (u16::MAX, 1));
    }
}
