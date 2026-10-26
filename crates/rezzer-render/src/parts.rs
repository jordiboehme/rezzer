// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The files of a split document (spec 5 and 7, decisions 10 and 13): a main document with
//! the description, the stats, the extracts and the list of the parts, and one file per
//! part with its own frontmatter and a `## Relations` section.
//!
//! The main document holds, in this order: `# <title>`, the description, `## Stats` (the
//! counts, zero left out), `## Overview`, `## Outline` (the top two levels), `## Tables`,
//! `## Form`, `## Dates and people`, `## Not read` and `## Parts`. Each list holds at most
//! 20 entries and then says how many more there are. When the main document is over the
//! part size, the lists hold 10, then 5. When it is still over, the list of the parts moves
//! to `contents` parts and the main document links the first one, with the lists at 20,
//! 10, then 5 again. As a last resort the other lists are left out and a line says so, and
//! the title and the description are cut.
//!
//! Nothing the main document shortens is in no file (P5-V): when `## Overview` drops rows,
//! facts or arrows, cuts a text, or is left out, it links the `overview` part, which holds
//! the whole overview; when `## Not read` drops losses or cuts a text, it links the
//! `not-read` part, which holds the whole list. These parts and the `contents` parts are
//! parts like the others: they follow the content's parts in the order `overview`,
//! `not-read`, `contents`, each in the `follows` chain. A `contents` part lists every part
//! but the `contents` parts, and each one but the last links the next.
//!
//! A part file: the frontmatter with the part keys, `# <part title>`, the unit, then
//! `- part_of [<document title>](../<main name>)` and, from the second part on,
//! `- follows [<title of the part before>](<its file>.md)`. Crystalline 0.29.0 reads these
//! lines as relations and no other line of ours.

use rezzer_model::{Block, Completeness, Document, Inline, Loss};

use crate::blocks;
use crate::bullets;
use crate::describe::{self, Counts, Measure, body_tokens, counted};
use crate::split::{
    self, LIST_TITLE, NOT_READ_TITLE, Names, OVERVIEW_TITLE, TITLE_CAP, Unit, capped, family_title,
    part_budget, part_measure, pieces,
};
use crate::{
    OutputSection, PartMeta, RenderOptions, Selection, chosen_sheets, facets, frontmatter,
    overview, render,
};

/// What Rezzer writes for one document: the main document and its parts. A document that
/// fits has no parts, and its main document is the whole render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The file at `<main name>`.
    pub main: String,
    /// The parts in order; empty when the document fits in one file.
    pub parts: Vec<PartFile>,
}

/// One part file of a split document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFile {
    /// Where it is written, next to the main document: `<bundle>/<file>.md`, with `bundle`
    /// the parts folder that [`render_output`] gets (`<stem>.parts` when the pipeline writes).
    pub path: String,
    /// Its title: `<document title>: <unit title or location>`.
    pub title: String,
    /// Where it lies in the document, for example `pages 12 to 20` or `Data!A1:F400`.
    pub location: String,
    /// The whole file: frontmatter, title, body and relations.
    pub markdown: String,
}

/// The list sizes of the main document, tried in this order (decision 13).
const CAPS: [usize; 3] = [20, 10, 5];

/// The description in the main document of the last resort is cut to this many characters.
const DESCRIPTION_CAP: usize = 500;

/// Room a `contents` part keeps for the line that links the next one.
const GOES_ON_ROOM: usize = 128;

/// The whole texts the main document may shorten, cut into part pieces.
struct Trailing<'t> {
    overview: Option<&'t str>,
    overview_pieces: Vec<String>,
    not_read_pieces: Vec<String>,
}

/// Renders `doc` as one file when its body fits `part_size` tokens (or `part_size` is 0),
/// else as a main document and parts (spec 5). `main_name` is the main document's file name
/// with `.md`; `bundle` is the folder next to it that holds the parts.
pub fn render_output(
    doc: &Document,
    completeness: Completeness,
    opts: &RenderOptions,
    part_size: usize,
    main_name: &str,
    bundle: &str,
) -> Output {
    let Some(units) = split::split(doc, opts, part_size, bundle) else {
        return Output {
            main: render(doc, completeness, opts),
            parts: Vec::new(),
        };
    };
    let selection = chosen_sheets(opts).map(|titles| Selection::of(doc, titles));
    let writer = Writer {
        doc,
        completeness,
        opts,
        part_size,
        main_name,
        bundle,
        doc_title: capped(frontmatter::title(doc), TITLE_CAP),
        budget: part_budget(doc, opts, part_size),
        measure: part_measure(bundle),
        counts: describe::counts(doc),
        selection,
    };
    writer.write(units)
}

/// One part as planned: its file name without `.md`, title, location and body.
#[derive(Debug, Clone)]
struct Entry {
    file: String,
    title: String,
    location: String,
    markdown: String,
}

/// How the main document is written: how long its lists are, whether the part list is in
/// `contents` parts, and whether it is the last resort.
#[derive(Debug, Clone, Copy)]
struct Shape {
    cap: usize,
    contents: bool,
    last_resort: bool,
}

struct Writer<'a> {
    doc: &'a Document,
    completeness: Completeness,
    opts: &'a RenderOptions,
    part_size: usize,
    main_name: &'a str,
    bundle: &'a str,
    /// The document title as part titles and `part_of` show it.
    doc_title: String,
    /// What a `not-read` or `contents` part may hold, by `measure` (F12).
    budget: usize,
    measure: Measure,
    counts: Counts,
    selection: Option<Selection<'a>>,
}

impl Writer<'_> {
    fn want(&self, section: OutputSection) -> bool {
        self.opts
            .sections
            .as_ref()
            .is_none_or(|l| l.contains(&section))
    }

    /// `<document title>: <title>`.
    fn part_title(&self, title: &str) -> String {
        format!("{}: {title}", self.doc_title)
    }

    /// The content's entries, their titles made final for the parts the main document adds
    /// (`extra`: `Overview`, `Not read`, `List of parts`), so no title is used twice.
    fn content(&self, units: &[Unit], extra: &[&str]) -> Vec<Entry> {
        let mut units = units.to_vec();
        split::finish_titles(&mut units, extra);
        units
            .into_iter()
            .map(|u| Entry {
                title: self.part_title(&u.title),
                file: u.file,
                location: u.location,
                markdown: u.markdown,
            })
            .collect()
    }

    fn write(&self, units: Vec<Unit>) -> Output {
        let losses = &self.doc.losses;
        let not_read_pieces = if self.want(OutputSection::NotRead) && !losses.is_empty() {
            pieces(&facets::not_read_list(losses), self.budget, self.measure)
        } else {
            Vec::new()
        };
        let overview = self
            .want(OutputSection::Overview)
            .then(|| overview::render_whole(self.doc, self.selection.as_ref()))
            .flatten();
        let trailing = Trailing {
            overview: overview.as_deref(),
            overview_pieces: overview
                .as_deref()
                .map(|text| pieces(text, self.budget, self.measure))
                .unwrap_or_default(),
            not_read_pieces,
        };
        let note_path_len = self
            .main_name
            .strip_suffix(".md")
            .unwrap_or(self.main_name)
            .chars()
            .count();
        for contents in [false, true] {
            for cap in CAPS {
                let shape = Shape {
                    cap,
                    contents,
                    last_resort: false,
                };
                let (main, content, extra) = self.layout(shape, &units, &trailing);
                if body_tokens(&main, note_path_len) <= self.part_size {
                    return self.finish(main, &content, &extra);
                }
            }
        }
        let last = Shape {
            cap: CAPS[CAPS.len() - 1],
            contents: true,
            last_resort: true,
        };
        let (main, content, extra) = self.layout(last, &units, &trailing);
        self.finish(main, &content, &extra)
    }

    /// The main document for `shape` and the parts that follow the content's: `overview`
    /// and `not-read` when the main document shortens them (P5-V), `contents` when the list
    /// of the parts moves there.
    fn layout(
        &self,
        shape: Shape,
        units: &[Unit],
        trailing: &Trailing<'_>,
    ) -> (String, Vec<Entry>, Vec<Entry>) {
        let mut names = Names::taken(units.iter().map(|u| u.file.as_str()));
        let mut extra: Vec<Entry> = Vec::new();
        let overview = self.overview_section(shape, trailing.overview);
        let overview_cut = overview.as_ref().is_some_and(|(_, cut)| *cut);
        let not_read = self.not_read_section(shape);
        let not_read_cut = not_read.as_ref().is_some_and(|(_, cut)| *cut);
        let own: Vec<&str> = [
            (overview_cut, OVERVIEW_TITLE),
            (not_read_cut, NOT_READ_TITLE),
            (shape.contents, LIST_TITLE),
        ]
        .into_iter()
        .filter_map(|(written, title)| written.then_some(title))
        .collect();
        let content = self.content(units, &own);
        if overview_cut {
            extra.extend(self.family(
                &trailing.overview_pieces,
                "overview",
                OVERVIEW_TITLE,
                &mut names,
            ));
        }
        if not_read_cut {
            extra.extend(self.family(
                &trailing.not_read_pieces,
                "not-read",
                NOT_READ_TITLE,
                &mut names,
            ));
        }
        let listed: Vec<&Entry> = content.iter().chain(&extra).collect();
        let contents = if shape.contents {
            self.contents(&listed, &mut names)
        } else {
            Vec::new()
        };
        let parts = self.parts_section(&listed, contents.len());
        let count = listed.len() + contents.len();
        let main = self.main(
            shape,
            count,
            overview.map(|(text, _)| text),
            not_read.map(|(text, _)| text),
            parts,
        );
        extra.extend(contents);
        (main, content, extra)
    }

    /// The parts of a trailing list cut into `list`: `<base>`, then `<base>-2` and so on
    /// where those are free, titled `<title>`, `<title>, part 2` and located `<title>`.
    fn family(&self, list: &[String], base: &str, title: &str, names: &mut Names) -> Vec<Entry> {
        list.iter()
            .enumerate()
            .map(|(k, piece)| Entry {
                // The bare name is reserved for this part, so no unit has it.
                file: if k == 0 {
                    base.to_string()
                } else {
                    names.take(base, "", k + 1)
                },
                title: self.part_title(&family_title(title, k + 1)),
                location: title.to_string(),
                markdown: piece.clone(),
            })
            .collect()
    }

    /// The `contents` parts: the list of `listed` with links in the same folder, each piece
    /// but the last ending with a link to the next one.
    fn contents(&self, listed: &[&Entry], names: &mut Names) -> Vec<Entry> {
        let text = format!("## Parts\n\n{}", list_lines(listed, ""));
        let budget = self
            .budget
            .saturating_sub(GOES_ON_ROOM)
            .max(self.budget / 2);
        let list = pieces(&text, budget, self.measure);
        let mut parts = self.family(&list, "contents", LIST_TITLE, names);
        for k in 1..parts.len() {
            let goes_on = format!(
                "\n\nThe list goes on in [{}]({}).",
                label(&family_title(LIST_TITLE, k + 1), false),
                destination(&format!("{}.md", parts[k].file))
            );
            parts[k - 1].markdown.push_str(&goes_on);
        }
        parts
    }

    /// Every part file, in order, with its relations: the content's parts, then `extra`.
    fn finish(&self, main: String, content: &[Entry], extra: &[Entry]) -> Output {
        let listed: Vec<&Entry> = content.iter().chain(extra).collect();
        let count = listed.len();
        let part_of = format!(
            "- part_of [{}]({})\n",
            label(&self.doc_title, false),
            destination(&format!("../{}", self.main_name))
        );
        let parts = listed
            .iter()
            .enumerate()
            .map(|(k, e)| {
                let mut opts = self.opts.clone();
                opts.parts = None;
                opts.part = Some(PartMeta {
                    index: k + 1,
                    count,
                    location: e.location.clone(),
                    title: e.title.clone(),
                });
                let mut md = frontmatter::render(self.doc, self.completeness, &opts);
                md.push('\n');
                md.push_str(&blocks::heading_line(1, &label(&e.title, false)));
                md.push_str("\n\n");
                // Every body follows P5-S and P5-X; the relations below stay `- ` bullets.
                md.push_str(&bullets::guard(&e.markdown));
                md.push_str("\n\n## Relations\n\n");
                md.push_str(&part_of);
                if let Some(before) = k.checked_sub(1).map(|b| listed[b]) {
                    md.push_str(&format!(
                        "- follows [{}]({})\n",
                        label(&before.title, false),
                        destination(&format!("{}.md", before.file))
                    ));
                }
                PartFile {
                    path: format!("{}/{}.md", self.bundle, e.file),
                    title: e.title.clone(),
                    location: e.location.clone(),
                    markdown: md,
                }
            })
            .collect();
        Output { main, parts }
    }

    /// The main document for `shape`, with `count` parts in all and its `## Overview`,
    /// `## Not read` and `## Parts` sections.
    fn main(
        &self,
        shape: Shape,
        count: usize,
        overview: Option<String>,
        not_read: Option<String>,
        parts: String,
    ) -> String {
        let mut opts = self.opts.clone();
        opts.part = None;
        opts.parts = Some(count);
        let mut out = frontmatter::render(self.doc, self.completeness, &opts);
        let title = frontmatter::title(self.doc).trim();
        let title = if shape.last_resort {
            capped(title, TITLE_CAP)
        } else {
            title.to_string()
        };
        out.push('\n');
        out.push_str(&blocks::heading_line(1, &blocks::line_text(&title)));
        out.push('\n');

        let mut sections: Vec<String> = Vec::new();
        let description = frontmatter::shown_description(self.doc, &opts);
        let description = if shape.last_resort {
            capped(&description, DESCRIPTION_CAP)
        } else {
            description
        };
        sections.push(blocks::paragraph(&[Inline::plain(description)]));
        sections.extend(stats(&self.counts));
        sections.extend(overview);
        // The extracts, each with the name the last resort gives when it leaves it out.
        let extracts: [(&str, Option<String>); 4] = [
            ("Outline", self.outline(shape.cap)),
            ("Tables", self.tables(shape.cap)),
            ("Form", self.form()),
            ("Dates and people", self.dates_and_people(shape.cap)),
        ];
        let mut left_out: Vec<&str> = Vec::new();
        for (name, text) in extracts {
            match text {
                Some(_) if shape.last_resort => left_out.push(name),
                Some(text) => sections.push(text),
                None => {}
            }
        }
        sections.extend(not_read);
        sections.push(parts);
        if !left_out.is_empty() {
            sections.push(format!(
                "These lists are left out to keep this file within the size limit: {}.",
                left_out.join(", ")
            ));
        }
        for section in sections.iter().filter(|s| !s.is_empty()) {
            out.push('\n');
            out.push_str(&bullets::guard(section));
            out.push('\n');
        }
        out
    }

    /// `## Overview` for `shape` (`full` is the whole one), and whether it is shortened: rows,
    /// facts or arrows past the cap, a text cut, or the whole section at the last resort. A
    /// shortened overview links the `overview` part, which holds it whole.
    fn overview_section(&self, shape: Shape, full: Option<&str>) -> Option<(String, bool)> {
        let full = full?;
        let link = format!(
            "[Overview]({})",
            destination(&format!("{}/overview.md", self.bundle))
        );
        if shape.last_resort {
            return Some((format!("## Overview\n\nThe overview is in {link}."), true));
        }
        let text = overview::render_capped(self.doc, self.selection.as_ref(), shape.cap)?;
        if text == full {
            return Some((text, false));
        }
        Some((format!("{text}\n\nThe whole overview is in {link}."), true))
    }

    /// `## Outline`: the top two levels of the outline, at most `cap` entries.
    fn outline(&self, cap: usize) -> Option<String> {
        let entries: Vec<_> = self
            .doc
            .facets
            .outline
            .iter()
            .filter(|e| e.level <= 2)
            .collect();
        if entries.is_empty() {
            return None;
        }
        let mut lines: Vec<String> = entries
            .iter()
            .take(cap)
            .map(|e| {
                let indent = if e.level >= 2 { "  " } else { "" };
                let page = e.page.map(|p| format!(", page {p}")).unwrap_or_default();
                format!("{indent}- {}{page}", item(&e.title))
            })
            .collect();
        more(&mut lines, entries.len(), cap);
        Some(format!("## Outline\n\n{}", lines.join("\n")))
    }

    /// `## Tables`: the tables the stats count, at most `cap`, each with its key or number
    /// and its range or page.
    fn tables(&self, cap: usize) -> Option<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut total = 0usize;
        describe::each_table(&self.doc.body, &mut |table, sheet, page| {
            total += 1;
            if lines.len() >= cap {
                return;
            }
            let layout = match table {
                Block::Table { layout, .. } => layout.as_deref(),
                _ => None,
            };
            let (name, place) = match layout {
                Some(l) => {
                    let range = match sheet {
                        Some(sheet) => format!("{sheet}!{}", l.range),
                        None => l.range.clone(),
                    };
                    (l.key.clone(), Some(range))
                }
                None => (format!("Table {total}"), page.map(|p| format!("page {p}"))),
            };
            let place = place
                .map(|p| format!(" {}", blocks::line_text(&one_line(&p))))
                .unwrap_or_default();
            lines.push(format!("- {}{place}", item(&name)));
        });
        if lines.is_empty() {
            return None;
        }
        more(&mut lines, total, cap);
        Some(format!("## Tables\n\n{}", lines.join("\n")))
    }

    /// `## Form`: how many fields, how many of them are calculated, how many signatures.
    fn form(&self) -> Option<String> {
        let fields = &self.doc.facets.fields;
        if fields.is_empty() {
            return None;
        }
        let calculated = fields
            .iter()
            .filter(|f| {
                f.calc_order.is_some() || f.actions.iter().any(|a| a.trigger == "calculate")
            })
            .count();
        let mut named = vec![counted(fields.len(), "form field", "form fields")];
        if calculated > 0 {
            named.push(format!("{calculated} calculated"));
        }
        if self.counts.signatures > 0 {
            named.push(counted(self.counts.signatures, "signature", "signatures"));
        }
        Some(format!("## Form\n\n{}.", named.join(", ")))
    }

    /// `## Dates and people`: the authors (at most `cap`), the created and modified dates.
    fn dates_and_people(&self, cap: usize) -> Option<String> {
        let p = &self.doc.properties;
        let mut lines: Vec<String> = Vec::new();
        if !p.authors.is_empty() {
            let names: Vec<String> = p
                .authors
                .iter()
                .take(cap)
                .map(|a| blocks::line_text(&one_line(&capped(a, TITLE_CAP))))
                .collect();
            let mut text = names.join(", ");
            if p.authors.len() > cap {
                text.push_str(&format!(" and {} more", p.authors.len() - cap));
            }
            lines.push(format!("- Authors: {text}"));
        }
        for (name, value) in [("Created", &p.created), ("Modified", &p.modified)] {
            if let Some(value) = value {
                let text = capped(&facets::value_text(value), TITLE_CAP);
                lines.push(format!("- {name}: {}", blocks::line_text(&one_line(&text))));
            }
        }
        (!lines.is_empty()).then(|| format!("## Dates and people\n\n{}", lines.join("\n")))
    }

    /// `## Not read` for `shape`, and whether it is shortened: losses past the cap, a text
    /// cut, or the list at the last resort. A shortened list links the `not-read` part, which
    /// holds it whole (P5-V).
    fn not_read_section(&self, shape: Shape) -> Option<(String, bool)> {
        let losses = &self.doc.losses;
        if !self.want(OutputSection::NotRead) || losses.is_empty() {
            return None;
        }
        let link = format!(
            "[Not read]({})",
            destination(&format!("{}/not-read.md", self.bundle))
        );
        if shape.last_resort {
            return Some((
                format!(
                    "## Not read\n\nThe list of the {} entries is in {link}.",
                    losses.len()
                ),
                true,
            ));
        }
        let mut text_cut = false;
        let shown: Vec<Loss> = losses
            .iter()
            .take(shape.cap)
            .map(|l| {
                let location = capped(&l.location, TITLE_CAP);
                let what = capped(&l.what, TITLE_CAP);
                text_cut |= location != l.location || what != l.what;
                Loss {
                    location,
                    what,
                    ..l.clone()
                }
            })
            .collect();
        let mut text = facets::not_read_list(&shown);
        let more = losses.len() - shown.len();
        if more > 0 {
            text.push_str(&format!("\n\nAnd {more} more in {link}."));
        } else if text_cut {
            text.push_str(&format!("\n\nThe whole texts are in {link}."));
        }
        Some((text, more > 0 || text_cut))
    }

    /// `## Parts`: every part in order, or the link to the first of `contents` parts.
    fn parts_section(&self, listed: &[&Entry], contents: usize) -> String {
        let link = |text: &str| {
            format!(
                "[{text}]({})",
                destination(&format!("{}/contents.md", self.bundle))
            )
        };
        match contents {
            0 => format!(
                "## Parts\n\n{}",
                list_lines(listed, &format!("{}/", self.bundle))
            ),
            1 => format!(
                "## Parts\n\nThe list of all {} parts is in {}.",
                listed.len(),
                link(LIST_TITLE)
            ),
            _ => format!(
                "## Parts\n\nThe list of all {} parts starts in {}.",
                listed.len(),
                link(LIST_TITLE)
            ),
        }
    }
}

/// `## Stats`: the counts in the order of spec 5, zero left out; `None` when all are zero.
fn stats(c: &Counts) -> Option<String> {
    let named: Vec<String> = [
        (c.pages, "page", "pages"),
        (c.sheets, "sheet", "sheets"),
        (c.tables, "table", "tables"),
        (c.formulas, "formula", "formulas"),
        (c.form_fields, "form field", "form fields"),
        (c.signatures, "signature", "signatures"),
        (c.revisions, "revision", "revisions"),
        (
            c.vision,
            "picture that needs vision",
            "pictures that need vision",
        ),
        (c.not_read, "part not read", "parts not read"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| counted(n, one, many))
    .collect();
    (!named.is_empty()).then(|| format!("## Stats\n\n{}.", named.join(", ")))
}

/// `- and <n> more` after `lines` when `total` is over `cap`.
fn more(lines: &mut Vec<String>, total: usize, cap: usize) {
    if total > cap {
        lines.push(format!("- and {} more", total - cap));
    }
}

/// The list of the parts, one `- [<title>](<prefix><file>.md): <location>` line each.
fn list_lines(listed: &[&Entry], prefix: &str) -> String {
    let lines: Vec<String> = listed
        .iter()
        .map(|e| {
            format!(
                "- [{}]({}): {}",
                label(&e.title, true),
                destination(&format!("{prefix}{}.md", e.file)),
                blocks::line_text(&one_line(&e.location))
            )
        })
        .collect();
    lines.join("\n")
}

/// Text from the file on one line: every control character a space.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Text from the file as the start of a list item: cut to [`TITLE_CAP`] characters, on one
/// line, escaped, and with nothing at its start that opens a block.
fn item(text: &str) -> String {
    let mut out = String::new();
    blocks::escape_line_start(
        &blocks::line_text(&one_line(&capped(text, TITLE_CAP))),
        &mut out,
    );
    out
}

/// A title as link text or heading text: every Markdown character escaped with a backslash,
/// so the text is at most twice as long (the part overhead counts on that), and every control
/// character a space. With `bracket_entity`, a `]` is the character reference `&#93;`: in a
/// list item that opens with the link, Crystalline 0.29.0 would read the text up to an
/// escaped `\]` as an observation category.
///
/// A `(` and `)` are escaped too, so `](` never stands inside the text: Crystalline reads a
/// link up to the first `](` it finds, escaped or not.
fn label(text: &str, bracket_entity: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            ']' if bracket_entity => out.push_str("&#93;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '~' | '|' | '&' | '<' | '(' | ')' | '$' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// A relative path as a link destination that CommonMark and Crystalline 0.29.0 read back as
/// written: `%` and `#` encoded as in asset paths, then `<`, `>`, `\`, a backtick, `&`, `[`,
/// `]` and control characters. A path with whitespace of any other kind (Crystalline cuts a
/// bare destination at the first one), `(`, `)` or `<` stands in angle brackets.
fn destination(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for c in path.chars() {
        match c {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '\\' => out.push_str("%5C"),
            '`' => out.push_str("%60"),
            '&' => out.push_str("%26"),
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            c if c.is_control() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    if path
        .chars()
        .any(|c| (c.is_whitespace() && !c.is_control()) || matches!(c, '(' | ')' | '<'))
    {
        format!("<{out}>")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{destination, label};

    #[test]
    fn labels_escape_and_stay_on_one_line() {
        assert_eq!(
            label("a [b](c) \\ `d`", false),
            "a \\[b\\]\\(c\\) \\\\ \\`d\\`"
        );
        assert_eq!(label("x]y\nz", true), "x&#93;y z");
        // Never more than twice as long without the entity.
        let hostile: String = "\\`*_[]~|&<()$".repeat(10);
        assert_eq!(
            label(&hostile, false).chars().count(),
            2 * hostile.chars().count()
        );
    }

    #[test]
    fn destinations_encode_and_bracket() {
        assert_eq!(destination("../a.md"), "../a.md");
        assert_eq!(
            destination("../Q3 report (final) #2.pdf.md"),
            "<../Q3 report (final) %232.pdf.md>"
        );
        assert_eq!(destination("a%b>c\\d`e&f.md"), "a%25b%3Ec%5Cd%60e%26f.md");
        assert_eq!(destination("a<b.md"), "<a%3Cb.md>");
        assert_eq!(destination("a\tb.md"), "a%09b.md");
        assert_eq!(destination("a\u{3000}b.md"), "<a\u{3000}b.md>");
        assert_eq!(destination("a\u{a0}b.md"), "<a\u{a0}b.md>");
        assert_eq!(destination("[[x]].md"), "%5B%5Bx%5D%5D.md");
    }
}
