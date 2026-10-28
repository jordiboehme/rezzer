// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fields of drawn boxes: a box whose text is a label at its top or left and a value in
//! another style, boxes of fields next to each other in a row or a column, and the titles of
//! boxes beside them and of framed boxes over their lines.

use super::*;
use crate::layout::Slot;
use crate::regions::{Index, Seg, grids, regions_of};
use crate::tables::LABEL_STEP;
use crate::tables::banded::heads_lines;

/// The note before the first table of drawn boxes of a page.
pub(super) const BOX_FOUND_BY: &str = "Found by: label and value pairs from drawn boxes; the label \
     is the smaller text at the top of each box.";

/// Boxes of one row or column of fields touch within this share of the size of their
/// smallest text: farther apart, they are boxes on their own. Relative to the text, so a form
/// printed larger or smaller chains the same.
const TOUCH: f32 = 0.5;

/// Fields of drawn boxes (reading regions, in the frame of the runs): a box whose text is a
/// label at its top or left in one style (smaller by half a point or in another weight or
/// slant) and a value in another is a field, and a box with only a label smaller than the
/// page's largest text size an empty one. Fields of boxes next to each other in one row
/// (tops within [`SNAP`]) or one column (left edges within [`SNAP`]) make one table of two
/// columns, when one of them holds a value; a row of boxes that heads lines in its columns
/// below is a header band, not fields. Their runs leave `flow`. Also says, for boxes beside
/// fields that are not read as fields, why: a box of three styles or more, or one whose
/// label stands below its value. A box beside boxed fields whose text is bold alone is a
/// title: its runs leave `flow` as a [`BoxTitle`].
pub(crate) fn box_pairs(
    page: u32,
    flow: &mut Flow,
    boxes: &[[f32; 4]],
    rules: &[Rule],
    text_sizes: &[f32],
) -> (Vec<Planned>, Vec<BoxTitle>, Vec<String>) {
    if boxes.is_empty() || flow.runs.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let points: Vec<(f32, f32, f32)> = flow
        .runs
        .iter()
        .map(|r| {
            if r.off_axis {
                (f32::NAN, f32::NAN, 0.0)
            } else {
                (r.x + r.w.max(0.0) / 2.0, r.y + 0.25 * r.size, r.size)
            }
        })
        .collect();
    let Ok(regions) = regions_of(&points, boxes) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let mut in_box: Vec<Vec<usize>> = vec![Vec::new(); boxes.len()];
    for (i, r) in regions.iter().enumerate() {
        if let Some(b) = r {
            in_box[*b as usize].push(i);
            // The text of a box is a block of its own (see [`Slot::Block`]): a paragraph never
            // goes on from one box into the next (boxes of fields in a column whose labels and
            // values are of one style), nor into text outside its box. Box blocks count down
            // from the top of the numbers, apart from the blocks of cells read one by one.
            if let Some(slot) = flow.slots.get_mut(i)
                && *slot == Slot::Free
            {
                *slot = Slot::Block(usize::MAX - *b as usize);
            }
        }
    }
    // Framed boxes with a title over their lines, found by the frame: the frame's text is
    // what lies in it outside any box inside it with text.
    let framed = framed_titles(flow, boxes, &points, &regions, &in_box);
    let mut consumed = vec![false; boxes.len()];
    for f in &framed {
        for &b in &f.boxes {
            consumed[b] = true;
        }
    }
    let mut how = Reading {
        one_field: true,
        smaller: false,
        small: small_size(text_sizes).saturating_sub(1),
        labels: &[],
    };
    let mut fields: Vec<Option<Field>> = (0..boxes.len()).map(|_| None).collect();
    let mut unsure: Vec<(usize, &'static str)> = Vec::new();
    for (b, ids) in in_box.iter().enumerate() {
        if ids.is_empty() || consumed[b] {
            continue;
        }
        match parse(flow, ids, how) {
            Parsed::Fields(mut f) if f.len() == 1 => fields[b] = f.pop(),
            Parsed::Unsure(why) => unsure.push((b, why)),
            _ => {}
        }
    }
    // The style of the text alone in an empty boxed field is a label's: boxes whose label and
    // value differ in weight, slant or font only are read again knowing it.
    let mut known: Vec<Style> = fields
        .iter()
        .flatten()
        .filter(|f| f.value.is_empty())
        .filter_map(|f| f.label.first().map(|&i| style_of(&flow.runs[i])))
        .collect();
    known.sort_unstable();
    known.dedup();
    if !known.is_empty() && unsure.iter().any(|u| u.1 == WEIGHT_ONLY) {
        how.labels = &known;
        for (b, why) in &mut unsure {
            if *why != WEIGHT_ONLY {
                continue;
            }
            match parse(flow, &in_box[*b], how) {
                Parsed::Fields(mut f) if f.len() == 1 => {
                    fields[*b] = f.pop();
                    *why = "";
                }
                Parsed::Unsure(again) => *why = again,
                _ => *why = "",
            }
        }
        unsure.retain(|u| !u.1.is_empty());
    }
    // How near two boxes touch: by the size of their smallest text (see [`TOUCH`]).
    let touch: Vec<f32> = in_box
        .iter()
        .map(|ids| {
            let small = ids
                .iter()
                .filter(|&&i| inked(&flow.runs[i]))
                .map(|&i| flow.runs[i].size)
                .fold(f32::INFINITY, f32::min);
            if small.is_finite() {
                TOUCH * small
            } else {
                0.0
            }
        })
        .collect();
    let near = |p: usize, q: usize| {
        let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
        let row =
            (a[3] - b[3]).abs() <= t && ((a[2] - b[0]).abs() <= t || (b[2] - a[0]).abs() <= t);
        let column =
            (a[0] - b[0]).abs() <= t && ((a[1] - b[3]).abs() <= t || (b[1] - a[3]).abs() <= t);
        row || column
    };
    let inked_of = |b: usize| -> Vec<usize> {
        in_box[b]
            .iter()
            .copied()
            .filter(|&i| inked(&flow.runs[i]) && !flow.runs[i].off_axis)
            .collect()
    };
    // A box of one line or two in one style, not bold, beside a boxed field whose label has that
    // style and stands as far under its box's top, with nothing under it: the label of an empty
    // field (the page's text sizes may not show it as a label's when few values are filled in).
    let label_at = |b: usize, ids: &[usize]| -> Option<(Style, f32)> {
        let first = *ids
            .iter()
            .max_by(|&&x, &&y| flow.runs[x].y.total_cmp(&flow.runs[y].y))?;
        let r = &flow.runs[first];
        Some((style_of(r), boxes[b][3] - r.y))
    };
    // The label places of the field boxes, found once.
    let labelled: Vec<(usize, (Style, f32))> = (0..boxes.len())
        .filter_map(|f| {
            let field = fields[f].as_ref()?;
            Some((f, label_at(f, &field.label)?))
        })
        .collect();
    let mut empty: Vec<(usize, Vec<usize>)> = Vec::new();
    for b in 0..boxes.len() {
        if labelled.is_empty() {
            break;
        }
        let ids = inked_of(b);
        if fields[b].is_some() || consumed[b] || ids.is_empty() || unsure.iter().any(|u| u.0 == b) {
            continue;
        }
        let one = ids.iter().all(|&i| {
            style_of(&flow.runs[i]) == style_of(&flow.runs[ids[0]]) && !flow.runs[i].bold
        });
        if !one || lines_of(flow, &ids).len() > LABEL_LINES {
            continue;
        }
        let Some((style, offset)) = label_at(b, &ids) else {
            continue;
        };
        let size = flow.runs[ids[0]].size;
        let beside = labelled
            .iter()
            .any(|&(f, (s, o))| near(b, f) && s == style && (o - offset).abs() <= 0.5 * size);
        if beside {
            empty.push((b, ids));
        }
    }
    for (b, ids) in empty {
        fields[b] = Some(Field {
            label: lines_of(flow, &ids).into_iter().flatten().collect(),
            value: Vec::new(),
        });
    }
    // Every other box is a box of its row or column, with no field: an address, a title, text
    // of one style with no label, a box not clear, a box with no text. The fields on both sides
    // of it stay one row; one with text is said or read as a title below.
    let bridge: Vec<bool> = (0..boxes.len())
        .map(|b| fields[b].is_none() && !consumed[b])
        .collect();
    let two_fields = |c: &[usize]| c.iter().filter(|&&b| fields[b].is_some()).count() >= 2;
    // Rows first: boxes left to right, each going on the row whose last box it touches.
    let mut order: Vec<usize> = (0..boxes.len())
        .filter(|&b| fields[b].is_some() || bridge[b])
        .collect();
    order.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
    // A box with no field joins the fields on both sides of it: one at an end of its row
    // joins nothing, so it is no part of the row (the lines under a title there are its own).
    let chains = |order: &[usize], next: &dyn Fn(usize, usize) -> bool| {
        let mut out: Vec<Vec<usize>> = Vec::new();
        for &b in order {
            match out.iter_mut().find(|c| next(*c.last().expect("a box"), b)) {
                Some(c) => c.push(b),
                None => out.push(vec![b]),
            }
        }
        out.into_iter()
            .filter_map(|c| {
                let first = c.iter().position(|&b| fields[b].is_some())?;
                let last = c.iter().rposition(|&b| fields[b].is_some())?;
                Some(c[first..=last].to_vec())
            })
            .collect::<Vec<_>>()
    };
    let has_value = |c: &[usize]| {
        c.iter()
            .any(|&b| fields[b].as_ref().is_some_and(|f| !f.value.is_empty()))
    };
    // The page's lines, once needed: a box drawn with no text in it is no reading region, so
    // the boxes beside it touch it, not each other; the lines show it drawn.
    let lines = std::cell::OnceCell::new();
    // Sorted by their place across, so the lines near a place are found by a search.
    let lines = || {
        lines.get_or_init(|| {
            let (mut hs, mut vs) = grids(rules).map(|g| (g.hs, g.vs)).unwrap_or_default();
            hs.sort_by(|a, b| a.at.total_cmp(&b.at));
            vs.sort_by(|a, b| a.at.total_cmp(&b.at));
            (hs, vs)
        })
    };
    // Whether lines `segs` at `at` (within `t`) cover the stretch `lo..hi`.
    let covered = |segs: &[Seg], at: f32, lo: f32, hi: f32, t: f32| {
        let mut end = lo;
        let tol = t.max(SNAP);
        let from = segs.partition_point(|s| s.at < at - tol);
        let mut parts: Vec<&Seg> = segs[from..]
            .iter()
            .take_while(|s| s.at <= at + tol)
            .filter(|s| s.to > lo && s.from < hi)
            .collect();
        parts.sort_by(|a, b| a.from.total_cmp(&b.from));
        for s in parts {
            if s.from > end + t.max(SNAP) {
                return false;
            }
            end = end.max(s.to);
        }
        end >= hi - t.max(SNAP)
    };
    // Two boxes of a row with an empty drawn box between them: its top and bottom drawn across
    // the gap. Two of a column: its sides drawn down the gap.
    // The gap is at most as wide as twice the wider box: an empty box beside boxes like it.
    let empty_row = |a: [f32; 4], b: [f32; 4], t: f32| {
        let gap = b[0] - a[2];
        if gap <= t || gap > 2.0 * (a[2] - a[0]).max(b[2] - b[0]) {
            return false;
        }
        let (hs, _) = lines();
        covered(hs, a[3], a[2], b[0], t) && covered(hs, a[1].max(b[1]), a[2], b[0], t)
    };
    let empty_column = |a: [f32; 4], b: [f32; 4], t: f32| {
        let gap = a[1] - b[3];
        if gap <= t || gap > 2.0 * (a[3] - a[1]).max(b[3] - b[1]) {
            return false;
        }
        let (_, vs) = lines();
        covered(vs, a[0], b[3], a[1], t) && covered(vs, a[2], b[3], a[1], t)
    };
    let mut hs: Option<Vec<Seg>> = None;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut used = vec![false; boxes.len()];
    for c in chains(&order, &|p, q| {
        let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
        (a[3] - b[3]).abs() <= t && ((a[2] - b[0]).abs() <= t || empty_row(a, b, t))
    }) {
        if !two_fields(&c) || !has_value(&c) {
            continue;
        }
        // A row of boxes over lines in its columns heads a table.
        let mut xs: Vec<f32> = c.iter().map(|&b| boxes[b][0]).collect();
        xs.push(boxes[c[c.len() - 1]][2]);
        let b = [
            xs[0],
            c.iter().map(|&b| boxes[b][1]).fold(f32::MAX, f32::min),
            xs[xs.len() - 1],
            c.iter().map(|&b| boxes[b][3]).fold(f32::MIN, f32::max),
        ];
        for &k in &c {
            used[k] = true;
        }
        let hs = hs.get_or_insert_with(|| grids(rules).map(|g| g.hs).unwrap_or_default());
        if !heads_lines(flow, hs, b, &xs) {
            groups.push(c);
        }
    }
    let mut rest: Vec<usize> = order.into_iter().filter(|&b| !used[b]).collect();
    rest.sort_by(|&a, &b| boxes[b][3].total_cmp(&boxes[a][3]));
    for c in chains(&rest, &|p, q| {
        let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
        (a[0] - b[0]).abs() <= t && ((a[1] - b[3]).abs() <= t || empty_column(a, b, t))
    }) {
        if two_fields(&c) && has_value(&c) {
            groups.push(c);
        }
    }
    // A box that is no field beside the boxes of fields says why; so do boxes next to each
    // other whose label and value differ in weight only, which are fields but for that.
    let paired: Vec<usize> = groups
        .iter()
        .flatten()
        .copied()
        .filter(|&b| fields[b].is_some())
        .collect();
    // A box of bold text alone beside the fields is a title (see below); other text of one
    // style with no label beside them is said, by its first line.
    let title_box = |b: usize| {
        let ids: Vec<usize> = in_box[b]
            .iter()
            .copied()
            .filter(|&i| inked(&flow.runs[i]))
            .collect();
        !ids.is_empty()
            && ids
                .iter()
                .all(|&i| flow.runs[i].bold && !flow.runs[i].off_axis)
            && lines_of(flow, &ids).len() <= LABEL_LINES
    };
    for (b, &bridged) in bridge.iter().enumerate() {
        if bridged
            && !inked_of(b).is_empty()
            && !title_box(b)
            && !unsure.iter().any(|u| u.0 == b)
            && paired.iter().any(|&p| near(b, p))
        {
            unsure.push((b, NO_LABEL));
        }
    }
    let said_of = |b: &usize, why: &&str| {
        paired.iter().any(|&p| near(*b, p))
            || ((*why == WEIGHT_ONLY || *why == ADDRESS)
                && unsure
                    .iter()
                    .any(|(o, w)| o != b && *w == *why && near(*b, *o)))
    };
    let reasons: Vec<&str> = unsure
        .iter()
        .filter(|(b, why)| said_of(b, why))
        .map(|(_, why)| *why)
        .collect();
    let mut said: Vec<String> = Vec::new();
    // A box of an address, or of text with no label, is named by its first line.
    for (kinds, why) in [
        (&[ADDRESS, NAMED_ADDRESS][..], ADDRESS),
        (&[NO_LABEL][..], NO_LABEL),
    ] {
        let names: Vec<String> = unsure
            .iter()
            .filter(|(b, w)| kinds.contains(w) && said_of(b, w))
            .filter_map(|(b, _)| {
                lines_of(flow, &inked_of(*b))
                    .first()
                    .map(|l| words(flow, l))
            })
            .collect();
        if names.is_empty() {
            continue;
        }
        let (s, verb) = if names.len() == 1 {
            ("", "is")
        } else {
            ("es", "are")
        };
        said.push(format!(
            "the drawn box{s} {} beside label and value fields on page {page} {verb} not read as \
             a label and its value ({why}): the text is read as it stands",
            names.join(", ")
        ));
    }
    for why in [THREE_STYLES, LABEL_BELOW, WEIGHT_ONLY] {
        let n = reasons.iter().filter(|w| **w == why).count();
        if n > 0 {
            let (s, verb) = if n == 1 { ("", "is") } else { ("es", "are") };
            let place = if why == WEIGHT_ONLY {
                "of label and value fields"
            } else {
                "beside label and value fields"
            };
            said.push(format!(
                "{n} drawn box{s} {place} on page {page} {verb} not read as a label and its \
                 value ({why}): the text is read as it stands"
            ));
        }
    }
    if groups.is_empty() && framed.is_empty() {
        return (Vec::new(), Vec::new(), said);
    }
    let mut planned: Vec<Planned> = Vec::new();
    let mut target: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    for c in &groups {
        let list: Vec<&Field> = c.iter().filter_map(|&b| fields[b].as_ref()).collect();
        let top = c.iter().map(|&b| boxes[b][3]).fold(f32::MIN, f32::max);
        if let Some(p) = pairs_table(flow, &list, top, BOX_FOUND_BY) {
            claim(&mut target, planned.len(), &list);
            planned.push(p);
        }
    }
    // Boxes beside the fields that hold bold text alone are titles, and so are the titles of
    // framed boxes, with their text.
    let mut titles: Vec<BoxTitle> = Vec::new();
    let mut title_of: Vec<Option<(usize, bool)>> = vec![None; flow.runs.len()];
    for f in &framed {
        for &i in &f.title {
            title_of[i] = Some((titles.len(), false));
        }
        for &i in &f.text {
            title_of[i] = Some((titles.len(), true));
        }
        let first = f.title.iter().chain(&f.text).min().copied().unwrap_or(0);
        titles.push(BoxTitle {
            at: flow.at[first],
            top: f.top,
            runs: Vec::new(),
            text: Vec::new(),
        });
    }
    for (b, ids) in in_box.iter().enumerate() {
        if consumed[b] {
            continue;
        }
        if fields[b].is_some() || !title_box(b) || !paired.iter().any(|&p| near(b, p)) {
            continue;
        }
        for &i in ids {
            title_of[i] = Some((titles.len(), false));
        }
        titles.push(BoxTitle {
            at: flow.at[ids[0]],
            top: boxes[b][3],
            runs: Vec::new(),
            text: Vec::new(),
        });
    }
    // The runs of the tables and the titles leave the flow at once.
    let picked: Vec<usize> = (0..flow.runs.len())
        .filter(|&i| target[i].is_some() || title_of[i].is_some())
        .collect();
    let taken = flow.take(|i, _| target[i].is_some() || title_of[i].is_some());
    for ((run, _, _), i) in taken.into_iter().zip(picked) {
        match (target[i], title_of[i]) {
            (Some((t, r, c)), _) => planned[t].rows[r][c].runs.push(run),
            (None, Some((k, false))) => titles[k].runs.push(run),
            (None, Some((k, true))) => titles[k].text.push(run),
            (None, None) => {}
        }
    }
    (planned, titles, said)
}

/// A framed box read as a title over its lines: the boxes it takes (the frame and a title
/// box in it), the runs of its title and of its text, and the frame's top.
struct Framed {
    boxes: Vec<usize>,
    title: Vec<usize>,
    text: Vec<usize>,
    top: f32,
}

/// The framed boxes of a page that hold a title over lines of text: a bold line or two at the
/// top of the frame (or alone in a box in the frame above its lines), smaller than the lines
/// under it, which are in one style, none of them bold, two lines or more. The frame's own
/// text is what lies in it outside its boxes with text: runs `regions` gives no box (the
/// frame holds a box with text, so it is no region of its own) whose smallest holder it is,
/// and runs of the frame itself. `points` are the runs' ink middles and sizes.
fn framed_titles(
    flow: &Flow,
    boxes: &[[f32; 4]],
    points: &[(f32, f32, f32)],
    regions: &[Option<u32>],
    in_box: &[Vec<usize>],
) -> Vec<Framed> {
    let Some(mut index) = Index::new(boxes) else {
        return Vec::new();
    };
    let area = |b: [f32; 4]| (b[2] - b[0]) * (b[3] - b[1]);
    let mut own: Vec<Vec<usize>> = in_box.to_vec();
    for (i, r) in regions.iter().enumerate() {
        if r.is_some() || !inked(&flow.runs[i]) {
            continue;
        }
        let (x, y, size) = points[i];
        let mut best: Option<u32> = None;
        index.holders(x, y, size, |b| {
            if best.is_none_or(|o| area(boxes[b as usize]) < area(boxes[o as usize])) {
                best = Some(b);
            }
        });
        if index.work > crate::regions::BOX_WORK {
            return Vec::new();
        }
        if let Some(b) = best {
            own[b as usize].push(i);
        }
    }
    let bold_lines = |ids: &[usize]| -> Option<Vec<Vec<usize>>> {
        let ids: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|&i| inked(&flow.runs[i]))
            .collect();
        let lines = lines_of(flow, &ids);
        (!lines.is_empty()
            && lines.len() <= LABEL_LINES
            && lines.iter().flatten().all(|&i| flow.runs[i].bold))
        .then_some(lines)
    };
    // The text under a title: lines of one style, none bold, larger than the title.
    let text_of = |lines: &[Vec<usize>], title_size: f32| {
        let mut styles: Vec<Style> = lines
            .iter()
            .flatten()
            .map(|&i| style_of(&flow.runs[i]))
            .collect();
        styles.sort_unstable();
        styles.dedup();
        lines.len() >= 2
            && styles.len() == 1
            && !styles[0].1
            && styles[0].0 as f32 / 2.0 >= LABEL_STEP * title_size
    };
    let mut out = Vec::new();
    for (b, ids) in own.iter().enumerate() {
        let mut ids = ids.clone();
        ids.retain(|&i| inked(&flow.runs[i]) && !flow.runs[i].off_axis);
        if ids.is_empty() {
            continue;
        }
        let fb = boxes[b];
        let lines = lines_of(flow, &ids);
        let size = |l: &Vec<usize>| l.iter().map(|&i| flow.runs[i].size).fold(0.0f32, f32::max);
        // The title at the top of the frame itself.
        let lead = lines
            .iter()
            .take_while(|l| l.iter().all(|&i| flow.runs[i].bold))
            .count();
        if (1..=LABEL_LINES).contains(&lead) {
            let title_size = lines[..lead].iter().map(size).fold(0.0f32, f32::max);
            if text_of(&lines[lead..], title_size) {
                out.push(Framed {
                    boxes: vec![b],
                    title: lines[..lead].iter().flatten().copied().collect(),
                    text: lines[lead..].iter().flatten().copied().collect(),
                    top: fb[3],
                });
                continue;
            }
        }
        // A title in a box of its own at the top of the frame.
        if lead > 0 {
            continue;
        }
        // The top of the frame's own lines: the title box stands above them.
        let lines_top = ids
            .iter()
            .map(|&i| flow.runs[i].y + 0.75 * flow.runs[i].size)
            .fold(f32::MIN, f32::max);
        let inner = (0..boxes.len()).find(|&t| {
            let tb = boxes[t];
            t != b
                && tb[0] >= fb[0] - SNAP
                && tb[2] <= fb[2] + SNAP
                && tb[3] <= fb[3] + SNAP
                && tb[1] >= lines_top - SNAP
                && bold_lines(&own[t]).is_some()
        });
        let Some(t) = inner else {
            continue;
        };
        let title = bold_lines(&own[t]).unwrap_or_default();
        let title_size = title.iter().map(size).fold(0.0f32, f32::max);
        if text_of(&lines, title_size) {
            out.push(Framed {
                boxes: vec![b, t],
                title: title.into_iter().flatten().collect(),
                text: ids,
                top: fb[3],
            });
        }
    }
    out
}
