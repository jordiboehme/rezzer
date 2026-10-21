// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a render takes, worked out before it runs.
//!
//! hayro sizes several buffers from values in the file, and an allocation that fails ends
//! the process, which no `catch_unwind` can stop. So each page is first run through the same
//! interpreter, with the same settings, into [`Dry`], a device that draws nothing and adds up
//! the buffers the renderer would build for each call, at the transform it has then (forms,
//! patterns, Type3 glyphs, soft masks and annotation appearances are followed the way the
//! renderer follows them). A page whose estimate passes the render room is not rendered.
//!
//! The renderer's buffers that depend on the file, and how they are counted (bytes):
//! - the page pixmap and the PNG: width x height x 4 each;
//! - a raster image: what its filter unpacks to (the measured size; a DCT, JPX or JBIG2
//!   frame or the CCITT `/Columns` and `/Rows` give the decoded size), the samples at the
//!   stated size, an RGB copy, and the RGBA copy at the size it is drawn (kept until the end of
//!   the render); its soft mask the same way;
//! - a stencil mask: its samples, the RGBA copy, and with a pattern as paint a sub-render of
//!   the page's size and a mask kept until the end;
//! - a tiling pattern: a cell pixmap and render context sized from its step, as the renderer
//!   sizes it, for every fill (none is cached), and the cost of its content inside the cell;
//! - a shading pattern that is not drawn as a gradient: a texture of the filled area; a mesh
//!   shading also a map entry for each pixel its triangles cover, and its patches' triangles;
//! - a soft mask: a render of the page's size and the mask, once for each mask in a render;
//! - a Type3 glyph: its content, as drawn.
//!
//! Stencil masks and inline stencils do not show their data to a device, so [`stencils`]
//! checks every stencil in the file once: one whose decoded size cannot be known, or alone
//! passes the room, keeps all renders of the file out. What the interpreter and the outline
//! cache keep is bounded by the guard's stream sizes, like the text walk.

use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};

use hayro_interpret::encode::{EncodedShadingType, texture_dimensions};
use hayro_interpret::font::Glyph;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::shading::ShadingType;
use hayro_interpret::util::x_y_advances;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, Paint, SoftMask, interpret_page,
};
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::{Array, Dict, Name, Object, Stream};
use hayro_syntax::page::Page;
use kurbo::{Affine, BezPath, Rect, Shape};

use crate::fallback::filters_of;

/// Device calls one page's estimate may take.
const MAX_CALLS: u64 = 16_000_000;

/// Device calls between two questions to the stop callback.
const POLL: u64 = 4096;

/// Nesting of patterns, soft masks and Type3 glyphs the estimate follows.
const MAX_DEPTH: usize = 32;

/// Bytes the renderer's map of a mesh shading takes for each pixel a triangle covers.
const MESH_ENTRY: u64 = 64;

/// Triangles the renderer makes of each patch of a patch mesh.
const PATCH_TRIANGLES: u64 = 2 * 19 * 19;

/// What a render's memory goes to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    Page,
    Pictures,
    Patterns,
    Shadings,
    SoftMasks,
}

impl Kind {
    pub(crate) fn words(self) -> &'static str {
        match self {
            Kind::Page => "the page itself",
            Kind::Pictures => "its pictures",
            Kind::Patterns => "its tiling patterns",
            Kind::Shadings => "its shadings",
            Kind::SoftMasks => "its soft masks",
        }
    }
}

/// Why a page's estimate was not finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unknown {
    /// The estimate passed the room on the way: what took the most, and the bytes so far.
    Over(Kind, u64),
    /// Something's size cannot be known before rendering.
    Unbounded(String),
    /// The caller asked to stop.
    Stopped,
}

/// Payload of the panic that ends an estimate early.
struct Abort(Unknown);

/// What the estimate needs to know of the file.
pub(crate) struct Facts<'f> {
    /// What a stream of the canonical bytes unpacks to, as the guard measured it.
    pub sizes: &'f dyn Fn(u32, u16) -> Option<u64>,
    pub limits: &'f pdfparts::Limits,
    pub stop: &'f dyn Fn() -> bool,
}

/// One render target: the page, a pattern cell, a soft mask.
struct Level {
    w: u64,
    h: u64,
    /// Bytes kept until this target is done.
    kept: u64,
    /// Soft masks this target's renderer already built.
    masks: HashSet<u128>,
}

/// A device that adds up what the renderer would build.
struct Dry<'f> {
    facts: &'f Facts<'f>,
    room: u64,
    levels: Vec<Level>,
    /// Bytes kept by all open targets.
    alive: u64,
    peak: u64,
    by_kind: [u64; 5],
    calls: u64,
    depth: usize,
}

impl Dry<'_> {
    fn level(&self) -> &Level {
        self.levels.last().expect("a render target")
    }

    fn kind_index(kind: Kind) -> usize {
        kind as usize
    }

    fn most(&self) -> Kind {
        [
            Kind::Page,
            Kind::Pictures,
            Kind::Patterns,
            Kind::Shadings,
            Kind::SoftMasks,
        ]
        .into_iter()
        .max_by_key(|k| self.by_kind[Self::kind_index(*k)])
        .unwrap_or_default()
    }

    /// A buffer that lives for the call (`passing`) and one kept until the target is done.
    fn charge(&mut self, kind: Kind, passing: u64, kept: u64) {
        self.by_kind[Self::kind_index(kind)] =
            self.by_kind[Self::kind_index(kind)].saturating_add(passing.saturating_add(kept));
        let now = self.alive.saturating_add(passing).saturating_add(kept);
        self.peak = self.peak.max(now);
        self.alive = self.alive.saturating_add(kept);
        if let Some(l) = self.levels.last_mut() {
            l.kept = l.kept.saturating_add(kept);
        }
        if self.peak > self.room {
            let most = self.most();
            panic::resume_unwind(Box::new(Abort(Unknown::Over(most, self.peak))));
        }
    }

    fn tick(&mut self) {
        self.calls += 1;
        if self.calls > MAX_CALLS {
            panic::resume_unwind(Box::new(Abort(Unknown::Unbounded(format!(
                "it draws more than {MAX_CALLS} times, too much to work out its render first"
            )))));
        }
        if self.calls.is_multiple_of(POLL) && (self.facts.stop)() {
            panic::resume_unwind(Box::new(Abort(Unknown::Stopped)));
        }
    }

    fn unbounded(why: String) -> ! {
        panic::resume_unwind(Box::new(Abort(Unknown::Unbounded(why))))
    }

    /// Runs `inner` with a new render target of `w` by `h` pixels; what it keeps is freed
    /// after, but the target's own pixmap stays when `result` says so.
    fn nested(&mut self, kind: Kind, w: u64, h: u64, result: u64, inner: impl FnOnce(&mut Self)) {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Self::unbounded(format!(
                "its patterns, soft masks or Type3 glyphs nest more than {MAX_DEPTH} deep"
            ));
        }
        // The target's render context and pixmap.
        let target = w.saturating_mul(h).saturating_mul(8);
        self.levels.push(Level {
            w,
            h,
            kept: 0,
            masks: HashSet::new(),
        });
        self.charge(kind, 0, target);
        inner(self);
        let done = self.levels.pop().map_or(0, |l| l.kept);
        self.alive = self.alive.saturating_sub(done);
        self.charge(kind, 0, result);
        self.depth -= 1;
    }

    fn soft_mask(&mut self, mask: Option<&SoftMask<'_>>) {
        let Some(mask) = mask else {
            return;
        };
        let key = mask.cache_key();
        let (w, h) = (self.level().w, self.level().h);
        let new = self.levels.last_mut().is_some_and(|l| l.masks.insert(key));
        if !new {
            return;
        }
        // A sub-render of the target's size; the mask (a byte a pixel) stays.
        let mask = mask.clone();
        self.nested(Kind::SoftMasks, w, h, w.saturating_mul(h), |dry| {
            mask.interpret(dry);
        });
    }

    /// The paint of a fill or stroke over `area` (device pixels).
    fn paint(&mut self, paint: &Paint<'_>, area: Rect, is_stroke: bool) {
        let Paint::Pattern(p) = paint else {
            return;
        };
        let (cw, ch) = (self.level().w as f64, self.level().h as f64);
        let area = area.intersect(Rect::new(0.0, 0.0, cw, ch));
        match &**p {
            Pattern::Shading(s) => {
                match s.shading.shading_type.as_ref() {
                    ShadingType::TriangleMesh { triangles, .. } => {
                        let covered: u64 = triangles
                            .iter()
                            .map(|t| {
                                let pts = [t.p0.point, t.p1.point, t.p2.point];
                                pixels_under(&pts.map(|p| s.matrix * p))
                            })
                            .fold(0, u64::saturating_add);
                        self.charge(Kind::Shadings, covered.saturating_mul(MESH_ENTRY), 0);
                    }
                    ShadingType::CoonsPatchMesh { patches, .. } => {
                        let covered: u64 = patches
                            .iter()
                            .map(|p| pixels_under(&p.control_points.map(|c| s.matrix * c)))
                            .fold(0, u64::saturating_add);
                        let triangles = (patches.len() as u64)
                            .saturating_mul(PATCH_TRIANGLES)
                            .saturating_mul(256);
                        self.charge(
                            Kind::Shadings,
                            covered.saturating_mul(MESH_ENTRY).saturating_add(triangles),
                            0,
                        );
                    }
                    ShadingType::TensorProductPatchMesh { patches, .. } => {
                        let covered: u64 = patches
                            .iter()
                            .map(|p| pixels_under(&p.control_points.map(|c| s.matrix * c)))
                            .fold(0, u64::saturating_add);
                        let triangles = (patches.len() as u64)
                            .saturating_mul(PATCH_TRIANGLES)
                            .saturating_mul(256);
                        self.charge(
                            Kind::Shadings,
                            covered.saturating_mul(MESH_ENTRY).saturating_add(triangles),
                            0,
                        );
                    }
                    _ => {}
                }
                // The renderer encodes every shading; a gradient it can draw natively needs
                // no texture.
                let native = match s.shading.shading_type.as_ref() {
                    ShadingType::RadialAxial { .. } => {
                        let encoded = s.encode();
                        matches!(&encoded.shading_type, EncodedShadingType::RadialAxial(g)
                            if g.as_svg_gradient(&encoded, area, 0.01).is_some())
                    }
                    _ => false,
                };
                if !native {
                    let (w, h) = texture_dimensions(area, 1.0);
                    self.charge(Kind::Shadings, 0, u64::from(w) * u64::from(h) * 4);
                }
            }
            Pattern::Tiling(t) => {
                // As the renderer sizes the cell (hayro 0.8.0, paint.rs).
                let bbox = t.bbox;
                let (bw, bh) = (bbox.width() as f32, bbox.height() as f32);
                let (xa, ya) = x_y_advances(&t.matrix);
                let xs = (xa.length() as f32).max(1.0 / bw).min(3000.0 / bw);
                let ys = (ya.length() as f32).max(1.0 / bh).min(3000.0 / bh);
                let pw = u64::from((xs * t.x_step).abs().round() as u16);
                let ph = u64::from((ys * t.y_step).abs().round() as u16);
                let cell = pw.saturating_mul(ph).saturating_mul(4);
                let initial = Affine::scale_non_uniform(f64::from(xs), f64::from(ys))
                    * Affine::translate((-bbox.x0, -bbox.y0));
                let t = t.clone();
                self.nested(Kind::Patterns, pw, ph, cell, |dry| {
                    t.interpret(dry, initial, is_stroke);
                });
            }
        }
    }

    fn raster(&mut self, stream: &Stream<'_>, transform: Affine) {
        let dict = stream.dict();
        let spec = Spec::of(dict);
        let decoded =
            decoded_size(&spec, stream, self.facts).unwrap_or_else(|why| Self::unbounded(why));
        let alpha = dict
            .get::<Stream<'_>>(b"SMask")
            .map(|m| {
                let s = Spec::of(m.dict());
                decoded_size(&s, &m, self.facts)
                    .map(|d| {
                        d.bytes
                            .saturating_add(d.w.saturating_mul(d.h).saturating_mul(2))
                    })
                    .unwrap_or_else(|why| Self::unbounded(why))
            })
            .unwrap_or(0);
        let (passing, kept) = image_bytes(&spec, &decoded, transform, alpha);
        self.charge(Kind::Pictures, passing, kept);
    }

    fn stencil(&mut self, w: u32, h: u32, transform: Affine) -> u64 {
        let spec = Spec {
            w: u64::from(w),
            h: u64::from(h),
            comps: 1,
            out: 1,
            direct: false,
            native: true,
        };
        let decoded = Decoded {
            w: spec.w,
            h: spec.h,
            bytes: spec.w.saturating_mul(spec.h),
        };
        let (passing, kept) = image_bytes(&spec, &decoded, transform, 0);
        self.charge(Kind::Pictures, passing, kept);
        spec.w.saturating_mul(spec.h)
    }
}

/// How an image is stated in its dictionary.
struct Spec {
    w: u64,
    h: u64,
    /// Components a sample has.
    comps: u64,
    /// Components after conversion: 1 for gray, 3 for color.
    out: u64,
    /// 8-bit samples without a `/Decode` array, used as unpacked without a copy.
    direct: bool,
    /// Gray or device RGB samples, which need no conversion buffer.
    native: bool,
}

impl Spec {
    fn of(dict: &Dict<'_>) -> Spec {
        let int = |a: &[u8], b: &[u8]| {
            dict.get::<u32>(a)
                .or_else(|| dict.get::<u32>(b))
                .map_or(0, u64::from)
        };
        let (comps, out, native) = color_components(
            dict.get::<Object<'_>>(b"ColorSpace")
                .or_else(|| dict.get::<Object<'_>>(b"CS")),
        );
        let bpc = int(b"BitsPerComponent", b"BPC");
        let decode = dict.contains_key(b"Decode") || dict.contains_key(b"D");
        Spec {
            w: int(b"Width", b"W"),
            h: int(b"Height", b"H"),
            comps,
            out,
            direct: bpc == 8 && !decode,
            native,
        }
    }
}

/// Components of a color space, after conversion, and whether its samples are used without a
/// conversion buffer; unknown spaces count as four components.
fn color_components(cs: Option<Object<'_>>) -> (u64, u64, bool) {
    let name_of = |n: &[u8]| match n {
        b"DeviceGray" | b"G" | b"CalGray" => (1, 1, true),
        b"DeviceRGB" | b"RGB" => (3, 3, true),
        b"CalRGB" | b"Lab" => (3, 3, false),
        _ => (4, 3, false),
    };
    match cs {
        None => (4, 3, false),
        Some(Object::Name(n)) => name_of(n.as_ref()),
        Some(Object::Array(a)) => {
            let first = a.iter::<Name<'_>>().next();
            match first.as_deref() {
                Some(b"ICCBased") => {
                    let n = a
                        .iter::<Stream<'_>>()
                        .next()
                        .and_then(|s| s.dict().get::<u32>(b"N"))
                        .map_or(4, u64::from)
                        .clamp(1, 32);
                    (n, if n == 1 { 1 } else { 3 }, false)
                }
                Some(b"Indexed" | b"I") => (1, 3, false),
                Some(b"Separation") => (1, 3, false),
                Some(b"DeviceN") => {
                    let n = a
                        .iter::<Array<'_>>()
                        .next()
                        .map_or(4, |names| names.raw_iter().count() as u64)
                        .clamp(1, 32);
                    (n, 3, false)
                }
                Some(other) => name_of(other),
                None => (4, 3, false),
            }
        }
        Some(_) => (4, 3, false),
    }
}

/// What a decoder builds: the size of the picture it gives and the bytes it unpacks to.
struct Decoded {
    w: u64,
    h: u64,
    bytes: u64,
}

/// What the filter chain of an image unpacks to. `Err` when it cannot be known.
fn decoded_size(spec: &Spec, stream: &Stream<'_>, facts: &Facts<'_>) -> Result<Decoded, String> {
    let (names, _) = filters_of(stream.dict());
    let stated = Decoded {
        w: spec.w,
        h: spec.h,
        bytes: spec.w.saturating_mul(spec.h).saturating_mul(spec.comps),
    };
    let Some(last) = names.last() else {
        return Ok(stated);
    };
    // The codec's own header can only be read when it is the only filter.
    let alone = names.len() == 1;
    let data = stream.raw_data();
    let id = stream.obj_id();
    match last.as_slice() {
        b"DCTDecode" | b"DCT" => {
            // The decoder cuts a frame larger than the dictionary to the dictionary's size.
            let frame = if alone { jpeg_frame(&data) } else { None };
            let (fw, fh, fc, progressive) = frame.unwrap_or((0, 0, 0, true));
            let (w, h) = larger((spec.w, spec.h), (fw, fh));
            let c = fc.max(spec.comps.min(4)).max(1);
            // A progressive frame (or one whose kind is not known) keeps 16-bit coefficients
            // next to the output.
            let copies = if progressive { 3 } else { 1 };
            Ok(Decoded {
                w,
                h,
                bytes: w.saturating_mul(h).saturating_mul(c).saturating_mul(copies),
            })
        }
        b"JPXDecode" => {
            let (w, h, c) = alone
                .then(|| jpx_frame(&data))
                .flatten()
                .ok_or_else(|| "the size of a JPEG 2000 picture cannot be read".to_string())?;
            Ok(Decoded {
                w,
                h,
                bytes: w
                    .saturating_mul(h)
                    .saturating_mul(c.max(1))
                    .saturating_mul(5),
            })
        }
        b"JBIG2Decode" => {
            let (w, h) = alone
                .then(|| jbig2_page(&data))
                .flatten()
                .ok_or_else(|| "the size of a JBIG2 picture cannot be read".to_string())?;
            let (w, h) = larger((spec.w, spec.h), (w, h));
            Ok(Decoded {
                w,
                h,
                bytes: w.saturating_mul(h).saturating_mul(2),
            })
        }
        b"CCITTFaxDecode" | b"CCF" => {
            let parms = ccitt_parms(stream.dict(), names.len() - 1);
            let columns = parms
                .as_ref()
                .and_then(|p| p.get::<u32>(b"Columns"))
                .map_or(1728, u64::from);
            let rows = parms
                .as_ref()
                .and_then(|p| p.get::<u32>(b"Rows"))
                .map_or(0, u64::from)
                .max(spec.h);
            Ok(Decoded {
                w: spec.w,
                h: spec.h,
                bytes: columns.saturating_mul(rows),
            })
        }
        _ => {
            let measured = match (u32::try_from(id.obj_number), u16::try_from(id.gen_number)) {
                (Ok(n), Ok(g)) if n > 0 => (facts.sizes)(n, g),
                _ => None,
            };
            let unpacked = match measured {
                Some(n) => n,
                // An inline image: its data is measured here.
                None => {
                    let (names, parms) = filters_of(stream.dict());
                    let names: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
                    pdfparts::measure_stream(
                        &data,
                        &names,
                        &parms,
                        facts.limits,
                        facts.limits.max_total_bytes,
                        facts.stop,
                    )
                    .map_err(|why| format!("an inline picture cannot be measured: {why}"))?
                }
            };
            Ok(Decoded {
                bytes: unpacked,
                ..stated
            })
        }
    }
}

fn larger(a: (u64, u64), b: (u64, u64)) -> (u64, u64) {
    if b.0.saturating_mul(b.1) > a.0.saturating_mul(a.1) {
        b
    } else {
        a
    }
}

/// The decode parameters of filter `i` of a stream.
fn ccitt_parms<'a>(dict: &Dict<'a>, i: usize) -> Option<Dict<'a>> {
    dict.get::<Dict<'a>>(b"DecodeParms")
        .or_else(|| dict.get::<Dict<'a>>(b"DP"))
        .or_else(|| {
            dict.get::<Array<'a>>(b"DecodeParms")
                .or_else(|| dict.get::<Array<'a>>(b"DP"))
                .and_then(|a| a.iter::<Object<'a>>().nth(i))
                .and_then(Object::into_dict)
        })
}

/// What drawing an image takes: the bytes that live for the call and those kept until the end
/// of the render, as the renderer builds them (hayro 0.8.0, image.rs).
fn image_bytes(spec: &Spec, d: &Decoded, transform: Affine, alpha: u64) -> (u64, u64) {
    let (w, h) = (d.w.max(1), d.h.max(1));
    let px = w.saturating_mul(h);
    // The samples, unless the unpacked data is used as it is.
    let samples = if spec.direct {
        0
    } else {
        px.saturating_mul(spec.comps)
    };
    // A conversion to RGB writes a new buffer unless the samples are RGB already.
    let rgb = if spec.native { 0 } else { px.saturating_mul(3) };
    // The size the renderer resamples to when the picture is drawn smaller.
    let (xa, ya) = x_y_advances(&transform);
    let (xs, ys) = (
        xa.length() * spec.w.max(1) as f64 / w as f64,
        ya.length() * spec.h.max(1) as f64 / h as f64,
    );
    let (nw, nh, resized) = if xs < 1.0 || ys < 1.0 {
        let side =
            |v: u64, s: f64| ((v as f64 * s).ceil().clamp(1.0, f64::from(u16::MAX / 2))) as u64;
        let (nw, nh) = (side(w, xs), side(h, ys));
        (nw, nh, nw.saturating_mul(nh).saturating_mul(spec.out + 1))
    } else {
        (w, h, 0)
    };
    let rgba = nw.saturating_mul(nh).saturating_mul(4);
    // Drawn at its size, the RGBA copy is the kept one; drawn smaller, it is made of the
    // resampled samples.
    let passing = d
        .bytes
        .saturating_add(samples)
        .saturating_add(rgb)
        .saturating_add(resized)
        .saturating_add(alpha);
    (passing, rgba)
}

/// Pixels the box of `points` covers, each side clamped as the renderer clamps it.
fn pixels_under(points: &[kurbo::Point]) -> u64 {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let side = |a: f64, b: f64| {
        let a = a.floor().clamp(0.0, 65_535.0);
        let b = b.ceil().clamp(0.0, 65_535.0);
        (b - a).max(0.0) as u64
    };
    side(x0, x1).saturating_mul(side(y0, y1))
}

/// Width, height and components of a JPEG frame (its SOF marker), and whether it is not a
/// baseline or extended sequential frame.
pub(crate) fn jpeg_frame(data: &[u8]) -> Option<(u64, u64, u64, bool)> {
    let mut i = 2;
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        match marker {
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let h = u16::from_be_bytes([*data.get(i + 5)?, *data.get(i + 6)?]);
                let w = u16::from_be_bytes([*data.get(i + 7)?, *data.get(i + 8)?]);
                let c = *data.get(i + 9)?;
                let sequential = matches!(marker, 0xC0 | 0xC1);
                return Some((u64::from(w), u64::from(h), u64::from(c), !sequential));
            }
            _ => {
                let len = u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]);
                i += 2 + usize::from(len);
            }
        }
    }
    None
}

/// Width, height and components of a JPEG 2000 picture: the `ihdr` box of a JP2 file, or the
/// SIZ marker of a bare codestream.
pub(crate) fn jpx_frame(data: &[u8]) -> Option<(u64, u64, u64)> {
    let u32_at = |i: usize| -> Option<u64> {
        Some(u64::from(u32::from_be_bytes(
            data.get(i..i + 4)?.try_into().ok()?,
        )))
    };
    let head = &data[..data.len().min(1 << 16)];
    if let Some(at) = head.windows(4).position(|w| w == b"ihdr") {
        let h = u32_at(at + 4)?;
        let w = u32_at(at + 8)?;
        let c = u64::from(u16::from_be_bytes(
            data.get(at + 12..at + 14)?.try_into().ok()?,
        ));
        return Some((w, h, c));
    }
    let at = head.windows(2).position(|w| w == [0xFF, 0x51])?;
    // Lsiz, Rsiz, Xsiz, Ysiz, XOsiz, YOsiz, then tiles, then Csiz.
    let x = u32_at(at + 6)?;
    let y = u32_at(at + 10)?;
    let xo = u32_at(at + 14)?;
    let yo = u32_at(at + 18)?;
    let c = u64::from(u16::from_be_bytes(
        data.get(at + 38..at + 40)?.try_into().ok()?,
    ));
    Some((x.saturating_sub(xo), y.saturating_sub(yo), c))
}

/// Width and height of the page of an embedded JBIG2 stream (its page information segment).
pub(crate) fn jbig2_page(data: &[u8]) -> Option<(u64, u64)> {
    let mut i = 0usize;
    for _ in 0..64 {
        let number = u32::from_be_bytes(data.get(i..i + 4)?.try_into().ok()?);
        let flags = *data.get(i + 4)?;
        let kind = flags & 0x3F;
        let refs = *data.get(i + 5)? >> 5;
        if refs > 4 {
            return None;
        }
        let ref_size = if number <= 256 {
            1
        } else if number <= 65_536 {
            2
        } else {
            4
        };
        let page_size = if flags & 0x40 != 0 { 4 } else { 1 };
        let mut at = i + 6 + usize::from(refs) * ref_size + page_size;
        let len = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?);
        at += 4;
        if kind == 48 {
            let w = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?);
            let h = u32::from_be_bytes(data.get(at + 4..at + 8)?.try_into().ok()?);
            if h == u32::MAX {
                return None;
            }
            return Some((u64::from(w), u64::from(h)));
        }
        if len == u32::MAX {
            return None;
        }
        i = at.checked_add(usize::try_from(len).ok()?)?;
    }
    None
}

impl<'a> Device<'a> for Dry<'_> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        self.tick();
        self.soft_mask(props.soft_mask.as_ref());
        let mut area = (props.transform * path.bounding_box().to_path(0.0)).bounding_box();
        let stroke = matches!(mode, DrawMode::Stroke(_) | DrawMode::FillAndStroke(..));
        if let DrawMode::Stroke(s) | DrawMode::FillAndStroke(_, s) = mode {
            let (a, b) = x_y_advances(&props.transform);
            let f = a.length().max(b.length()) * f64::from(s.line_width);
            area = area.inflate(f, f);
        }
        self.paint(&props.paint, area, stroke);
    }

    fn push_clip_path(&mut self, _: &ClipPath) {
        self.tick();
    }

    fn push_transparency_group(&mut self, _: f32, mask: Option<SoftMask<'a>>, _: BlendMode) {
        self.tick();
        self.soft_mask(mask.as_ref());
    }

    fn draw_glyph_run(
        &mut self,
        run: &hayro_interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        mode: &DrawMode,
    ) {
        self.tick();
        self.soft_mask(props.soft_mask.as_ref());
        let (w, h) = (self.level().w as f64, self.level().h as f64);
        // Outline glyphs with a pattern take the pattern once for the run, over at most the
        // whole target.
        let stroke = matches!(mode, DrawMode::Stroke(_));
        self.paint(&props.paint, Rect::new(0.0, 0.0, w, h), stroke);
        for g in run.glyphs() {
            if let Glyph::Type3(t) = &**g {
                self.tick();
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    Self::unbounded(format!("its Type3 glyphs nest more than {MAX_DEPTH} deep"));
                }
                t.interpret(self, props.transform, g.transform(), &props.paint);
                self.depth -= 1;
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        self.tick();
        self.soft_mask(props.soft_mask.as_ref());
        match image {
            Image::Raster(r) => self.raster(r.stream(), props.transform),
            Image::Stencil(s) => {
                let px = self.stencil(s.width(), s.height(), props.transform);
                // The paint shows only once the stencil is unpacked; it was just charged, and
                // its decoded size was checked for the whole file before.
                let mut paint = None;
                if px <= self.room {
                    s.with_stencil(|_, p| paint = Some(p.clone()), Some((1, 1)));
                }
                if let Some(paint @ Paint::Pattern(_)) = paint {
                    let (w, h) = (self.level().w, self.level().h);
                    // A sub-render of the target's size, an RGB copy and a mask that stays.
                    self.charge(
                        Kind::Pictures,
                        w.saturating_mul(h)
                            .saturating_mul(8)
                            .saturating_add(px.saturating_mul(3)),
                        w.saturating_mul(h),
                    );
                    self.paint(&paint, Rect::new(0.0, 0.0, w as f64, h as f64), false);
                }
            }
        }
    }

    fn pop_clip(&mut self) {
        self.tick();
    }

    fn pop_transparency_group(&mut self) {
        self.tick();
    }
}

/// The most bytes the render of `page` at `transform` into `w` by `h` pixels holds at once,
/// with the same interpreter `settings` the renderer uses. `Err` when it passes `room` on the
/// way, or cannot be known.
pub(crate) fn estimate(
    page: &Page<'_>,
    transform: Affine,
    (w, h): (u16, u16),
    settings: &InterpreterSettings,
    room: u64,
    facts: &Facts<'_>,
) -> Result<u64, Unknown> {
    let (w, h) = (u64::from(w), u64::from(h));
    let mut dry = Dry {
        facts,
        room,
        levels: vec![Level {
            w,
            h,
            kept: 0,
            masks: HashSet::new(),
        }],
        alive: 0,
        peak: 0,
        by_kind: [0; 5],
        calls: 0,
        depth: 0,
    };
    let run = panic::catch_unwind(AssertUnwindSafe(|| {
        // The page's pixmap and render context, and at the end its PNG.
        dry.charge(
            Kind::Page,
            w.saturating_mul(h).saturating_mul(4),
            w.saturating_mul(h).saturating_mul(4),
        );
        let cache = InterpreterCache::new();
        let mut ctx = Context::new(
            transform,
            Rect::new(0.0, 0.0, w as f64, h as f64),
            &cache,
            page.xref(),
            settings.clone(),
        );
        interpret_page(page, &mut ctx, &mut dry);
        dry.peak
    }));
    match run {
        Ok(peak) => Ok(peak),
        Err(payload) => match payload.downcast::<Abort>() {
            Ok(abort) => Err(abort.0),
            Err(_) => Err(Unknown::Unbounded(
                "working out its render stopped with an internal error".into(),
            )),
        },
    }
}

/// Checks every stencil mask of the file, in image streams and inline in content: their
/// data is unpacked before any device sees them, so one whose decoded size cannot be known,
/// or alone takes more than `room`, keeps the renders out. `Err` says which.
pub(crate) fn stencils(
    doc: &hayro_syntax::Pdf,
    room: u64,
    facts: &Facts<'_>,
) -> Result<(), String> {
    let check = |stream: &Stream<'_>| -> Result<(), String> {
        let dict = stream.dict();
        let is_stencil = dict
            .get::<bool>(b"ImageMask")
            .or_else(|| dict.get::<bool>(b"IM"))
            .unwrap_or(false);
        if !is_stencil {
            return Ok(());
        }
        let spec = Spec {
            comps: 1,
            out: 1,
            direct: false,
            native: true,
            ..Spec::of(dict)
        };
        let d = decoded_size(&spec, stream, facts)
            .map_err(|why| format!("the file has a stencil mask whose size is unknown: {why}"))?;
        let bytes = d
            .bytes
            .saturating_add(spec.w.saturating_mul(spec.h).saturating_mul(2));
        if bytes > room {
            return Err(format!(
                "the file has a stencil mask that unpacks to about {bytes} bytes, more than the \
                 {room} bytes a render may take"
            ));
        }
        Ok(())
    };
    for (k, obj) in doc.objects().into_iter().enumerate() {
        if k % 1024 == 0 && (facts.stop)() {
            return Err("the time limit was reached".into());
        }
        let Some(stream) = obj.into_stream() else {
            continue;
        };
        let dict = stream.dict();
        if dict.get::<Name<'_>>(b"Subtype").as_deref() == Some(b"Image") {
            check(&stream)?;
            continue;
        }
        // Inline images live in content: of a page, a form, a pattern, a Type3 glyph or an
        // appearance. Other streams give no inline images when read as content.
        let Ok(data) = stream.decoded() else {
            continue;
        };
        let mut ops = TypedIter::new(&data);
        while let Some(op) = ops.next() {
            if let TypedInstruction::InlineImage(i) = op {
                check(i.0)?;
            }
        }
    }
    Ok(())
}
