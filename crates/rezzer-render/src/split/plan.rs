// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Splitter: the document cut into items, expanded and chunked into specs.

use super::*;

impl<'a> Splitter<'a> {
    /// The units on the top level of the content.
    pub(super) fn top_items(&mut self) -> Vec<Item<'a>> {
        let doc = self.doc;
        let body = &doc.body;
        let title = frontmatter::shown_title(doc, self.opts).trim();
        let skip_first = body.len() - content_body(doc, title).len();
        // The blocks that are no unit's: the title, unchosen sheets, the PDF outline (a
        // trailing unit) and the cited footnote definitions.
        // A `Footnotes` section keeps only the definitions no unit cites; with none left it
        // is no unit's either.
        let left_out = |i: usize, b: &Block| {
            i < skip_first
                || self.selection.as_ref().is_some_and(|s| !s.keeps_block(b))
                || is_pdf_outline(doc, b)
                || footnote_label(b).is_some_and(|l| self.footnotes.contains_key(l))
                || footnotes_section(b).is_some()
        };
        let mut runs: Vec<Run> = Vec::new();
        let mut from = None;
        for (i, b) in body.iter().enumerate() {
            match (left_out(i, b), from) {
                (true, Some(f)) => {
                    runs.push(Run::Blocks(f, i));
                    from = None;
                }
                (false, None) => from = Some(i),
                _ => {}
            }
            if let Some(list) = footnotes_section(b)
                && i >= skip_first
            {
                let rest: Vec<Block> = list
                    .iter()
                    .filter(|d| footnote_label(d).is_none_or(|l| !self.footnotes.contains_key(l)))
                    .cloned()
                    .collect();
                if !rest.is_empty() {
                    let Block::Section {
                        kind,
                        title,
                        anchor,
                        ..
                    } = b
                    else {
                        continue;
                    };
                    runs.push(Run::Owned(Block::Section {
                        kind: *kind,
                        title: title.clone(),
                        anchor: anchor.clone(),
                        blocks: rest,
                    }));
                }
            }
        }
        if let Some(f) = from {
            runs.push(Run::Blocks(f, body.len()));
        }
        let start = Item {
            base: "start".into(),
            title: START_TITLE.into(),
            location: START_TITLE.into(),
            family: None,
            pages: None,
            page_names: false,
            wraps: Vec::new(),
            content: Content::Slice {
                list: body,
                from: 0,
                to: 0,
            },
            expand: Expand::Chunk,
            lead: 0,
            heading_only: false,
            rezzer: true,
            last: true,
            top: 0,
            paragraphs: None,
            table: None,
            held: Vec::new(),
        };
        let pages = body.iter().any(|b| page_number(b).is_some());
        let chapters = if pages { chapters(doc) } else { Vec::new() };
        let mut items = Vec::new();
        for run in runs {
            let (from, to) = match run {
                Run::Blocks(from, to) => (from, to),
                Run::Owned(section) => {
                    items.push(Item {
                        base: "footnotes".into(),
                        title: FOOTNOTES_TITLE.into(),
                        location: FOOTNOTES_TITLE.into(),
                        content: Content::Owned(vec![section]),
                        expand: Expand::Text,
                        ..start.clone()
                    });
                    continue;
                }
            };
            if !chapters.is_empty() {
                items.extend(self.chapter_items(body, from, to, &chapters, &start));
            } else if pages {
                items.extend(self.page_items(body, from, to, &start));
            } else {
                items.extend(self.items_of(body, from, to, &[], &start, true));
            }
        }
        for (k, item) in items.iter_mut().enumerate() {
            item.top = k;
        }
        items
    }

    /// Pages grouped into outline chapters: each run of blocks from a chapter's first page to
    /// the next chapter's.
    fn chapter_items(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        chapters: &[(u32, &'a str)],
        start: &Item<'a>,
    ) -> Vec<Item<'a>> {
        // The chapter of each block: the last one whose first page is at or before the
        // block's page; blocks between pages go with the page before.
        let mut out: Vec<Item<'a>> = Vec::new();
        let mut current: Option<usize> = None;
        let mut group_from = from;
        let mut pages: Option<(u32, u32)> = None;
        let chapter_of = |n: u32| chapters.iter().rposition(|(p, _)| *p <= n);
        let close = |out: &mut Vec<Item<'a>>,
                     chapter: Option<usize>,
                     a: usize,
                     b: usize,
                     pages: Option<(u32, u32)>| {
            if a >= b {
                return;
            }
            let (base, title) = match chapter {
                Some(k) => (slug(chapters[k].1), capped(chapters[k].1, TITLE_CAP)),
                None => (start.base.clone(), start.title.clone()),
            };
            let location = pages.map_or_else(|| title.clone(), pages_location);
            out.push(Item {
                base,
                title,
                location,
                family: None,
                pages,
                page_names: false,
                wraps: Vec::new(),
                content: Content::Slice {
                    list,
                    from: a,
                    to: b,
                },
                expand: Expand::Group,
                lead: 0,
                heading_only: false,
                rezzer: chapter.is_none(),
                last: true,
                top: start.top,
                paragraphs: None,
                table: None,
                held: Vec::new(),
            });
        };
        for (i, b) in list.iter().enumerate().take(to).skip(from) {
            if let Some(n) = page_number(b) {
                let chapter = chapter_of(n);
                if i > group_from && chapter != current {
                    close(&mut out, current, group_from, i, pages);
                    group_from = i;
                    pages = None;
                }
                current = chapter;
                pages = Some(pages.map_or((n, n), |(a, _)| (a, n)));
            }
        }
        close(&mut out, current, group_from, to, pages);
        out
    }

    /// Pages without an outline: each page a unit `page-<n>`, packed into `pages-<a>-<b>`.
    fn page_items(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        start: &Item<'a>,
    ) -> Vec<Item<'a>> {
        let mut out = Vec::new();
        let mut i = from;
        while i < to {
            if let Some(n) = page_number(&list[i]) {
                out.push(self.page_item(list, i, n, None, &[], true));
                i += 1;
                continue;
            }
            let mut j = i;
            while j < to && page_number(&list[j]).is_none() {
                j += 1;
            }
            out.extend(self.items_of(list, i, j, &[], start, true));
            i = j;
        }
        out
    }

    /// A page section as a unit: `page-<n>` on the top level, else a piece of `parent`.
    fn page_item(
        &mut self,
        list: &'a [Block],
        i: usize,
        n: u32,
        parent: Option<&Item<'a>>,
        wraps: &[Wrap<'a>],
        last: bool,
    ) -> Item<'a> {
        let location = pages_location((n, n));
        // Only pages on the top level, without an outline, are named by their numbers.
        let page_names = parent.is_none();
        let (base, title, family) = match parent {
            Some(p) => {
                let family = self.family(p);
                (p.base.clone(), p.title.clone(), family)
            }
            None => (format!("page-{n}"), location.clone(), None),
        };
        Item {
            base,
            title,
            location,
            family,
            pages: Some((n, n)),
            page_names,
            wraps: wraps.to_vec(),
            content: Content::Slice {
                list,
                from: i,
                to: i + 1,
            },
            expand: Expand::Section,
            lead: 0,
            heading_only: false,
            rezzer: parent.is_some_and(|p| p.rezzer),
            last,
            top: parent.map_or(0, |p| p.top),
            paragraphs: None,
            table: None,
            held: Vec::new(),
        }
    }

    /// The units of `list[from..to]` inside `wraps`: each section on its own, the runs of
    /// other blocks at their headings. Untitled units are pieces of `parent`.
    fn items_of(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Vec<Item<'a>> {
        let mut out = Vec::new();
        let mut i = from;
        while i < to {
            if is_section(&list[i]) {
                let at_end = last && i + 1 == to;
                out.push(self.section_item(list, i, wraps, parent, at_end));
                i += 1;
                continue;
            }
            let mut j = i;
            while j < to && !is_section(&list[j]) {
                j += 1;
            }
            out.extend(self.chunks(list, i, j, wraps, parent, last && j == to));
            i = j;
        }
        out
    }

    fn section_item(
        &mut self,
        list: &'a [Block],
        i: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Item<'a> {
        if let Some(n) = page_number(&list[i]) {
            return self.page_item(list, i, n, Some(parent), wraps, last);
        }
        let title = match &list[i] {
            Block::Section { title, .. } => title.as_deref().filter(|t| !t.trim().is_empty()),
            _ => None,
        };
        self.titled_item(title, list, i, i + 1, wraps, parent, Expand::Section, last)
    }

    /// An item titled `title`, else a piece of `parent`.
    #[allow(clippy::too_many_arguments)]
    fn titled_item(
        &mut self,
        title: Option<&str>,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        expand: Expand,
        last: bool,
    ) -> Item<'a> {
        // A piece of a unit Rezzer titled itself keeps that title.
        let rezzer = title.is_none() && parent.rezzer;
        let (base, title, location, family) = match title {
            Some(t) => {
                let t = capped(t, TITLE_CAP);
                (slug(&t), t.clone(), t, None)
            }
            None => {
                let family = self.family(parent);
                (
                    parent.base.clone(),
                    parent.title.clone(),
                    parent.location.clone(),
                    family,
                )
            }
        };
        Item {
            base,
            title,
            location,
            family,
            // A piece holds pages of its parent; a titled part says where it is by its title.
            pages: parent.pages.filter(|_| family.is_some()),
            page_names: false,
            wraps: wraps.to_vec(),
            content: Content::Slice { list, from, to },
            expand,
            lead: 0,
            heading_only: false,
            rezzer,
            last,
            top: parent.top,
            paragraphs: None,
            table: None,
            held: Vec::new(),
        }
    }

    /// `list[from..to]` (no sections) at its headings of the smallest level; the blocks
    /// before the first such heading are a piece of `parent`. Without headings, blocks.
    fn chunks(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Vec<Item<'a>> {
        let Some(min) = list[from..to].iter().filter_map(heading_level).min() else {
            return self.atoms(list, from, to, wraps, parent, last);
        };
        let mut borders: Vec<usize> = (from..to)
            .filter(|&i| heading_level(&list[i]) == Some(min))
            .collect();
        let mut out = Vec::new();
        if borders.first() != Some(&from) {
            borders.insert(0, from);
        }
        for (k, &a) in borders.iter().enumerate() {
            let b = borders.get(k + 1).copied().unwrap_or(to);
            let title = match &list[a] {
                Block::Heading { inlines, .. } if heading_level(&list[a]) == Some(min) => {
                    let html = self.doc.source.format == FormatId::Markdown;
                    Some(rezzer_model::text::one_line(&heading_words(inlines, html)))
                }
                _ => None,
            };
            let title = title.filter(|t| !t.trim().is_empty());
            out.push(self.titled_item(
                title.as_deref(),
                list,
                a,
                b,
                wraps,
                parent,
                Expand::Chunk,
                last && b == to,
            ));
        }
        out
    }

    /// `list[from..to]` as blocks: a heading or caption with the block after it, a table with
    /// the notes after it. Each is a piece of `parent`.
    fn atoms(
        &mut self,
        list: &'a [Block],
        from: usize,
        to: usize,
        wraps: &[Wrap<'a>],
        parent: &Item<'a>,
        last: bool,
    ) -> Vec<Item<'a>> {
        let mut out = Vec::new();
        let mut i = from;
        // The paragraphs before, counted from the start of the run.
        let mut before = 0;
        while i < to {
            let mut j = i;
            while j + 1 < to && leads(&list[j]) {
                j += 1;
            }
            j += 1;
            if matches!(list[j - 1], Block::Table { .. }) {
                while j < to && is_note(&list[j]) {
                    j += 1;
                }
            }
            let mut item = self.titled_item(
                None,
                list,
                i,
                j,
                wraps,
                parent,
                Expand::Atom,
                last && j == to,
            );
            let held = list[i..j]
                .iter()
                .filter(|b| matches!(b, Block::Paragraph { .. }))
                .count();
            if held > 0 {
                item.paragraphs = Some((before + 1, before + held));
                before += held;
            }
            out.push(item);
            i = j;
        }
        out
    }

    // -----------------------------------------------------------------------------------
    // Planning

    /// Packs `items` (one level, in order) into specs, cutting those over the limit further.
    pub(super) fn plan(&mut self, items: Vec<Item<'a>>, out: &mut Vec<Spec<'a>>) {
        let mut costs: Vec<usize> = items.iter().map(|item| self.cost(item)).collect();
        let mut items: Vec<Option<Item<'a>>> = items.into_iter().map(Some).collect();
        let mut i = 0;
        while i < items.len() {
            let Some(item) = items[i].take() else {
                i += 1;
                continue;
            };
            // A column profile that went with the first rows of its table.
            if self.all_moved(&item) {
                i += 1;
                continue;
            }
            if costs[i] > self.budget {
                self.expand(item, out);
                i += 1;
                continue;
            }
            // A heading alone that cannot be packed with the item after it goes into that
            // item, so no part holds only a heading.
            if item.heading_only
                && let Some(Some(next)) = items.get_mut(i + 1)
                && next.expand == Expand::Chunk
                && packs(&item, next)
                && costs[i] + 2 + costs[i + 1] > self.budget
                && let (
                    Content::Slice { from, to, .. },
                    Content::Slice {
                        from: next_from, ..
                    },
                ) = (&item.content, &mut next.content)
            {
                next.lead += to - from;
                *next_from = *from;
                costs[i + 1] = self.cost(next);
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let mut sum = costs[i];
            let mut prev = &item;
            while j < items.len() {
                let Some(next) = items[j].as_ref() else {
                    break;
                };
                if costs[j] > self.budget || !packs(prev, next) || sum + 2 + costs[j] > self.budget
                {
                    break;
                }
                sum += 2 + costs[j];
                prev = next;
                j += 1;
            }
            if j == i + 1 {
                out.push(measured(item, costs[i]));
                i += 1;
                continue;
            }
            let group: Vec<Item<'a>> = std::iter::once(item)
                .chain(items[i + 1..j].iter_mut().filter_map(Option::take))
                .collect();
            let packed = merge(&group);
            let cost = self.cost(&packed);
            if cost <= self.budget {
                out.push(measured(packed, cost));
            } else {
                out.extend(group.into_iter().map(spec));
            }
            i = j;
        }
    }

    fn expand(&mut self, item: Item<'a>, out: &mut Vec<Spec<'a>>) {
        let Content::Slice { list, from, to } = item.content else {
            out.push(cut_spec(item, self));
            return;
        };
        let mut children = match item.expand {
            Expand::Group => {
                let mut children = Vec::new();
                for i in from..to {
                    let at_end = item.last && i + 1 == to;
                    match page_number(&list[i]) {
                        Some(n) => children.push(self.page_item(
                            list,
                            i,
                            n,
                            Some(&item),
                            &item.wraps,
                            at_end,
                        )),
                        None => children.extend(self.items_of(
                            list,
                            i,
                            i + 1,
                            &item.wraps,
                            &item,
                            at_end,
                        )),
                    }
                }
                children
            }
            Expand::Section => match &list[from] {
                Block::Section {
                    kind,
                    title,
                    anchor,
                    blocks: inner,
                } if item.wraps.len() < MAX_DEPTH => {
                    let mut wraps = item.wraps.clone();
                    wraps.push(Wrap {
                        kind: *kind,
                        title: title.as_deref(),
                        anchor: anchor.as_ref().map(|a| a.0.as_str()),
                    });
                    let mut parent = item.clone();
                    parent.family = self.family(&item);
                    self.items_of(inner, 0, inner.len(), &wraps, &parent, item.last)
                }
                _ => Vec::new(),
            },
            Expand::Chunk => self.chunk_children(&item, list, from, to),
            Expand::Atom => {
                self.rows(item, list, from, to, out);
                return;
            }
            Expand::Text => Vec::new(),
        };
        // A section that holds only a section: go down to it without measuring again.
        if item.expand == Expand::Section
            && children.len() == 1
            && children[0].expand == Expand::Section
            && let Some(child) = children.pop()
        {
            self.expand(child, out);
            return;
        }
        // A border that gives nothing finer is cut as text.
        if children.is_empty() || (children.len() == 1 && children[0].expand == item.expand) {
            out.push(cut_spec(item, self));
            return;
        }
        self.plan(children, out);
    }

    /// The parts of a heading chunk: its blocks at the next headings, the heading with the
    /// first; without more headings, its blocks.
    fn chunk_children(
        &mut self,
        item: &Item<'a>,
        list: &'a [Block],
        from: usize,
        to: usize,
    ) -> Vec<Item<'a>> {
        let mut parent = item.clone();
        parent.family = self.family(item);
        parent.lead = 0;
        // The headings it took from a part around it stay in front of its own.
        let own = (from + item.lead).min(to);
        let head = own < to && heading_level(&list[own]).is_some();
        let rest = if head { own + 1 } else { own };
        let deeper = list[rest..to].iter().any(|b| heading_level(b).is_some());
        if !deeper {
            return self.atoms(list, from, to, &item.wraps, &parent, item.last);
        }
        let mut children = self.chunks(list, rest, to, &item.wraps, &parent, item.last);
        if rest > from {
            match children.first_mut() {
                // The blocks before the first heading are a piece already: the headings join
                // them.
                Some(first) if first.family == parent.family => {
                    if let Content::Slice { from: f, .. } = &mut first.content {
                        *f = from;
                    }
                }
                _ => {
                    let mut heading = self.titled_item(
                        None,
                        list,
                        from,
                        rest,
                        &item.wraps,
                        &parent,
                        Expand::Atom,
                        false,
                    );
                    heading.heading_only = true;
                    children.insert(0, heading);
                }
            }
        }
        children
    }

    /// One block over the limit: a table into row ranges (retried with half the size up to 8
    /// times while a range is over), anything else cut as text. The table's column profile
    /// goes with its first rows when they have room for it, and neighbouring ranges are
    /// joined again while they fit: the row estimate is careful, and a range cut again
    /// leaves a small rest.
    fn rows(
        &mut self,
        item: Item<'a>,
        list: &'a [Block],
        from: usize,
        to: usize,
        out: &mut Vec<Spec<'a>>,
    ) {
        let Some(t) = (from..to).find(|&i| matches!(list[i], Block::Table { .. })) else {
            out.push(cut_spec(item, self));
            return;
        };
        let (lead, table, tail) = (&list[from..t], &list[t], &list[t + 1..to]);
        // Small ranges first, a quarter of the part: joined again below while they fit, they
        // fill the parts better than the careful row estimate does on its own.
        let mut max = (self.budget / 16).max(1);
        let costs_of = |this: &Self, ranges: &[rezzer_model::RowRange]| -> Vec<usize> {
            ranges
                .iter()
                .map(|r| this.blocks_cost(&item, r.blocks.clone()))
                .collect()
        };
        let fit = |costs: &[usize]| costs.iter().all(|c| *c <= self.budget);
        let mut ranges = split_rows(lead, table, tail, max);
        let mut costs = costs_of(self, &ranges);
        // Only a range over the limit is cut again, with half the size, up to 8 times; the
        // others keep their size. A range of one row over the limit is cut as text.
        let mut tries = 0;
        while !fit(&costs) && tries < ROW_RETRIES && max > 1 {
            tries += 1;
            max = (max / 2).max(1);
            let mut next: Vec<(rezzer_model::RowRange, usize)> = Vec::new();
            let mut changed = false;
            for (range, cost) in ranges.into_iter().zip(costs) {
                if cost <= self.budget || range.first >= range.last {
                    next.push((range, cost));
                    continue;
                }
                let sub = resplit(&range, max);
                if sub.len() <= 1 {
                    next.push((range, cost));
                    continue;
                }
                changed = true;
                let sub_costs = costs_of(self, &sub);
                next.extend(sub.into_iter().zip(sub_costs));
            }
            (ranges, costs) = next.into_iter().unzip();
            if !changed {
                break;
            }
        }
        if ranges.len() == 1 {
            out.push(cut_spec(item, self));
            return;
        }
        // The column profile describes every row: it goes with the first ones, when they
        // have room for it, and its own place stays empty.
        let mut profile = None;
        if let Some(p) = column_profile(list, t, table)
            && costs[0] <= self.budget
        {
            let cost = self.blocks_cost(&item, with_profile(&ranges[0], Some(p)));
            if cost <= self.budget {
                profile = Some(p);
                costs[0] = cost;
            }
        }
        // Neighbours are joined while they fit: their sizes, less the lead and header band
        // that each one after the first repeats, decide a run. One render confirms it; a run
        // over the limit gives back its last range until it fits.
        let fixed = self.blocks_cost(&item, header_only(&ranges[0]));
        let mut kept: Vec<(rezzer_model::RowRange, usize)> = Vec::with_capacity(ranges.len());
        let mut run: Vec<(rezzer_model::RowRange, usize)> = Vec::new();
        let mut sum = 0usize;
        for (range, cost) in ranges.into_iter().zip(costs) {
            let more = sum.saturating_add(cost.saturating_sub(fixed));
            if !run.is_empty() && (cost > self.budget || sum > self.budget || more > self.budget) {
                self.join_run(&item, std::mem::take(&mut run), profile, &mut kept);
            }
            sum = if run.is_empty() { cost } else { more };
            run.push((range, cost));
        }
        self.join_run(&item, run, profile, &mut kept);
        if kept.len() == 1 {
            out.push(cut_spec(item, self));
            return;
        }
        if let Some(p) = profile {
            self.moved.insert(std::ptr::from_ref(p));
        }
        let key = table_key(list, table);
        let sheet = item
            .wraps
            .last()
            .filter(|w| w.kind == SectionKind::Sheet)
            .and_then(|w| w.title);
        let count = kept.len();
        for (k, (range, cost)) in kept.into_iter().enumerate() {
            let data = range.table.and_then(|i| match &range.blocks[i] {
                Block::Table {
                    layout: Some(layout),
                    ..
                } => layout.data.clone(),
                _ => None,
            });
            let location = match (sheet, data) {
                (Some(sheet), Some(data)) => format!("{sheet}!{data}"),
                _ if range.first == range.last => format!("{}, row {}", item.location, range.first),
                _ => format!("{}, rows {} to {}", item.location, range.first, range.last),
            };
            let location = capped(&location, TITLE_CAP);
            let suffix = format!("{key}-rows-{}-{}", range.first, range.last);
            let blocks = with_profile(&range, profile.filter(|_| k == 0));
            let row_item = Item {
                base: item.base.clone(),
                title: location.clone(),
                location,
                family: None,
                pages: item.pages,
                page_names: false,
                wraps: item.wraps.clone(),
                content: Content::Owned(blocks),
                expand: Expand::Text,
                lead: 0,
                heading_only: false,
                rezzer: false,
                last: item.last && k + 1 == count,
                top: item.top,
                paragraphs: None,
                table: Some(item.title.clone()),
                held: Vec::new(),
            };
            let mut s = if cost > self.budget {
                cut_spec(row_item, self)
            } else {
                measured(row_item, cost)
            };
            s.suffix = suffix;
            if k == 0 {
                s.anchors = vec![(list, from, to)];
            }
            out.push(s);
        }
    }

    /// The ranges of `run` joined into as few as fit, in order, onto `kept`: the longest
    /// start of the run that fits by one render, then the rest the same way. The very first
    /// range of the table carries `profile`.
    fn join_run(
        &self,
        item: &Item<'a>,
        run: Vec<(rezzer_model::RowRange, usize)>,
        profile: Option<&Block>,
        kept: &mut Vec<(rezzer_model::RowRange, usize)>,
    ) {
        let mut i = 0;
        while i < run.len() {
            let mut n = run.len() - i;
            while n > 1 {
                let joined = run[i + 1..i + n]
                    .iter()
                    .try_fold(run[i].0.clone(), |acc, (r, _)| {
                        rezzer_model::join_ranges(&acc, r)
                    });
                if let Some(joined) = joined {
                    let first = kept.is_empty();
                    let cost =
                        self.blocks_cost(item, with_profile(&joined, profile.filter(|_| first)));
                    if cost <= self.budget {
                        kept.push((joined, cost));
                        break;
                    }
                }
                n -= 1;
            }
            if n == 1 {
                kept.push(run[i].clone());
            }
            i += n;
        }
    }
}
