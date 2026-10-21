// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Document scripts and actions: the JavaScript name tree, the open action, document, page,
//! field and link actions. They are listed, never run or followed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lopdf::{Dictionary, Object, ObjectId};

use crate::guard::{Cap, Filter, Measure, cap_words, stream_filters, unpack};
use crate::objects::{
    BrokenTexts, Decode, Shared, TextBudget, Tree, TreeKey, Work, dict, final_id, get_dict,
    get_name, resolve, text_string, text_string_owned, walk_tree,
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

/// The script text the document holds: what is left of [`Limits::max_script_total_bytes`],
/// each script object's text once, and the scripts left without their text.
pub(crate) struct ScriptRoom {
    left: u64,
    texts: HashMap<ObjectId, Option<Arc<str>>>,
    refused: usize,
    first: Option<String>,
    /// What is left of the same total for the actions that follow the first of a chain
    /// through `/Next`, each counted at [`ACTION_COST`] plus its kind and target, and how many
    /// chains were cut short.
    actions_left: u64,
    chains_cut: usize,
}

/// What one listed action holds besides its texts, as measured: the entry in the reader's
/// list, its strings' headers and the copy in the list of scripts.
const ACTION_COST: u64 = 256;

impl ScriptRoom {
    pub(crate) fn new(total: u64) -> ScriptRoom {
        ScriptRoom {
            left: total,
            texts: HashMap::new(),
            refused: 0,
            first: None,
            actions_left: total,
            chains_cut: 0,
        }
    }

    /// Takes room for one more action of `bytes`; `false` when it does not fit, and then the
    /// chain it stands in, which ends there, is counted as cut short.
    fn take_action(&mut self, bytes: u64) -> bool {
        let n = ACTION_COST.saturating_add(bytes);
        if n <= self.actions_left {
            self.actions_left -= n;
            return true;
        }
        self.chains_cut += 1;
        false
    }

    /// Takes `n` bytes; when they do not fit, counts the script `what` as left without text.
    fn reserve(&mut self, n: u64, what: impl FnOnce() -> String) -> bool {
        if n <= self.left {
            self.left -= n;
            return true;
        }
        self.refused += 1;
        self.first.get_or_insert_with(what);
        false
    }

    /// The issues for the scripts left without their text and the actions left out, if any.
    pub(crate) fn issues(&self, total: u64) -> Vec<Issue> {
        let mut out: Vec<Issue> = self.text_issue(total).into_iter().collect();
        if self.chains_cut > 0 {
            let n = self.chains_cut;
            out.push(Issue::new(
                SCRIPTS,
                IssueKind::Limit,
                format!(
                    "{n} {} cut short: the actions that follow others through /Next take more \
                     than the {total} bytes they may hold together, so the rest of {} is not \
                     listed",
                    if n == 1 {
                        "action chain is"
                    } else {
                        "action chains are"
                    },
                    if n == 1 { "it" } else { "them" },
                ),
            ));
        }
        out
    }

    fn text_issue(&self, total: u64) -> Option<Issue> {
        let first = self.first.as_ref()?;
        let n = self.refused;
        Some(Issue::new(
            SCRIPTS,
            IssueKind::Limit,
            format!(
                "{n} {} listed without {} text: the scripts of the document together take more \
                 than the {total} bytes of script text it may hold (the first is {first})",
                if n == 1 { "script is" } else { "scripts are" },
                if n == 1 { "its" } else { "their" },
            ),
        ))
    }
}

const CHAIN_LIMIT: &str =
    "an action chain is longer or deeper than the limits allow; the rest is not read";

/// The words for an action type: lower case, words joined by hyphens.
fn kind_word(s: &[u8]) -> String {
    let s = &s[..s.len().min(crate::objects::NAME_BYTES)];
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
    /// screens) in page order. Plain jumps inside the document are left out, and so is the
    /// address a link starts with, which [`Pdf::links`] lists. Read on the first call and kept.
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
            if let Some(r) = final_id(doc, a)
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
            for (k, (trigger, action)) in entries.into_iter().enumerate() {
                // The address a link starts with is listed by `links()`; later ones in its
                // chain are listed here.
                let mut skip_uri = link
                    && k == 0
                    && ad.has(b"A")
                    && dict(doc, action).and_then(|d| get_name(doc, d, b"S")) == Some(b"URI");
                let found = self.action_chain(action, budget, chains, &mut found_issues);
                chains
                    .broken
                    .after(&mut chains.shared, || format!("the {trigger} action"));
                let Ok(found) = found else {
                    return Err(budget.limit_issue(SCRIPTS, "scripts"));
                };
                for a in found {
                    if skip_uri && a.kind == "uri" {
                        skip_uri = false;
                        continue;
                    }
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
        let start = final_id(doc, action);
        if let Some(found) = start.and_then(|id| chains.cache.get(&id)) {
            // The actions are copied, so each copy is charged.
            let mut copied = Vec::with_capacity(found.len());
            for (k, a) in found.iter().enumerate() {
                if !budget.charge(a.weight()) {
                    return Err(Spent);
                }
                if k > 0 && !self.take_action(a) {
                    break;
                }
                copied.push(a.clone());
            }
            return Ok(copied);
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
            if let Some(id) = final_id(doc, obj)
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
                let a = self.action(d, s, budget, &mut chains.shared, location, issues)?;
                // The actions that follow the first through /Next share one room; past it the
                // chain ends, and one issue counts the chains cut short.
                if depth > 1 && !self.take_action(&a) {
                    break;
                }
                out.push(a);
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
        // Read once for every reader of the document, and its text held once.
        let known = id.and_then(|id| {
            self.script_room
                .lock()
                .ok()
                .and_then(|room| room.texts.get(&id).cloned())
        });
        if let (Some(id), Some(text)) = (id, known) {
            shared.scripts.insert(id, text.clone());
            return Ok(text);
        }
        let text: Option<Arc<str>> = self
            .read_script(id, obj, budget, location, issues)?
            .map(Into::into);
        if text.is_none() && !matches!(obj, Object::Stream(_) | Object::Null) {
            shared.check(doc, js);
        }
        if let Some(id) = id {
            shared.scripts.insert(id, text.clone());
            if let Ok(mut room) = self.script_room.lock() {
                room.texts.insert(id, text.clone());
            }
        }
        Ok(text)
    }

    /// The text of a `/JS` value `obj` (object `id`, if indirect). A script longer than
    /// `max_script_bytes` is not read: a string is left as it is, a stream is refused by the
    /// size the guard measured before it is unpacked, and a stream the guard has no size for
    /// is unpacked only up to that cap. Every copy held is charged to `budget`: the unpacked
    /// bytes and the text.
    fn read_script(
        &self,
        id: Option<ObjectId>,
        obj: &Object,
        budget: &mut TextBudget<'_>,
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<Option<String>, Spent> {
        let doc = &self.doc;
        let cap = self.limits.max_script_bytes;
        let fail = |kind, what: String| Issue::new(location, kind, what);
        let named = || match id {
            Some((n, g)) => format!("the script in object {n} {g}"),
            None => "a script".to_string(),
        };
        let too_long = |what: String| {
            fail(
                IssueKind::Limit,
                format!("{what}, more than the {cap} bytes a script may have; it is not read"),
            )
        };
        match Some(obj) {
            Some(Object::String(s, _)) => {
                if s.len() as u64 > cap {
                    issues.push(too_long(format!("{} is {} bytes long", named(), s.len())));
                    return Ok(None);
                }
                // The string itself is an object already charged; the text is the copy.
                if !self.reserve_script(s.len() as u64, &named) {
                    return Ok(None);
                }
                let text = text_string(s);
                let grown = (text.len() as u64).saturating_sub(s.len() as u64);
                if grown > 0 && !self.reserve_script(grown, &named) {
                    self.refund_script(s.len() as u64);
                    return Ok(None);
                }
                if !budget.take(text.len() as u64) {
                    return Err(Spent);
                }
                Ok(Some(text))
            }
            Some(Object::Stream(stream)) => {
                let refused = id.is_some_and(|id| self.verdict.refused_objects.contains(&id));
                let filters = stream_filters(doc, &stream.dict);
                let size = id.and_then(|id| self.verdict.sizes.get(&id).copied());
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
                if let Some(n) = size
                    && n > cap
                {
                    issues.push(too_long(format!("{} unpacks to {n} bytes", named())));
                    return Ok(None);
                }
                // Room for the text it may give: the measured size, else the cap.
                let room = size.unwrap_or(cap);
                if !self.reserve_script(room, &named) {
                    return Ok(None);
                }
                if !budget.take(size.unwrap_or(stream.content.len() as u64)) {
                    return Err(Spent);
                }
                let limits = Limits {
                    max_stream_bytes: cap.min(self.limits.max_stream_bytes),
                    ..self.limits
                };
                match unpack(
                    &stream.content,
                    &filters,
                    &limits,
                    self.limits.max_total_bytes,
                    &|| false,
                ) {
                    (Measure::Fits(_), Some(bytes)) => {
                        // The unpacked bytes were charged; the text is a second copy only when
                        // the script is not plain ASCII.
                        let (text, copied) = text_string_owned(bytes);
                        let len = text.len() as u64;
                        if len > room {
                            if !self.reserve_script(len - room, &named) {
                                self.refund_script(room);
                                return Ok(None);
                            }
                        } else {
                            self.refund_script(room - len);
                        }
                        if copied && !budget.take(len) {
                            return Err(Spent);
                        }
                        Ok(Some(text))
                    }
                    (Measure::Over(Cap::Stream), _) => {
                        self.refund_script(room);
                        issues.push(too_long(format!("{} unpacks to more bytes", named())));
                        Ok(None)
                    }
                    (Measure::Over(over), _) => {
                        self.refund_script(room);
                        issues.push(fail(
                            IssueKind::Limit,
                            format!("a script stream {}", cap_words(over, &self.limits)),
                        ));
                        Ok(None)
                    }
                    _ => {
                        self.refund_script(room);
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }

    /// Takes `n` bytes of the document's script text room; `false`, and the script `named`
    /// counted as left without its text, when they do not fit.
    fn reserve_script(&self, n: u64, named: &dyn Fn() -> String) -> bool {
        match self.script_room.lock() {
            Ok(mut room) => room.reserve(n, named),
            Err(_) => false,
        }
    }

    /// Takes room for one listed action of the document.
    fn take_action(&self, a: &Action) -> bool {
        let bytes = a.kind.len() as u64 + a.target.as_ref().map_or(0, |t| t.len() as u64);
        match self.script_room.lock() {
            Ok(mut room) => room.take_action(bytes),
            Err(_) => false,
        }
    }

    /// Gives back `n` bytes taken for a text that turned out shorter or was not read.
    fn refund_script(&self, n: u64) {
        if let Ok(mut room) = self.script_room.lock() {
            room.left = room.left.saturating_add(n);
        }
    }
}
