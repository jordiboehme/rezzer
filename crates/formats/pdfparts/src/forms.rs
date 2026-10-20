// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Interactive forms (AcroForm): the fields with their type, value, flags, options and
//! scripts, the calculation order, and whether the file also holds an XFA form.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Object, ObjectId};

use crate::actions::{Action, Script, Spent, event_word};
use crate::objects::{TextBudget, dict, get, get_dict, get_text, reference, resolve, text_string};
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
    opt: Option<&'a Object>,
}

/// One field node met by the walk.
pub(crate) struct Node<'a> {
    pub id: Option<ObjectId>,
    pub dict: &'a Dictionary,
    pub full_name: String,
    /// It has no child fields.
    pub terminal: bool,
    /// Its widgets: kids without a name, or the node itself when it has no kids.
    pub widgets: Vec<(Option<ObjectId>, &'a Dictionary)>,
    ft: Option<&'a [u8]>,
    ff: u32,
    v: Option<&'a Object>,
    opt: Option<&'a Object>,
}

impl<'a> Node<'a> {
    pub(crate) fn is_signature(&self) -> bool {
        self.ft == Some(b"Sig")
    }

    pub(crate) fn value(&self) -> Option<&'a Object> {
        self.v
    }
}

/// Where one walk of the field tree stands.
struct Walk<'a, 'v> {
    pdf: &'a Pdf,
    seen: HashSet<ObjectId>,
    repeats: usize,
    first_repeat: Option<ObjectId>,
    nodes: usize,
    budget: &'v mut TextBudget<'a>,
    issues: Vec<Issue>,
    /// The walk ends: a limit was hit or `visit` asked to stop.
    done: bool,
}

impl Pdf {
    /// The interactive form, or `None` when the file has none. Read on the first call and
    /// kept. A field reached twice (a cycle or a shared field) is a Damaged issue; the rest
    /// is still read.
    pub fn form(&self) -> Option<&Form> {
        self.form.get_or_init(|| self.read_form()).as_ref()
    }

    pub(crate) fn acroform(&self) -> Option<&Dictionary> {
        get_dict(&self.doc, self.catalog()?, b"AcroForm")
    }

    fn read_form(&self) -> Option<Form> {
        let doc = &self.doc;
        let acro = self.acroform()?;
        let mut form = Form {
            xfa: acro.has(b"XFA"),
            need_appearances: matches!(
                get(doc, acro, b"NeedAppearances"),
                Some(Object::Boolean(true))
            ),
            ..Form::default()
        };
        let pages = self.widget_pages();
        let mut names: HashMap<ObjectId, String> = HashMap::new();
        let mut budget = self.text_budget();
        let mut issues = Vec::new();
        self.walk_fields(&mut budget, &mut issues, |node, budget, issues| {
            if let Some(id) = node.id {
                if !budget.charge(node.full_name.len()) {
                    return Err(budget.limit_issue(FORM, "form fields"));
                }
                names.insert(id, node.full_name.clone());
            }
            if !node.terminal {
                return Ok(());
            }
            let field = self.field(node, &pages, budget, issues);
            match field {
                Some(f) => {
                    form.fields.push(f);
                    Ok(())
                }
                None => Err(budget.limit_issue(FORM, "form fields")),
            }
        });
        if let Some(Object::Array(co)) = get(doc, acro, b"CO") {
            for id in co.iter().filter_map(reference) {
                if let Some(name) = names.get(&id) {
                    if !budget.charge(name.len()) {
                        issues.push(budget.limit_issue(FORM, "form fields"));
                        break;
                    }
                    form.calc_order.push(name.clone());
                }
            }
        }
        self.note(issues);
        Some(form)
    }

    /// One terminal field. `None` when the budget is spent.
    fn field(
        &self,
        node: &Node<'_>,
        pages: &HashMap<ObjectId, u32>,
        budget: &mut TextBudget<'_>,
        issues: &mut Vec<Issue>,
    ) -> Option<Field> {
        let doc = &self.doc;
        let kind = kind_of(node.ft, node.ff);
        let value = node.v.and_then(|v| match resolve(doc, v)? {
            Object::String(s, _) => Some(text_string(s)),
            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
            Object::Integer(i) => Some(i.to_string()),
            Object::Real(r) => Some(r.to_string()),
            Object::Array(items) => Some(
                items
                    .iter()
                    .take(self.limits.max_items)
                    .filter_map(|o| match resolve(doc, o)? {
                        Object::String(s, _) => Some(text_string(s)),
                        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            _ => None,
        });
        let alt_name = get_text(doc, node.dict, b"TU");
        if !budget.charge(
            value.as_ref().map_or(0, String::len) + alt_name.as_ref().map_or(0, String::len),
        ) {
            return None;
        }
        let mut options = Vec::new();
        if let Some(Object::Array(opt)) = node.opt.and_then(|o| resolve(doc, o)) {
            for o in opt.iter().take(self.limits.max_items) {
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
                    if !budget.charge(s.len()) {
                        return None;
                    }
                    options.push(s);
                }
            }
        }
        let mut actions = Vec::new();
        for (event, a) in self.field_actions(node, budget, issues).ok()? {
            if let Some(text) = a.text {
                actions.push((event, text));
            }
        }
        let mut on = Vec::new();
        for (id, w) in &node.widgets {
            let page = id.and_then(|id| pages.get(&id).copied()).or_else(|| {
                w.get(b"P")
                    .ok()
                    .and_then(reference)
                    .and_then(|p| self.page_number(p))
            });
            if let Some(p) = page
                && !on.contains(&p)
            {
                on.push(p);
            }
        }
        if !budget.take(on.len() as u64 * 4) {
            return None;
        }
        Some(Field {
            full_name: node.full_name.clone(),
            alt_name,
            flag_words: flag_words(&kind, node.ff),
            kind,
            value,
            flags: node.ff,
            options,
            actions,
            pages: on,
        })
    }

    /// The actions of a field and its widgets in order: the field's `/AA`, then each
    /// widget's `/A` (event `activate`) and `/AA`. An action object met twice counts once.
    fn field_actions(
        &self,
        node: &Node<'_>,
        budget: &mut TextBudget<'_>,
        issues: &mut Vec<Issue>,
    ) -> Result<Vec<(String, Action)>, Spent> {
        let doc = &self.doc;
        let location = format!("field {}", node.full_name);
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut sources: Vec<(Option<&Object>, Option<&Dictionary>)> =
            vec![(None, get_dict(doc, node.dict, b"AA"))];
        for (_, w) in &node.widgets {
            if std::ptr::eq(*w, node.dict) {
                sources.push((w.get(b"A").ok(), None));
            } else {
                sources.push((w.get(b"A").ok(), get_dict(doc, w, b"AA")));
            }
        }
        for (a, aa) in sources {
            let entries = a.map(|a| (b"A".as_slice(), a)).into_iter().chain(
                aa.into_iter()
                    .flat_map(|aa| aa.iter().map(|(k, v)| (k.as_slice(), v))),
            );
            for (key, action) in entries {
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
                let mut found = Vec::new();
                let read = self.action_chain(action, budget, &mut found, &location, issues);
                out.extend(found.into_iter().map(|a| (event.clone(), a)));
                if read.is_err() {
                    return Err(Spent);
                }
            }
        }
        Ok(out)
    }

    /// The scripts and actions of every terminal field, for [`Pdf::scripts`].
    pub(crate) fn field_scripts<'a>(
        &'a self,
        out: &mut Vec<Script>,
        budget: &mut TextBudget<'a>,
        issues: &mut Vec<Issue>,
    ) -> Result<(), Spent> {
        if self.acroform().is_none() {
            return Ok(());
        }
        let mut spent = false;
        let mut walk_issues = Vec::new();
        self.walk_fields(budget, &mut walk_issues, |node, budget, issues| {
            if !node.terminal {
                return Ok(());
            }
            let Ok(actions) = self.field_actions(node, budget, issues) else {
                spent = true;
                return Err(budget.limit_issue("scripts", "scripts"));
            };
            for (event, a) in actions {
                let s = Script {
                    trigger: format!("field {} {event}", node.full_name),
                    name: None,
                    kind: a.kind,
                    target: a.target,
                    text: a.text,
                };
                if !budget.charge(s.weight()) {
                    spent = true;
                    return Err(budget.limit_issue("scripts", "scripts"));
                }
                out.push(s);
            }
            Ok(())
        });
        issues.append(&mut walk_issues);
        if spent { Err(Spent) } else { Ok(()) }
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

    /// Walks the field tree from `/Fields` in tree order and calls `visit` for every field
    /// node. Full names are charged to `budget`. A node reached twice, depth and the number
    /// of nodes are issues; `visit` ends the walk with an issue.
    pub(crate) fn walk_fields<'a>(
        &'a self,
        budget: &mut TextBudget<'a>,
        issues: &mut Vec<Issue>,
        mut visit: impl FnMut(&Node<'a>, &mut TextBudget<'a>, &mut Vec<Issue>) -> Result<(), Issue>,
    ) {
        let Some(acro) = self.acroform() else {
            return;
        };
        let Some(Object::Array(fields)) = get(&self.doc, acro, b"Fields") else {
            return;
        };
        let mut walk = Walk {
            pdf: self,
            seen: HashSet::new(),
            repeats: 0,
            first_repeat: None,
            nodes: 0,
            budget,
            issues: Vec::new(),
            done: false,
        };
        for f in fields {
            walk.node(f, "", Inherited::default(), 1, &mut visit);
            if walk.done {
                break;
            }
        }
        if let Some(id) = walk.first_repeat {
            walk.issues.push(Issue::new(
                FORM,
                IssueKind::Damaged,
                format!(
                    "{} {} reached twice: the field tree has a cycle or a shared field (the first is object {} {})",
                    walk.repeats,
                    if walk.repeats == 1 { "field is" } else { "fields are" },
                    id.0,
                    id.1
                ),
            ));
        }
        issues.append(&mut walk.issues);
    }
}

impl<'a> Walk<'a, '_> {
    fn node(
        &mut self,
        obj: &'a Object,
        parent: &str,
        inherited: Inherited<'a>,
        depth: usize,
        visit: &mut impl FnMut(&Node<'a>, &mut TextBudget<'a>, &mut Vec<Issue>) -> Result<(), Issue>,
    ) {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let limits = &pdf.limits;
        if self.done {
            return;
        }
        if depth > limits.max_depth {
            self.issues.push(Issue::new(
                FORM,
                IssueKind::Limit,
                format!("the field tree is deeper than {} levels", limits.max_depth),
            ));
            return;
        }
        let id = reference(obj);
        if let Some(id) = id
            && !self.seen.insert(id)
        {
            self.repeats += 1;
            self.first_repeat.get_or_insert(id);
            return;
        }
        let Some(d) = dict(doc, obj) else {
            return;
        };
        self.nodes += 1;
        if self.nodes > limits.max_items {
            self.done = true;
            self.issues.push(Issue::new(
                FORM,
                IssueKind::Limit,
                format!("the form has more than {} field nodes", limits.max_items),
            ));
            return;
        }
        let full_name = match get_text(doc, d, b"T") {
            Some(t) if parent.is_empty() => t,
            Some(t) => format!("{parent}.{t}"),
            None => parent.to_string(),
        };
        if !self.budget.charge(full_name.len()) {
            self.done = true;
            self.issues
                .push(self.budget.limit_issue(FORM, "form fields"));
            return;
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
            opt: d.get(b"Opt").ok().or(inherited.opt),
        };
        // Kids with a name, or with kids of their own, are fields; the others are widgets.
        let mut fields = Vec::new();
        let mut widgets = Vec::new();
        match get(doc, d, b"Kids") {
            Some(Object::Array(kids)) => {
                for kid in kids {
                    let Some(kd) = dict(doc, kid) else {
                        continue;
                    };
                    if kd.has(b"T") || kd.has(b"Kids") {
                        fields.push(kid);
                    } else if reference(kid).is_none_or(|k| !self.seen.contains(&k)) {
                        widgets.push((reference(kid), kd));
                    }
                }
            }
            _ => widgets.push((id, d)),
        }
        let node = Node {
            id,
            dict: d,
            full_name,
            terminal: fields.is_empty(),
            widgets,
            ft: here.ft,
            ff: here.ff.unwrap_or(0),
            v: here.v,
            opt: here.opt,
        };
        if let Err(issue) = visit(&node, self.budget, &mut self.issues) {
            self.issues.push(issue);
            self.done = true;
            return;
        }
        for kid in fields {
            self.node(kid, &node.full_name, here, depth + 1, visit);
            if self.done {
                return;
            }
        }
    }
}
