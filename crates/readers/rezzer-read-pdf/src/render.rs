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

use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{self, Pixmap, RenderContext};
use hayro::{RenderCache, RenderSettings};
use hayro_interpret::InterpreterSettings;
use hayro_interpret::util::TransformExt;
use kurbo::Affine;
use pdfparts::{PageInfo, Pdf};
use rezzer_limits::Budget;
use rezzer_model::{Anchor, Asset, Blob, Block, LossReason, ReadOptions, Severity};
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
}

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
    /// Only small pictures, each of at least 1 cm2, in the order the page draws them.
    Small(Vec<Small>),
}

/// A small picture on a page that does not need vision as a whole.
pub(crate) struct Small {
    /// Its box in the visible page box (`PageItems::images`), where its crop is cut.
    pub b: [f32; 4],
    /// Its box in the frame of the runs (turned when the page is), which the text names.
    pub framed: [f32; 4],
    /// What the picture is (`PageItems::images_id`): equal pictures share one asset.
    pub id: u64,
    /// It looks like a barcode or QR code.
    pub code: bool,
    /// A logo: it needs no vision.
    pub logo: bool,
    /// The marked content id it was drawn in (`PageItems::images_mcid`).
    pub mcid: Option<u32>,
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
pub(crate) fn vision(
    info: &PageInfo,
    items: &PageItems,
    skip: &Spots,
    everywhere: &HashSet<u64>,
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
    if !words.is_empty() {
        // The page is rendered whole: its small pictures are named in its alt text and not
        // cut out again.
        if let Some(&k) = small.first() {
            first = Some(items.images[k]);
            words.push(small_pictures(items, &small));
        }
        return Some(Seen::Page(Vision {
            alt: words.join(", "),
            first,
        }));
    }
    if small.is_empty() {
        return None;
    }
    let (fw, fh) = items.turn.size(width, height);
    let list = small
        .into_iter()
        .map(|k| {
            let b = items.images[k];
            let framed = items.turn.place_box(b, width, height);
            let id = items.images_id.get(k).copied().unwrap_or(0);
            let code = looks_like_code(items, k);
            let logo = !code && (in_logo_place(&framed, fw, fh) || everywhere.contains(&id));
            Small {
                b,
                framed,
                id,
                code,
                logo,
                mcid: items.images_mcid.get(k).copied().flatten(),
            }
        })
        .collect();
    Some(Seen::Small(list))
}

/// The alt text for small pictures `small` of a page that needs vision as a whole: how many,
/// and the box of each of the first [`SMALL_LISTED`] in points. A near-square one of 1-bit
/// pixels, at least 21 a side, is named a likely code.
fn small_pictures(items: &PageItems, small: &[usize]) -> String {
    let n = small.len();
    let mut boxes: Vec<String> = small
        .iter()
        .take(SMALL_LISTED)
        .map(|&k| {
            let mut s = box_words(&items.images[k]);
            if looks_like_code(items, k) {
                s.push_str(CODE_WORDS);
            }
            s
        })
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

/// What a likely code is called.
const CODE_WORDS: &str = ", likely a barcode or QR code, not decoded";

/// `box 40, 760 to 100, 800`: a box in whole points.
fn box_words(b: &[f32; 4]) -> String {
    let r = |v: f32| v.round() as i64;
    format!("box {}, {} to {}, {}", r(b[0]), r(b[1]), r(b[2]), r(b[3]))
}

/// What a small picture is called: a logo, or a small picture (which needs vision).
fn small_name(logo: bool) -> &'static str {
    if logo {
        "logo or small picture"
    } else {
        "small picture"
    }
}

/// The words for one place of a small picture: `small picture, box 40, 760 to 100, 800`.
pub(crate) fn place_alt(s: &Small) -> String {
    let mut out = format!("{}, {}", small_name(s.logo), box_words(&s.framed));
    if s.code {
        out.push_str(CODE_WORDS);
    }
    out
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
    code: bool,
}

impl Places {
    /// The places of the small picture `s`, first seen on page `n`.
    pub(crate) fn new(n: u32, s: &Small) -> Places {
        let mut p = Places {
            logo: true,
            code: s.code,
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
        self.code |= s.code;
    }

    /// The asset's alt text: the place's words for one place; for more, the pages and the
    /// box of each place listed, then how many more places there are.
    pub(crate) fn alt(&self) -> String {
        let name = small_name(self.logo);
        let code = if self.code { CODE_WORDS } else { "" };
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
}

/// A small picture to cut out of its page's render.
pub(crate) struct Crop {
    /// The index of its asset in the document.
    pub asset: usize,
    /// Its box in the visible page box.
    pub b: [f32; 4],
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
}

impl LeftOut {
    fn page(n: u32, why: String) -> LeftOut {
        LeftOut {
            first: n,
            what: format!("page {n} is not rendered"),
            why,
            pages: 1,
            pictures: 0,
        }
    }

    /// `pictures` small pictures, the first being `id` on page `n`.
    fn pictures(n: u32, id: &str, pictures: usize, why: String) -> LeftOut {
        LeftOut {
            first: n,
            what: format!("picture {id} is not written"),
            why,
            pages: 0,
            pictures,
        }
    }

    fn words(&self) -> String {
        let (n, why) = (self.first, &self.why);
        let s = |k: usize| if k == 1 { "" } else { "s" };
        match (self.pages, self.pictures) {
            (1, 0) | (0, 1) => format!("{}: {why}", self.what),
            (k, 0) => {
                format!("{k} pages that need vision are not rendered, from page {n} on: {why}")
            }
            (0, k) => format!("{k} small pictures are not written, from page {n} on: {why}"),
            (k, m) => format!(
                "{k} page{} that need vision and {m} small picture{} are not written, from page \
                 {n} on: {why}",
                s(k),
                s(m)
            ),
        }
    }
}

/// The small pictures of `need` that have no file yet and are not counted in a loss: an
/// asset shared with earlier pages is cut from the first page that renders.
fn pending<'n>(need: &'n Need, counted: &HashSet<usize>, out: &Out) -> Vec<&'n Crop> {
    need.crops
        .iter()
        .filter(|c| out.doc.assets[c.asset].sha256.is_empty() && !counted.contains(&c.asset))
        .collect()
}

/// Renders the pages of `needs` with `extract_assets`: a page that needs vision into a blob
/// of its asset, a page with small pictures into one blob for each picture first seen there,
/// cut from the render. One render buffer lives at a time. What is not rendered gets a loss:
/// one per page for a page cap or a render that failed, one for all that is left past the
/// render count, the blob room or the time.
pub(crate) fn render_pages(
    pdf: &Pdf,
    needs: &[Need],
    opts: &ReadOptions,
    budget: &Budget,
    out: &mut Out,
) {
    if !opts.extract_assets || needs.is_empty() {
        return;
    }
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
                format!("page {} is not rendered: {why}", need.page),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        return;
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
                    format!("page {} is not rendered: {why}", need.page),
                    LossReason::Damaged,
                    Severity::Warning,
                );
            }
            return;
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
    // What is left out for the render count, the blob room or the time, and the small
    // pictures it counts.
    let mut left_out: Option<LeftOut> = None;
    let mut counted: HashSet<usize> = HashSet::new();
    // Why each page with small pictures was not rendered.
    let mut failed: HashMap<u32, LossReason> = HashMap::new();
    // The content streams and stencils checked for earlier pages.
    let mut seen = HashSet::new();
    let mut rendered = 0;
    for need in needs {
        let n = need.page;
        // A page with small pictures only is rendered for those without a file yet.
        let todo = pending(need, &counted, out);
        if need.whole.is_none() && todo.is_empty() {
            continue;
        }
        let mut fail = |why: LossReason, what: String, out: &mut Out| {
            out.loss(
                format!("p{n}"),
                format!("page {n} is not rendered: {what}"),
                why,
                Severity::Warning,
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
            Some(_) => LeftOut::page(n, why),
            None => {
                counted.extend(todo.iter().map(|c| c.asset));
                let mut left = LeftOut::page(n, why);
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
        let page = match render_page(&doc, n, budget, room, &facts, &mut seen) {
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
                let png = match encode(page.pixmap) {
                    Ok(png) => png,
                    Err(why) => {
                        fail(LossReason::Damaged, why, out);
                        continue;
                    }
                };
                if let Err(why) = keep(png, asset, out) {
                    left_out = Some(LeftOut::page(n, why));
                }
            }
            None => {
                left_out = cut_out(need, &todo, page, budget, room, out);
                if left_out.is_some() {
                    counted.extend(
                        todo.iter()
                            .filter(|c| out.doc.assets[c.asset].sha256.is_empty())
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
            Severity::Warning,
        );
    }
    unwritten(needs, &counted, &failed, out);
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

/// Cuts the small pictures `todo` of `need` out of its render, one at a time, each into a
/// blob of its asset. The render is dropped before the last one is written. A crop takes its
/// pixels and its PNG next to the render, which must fit the render room the page was checked
/// against. A picture not cut here stays without a file, for a later page. Returns what is
/// left out when the time or the blob room runs out.
fn cut_out(
    need: &Need,
    todo: &[&Crop],
    page: Rendered,
    budget: &Budget,
    room: u64,
    out: &mut Out,
) -> Option<LeftOut> {
    let n = need.page;
    let mut page = Some(page);
    let pixels = |p: &Rendered| u64::from(p.pixmap.width()) * u64::from(p.pixmap.height());
    for (i, c) in todo.iter().enumerate() {
        let id = out.doc.assets[c.asset].id.clone();
        let left = |why: String| Some(LeftOut::pictures(n, &id, todo.len() - i, why));
        if let Err(hit) = budget.check_time() {
            return left(hit.to_string());
        }
        let p = page.as_ref()?;
        let user = [
            c.b[0] + need.origin.0,
            c.b[1] + need.origin.1,
            c.b[2] + need.origin.0,
            c.b[3] + need.origin.1,
        ];
        let Some(rect) = crop_rect(p, user, CROP_MARGIN) else {
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
        let takes = pixels(p) * 4 + rect.pixels() * cost::PNG_BYTES;
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
        let pixmap = crop(p, rect);
        if i + 1 == todo.len() {
            page = None;
        }
        match encode(pixmap) {
            Ok(png) => {
                if let Err(why) = keep(png, c.asset, out) {
                    return left(why);
                }
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

/// Why a render did not happen.
enum Failed {
    /// The time ran out: no later page is rendered either.
    Limit(String),
    /// The page would take more than the render room, or what it takes cannot be known.
    Room(String),
    Damaged(String),
}

/// A rendered page: its pixels, and the transform from user space to them.
pub(crate) struct Rendered {
    pub pixmap: Pixmap,
    pub transform: Affine,
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

/// Renders page `page` (1-based) of `doc`, opened from the canonical bytes, at 144 dpi, at a
/// smaller scale when that would pass [`MAX_PIXELS`]. The time is checked first, then what
/// the render and its PNG take is worked out against `room` bytes.
fn render_page(
    doc: &hayro_syntax::Pdf,
    page: u32,
    budget: &Budget,
    room: u64,
    facts: &cost::Facts<'_>,
    seen: &mut HashSet<hayro_syntax::object::ObjectIdentifier>,
) -> Result<Rendered, Failed> {
    budget
        .check_time()
        .map_err(|hit| Failed::Limit(hit.to_string()))?;
    let Some(p) = usize::try_from(page)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| doc.pages().get(i))
    else {
        return Err(Failed::Damaged("the renderer does not find it".into()));
    };
    let (w, h) = p.render_dimensions();
    let (w, h) = (f64::from(w), f64::from(h));
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(Failed::Damaged("the page has no size".into()));
    }
    // A page whose PNG would not fit the room is rendered smaller.
    let (px, py) = pixel_size(w, h, MAX_PIXELS.min(room / cost::PNG_BYTES));
    let transform = Affine::scale_non_uniform(f64::from(px) / w, f64::from(py) / h)
        * p.initial_transform(true).to_kurbo();
    let settings = InterpreterSettings::default();
    stencils::page_stencils(p, room, facts, seen).map_err(Failed::Room)?;
    match cost::estimate(p, transform, (px, py), &settings, room, facts) {
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
    match run {
        Ok(pixmap) => Ok(Rendered { pixmap, transform }),
        Err(_) => Err(Failed::Damaged(
            "the renderer stopped with an internal error".into(),
        )),
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

/// The alt text of each Figure of the tag tree by the page and marked content id of its own
/// content, so a picture drawn in that marked content gets it.
pub(crate) fn figure_alts(pdf: &Pdf) -> HashMap<(u32, u32), String> {
    let mut out = HashMap::new();
    if let Some(tree) = pdf.struct_tree() {
        for e in &tree.elems {
            if let (true, Some(page), Some(alt)) = (e.role == "Figure", e.page, e.alt.as_ref())
                && !alt.trim().is_empty()
            {
                for &mcid in &e.mcids {
                    out.entry((page, mcid)).or_insert_with(|| alt.clone());
                }
            }
        }
    }
    out
}

/// The alt text of each page's first figure in the tag tree.
pub(crate) fn figure_captions(pdf: &Pdf) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    if let Some(tree) = pdf.struct_tree() {
        for e in &tree.elems {
            if let (true, Some(page), Some(alt)) = (e.role == "Figure", e.page, e.alt.as_ref())
                && !alt.trim().is_empty()
            {
                out.entry(page).or_insert_with(|| alt.clone());
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
