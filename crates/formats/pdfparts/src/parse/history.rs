// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Objects as an earlier revision of the file defines them. The caller passes the table of a
//! revision as a lookup; objects are read from the original bytes with the same bounded
//! parser as the open. Every object is read at most once per file position (and per value of
//! an indirect `/Length`, which a later save may change) and every object stream is unpacked
//! at most once, whichever revision asks.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use lopdf::{Object, ObjectId};

use super::crypt::Crypt;
use super::lexer::Lexer;
use super::object::{Limit, bounded, finish, head_at, header_at, length_ref};
use super::predict::{last_parms, unpredict};
use super::xref::unpack_stream;
pub(crate) use super::xref::{Entry, Part, Sections, Step, last_startxref};
use super::{Budget, Fail};
use crate::OpenError;

/// The table of one revision: where object `n` is, or `None` when it is free or missing.
pub(crate) type View<'v> = &'v dyn Fn(u32) -> Option<Entry>;

/// The unpacked data of an object stream and where its members start.
struct Packed {
    data: Vec<u8>,
    /// Object number and start of each member, in the order listed.
    members: Vec<(u32, usize)>,
    /// The member starts, sorted, to find where a member ends.
    starts: Vec<usize>,
}

/// Reads objects of any revision, each once.
pub(crate) struct Loader<'a> {
    bytes: &'a [u8],
    base: usize,
    pub budget: Budget<'a>,
    crypt: Option<&'a Crypt>,
    headers: HashMap<usize, Option<ObjectId>>,
    /// The indirect `/Length` of the object at a position, once it was read.
    length_refs: HashMap<usize, Option<ObjectId>>,
    /// Objects by position and the value of their indirect `/Length`.
    tops: HashMap<(usize, Option<i64>), Option<Rc<Object>>>,
    packed: HashMap<usize, Option<Rc<Packed>>>,
    members: HashMap<(usize, u32), Option<Rc<Object>>>,
    busy: HashSet<usize>,
    /// Objects that could not be read, each once however often it is asked for.
    pub damaged: HashSet<ObjectId>,
}

impl<'a> Loader<'a> {
    pub fn new(
        bytes: &'a [u8],
        base: usize,
        budget: Budget<'a>,
        crypt: Option<&'a Crypt>,
    ) -> Loader<'a> {
        Loader {
            bytes,
            base,
            budget,
            crypt,
            headers: HashMap::new(),
            length_refs: HashMap::new(),
            tops: HashMap::new(),
            packed: HashMap::new(),
            members: HashMap::new(),
            busy: HashSet::new(),
            damaged: HashSet::new(),
        }
    }

    /// Reads the cross-reference section at offset `off`.
    pub fn section(&mut self, sections: &mut Sections, off: u64) -> Result<Step, OpenError> {
        sections.read(self.bytes, self.base, off, &mut self.budget)
    }

    /// Adds the starts of objects and sections, once all sections are read: sorting them per
    /// section would make the walk quadratic.
    pub fn add_starts(&mut self, starts: Vec<usize>) {
        self.budget.ends.add_starts(starts);
    }

    /// The number of the indirect `/Length` object of `id` in a revision, once `id` was read.
    pub fn length_dep(&self, view: View, id: ObjectId) -> Option<u32> {
        let Some(Entry::Normal { pos, .. }) = view(id.0) else {
            return None;
        };
        let other = pos.checked_sub(self.base).filter(|_| self.base > 0);
        std::iter::once(pos)
            .chain(other)
            .find_map(|at| self.length_refs.get(&at).copied().flatten())
            .map(|r| r.0)
    }

    /// The first object of the file is a linearization dictionary.
    pub fn linearized(&mut self) -> bool {
        match head_at(self.bytes, self.base, &mut self.budget) {
            Ok(h) => matches!(&h.value, Object::Dictionary(d) if d.get(b"Linearized").is_ok()),
            Err(_) => false,
        }
    }

    /// Object `id` as the revision `view` defines it. `None` when it is free, missing or
    /// damaged.
    pub fn load(&mut self, view: View, id: ObjectId) -> Result<Option<Rc<Object>>, OpenError> {
        match view(id.0) {
            Some(Entry::Normal { pos, generation }) if generation == id.1 => {
                self.top(view, id, pos, true)
            }
            Some(Entry::Compressed { container, index }) if id.1 == 0 => {
                self.member(view, id.0, container, index)
            }
            _ => Ok(None),
        }
    }

    fn fail<T>(&mut self, f: Fail, id: ObjectId) -> Result<Option<T>, OpenError> {
        match f {
            Fail::Bad(_) => {
                self.damaged.insert(id);
                Ok(None)
            }
            Fail::Stop(e) => Err(e),
        }
    }

    fn header(&mut self, pos: usize) -> Result<Option<ObjectId>, OpenError> {
        if let Some(h) = self.headers.get(&pos) {
            return Ok(*h);
        }
        let h = header_at(self.bytes, pos, &mut self.budget).map_err(Fail::into_open)?;
        self.headers.insert(pos, h);
        Ok(h)
    }

    /// The plain object `id` at `pos` (or at `pos` counted from the first byte). `members`
    /// allows an indirect `/Length` inside an object stream. A position without the header
    /// of `id` counts as damaged.
    fn top(
        &mut self,
        view: View,
        id: ObjectId,
        pos: usize,
        members: bool,
    ) -> Result<Option<Rc<Object>>, OpenError> {
        let other = pos.checked_sub(self.base).filter(|_| self.base > 0);
        for at in std::iter::once(pos).chain(other) {
            if at >= self.bytes.len() || self.header(at)? != Some(id) {
                continue;
            }
            if !self.busy.insert(at) {
                return Ok(None);
            }
            let found = self.top_at(view, id, at, members);
            self.busy.remove(&at);
            return found;
        }
        self.damaged.insert(id);
        Ok(None)
    }

    fn top_at(
        &mut self,
        view: View,
        id: ObjectId,
        at: usize,
        members: bool,
    ) -> Result<Option<Rc<Object>>, OpenError> {
        if let Some(r) = self.length_refs.get(&at).copied() {
            let length = match r {
                Some(r) => self.length_of(view, r, members)?,
                None => None,
            };
            if let Some(found) = self.tops.get(&(at, length)) {
                return Ok(found.clone());
            }
        }
        let head = match head_at(self.bytes, at, &mut self.budget) {
            Ok(h) => h,
            Err(f) => return self.fail(f, id),
        };
        let r = length_ref(&head);
        self.length_refs.insert(at, r);
        let length = match r {
            Some(r) => self.length_of(view, r, members)?,
            None => None,
        };
        let found = self.finish_top(id, head, length)?;
        self.tops.insert((at, length), found.clone());
        Ok(found)
    }

    fn finish_top(
        &mut self,
        id: ObjectId,
        head: super::object::Head,
        length: Option<i64>,
    ) -> Result<Option<Rc<Object>>, OpenError> {
        let mut obj = match finish(self.bytes, head, length, &mut self.budget) {
            Ok((_, o)) => o,
            Err(f) => return self.fail(f, id),
        };
        if let Some(crypt) = self.crypt
            && id != crypt.skip
            && crypt.decrypt(id, &mut obj).is_err()
        {
            self.damaged.insert(id);
            return Ok(None);
        }
        Ok(Some(Rc::new(obj)))
    }

    fn length_of(
        &mut self,
        view: View,
        r: ObjectId,
        members: bool,
    ) -> Result<Option<i64>, OpenError> {
        let found = match view(r.0) {
            Some(Entry::Normal { pos, generation }) if generation == r.1 => {
                self.top(view, r, pos, false)?
            }
            Some(Entry::Compressed { container, index }) if members && r.1 == 0 => {
                self.member(view, r.0, container, index)?
            }
            _ => None,
        };
        Ok(found.and_then(|o| o.as_i64().ok()))
    }

    fn member(
        &mut self,
        view: View,
        n: u32,
        container: u32,
        index: u32,
    ) -> Result<Option<Rc<Object>>, OpenError> {
        let Some(Entry::Normal { pos, generation }) = view(container) else {
            return Ok(None);
        };
        let Some(packed) = self.unpack(view, (container, generation), pos)? else {
            return Ok(None);
        };
        if let Some(found) = self.members.get(&(pos, index)) {
            return Ok(found
                .clone()
                .filter(|_| packed_num(&packed, index) == Some(n)));
        }
        let Some(&(num, at)) = packed.members.get(index as usize) else {
            return Ok(None);
        };
        let found = if at >= packed.data.len() {
            self.damaged.insert((n, 0));
            None
        } else {
            let after = |p: usize| {
                let i = packed.starts.partition_point(|&s| s <= p);
                packed.starts.get(i).copied()
            };
            let end = after(at).unwrap_or(packed.data.len());
            let limit = Limit {
                end,
                member: true,
                next: Some(after(end).unwrap_or(packed.data.len())),
            };
            match bounded(&packed.data, at, limit, &mut self.budget, |l, b| {
                l.value(b, 0)
            }) {
                Ok((v, _)) => Some(Rc::new(v)),
                Err(f) => self.fail(f, (n, 0))?,
            }
        };
        self.members.insert((pos, index), found.clone());
        Ok(found.filter(|_| num == n))
    }

    /// Unpacks the object stream at `pos` once.
    fn unpack(
        &mut self,
        view: View,
        id: ObjectId,
        pos: usize,
    ) -> Result<Option<Rc<Packed>>, OpenError> {
        if let Some(p) = self.packed.get(&pos) {
            return Ok(p.clone());
        }
        // Marked first, so a container that needs itself is not unpacked again.
        self.packed.insert(pos, None);
        // A container that is not there was counted by `top`.
        let packed = match self.top(view, id, pos, false)? {
            Some(o) => match o.as_ref() {
                Object::Stream(s) if s.dict.has_type(b"ObjStm") => self.members_of(s)?,
                _ => None,
            }
            .or_else(|| {
                self.damaged.insert(id);
                None
            }),
            None => None,
        };
        let packed = packed.map(Rc::new);
        self.packed.insert(pos, packed.clone());
        Ok(packed)
    }

    fn members_of(&mut self, s: &lopdf::Stream) -> Result<Option<Packed>, OpenError> {
        let Some(data) = unpack_stream(s, &mut self.budget)? else {
            return Ok(None);
        };
        let data = unpredict(data, last_parms(&s.dict));
        let int = |k: &[u8]| s.dict.get(k).ok().and_then(|o| o.as_i64().ok());
        let count = int(b"N").unwrap_or(0).max(0) as u64;
        if count > self.budget.limits.max_items as u64 {
            return Err(OpenError::Limit(format!(
                "an object stream lists more than {} members",
                self.budget.limits.max_items
            )));
        }
        let first = int(b"First").unwrap_or(0).max(0) as usize;
        let mut members = Vec::new();
        let mut l = Lexer::new(&data, 0);
        for _ in 0..count {
            let (Some(m), Some(off)) = (l.uint(), l.uint()) else {
                break;
            };
            self.budget.value(0).map_err(Fail::into_open)?;
            let m = u32::try_from(m).unwrap_or(0);
            let at = usize::try_from(off).map_or(usize::MAX, |o| first.saturating_add(o));
            members.push((m, at));
        }
        let mut starts: Vec<usize> = members.iter().map(|&(_, at)| at).collect();
        starts.sort_unstable();
        starts.dedup();
        Ok(Some(Packed {
            data,
            members,
            starts,
        }))
    }
}

fn packed_num(p: &Packed, index: u32) -> Option<u32> {
    p.members.get(index as usize).map(|&(n, _)| n)
}
