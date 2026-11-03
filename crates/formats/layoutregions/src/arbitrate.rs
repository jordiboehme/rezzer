// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use crate::input::{PageIn, PrimId};
use crate::limits::Limits;
use crate::region::{
    Alternative, Candidate, Certainty, Issue, IssueKind, Kind, PageRegions, Region, RegionId,
    Score, Signal,
};
use crate::segment::{SegId, Segments, Source};
use crate::typers::{MAX_SCORE, Typed};
use crate::work;

/// The answer to an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    Accepted(RegionId),
    /// A piece of the candidate already belongs to this region.
    Refused {
        by: RegionId,
    },
    /// The region limit is reached.
    Limit,
}

/// Arbitration by order: the first claim on a piece wins. [`arbitrate()`] is the scored rule.
#[derive(Debug)]
pub struct InOrder {
    page: u32,
    limits: Limits,
    owner: HashMap<PrimId, RegionId>,
    regions: Vec<Region>,
    issues: Vec<Issue>,
}

impl InOrder {
    pub fn new(page: &PageIn, limits: &Limits) -> InOrder {
        InOrder {
            page: page.page,
            limits: *limits,
            owner: HashMap::new(),
            regions: Vec::new(),
            issues: Vec::new(),
        }
    }

    /// Offers a candidate. It is accepted when no piece of it belongs to a region yet. When
    /// the region limit is reached, one `Limit` issue is recorded the first time only, not
    /// once per refused offer.
    pub fn offer(&mut self, c: Candidate) -> Offer {
        self.offer_as(c, Certainty::Rule)
    }

    /// [`InOrder::offer`] for a region decided with `certainty`: `Exact` where the file states
    /// the region (a tagged table), `Rule` where a rule found it.
    pub fn offer_as(&mut self, c: Candidate, certainty: Certainty) -> Offer {
        work::add(1 + c.prims.len() as u64);
        if self.regions.len() >= self.limits.max_regions {
            self.note_limit();
            return Offer::Limit;
        }
        if let Some(by) = c.prims.iter().find_map(|p| self.owner.get(p).copied()) {
            return Offer::Refused { by };
        }
        let id = RegionId(self.regions.len() as u32);
        for p in &c.prims {
            self.owner.insert(*p, id);
        }
        self.regions
            .push(Region::decided(id, self.page, c, certainty));
        Offer::Accepted(id)
    }

    /// Whether a region already holds the piece.
    pub fn is_taken(&self, p: PrimId) -> bool {
        self.owner.contains_key(&p)
    }

    pub fn finish(self) -> PageRegions {
        PageRegions {
            page: self.page,
            regions: self.regions,
            issues: self.issues,
        }
    }

    fn note_limit(&mut self) {
        if !self.issues.iter().any(|i| i.kind == IssueKind::Limit) {
            self.issues.push(Issue {
                what: format!(
                    "the limit of {} regions per page is reached",
                    self.limits.max_regions
                ),
                kind: IssueKind::Limit,
            });
        }
    }

    fn note_stopped(&mut self) {
        self.issues.push(Issue {
            what: "the stop callback ended the region search".to_string(),
            kind: IssueKind::Stopped,
        });
    }
}

/// One way of finding regions. `C` is the host's context.
pub trait Finder<C: ?Sized> {
    /// The signal id of its regions, such as `pdf.box_pairs`.
    fn name(&self) -> &'static str;
    fn find(&mut self, page: &PageIn, cx: &mut C, arb: &mut InOrder);
}

/// Calls each finder once, in slice order. `stop()` is asked before every finder, the first
/// one included; when it is true, that finder and the rest are skipped and a `Stopped` issue
/// is recorded.
pub fn find_in_order<C: ?Sized>(
    page: &PageIn,
    finders: &mut [&mut dyn Finder<C>],
    cx: &mut C,
    arb: &mut InOrder,
    stop: &dyn Fn() -> bool,
) {
    for f in finders.iter_mut() {
        work::add(1);
        if stop() {
            arb.note_stopped();
            return;
        }
        f.find(page, cx, arb);
    }
}

/// What a decision by score needs: a least score, a lead over the next kind, and how many
/// alternatives a decision keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Margins {
    /// The least score that decides a segment.
    pub min: Score,
    /// How far the best kind must lead the next one.
    pub margin: Score,
    /// Alternatives kept per decision (see [`Limits::max_alternatives`]).
    pub max_alternatives: usize,
}

impl Default for Margins {
    /// The starting values: min 300, margin 150, and the alternatives of
    /// [`Limits::default`].
    fn default() -> Self {
        Margins::of(&Limits::default())
    }
}

impl Margins {
    /// The starting values, with the alternatives `limits` keeps.
    pub fn of(limits: &Limits) -> Margins {
        Margins {
            min: Score(300),
            margin: Score(150),
            max_alternatives: limits.max_alternatives,
        }
    }
}

/// The kind a segment is decided as, how sure, with its evidence and the kinds that lost.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The segment of the tree that stands as one region.
    pub seg: SegId,
    pub kind: Kind,
    /// `Exact` for a region the file states, `Rule` for a clear winner, `Unsure` for `Spatial`.
    pub certainty: Certainty,
    /// The winner's score; 1000 for a region the file states, 0 for `Spatial`.
    pub score: Score,
    /// The winner's signals; for `Spatial`, the `spatial.*` refusals that say why no table was
    /// read (weight 0), if any; for a region the file states, `given.file_states`.
    pub signals: Vec<Signal>,
    /// The kinds that lost, best first (see [`arbitrate()`]).
    pub alternatives: Vec<Alternative>,
}

/// The decisions of a page: the segments of `segs` that stand, in the order of the tree's cuts,
/// each with one kind. Every primitive of the page is in exactly one of them. `typed` holds the
/// candidates of each segment (see [`crate::typers::type_segments`]); a segment it lacks has
/// none.
///
/// One segment: the best candidate wins (certainty `Rule`) only with a signal, a score of at
/// least `m.min` and a lead of at least `m.margin` over the next kind; else the segment is
/// `Spatial` (certainty `Unsure`, score 0) with every candidate as an alternative. Candidates of
/// kind `Spatial` (the table rules' `spatial.*` refusals, of weight 0) never win and are no
/// alternative: their signals say why an unsure segment read no table.
///
/// A region the file states is decided as given: its kind, certainty `Exact`, score 1000.
///
/// The tree, bottom up: a cut (the children, or an other cut of `alternatives`) reads as sure as
/// its weakest decision (a `Spatial` one counts 0, a given one 1000). A leaf with no text,
/// picture or code (rules and boxes alone) has nothing to read: it is decided `Spatial` and
/// does not count. Among the cuts of a segment the one with the highest weakest score wins when
/// it leads every other by the margin, else the children stand; the losing cuts' kinds are
/// alternatives on the decisions that share primitives with them. The segment's own decision
/// replaces the cut when it is not `Spatial`, its score is at least the cut's weakest plus the
/// margin, and it leads by the margin every decision of the cut (grandchildren included) of
/// another kind, for an unsure one its best alternative of another kind: a key/value or table
/// reading below is never folded into a weaker reading of another kind. Nor does it replace
/// the cut when a sure decision of its own kind there scores as much as it does while a sure
/// decision of another kind stands beside it (its reading only spreads that child's over its
/// neighbours), or when it is a figure over a sure decision of a text kind (a figure keeps the
/// text of its region as text). It then carries the
/// cut's kinds as alternatives. Otherwise the cut stands and the segment's best candidate is
/// an alternative on each of its decisions. The page root never replaces its cut: its
/// candidates are alternatives on the decisions under it. Alternatives are one per kind (the
/// best score), never of a decision's own kind, best first, at most `m.max_alternatives`; a
/// given region has none.
///
/// Each decision is made once and moved, never copied with its subtree: the decisions of a
/// segment stand together at the end of one list while it is read.
pub fn arbitrate(
    segs: &Segments,
    typed: &BTreeMap<SegId, Vec<Typed>>,
    m: &Margins,
) -> Vec<Decision> {
    let mut a = Arbiter {
        segs,
        typed,
        m,
        given: HashMap::new(),
        out: Vec::new(),
    };
    if segs.all.get(segs.root.0 as usize).is_some() {
        a.read(segs.root);
    }
    a.out
}

struct Arbiter<'a> {
    segs: &'a Segments,
    typed: &'a BTreeMap<SegId, Vec<Typed>>,
    m: &'a Margins,
    /// The decisions of the given regions: one stands in the root's children and in its other
    /// cuts.
    given: HashMap<SegId, Decision>,
    /// The decisions read so far: those of the segment being read stand at the end.
    out: Vec<Decision>,
}

impl Arbiter<'_> {
    fn cands(&self, s: SegId) -> &[Typed] {
        self.typed.get(&s).map_or(&[], Vec::as_slice)
    }

    /// Reads `s`: its decisions are pushed to the end of `out`. Returns their weakest score,
    /// `None` when none counts.
    fn read(&mut self, s: SegId) -> Option<u16> {
        work::add(1);
        let seg = &self.segs.all[s.0 as usize];
        if let Source::Given(kind) = seg.source {
            let d = self
                .given
                .entry(s)
                .or_insert_with(|| given(s, kind))
                .clone();
            self.out.push(d);
            return Some(MAX_SCORE);
        }
        let own = decide(s, self.cands(s), self.m);
        if seg.children.is_empty() && seg.alternatives.is_empty() {
            let readable = seg
                .prims
                .iter()
                .any(|p| matches!(p, PrimId::Text(_) | PrimId::Picture(_) | PrimId::Code(_)));
            let weakest = readable.then(|| strength(&own));
            self.out.push(own);
            return weakest;
        }
        let root = seg.source == Source::Page;
        self.parent(s, root, own)
    }

    /// Reads the segment `s` with cuts (its children first, then its other cuts) and its own
    /// decision `own`.
    fn parent(&mut self, s: SegId, root: bool, own: Decision) -> Option<u16> {
        let seg = &self.segs.all[s.0 as usize];
        let start = self.out.len();
        let mut cuts: Vec<(Range<usize>, Option<u16>)> = Vec::new();
        let lists = (!seg.children.is_empty())
            .then_some(&seg.children)
            .into_iter()
            .chain(seg.alternatives.iter());
        for cut in lists {
            let from = self.out.len();
            let mut weakest: Option<u16> = None;
            for &c in cut {
                let w = self.read(c);
                weakest = match (weakest, w) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            cuts.push((from..self.out.len(), weakest));
        }
        let key = |w: Option<u16>| u32::from(w.unwrap_or(0));
        let margin = u32::from(self.m.margin.0);
        let mut best = 0;
        for k in 1..cuts.len() {
            if key(cuts[k].1) > key(cuts[best].1) {
                best = k;
            }
        }
        let ahead = (0..cuts.len())
            .filter(|&k| k != best)
            .all(|k| key(cuts[best].1) >= key(cuts[k].1) + margin);
        if !ahead {
            best = 0;
        }
        let (chosen, weakest) = cuts[best].clone();
        if cuts.len() > 1 {
            let losers: Vec<usize> = cuts
                .iter()
                .enumerate()
                .filter(|&(k, _)| k != best)
                .flat_map(|(_, (r, _))| r.clone())
                .collect();
            self.share(chosen.clone(), &losers);
            // Keep the chosen cut alone at the end: the cuts after it are cut off, those before
            // it go and it moves up (nothing moves when the children's cut stands).
            self.out.truncate(chosen.end);
            self.out.drain(start..chosen.start);
        }
        let standing = start..self.out.len();
        let leads = |score: u16| u32::from(own.score.0) >= u32::from(score) + margin;
        let cut = &self.out[standing.clone()];
        let sure_other = |d: &Decision| {
            d.certainty == Certainty::Rule && d.kind != own.kind && d.kind != Kind::Spatial
        };
        // A parent of the kind of a sure child it does not outscore only spreads that child's
        // reading over its neighbours: no evidence for them, so it folds no sure decision of
        // another kind (an address under a table, a company cell beside boxed fields).
        let spreads = cut.iter().any(|d| {
            d.certainty == Certainty::Rule && d.kind == own.kind && d.score.0 >= own.score.0
        }) && cut.iter().any(sure_other);
        // A figure keeps the text of its region as text: it folds no sure decision of a text
        // kind.
        let hides_text = own.kind == Kind::Figure
            && cut
                .iter()
                .any(|d| sure_other(d) && !matches!(d.kind, Kind::Code | Kind::Figure));
        let replaces = !root
            && !spreads
            && !hides_text
            && own.kind != Kind::Spatial
            && u32::from(own.score.0) >= key(weakest) + margin
            && self.out[standing.clone()]
                .iter()
                .all(|d| match d.certainty {
                    Certainty::Unsure => d
                        .alternatives
                        .iter()
                        .find(|a| a.kind != own.kind)
                        .is_none_or(|a| leads(a.score.0)),
                    _ => d.kind == own.kind || leads(d.score.0),
                });
        work::add(1 + standing.len() as u64);
        if replaces {
            let mut own = own;
            let kids: Vec<Alternative> = self.out[standing.clone()]
                .iter()
                .filter_map(alternative)
                .collect();
            add(&mut own, kids, self.m);
            self.out.truncate(start);
            let w = Some(own.score.0);
            self.out.push(own);
            return w;
        }
        let cands = self.cands(s);
        let mut order: Vec<&Typed> = cands.iter().filter(|t| t.kind != Kind::Spatial).collect();
        sort(&mut order);
        let take = if root { self.m.max_alternatives } else { 1 };
        let from: Vec<Alternative> = order.into_iter().take(take).map(as_alternative).collect();
        if !from.is_empty() {
            for d in &mut self.out[standing] {
                add(d, from.clone(), self.m);
            }
        }
        weakest
    }

    /// Puts the kinds of the decisions `losers` (indexes into `out`) of other cuts on each
    /// decision of `chosen` that shares a primitive with them.
    fn share(&mut self, chosen: Range<usize>, losers: &[usize]) {
        let mut at: HashMap<PrimId, usize> = HashMap::new();
        for k in chosen {
            let prims = &self.segs.all[self.out[k].seg.0 as usize].prims;
            work::add(prims.len() as u64);
            for p in prims {
                at.insert(*p, k);
            }
        }
        let mut puts: Vec<(usize, Alternative)> = Vec::new();
        for &l in losers {
            let d = &self.out[l];
            let Some(alt) = alternative(d) else { continue };
            let prims = &self.segs.all[d.seg.0 as usize].prims;
            work::add(prims.len() as u64);
            let mut hit: Vec<usize> = prims
                .iter()
                .filter_map(|p| at.get(p).copied())
                .collect::<HashSet<usize>>()
                .into_iter()
                .collect();
            hit.sort_unstable();
            puts.extend(hit.into_iter().map(|k| (k, alt.clone())));
        }
        for (k, alt) in puts {
            add(&mut self.out[k], vec![alt], self.m);
        }
    }
}

/// How sure a decision is for the weakest of its cut: a given region 1000, a spatial one 0,
/// any other its score.
fn strength(d: &Decision) -> u16 {
    match (d.certainty, d.kind) {
        (Certainty::Exact, _) => MAX_SCORE,
        (_, Kind::Spatial) => 0,
        _ => d.score.0,
    }
}

/// A decision of another reading as an alternative: none for a spatial or a given one.
fn alternative(d: &Decision) -> Option<Alternative> {
    (d.kind != Kind::Spatial && d.certainty != Certainty::Exact).then(|| Alternative {
        kind: d.kind,
        score: d.score,
        signals: d.signals.clone(),
    })
}

/// Best score first, then in kind order.
fn sort(c: &mut [&Typed]) {
    c.sort_by(|a, b| b.score.cmp(&a.score).then(a.kind.cmp(&b.kind)));
}

/// The decision of a region the file states.
fn given(s: SegId, kind: Kind) -> Decision {
    Decision {
        seg: s,
        kind,
        certainty: Certainty::Exact,
        score: Score(MAX_SCORE),
        signals: vec![Signal {
            id: "given.file_states",
            detail: kind.name().to_string(),
            weight: MAX_SCORE,
        }],
        alternatives: Vec::new(),
    }
}

/// One segment: the best candidate wins only with positive evidence and a clear margin.
fn decide(s: SegId, cands: &[Typed], m: &Margins) -> Decision {
    work::add(1 + cands.len() as u64);
    let (spatial, mut c): (Vec<&Typed>, Vec<&Typed>) =
        cands.iter().partition(|t| t.kind == Kind::Spatial);
    sort(&mut c);
    let second = c.get(1).map_or(Score(0), |t| t.score);
    match c.first() {
        Some(best)
            if best.score >= m.min
                && !best.signals.is_empty()
                && best.score.0 - second.0 >= m.margin.0 =>
        {
            let mut d = Decision {
                seg: s,
                kind: best.kind,
                certainty: Certainty::Rule,
                score: best.score,
                signals: best.signals.clone(),
                alternatives: Vec::new(),
            };
            add(&mut d, alternatives(&c[1..], m), m);
            d
        }
        _ => unsure(s, &c, &spatial, m),
    }
}

/// The candidates `c` (sorted, one per kind) as alternatives, as many as a decision keeps.
fn alternatives(c: &[&Typed], m: &Margins) -> Vec<Alternative> {
    c.iter()
        .take(m.max_alternatives)
        .map(|t| as_alternative(t))
        .collect()
}

/// Kind `Spatial`, certainty `Unsure`, every candidate an alternative; the signals are the
/// `spatial.*` refusals that say why no table was read.
fn unsure(s: SegId, c: &[&Typed], spatial: &[&Typed], m: &Margins) -> Decision {
    let mut d = Decision {
        seg: s,
        kind: Kind::Spatial,
        certainty: Certainty::Unsure,
        score: Score(0),
        signals: spatial
            .iter()
            .flat_map(|t| t.signals.iter().cloned())
            .collect(),
        alternatives: Vec::new(),
    };
    add(&mut d, alternatives(c, m), m);
    d
}

fn as_alternative(t: &Typed) -> Alternative {
    Alternative {
        kind: t.kind,
        score: t.score,
        signals: t.signals.clone(),
    }
}

/// Adds `more` to the alternatives of `d`: one per kind (the best score), never of its own
/// kind or of `Spatial`, best first then in kind order, at most `m.max_alternatives`. A given
/// region takes none.
fn add(d: &mut Decision, more: Vec<Alternative>, m: &Margins) {
    if d.certainty == Certainty::Exact {
        return;
    }
    work::add(1 + more.len() as u64);
    for a in more {
        if a.kind == d.kind || a.kind == Kind::Spatial {
            continue;
        }
        match d.alternatives.iter_mut().find(|x| x.kind == a.kind) {
            Some(x) if x.score >= a.score => {}
            Some(x) => *x = a,
            None => d.alternatives.push(a),
        }
    }
    d.alternatives
        .sort_by(|a, b| b.score.cmp(&a.score).then(a.kind.cmp(&b.kind)));
    d.alternatives.truncate(m.max_alternatives);
}
