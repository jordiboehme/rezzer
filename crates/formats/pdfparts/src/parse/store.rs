// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Objects as the document needs them. A plain object is read at its offset; a member of an
//! object stream is read when the first member of its container is needed, and the container
//! is unpacked exactly once. Reachability from the trailer decides what is kept.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use lopdf::{Dictionary, Object, ObjectId};

use super::crypt::Crypt;
use super::lexer::Lexer;
use super::object::{finish, head_at, header_at, length_ref};
use super::predict::{last_parms, unpredict};
use super::xref::{Entry, rebuild, unpack_stream};
use super::{Budget, Fail};
use crate::{Issue, IssueKind, OpenError};

/// The highest object number ISO 32000-1 allows (Annex C). Higher numbers are taken as
/// missing: lopdf's writer adds one to the highest number it writes and walks every number
/// up to it.
pub(crate) const OBJECT_MAX: u32 = 8_388_607;

/// The text of the issue a rebuilt table gives.
pub(crate) const REBUILT: &str = "the cross-reference table is damaged, rebuilt from a file scan";

pub(crate) struct Store<'a> {
    bytes: &'a [u8],
    base: usize,
    table: HashMap<u32, Entry>,
    /// The table found by a scan of the file, made the first time the table fails.
    scanned: Option<HashMap<u32, (usize, u16)>>,
    pub budget: Budget<'a>,
    pub crypt: Option<Crypt>,
    /// Parsed members of unpacked object streams that nobody has taken yet.
    members: HashMap<u32, Object>,
    unpacked: HashSet<u32>,
    busy: HashSet<u32>,
    lengths: HashMap<ObjectId, Option<i64>>,
    /// The header found at each position read so far.
    headers: HashMap<usize, Option<ObjectId>>,
    pub issues: Vec<Issue>,
}

impl<'a> Store<'a> {
    pub fn new(
        bytes: &'a [u8],
        base: usize,
        table: HashMap<u32, Entry>,
        budget: Budget<'a>,
        scanned: Option<HashMap<u32, (usize, u16)>>,
    ) -> Store<'a> {
        let mut store = Store {
            bytes,
            base,
            table,
            scanned: None,
            budget,
            crypt: None,
            members: HashMap::new(),
            unpacked: HashSet::new(),
            busy: HashSet::new(),
            lengths: HashMap::new(),
            headers: HashMap::new(),
            issues: Vec::new(),
        };
        if scanned.is_some() {
            store.note_rebuilt();
            store.scanned = scanned;
        }
        store
    }

    fn note_rebuilt(&mut self) {
        self.issues
            .push(Issue::new("document", IssueKind::Damaged, REBUILT));
    }

    pub fn entry(&self, n: u32) -> Option<Entry> {
        self.table.get(&n).copied()
    }

    fn damaged(&mut self, id: ObjectId, what: &str) {
        self.issues.push(Issue::new(
            &format!("object {} {}", id.0, id.1),
            IssueKind::Damaged,
            what,
        ));
    }

    /// Reads object `id`. `None` when it is missing or damaged (with an issue for damage).
    pub fn load(&mut self, id: ObjectId) -> Result<Option<Object>, OpenError> {
        if id.0 > OBJECT_MAX {
            return Ok(None);
        }
        let mut tried = None;
        match self.entry(id.0) {
            Some(Entry::Normal { pos, generation }) if generation == id.1 => {
                if let Some(o) = self.top(id, pos, true)? {
                    return Ok(Some(o));
                }
                tried = Some(pos);
            }
            Some(Entry::Compressed { container, .. }) if id.1 == 0 => {
                return self.member(id.0, container);
            }
            Some(_) => return Ok(None),
            None if self.scanned.is_none() => return Ok(None),
            None => {}
        }
        match self.scanned_pos(id)? {
            Some(pos) if Some(pos) != tried => self.top(id, pos, true),
            _ => Ok(None),
        }
    }

    /// The header at `pos`, read once per position.
    fn header(&mut self, pos: usize) -> Result<Option<ObjectId>, OpenError> {
        if let Some(h) = self.headers.get(&pos) {
            return Ok(*h);
        }
        let h = header_at(self.bytes, pos, &mut self.budget).map_err(Fail::into_open)?;
        self.headers.insert(pos, h);
        Ok(h)
    }

    /// Where a scan of the file finds object `id`. The table pointed at something else, so
    /// the scan is made now if it was not made yet.
    fn scanned_pos(&mut self, id: ObjectId) -> Result<Option<usize>, OpenError> {
        if self.scanned.is_none() {
            let found = rebuild(self.bytes, &mut self.budget)?;
            self.note_rebuilt();
            self.scanned = Some(found.table);
        }
        let at = self.scanned.as_ref().and_then(|t| t.get(&id.0)).copied();
        Ok(at
            .filter(|(_, generation)| *generation == id.1)
            .map(|(pos, _)| pos))
    }

    /// The integer value of the plain object `r` at `pos`, if that is what is there.
    fn int_at(&mut self, r: ObjectId, pos: usize) -> Result<Option<Option<i64>>, OpenError> {
        if self.header(pos)? != Some(r) {
            return Ok(None);
        }
        match head_at(self.bytes, pos, &mut self.budget) {
            Ok(head) if head.id == r && head.stream.is_none() => Ok(Some(head.value.as_i64().ok())),
            Ok(_) | Err(Fail::Bad(_)) => Ok(None),
            Err(Fail::Stop(e)) => Err(e),
        }
    }

    /// Reads the plain object `id` at `pos` and decrypts it. `members` allows an indirect
    /// `/Length` inside an object stream.
    fn top(
        &mut self,
        id: ObjectId,
        pos: usize,
        members: bool,
    ) -> Result<Option<Object>, OpenError> {
        let mut found = None;
        // Offsets count from `%PDF-`; some writers count from the first byte.
        let other = pos.checked_sub(self.base).filter(|_| self.base > 0);
        for at in std::iter::once(pos).chain(other) {
            if at >= self.bytes.len() {
                continue;
            }
            // The header is read first, so an entry that points at another object costs
            // nothing more than its header.
            if self.header(at)? != Some(id) {
                continue;
            }
            match head_at(self.bytes, at, &mut self.budget) {
                Ok(head) => {
                    found = Some(head);
                    break;
                }
                Err(Fail::Bad(why)) => {
                    self.damaged(id, why);
                    return Ok(None);
                }
                Err(Fail::Stop(e)) => return Err(e),
            }
        }
        let Some(head) = found else {
            return Ok(None);
        };
        let length = match length_ref(&head) {
            Some(r) => self.length_of(r, members)?,
            None => None,
        };
        let mut obj = match finish(self.bytes, head, length, &mut self.budget) {
            Ok((_, o)) => o,
            Err(Fail::Bad(why)) => {
                self.damaged(id, why);
                return Ok(None);
            }
            Err(Fail::Stop(e)) => return Err(e),
        };
        if let Some(crypt) = &self.crypt
            && id != crypt.skip
            && crypt.decrypt(id, &mut obj).is_err()
        {
            self.damaged(id, "the object could not be decrypted");
            return Ok(None);
        }
        Ok(Some(obj))
    }

    /// The integer value of an indirect `/Length`, from a plain object or a member of an
    /// object stream (unpacked at most once, never while it is being unpacked).
    fn length_of(&mut self, r: ObjectId, members: bool) -> Result<Option<i64>, OpenError> {
        if let Some(v) = self.lengths.get(&r) {
            return Ok(*v);
        }
        let v = match self.entry(r.0) {
            Some(Entry::Normal { pos, generation }) if generation == r.1 => {
                match self.int_at(r, pos)? {
                    Some(v) => v,
                    None => match self.scanned_pos(r)? {
                        Some(at) if at != pos => self.int_at(r, at)?.flatten(),
                        _ => None,
                    },
                }
            }
            Some(Entry::Compressed { container, .. })
                if members && r.1 == 0 && !self.busy.contains(&container) =>
            {
                if !self.unpacked.contains(&container) {
                    self.unpack(container)?;
                }
                self.members.get(&r.0).and_then(|o| o.as_i64().ok())
            }
            _ => None,
        };
        self.lengths.insert(r, v);
        Ok(v)
    }

    fn member(&mut self, n: u32, container: u32) -> Result<Option<Object>, OpenError> {
        if !self.unpacked.contains(&container) {
            self.unpack(container)?;
        }
        let found = self.members.remove(&n);
        if let Some(Object::Integer(v)) = &found {
            self.lengths.insert((n, 0), Some(*v));
        }
        Ok(found)
    }

    /// Unpacks object stream `c` once and parses the members the table places in it.
    fn unpack(&mut self, c: u32) -> Result<(), OpenError> {
        self.unpacked.insert(c);
        let Some(Entry::Normal { pos, generation }) = self.entry(c) else {
            self.damaged(
                (c, 0),
                "the object stream is listed inside another object stream",
            );
            return Ok(());
        };
        self.busy.insert(c);
        let container = self.top((c, generation), pos, false)?;
        let result = match container {
            Some(Object::Stream(s)) if s.dict.has_type(b"ObjStm") => self.members_of(c, &s),
            Some(_) => {
                self.damaged((c, generation), "the object is not an object stream");
                Ok(())
            }
            None => Ok(()),
        };
        self.busy.remove(&c);
        result
    }

    fn members_of(&mut self, c: u32, s: &lopdf::Stream) -> Result<(), OpenError> {
        let Some(data) = unpack_stream(s, &mut self.budget)? else {
            self.damaged((c, 0), "the object stream cannot be unpacked");
            return Ok(());
        };
        let data = unpredict(data, last_parms(&s.dict));
        let int = |d: &Dictionary, k: &[u8]| d.get(k).ok().and_then(|o| o.as_i64().ok());
        let count = int(&s.dict, b"N").unwrap_or(0).max(0) as u64;
        if count > self.budget.limits.max_items as u64 {
            return Err(OpenError::Limit(format!(
                "object stream {c} lists more than {} members",
                self.budget.limits.max_items
            )));
        }
        let first = int(&s.dict, b"First").unwrap_or(0).max(0) as usize;
        // Object number to offset, for the members the table puts here. A member at the index
        // the table names wins over another with the same number.
        let mut wanted: HashMap<u32, (usize, bool)> = HashMap::new();
        let mut l = Lexer::new(&data, 0);
        for i in 0..count {
            let (Some(m), Some(off)) = (l.uint(), l.uint()) else {
                break;
            };
            self.budget.value(0).map_err(Fail::into_open)?;
            let (Ok(m), Ok(off)) = (u32::try_from(m), usize::try_from(off)) else {
                continue;
            };
            if let Some(Entry::Compressed { container, index }) = self.entry(m)
                && container == c
            {
                let exact = u64::from(index) == i;
                match wanted.get(&m) {
                    Some((_, true)) => {}
                    Some(_) if !exact => {}
                    _ => {
                        wanted.insert(m, (off, exact));
                    }
                }
            }
        }
        let mut list: Vec<(u32, usize)> = wanted.into_iter().map(|(m, (o, _))| (m, o)).collect();
        list.sort_unstable();
        for (m, off) in list {
            let at = first.saturating_add(off);
            if at >= data.len() {
                self.damaged((m, 0), "the member lies outside its object stream");
                continue;
            }
            match Lexer::new(&data, at).value(&mut self.budget, 0) {
                Ok(v) => {
                    self.members.insert(m, v);
                }
                Err(Fail::Bad(why)) => self.damaged((m, 0), why),
                Err(Fail::Stop(e)) => return Err(e),
            }
        }
        Ok(())
    }

    /// Every object reachable from `roots`, breadth first. Objects nothing refers to never
    /// reach the result.
    pub fn reach(&mut self, roots: &[&Object]) -> Result<BTreeMap<ObjectId, Object>, OpenError> {
        let mut out = BTreeMap::new();
        let mut seen = HashSet::new();
        let mut todo = VecDeque::new();
        for r in roots {
            refs(r, &mut |id| {
                if seen.insert(id) {
                    todo.push_back(id);
                }
            });
        }
        let skip = self.crypt.as_ref().map(|c| c.skip);
        let mut missing = 0usize;
        while let Some(id) = todo.pop_front() {
            if Some(id) == skip {
                continue;
            }
            self.budget.poll().map_err(Fail::into_open)?;
            let Some(obj) = self.load(id)? else {
                missing += 1;
                continue;
            };
            refs(&obj, &mut |r| {
                if seen.insert(r) {
                    todo.push_back(r);
                }
            });
            out.insert(id, obj);
        }
        if missing > 0 {
            self.issues.push(Issue::new(
                "document",
                IssueKind::Damaged,
                format!(
                    "{missing} referenced {} missing",
                    if missing == 1 {
                        "object is"
                    } else {
                        "objects are"
                    }
                ),
            ));
        }
        Ok(out)
    }
}

/// Calls `f` for every reference inside `obj`. Nesting was bounded when the value was parsed.
pub(crate) fn refs(obj: &Object, f: &mut dyn FnMut(ObjectId)) {
    match obj {
        Object::Reference(id) => f(*id),
        Object::Array(a) => a.iter().for_each(|o| refs(o, f)),
        Object::Dictionary(d) => d.iter().for_each(|(_, o)| refs(o, f)),
        Object::Stream(s) => s.dict.iter().for_each(|(_, o)| refs(o, f)),
        _ => {}
    }
}
