// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rewriting links to anchors so they point into the unit that holds the target.

use super::*;

/// Where the units' anchors are, for the links of unit `current`.
pub(super) struct Links<'x> {
    pub(super) at: &'x HashMap<&'x str, usize>,
    /// Each unit's first file name.
    pub(super) files: &'x [String],
    /// The fragment a heading anchor has in the part file that holds it.
    pub(super) frags: &'x HashMap<&'x str, String>,
    pub(super) current: usize,
}

impl Links<'_> {
    /// The new target of an anchor link `#frag` whose anchor lies in another unit:
    /// `<file>.md#<fragment>`, the fragment the heading has in that file (a second
    /// `Summary` is `summary` in a part where it comes first). The anchor is the fragment
    /// itself, `h:<fragment>`, or the heading slug of the decoded fragment.
    fn anchor_target(&self, frag: &str) -> Option<String> {
        let decoded = heading_slug(&percent_decode(frag));
        let candidates = [
            frag.to_string(),
            format!("h:{frag}"),
            format!("h:{decoded}"),
        ];
        for c in &candidates {
            if let Some(&k) = self.at.get(c.as_str()) {
                if k == self.current {
                    return None;
                }
                let file = self.files.get(k)?;
                let frag = match self.frags.get(c.as_str()) {
                    Some(f) => f.as_str(),
                    None => c.strip_prefix("h:").unwrap_or(c),
                };
                return Some(format!("{file}.md#{frag}"));
            }
        }
        None
    }

    /// The target an inline gets in its unit, when it changes.
    fn target(&self, inline: &Inline) -> Option<String> {
        match inline {
            Inline::Link {
                target,
                internal: true,
                wiki: false,
                ..
            } => match target.strip_prefix('#') {
                Some(frag) => self.anchor_target(frag),
                None => relative_path(target).then(|| format!("../{target}")),
            },
            Inline::Image { target, .. } => relative_path(target).then(|| format!("../{target}")),
            _ => None,
        }
    }
}

/// The runs of blocks whose anchors lead to `s`: its own runs, else its slice.
pub(super) fn anchor_runs<'a>(s: &Spec<'a>) -> Vec<(&'a [Block], usize, usize)> {
    match &s.item.content {
        _ if !s.anchors.is_empty() => s.anchors.clone(),
        Content::Slice { list, from, to } => vec![(*list, *from, *to)],
        Content::Owned(_) => Vec::new(),
    }
}

/// The fragment each heading anchor has in the part file of its unit: the headings the
/// unit shows in order (its sections' titles first, a table's repeated lead too), slugged as
/// the Markdown reader does, a repeated slug numbered `-1`, `-2` within the file.
pub(super) fn fragments<'a>(
    specs: &[Spec<'a>],
    at: &HashMap<&'a str, usize>,
) -> HashMap<&'a str, String> {
    let mut out: HashMap<&'a str, String> = HashMap::new();
    for (k, s) in specs.iter().enumerate() {
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut unique = |text: &str| {
            let slug = heading_slug(text);
            let n = seen.entry(slug.clone()).or_insert(0);
            let frag = if *n == 0 { slug } else { format!("{slug}-{n}") };
            *n += 1;
            frag
        };
        let mut take = |anchor: Option<&str>, frag: String| {
            if let Some(a) = anchor
                && let Some((key, unit)) = at.get_key_value(a)
                && *unit == k
            {
                out.entry(*key).or_insert(frag);
            }
        };
        for w in &s.item.wraps {
            if let Some(title) = w.title.filter(|t| !t.trim().is_empty()) {
                let frag = unique(title);
                take(w.anchor, frag);
            }
        }
        walk(s.item.content.blocks(), 0, &mut |node| match node {
            Node::Block(Block::Heading {
                inlines, anchor, ..
            }) => {
                let frag = unique(&rezzer_model::plain_text(inlines));
                take(anchor.as_ref().map(|a| a.0.as_str()), frag);
            }
            Node::Block(Block::Section {
                title: Some(title),
                anchor,
                ..
            }) if !title.trim().is_empty() => {
                let frag = unique(title);
                take(anchor.as_ref().map(|a| a.0.as_str()), frag);
            }
            _ => {}
        });
    }
    out
}

pub(super) fn annotation_needs_rewrite(
    a: &rezzer_model::Annotation,
    links: &Links<'_>,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut found = false;
    walk_inlines(&a.body, depth, &mut |node| {
        if !found && let Node::Inline(inline) = node {
            found = links.target(inline).is_some();
        }
    });
    found
        || a.replies
            .iter()
            .any(|r| annotation_needs_rewrite(r, links, depth + 1))
}

pub(super) fn rewrite_annotation(
    a: &mut rezzer_model::Annotation,
    links: &Links<'_>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    rewrite_inlines(&mut a.body, links, depth);
    for r in &mut a.replies {
        rewrite_annotation(r, links, depth + 1);
    }
}

/// A target that is a relative path: one folder down it needs `../`.
fn relative_path(target: &str) -> bool {
    if target.is_empty() || target.starts_with(['#', '/', '\\', '?']) {
        return false;
    }
    let before = target.split(['/', '?', '#']).next().unwrap_or("");
    !before.contains(':')
}

pub(super) fn block_needs_rewrite(b: &Block, links: &Links<'_>, depth: usize) -> bool {
    let mut found = false;
    walk(std::slice::from_ref(b), depth, &mut |node| {
        if !found && let Node::Inline(inline) = node {
            found = links.target(inline).is_some();
        }
    });
    found
}

pub(super) fn rewrite_block(b: &mut Block, links: &Links<'_>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match b {
        Block::Section { blocks, .. }
        | Block::Quote { blocks, .. }
        | Block::Callout { blocks, .. } => {
            for inner in blocks {
                rewrite_block(inner, links, depth + 1);
            }
        }
        Block::List { items, .. } => {
            for inner in items.iter_mut().flatten() {
                rewrite_block(inner, links, depth + 1);
            }
        }
        Block::Paragraph { inlines, .. }
        | Block::Heading { inlines, .. }
        | Block::Task { inlines, .. } => rewrite_inlines(inlines, links, depth + 1),
        Block::Table { rows, .. } => {
            for cell in rows.iter_mut().flatten() {
                rewrite_inlines(&mut cell.inlines, links, depth + 1);
            }
        }
        Block::Code { .. } | Block::Math { .. } | Block::Media { .. } | Block::Rule { .. } => {}
    }
}

fn rewrite_inlines(list: &mut [Inline], links: &Links<'_>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for inline in list {
        if let Some(new) = links.target(inline) {
            match inline {
                Inline::Link { target, .. } | Inline::Image { target, .. } => *target = new,
                _ => {}
            }
        }
        if let Inline::Link { inlines, .. } = inline {
            rewrite_inlines(inlines, links, depth + 1);
        }
    }
}
