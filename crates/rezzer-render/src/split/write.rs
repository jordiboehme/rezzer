// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Splitter: writing the specs as units, with trailing notes and logic parts.

use super::*;

impl<'a> Splitter<'a> {
    /// Names the units, renders them with their links and adds the trailing sections.
    pub(super) fn write(&self, specs: Vec<Spec<'a>>) -> Vec<Unit> {
        let mut names = Names::new();
        // Every piece the plan expects is named up front, in document order (P5-T): the
        // anchors lead to a unit's first piece, and the pieces of one unit are numbered in
        // order. A piece more than planned gets the next free name when it is written; a
        // piece fewer leaves its name unused.
        let mut order = 0usize;
        let mut files: Vec<String> = Vec::with_capacity(specs.len());
        let mut further: Vec<Vec<String>> = Vec::with_capacity(specs.len());
        for s in &specs {
            files.push(names.take(&s.item.base, &s.suffix, order + 1));
            further.push(
                (1..s.pieces.max(1))
                    .map(|p| names.take(&s.item.base, &s.suffix, order + p + 1))
                    .collect(),
            );
            order += s.pieces.max(1);
        }
        // The unit of each anchor: the first unit that holds it. A section cut into units
        // leads to the first of them.
        let mut at: HashMap<&'a str, usize> = HashMap::new();
        for (k, s) in specs.iter().enumerate() {
            let wraps = s.item.wraps.iter().filter_map(|w| w.anchor);
            for anchor in wraps.chain(s.wrap_anchors.iter().copied()) {
                at.entry(anchor).or_insert(k);
            }
            for (list, from, to) in anchor_runs(s) {
                collect_anchors(&list[from..to], 0, &mut |anchor| {
                    at.entry(anchor).or_insert(k);
                });
            }
        }
        let frags = fragments(&specs, &at);

        let notes = Notes::new(&self.doc.facets.annotations);
        // One title per unit before its pieces are numbered: a family (a section cut into
        // several units) is one unit, named by its first member.
        let family_of = |k: usize| specs[k].item.family.unwrap_or(usize::MAX - k);
        let mut seen: HashSet<usize> = HashSet::new();
        let firsts: Vec<(usize, usize)> = (0..specs.len())
            .filter(|&k| seen.insert(family_of(k)))
            .map(|k| (family_of(k), k))
            .collect();
        let pairs: Vec<(&str, &str, bool)> = firsts
            .iter()
            .map(|&(_, k)| {
                (
                    specs[k].item.title.as_str(),
                    specs[k].item.location.as_str(),
                    specs[k].item.rezzer,
                )
            })
            .collect();
        let titles: HashMap<usize, String> = firsts
            .iter()
            .map(|&(f, _)| f)
            .zip(unit_titles(&pairs))
            .collect();
        let first_location: HashMap<usize, String> = firsts
            .iter()
            .map(|&(f, k)| (f, specs[k].item.location.clone()))
            .collect();
        let mut units: Vec<Unit> = Vec::new();
        let mut families: HashMap<usize, usize> = HashMap::new();
        for (k, s) in specs.iter().enumerate() {
            let links = Links {
                at: &at,
                files: &files,
                frags: &frags,
                current: k,
            };
            let text = self.render(
                &s.item.wraps,
                &s.item.content,
                s.item.last,
                &notes,
                Some(&links),
            );
            let pieces = self.pieces(&text);
            let lone = pieces.len() == 1 && s.item.family.is_none();
            let family = family_of(k);
            let unit_title = &titles[&family];
            for (p, piece) in pieces.into_iter().enumerate() {
                let file = match p {
                    0 => files[k].clone(),
                    _ => match further[k].get(p - 1) {
                        Some(name) => name.clone(),
                        None => names.take(&s.item.base, &s.suffix, units.len() + 1),
                    },
                };
                let n = families.entry(family).or_insert(0);
                *n += 1;
                let piece_number = if lone { 0 } else { *n };
                units.push(Unit {
                    file,
                    title: family_title(unit_title, piece_number),
                    location: s.item.shown_location(),
                    markdown: piece,
                    naming: Naming {
                        own: s.item.rezzer,
                        base: unit_title.clone(),
                        location: first_location[&family].clone(),
                        piece: piece_number,
                    },
                });
            }
        }
        // The trailing sections are in no unit of the content: every anchor link from them
        // leads to another file.
        let links = Links {
            at: &at,
            files: &files,
            frags: &frags,
            current: usize::MAX,
        };
        for (base, location, text) in self.trailing(&notes, &links) {
            self.push_trailing(&mut units, &mut names, base, location, &text);
        }
        finish_titles(&mut units, &[]);
        units
    }

    /// `text` cut to the budget, each piece without blank lines at its ends.
    pub(super) fn pieces(&self, text: &str) -> Vec<String> {
        pieces(text, self.budget, self.measure)
    }

    /// The trailing sections in order, each with its name and location: properties, logic,
    /// annotations (those the content did not show), changes, embedded and the PDF outline.
    fn trailing(
        &self,
        notes: &Notes<'a>,
        links: &Links<'_>,
    ) -> Vec<(&'static str, &'static str, String)> {
        let selection = self.selection.as_ref();
        let mut out = Vec::new();
        if self.want(OutputSection::Properties)
            && let Some(text) = facets::properties(self.doc)
        {
            out.push(("properties", PROPERTIES_TITLE, text));
        }
        if self.want(OutputSection::Logic)
            && let Some(text) = facets::logic(self.doc, self.opts.math, selection)
        {
            out.push(("logic", LOGIC_TITLE, text));
        }
        if self.want(OutputSection::Annotations) {
            let mut ctx = self.ctx(notes);
            ctx.heading_offset = 0;
            ctx.notes = None;
            let list = &self.doc.facets.annotations;
            let list: Cow<'_, [rezzer_model::Annotation]> =
                if list.iter().any(|a| annotation_needs_rewrite(a, links, 0)) {
                    let mut owned = list.clone();
                    for a in &mut owned {
                        rewrite_annotation(a, links, 0);
                    }
                    Cow::Owned(owned)
                } else {
                    Cow::Borrowed(list)
                };
            if let Some(text) = facets::annotations(&list, &ctx, notes, selection) {
                out.push(("annotations", ANNOTATIONS_TITLE, text));
            }
        }
        if self.want(OutputSection::Changes)
            && let Some(text) = facets::changes(self.doc)
        {
            out.push(("changes", CHANGES_TITLE, text));
        }
        if self.want(OutputSection::Embedded)
            && let Some(text) = facets::embedded(self.doc, "../")
        {
            out.push(("embedded", EMBEDDED_TITLE, text));
        }
        if self.want(OutputSection::Content)
            && let Some(outline) = self.doc.body.iter().find(|b| is_pdf_outline(self.doc, b))
        {
            let ctx = self.ctx(notes);
            let text = blocks::blocks(std::slice::from_ref(outline), &ctx, 0, 2, false);
            if !text.is_empty() {
                out.push(("outline", OUTLINE_TITLE, text));
            }
        }
        out
    }

    /// A trailing section as units. Logic is named per `###` part it starts in
    /// (`logic-<sheet>`), when it does not fit whole.
    fn push_trailing(
        &self,
        units: &mut Vec<Unit>,
        names: &mut Names,
        base: &str,
        location: &str,
        text: &str,
    ) {
        let pieces = self.pieces(text);
        let whole = pieces.len() == 1;
        // The previous piece's file base and location, the `###` part it ends in (empty
        // outside Logic), and how many pieces that part has had.
        let mut prev: Option<(String, String, String)> = None;
        let mut n = 0usize;
        for piece in pieces {
            let part = if whole || base != "logic" {
                None
            } else {
                logic_part(&piece)
            };
            // A piece names every `###` part it holds (ruling P5-AH): the one it starts in,
            // then each one that begins in it.
            let here = |start: &str| {
                let mut held = vec![start.to_string()];
                held.extend(begun_logic_parts(&piece));
                (
                    format!("{base}-{}", slug(start)),
                    capped(&format!("{location}, {}", listed(&held)), TITLE_CAP),
                    start.to_string(),
                )
            };
            let (piece_base, piece_location, start) = match (&part, &prev) {
                (Some(PartStart::Begins(title) | PartStart::Continues(title)), _) => here(title),
                // A piece without a heading of its own goes on in the part before it.
                (None, Some((_, _, before))) if !before.is_empty() => here(before),
                (None, Some((b, l, _))) => (b.clone(), l.clone(), String::new()),
                (None, None) => (base.to_string(), location.to_string(), String::new()),
            };
            let goes_on = !matches!(part, Some(PartStart::Begins(_)))
                && prev.as_ref().is_some_and(|(b, _, _)| *b == piece_base);
            n = if goes_on { n + 1 } else { 1 };
            // The part the next piece goes on in: the last one that begins here, if any.
            let current = begun_logic_parts(&piece).pop().unwrap_or(start);
            prev = Some((piece_base.clone(), piece_location.clone(), current));
            let order = units.len() + 1;
            let title = family_title(&piece_location, n);
            units.push(Unit {
                file: names.take(&piece_base, "", order),
                naming: Naming {
                    own: true,
                    base: title.clone(),
                    location: piece_location.clone(),
                    piece: 0,
                },
                title,
                location: piece_location,
                markdown: piece,
            });
        }
    }
}

/// Where a piece of `## Logic` starts: at a `###` part, or inside one (its heading repeated
/// as continued).
pub(super) enum PartStart {
    Begins(String),
    Continues(String),
}

/// The `###` part a piece of `## Logic` begins in, its title without escapes; `None` when
/// it begins with other text.
pub(super) fn logic_part(piece: &str) -> Option<PartStart> {
    let mut lines = piece.lines().filter(|l| !l.trim().is_empty());
    let mut first = lines.next()?;
    if first == "## Logic" || first == "## Logic (continued)" {
        first = lines.next()?;
    }
    let title = first.strip_prefix("### ")?;
    Some(match title.strip_suffix(" (continued)") {
        Some(t) => PartStart::Continues(unescape(t)),
        None => PartStart::Begins(unescape(title)),
    })
}

/// The titles of the `###` parts that begin in a piece of `## Logic`, in order, without
/// escapes.
fn begun_logic_parts(piece: &str) -> Vec<String> {
    piece
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .filter(|t| !t.ends_with(" (continued)"))
        .map(unescape)
        .collect()
}

/// Names held by one part, as its title shows them (ruling P5-AH): repeats next to each
/// other once, then `A`, `A and B`, `A, B and C`, or the first three and `and <n> more`.
pub(super) fn listed(names: &[String]) -> String {
    let mut held: Vec<&str> = Vec::new();
    for name in names {
        if held.last() != Some(&name.as_str()) {
            held.push(name);
        }
    }
    match held.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, c, rest @ ..] => format!("{a}, {b}, {c} and {} more", rest.len()),
    }
}

/// Rendered line text back as plain text: a backslash before ASCII punctuation goes.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(next) = chars.peek().copied()
            && next.is_ascii_punctuation()
        {
            out.push(next);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}
