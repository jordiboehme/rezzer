// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Metamorphic tests of the region engine: changes to a page that keep its meaning (a scale,
//! a shift, a little jitter, runs split at word gaps or joined, rules in pieces or drawn as
//! thin bars, the primitives in another order) and the comparison of the engine's decisions
//! on the page before and after. A change may keep every decision or move a typed region to
//! a spatial one; it may never give a different typed read, lose a run or read one twice.
//!
//! Order counts only where it carries meaning. Two regions that both pages read the same must
//! keep their order when one stands over the other (one flow, see [`one_flow`]: a heading and
//! its section, the blocks of a column, a caption under its picture), and when the page reads
//! them one right after the other and a bond holds them together (see [`Bond`]): the foot of a
//! column and the head of the next, a side heading and its block, a label and its value on one
//! row, a caption beside its picture, a sentence that runs from one region into the next. The
//! order of every other two blocks that stand side by side is free and is no break.
//!
//! Regions are compared by every primitive they hold (runs, rules, boxes, pictures, codes),
//! each mapped back to the primitives of the page it came from ([`Origin`]).
//!
//! Ruling on spatial to typed: a typed region of the changed page whose primitives were all
//! spatial on the page is allowed and listed, not a break. The two pages are the same page,
//! so either may be the one a file brings: seen from the changed page, the page is a move of
//! that region to spatial, which the brief allows. The moves both ways are counted and pinned
//! (see [`pin_lines`]), so a change of the engine that moves more regions shows.
//!
//! This module is shared: the engine's own tests use it, and the PDF reader's tests include
//! it by path for the generated shapes and the corpus pages.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};

use layoutregions::{Bounds, BoxIn, GapIn, Kind, PageIn, PrimId, RuleIn, TextRun};

/// The decisions of one page: each its kind, its primitives and what made its segment (for
/// the reports), in the engine's order.
pub type Decisions = Vec<(Kind, Vec<PrimId>, String)>;

/// Where each primitive of a changed page came from: for each run the runs of the page (a
/// joined run comes from several), for each rule, box, picture and code the primitive of the
/// page (a piece of a rule from that rule, a bar from the rule it draws).
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub texts: Vec<Vec<u32>>,
    pub rules: Vec<PrimId>,
    pub boxes: Vec<PrimId>,
    pub pictures: Vec<PrimId>,
    pub codes: Vec<PrimId>,
}

impl Origin {
    /// Every primitive of `page` from itself.
    pub fn same(page: &PageIn) -> Origin {
        let ids = |n: usize, f: fn(u32) -> PrimId| (0..n as u32).map(f).collect();
        Origin {
            texts: (0..page.texts.len() as u32).map(|i| vec![i]).collect(),
            rules: ids(page.rules.len(), PrimId::Rule),
            boxes: ids(page.boxes.len(), PrimId::Box),
            pictures: ids(page.pictures.len(), PrimId::Picture),
            codes: ids(page.codes.len(), PrimId::Code),
        }
    }

    /// The primitives of the page that `p` of the changed page came from.
    pub fn of(&self, p: PrimId) -> Vec<PrimId> {
        let one = |list: &[PrimId], i: u32| list.get(i as usize).copied().into_iter().collect();
        match p {
            PrimId::Text(i) => self
                .texts
                .get(i as usize)
                .map(|o| o.iter().map(|&t| PrimId::Text(t)).collect())
                .unwrap_or_default(),
            PrimId::Rule(i) => one(&self.rules, i),
            PrimId::Box(i) => one(&self.boxes, i),
            PrimId::Picture(i) => one(&self.pictures, i),
            PrimId::Code(i) => one(&self.codes, i),
        }
    }
}

/// A change to a page that keeps its meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Transform {
    /// Every place and size times this many hundredths.
    Scale(u32),
    /// Everything moved right and down by up to [`SHIFT`] within the frame: the content keeps
    /// the frame, only its margins change.
    Shift,
    /// Everything moved right and up by [`SHIFT_GROW`]; the frame grows by as much, so the
    /// top and right margins stay as they are (a page of another paper size).
    ShiftGrow,
    /// Each run's place, baseline, width and height moved by up to [`JITTER`] of its text
    /// size, from a fixed seed.
    Jitter,
    /// Each run cut into one run per word at the word gaps it states.
    SplitRuns,
    /// Runs next to each other on one baseline, in one style, a word space apart, joined into
    /// one run with a space (their gap kept as the run's word gap).
    MergeRuns,
    /// Each line rule cut into three touching pieces.
    SplitRules,
    /// Each line rule drawn as a thin filled box instead.
    RuleAsBox,
    /// The runs in another order (each keeps its line number).
    ReorderRuns,
    /// The rules, boxes, pictures and codes in another order.
    ReorderDrawn,
}

/// The most [`Transform::Shift`] moves the content, in points.
pub const SHIFT: [f32; 2] = [12.0, 12.0];

/// The offset of [`Transform::ShiftGrow`], in points.
pub const SHIFT_GROW: [f32; 2] = [31.0, 47.0];

/// The most [`Transform::Jitter`] moves a run, as a share of its text size.
pub const JITTER: f32 = 0.005;

/// The gaps [`Transform::MergeRuns`] joins, as shares of the text size: a word space, over a
/// gap inside a word (a run cut by the producer) and under any gap between columns.
pub const MERGE_GAP: [f32; 2] = [0.1, 0.3];

/// The thickness of a bar [`Transform::RuleAsBox`] draws for a rule without one, in points.
pub const BAR: f32 = 0.5;

/// The seed of the jitter and the shuffles.
pub const SEED: u64 = 0x5eed_2026_1111;

impl Transform {
    /// Every change, in the order the reports list them.
    pub const ALL: [Transform; 13] = [
        Transform::Scale(50),
        Transform::Scale(75),
        Transform::Scale(150),
        Transform::Scale(200),
        Transform::Shift,
        Transform::ShiftGrow,
        Transform::Jitter,
        Transform::SplitRuns,
        Transform::MergeRuns,
        Transform::SplitRules,
        Transform::RuleAsBox,
        Transform::ReorderRuns,
        Transform::ReorderDrawn,
    ];

    /// A short name: `scale-50`, `shift`, `split-runs` and so on.
    pub fn name(self) -> String {
        match self {
            Transform::Scale(p) => format!("scale-{p}"),
            Transform::Shift => "shift".into(),
            Transform::ShiftGrow => "shift-grow".into(),
            Transform::Jitter => "jitter".into(),
            Transform::SplitRuns => "split-runs".into(),
            Transform::MergeRuns => "merge-runs".into(),
            Transform::SplitRules => "split-rules".into(),
            Transform::RuleAsBox => "rule-as-box".into(),
            Transform::ReorderRuns => "reorder-runs".into(),
            Transform::ReorderDrawn => "reorder-drawn".into(),
        }
    }

    /// The page changed, and where each of its primitives came from.
    pub fn apply(self, page: &PageIn) -> (PageIn, Origin) {
        assert!(
            page.given.is_empty(),
            "the changes keep no given regions: the pages are the engine's input before any"
        );
        let same = Origin::same(page);
        match self {
            Transform::Scale(p) => (scale(page, p as f32 / 100.0), same),
            Transform::Shift => (shift(page), same),
            Transform::ShiftGrow => (shift_grow(page), same),
            Transform::Jitter => (jitter(page), same),
            Transform::SplitRuns => {
                let (p, texts) = split_runs(page);
                (p, Origin { texts, ..same })
            }
            Transform::MergeRuns => {
                let (p, texts) = merge_runs(page);
                (p, Origin { texts, ..same })
            }
            Transform::SplitRules => split_rules(page, same),
            Transform::RuleAsBox => rule_as_box(page, same),
            Transform::ReorderRuns => {
                let (p, texts) = reorder_runs(page);
                (p, Origin { texts, ..same })
            }
            Transform::ReorderDrawn => reorder_drawn(page),
        }
    }
}

/// A small generator of the same numbers on every run (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in -1 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }

    /// The numbers 0 to `n - 1` in a shuffled order.
    pub fn order(&mut self, n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
        v
    }
}

fn map_bounds(b: &Bounds, f: &dyn Fn(f32, f32) -> (f32, f32)) -> Bounds {
    let (x0, y0) = f(b.x0, b.y0);
    let (x1, y1) = f(b.x1, b.y1);
    Bounds { x0, y0, x1, y1 }
}

/// `page` with every point mapped by `f` and every length (sizes, widths, thicknesses)
/// times `k`.
fn mapped(page: &PageIn, f: &dyn Fn(f32, f32) -> (f32, f32), k: f32, size: [f32; 2]) -> PageIn {
    let mut out = page.clone();
    out.size = size;
    for t in &mut out.texts {
        let (x, y) = f(t.x, t.y);
        t.x = x;
        t.y = y;
        t.w *= k;
        t.h *= k;
        t.size *= k;
        for g in &mut t.gaps {
            g.from = f(g.from, 0.0).0;
            g.to = f(g.to, 0.0).0;
        }
    }
    for r in &mut out.rules {
        let (x0, y0) = f(r.x0, r.y0);
        let (x1, y1) = f(r.x1, r.y1);
        (r.x0, r.y0, r.x1, r.y1) = (x0, y0, x1, y1);
        r.thick *= k;
    }
    for b in &mut out.boxes {
        b.bounds = map_bounds(&b.bounds, f);
    }
    for p in &mut out.pictures {
        p.bounds = map_bounds(&p.bounds, f);
    }
    for c in &mut out.codes {
        c.bounds = map_bounds(&c.bounds, f);
    }
    out
}

fn scale(page: &PageIn, k: f32) -> PageIn {
    mapped(
        page,
        &|x, y| (x * k, y * k),
        k,
        [page.size[0] * k, page.size[1] * k],
    )
}

/// The right edge and the bottom of everything on `page`.
fn reach(page: &PageIn) -> (f32, f32) {
    let mut right = f32::MIN;
    let mut bottom = f32::MAX;
    let mut take = |x1: f32, y0: f32| {
        right = right.max(x1);
        bottom = bottom.min(y0);
    };
    for t in &page.texts {
        take(t.x + t.w.max(0.0), t.y.min(t.y + t.h));
    }
    for r in &page.rules {
        take(r.x0.max(r.x1), r.y0.min(r.y1));
    }
    for b in page
        .boxes
        .iter()
        .map(|b| b.bounds)
        .chain(page.pictures.iter().map(|p| p.bounds))
        .chain(page.codes.iter().map(|c| c.bounds))
    {
        take(b.x1.max(b.x0), b.y0.min(b.y1));
    }
    (right, bottom)
}

fn shift(page: &PageIn) -> PageIn {
    let (right, bottom) = reach(page);
    if right == f32::MIN {
        return page.clone();
    }
    let dx = SHIFT[0].min(page.size[0] - right).max(0.0);
    let dy = SHIFT[1].min(bottom).max(0.0);
    mapped(page, &|x, y| (x + dx, y - dy), 1.0, page.size)
}

fn shift_grow(page: &PageIn) -> PageIn {
    let [dx, dy] = SHIFT_GROW;
    mapped(
        page,
        &|x, y| (x + dx, y + dy),
        1.0,
        [page.size[0] + dx, page.size[1] + dy],
    )
}

fn jitter(page: &PageIn) -> PageIn {
    let mut rng = Rng::new(SEED);
    let mut out = page.clone();
    for t in &mut out.texts {
        let d = JITTER * t.size;
        let dx = d * rng.unit();
        t.x += dx;
        t.y += d * rng.unit();
        t.w = (t.w + d * rng.unit()).max(0.0);
        t.h = (t.h + d * rng.unit()).max(0.0);
        for g in &mut t.gaps {
            g.from += dx;
            g.to += dx;
        }
    }
    out
}

/// The words of `text` as `split_whitespace` counts them, with their byte ranges.
fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

fn split_runs(page: &PageIn) -> (PageIn, Vec<Vec<u32>>) {
    let mut out = page.clone();
    out.texts.clear();
    let mut origin = Vec::new();
    for (i, t) in page.texts.iter().enumerate() {
        let ws = words(&t.text);
        let mut gaps: Vec<GapIn> = t
            .gaps
            .iter()
            .copied()
            .filter(|g| g.words >= 1 && (g.words as usize) < ws.len() && g.to >= g.from)
            .collect();
        gaps.sort_by_key(|g| g.words);
        gaps.dedup_by_key(|g| g.words);
        if gaps.is_empty() {
            out.texts.push(t.clone());
            origin.push(vec![i as u32]);
            continue;
        }
        // One piece before each stated gap and one after the last: its first word and where
        // its ink starts. Piece `n` ends where gap `n` starts, the last at the run's end.
        let mut cuts: Vec<(usize, f32)> = vec![(0, t.x)];
        cuts.extend(gaps.iter().map(|g| (g.words as usize, g.to)));
        for (n, &(from, x0)) in cuts.iter().enumerate() {
            let to = cuts.get(n + 1).map_or(ws.len(), |c| c.0);
            let x1 = gaps.get(n).map_or(t.x + t.w, |g| g.from);
            let text = t.text[ws[from].0..ws[to - 1].1].to_string();
            let inner: Vec<GapIn> = t
                .gaps
                .iter()
                .filter(|g| (g.words as usize) > from && (g.words as usize) < to)
                .map(|g| GapIn {
                    words: g.words - from as u32,
                    ..*g
                })
                .collect();
            out.texts.push(TextRun {
                text,
                x: x0,
                w: (x1 - x0).max(0.0),
                gaps: inner,
                ..t.clone()
            });
            origin.push(vec![i as u32]);
        }
    }
    (out, origin)
}

/// Whether `b` may be joined onto `a` as the next word of one run: one line and style, and a
/// gap of a word space ([`MERGE_GAP`]).
fn joins(a: &TextRun, b: &TextRun) -> bool {
    let gap = b.x - (a.x + a.w);
    a.line == b.line
        && a.font == b.font
        && a.font_kind == b.font_kind
        && a.bold == b.bold
        && a.italic == b.italic
        && a.color == b.color
        && a.shift == b.shift
        && a.off_axis == b.off_axis
        && !a.off_axis
        && a.size == b.size
        && (a.y - b.y).abs() <= 0.01 * a.size
        && gap >= MERGE_GAP[0] * a.size
        && gap <= MERGE_GAP[1] * a.size
        && !a.text.trim().is_empty()
        && !b.text.trim().is_empty()
}

fn merge_runs(page: &PageIn) -> (PageIn, Vec<Vec<u32>>) {
    let mut out = page.clone();
    out.texts.clear();
    let mut origin: Vec<Vec<u32>> = Vec::new();
    for (i, t) in page.texts.iter().enumerate() {
        if let Some(last) = out.texts.last_mut()
            && joins(last, t)
        {
            let before = words(&last.text).len() as u32;
            let gap = GapIn {
                words: before,
                from: last.x + last.w,
                to: t.x,
            };
            last.gaps.push(gap);
            last.gaps.extend(t.gaps.iter().map(|g| GapIn {
                words: g.words + before,
                ..*g
            }));
            last.text = format!("{} {}", last.text.trim_end(), t.text.trim_start());
            last.w = t.x + t.w - last.x;
            last.h = last.h.max(t.h);
            if let Some(o) = origin.last_mut() {
                o.push(i as u32);
            }
            continue;
        }
        out.texts.push(t.clone());
        origin.push(vec![i as u32]);
    }
    (out, origin)
}

fn is_line(r: &RuleIn) -> bool {
    !r.rect && (r.x0 != r.x1 || r.y0 != r.y1)
}

fn split_rules(page: &PageIn, same: Origin) -> (PageIn, Origin) {
    let mut out = page.clone();
    out.rules.clear();
    let mut rules = Vec::new();
    for (i, r) in page.rules.iter().enumerate() {
        if !is_line(r) {
            out.rules.push(r.clone());
            rules.push(PrimId::Rule(i as u32));
            continue;
        }
        let at = |k: f32| (r.x0 + (r.x1 - r.x0) * k, r.y0 + (r.y1 - r.y0) * k);
        let cuts = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
        for w in cuts.windows(2) {
            let (x0, y0) = if w[0] == 0.0 { (r.x0, r.y0) } else { at(w[0]) };
            let (x1, y1) = if w[1] == 1.0 { (r.x1, r.y1) } else { at(w[1]) };
            out.rules.push(RuleIn {
                x0,
                y0,
                x1,
                y1,
                ..r.clone()
            });
            rules.push(PrimId::Rule(i as u32));
        }
    }
    (out, Origin { rules, ..same })
}

fn rule_as_box(page: &PageIn, same: Origin) -> (PageIn, Origin) {
    let mut out = page.clone();
    out.rules.clear();
    let (mut rules, mut boxes) = (Vec::new(), same.boxes.clone());
    for (i, r) in page.rules.iter().enumerate() {
        if !is_line(r) || r.dashed {
            out.rules.push(r.clone());
            rules.push(PrimId::Rule(i as u32));
            continue;
        }
        let half = if r.thick > 0.0 { r.thick } else { BAR } / 2.0;
        let (h, v) = if (r.y1 - r.y0).abs() <= (r.x1 - r.x0).abs() {
            (0.0, half)
        } else {
            (half, 0.0)
        };
        out.boxes.push(BoxIn {
            bounds: Bounds {
                x0: r.x0.min(r.x1) - h,
                y0: r.y0.min(r.y1) - v,
                x1: r.x0.max(r.x1) + h,
                y1: r.y0.max(r.y1) + v,
            },
            filled: true,
            stroked: false,
            widget: false,
        });
        boxes.push(PrimId::Rule(i as u32));
    }
    (
        out,
        Origin {
            rules,
            boxes,
            ..same
        },
    )
}

fn reorder_runs(page: &PageIn) -> (PageIn, Vec<Vec<u32>>) {
    let order = Rng::new(SEED).order(page.texts.len());
    let mut out = page.clone();
    out.texts = order.iter().map(|&i| page.texts[i].clone()).collect();
    (out, order.iter().map(|&i| vec![i as u32]).collect())
}

fn reorder_drawn(page: &PageIn) -> (PageIn, Origin) {
    let mut rng = Rng::new(SEED);
    let mut out = page.clone();
    let [r, b, p, c] = [
        rng.order(page.rules.len()),
        rng.order(page.boxes.len()),
        rng.order(page.pictures.len()),
        rng.order(page.codes.len()),
    ];
    out.rules = r.iter().map(|&i| page.rules[i].clone()).collect();
    out.boxes = b.iter().map(|&i| page.boxes[i].clone()).collect();
    out.pictures = p.iter().map(|&i| page.pictures[i].clone()).collect();
    out.codes = c.iter().map(|&i| page.codes[i].clone()).collect();
    let ids = |o: &[usize], f: fn(u32) -> PrimId| o.iter().map(|&i| f(i as u32)).collect();
    let origin = Origin {
        rules: ids(&r, PrimId::Rule),
        boxes: ids(&b, PrimId::Box),
        pictures: ids(&p, PrimId::Picture),
        codes: ids(&c, PrimId::Code),
        ..Origin::same(page)
    };
    (out, origin)
}

/// What a change did to a page's reading that it must not do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Break {
    /// A typed region of the changed page holds the primitives of a typed region of the page,
    /// but as another kind.
    Kind,
    /// A typed region of the changed page holds other primitives than any typed region of the
    /// page (it took some from a typed region, or a region was cut or joined).
    Members,
    /// Two typed regions both pages read the same, of one flow (see [`one_flow`]) or held by
    /// a bond (see [`Bond`]), stand in another order, or a region stands inside a sentence
    /// that ran from one region into the next. The order of regions that do not depend on
    /// each other is free. One break per page at most: the first region found out of order
    /// is named, not every one.
    Order,
    /// A run of the changed page is in no region.
    Lost,
    /// A run of the changed page is in two regions.
    Twice,
}

impl Break {
    pub fn name(self) -> &'static str {
        match self {
            Break::Kind => "kind",
            Break::Members => "members",
            Break::Order => "order",
            Break::Lost => "lost",
            Break::Twice => "twice",
        }
    }
}

/// The class of regions of `kinds`: `pairs` (labels and values or totals), `table`, `band`
/// (a running header or footer) or `text` (the other kinds).
pub fn class_of(kinds: &[Kind]) -> &'static str {
    let has = |k: Kind| kinds.contains(&k);
    if has(Kind::LabelValue) || has(Kind::Totals) {
        "pairs"
    } else if has(Kind::Table) {
        "table"
    } else if has(Kind::HeaderBand) || has(Kind::FooterBand) {
        "band"
    } else {
        "text"
    }
}

/// One break: what it is, the kinds of the regions it touches (on the page and the changed
/// page) and a detail for the report.
#[derive(Debug, Clone, PartialEq)]
pub struct Broken {
    pub what: Break,
    pub kinds: Vec<Kind>,
    pub detail: String,
}

impl Broken {
    /// The class of the regions it touches (see [`class_of`]).
    pub fn class(&self) -> &'static str {
        class_of(&self.kinds)
    }
}

/// The task that owns the breaks of change `t`, of kind `what`, in class `class` (see
/// [`class_of`]):
///
/// - rules drawn as filled bars are input the PDF reader never hands over (it turns bars
///   into lines before the engine), so the engine's own taking of such input comes after
///   0.1.0. Runs in another order are read in the order of their places
///   (`layoutregions::RunOrder`, Task 31c): a break under that change is a break like any
///   other;
/// - labels and values: after 0.1.0. The tasks that typed these leaves are closed (label
///   and value groups, then label order: each pin they owned is fixed or carries its traced
///   cause), so a new break of this class is traced and either fixed or kept with its
///   cause; no task before the release is its owner by default. The changes
///   whose cause is the segmentation (rules in pieces part the strips; absolute points in
///   the drawn structure under a scale of 0.5 or 2) stay with Task 31;
/// - a text region read as another text kind (heading, list, paragraph, legend): after
///   0.1.0, for the same reason;
/// - every other cut, join or order of regions (the segmentation, the bands of running
///   headers and footers): Task 31.
pub fn owner(t: Transform, what: Break, class: &str) -> &'static str {
    match (t, what, class) {
        (Transform::RuleAsBox, _, _) => "after 0.1.0",
        (Transform::SplitRules | Transform::Scale(50) | Transform::Scale(200), _, "pairs") => "31",
        (_, _, "pairs") => "after 0.1.0",
        (_, Break::Kind, "text") => "after 0.1.0",
        _ => "31",
    }
}

/// The comparison of one page and the same page changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    /// Typed regions that became spatial (allowed): the kind they had, and `kind: first words`.
    pub to_spatial: Vec<(Kind, String)>,
    /// Typed regions of the changed page whose primitives were all spatial on the page
    /// (allowed by the ruling in the module doc): their kind, and `kind: first words`.
    pub from_spatial: Vec<(Kind, String)>,
    /// What the change must not do.
    pub broken: Vec<Broken>,
}

impl Verdict {
    pub fn same(&self) -> bool {
        self.to_spatial.is_empty() && self.from_spatial.is_empty() && self.broken.is_empty()
    }
}

/// The first words of the runs among `members` of `page`, or what else they are, for a report.
fn label(page: &PageIn, kind: Kind, members: &[PrimId]) -> String {
    let mut text = String::new();
    let (mut drawn, mut pictures) = (0usize, 0usize);
    for p in members {
        match *p {
            PrimId::Text(i) if text.chars().count() <= 40 => {
                if let Some(t) = page.texts.get(i as usize) {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(t.text.trim());
                }
            }
            PrimId::Text(_) => {}
            PrimId::Rule(_) | PrimId::Box(_) => drawn += 1,
            PrimId::Picture(_) | PrimId::Code(_) => pictures += 1,
        }
    }
    let short: String = text.chars().take(40).collect();
    if short.is_empty() {
        format!(
            "{}: ({drawn} rules or boxes, {pictures} pictures or codes)",
            kind.name()
        )
    } else {
        format!("{}: {short}", kind.name())
    }
}

/// The primitives of the page a decision holds, through `origin`, sorted and once.
fn members_of(prims: &[PrimId], origin: &Origin) -> Vec<PrimId> {
    let mut ids: Vec<PrimId> = prims.iter().flat_map(|&p| origin.of(p)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Two regions are one flow when this share of the width of the narrower one, or more, stands
/// over or under the other: a heading over its section, the paragraphs and rows of one
/// column, a caption under its picture, a block across the page over the columns under it.
/// Half the width: two blocks side by side whose ragged ends reach a little over each other
/// are no flow, and a block set in a little against the one over it still is.
pub const FLOW_SHARE: f32 = 0.5;

/// Whether the regions that reach from `a[0]` to `a[1]` and from `b[0]` to `b[1]` across the
/// page are one flow (see [`FLOW_SHARE`]): the order of the two carries meaning. Two regions
/// that stand side by side (a head block and a date box, two side boxes, the columns of a
/// footer, a margin note and a line of a form) do not depend on each other, and their order
/// is free.
pub fn one_flow(a: [f32; 2], b: [f32; 2]) -> bool {
    let shared = a[1].min(b[1]) - a[0].max(b[0]);
    let narrow = (a[1] - a[0]).min(b[1] - b[0]);
    // A region without width (a mark, a rule down the page) is in the flow it stands in.
    if narrow <= 0.0 {
        return shared >= 0.0;
    }
    shared >= FLOW_SHARE * narrow
}

/// Where the region of `members` reaches from and to across `page`: by its text runs, or by
/// all it holds when it holds no text (the rules and boxes of a region with text may reach
/// far past its text: a rule under a title, the frame of a table).
fn across(page: &PageIn, members: &[PrimId]) -> Option<[f32; 2]> {
    let texts: Vec<PrimId> = members
        .iter()
        .copied()
        .filter(|p| matches!(p, PrimId::Text(_)))
        .collect();
    let of = if texts.is_empty() { members } else { &texts };
    layoutregions::segment::bounds_of(page, of).map(|b| [b.x0, b.x1])
}

/// Why the order of two regions that do not stand over each other carries meaning. A bond
/// holds between a region and the next typed region the page reads (see [`bond`]). Geometry
/// alone cannot tell two columns of one text from two blocks of an address, or a label from
/// a name, so each bond asks for a sign of its own; without one the order is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bond {
    /// The foot of a column of running text and the head of the next column of its set,
    /// also when that head is a heading over the text of the column
    /// ([`heads_next_column`]).
    Column,
    /// A heading and the block it stands left of, on the block's rows: a side heading in the
    /// margin of a block that is as wide as it or wider.
    Heading,
    /// A label and its value on one row, or a column of labels and the column of their
    /// values, row by row. Nothing may stand between the two.
    Field,
    /// A picture and the caption that stands beside it. Nothing may stand between the two.
    Caption,
    /// A sentence that runs from the end of one region into the start of the next. Nothing
    /// may stand between the two.
    Sentence,
}

/// Two columns are of one set when the narrower one has this share of the width of the
/// other or more, and a line of running text is full when it reaches this share of the width
/// of its region. Four fifths: the columns of a set are set to one width, and a column that
/// holds only the short end of a paragraph, or a block of short lines (an address, a list of
/// names), is no sign of a text that runs on.
pub const COLUMN_SHARE: f32 = 0.8;

/// A block stands on the rows of another when this share of its height, or more, lies within
/// the height of the other; two lines are one row when their baselines are apart by less
/// than this share of the smaller text size.
pub const ROW_SHARE: f32 = 0.5;

/// What the order check knows of a typed region of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Stands {
    pub kind: Kind,
    /// From and to across the page (by its text, as [`one_flow`] takes it).
    pub x: [f32; 2],
    /// Bottom and top.
    pub y: [f32; 2],
    /// Its lines of text from the top: baseline, left and right end, text size.
    pub lines: Vec<[f32; 4]>,
    /// Its text, the lines from the top joined by spaces.
    pub text: String,
    /// Every line of it ends in a colon: a column of labels.
    pub labels: bool,
}

impl Stands {
    fn width(&self) -> f32 {
        self.x[1] - self.x[0]
    }

    /// Running text in a column: three lines or more, and more than half of the lines over
    /// the last one are full (see [`COLUMN_SHARE`]). Two lines over the last at least: the
    /// widest line of a block is full by itself, so one line says nothing (every block of two
    /// lines would be prose). A block of short lines that happen to be of one length still
    /// reads as prose: a limit of the test.
    fn prose(&self) -> bool {
        let Some((_, over)) = self.lines.split_last() else {
            return false;
        };
        let full = over
            .iter()
            .filter(|l| l[2] - l[1] >= COLUMN_SHARE * self.width())
            .count();
        over.len() >= 2 && 2 * full > over.len()
    }

    /// Two lines or more, the last one full (see [`COLUMN_SHARE`]): the text does not end
    /// here, its line was broken because it was full.
    fn ends_full(&self) -> bool {
        self.lines.len() >= 2
            && self
                .lines
                .last()
                .is_some_and(|l| l[2] - l[1] >= COLUMN_SHARE * self.width())
    }

    fn running(&self) -> bool {
        matches!(self.kind, Kind::Paragraph | Kind::List)
    }

    /// Whether `self` stands on the rows of `other` (see [`ROW_SHARE`]).
    fn on_rows_of(&self, other: &Stands) -> bool {
        let shared = self.y[1].min(other.y[1]) - self.y[0].max(other.y[0]);
        let h = self.y[1] - self.y[0];
        h > 0.0 && shared >= ROW_SHARE * h
    }

    /// Whether `self` and `other` hold the same count of lines and each line of one shares
    /// its baseline with the line of the other (see [`ROW_SHARE`]).
    fn row_by_row(&self, other: &Stands) -> bool {
        !self.lines.is_empty()
            && self.lines.len() == other.lines.len()
            && self
                .lines
                .iter()
                .zip(&other.lines)
                .all(|(a, b)| (a[0] - b[0]).abs() < ROW_SHARE * a[3].min(b[3]))
    }
}

/// What `members` of `page` are to the order check; none when none of them is on the page.
pub fn stands(page: &PageIn, kind: Kind, members: &[PrimId]) -> Option<Stands> {
    let x = across(page, members)?;
    let all = layoutregions::segment::bounds_of(page, members)?;
    let mut runs: Vec<&TextRun> = members
        .iter()
        .filter_map(|p| match *p {
            PrimId::Text(i) => page.texts.get(i as usize),
            _ => None,
        })
        .collect();
    // From the top, and from the left within one baseline.
    runs.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    let mut lines: Vec<[f32; 4]> = Vec::new();
    let mut words: Vec<Vec<(f32, &str)>> = Vec::new();
    for r in runs {
        match (lines.last_mut(), words.last_mut()) {
            (Some(l), Some(w)) if (l[0] - r.y).abs() < ROW_SHARE * l[3].min(r.size) => {
                l[1] = l[1].min(r.x);
                l[2] = l[2].max(r.x + r.w);
                w.push((r.x, r.text.trim()));
            }
            _ => {
                lines.push([r.y, r.x, r.x + r.w, r.size]);
                words.push(vec![(r.x, r.text.trim())]);
            }
        }
    }
    for w in &mut words {
        w.sort_by(|a, b| a.0.total_cmp(&b.0));
        w.retain(|(_, t)| !t.is_empty());
    }
    let labels = !words.is_empty()
        && words
            .iter()
            .all(|w| w.last().is_some_and(|(_, t)| t.ends_with(':')));
    let text = words
        .iter()
        .flat_map(|w| w.iter().map(|(_, t)| *t))
        .collect::<Vec<_>>()
        .join(" ");
    Some(Stands {
        kind,
        x,
        y: [all.y0, all.y1],
        lines,
        text,
        labels,
    })
}

/// Whether `text` ends in the middle of a sentence: on a letter, a digit, a comma or a
/// hyphen, not on a mark that ends a sentence or opens what follows.
fn runs_on(text: &str) -> bool {
    text.chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, ',' | '-'))
}

/// Whether `text` goes on a sentence: it starts with a small letter.
fn goes_on(text: &str) -> bool {
    text.chars().next().is_some_and(char::is_lowercase)
}

/// The bond between `a` and `b`, which the page reads one right after the other (`a`
/// first), or none: their order is free unless one stands over the other ([`one_flow`]).
///
/// - [`Bond::Sentence`]: both are running text, `a` has two lines or more and ends on a
///   full line ([`COLUMN_SHARE`]) in the middle of a sentence, and `b` starts with a small
///   letter. The full line is the sign that the text was broken there and runs on: a line of
///   words alone (a head line, a label, an address) over text that starts small is no
///   sentence. `b` stands where a text goes on: under `a` in its flow, or right of it and
///   starting above its end (the head of the next column). A block to the left under `a`
///   goes on nothing.
///
/// The others hold between regions side by side only (no flow):
///
/// - [`Bond::Column`]: both are running text, `a` is prose with full lines, `b` stands right
///   of it at a like width ([`COLUMN_SHARE`]) and starts above the end of `a`. A next column
///   that starts with a heading has its sign in the text under the heading, which this pair
///   does not show: see [`heads_next_column`].
/// - [`Bond::Heading`]: one is a heading that stands left of the other, on its rows, and the
///   other is as wide as the heading or wider and is not typed a label/value or totals
///   region (a side heading never heads a field beside a title). A side heading stands in
///   the margin of the block it heads. A title beside a narrower block (a number box, a
///   date) heads nothing there: the two are a free pair.
/// - [`Bond::Field`]: the two hold their lines row by row, and a sign says that the left one
///   is a label: one of them is typed a label/value or totals region and neither is a
///   heading (a title beside a number field is a free pair), or every line of the left one
///   ends in a colon. The right one is never a heading: a title is no value of a label, as
///   in the tag truth. Bands (the columns of a footer) never bond.
/// - [`Bond::Caption`]: one is a figure, the other running text that opens as a caption
///   does and stands on the rows of the figure.
pub fn bond(a: &Stands, b: &Stands) -> Option<Bond> {
    // Where a text goes on: under its end in the same flow, or at the head of a column to
    // the right, which starts above that end.
    let under = one_flow(a.x, b.x) && b.y[1] <= a.y[1];
    let next_column = !one_flow(a.x, b.x) && b.x[0] > a.x[0] && b.y[1] > a.y[0];
    if a.running()
        && b.running()
        && (under || next_column)
        && a.ends_full()
        && runs_on(&a.text)
        && goes_on(&b.text)
    {
        return Some(Bond::Sentence);
    }
    if one_flow(a.x, b.x) {
        return None;
    }
    let (left, right) = if a.x[0] <= b.x[0] { (a, b) } else { (b, a) };
    let band = |s: &Stands| matches!(s.kind, Kind::HeaderBand | Kind::FooterBand);
    if band(a) || band(b) {
        return None;
    }
    let field = |s: &Stands| matches!(s.kind, Kind::LabelValue | Kind::Totals);
    if left.kind == Kind::Heading
        && right.kind != Kind::Heading
        && !field(right)
        && left.on_rows_of(right)
        && right.width() >= left.width()
    {
        return Some(Bond::Heading);
    }
    let (figure, other) = if a.kind == Kind::Figure {
        (a, b)
    } else {
        (b, a)
    };
    if figure.kind == Kind::Figure
        && other.running()
        && other.on_rows_of(figure)
        && layoutregions::extract::table::caption::caption(&other.text).is_some()
    {
        return Some(Bond::Caption);
    }
    // A title is never the value of a field, by no sign, and it is no label of the field it
    // stands beside by the type alone: only its own colon makes it a label.
    let typed = left.kind != Kind::Heading && (field(a) || field(b));
    if left.row_by_row(right) && right.kind != Kind::Heading && (typed || left.labels) {
        return Some(Bond::Field);
    }
    let alike = a.width().min(b.width()) >= COLUMN_SHARE * a.width().max(b.width());
    if a.running() && b.running() && std::ptr::eq(left, a) && a.prose() && alike && b.y[1] > a.y[0]
    {
        return Some(Bond::Column);
    }
    None
}

/// Whether the heading `head`, which the page reads right after `a`, opens the next column
/// of the column set of `a`: a [`Bond::Column`] between `a` and `head`. `under` is the next
/// region the page reads in the flow of `head`.
///
/// The heading opens the next part of the text that `a` ends in its column. A heading is
/// neither running text nor of the width of a column, so the sign of the set is taken from
/// the text it heads: `a` is prose with full lines, `under` is running text that starts
/// under the top of `head`, right of `a` and at a like width ([`COLUMN_SHARE`]), and `head`
/// stands right of `a` and starts above its end. A heading over a block of short lines, or
/// over a table, is no sign of a column.
pub fn heads_next_column(a: &Stands, head: &Stands, under: &Stands) -> bool {
    let beside = |s: &Stands| !one_flow(a.x, s.x) && s.x[0] > a.x[0];
    let alike = a.width().min(under.width()) >= COLUMN_SHARE * a.width().max(under.width());
    head.kind == Kind::Heading
        && a.running()
        && a.prose()
        && under.running()
        && beside(head)
        && beside(under)
        && one_flow(head.x, under.x)
        && under.y[1] <= head.y[1]
        && alike
        && head.y[1] > a.y[0]
}

/// A typed decision: its place among the decisions, its kind, its primitives (of the page)
/// and what made its segment.
struct Typed {
    at: usize,
    kind: Kind,
    members: Vec<PrimId>,
    made: String,
}

/// The typed decisions of `ds` that hold something, their primitives through `origin`.
fn typed(ds: &Decisions, origin: &Origin) -> Vec<Typed> {
    ds.iter()
        .enumerate()
        .filter(|(_, (k, _, _))| *k != Kind::Spatial)
        .map(|(at, (kind, prims, made))| Typed {
            at,
            kind: *kind,
            members: members_of(prims, origin),
            made: made.clone(),
        })
        .filter(|t| !t.members.is_empty())
        .collect()
}

/// The decisions (kind and what made the segment) each primitive of the page is in.
fn kinds_of(ds: &Decisions, origin: &Origin) -> HashMap<PrimId, Vec<(Kind, String)>> {
    let mut k: HashMap<PrimId, Vec<(Kind, String)>> = HashMap::new();
    for (kind, prims, made) in ds {
        for p in members_of(prims, origin) {
            k.entry(p).or_default().push((*kind, made.clone()));
        }
    }
    k
}

/// Compares the decisions `base` on `page` with the decisions `changed` on the changed page
/// (of `changed_runs` runs), whose primitives came from those of `page` as `origin` says.
pub fn compare(
    page: &PageIn,
    base: &Decisions,
    changed_runs: usize,
    changed: &Decisions,
    origin: &Origin,
) -> Verdict {
    let mut v = Verdict::default();
    let same = Origin::same(page);
    // Each run of the changed page in exactly one region.
    let mut held = vec![0u32; changed_runs];
    for (_, prims, _) in changed {
        for p in prims {
            if let PrimId::Text(j) = *p
                && let Some(c) = held.get_mut(j as usize)
            {
                *c += 1;
            }
        }
    }
    for (j, &c) in held.iter().enumerate() {
        let what = match c {
            0 => Break::Lost,
            1 => continue,
            _ => Break::Twice,
        };
        v.broken.push(Broken {
            what,
            kinds: Vec::new(),
            detail: label(page, Kind::Spatial, &origin.of(PrimId::Text(j as u32))),
        });
    }
    let base_kind = kinds_of(base, &same);
    let new_kind = kinds_of(changed, origin);
    let none: Vec<(Kind, String)> = Vec::new();
    let was = |p: &PrimId| base_kind.get(p).unwrap_or(&none).iter();
    let b = typed(base, &same);
    let c = typed(changed, origin);
    let mut matched: Vec<(usize, usize)> = Vec::new();
    let mut b_matched = vec![false; b.len()];
    for t in &c {
        if let Some(bi) = b
            .iter()
            .position(|x| x.kind == t.kind && x.members == t.members)
        {
            b_matched[bi] = true;
            matched.push((b[bi].at, t.at));
            continue;
        }
        if t.members
            .iter()
            .all(|p| was(p).all(|(k, _)| *k == Kind::Spatial))
        {
            v.from_spatial
                .push((t.kind, label(page, t.kind, &t.members)));
            continue;
        }
        let mut kinds: Vec<Kind> = t
            .members
            .iter()
            .flat_map(|p| was(p).map(|(k, _)| *k))
            .chain([t.kind])
            .filter(|k| *k != Kind::Spatial)
            .collect();
        kinds.sort();
        kinds.dedup();
        let mut before: Vec<String> = t
            .members
            .iter()
            .flat_map(|p| was(p).map(|(k, m)| format!("{} {m}", k.name())))
            .collect();
        before.sort_unstable();
        before.dedup();
        let (what, detail) = match b.iter().find(|x| x.members == t.members) {
            Some(x) => (
                Break::Kind,
                format!("{} was {}", label(page, t.kind, &t.members), x.kind.name()),
            ),
            None => (Break::Members, label(page, t.kind, &t.members)),
        };
        v.broken.push(Broken {
            what,
            kinds,
            detail: format!("{detail} [now {}; was {}]", t.made, before.join(" | ")),
        });
    }
    for (bi, x) in b.iter().enumerate() {
        let spatial_now = x.members.iter().all(|p| {
            new_kind
                .get(p)
                .unwrap_or(&none)
                .iter()
                .all(|(k, _)| *k == Kind::Spatial)
        });
        if !b_matched[bi] && spatial_now {
            v.to_spatial.push((x.kind, label(page, x.kind, &x.members)));
        }
    }
    // The typed regions read the same keep their order where it carries meaning: between two
    // regions of one flow (see [`one_flow`]), and between a region and the next one the page
    // reads when a bond holds them (see [`bond`]). The order of every other two blocks that
    // stand side by side is free.
    let places: HashMap<usize, Stands> = b
        .iter()
        .filter_map(|x| stands(page, x.kind, &x.members).map(|s| (x.at, s)))
        .collect();
    let flow = |x: usize, y: usize| match (places.get(&x), places.get(&y)) {
        (Some(p), Some(q)) => one_flow(p.x, q.x),
        // A region without a place on the page: its order counts, as before.
        _ => true,
    };
    matched.sort_by_key(|&(_, cn)| cn);
    let mut moved = matched.iter().enumerate().find_map(|(j, &(late, _))| {
        matched[..j]
            .iter()
            .any(|&(early, _)| early > late && flow(early, late))
            .then_some(late)
    });
    // The bond that was broken, and the region it held the moved one to.
    let mut why: Option<(Bond, usize)> = None;
    if moved.is_none() {
        // Where each region of the page that is read the same stands among the typed
        // regions of the changed page.
        let now: HashMap<usize, usize> = matched
            .iter()
            .filter_map(|&(at, cn)| c.iter().position(|t| t.at == cn).map(|k| (at, k)))
            .collect();
        moved = b.windows(2).enumerate().find_map(|(k, w)| {
            let (first, next) = (w[0].at, w[1].at);
            let (p, q) = (places.get(&first)?, places.get(&next)?);
            let held = bond(p, q).or_else(|| {
                // A heading at the head of the next column: the sign is in the next region
                // the page reads in the flow of the heading.
                let under = b[k + 2..]
                    .iter()
                    .filter_map(|x| places.get(&x.at))
                    .find(|s| one_flow(q.x, s.x))?;
                heads_next_column(p, q, under).then_some(Bond::Column)
            })?;
            let (i, j) = (*now.get(&first)?, *now.get(&next)?);
            // A sentence, a label with its value and a picture with its caption are read
            // one right after the other: a region read between the two parts them, also
            // when each keeps its side.
            let apart = matches!(held, Bond::Sentence | Bond::Field | Bond::Caption) && j != i + 1;
            (j < i || apart).then(|| {
                why = Some((held, first));
                next
            })
        });
    }
    if let Some(at) = moved {
        let moved = b.iter().find(|x| x.at == at);
        v.broken.push(Broken {
            what: Break::Order,
            kinds: moved.map(|x| vec![x.kind]).unwrap_or_default(),
            detail: moved.map_or_else(String::new, |x| {
                let held = why.and_then(|(bond, to)| {
                    let to = b.iter().find(|y| y.at == to)?;
                    Some(format!(
                        " [{bond:?} with {}]",
                        label(page, to.kind, &to.members)
                    ))
                });
                format!(
                    "{} moved{}",
                    label(page, x.kind, &x.members),
                    held.unwrap_or_default()
                )
            }),
        });
    }
    v
}

/// One finding of a sweep: the change, the page (0-based) and the verdict.
#[derive(Debug, Clone)]
pub struct Finding {
    pub transform: Transform,
    pub page: usize,
    pub verdict: Verdict,
}

/// Runs every change of `transforms` on every page of `pages` (one file) and compares the
/// decisions of `decide` on the file and on the changed file. The facts of a file are learned
/// from its pages, so `decide` sees the changed file whole.
pub fn sweep(
    pages: &[PageIn],
    transforms: &[Transform],
    decide: &dyn Fn(&[PageIn]) -> Vec<Decisions>,
) -> Vec<Finding> {
    let base = decide(pages);
    let mut out = Vec::new();
    for &t in transforms {
        let (changed, origins): (Vec<PageIn>, Vec<Origin>) =
            pages.iter().map(|p| t.apply(p)).unzip();
        let decided = decide(&changed);
        for (n, page) in pages.iter().enumerate() {
            let verdict = compare(
                page,
                &base[n],
                changed[n].texts.len(),
                &decided[n],
                &origins[n],
            );
            out.push(Finding {
                transform: t,
                page: n,
                verdict,
            });
        }
    }
    out
}

/// The breaks of `findings` as lines
/// `<name>\t<transform>\t<page>\t<break>\t<class>\t<owner>\t<detail>`, the pages counted
/// from 1.
pub fn break_lines(name: &str, findings: &[Finding]) -> Vec<String> {
    let mut out = Vec::new();
    for f in findings {
        for b in &f.verdict.broken {
            out.push(format!(
                "{name}\t{}\t{}\t{}\t{}\t{}\t{}",
                f.transform.name(),
                f.page + 1,
                b.what.name(),
                b.class(),
                owner(f.transform, b.what, b.class()),
                b.detail
            ));
        }
    }
    out
}

/// The moves to and from spatial of `findings` as lines `<name>\t<transform>\t<page>\t<move>
/// \t<region>`.
pub fn move_lines(name: &str, findings: &[Finding]) -> Vec<String> {
    let mut out = Vec::new();
    for f in findings {
        for (m, list) in [
            ("to-spatial", &f.verdict.to_spatial),
            ("from-spatial", &f.verdict.from_spatial),
        ] {
            for (_, r) in list {
                out.push(format!(
                    "{name}\t{}\t{}\t{m}\t{r}",
                    f.transform.name(),
                    f.page + 1
                ));
            }
        }
    }
    out
}

/// The pin lines of `findings`, each `<name>\t<page>\t<transform>\t<what>\t<class>\t<count>
/// \t<owner>`:
///
/// - one line per page (counted from 1), change, break and class with its number of breaks;
/// - one line per page, change, move (`to-spatial`, `from-spatial`) and class with its number
///   of moves on that page, owner `-`.
///
/// A pinned file holds these lines: a break or a move more or less on any page moves a line,
/// so a move that goes on one page and comes on another does not cancel out.
pub fn pin_lines(name: &str, findings: &[Finding]) -> Vec<String> {
    let mut breaks: BTreeMap<(usize, String, &'static str, &'static str), (usize, &str)> =
        BTreeMap::new();
    let mut moves: BTreeMap<(usize, String, &'static str, &'static str), usize> = BTreeMap::new();
    for f in findings {
        for b in &f.verdict.broken {
            let o = owner(f.transform, b.what, b.class());
            breaks
                .entry((f.page + 1, f.transform.name(), b.what.name(), b.class()))
                .or_insert((0, o))
                .0 += 1;
        }
        for (m, list) in [
            ("to-spatial", &f.verdict.to_spatial),
            ("from-spatial", &f.verdict.from_spatial),
        ] {
            for (k, _) in list {
                *moves
                    .entry((f.page + 1, f.transform.name(), m, class_of(&[*k])))
                    .or_insert(0) += 1;
            }
        }
    }
    let mut out: Vec<String> = breaks
        .into_iter()
        .map(|((page, t, w, c), (n, o))| format!("{name}\t{page}\t{t}\t{w}\t{c}\t{n}\t{o}"))
        .collect();
    out.extend(
        moves
            .into_iter()
            .map(|((page, t, m, c), n)| format!("{name}\t{page}\t{t}\t{m}\t{c}\t{n}\t-")),
    );
    out
}

/// The order of pin lines: by name, then page by number, then the rest.
fn pin_order(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |l: &str| {
        let mut f = l.splitn(3, '\t');
        let name = f.next().unwrap_or("").to_string();
        let page = f
            .next()
            .and_then(|p| p.parse::<u64>().ok())
            .unwrap_or(u64::MAX);
        (name, page, f.next().unwrap_or("").to_string())
    };
    key(a).cmp(&key(b))
}

const PIN_HEAD: &str = "# name\tpage\ttransform\twhat\tclass\tcount\towner\tcause";

/// The fields of a pinned line that are checked: all but the cause.
const PIN_FIELDS: usize = 7;

/// The fields that name a pin: the count and what follows it say how it stands.
const PIN_KEY: usize = 5;

/// The first `n` fields of the pin line `l`.
fn fields(l: &str, n: usize) -> String {
    l.split('\t').take(n).collect::<Vec<_>>().join("\t")
}

/// The pinned lines of the file `path` (its comment lines left out), without the cause a
/// line may carry as its last field.
pub fn pinned(path: &std::path::Path) -> Vec<String> {
    pinned_whole(path)
        .iter()
        .map(|l| fields(l, PIN_FIELDS))
        .collect()
}

/// The open owner a traced cause names: a cause written by hand may start with `owner <who>`
/// (`owner Task 37. Traced ...`, `owner after 0.1.0, kept. ...`), where `<who>` is
/// `after 0.1.0` or the name of a task (one word of letters and digits). `None` for a cause
/// that names no owner.
pub fn traced_owner(cause: &str) -> Option<&str> {
    const LATER: &str = "after 0.1.0";
    let who = cause.strip_prefix("owner ")?;
    if who.starts_with(LATER) {
        return Some(&who[..LATER.len()]);
    }
    let who = who.strip_prefix("Task ").unwrap_or(who);
    let end = who
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(who.len());
    (end > 0).then_some(&who[..end])
}

/// The found pin line `l` with the owner its pin names: when the pin of its name, page,
/// change, kind and class in `whole` (the pinned lines with their causes) carries a cause
/// that names an open owner (see [`traced_owner`]), that owner stands in the owner column
/// instead of the computed one (see [`owner`]: it knows the class of a break, not its traced
/// cause). A move to or from spatial is no break and keeps its `-`.
fn with_traced_owner(l: &str, whole: &[String]) -> String {
    if l.split('\t').nth(PIN_FIELDS - 1) == Some("-") {
        return l.to_string();
    }
    let key = fields(l, PIN_KEY);
    let traced = whole
        .iter()
        .find(|w| fields(w, PIN_KEY) == key)
        .and_then(|w| w.split('\t').nth(PIN_FIELDS))
        .and_then(traced_owner);
    match traced {
        Some(who) => format!("{}\t{who}", fields(l, PIN_FIELDS - 1)),
        None => l.to_string(),
    }
}

/// The pinned lines of the file `path` as they stand in it, with their causes.
fn pinned_whole(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// The names the file `path` pins.
pub fn pinned_names(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = pinned(path)
        .iter()
        .filter_map(|l| l.split('\t').next().map(str::to_string))
        .collect();
    names.dedup();
    names
}

/// Checks `lines` (see [`pin_lines`]) against the pinned file `path`, for the names in
/// `names` only: a line that moved, came or went fails with each such line. With
/// `REZZER_WRITE_META` set, the lines of `names` in the file are replaced by `lines` instead;
/// the lines of every other name stay as they are. A line may carry the traced cause of its
/// break as a last field, written by hand: it is not checked, and a rewritten line keeps the
/// cause of the pin of its name, page, change, kind and class. A cause that names an open
/// owner sets the owner column of its line (see [`traced_owner`]), so the column never names
/// another owner than the cause does.
pub fn check_pins(path: &std::path::Path, names: &[String], lines: &[String]) {
    let write = std::env::var_os("REZZER_WRITE_META").is_some();
    check_or_write_pins(path, names, lines, write);
}

/// [`check_pins`] with the write mode given, so a test can write pins without setting the
/// process environment (other tests of the same binary may run at the same time).
pub fn check_or_write_pins(
    path: &std::path::Path,
    names: &[String],
    lines: &[String],
    write: bool,
) {
    let mine = |l: &String| {
        names
            .iter()
            .any(|n| l.split('\t').next() == Some(n.as_str()))
    };
    let old = pinned(path);
    let whole = pinned_whole(path);
    let lines: Vec<String> = lines.iter().map(|l| with_traced_owner(l, &whole)).collect();
    let lines = &lines;
    if write {
        let old: Vec<String> = old.iter().map(|l| with_traced_owner(l, &whole)).collect();
        let cause = |l: &String| {
            let key = fields(l, PIN_KEY);
            whole
                .iter()
                .find(|w| fields(w, PIN_KEY) == key)
                .and_then(|w| w.split('\t').nth(PIN_FIELDS))
                .map(|c| format!("{l}\t{c}"))
                .unwrap_or_else(|| l.clone())
        };
        let mut all: Vec<String> = old.iter().filter(|l| !mine(l)).map(cause).collect();
        all.extend(lines.iter().map(cause));
        all.sort_by(|a, b| pin_order(a, b));
        let mut text = vec![PIN_HEAD.to_string()];
        text.extend(all);
        std::fs::write(path, text.join("\n") + "\n").expect("write pins");
        return;
    }
    let pinned: Vec<String> = old.into_iter().filter(|l| mine(l)).collect();
    let gone: Vec<&String> = pinned.iter().filter(|l| !lines.contains(l)).collect();
    let came: Vec<&String> = lines.iter().filter(|l| !pinned.contains(l)).collect();
    assert!(
        gone.is_empty() && came.is_empty(),
        "metamorphic breaks moved against {}:\npinned, not found:\n{}\nfound, not pinned:\n{}",
        path.display(),
        gone.iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        came.iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
