// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Document scripts and actions: the JavaScript name tree, the open action, document, page,
//! field and link actions. They are listed, never run or followed.

use std::collections::HashSet;

use lopdf::{Dictionary, Object, ObjectId};

use crate::guard::{Filter, Measure, cap_words, stream_filters, unpack};
use crate::objects::{
    TextBudget, Tree, TreeKey, dict, get, get_dict, get_name, reference, resolve, text_string,
    walk_tree,
};
use crate::{Issue, IssueKind, Pdf};

/// One action the document holds, with what starts it. Nothing here is run or followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// What starts it: `document` (a document-level script), `open`, `will-close`,
    /// `will-save`, `did-save`, `will-print`, `did-print`, `page-open`, `page-close`,
    /// `field <full name> <event>` or `link page <N>`.
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
    fn weight(&self) -> usize {
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        self.trigger.len()
            + self.kind.len()
            + text(&self.name)
            + text(&self.target)
            + text(&self.text)
    }
}

/// One action of a chain, read but not run.
#[derive(Debug, Clone)]
pub(crate) struct Action {
    pub kind: String,
    pub target: Option<String>,
    pub text: Option<String>,
}

/// The spent budget ends the reader.
pub(crate) struct Spent;

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
    /// order, then link actions. Plain jumps inside the document are left out, and so are
    /// link addresses, which [`Pdf::links`] lists. Read on the first call and kept.
    pub fn scripts(&self) -> &[Script] {
        self.scripts.get_or_init(|| self.read_scripts())
    }

    fn read_scripts(&self) -> Vec<Script> {
        let mut issues = Vec::new();
        let mut out = Vec::new();
        let mut budget = self.text_budget();
        // A spent budget has added its issue; what was read is kept.
        let _ = self.collect_scripts(&mut out, &mut budget, &mut issues);
        self.note(issues);
        out
    }

    fn collect_scripts<'a>(
        &'a self,
        out: &mut Vec<Script>,
        budget: &mut TextBudget<'a>,
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
                self.push_chain(out, "document", Some(name), v, budget, issues)?;
            }
        }
        if let Some(open) = catalog.and_then(|c| c.get(b"OpenAction").ok())
            && dict(doc, open).is_some()
        {
            self.push_chain(out, "open", None, open, budget, issues)?;
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
                self.push_chain(out, trigger, None, v, budget, issues)?;
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
                self.push_chain(out, trigger, name, v, budget, issues)?;
            }
        }
        self.field_scripts(out, budget, issues)?;
        let mut seen = HashSet::new();
        let mut spent = false;
        let mut found_issues = Vec::new();
        self.each_annot(SCRIPTS, issues, |page, _, a| {
            if let Some(r) = reference(a)
                && !seen.insert(r)
            {
                return Ok(());
            }
            let Some(ad) = dict(doc, a) else {
                return Ok(());
            };
            if get_name(doc, ad, b"Subtype") != Some(b"Link") {
                return Ok(());
            }
            let Ok(action) = ad.get(b"A") else {
                return Ok(());
            };
            let mut found = Vec::new();
            if self
                .action_chain(action, budget, &mut found, SCRIPTS, &mut found_issues)
                .is_err()
            {
                spent = true;
                return Err(budget.limit_issue(SCRIPTS, "scripts"));
            }
            for a in found.into_iter().filter(|a| a.kind != "uri") {
                let s = Script {
                    trigger: format!("link page {page}"),
                    name: None,
                    kind: a.kind,
                    target: a.target,
                    text: a.text,
                };
                if !budget.charge(s.weight()) {
                    spent = true;
                    return Err(budget.limit_issue(SCRIPTS, "scripts"));
                }
                out.push(s);
            }
            Ok(())
        });
        issues.append(&mut found_issues);
        if spent { Err(Spent) } else { Ok(()) }
    }

    /// Adds the actions of the chain at `action` with one trigger. A spent budget adds the
    /// Limit issue and ends the reader.
    pub(crate) fn push_chain(
        &self,
        out: &mut Vec<Script>,
        trigger: &str,
        name: Option<String>,
        action: &Object,
        budget: &mut TextBudget<'_>,
        issues: &mut Vec<Issue>,
    ) -> Result<(), Spent> {
        let mut found = Vec::new();
        let read = self.action_chain(action, budget, &mut found, SCRIPTS, issues);
        for a in found {
            let s = Script {
                trigger: trigger.to_string(),
                name: name.clone(),
                kind: a.kind,
                target: a.target,
                text: a.text,
            };
            if !budget.charge(s.weight()) {
                issues.push(budget.limit_issue(SCRIPTS, "scripts"));
                return Err(Spent);
            }
            out.push(s);
        }
        if read.is_err() {
            issues.push(budget.limit_issue(SCRIPTS, "scripts"));
            return Err(Spent);
        }
        Ok(())
    }

    /// The actions of the chain that starts at `action`, `/Next` entries after the action
    /// that names them. Plain jumps inside the document are left out. A cycle ends the
    /// chain; it is bounded by `max_items` and `max_depth`. Script text is charged to
    /// `budget`; `Err` when it is spent.
    pub(crate) fn action_chain(
        &self,
        action: &Object,
        budget: &mut TextBudget<'_>,
        out: &mut Vec<Action>,
        location: &str,
        issues: &mut Vec<Issue>,
    ) -> Result<(), Spent> {
        let doc = &self.doc;
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut stack: Vec<(&Object, usize)> = vec![(action, 1)];
        let mut count = 0usize;
        while let Some((obj, depth)) = stack.pop() {
            if let Some(id) = reference(obj)
                && !seen.insert(id)
            {
                issues.push(Issue::new(
                    location,
                    IssueKind::Damaged,
                    format!(
                        "an action chain reaches object {} {} a second time",
                        id.0, id.1
                    ),
                ));
                continue;
            }
            count += 1;
            if count > self.limits.max_items || depth > self.limits.max_depth {
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
            // /Next is one action or an array of them, read in order.
            let next: &[Object] = match d.get(b"Next").ok() {
                Some(next) => match resolve(doc, next) {
                    Some(Object::Array(list)) => list,
                    Some(_) => std::slice::from_ref(next),
                    None => &[],
                },
                None => &[],
            };
            // Entries waiting count against the bound too, so a list that many actions
            // share is not stacked again and again.
            if count + stack.len() + next.len() > self.limits.max_items {
                issues.push(Issue::new(location, IssueKind::Limit, CHAIN_LIMIT));
                break;
            }
            stack.extend(next.iter().rev().map(|n| (n, depth + 1)));
        }
        Ok(())
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
