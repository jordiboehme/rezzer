// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fields of drawn boxes: a box whose text is a label at its top or left and a value in
//! another style, boxes of fields next to each other in a row or a column, and the titles of
//! boxes beside them and of framed boxes over their lines.

use layoutregions::segment::drawn::{Grids, Line, LinesAt, Unsearched};

use super::*;
use crate::regions::{Index, regions_of};
use crate::tables::LABEL_STEP;
use crate::tables::banded::{Heads, heads_lines};
use layoutregions::extract::pairs::{touch_reach, touching};

pub(super) use crate::found::BOX_FOUND_BY;

/// What [`box_pairs`] reads: the tables and titles, the losses of the boxes read as they stand
/// with their runs (see [`Doubts`]), and why the page was looked at only in part.
pub(crate) type BoxesRead = (
    Vec<Planned>,
    Vec<BoxTitle>,
    (Vec<String>, Doubts),
    Option<String>,
);

/// Fields of drawn boxes (reading regions, in the frame of the runs): a box whose text is a
/// label at its top or left in one style (smaller by half a point or in another weight or
/// slant) and a value in another is a field, and a box with only a label smaller than the
/// page's largest text size an empty one. Fields of boxes next to each other in one row
/// (tops within [`SNAP`]) or one column (left edges within [`SNAP`]) make one table of two
/// columns, when one of them holds a value; a row of boxes that heads lines in its columns
/// below is a header band, not fields. Their runs leave `flow`. Also says, for boxes beside
/// fields that are not read as fields, why: a box of three styles or more, or one whose
/// label stands below its value. A box beside boxed fields whose text is bold alone is a
/// title: its runs leave `flow` as a [`BoxTitle`]. Whether rows of boxes head lines is looked
/// at within one work bound for the page and until `stop`: past either, the rows left are
/// read as text and the last value says why.
pub(crate) fn box_pairs(
    page: u32,
    flow: &mut Flow,
    boxes: &[[f32; 4]],
    grids: Result<&Grids, Unsearched>,
    text_sizes: &[f32],
    stop: &dyn Fn() -> bool,
) -> BoxesRead {
    if boxes.is_empty() || flow.runs.is_empty() {
        return (Vec::new(), Vec::new(), (Vec::new(), Vec::new()), None);
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
        return (Vec::new(), Vec::new(), (Vec::new(), Vec::new()), None);
    };
    let mut in_box: Vec<Vec<usize>> = vec![Vec::new(); boxes.len()];
    for (i, r) in regions.iter().enumerate() {
        if let Some(b) = r {
            in_box[*b as usize].push(i);
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
    let (mut fields, mut unsure) = box_fields(&flow.runs, &in_box, &consumed, text_sizes);
    // How near two boxes touch: by the size of their smallest text (see `touch_reach`).
    let touch: Vec<f32> = in_box
        .iter()
        .map(|ids| touch_reach(&flow.runs, ids))
        .collect();
    let near = |p: usize, q: usize| touching(boxes[p], boxes[q], touch[p].max(touch[q]));
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
        if !one || lines_of(&flow.runs, &ids).len() > LABEL_LINES {
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
            label: lines_of(&flow.runs, &ids).into_iter().flatten().collect(),
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
    let with_fields = |chains: Vec<Vec<usize>>| {
        chains
            .into_iter()
            .filter_map(|c| {
                let first = c.iter().position(|&b| fields[b].is_some())?;
                let last = c.iter().rposition(|&b| fields[b].is_some())?;
                Some(c[first..=last].to_vec())
            })
            .collect::<Vec<_>>()
    };
    // The widest touch of two boxes, and the largest width and height of a box: what the
    // search for the chain of a box looks at (see `chain_boxes`).
    let reach = touch.iter().copied().fold(0.0f32, f32::max);
    let longest =
        |from: usize, to: usize| boxes.iter().map(|b| b[to] - b[from]).fold(0.0f32, f32::max);
    let (widest, tallest) = (longest(0, 2), longest(1, 3));
    let has_value = |c: &[usize]| {
        c.iter()
            .any(|&b| fields[b].as_ref().is_some_and(|f| !f.value.is_empty()))
    };
    // The page's lines: a box drawn with no text in it is no reading region, so the boxes
    // beside it touch it, not each other; the lines show it drawn. They are sorted by their
    // place across, so the lines near a place are found by a search.
    let (hs, vs): (&[Line], &[Line]) = match grids {
        Ok(g) => (&g.hs, &g.vs),
        Err(_) => (&[], &[]),
    };
    // Whether the flat lines, or the upright ones, at `at` (within `t`) cover the stretch
    // `lo..hi`.
    let (flat, upright) = (Cover::new(hs), Cover::new(vs));
    let covered = |segs: &Cover<'_>, at: f32, lo: f32, hi: f32, t: f32| segs.covers(at, lo, hi, t);
    // Two boxes of a row with an empty drawn box between them: its top and bottom drawn across
    // the gap. Two of a column: its sides drawn down the gap.
    // The gap is at most as wide as twice the wider box: an empty box beside boxes like it.
    let empty_row = |a: [f32; 4], b: [f32; 4], t: f32| {
        let gap = b[0] - a[2];
        if gap <= t || gap > 2.0 * (a[2] - a[0]).max(b[2] - b[0]) {
            return false;
        }
        covered(&flat, a[3], a[2], b[0], t) && covered(&flat, a[1].max(b[1]), a[2], b[0], t)
    };
    let empty_column = |a: [f32; 4], b: [f32; 4], t: f32| {
        let gap = a[1] - b[3];
        if gap <= t || gap > 2.0 * (a[3] - a[1]).max(b[3] - b[1]) {
            return false;
        }
        covered(&upright, a[0], b[3], a[1], t) && covered(&upright, a[2], b[3], a[1], t)
    };
    // The page's lines, built once for every row, and the work they take for the page.
    let mut heads: Option<Heads> = None;
    let mut work = 0usize;
    let mut spent: Option<&str> = None;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut used = vec![false; boxes.len()];
    // Rows: boxes with their tops at one height, each starting where the one before it ends
    // or past an empty drawn box, which is at most twice as wide as the wider of the two.
    let rows = chain_boxes(
        &order,
        &|b| boxes[b][3],
        reach,
        &|p, q| boxes[q][0] - boxes[p][2],
        &|p| reach.max(2.0 * (boxes[p][2] - boxes[p][0]).max(widest)),
        &|p, q| {
            let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
            (a[3] - b[3]).abs() <= t && ((a[2] - b[0]).abs() <= t || empty_row(a, b, t))
        },
    );
    for c in with_fields(rows) {
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
        // Past the time limit or the work bound, a row is read as text: no pairs.
        if spent.is_none() && stop() {
            spent = Some("the time limit was reached");
        }
        if spent.is_some() {
            continue;
        }
        let heads = heads.get_or_insert_with(|| Heads::new(flow));
        match heads_lines(flow, heads, hs, b, &xs, &mut work) {
            Some(false) => groups.push(c),
            Some(true) => {}
            None => spent = Some("the page has too much text below its rows of boxes"),
        }
    }
    let skipped = spent.map(|why| {
        format!(
            "label and value pairs in drawn boxes on page {page} are looked for only in part: \
             {why}; the rows of boxes past that are read as text"
        )
    });
    let mut rest: Vec<usize> = order.into_iter().filter(|&b| !used[b]).collect();
    rest.sort_by(|&a, &b| boxes[b][3].total_cmp(&boxes[a][3]));
    // Columns: boxes with their left edges at one place, each starting where the one over it
    // ends or under an empty drawn box, which is at most twice as tall as the taller of the
    // two.
    let columns = chain_boxes(
        &rest,
        &|b| boxes[b][0],
        reach,
        &|p, q| boxes[p][1] - boxes[q][3],
        &|p| reach.max(2.0 * (boxes[p][3] - boxes[p][1]).max(tallest)),
        &|p, q| {
            let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
            (a[0] - b[0]).abs() <= t && ((a[1] - b[3]).abs() <= t || empty_column(a, b, t))
        },
    );
    for c in with_fields(columns) {
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
            && lines_of(&flow.runs, &ids).len() <= LABEL_LINES
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
    // Boxes side by side or one under the other that each hold a short line over a line, all
    // in one style, look like fields whose label is not set apart from its value: they are
    // read box by box, and that is said.
    let one_style = |b: usize| {
        let ids = inked_of(b);
        let lines = lines_of(&flow.runs, &ids);
        let first = lines
            .first()
            .map(|l| words(&flow.runs, l))
            .unwrap_or_default();
        lines.len() == 2
            && fields[b].as_ref().is_none_or(|f| f.value.is_empty())
            && !unsure.iter().any(|u| u.0 == b)
            && !paired.contains(&b)
            && first.chars().count() <= LABEL_CHARS
            && first.split_whitespace().count() <= LABEL_WORDS
            && ids
                .windows(2)
                .all(|w| same_style(&flow.runs[w[0]], &flow.runs[w[1]]))
    };
    let alike: Vec<usize> = (0..boxes.len()).filter(|&b| one_style(b)).collect();
    let mut lone: Vec<usize> = alike
        .iter()
        .copied()
        .filter(|&b| alike.iter().any(|&o| o != b && near(b, o)))
        .collect();
    // In reading order: top down, then left to right.
    lone.sort_by(|&p, &q| {
        boxes[q][3]
            .total_cmp(&boxes[p][3])
            .then(boxes[p][0].total_cmp(&boxes[q][0]))
    });
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
    // The boxes said, with the ids of their runs, which are read as they stand.
    let ids_of = |b: usize| -> Vec<u32> { in_box[b].iter().map(|&i| flow.ids[i]).collect() };
    let mut doubts: Doubts = unsure
        .iter()
        .filter(|(b, why)| said_of(b, why))
        .map(|&(b, why)| (ids_of(b), why))
        .collect();
    doubts.extend(lone.iter().map(|&b| (ids_of(b), NO_LABEL)));
    // A box of an address, or of text with no label, is named by its first line.
    for (kinds, why) in [
        (&[ADDRESS, NAMED_ADDRESS][..], ADDRESS),
        (&[NO_LABEL][..], NO_LABEL),
    ] {
        let names: Vec<String> = unsure
            .iter()
            .filter(|(b, w)| kinds.contains(w) && said_of(b, w))
            .filter_map(|(b, _)| {
                lines_of(&flow.runs, &inked_of(*b))
                    .first()
                    .map(|l| words(&flow.runs, l))
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
    let names: Vec<String> = lone
        .iter()
        .filter_map(|&b| {
            lines_of(&flow.runs, &inked_of(b))
                .first()
                .map(|l| words(&flow.runs, l))
        })
        .collect();
    if !names.is_empty() {
        said.push(format!(
            "the drawn boxes {} on page {page} are not read as labels and their values \
             ({NO_LABEL}): the text of each box is read as it stands",
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
        return (Vec::new(), Vec::new(), (said, doubts), skipped);
    }
    let mut planned: Vec<Planned> = Vec::new();
    let mut target: Vec<Option<(usize, usize, usize)>> = vec![None; flow.runs.len()];
    for c in &groups {
        let list: Vec<&Field> = c.iter().filter_map(|&b| fields[b].as_ref()).collect();
        let top = c.iter().map(|&b| boxes[b][3]).fold(f32::MIN, f32::max);
        if let Some(mut p) = pairs_table(flow, &list, top, (BOX_FOUND_BY, Detector::BoxPairs)) {
            p.drawn = union(c.iter().map(|&b| boxes[b]));
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
            ids: Vec::new(),
            text_ids: Vec::new(),
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
            ids: Vec::new(),
            text_ids: Vec::new(),
        });
    }
    // The runs of the tables and the titles leave the flow at once.
    let picked: Vec<usize> = (0..flow.runs.len())
        .filter(|&i| target[i].is_some() || title_of[i].is_some())
        .collect();
    let taken = flow.take(|i, _| target[i].is_some() || title_of[i].is_some());
    for (taken, i) in taken.into_iter().zip(picked) {
        let (run, id) = (taken.run, taken.id);
        match (target[i], title_of[i]) {
            (Some((t, r, c)), _) => {
                planned[t].rows[r][c].runs.push(run);
                planned[t].ids.push(id);
            }
            (None, Some((k, false))) => {
                titles[k].runs.push(run);
                titles[k].ids.push(id);
            }
            (None, Some((k, true))) => {
                titles[k].text.push(run);
                titles[k].ids.push(id);
                titles[k].text_ids.push(id);
            }
            (None, None) => {}
        }
    }
    (planned, titles, (said, doubts), skipped)
}

/// The lines of a page in one direction, by their place and then their start, to ask
/// whether those at one place cover a stretch.
struct Cover<'a> {
    segs: &'a [Line],
    /// The lines for the search, made when the first stretch is asked for.
    lines: std::cell::OnceCell<LinesAt<'a>>,
}

impl<'a> Cover<'a> {
    fn new(segs: &'a [Line]) -> Cover<'a> {
        Cover {
            segs,
            lines: std::cell::OnceCell::new(),
        }
    }

    /// Whether the lines at `at` (within `t`, at least [`SNAP`]) cover the stretch `lo..hi`:
    /// those that reach into it, by their start, leave no gap wider than that at its start,
    /// between them or at its end. Only the lines that reach into the stretch are looked at
    /// (see [`LinesAt`]): a column of some thousand boxes has as many lines at one place,
    /// and each pair of boxes asked for all of them.
    fn covers(&self, at: f32, lo: f32, hi: f32, t: f32) -> bool {
        let tol = t.max(SNAP);
        let lines = self.lines.get_or_init(|| LinesAt::new(self.segs));
        let mut parts: Vec<&Line> = Vec::new();
        let segs = self.segs;
        lines.reaching((at - tol, at + tol), (lo, hi), |k, _| parts.push(&segs[k]));
        parts.sort_by(|a, b| a.from.total_cmp(&b.from));
        let mut end = lo;
        for s in parts {
            if s.from > end + tol {
                return false;
            }
            end = end.max(s.to);
        }
        end >= hi - tol
    }
}

/// Puts the boxes of `order` on chains, in that order: each goes on the first chain made
/// whose last box `next` joins it to, else it starts a chain. The chains come in the order
/// they were started.
///
/// So that a box is not tested against the end of every chain (a page of some thousand boxes
/// in rows or columns, read once for each of its regions, took billions of tests), the ends
/// are kept by their place across the chain (`key`: the top of a row, the left edge of a
/// column) and a box looks only at the ends near its own. What the caller states of `next`:
/// it joins only two boxes whose keys are at most `reach` apart; `gap(p, q)` is how far `q`
/// starts past the end of `p` along the chain, which only grows for one `p` over the boxes
/// of `order` after it; and `next` joins nothing to `p` past a gap of `far(p)`. An end past
/// its `far` is dropped from the search for good. A key, a gap or a bound that is no
/// number never drops or hides an end: such a box is tested as before.
fn chain_boxes(
    order: &[usize],
    key: &dyn Fn(usize) -> f32,
    reach: f32,
    gap: &dyn Fn(usize, usize) -> f32,
    far: &dyn Fn(usize) -> f32,
    next: &dyn Fn(usize, usize) -> bool,
) -> Vec<Vec<usize>> {
    // The cells of the keys are at least a snap wide, so keys within `reach` of each other
    // stand at most one cell apart; one more cell on each side covers the rounding of the
    // division. A reach that is no number makes one cell of all.
    let width = if reach.is_finite() {
        reach.max(SNAP)
    } else {
        f32::INFINITY
    };
    // Ends with a key that is no number are looked at by every box.
    const LOOSE: i64 = i64::MIN;
    let cell = |b: usize| -> i64 {
        let k = key(b);
        if k.is_finite() {
            ((k / width).floor() as i64).clamp(i64::MIN + 3, i64::MAX - 2)
        } else {
            LOOSE
        }
    };
    let mut out: Vec<Vec<usize>> = Vec::new();
    // The chains that may still grow, by the cell of their last box.
    let mut ends: HashMap<i64, Vec<usize>> = HashMap::new();
    for &b in order {
        crate::work::add(1);
        let at = cell(b);
        let mut best: Option<usize> = None;
        let mut look = |cells: &mut HashMap<i64, Vec<usize>>, c: i64| {
            let Some(list) = cells.get_mut(&c) else {
                return;
            };
            list.retain(|&chain| {
                crate::work::add(1);
                let last = *out[chain].last().expect("a box");
                if gap(last, b) > far(last) {
                    return false;
                }
                if best.is_none_or(|o| chain < o) && next(last, b) {
                    best = Some(chain);
                }
                true
            });
            if list.is_empty() {
                cells.remove(&c);
            }
        };
        if at == LOOSE {
            let all: Vec<i64> = ends.keys().copied().collect();
            for c in all {
                look(&mut ends, c);
            }
        } else {
            for c in at - 2..=at + 2 {
                look(&mut ends, c);
            }
            look(&mut ends, LOOSE);
        }
        match best {
            Some(chain) => {
                let from = cell(*out[chain].last().expect("a box"));
                if from != at {
                    if let Some(list) = ends.get_mut(&from) {
                        list.retain(|&c| c != chain);
                    }
                    ends.entry(at).or_default().push(chain);
                }
                out[chain].push(b);
            }
            None => {
                ends.entry(at).or_default().push(out.len());
                out.push(vec![b]);
            }
        }
    }
    out
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
        let lines = lines_of(&flow.runs, &ids);
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
        let lines = lines_of(&flow.runs, &ids);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Numbers that repeat for one seed, in `0..n`.
    struct Dice(u64);

    impl Dice {
        fn roll(&mut self, n: u32) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) % u64::from(n)) as f32
        }
    }

    /// The chains as they were made before Task 37: each box tested against the end of
    /// every chain.
    fn chains_one_by_one(order: &[usize], next: &dyn Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
        let mut out: Vec<Vec<usize>> = Vec::new();
        for &b in order {
            match out.iter_mut().find(|c| next(*c.last().expect("a box"), b)) {
                Some(c) => c.push(b),
                None => out.push(vec![b]),
            }
        }
        out
    }

    /// Whether lines cover a stretch, as it was asked before Task 37: every line at the
    /// place looked at.
    fn covered_one_by_one(segs: &[Line], at: f32, lo: f32, hi: f32, t: f32) -> bool {
        let mut end = lo;
        let tol = t.max(SNAP);
        let from = segs.partition_point(|s| s.at < at - tol);
        let mut parts: Vec<&Line> = segs[from..]
            .iter()
            .take_while(|s| s.at <= at + tol)
            .filter(|s| s.to > lo && s.from < hi)
            .collect();
        parts.sort_by(|a, b| a.from.total_cmp(&b.from));
        for s in parts {
            if s.from > end + tol {
                return false;
            }
            end = end.max(s.to);
        }
        end >= hi - tol
    }

    /// Task 37: the search by the ends near a box makes the chains the test of every end
    /// made, for rows and for columns, on boxes that touch, overlap, stand apart and stand
    /// in each other, with a touch of nothing, of a little and of much.
    #[test]
    fn chains_by_near_ends_are_the_chains_of_every_end() {
        for seed in 0..40u64 {
            let mut dice = Dice(seed);
            let n = 60 + seed as usize;
            let boxes: Vec<[f32; 4]> = (0..n)
                .map(|_| {
                    let (x, y) = (10.0 * dice.roll(12), 10.0 * dice.roll(12));
                    let (w, h) = (10.0 * (1.0 + dice.roll(3)), 10.0 * (1.0 + dice.roll(3)));
                    // Some boxes a little off the grid, within a touch or just past it.
                    let off = 0.5 * dice.roll(5);
                    [x + off, y, x + off + w, y + h]
                })
                .collect();
            let touch: Vec<f32> = (0..n)
                .map(|_| [0.0, 1.0, 2.5][dice.roll(3) as usize])
                .collect();
            let reach = touch.iter().copied().fold(0.0f32, f32::max);
            let widest = boxes.iter().map(|b| b[2] - b[0]).fold(0.0f32, f32::max);
            let tallest = boxes.iter().map(|b| b[3] - b[1]).fold(0.0f32, f32::max);
            // A gap up to twice the wider box joins when the dice say a line is drawn over
            // it: by the two boxes alone, as the lines of a page are.
            let drawn = |p: usize, q: usize| (p * 31 + q * 17 + seed as usize).is_multiple_of(3);
            let row = |p: usize, q: usize| {
                let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
                let gap = b[0] - a[2];
                let over = gap > t && gap <= 2.0 * (a[2] - a[0]).max(b[2] - b[0]) && drawn(p, q);
                (a[3] - b[3]).abs() <= t && ((a[2] - b[0]).abs() <= t || over)
            };
            let column = |p: usize, q: usize| {
                let (a, b, t) = (boxes[p], boxes[q], touch[p].max(touch[q]));
                let gap = a[1] - b[3];
                let over = gap > t && gap <= 2.0 * (a[3] - a[1]).max(b[3] - b[1]) && drawn(p, q);
                (a[0] - b[0]).abs() <= t && ((a[1] - b[3]).abs() <= t || over)
            };
            let mut across: Vec<usize> = (0..n).collect();
            across.sort_by(|&a, &b| boxes[a][0].total_cmp(&boxes[b][0]));
            let rows = chain_boxes(
                &across,
                &|b| boxes[b][3],
                reach,
                &|p, q| boxes[q][0] - boxes[p][2],
                &|p| reach.max(2.0 * (boxes[p][2] - boxes[p][0]).max(widest)),
                &row,
            );
            assert_eq!(rows, chains_one_by_one(&across, &row), "rows, seed {seed}");
            let mut down: Vec<usize> = (0..n).collect();
            down.sort_by(|&a, &b| boxes[b][3].total_cmp(&boxes[a][3]));
            let columns = chain_boxes(
                &down,
                &|b| boxes[b][0],
                reach,
                &|p, q| boxes[p][1] - boxes[q][3],
                &|p| reach.max(2.0 * (boxes[p][3] - boxes[p][1]).max(tallest)),
                &column,
            );
            assert_eq!(
                columns,
                chains_one_by_one(&down, &column),
                "columns, seed {seed}"
            );
            assert!(rows.iter().chain(&columns).any(|c| c.len() > 1));
        }
    }

    /// Task 37: the lines that reach into a stretch, found by their order, say the same as
    /// every line at the place, for lines apart, lines that overlap and one long line among
    /// short ones, at places within the tolerance of each other.
    #[test]
    fn a_stretch_is_covered_as_every_line_at_its_place_says() {
        let (mut yes, mut no) = (0, 0);
        for seed in 0..40u64 {
            let mut dice = Dice(seed + 100);
            let mut segs: Vec<Line> = (0..80)
                .map(|_| {
                    let from = 5.0 * dice.roll(40);
                    Line {
                        at: 0.5 * dice.roll(8),
                        from,
                        to: from + 5.0 * (1.0 + dice.roll(4)),
                    }
                })
                .collect();
            if seed % 2 == 0 {
                segs.push(Line {
                    at: 1.0,
                    from: 0.0,
                    to: 150.0,
                });
            }
            segs.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
            let cover = Cover::new(&segs);
            for _ in 0..200 {
                let at = 0.5 * dice.roll(8);
                let lo = 5.0 * dice.roll(40);
                let hi = lo + 5.0 * (1.0 + dice.roll(6));
                let t = [0.0, 0.6, 2.0][dice.roll(3) as usize];
                let want = covered_one_by_one(&segs, at, lo, hi, t);
                assert_eq!(cover.covers(at, lo, hi, t), want, "seed {seed}");
                if want {
                    yes += 1;
                } else {
                    no += 1;
                }
            }
        }
        assert!(yes > 100 && no > 100, "{yes} covered, {no} not");
    }

    /// A line with a start that is no number has no order to search in: every line at the
    /// place is looked at, as before.
    #[test]
    fn lines_that_are_no_numbers_are_looked_at_one_by_one() {
        let segs = [
            Line {
                at: 1.0,
                from: f32::NAN,
                to: 30.0,
            },
            Line {
                at: 1.0,
                from: 0.0,
                to: 20.0,
            },
            Line {
                at: 1.0,
                from: 20.0,
                to: 40.0,
            },
        ];
        let cover = Cover::new(&segs);
        for (lo, hi) in [(0.0, 40.0), (5.0, 35.0), (0.0, 50.0)] {
            assert_eq!(
                cover.covers(1.0, lo, hi, 0.5),
                covered_one_by_one(&segs, 1.0, lo, hi, 0.5),
                "{lo}..{hi}"
            );
        }
    }
}
