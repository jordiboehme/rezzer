// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Metamorphic tests of the region engine: changes to a page that keep its meaning (a scale,
//! a shift, a little jitter, runs split at word gaps or joined, rules in pieces or drawn as
//! thin bars, the primitives in another order) and the comparison of the engine's decisions
//! on the page before and after. A change may keep every decision or move a typed region to
//! a spatial one; it may never give a different typed read, lose a run or read one twice.
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
    /// The typed regions both pages read the same stand in another order. One break per page
    /// at most: the first region found out of order is named, not every one.
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
/// - runs in another order and rules drawn as filled bars are input the PDF reader never
///   hands over (it orders the runs and turns bars into lines before the engine), so the
///   engine's own taking of such input comes after 0.1.0;
/// - labels and values: Task 26, but for the changes whose cause is the segmentation (rules
///   in pieces part the strips; absolute points in the drawn structure under a scale of 0.5
///   or 2), which Task 31 owns;
/// - a text region read as another text kind (heading, list, paragraph, legend): Task 36a;
/// - every other cut, join or order of regions (the segmentation, the bands of running
///   headers and footers): Task 31.
pub fn owner(t: Transform, what: Break, class: &str) -> &'static str {
    match (t, what, class) {
        (Transform::ReorderRuns | Transform::RuleAsBox, _, _) => "after 0.1.0",
        (Transform::SplitRules | Transform::Scale(50) | Transform::Scale(200), _, "pairs") => "31",
        (_, _, "pairs") => "26",
        (_, Break::Kind, "text") => "36a",
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
    // The typed regions read the same keep their order.
    matched.sort_by_key(|&(_, cn)| cn);
    if let Some(w) = matched.windows(2).find(|w| w[0].0 > w[1].0) {
        let moved = b.iter().find(|x| x.at == w[1].0);
        v.broken.push(Broken {
            what: Break::Order,
            kinds: moved.map(|x| vec![x.kind]).unwrap_or_default(),
            detail: moved.map_or_else(String::new, |x| {
                format!("{} moved", label(page, x.kind, &x.members))
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
/// - one line per change, move (`to-spatial`, `from-spatial`) and class with its number of
///   moves over the whole file, page `*` and owner `-`.
///
/// A pinned file holds these lines: a break more or less on any page, a new break or a
/// different number of moves moves a line.
pub fn pin_lines(name: &str, findings: &[Finding]) -> Vec<String> {
    let mut breaks: BTreeMap<(usize, String, &'static str, &'static str), (usize, &str)> =
        BTreeMap::new();
    let mut moves: BTreeMap<(String, &'static str, &'static str), usize> = BTreeMap::new();
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
                    .entry((f.transform.name(), m, class_of(&[*k])))
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
            .map(|((t, m, c), n)| format!("{name}\t*\t{t}\t{m}\t{c}\t{n}\t-")),
    );
    out
}

/// The order of pin lines: by name, then page by number (`*` last), then the rest.
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

const PIN_HEAD: &str = "# name\tpage\ttransform\twhat\tclass\tcount\towner";

/// The pinned lines of the file `path` (its comment lines left out).
pub fn pinned(path: &std::path::Path) -> Vec<String> {
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
/// the lines of every other name stay as they are.
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
    if write {
        let mut all: Vec<String> = old.into_iter().filter(|l| !mine(l)).collect();
        all.extend(lines.iter().cloned());
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
