// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Interactive forms (AcroForm): the fields with their type, value, flags, options and
//! scripts, the calculation order, and whether the file also holds an XFA form.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Object, ObjectId};

use crate::actions::{Chains, event_word};
use crate::objects::{
    TextBudget, Work, dict, get, get_dict, get_text, reference, resolve, text_string,
};
use crate::{Issue, IssueKind, Pdf};

/// The interactive form of a document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Form {
    /// The terminal fields in tree order.
    pub fields: Vec<Field>,
    /// Full names of the fields in `/CO`, the order in which calculations run.
    pub calc_order: Vec<String>,
    /// The file also holds an XFA form. Its data is not read.
    pub xfa: bool,
    /// `/NeedAppearances`: a viewer should draw the fields anew.
    pub need_appearances: bool,
}

/// One terminal field: a field without child fields. `/FT`, `/Ff`, `/V` and `/Opt` are
/// taken from the nearest parent that has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The partial names (`/T`) from the root down, joined by dots.
    pub full_name: String,
    /// `/TU`, the name shown to the user.
    pub alt_name: Option<String>,
    pub kind: FieldKind,
    /// `/V` as text: a string, a name such as `Yes` or `Off`, or the items of a list joined
    /// by `, `. `None` when it is not set or holds a signature.
    pub value: Option<String>,
    /// `/Ff` as written.
    pub flags: u32,
    /// The flags in words, for example `read-only`, `required`, `multiline`.
    pub flag_words: Vec<&'static str>,
    /// `/Opt` of a choice field: the shown text, followed by the export value in brackets
    /// when it differs.
    pub options: Vec<String>,
    /// JavaScript of the field and its widgets: the event (`format`, `keystroke`,
    /// `validate`, `calculate`, `activate`, `mouse-up` and so on) and the script text.
    pub actions: Vec<(String, String)>,
    /// The pages its widgets lie on, each once, in order.
    pub pages: Vec<u32>,
}

/// The type of a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Checkbox,
    Radio,
    PushButton,
    Combo,
    List,
    Signature,
    /// `/FT` as written, or empty when there is none.
    Unknown(String),
}

/// Flag bits (1-based, ISO 32000 12.7.4) and their words, for every field.
const COMMON_FLAGS: [(u32, &str); 3] = [(1, "read-only"), (2, "required"), (3, "no-export")];
const TEXT_FLAGS: [(u32, &str); 7] = [
    (13, "multiline"),
    (14, "password"),
    (21, "file-select"),
    (23, "do-not-spell-check"),
    (24, "do-not-scroll"),
    (25, "comb"),
    (26, "rich-text"),
];
const BUTTON_FLAGS: [(u32, &str); 2] = [(15, "no-toggle-to-off"), (26, "radios-in-unison")];
const CHOICE_FLAGS: [(u32, &str); 5] = [
    (19, "edit"),
    (20, "sort"),
    (22, "multi-select"),
    (23, "do-not-spell-check"),
    (27, "commit-on-select-change"),
];

const FORM: &str = "form";

fn bit(flags: u32, n: u32) -> bool {
    flags & (1 << (n - 1)) != 0
}

fn kind_of(ft: Option<&[u8]>, flags: u32) -> FieldKind {
    match ft {
        Some(b"Tx") => FieldKind::Text,
        Some(b"Btn") if bit(flags, 17) => FieldKind::PushButton,
        Some(b"Btn") if bit(flags, 16) => FieldKind::Radio,
        Some(b"Btn") => FieldKind::Checkbox,
        Some(b"Ch") if bit(flags, 18) => FieldKind::Combo,
        Some(b"Ch") => FieldKind::List,
        Some(b"Sig") => FieldKind::Signature,
        Some(other) => FieldKind::Unknown(String::from_utf8_lossy(other).into_owned()),
        None => FieldKind::Unknown(String::new()),
    }
}

fn flag_words(kind: &FieldKind, flags: u32) -> Vec<&'static str> {
    let own: &[(u32, &str)] = match kind {
        FieldKind::Text => &TEXT_FLAGS,
        FieldKind::Checkbox | FieldKind::Radio | FieldKind::PushButton => &BUTTON_FLAGS,
        FieldKind::Combo | FieldKind::List => &CHOICE_FLAGS,
        _ => &[],
    };
    COMMON_FLAGS
        .iter()
        .chain(own)
        .filter(|(n, _)| bit(flags, *n))
        .map(|(_, w)| *w)
        .collect()
}

/// What a field takes from its parents.
#[derive(Clone, Copy, Default)]
struct Inherited<'a> {
    ft: Option<&'a [u8]>,
    ff: Option<u32>,
    v: Option<&'a Object>,
    /// The field node that holds `v`.
    v_holder: Option<ObjectId>,
    opt: Option<&'a Object>,
}

/// One action of a field as the form read it, for [`Pdf::scripts`].
#[derive(Debug)]
pub(crate) struct FieldAction {
    /// Index into [`Form::fields`].
    pub field: usize,
    pub event: String,
    pub kind: String,
    pub target: Option<String>,
    /// Index into the field's `actions` for a JavaScript action.
    pub js: Option<usize>,
}

/// Where the signature dictionary of a signature field is.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SigValue {
    /// `/V` is this object.
    Object(ObjectId),
    /// `/V` is written straight into this field node.
    In(ObjectId),
}

/// The form read once, with what the scripts and signature readers need from it.
#[derive(Debug, Default)]
pub(crate) struct FormData {
    pub form: Option<Form>,
    pub actions: Vec<FieldAction>,
    /// Signature fields that hold a value: index into [`Form::fields`] and the value.
    pub sigs: Vec<(usize, SigValue)>,
    /// Every widget that belongs to a field.
    pub widgets: HashSet<ObjectId>,
}

/// Why the walk ends early.
struct Stop;

/// One walk of the field tree.
struct Build<'a> {
    pdf: &'a Pdf,
    budget: TextBudget<'a>,
    work: Work,
    chains: Chains,
    /// Widget pages from the pages' `/Annots`.
    pages: HashMap<ObjectId, u32>,
    /// Field nodes and widgets met so far.
    seen: HashSet<ObjectId>,
    repeats: usize,
    first_repeat: Option<ObjectId>,
    /// `/Kids` array objects read so far, and the fields that named one again.
    kids_arrays: HashSet<ObjectId>,
    shared_kids: usize,
    first_shared_kids: Option<ObjectId>,
    /// The fields `/CO` names, and their full names once met.
    co: HashSet<ObjectId>,
    co_names: HashMap<ObjectId, String>,
    data: FormData,
    fields: Vec<Field>,
    issues: Vec<Issue>,
}

impl Pdf {
    /// The interactive form, or `None` when the file has none. Read on the first call and
    /// kept. A field reached twice (a cycle or a shared field) and a `/Kids` array that
    /// several fields name are Damaged issues; the rest is still read.
    pub fn form(&self) -> Option<&Form> {
        self.form_data().form.as_ref()
    }

    pub(crate) fn form_data(&self) -> &FormData {
        self.form.get_or_init(|| self.read_form())
    }

    fn acroform(&self) -> Option<&Dictionary> {
        get_dict(&self.doc, self.catalog()?, b"AcroForm")
    }

    fn read_form(&self) -> FormData {
        let doc = &self.doc;
        let Some(acro) = self.acroform() else {
            return FormData::default();
        };
        let mut b = Build {
            pdf: self,
            budget: self.text_budget(),
            work: Work::new(&self.limits),
            chains: Chains::new(&self.limits, FORM),
            pages: self.widget_pages(),
            seen: HashSet::new(),
            repeats: 0,
            first_repeat: None,
            kids_arrays: HashSet::new(),
            shared_kids: 0,
            first_shared_kids: None,
            co: HashSet::new(),
            co_names: HashMap::new(),
            data: FormData::default(),
            fields: Vec::new(),
            issues: Vec::new(),
        };
        let co: &[Object] = match get(doc, acro, b"CO") {
            Some(Object::Array(co)) => co,
            _ => &[],
        };
        let mut read = b.count(co.len());
        if read.is_ok() {
            b.co = co.iter().filter_map(reference).collect();
        }
        if let (Ok(()), Some(Object::Array(fields))) = (&read, get(doc, acro, b"Fields")) {
            read = b.count(fields.len());
            if read.is_ok() {
                for f in fields {
                    read = b.node(f, "", Inherited::default(), 1);
                    if read.is_err() {
                        break;
                    }
                }
            }
        }
        let mut calc_order = Vec::new();
        for id in co.iter().filter_map(reference) {
            if let Some(name) = b.co_names.get(&id) {
                calc_order.push(name.clone());
            }
        }
        b.finish();
        let Build {
            mut data,
            fields,
            issues,
            ..
        } = b;
        self.note(issues);
        data.form = Some(Form {
            fields,
            calc_order,
            xfa: acro.has(b"XFA"),
            need_appearances: matches!(
                get(doc, acro, b"NeedAppearances"),
                Some(Object::Boolean(true))
            ),
        });
        data
    }

    /// The page of each widget annotation, from the pages' `/Annots`.
    fn widget_pages(&self) -> HashMap<ObjectId, u32> {
        let mut pages = HashMap::new();
        let mut issues = Vec::new();
        self.each_annot("form fields", &mut issues, |page, _, a| {
            if let Some(id) = reference(a) {
                pages.entry(id).or_insert(page);
            }
            Ok(())
        });
        self.note(issues);
        pages
    }
}

impl<'a> Build<'a> {
    /// Counts `n` visited entries; past the bound, one Limit issue and the walk ends.
    fn count(&mut self, n: usize) -> Result<(), Stop> {
        if self.work.take(n) {
            return Ok(());
        }
        self.issues
            .extend(self.work.limit_issue(FORM, "form fields"));
        Err(Stop)
    }

    /// Charges a copy; when the budget is spent, one Limit issue and the walk ends.
    fn charge(&mut self, bytes: usize) -> Result<(), Stop> {
        if self.budget.charge(bytes) {
            return Ok(());
        }
        self.issues
            .push(self.budget.limit_issue(FORM, "form fields"));
        Err(Stop)
    }

    /// The grouped issues of the walk.
    fn finish(&mut self) {
        if let Some(id) = self.first_repeat {
            self.issues.push(Issue::new(
                FORM,
                IssueKind::Damaged,
                format!(
                    "{} {} reached twice: the field tree has a cycle or a shared field or widget (the first is object {} {})",
                    self.repeats,
                    if self.repeats == 1 { "node is" } else { "nodes are" },
                    id.0,
                    id.1
                ),
            ));
        }
        if let Some(id) = self.first_shared_kids {
            self.issues.push(Issue::new(
                FORM,
                IssueKind::Damaged,
                format!(
                    "{} {} a /Kids array that another field names; it is read for the first only (the first is object {} {})",
                    self.shared_kids,
                    if self.shared_kids == 1 { "field names" } else { "fields name" },
                    id.0,
                    id.1
                ),
            ));
        }
        self.chains.finish(&mut self.issues);
    }

    fn node(
        &mut self,
        obj: &'a Object,
        parent: &str,
        inherited: Inherited<'a>,
        depth: usize,
    ) -> Result<(), Stop> {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let limits = &pdf.limits;
        if depth > limits.max_depth {
            self.issues.push(Issue::new(
                FORM,
                IssueKind::Limit,
                format!("the field tree is deeper than {} levels", limits.max_depth),
            ));
            return Ok(());
        }
        let id = reference(obj);
        if let Some(id) = id
            && !self.seen.insert(id)
        {
            self.repeats += 1;
            self.first_repeat.get_or_insert(id);
            return Ok(());
        }
        let Some(d) = dict(doc, obj) else {
            return Ok(());
        };
        let full_name = match get_text(doc, d, b"T") {
            Some(t) if parent.is_empty() => t,
            Some(t) => format!("{parent}.{t}"),
            None => parent.to_string(),
        };
        self.charge(full_name.len())?;
        if let Some(id) = id
            && self.co.contains(&id)
        {
            self.charge(full_name.len())?;
            self.co_names.insert(id, full_name.clone());
        }
        let here = Inherited {
            ft: match get(doc, d, b"FT") {
                Some(Object::Name(n)) => Some(n.as_slice()),
                _ => inherited.ft,
            },
            ff: match get(doc, d, b"Ff") {
                Some(Object::Integer(i)) => Some(*i as u32),
                _ => inherited.ff,
            },
            v: d.get(b"V").ok().or(inherited.v),
            v_holder: if d.has(b"V") { id } else { inherited.v_holder },
            opt: d.get(b"Opt").ok().or(inherited.opt),
        };
        // Kids with a name, or with kids of their own, are fields; the others are widgets.
        let mut fields = Vec::new();
        let mut widgets = Vec::new();
        // A /Kids that is not a readable array counts as none: the node is its own widget.
        let mut shared = false;
        match d.get(b"Kids").ok().map(|k| (k, resolve(doc, k))) {
            Some((k, Some(Object::Array(kids)))) => {
                if let Some(kid_id) = reference(k)
                    && !self.kids_arrays.insert(kid_id)
                {
                    shared = true;
                    self.shared_kids += 1;
                    self.first_shared_kids.get_or_insert(kid_id);
                } else {
                    self.count(kids.len())?;
                    for kid in kids {
                        let Some(kd) = dict(doc, kid) else {
                            continue;
                        };
                        if kd.has(b"T") || kd.has(b"Kids") {
                            fields.push(kid);
                            continue;
                        }
                        if let Some(w) = reference(kid)
                            && !self.seen.insert(w)
                        {
                            self.repeats += 1;
                            self.first_repeat.get_or_insert(w);
                            continue;
                        }
                        widgets.push((reference(kid), kd));
                    }
                }
            }
            _ => widgets.push((id, d)),
        }
        // A node whose kids another field read first is not listed: the sharing issue says it.
        if fields.is_empty() && !shared {
            self.field(d, full_name.clone(), &here, &widgets)?;
        }
        for kid in fields {
            self.node(kid, &full_name, here, depth + 1)?;
        }
        Ok(())
    }

    /// Reads one terminal field and its actions.
    fn field(
        &mut self,
        d: &'a Dictionary,
        full_name: String,
        here: &Inherited<'a>,
        widgets: &[(Option<ObjectId>, &'a Dictionary)],
    ) -> Result<(), Stop> {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let ff = here.ff.unwrap_or(0);
        let kind = kind_of(here.ft, ff);
        let value = match here.v.and_then(|v| resolve(doc, v)) {
            Some(Object::String(s, _)) => Some(text_string(s)),
            Some(Object::Name(n)) => Some(String::from_utf8_lossy(n).into_owned()),
            Some(Object::Integer(i)) => Some(i.to_string()),
            Some(Object::Real(r)) => Some(r.to_string()),
            Some(Object::Array(items)) => {
                self.count(items.len())?;
                let parts: Vec<String> = items
                    .iter()
                    .filter_map(|o| match resolve(doc, o)? {
                        Object::String(s, _) => Some(text_string(s)),
                        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
                        _ => None,
                    })
                    .collect();
                Some(parts.join(", "))
            }
            _ => None,
        };
        let alt_name = get_text(doc, d, b"TU");
        self.charge(
            value.as_ref().map_or(0, String::len) + alt_name.as_ref().map_or(0, String::len),
        )?;
        let mut options = Vec::new();
        if let Some(Object::Array(opt)) = here.opt.and_then(|o| resolve(doc, o)) {
            self.count(opt.len())?;
            for o in opt {
                let text = |o: &Object| match resolve(doc, o)? {
                    Object::String(s, _) => Some(text_string(s)),
                    _ => None,
                };
                let shown = match resolve(doc, o) {
                    Some(Object::Array(pair)) => match pair.as_slice() {
                        [export, display] => match (text(export), text(display)) {
                            (Some(e), Some(d)) if e == d => Some(d),
                            (Some(e), Some(d)) => Some(format!("{d} ({e})")),
                            (_, d) => d,
                        },
                        _ => None,
                    },
                    _ => text(o),
                };
                if let Some(s) = shown {
                    self.charge(s.len())?;
                    options.push(s);
                }
            }
        }
        let index = self.fields.len();
        let actions = self.actions(d, index, widgets)?;
        let mut on = Vec::new();
        for (id, w) in widgets {
            if let Some(id) = id {
                self.data.widgets.insert(*id);
            }
            let page = id.and_then(|id| self.pages.get(&id).copied()).or_else(|| {
                w.get(b"P")
                    .ok()
                    .and_then(reference)
                    .and_then(|p| pdf.page_number(p))
            });
            if let Some(p) = page
                && !on.contains(&p)
            {
                on.push(p);
            }
        }
        self.charge(on.len() * 4)?;
        if here.ft == Some(b"Sig") {
            let value = match here.v {
                Some(Object::Reference(id)) => Some(SigValue::Object(*id)),
                Some(_) => here.v_holder.map(SigValue::In),
                None => None,
            };
            if let Some(v) = value {
                self.data.sigs.push((index, v));
            }
        }
        self.fields.push(Field {
            full_name,
            alt_name,
            flag_words: flag_words(&kind, ff),
            kind,
            value,
            flags: ff,
            options,
            actions,
            pages: on,
        });
        Ok(())
    }

    /// The actions of a field and its widgets in order: the field's `/AA`, then each
    /// widget's `/A` (event `activate`) and `/AA`. An action object met twice in one field
    /// counts once. Returns the JavaScript of the field; every action is kept for
    /// [`Pdf::scripts`].
    fn actions(
        &mut self,
        d: &'a Dictionary,
        index: usize,
        widgets: &[(Option<ObjectId>, &'a Dictionary)],
    ) -> Result<Vec<(String, String)>, Stop> {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let mut sources: Vec<(Option<&Object>, Option<&Dictionary>)> =
            vec![(None, get_dict(doc, d, b"AA"))];
        for (_, w) in widgets {
            if std::ptr::eq(*w, d) {
                sources.push((w.get(b"A").ok(), None));
            } else {
                sources.push((w.get(b"A").ok(), get_dict(doc, w, b"AA")));
            }
        }
        let mut js = Vec::new();
        let mut seen = HashSet::new();
        for (a, aa) in sources {
            let entries = a.map(|a| (b"A".as_slice(), a)).into_iter().chain(
                aa.into_iter()
                    .flat_map(|aa| aa.iter().map(|(k, v)| (k.as_slice(), v))),
            );
            for (key, action) in entries {
                self.count(1)?;
                if let Some(id) = reference(action)
                    && !seen.insert(id)
                {
                    continue;
                }
                let event = if key == b"A" {
                    "activate".to_string()
                } else {
                    event_word(key)
                };
                let Ok(found) =
                    pdf.action_chain(action, &mut self.budget, &mut self.chains, &mut self.issues)
                else {
                    self.issues
                        .push(self.budget.limit_issue(FORM, "form fields"));
                    return Err(Stop);
                };
                for a in found {
                    self.charge(event.len())?;
                    let slot = a.text.map(|text| {
                        js.push((event.clone(), text));
                        js.len() - 1
                    });
                    self.data.actions.push(FieldAction {
                        field: index,
                        event: event.clone(),
                        kind: a.kind,
                        target: a.target,
                        js: slot,
                    });
                }
            }
        }
        Ok(js)
    }
}
