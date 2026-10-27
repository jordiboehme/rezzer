// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The link graph: nodes and edges for links and images.

use rezzer_model::text::{heading_slug, percent_decode};
use rezzer_model::{Anchor, Document, Edge, Embedded, Graph, Node};

use super::Builder;

impl<'b> Builder<'b> {
    /// Adds a node for `target` unless it exists and returns its id. A fragment-only target
    /// (`#hello`) gets the id of the heading anchor it points to (`h:hello`).
    fn node_for(&mut self, target: &str, internal: bool, image: bool) -> String {
        // "self" is the document node, so a target that is literally "self" gets another id.
        let (id, anchor) = match target.strip_prefix('#') {
            Some(frag) if !frag.is_empty() => {
                let id = if frag.starts_with("h:") {
                    frag.to_string()
                } else {
                    // The same slug as the heading anchors, so `#Hello-World` and
                    // `#hello%20world` both point to `h:hello-world`.
                    let slug = heading_slug(&percent_decode(frag));
                    if slug.is_empty() {
                        format!("h:{frag}")
                    } else {
                        format!("h:{slug}")
                    }
                };
                let anchor = Some(Anchor(id.clone()));
                (id, anchor)
            }
            _ if target == "self" => ("link:self".to_string(), None),
            _ => (target.to_string(), None),
        };
        if let Some(&at) = self.node_ids.get(&id) {
            // A target that is also embedded as a picture counts as a picture.
            if image {
                self.nodes[at].kind = Some("image".into());
            }
            return id;
        }
        let kind = if image {
            "image"
        } else if target.starts_with('#') {
            "anchor"
        } else if internal {
            "path"
        } else {
            "url"
        };
        self.node_ids.insert(id.clone(), self.nodes.len());
        self.nodes.push(Node {
            id: id.clone(),
            label: target.to_string(),
            kind: Some(kind.into()),
            anchor,
        });
        id
    }

    fn add_edge(&mut self, id: String, kind: &'static str) {
        if self.edge_keys.insert((id.clone(), kind)) {
            self.edges.push(Edge {
                from: "self".into(),
                to: id,
                kind: kind.into(),
                anchor: None,
            });
        }
    }

    pub(super) fn record_link(&mut self, target: &str, internal: bool) {
        if target.is_empty() {
            return;
        }
        let id = self.node_for(target, internal, false);
        self.add_edge(id, "links_to");
    }

    /// An image is a relation to another file and an embedded reference; it is not fetched.
    pub(super) fn record_image(&mut self, target: &str, alt: &str) {
        if target.is_empty() {
            return;
        }
        let id = self.node_for(target, is_internal(target), true);
        self.add_edge(id, "embeds");
        let name = if alt.trim().is_empty() {
            target
                .split(['?', '#'])
                .next()
                .and_then(|p| p.rsplit('/').next())
                .filter(|n| !n.is_empty())
                .unwrap_or(target)
                .to_string()
        } else {
            alt.to_string()
        };
        self.embedded.push(Embedded {
            text: None,
            kind: "image".into(),
            name,
            detail: None,
            target: Some(target.to_string()),
        });
    }

    pub(crate) fn add_graph(&mut self, doc: &mut Document, name: &str) {
        if self.edges.is_empty() {
            return;
        }
        let mut nodes = vec![Node {
            id: "self".into(),
            label: doc
                .properties
                .title
                .clone()
                .unwrap_or_else(|| name.to_string()),
            kind: Some("document".into()),
            anchor: None,
        }];
        nodes.append(&mut self.nodes);
        doc.facets.graphs.push(Graph {
            id: "links".into(),
            kind: "link".into(),
            nodes,
            edges: std::mem::take(&mut self.edges),
        });
    }
}

/// True for a target inside this document or a relative path; false for any URL with a scheme.
pub(super) fn is_internal(target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    if target.starts_with('#') {
        return true;
    }
    if target.starts_with("//") {
        return false;
    }
    let before = target.split(['/', '?', '#']).next().unwrap_or("");
    let has_scheme = before.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() >= 2
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    !has_scheme
}
