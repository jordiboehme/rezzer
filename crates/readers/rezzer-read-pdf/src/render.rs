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

/// Whether page `info` with the items read from it needs vision: it has no text layer over
/// a picture (a scan), its glyphs have no Unicode mapping, or it is only an invisible text
/// layer over a picture (an OCR layer); or its pictures, without the page furniture
/// `skip`, cover at least 2 % of it.
pub(crate) fn vision(info: &PageInfo, items: &PageItems, skip: &Spots) -> Option<Vision> {
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
    if !shown.is_empty() && pictured >= PICTURE_SHARE {
        let k = shown.len();
        words.push(format!(
            "{k} {}",
            if k == 1 { "picture" } else { "pictures" }
        ));
    } else {
        // Pictures too small to make the page need vision are still not dropped: each one of
        // at least 1 cm2 is named with its box.
        let small: Vec<usize> = shown_at
            .iter()
            .copied()
            .filter(|&k| visible(&items.images[k]) >= SMALL_PICTURE)
            .collect();
        if let Some(&k) = small.first() {
            first = Some(items.images[k]);
            words.push(small_pictures(items, &small));
        }
    }
    (!words.is_empty()).then(|| Vision {
        alt: words.join(", "),
        first,
    })
}

/// The alt text for small pictures `small` of a page: how many, and the box of each of the
/// first [`SMALL_LISTED`] in points. A near-square one of 1-bit pixels, at least 21 a side, is
/// named a likely code.
fn small_pictures(items: &PageItems, small: &[usize]) -> String {
    let n = small.len();
    let mut boxes: Vec<String> = small
        .iter()
        .take(SMALL_LISTED)
        .map(|&k| {
            let b = items.images[k];
            let r = |v: f32| v.round() as i64;
            let mut s = format!("box {}, {} to {}, {}", r(b[0]), r(b[1]), r(b[2]), r(b[3]));
            let (w, h) = (b[2] - b[0], b[3] - b[1]);
            let square = (w - h).abs() <= 0.1 * w.max(h);
            if square && items.images_code.get(k).copied().unwrap_or(false) {
                s.push_str(", likely a barcode or QR code, not decoded");
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

/// Puts `media` next to the block anchored `at`: after it when the run found there stands
/// above the picture, else before it. Without such a block it goes at the end. Returns where
/// it went.
pub(crate) fn place(
    blocks: &mut Vec<Block>,
    at: Option<&Anchor>,
    after: bool,
    media: Block,
) -> usize {
    let found = at.and_then(|a| blocks.iter().position(|b| anchor_of(b) == Some(a)));
    let i = match found {
        Some(i) if after => i + 1,
        Some(i) => i,
        None => blocks.len(),
    };
    blocks.insert(i, media);
    i
}

/// A page that needs vision, to render: its asset, whether a limit cut its text, and whether
/// its text came from the fallback parser (its content is then missing in the canonical copy).
pub(crate) struct Need {
    pub page: u32,
    pub asset: usize,
    pub cut: bool,
    pub fallback: bool,
}

/// The asset of page `n`, listed without a file until it is rendered.
pub(crate) fn asset(n: u32, alt: String, caption: Option<String>) -> Asset {
    Asset {
        id: format!("page-{n}"),
        path: None,
        sha256: String::new(),
        mime: "image/png".into(),
        caption,
        alt: Some(alt),
        needs_vision: true,
    }
}

/// Renders the pages of `needs` with `extract_assets`, each into a blob of its asset. One
/// render buffer lives at a time. What is not rendered gets a loss: one per page for a page
/// cap or a render that failed, one for all pages past the render count or the blob room.
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
    // The pages left out for the render count, the blob room or the time: the first, why,
    // how many.
    let mut left_out: Option<(u32, String, usize)> = None;
    // The content streams and stencils checked for earlier pages.
    let mut seen = HashSet::new();
    let mut rendered = 0;
    for need in needs {
        let n = need.page;
        let skip = |why: String, out: &mut Out| {
            out.loss(
                format!("p{n}"),
                format!("page {n} is not rendered: {why}"),
                LossReason::Limit,
                Severity::Warning,
            );
        };
        if let Some((_, _, k)) = &mut left_out {
            *k += 1;
            continue;
        }
        if rendered == MAX_RENDERS {
            left_out = Some((n, format!("a file renders at most {MAX_RENDERS} pages"), 1));
            continue;
        }
        if need.fallback {
            out.loss(
                format!("p{n}"),
                format!(
                    "page {n} is not rendered: its text was read with the fallback parser, and \
                     the renderer reads only the object layer's copy, where its content is \
                     missing"
                ),
                LossReason::Unsupported,
                Severity::Warning,
            );
            continue;
        }
        if need.cut {
            skip(
                "a limit cut its text, so its content is more than a render may take".into(),
                out,
            );
            continue;
        }
        let png = match render_png(&doc, n, budget, room, &facts, &mut seen) {
            Ok(png) => png,
            Err(Failed::Limit(why)) => {
                left_out = Some((n, why, 1));
                continue;
            }
            Err(Failed::Room(why)) => {
                skip(why, out);
                continue;
            }
            Err(Failed::Damaged(why)) => {
                out.loss(
                    format!("p{n}"),
                    format!("page {n} is not rendered: {why}"),
                    LossReason::Damaged,
                    Severity::Warning,
                );
                continue;
            }
        };
        rendered += 1;
        let size = png.len() as u64;
        if size > out.blob_room {
            left_out = Some((
                n,
                format!(
                    "its picture takes {size} bytes, more than the {} bytes left for extracted \
                     pictures and attachments (an eighth of the memory limit)",
                    out.blob_room
                ),
                1,
            ));
            continue;
        }
        out.blob_room -= size;
        let sha256: String = Sha256::digest(&png)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let asset = &mut out.doc.assets[need.asset];
        asset.sha256 = sha256;
        out.blobs.push(Blob {
            asset: asset.id.clone(),
            bytes: png,
        });
    }
    if let Some((first, why, k)) = left_out {
        let what = if k == 1 {
            format!("page {first} is not rendered: {why}")
        } else {
            format!("{k} pages that need vision are not rendered, from page {first} on: {why}")
        };
        out.loss(
            format!("p{first}"),
            what,
            LossReason::Limit,
            Severity::Warning,
        );
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

/// Renders page `page` (1-based) of `doc`, opened from the canonical bytes, to PNG at 144
/// dpi, at a smaller scale when that would pass [`MAX_PIXELS`]. The time is checked first,
/// then what the render takes is worked out against `room` bytes.
fn render_png(
    doc: &hayro_syntax::Pdf,
    page: u32,
    budget: &Budget,
    room: u64,
    facts: &cost::Facts<'_>,
    seen: &mut HashSet<hayro_syntax::object::ObjectIdentifier>,
) -> Result<Vec<u8>, Failed> {
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
        pixmap.into_png()
    }));
    match run {
        Ok(Ok(png)) => Ok(png),
        Ok(Err(e)) => Err(Failed::Damaged(format!(
            "the PNG could not be written ({e})"
        ))),
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
