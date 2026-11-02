// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use crate::input::{PageIn, PrimId};
use crate::limits::Limits;
use crate::region::{Candidate, Certainty, Issue, IssueKind, PageRegions, Region, RegionId};
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

/// Arbitration by order: the first claim on a piece wins. A later step adds the scored rule.
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
            .push(Region::decided(id, self.page, c, Certainty::Rule));
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
