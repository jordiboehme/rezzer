// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Revisions: each save of the file, with what it changed in words.
//!
//! The cross-reference chain gives the boundaries: every section along `startxref` and
//! `/Prev` is one save, and the `%%EOF` after it gives its end. The first section of a
//! linearized file only lists the objects of the first page, so it belongs to the save of the
//! section its `/Prev` names. Sections are read from the original bytes with the crate's own
//! parser. Entries are compared section by section, and only the objects a section lists are
//! parsed, for the newest `max_revisions` saves.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use lopdf::{Dictionary, Object, ObjectId};

use crate::objects::{TextBudget, Work, pdf_date, text_string};
use crate::parse::history::{Entry, Loader, Part, Sections, Step, View, last_startxref};
use crate::parse::{Budget, header_base};
use crate::{Issue, IssueKind, Limits, Pdf, PdfDate};

/// The saves of a file, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Revisions {
    /// The newest saves, at most `max_revisions`, oldest first.
    pub list: Vec<Revision>,
    /// Older saves that are counted but not compared.
    pub skipped: usize,
    pub issues: Vec<Issue>,
}

/// One save of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// 1 is the first save. When an issue says the cross-reference chain is cut, the numbers
    /// count from the oldest save that was read.
    pub number: u32,
    /// The byte offset after its `%%EOF` and the line end that follows.
    pub end: u64,
    /// The `/ModDate` of the Info dictionary as of this save.
    pub modified: Option<PdfDate>,
    /// What this save changed, in words. Empty for the first save.
    pub changes: Vec<String>,
}

const LOCATION: &str = "revisions";

/// What one entry of a section takes in memory while the saves are compared, on top of what
/// the parser charges for it: a slot in the map of its save and one in the running table.
const ENTRY_MEMORY: u64 = 128;

/// The words of an issue that cuts the chain.
const RELATIVE: &str = "revision numbers count from the oldest save read";

/// How many characters of a field value the words keep.
const VALUE_CHARS: usize = 80;

/// An entry of a section: in use, or free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Used(Entry),
    Free,
}

/// The entries one save lists, and its trailer.
struct Save {
    start: usize,
    end: usize,
    entries: HashMap<u32, Slot>,
    info: Option<Object>,
    root: Option<Object>,
}

impl Save {
    fn from(p: Part) -> Save {
        let mut entries: HashMap<u32, Slot> = p.freed.iter().map(|&n| (n, Slot::Free)).collect();
        entries.extend(p.table.into_iter().map(|(n, e)| (n, Slot::Used(e))));
        Save {
            start: p.start,
            end: p.end,
            entries,
            info: p.trailer.get(b"Info").ok().cloned(),
            root: p.trailer.get(b"Root").ok().cloned(),
        }
    }

    /// Takes the entries and trailer keys of the newer section `s` of the same save.
    fn absorb(&mut self, s: Save) {
        self.entries.extend(s.entries);
        self.info = s.info.or(self.info.take());
        self.root = s.root.or(self.root.take());
    }
}

/// Why the reading stopped early: the issue that says so.
struct Halt(Issue);

impl Pdf {
    /// The saves of the file, with what each one changed in words. `stop` is asked between
    /// saves. Objects are parsed from [`Pdf::original`] with the crate's own parser, under what
    /// is left of the object budget. The result is read once and kept: the first call's
    /// `stop` decides, and a later call returns the same result and costs nothing.
    pub fn revisions(&self, stop: &dyn Fn() -> bool) -> &Revisions {
        self.revisions.get_or_init(|| self.read_revisions(stop))
    }

    fn read_revisions(&self, stop: &dyn Fn() -> bool) -> Revisions {
        let bytes = self.original();
        let mut out = Revisions::default();
        let Some(base) = header_base(bytes) else {
            return out;
        };
        let limits = Limits {
            max_object_bytes: self.text_left.load(Ordering::SeqCst),
            ..self.limits
        };
        let budget = Budget::new(&limits, stop, bytes.len());
        let mut run = Run {
            loader: Loader::new(bytes, base, budget, self.crypt.as_ref()),
            work: Work::new(&self.limits),
            text: self.text_budget(),
            limits: self.limits,
            stop,
            issues: Vec::new(),
        };
        run.all(bytes, &mut out);
        if run.loader.damaged > 0 {
            let n = run.loader.damaged;
            run.issues.push(Issue::new(
                LOCATION,
                IssueKind::Damaged,
                format!(
                    "{n} {} that the saves list could not be read at their offsets",
                    if n == 1 { "object" } else { "objects" }
                ),
            ));
        }
        // What the parser charged stays charged: freed buffers may stay resident.
        let charged = run.loader.budget.bytes;
        run.text.take(charged);
        let mut seen = std::collections::HashSet::new();
        out.issues = run.issues;
        out.issues.retain(|i| seen.insert(i.clone()));
        out
    }
}

/// One reading of the revisions.
struct Run<'p, 'a> {
    loader: Loader<'a>,
    work: Work,
    text: TextBudget<'p>,
    limits: Limits,
    stop: &'a dyn Fn() -> bool,
    issues: Vec<Issue>,
}

/// What the walk along the chain finds.
#[derive(Default)]
struct Chain {
    /// The newest saves, newest first.
    saves: Vec<Save>,
    /// The table as of the newest save older than `saves`.
    base: HashMap<u32, Slot>,
    base_info: Option<Object>,
    base_root: Option<Object>,
    /// Saves found in all.
    total: usize,
    /// Where every section read starts, sorted.
    starts: Vec<usize>,
}

impl Run<'_, '_> {
    fn limit(&mut self, e: crate::OpenError) -> Halt {
        Halt(Issue::new(
            LOCATION,
            IssueKind::Limit,
            format!("{e}; the rest of the revisions is not compared"),
        ))
    }

    fn visit(&mut self, n: usize) -> Result<(), Halt> {
        if self.work.take(n) {
            return Ok(());
        }
        Err(Halt(self.work.issue(LOCATION, "revisions")))
    }

    fn all(&mut self, bytes: &[u8], out: &mut Revisions) {
        let mut chain = Chain::default();
        if let Err(Halt(issue)) = self.walk(bytes, &mut chain) {
            self.issues.push(issue);
        }
        let max = self.limits.max_revisions;
        while chain.saves.len() > max {
            let s = chain.saves.pop().expect("a save");
            for (n, slot) in s.entries {
                chain.base.insert(n, slot);
            }
            chain.base_info = s.info.or(chain.base_info.take());
            chain.base_root = s.root.or(chain.base_root.take());
        }
        out.skipped = chain.total - chain.saves.len();
        if out.skipped > 0 {
            self.issues.push(Issue::new(
                LOCATION,
                IssueKind::Limit,
                format!(
                    "the file has {} revisions; only the newest {} are compared",
                    chain.total,
                    chain.saves.len()
                ),
            ));
        }
        chain.starts.sort_unstable();
        let mut state = std::mem::take(&mut chain.base);
        let mut info = chain.base_info.take();
        let mut root = chain.base_root.take();
        let mut pages: Option<Rc<Pages>> = None;
        let mut halted = false;
        let first_number = out.skipped + 1;
        for (k, save) in chain.saves.into_iter().rev().enumerate() {
            let number = (first_number + k) as u32;
            let end = self.end_of(bytes, &save, &chain.starts, number);
            let mut rev = Revision {
                number,
                end,
                modified: None,
                changes: Vec::new(),
            };
            if !halted && (self.stop)() {
                halted = true;
                self.issues.push(Issue::new(
                    LOCATION,
                    IssueKind::Limit,
                    format!(
                        "stopped by the caller; the changes of revision {number} and later are not compared"
                    ),
                ));
            }
            let info_after = save.info.clone().or(info.clone());
            let root_after = save.root.clone().or(root.clone());
            if !halted {
                let ctx = Diff {
                    state: &state,
                    save: &save,
                    info: (info.as_ref(), info_after.as_ref()),
                    root: (root.as_ref(), root_after.as_ref()),
                };
                let found = if number == 1 {
                    Ok((Vec::new(), None))
                } else {
                    self.diff(&ctx, pages.take())
                };
                match found.and_then(|(changes, after)| {
                    let modified = self.modified(&ctx, info_after.as_ref())?;
                    Ok((changes, after, modified))
                }) {
                    Ok((changes, after, modified)) => {
                        rev.changes = changes;
                        rev.modified = modified;
                        pages = after;
                    }
                    Err(Halt(issue)) => {
                        halted = true;
                        self.issues.push(issue);
                    }
                }
            }
            for (n, slot) in save.entries {
                state.insert(n, slot);
            }
            info = info_after;
            root = root_after;
            out.list.push(rev);
        }
    }

    /// Reads the chain from the last `startxref`, newest section first.
    fn walk(&mut self, bytes: &[u8], chain: &mut Chain) -> Result<(), Halt> {
        let Some(mut off) = last_startxref(bytes) else {
            return Err(Halt(Issue::new(
                LOCATION,
                IssueKind::Damaged,
                "the file has no startxref; its revisions are not read",
            )));
        };
        let keep = self.limits.max_revisions.saturating_add(1);
        let mut sections = Sections::default();
        // The starts of the last two sections, and the save they make up.
        let mut tail: [Option<usize>; 2] = [None, None];
        let mut found = 0usize;
        let mut older: Option<Save> = None;
        let mut positions = Vec::new();
        let result = loop {
            if found.is_multiple_of(1024) && (self.stop)() {
                break Err(Halt(Issue::new(
                    LOCATION,
                    IssueKind::Limit,
                    format!("stopped by the caller; older revisions are not read; {RELATIVE}"),
                )));
            }
            if let Err(h) = self.visit(1) {
                break Err(h);
            }
            let step = match self.loader.section(&mut sections, off) {
                Ok(s) => s,
                Err(e) => break Err(self.limit(e)),
            };
            let part = match step {
                Step::Part(p) => p,
                Step::Seen => {
                    break Err(Halt(Issue::new(
                        LOCATION,
                        IssueKind::Damaged,
                        format!(
                            "the /Prev chain leads back to a section read already; it ends there; {RELATIVE}"
                        ),
                    )));
                }
                Step::Damaged => {
                    break Err(Halt(Issue::new(
                        LOCATION,
                        IssueKind::Damaged,
                        format!(
                            "the cross-reference section at offset {off} is damaged; older revisions are not read; {RELATIVE}"
                        ),
                    )));
                }
            };
            let entries = part.table.len() + part.freed.len();
            if let Err(h) = self.visit(entries) {
                break Err(h);
            }
            if let Err(e) = self
                .loader
                .budget
                .charge((entries as u64).saturating_mul(ENTRY_MEMORY))
            {
                break Err(self.limit(e.into_open()));
            }
            found += 1;
            chain.starts.push(part.start);
            positions.push(part.start);
            positions.extend(part.table.values().filter_map(|e| match e {
                Entry::Normal { pos, .. } => Some(*pos),
                Entry::Compressed { .. } => None,
            }));
            tail = [tail[1], Some(part.start)];
            let prev = part.prev;
            let save = Save::from(part);
            if chain.saves.len() < keep {
                chain.saves.push(save);
            } else {
                // Older saves only make up the table the compared saves start from.
                if let Some(o) = older.take() {
                    merge_older(chain, o);
                }
                older = Some(save);
            }
            match prev {
                Some(p) => off = p,
                None => break Ok(()),
            }
        };
        // The first section of a linearized file comes before the main section and names it
        // as /Prev: the two are one save.
        let pair = found >= 2
            && matches!(tail, [Some(a), Some(b)] if a < b)
            && result.is_ok()
            && self.loader.linearized();
        match older.take() {
            Some(last) if pair => {
                // The pair is the last kept save and `last`, or both are older.
                if found == keep + 1 {
                    let first = chain.saves.pop().expect("a save");
                    let mut main = last;
                    main.absorb(first);
                    older = Some(main);
                } else {
                    older = Some(last);
                }
            }
            Some(last) => older = Some(last),
            None if pair => {
                let mut main = chain.saves.pop().expect("a save");
                let first = chain.saves.pop().expect("a save");
                main.absorb(first);
                chain.saves.push(main);
            }
            None => {}
        }
        if let Some(o) = older {
            merge_older(chain, o);
        }
        chain.total = found - usize::from(pair);
        self.loader.add_starts(positions);
        result
    }

    /// Where a save ends: after the first `%%EOF` (and its line end) between the end of its
    /// section and the start of the next section in the file.
    fn end_of(&mut self, bytes: &[u8], save: &Save, starts: &[usize], number: u32) -> u64 {
        let next = starts
            .get(starts.partition_point(|&s| s <= save.start))
            .copied()
            .unwrap_or(bytes.len())
            .min(bytes.len());
        let from = save.end.min(next);
        let Some(at) = bytes[from..next].windows(5).position(|w| w == b"%%EOF") else {
            self.issues.push(Issue::new(
                LOCATION,
                IssueKind::Damaged,
                format!("revision {number} has no %%EOF after its cross-reference section"),
            ));
            return save.end as u64;
        };
        let at = from + at + 5;
        let eol = match &bytes[at..] {
            [b'\r', b'\n', ..] => 2,
            [b'\r' | b'\n', ..] => 1,
            _ => 0,
        };
        (at + eol) as u64
    }

    /// The `/ModDate` of the Info dictionary as of a save.
    fn modified(&mut self, d: &Diff, info: Option<&Object>) -> Result<Option<PdfDate>, Halt> {
        let after = d.after();
        let info = self.resolve(&after, info)?;
        let Some(dict) = info.as_deref().and_then(as_dict) else {
            return Ok(None);
        };
        let raw = match self.resolve(&after, dict.get(b"ModDate").ok())?.as_deref() {
            Some(Object::String(s, _)) => text_string(s),
            _ => return Ok(None),
        };
        self.copy(raw.len())?;
        Ok(Some(PdfDate {
            iso: pdf_date(&raw),
            raw,
        }))
    }

    /// Charges copied text.
    fn copy(&mut self, n: usize) -> Result<(), Halt> {
        if self.text.charge(n) {
            return Ok(());
        }
        Err(Halt(self.text.limit_issue(LOCATION, "revisions")))
    }

    /// Follows a reference in a revision; a direct value is copied. Only for small values.
    fn resolve(&mut self, view: View, obj: Option<&Object>) -> Result<Option<Rc<Object>>, Halt> {
        let Some(obj) = obj else {
            return Ok(None);
        };
        match obj {
            Object::Reference(id) => self.load(view, *id),
            o => Ok(Some(Rc::new(o.clone()))),
        }
    }

    fn load(&mut self, view: View, id: ObjectId) -> Result<Option<Rc<Object>>, Halt> {
        self.visit(1)?;
        match self.loader.load(view, id) {
            Ok(o) => Ok(o),
            Err(e) => Err(self.limit(e)),
        }
    }

    /// The object a reference names, in a revision. `None` for a direct value.
    fn deref(&mut self, view: View, obj: Option<&Object>) -> Result<Option<Rc<Object>>, Halt> {
        match obj {
            Some(Object::Reference(id)) => self.load(view, *id),
            _ => Ok(None),
        }
    }
}

/// The dictionary of a dictionary or a stream.
fn as_dict(o: &Object) -> Option<&Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// Adds an older save to the base table: entries already there are newer and stay.
fn merge_older(chain: &mut Chain, s: Save) {
    for (n, slot) in s.entries {
        chain.base.entry(n).or_insert(slot);
    }
    if chain.base_info.is_none() {
        chain.base_info = s.info;
    }
    if chain.base_root.is_none() {
        chain.base_root = s.root;
    }
}

/// The table before and after one save, and its trailer keys.
struct Diff<'s> {
    state: &'s HashMap<u32, Slot>,
    save: &'s Save,
    info: (Option<&'s Object>, Option<&'s Object>),
    root: (Option<&'s Object>, Option<&'s Object>),
}

impl Diff<'_> {
    fn before(&self) -> impl Fn(u32) -> Option<Entry> + '_ {
        |n| match self.state.get(&n) {
            Some(Slot::Used(e)) => Some(*e),
            _ => None,
        }
    }

    fn after(&self) -> impl Fn(u32) -> Option<Entry> + '_ {
        |n| match self.save.entries.get(&n).or_else(|| self.state.get(&n)) {
            Some(Slot::Used(e)) => Some(*e),
            _ => None,
        }
    }
}

mod words;

use words::Pages;

/// The object id an entry defines.
fn id_of(n: u32, e: Entry) -> ObjectId {
    match e {
        Entry::Normal { generation, .. } => (n, generation),
        Entry::Compressed { .. } => (n, 0),
    }
}

/// A field value as text, cut to [`VALUE_CHARS`].
fn value_text(v: Option<&Object>) -> String {
    let text = match v {
        Some(Object::String(s, _)) => text_string(s),
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        Some(Object::Integer(i)) => i.to_string(),
        Some(Object::Real(r)) => r.to_string(),
        Some(Object::Boolean(b)) => b.to_string(),
        Some(Object::Array(a)) => a
            .iter()
            .take(VALUE_CHARS)
            .map(|o| value_text(Some(o)))
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    if text.is_empty() {
        return "nothing".to_string();
    }
    if text.chars().count() > VALUE_CHARS {
        let cut: String = text.chars().take(VALUE_CHARS).collect();
        return format!("{cut}...");
    }
    text
}

/// The `/Type` of an object, else what kind of value it is.
fn type_word(o: &Object) -> String {
    if let Some(Ok(Object::Name(t))) = as_dict(o).map(|d| d.get(b"Type")) {
        return String::from_utf8_lossy(t).into_owned();
    }
    match o {
        Object::Stream(_) => "stream",
        Object::Dictionary(_) => "dictionary",
        Object::Array(_) => "array",
        Object::String(..) => "string",
        Object::Integer(_) | Object::Real(_) => "number",
        Object::Boolean(_) => "boolean",
        Object::Name(_) => "name",
        Object::Reference(_) => "reference",
        Object::Null => "null",
    }
    .to_string()
}

/// Objects that only hold the file together: they change with every save.
fn structural(o: &Object) -> bool {
    let Some(d) = as_dict(o) else {
        return false;
    };
    d.get(b"Linearized").is_ok()
        || matches!(d.get(b"Type"), Ok(Object::Name(t)) if t == b"XRef" || t == b"ObjStm")
}
