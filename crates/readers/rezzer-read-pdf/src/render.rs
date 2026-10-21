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
//! - the page pixmap has at most [`MAX_PIXELS`];
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

/// Pictures that cover more than this part of a page without text make it a scan.
const SCAN_SHARE: f32 = 0.5;

/// Why a page needs vision, and the box of its first picture.
pub(crate) struct Vision {
    /// The asset's alt text: `no text layer`, `3 pictures`.
    pub alt: String,
    pub first: Option<[f32; 4]>,
}

/// A picture box rounded to whole points, to find the same picture on many pages.
type Spot = [i32; 4];

fn spot(b: &[f32; 4]) -> Spot {
    b.map(|v| v.round() as i32)
}

/// Pictures drawn at the same place on at least three pages and on at least half of the pages
/// read. When they are small, they are page furniture (a logo in the margin, a strip along
/// the edge), like a running header, and do not make a page need vision.
pub(crate) fn furniture<'a>(pages: impl Iterator<Item = &'a PageItems>) -> HashSet<Spot> {
    let mut seen: HashMap<Spot, usize> = HashMap::new();
    let mut read = 0usize;
    for items in pages {
        read += 1;
        let on_page: HashSet<Spot> = items.images.iter().map(spot).collect();
        for s in on_page {
            *seen.entry(s).or_insert(0) += 1;
        }
    }
    seen.into_iter()
        .filter(|&(_, k)| k >= 3 && k * 2 >= read)
        .map(|(s, _)| s)
        .collect()
}

/// Whether page `info` with the items read from it needs vision: it has no text layer over
/// a picture (a scan), its glyphs have no Unicode mapping, or it is only an invisible text
/// layer over a picture (an OCR layer); or its pictures, without the page furniture
/// `skip`, cover at least 2 % of it.
pub(crate) fn vision(info: &PageInfo, items: &PageItems, skip: &HashSet<Spot>) -> Option<Vision> {
    let (width, height) = (info.width, info.height);
    let area = (width * height).max(1.0);
    let visible = |b: &[f32; 4]| {
        let w = (b[2].min(width) - b[0].max(0.0)).max(0.0);
        let h = (b[3].min(height) - b[1].max(0.0)).max(0.0);
        w * h
    };
    let drawn: Vec<&[f32; 4]> = items.images.iter().filter(|b| visible(b) > 0.0).collect();
    let covered = (drawn.iter().map(|b| visible(b)).sum::<f32>() / area).min(1.0);
    // Furniture is small: a picture over a quarter of the page is content wherever it is.
    let shown: Vec<&[f32; 4]> = drawn
        .into_iter()
        .filter(|b| !(skip.contains(&spot(b)) && visible(b) < FURNITURE_SHARE * area))
        .collect();
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
    if !shown.is_empty() && pictured >= PICTURE_SHARE {
        let k = shown.len();
        words.push(format!(
            "{k} {}",
            if k == 1 { "picture" } else { "pictures" }
        ));
    }
    (!words.is_empty()).then(|| Vision {
        alt: words.join(", "),
        first: shown.first().map(|b| **b),
    })
}

/// The top middle of the first picture, where the run nearest to it is looked for.
pub(crate) fn picture_top(v: &Vision) -> Option<(f32, f32)> {
    let b = v.first?;
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
    let (px, py) = pixel_size(w, h);
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
/// [`MAX_PIXELS`], each side at least 1 and at most 65,535 pixels.
fn pixel_size(w: f64, h: f64) -> (u16, u16) {
    let mut scale = SCALE;
    let pixels = w * scale * h * scale;
    if pixels > MAX_PIXELS as f64 {
        scale *= (MAX_PIXELS as f64 / pixels).sqrt();
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
        assert_eq!(pixel_size(595.0, 842.0), (1190, 1684));
        let (w, h) = pixel_size(200_000.0, 200_000.0);
        assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS);
        assert_eq!((w, h), (4000, 4000));
        // A long thin page keeps each side inside a pixmap's bounds.
        let (w, h) = pixel_size(1e9, 0.001);
        assert_eq!((w, h), (u16::MAX, 1));
    }
}
