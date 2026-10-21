// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stencil masks a page's render can reach, checked before the render: a stencil's data
//! is unpacked before any device sees it, so `cost.rs` cannot count it while it draws.

use std::collections::HashSet;

use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::{Array, Dict, Name, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};

use super::cost::{Facts, Spec, decoded_size};

/// One step of the walk of what a page's render reads as content.
enum Visit<'p> {
    Content(Stream<'p>),
    Resources(Resources<'p>),
    Image(Stream<'p>),
}

/// Objects one page's check may visit.
const VISIT_MAX: usize = 100_000;

/// Checks every stencil mask a render of `page` can reach, in image streams and inline in
/// content: their data is unpacked before any device sees it, so one whose decoded size
/// cannot be known, or alone takes more than `room`, keeps the page from rendering. Only the
/// streams the renderer reads as content are unpacked here (the page's content, forms, tiling
/// patterns, Type3 glyphs, soft mask groups and annotation appearances), each after its
/// measured size is checked against `room`; `seen` holds the streams checked for earlier
/// pages. `Err` says why the page cannot render.
pub(crate) fn page_stencils<'p>(
    page: &Page<'p>,
    room: u64,
    facts: &Facts<'_>,
    seen: &mut HashSet<ObjectIdentifier>,
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
            .map_err(|why| format!("it has a stencil mask whose size is unknown: {why}"))?;
        let bytes = d
            .bytes
            .saturating_add(spec.w.saturating_mul(spec.h).saturating_mul(2));
        if bytes > room {
            return Err(format!(
                "it has a stencil mask that unpacks to about {bytes} bytes, more than the \
                 {room} bytes a render may take"
            ));
        }
        Ok(())
    };
    let raw = page.raw();
    let mut todo: Vec<Visit<'p>> = vec![Visit::Resources(page.resources().clone())];
    if let Some(c) = raw.get::<Stream<'p>>(b"Contents") {
        todo.push(Visit::Content(c));
    }
    if let Some(a) = raw.get::<Array<'p>>(b"Contents") {
        todo.extend(a.iter::<Stream<'p>>().map(Visit::Content));
    }
    for annot in raw
        .get::<Array<'p>>(b"Annots")
        .map(|a| a.iter::<Dict<'p>>().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let Some(ap) = annot.get::<Dict<'p>>(b"AP") else {
            continue;
        };
        for key in [&b"N"[..], b"R", b"D"] {
            if let Some(s) = ap.get::<Stream<'p>>(key) {
                todo.push(Visit::Content(s));
            } else if let Some(states) = ap.get::<Dict<'p>>(key) {
                for k in states.keys().collect::<Vec<_>>() {
                    todo.extend(states.get::<Stream<'p>>(k.as_ref()).map(Visit::Content));
                }
            }
        }
    }
    let mut visits = 0usize;
    while let Some(v) = todo.pop() {
        visits += 1;
        if visits > VISIT_MAX {
            return Err(format!(
                "it reaches more than {VISIT_MAX} objects, too many to check before a render"
            ));
        }
        if visits.is_multiple_of(1024) && (facts.stop)() {
            return Err("the time limit was reached".into());
        }
        match v {
            Visit::Image(s) => {
                if !seen.insert(s.obj_id()) {
                    continue;
                }
                check(&s)?;
                if let Some(mask) = s.dict().get::<Stream<'p>>(b"Mask") {
                    check(&mask)?;
                }
            }
            Visit::Content(s) => {
                let id = s.obj_id();
                if !seen.insert(id) {
                    continue;
                }
                let size = match (u32::try_from(id.obj_number), u16::try_from(id.gen_number)) {
                    (Ok(n), Ok(g)) => (facts.sizes)(n, g),
                    _ => None,
                }
                .unwrap_or(s.raw_data().len() as u64);
                if size > room {
                    return Err(format!(
                        "its content stream {} {} unpacks to {size} bytes, more than the {room} \
                         bytes a render may take, so its pictures cannot be checked first",
                        id.obj_number, id.gen_number
                    ));
                }
                if let Ok(data) = s.decoded() {
                    let mut ops = TypedIter::new(&data);
                    while let Some(op) = ops.next() {
                        if let TypedInstruction::InlineImage(i) = op {
                            check(i.0)?;
                        }
                    }
                }
                if let Some(r) = s.dict().get::<Dict<'p>>(b"Resources") {
                    todo.push(Visit::Resources(Resources::new(r)));
                }
            }
            Visit::Resources(r) => {
                for k in r.x_objects.keys().collect::<Vec<_>>() {
                    let Some(x) = r.x_objects.get::<Stream<'p>>(k.as_ref()) else {
                        continue;
                    };
                    match x.dict().get::<Name<'_>>(b"Subtype").as_deref() {
                        Some(b"Image") => todo.push(Visit::Image(x)),
                        _ => todo.push(Visit::Content(x)),
                    }
                }
                for k in r.patterns.keys().collect::<Vec<_>>() {
                    todo.extend(r.patterns.get::<Stream<'p>>(k.as_ref()).map(Visit::Content));
                }
                for k in r.fonts.keys().collect::<Vec<_>>() {
                    let Some(font) = r.fonts.get::<Dict<'p>>(k.as_ref()) else {
                        continue;
                    };
                    if font.get::<Name<'_>>(b"Subtype").as_deref() != Some(b"Type3") {
                        continue;
                    }
                    if let Some(procs) = font.get::<Dict<'p>>(b"CharProcs") {
                        for g in procs.keys().collect::<Vec<_>>() {
                            todo.extend(procs.get::<Stream<'p>>(g.as_ref()).map(Visit::Content));
                        }
                    }
                    if let Some(res) = font.get::<Dict<'p>>(b"Resources") {
                        todo.push(Visit::Resources(Resources::new(res)));
                    }
                }
                for k in r.ext_g_states.keys().collect::<Vec<_>>() {
                    let group = r
                        .ext_g_states
                        .get::<Dict<'p>>(k.as_ref())
                        .and_then(|gs| gs.get::<Dict<'p>>(b"SMask"))
                        .and_then(|m| m.get::<Stream<'p>>(b"G"));
                    todo.extend(group.map(Visit::Content));
                }
            }
        }
    }
    Ok(())
}
