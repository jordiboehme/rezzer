// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Document scripts and actions: the JavaScript name tree, the open action, document, page,
//! field and link actions. They are listed, never run or followed.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Object, ObjectId};

use crate::guard::{Filter, Measure, cap_words, stream_filters, unpack};
use crate::objects::{
    TextBudget, Tree, TreeKey, Work, dict, get, get_dict, get_name, reference, resolve,
    text_string, walk_tree,
};
use crate::{Issue, IssueKind, Limits, Pdf};

/// One action the document holds, with what starts it. Nothing here is run or followed.
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
    pub target: Option<String>,
    /// The script text of a JavaScript action.
    pub text: Option<String>,
}

impl Script {
    /// What it adds to the action it lists: its trigger and name. The action's kind, target
    /// and text were charged as they were read.
    pub(crate) fn weight(&self) -> usize {
        self.trigger.len() + self.name.as_ref().map_or(0, String::len)
    }

    /// What a copy of an action that was read before holds.
    fn copied(&self) -> usize {
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        self.kind.len() + text(&self.target) + text(&self.text)
    }
}

/// One action of a chain, read but not run.
#[derive(Debug, Clone)]
pub(crate) struct Action {
    pub kind: String,
    pub target: Option<String>,
    pub text: Option<String>,
}

impl Action {
    /// What one copy of it holds.
    fn weight(&self) -> usize {
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        self.kind.len() + text(&self.target) + text(&self.text)
    }
}

/// The spent budget ends the reader.
pub(crate) struct Spent;

/// The action chains one reader has read: its visits, the chains by their first action, and
/// the actions reached a second time.
pub(crate) struct Chains {
    work: Work,
    cache: HashMap<ObjectId, Vec<Action>>,
    repeats: usize,
    first_repeat: Option<ObjectId>,
    location: &'static str,
}

impl Chains {
    pub(crate) fn new(limits: &Limits, location: &'static str) -> Chains {
        Chains {
            work: Work::new(limits),
            cache: HashMap::new(),
            repeats: 0,
            first_repeat: None,
            location,
        }
    }

    /// Adds the one issue for actions reached a second time.
    pub(crate) fn finish(&self, issues: &mut Vec<Issue>) {
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
                // Kind, target and text are copies of what the form keeps, so they are charged again.
                if !budget.charge(s.weight() + s.copied()) {
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
                && (data.widgets.contains(&r) || !seen.insert(r))
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
                let Ok(found) = self.action_chain(action, budget, chains, &mut found_issues) else {
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
        for a in self.action_chain(action, budget, chains, issues)? {
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
                out.push(self.action(d, s, budget, location, issues)?);
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
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<Action, Spent> {
        let doc = &self.doc;
        let kind = kind_word(s);
        let file = |key: &[u8]| -> Option<String> {
            match get(doc, d, key)? {
                Object::String(s, _) => Some(text_string(s)),
                Object::Dictionary(fs) => get(doc, fs, b"UF")
                    .or_else(|| get(doc, fs, b"F"))
                    .and_then(|o| match o {
                        Object::String(s, _) => Some(text_string(s)),
                        _ => None,
                    }),
                _ => None,
            }
        };
        let target = match s {
            b"URI" => match get(doc, d, b"URI") {
                Some(Object::String(u, _)) => Some(String::from_utf8_lossy(u).into_owned()),
                _ => None,
            },
            b"Launch" => file(b"F").or_else(|| {
                get_dict(doc, d, b"Win").and_then(|w| match get(doc, w, b"F") {
                    Some(Object::String(s, _)) => Some(text_string(s)),
                    _ => None,
                })
            }),
            b"SubmitForm" | b"ImportData" | b"GoToR" | b"GoToE" => file(b"F"),
            b"Named" => get_name(doc, d, b"N").map(|n| String::from_utf8_lossy(n).into_owned()),
            _ => None,
        };
        let text = if s == b"JavaScript" {
            self.script_text(d, budget, location, issues)?
        } else {
            None
        };
        // The kind and target are copies; the text was charged as it was read.
        let copied = kind.len() + target.as_ref().map_or(0, String::len);
        if !budget.take(copied as u64) {
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
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<Option<String>, Spent> {
        let doc = &self.doc;
        let Ok(js) = d.get(b"JS") else {
            return Ok(None);
        };
        let (id, obj) = match js {
            Object::Reference(id) => (Some(*id), doc.objects.get(id)),
            other => (None, Some(other)),
        };
        match obj {
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
