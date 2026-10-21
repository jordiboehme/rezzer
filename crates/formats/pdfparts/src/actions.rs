// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Document scripts and actions: the JavaScript name tree, the open action, document, page,
//! field and link actions. They are listed, never run or followed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lopdf::{Dictionary, Object, ObjectId};

use crate::guard::{Filter, Measure, cap_words, stream_filters, unpack};
use crate::objects::{
    BrokenTexts, Decode, Shared, TextBudget, Tree, TreeKey, Work, dict, final_id, get_dict,
    get_name, reference, resolve, text_string, walk_tree,
};
use crate::{Issue, IssueKind, Limits, Pdf};

/// One action the document holds, with what starts it. Nothing here is run or followed.
/// Texts that many actions name through one object are shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// What starts it: `document` (a document-level script), `open`, `will-close`,
    /// `will-save`, `did-save`, `will-print`, `did-print`, `page-open`, `page-close`,
    /// `field <full name> <event>`, `link page <N>` or `annotation page <N> <event>`.
    pub trigger: String,
    /// The name of a document-level script; `page <N>` for a page action.
    pub name: Option<String>,
    /// `javascript`, `launch`, `uri`, `submit-form`, `import-data`, `goto-remote`,
    /// `goto-embedded`, `reset-form`, `named`, `hide` and so on.
    pub kind: String,
    /// The file, address or name the action points at.
    pub target: Option<Arc<str>>,
    /// The script text of a JavaScript action.
    pub text: Option<Arc<str>>,
}

impl Script {
    /// What it adds to the action it lists: its trigger and name. The action's kind was
    /// charged as it was read, its target and text are shared.
    pub(crate) fn weight(&self) -> usize {
        self.trigger.len() + self.name.as_ref().map_or(0, String::len)
    }
}

/// One action of a chain, read but not run.
#[derive(Debug, Clone)]
pub(crate) struct Action {
    pub kind: String,
    pub target: Option<Arc<str>>,
    pub text: Option<Arc<str>>,
}

impl Action {
    /// What one copy of it holds besides the texts it shares.
    fn weight(&self) -> usize {
        self.kind.len()
    }
}

pub(crate) use crate::objects::Spent;

/// The action chains one reader has read: its visits, the chains by their first action, the
/// actions reached a second time, and the texts read once and shared.
pub(crate) struct Chains {
    work: Work,
    cache: HashMap<ObjectId, Vec<Action>>,
    repeats: usize,
    first_repeat: Option<ObjectId>,
    location: &'static str,
    pub(crate) shared: Shared,
    pub(crate) broken: BrokenTexts,
}

impl Chains {
    pub(crate) fn new(limits: &Limits, location: &'static str) -> Chains {
        Chains {
            work: Work::new(limits),
            cache: HashMap::new(),
            repeats: 0,
            first_repeat: None,
            location,
            shared: Shared::default(),
            broken: BrokenTexts::default(),
        }
    }

    /// Adds the one issue for actions reached a second time, and the one for texts that
    /// could not be read.
    pub(crate) fn finish(&self, issues: &mut Vec<Issue>) {
        issues.extend(self.broken.issue(self.location));
        if let Some(id) = self.first_repeat {
            issues.push(Issue::new(
                self.location,
                IssueKind::Damaged,
                format!(
                    "{} action {} an action a second time: a /Next chain loops or repeats (the first is object {} {})",
                    self.repeats,
                    if self.repeats == 1 { "entry reaches" } else { "entries reach" },
                    id.0,
                    id.1
                ),
            ));
        }
    }
}

const SCRIPTS: &str = "scripts";

const CHAIN_LIMIT: &str =
    "an action chain is longer or deeper than the limits allow; the rest is not read";

/// The words for an action type: lower case, words joined by hyphens.
fn kind_word(s: &[u8]) -> String {
    match s {
        b"JavaScript" => "javascript".into(),
        b"URI" => "uri".into(),
        b"GoToR" => "goto-remote".into(),
        b"GoToE" => "goto-embedded".into(),
        b"GoTo" => "goto".into(),
        b"GoTo3DView" => "goto-3d-view".into(),
        b"SetOCGState" => "set-layer-state".into(),
        _ => {
            let mut out = String::new();
            for (i, c) in String::from_utf8_lossy(s).chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    out.push('-');
                }
                out.extend(c.to_lowercase());
            }
            out
        }
    }
}

/// The words for a key of a field's or widget's `/AA`.
pub(crate) fn event_word(key: &[u8]) -> String {
    match key {
        b"K" => "keystroke".into(),
        b"F" => "format".into(),
        b"V" => "validate".into(),
        b"C" => "calculate".into(),
        b"Fo" => "focus".into(),
        b"Bl" => "blur".into(),
        b"D" => "mouse-down".into(),
        b"U" => "mouse-up".into(),
        b"E" => "mouse-enter".into(),
        b"X" => "mouse-exit".into(),
        b"PO" => "page-open".into(),
        b"PC" => "page-close".into(),
        b"PV" => "page-visible".into(),
        b"PI" => "page-invisible".into(),
        other => String::from_utf8_lossy(other).to_lowercase(),
    }
}

impl Pdf {
    /// Every script and action of the document, in this order: document-level scripts, the
    /// open action, document actions, page actions in page order, field actions in field
    /// order, then the actions of every other annotation (links, widgets outside the form,
    /// screens) in page order. Plain jumps inside the document are left out, and so are link
    /// addresses, which [`Pdf::links`] lists. Read on the first call and kept.
    pub fn scripts(&self) -> &[Script] {
        self.scripts.get_or_init(|| self.read_scripts())
    }

    fn read_scripts(&self) -> Vec<Script> {
        let mut issues = Vec::new();
        let mut out = Vec::new();
        let mut budget = self.text_budget();
        let mut chains = Chains::new(&self.limits, SCRIPTS);
        // A spent budget has added its issue; what was read is kept.
        if self
            .collect_scripts(&mut out, &mut budget, &mut chains, &mut issues)
            .is_err()
        {
            issues.push(budget.limit_issue(SCRIPTS, "scripts"));
        }
        chains.finish(&mut issues);
        self.note(issues);
        out
    }

    fn collect_scripts<'a>(
        &'a self,
        out: &mut Vec<Script>,
        budget: &mut TextBudget<'a>,
        chains: &mut Chains,
        issues: &mut Vec<Issue>,
    ) -> Result<(), Spent> {
        let doc = &self.doc;
        let catalog = self.catalog();
        let tree = catalog
            .and_then(|c| get_dict(doc, c, b"Names"))
            .and_then(|n| n.get(b"JavaScript").ok());
        if let Some(tree) = tree {
            let pairs = walk_tree(doc, tree, Tree::Name, &self.limits, budget, SCRIPTS, issues);
            for (k, v) in pairs {
                let name = match k {
                    TreeKey::Name(n) => text_string(&n),
                    TreeKey::Number(n) => n.to_string(),
                };
                self.push_chain(out, "document", Some(name), v, budget, chains, issues)?;
            }
        }
        if let Some(open) = catalog.and_then(|c| c.get(b"OpenAction").ok())
            && dict(doc, open).is_some()
        {
            self.push_chain(out, "open", None, open, budget, chains, issues)?;
        }
        if let Some(aa) = catalog.and_then(|c| get_dict(doc, c, b"AA")) {
            for (key, v) in aa.iter() {
                let trigger = match key.as_slice() {
                    b"WC" => "will-close",
                    b"WS" => "will-save",
                    b"DS" => "did-save",
                    b"WP" => "will-print",
                    b"DP" => "did-print",
                    _ => continue,
                };
                self.push_chain(out, trigger, None, v, budget, chains, issues)?;
            }
        }
        for (i, node) in self.pages.iter().enumerate() {
            let Some(aa) = doc
                .objects
                .get(&node.id)
                .and_then(|o| dict(doc, o))
                .and_then(|pd| get_dict(doc, pd, b"AA"))
            else {
                continue;
            };
            for (key, v) in aa.iter() {
                let trigger = match key.as_slice() {
                    b"O" => "page-open",
                    b"C" => "page-close",
                    _ => continue,
                };
                let name = Some(format!("page {}", i + 1));
                self.push_chain(out, trigger, name, v, budget, chains, issues)?;
            }
        }
        // Field actions as the form read them; nothing is read again.
        let data = self.form_data();
        if let Some(form) = &data.form {
            for fa in &data.actions {
                let field = &form.fields[fa.field];
                let s = Script {
                    trigger: format!("field {} {}", field.full_name, fa.event),
                    name: None,
                    kind: fa.kind.clone(),
                    target: fa.target.clone(),
                    text: fa.js.map(|j| field.actions[j].1.clone()),
                };
                // The kind is a copy of what the form keeps; target and text are shared.
                if !budget.charge(s.weight() + s.kind.len()) {
                    return Err(Spent);
                }
                out.push(s);
            }
        }
        // The actions of every other annotation: links, widgets outside the form, screens.
        let mut seen = HashSet::new();
        let mut found_issues = Vec::new();
        self.each_annot(SCRIPTS, issues, |page, _, a| {
            if let Some(r) = reference(a)
                && data.widgets.contains(&r)
            {
                return Ok(());
            }
            if let Some(r) = final_id(doc, a)
                && !seen.insert(r)
            {
                return Ok(());
            }
            let Some(ad) = dict(doc, a) else {
                return Ok(());
            };
            let link = get_name(doc, ad, b"Subtype") == Some(b"Link");
            let mut entries: Vec<(String, &Object)> = Vec::new();
            if let Ok(action) = ad.get(b"A") {
                let trigger = if link {
                    format!("link page {page}")
                } else {
                    format!("annotation page {page} activate")
                };
                entries.push((trigger, action));
            }
            if let Some(aa) = get_dict(doc, ad, b"AA") {
                for (key, v) in aa.iter() {
                    entries.push((format!("annotation page {page} {}", event_word(key)), v));
                }
            }
            for (trigger, action) in entries {
                let found = self.action_chain(action, budget, chains, &mut found_issues);
                chains
                    .broken
                    .after(&mut chains.shared, || format!("the {trigger} action"));
                let Ok(found) = found else {
                    return Err(budget.limit_issue(SCRIPTS, "scripts"));
                };
                // Link addresses are listed by `links()`.
                for a in found.into_iter().filter(|a| !(link && a.kind == "uri")) {
                    let s = Script {
                        trigger: trigger.clone(),
                        name: None,
                        kind: a.kind,
                        target: a.target,
                        text: a.text,
                    };
                    if !budget.charge(s.weight()) {
                        return Err(budget.limit_issue(SCRIPTS, "scripts"));
                    }
                    out.push(s);
                }
            }
            Ok(())
        });
        issues.append(&mut found_issues);
        // A spent budget ended the walk with its issue; what was read is kept.
        Ok(())
    }

    /// Adds the actions of the chain at `action` with one trigger. `Err` when the budget is
    /// spent; the caller adds the Limit issue.
    #[allow(clippy::too_many_arguments)]
    fn push_chain(
        &self,
        out: &mut Vec<Script>,
        trigger: &str,
        name: Option<String>,
        action: &Object,
        budget: &mut TextBudget<'_>,
        chains: &mut Chains,
        issues: &mut Vec<Issue>,
    ) -> Result<(), Spent> {
        let found = self.action_chain(action, budget, chains, issues);
        chains.broken.after(&mut chains.shared, || match &name {
            Some(n) => format!("{trigger} script {}", crate::objects::quote(n)),
            None => format!("{trigger} action"),
        });
        for a in found? {
            let s = Script {
                trigger: trigger.to_string(),
                name: name.clone(),
                kind: a.kind,
                target: a.target,
                text: a.text,
            };
            if !budget.charge(s.weight()) {
                return Err(Spent);
            }
            out.push(s);
        }
        Ok(())
    }

    /// The actions of the chain that starts at `action`, `/Next` entries after the action
    /// that names them. Plain jumps inside the document are left out. Every entry counts
    /// toward the reader's visits in `chains`; an action reached a second time is skipped and
    /// counted for one issue. A chain that starts at an object read before is not read again:
    /// its actions are copied, and the copies charged. Script text is charged to `budget`;
    /// `Err` when it is spent.
    pub(crate) fn action_chain(
        &self,
        action: &Object,
        budget: &mut TextBudget<'_>,
        chains: &mut Chains,
        issues: &mut Vec<Issue>,
    ) -> Result<Vec<Action>, Spent> {
        let doc = &self.doc;
        let start = reference(action);
        if let Some(found) = start.and_then(|id| chains.cache.get(&id)) {
            // The actions are copied, so each copy is charged.
            for a in found {
                if !budget.charge(a.weight()) {
                    return Err(Spent);
                }
            }
            return Ok(found.clone());
        }
        let location = chains.location;
        let mut out = Vec::new();
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut stack: Vec<(&Object, usize)> = Vec::new();
        if chains.work.take(1) {
            stack.push((action, 1));
        } else {
            issues.extend(chains.work.limit_issue(location, "action chains"));
        }
        while let Some((obj, depth)) = stack.pop() {
            if let Some(id) = reference(obj)
                && !seen.insert(id)
            {
                chains.repeats += 1;
                chains.first_repeat.get_or_insert(id);
                continue;
            }
            if depth > self.limits.max_depth {
                issues.push(Issue::new(location, IssueKind::Limit, CHAIN_LIMIT));
                break;
            }
            let Some(d) = dict(doc, obj) else {
                continue;
            };
            // Each action is charged, so chains that many fields share stay bounded.
            if !budget.charge(0) {
                return Err(Spent);
            }
            if let Some(s) = get_name(doc, d, b"S")
                && s != b"GoTo"
            {
                out.push(self.action(d, s, budget, &mut chains.shared, location, issues)?);
            }
            // /Next is one action or an array of them, read in order. Each entry counts as
            // it is stacked, so a list that many actions share costs each time.
            let next: &[Object] = match d.get(b"Next").ok() {
                Some(next) => match resolve(doc, next) {
                    Some(Object::Array(list)) => list,
                    Some(_) => std::slice::from_ref(next),
                    None => &[],
                },
                None => &[],
            };
            if !chains.work.take(next.len()) {
                issues.extend(chains.work.limit_issue(location, "action chains"));
                break;
            }
            stack.extend(next.iter().rev().map(|n| (n, depth + 1)));
        }
        if let Some(id) = start {
            chains.cache.insert(id, out.clone());
        }
        Ok(out)
    }

    fn action(
        &self,
        d: &Dictionary,
        s: &[u8],
        budget: &mut TextBudget<'_>,
        shared: &mut Shared,
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<Action, Spent> {
        let doc = &self.doc;
        let kind = kind_word(s);
        // A file specification: a string, or the `/UF` or `/F` of a dictionary.
        let file = |shared: &mut Shared, budget: &mut TextBudget<'_>, f: Option<&Object>| {
            let Some(f) = f else {
                return Ok(None);
            };
            match resolve(doc, f) {
                Some(Object::String(..)) => shared.text(doc, f, Decode::Text, budget),
                Some(Object::Dictionary(fs)) => match shared.text_at(doc, fs, b"UF", budget)? {
                    Some(n) => Ok(Some(n)),
                    None => shared.text_at(doc, fs, b"F", budget),
                },
                _ => {
                    shared.check(doc, f);
                    Ok(None)
                }
            }
        };
        let target = match s {
            b"URI" => match d.get(b"URI") {
                Ok(u) if matches!(resolve(doc, u), Some(Object::String(..))) => {
                    shared.text(doc, u, Decode::Lossy, budget)?
                }
                Ok(u) => {
                    shared.check(doc, u);
                    None
                }
                Err(_) => None,
            },
            b"Launch" => match file(shared, budget, d.get(b"F").ok())? {
                Some(f) => Some(f),
                None => {
                    let win = get_dict(doc, d, b"Win").and_then(|w| w.get(b"F").ok());
                    match win {
                        Some(f) if matches!(resolve(doc, f), Some(Object::String(..))) => {
                            shared.text(doc, f, Decode::Text, budget)?
                        }
                        _ => None,
                    }
                }
            },
            b"SubmitForm" | b"ImportData" | b"GoToR" | b"GoToE" => {
                file(shared, budget, d.get(b"F").ok())?
            }
            b"Named" => match d.get(b"N") {
                Ok(n) if matches!(resolve(doc, n), Some(Object::Name(_))) => {
                    shared.text(doc, n, Decode::Text, budget)?
                }
                _ => None,
            },
            _ => None,
        };
        let text = if s == b"JavaScript" {
            self.script_text(d, budget, shared, location, issues)?
        } else {
            None
        };
        // The kind is a copy; the texts were charged as they were read.
        if !budget.take(kind.len() as u64) {
            return Err(Spent);
        }
        Ok(Action { kind, target, text })
    }

    /// The text of `/JS`: a string, or a stream unpacked under the stream limits. Its
    /// length is charged to `budget` before a stream is unpacked.
    fn script_text(
        &self,
        d: &Dictionary,
        budget: &mut TextBudget<'_>,
        shared: &mut Shared,
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<Option<Arc<str>>, Spent> {
        let doc = &self.doc;
        let Ok(js) = d.get(b"JS") else {
            return Ok(None);
        };
        // A script object that many actions name is read once.
        let (id, obj) = match doc.dereference(js) {
            Ok(found) => found,
            Err(_) => {
                shared.check(doc, js);
                return Ok(None);
            }
        };
        if let Some(id) = id
            && let Some(t) = shared.scripts.get(&id)
        {
            return Ok(t.clone());
        }
        let text: Option<Arc<str>> = self
            .read_script(id, obj, budget, location, issues)?
            .map(Into::into);
        if text.is_none() && !matches!(obj, Object::Stream(_) | Object::Null) {
            shared.check(doc, js);
        }
        if let Some(id) = id {
            shared.scripts.insert(id, text.clone());
        }
        Ok(text)
    }

    /// The text of a `/JS` value `obj` (object `id`, if indirect).
    fn read_script(
        &self,
        id: Option<ObjectId>,
        obj: &Object,
        budget: &mut TextBudget<'_>,
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<Option<String>, Spent> {
        let doc = &self.doc;
        match Some(obj) {
            Some(Object::String(s, _)) => {
                if !budget.take(s.len() as u64) {
                    return Err(Spent);
                }
                Ok(Some(text_string(s)))
            }
            Some(Object::Stream(stream)) => {
                let refused = id.is_some_and(|id| self.verdict.refused_objects.contains(&id));
                let filters = stream_filters(doc, &stream.dict);
                let size = id.and_then(|id| self.verdict.sizes.get(&id).copied());
                let fail = |kind, what: String| Issue::new(location, kind, what);
                if refused || filters.contains(&Filter::Opaque) {
                    issues.push(fail(
                        if refused {
                            IssueKind::Limit
                        } else {
                            IssueKind::Unsupported
                        },
                        if refused {
                            "a script stream is over a limit and is not read".into()
                        } else {
                            "a script stream uses a filter that is not unpacked".into()
                        },
                    ));
                    return Ok(None);
                }
                if !budget.take(size.unwrap_or(stream.content.len() as u64)) {
                    return Err(Spent);
                }
                match unpack(
                    &stream.content,
                    &filters,
                    &self.limits,
                    self.limits.max_total_bytes,
                    &|| false,
                ) {
                    (Measure::Fits(_), Some(bytes)) => Ok(Some(text_string(&bytes))),
                    (Measure::Over(cap), _) => {
                        issues.push(fail(
                            IssueKind::Limit,
                            format!("a script stream {}", cap_words(cap, &self.limits)),
                        ));
                        Ok(None)
                    }
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }
}
