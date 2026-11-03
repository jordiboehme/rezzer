// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typers of text regions: each kind from named signals, positive evidence only, the
//! same scores at every scale, and today's decisions ahead of their rivals by the margin.

use layoutregions::file::{FilePage, learn};
use layoutregions::segment::drawn::drawn;
use layoutregions::segment::{SegId, Segment, Source, segment_page};
use layoutregions::typers::table::{refusal, table_columns, type_table};
use layoutregions::typers::text::{
    BAND_RUNNING, HEADING_BOLD, HEADING_SIZE_LARGE, HEADING_TITLE_LINES, LEGEND_ENTRIES,
    LIST_HANGING, LIST_ITEMS, LIST_MARKER, PARAGRAPH_DEMOTED, PARAGRAPH_JOINS, PARAGRAPH_SENTENCES,
    PARAGRAPH_TEXT_SIZE, TITLE_HEADING, TITLE_TOP,
};
use layoutregions::typers::{FileIndex, PageView, TYPERS, Typed, type_figure};
use layoutregions::*;

/// The margin a decision needs over the next kind (the starting margin of the arbitration).
const MARGIN: u16 = 150;

fn run(text: &str, x: f32, y: f32, size: f32, line: u32) -> TextRun {
    TextRun {
        text: text.to_string(),
        x,
        y,
        w: text.chars().count() as f32 * size * 0.5,
        h: size,
        size,
        font: 1,
        font_kind: FontKind::Proportional,
        bold: false,
        italic: false,
        color: None,
        shift: Shift::Base,
        line,
        off_axis: false,
        gaps: Vec::new(),
    }
}

fn bold(r: TextRun) -> TextRun {
    TextRun { bold: true, ..r }
}

fn page(number: u32, texts: Vec<TextRun>) -> PageIn {
    PageIn {
        page: number,
        size: [595.0, 842.0],
        turn: 0,
        texts,
        rules: Vec::new(),
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    }
}

fn no_amount(_: &str) -> bool {
    false
}

/// Four lines of prose in 9.2 points (sizes off the edges of the half-point keys at every
/// scale of the tests) from line `first` on, the first at `y`.
fn prose(first: u32, y: f32) -> Vec<TextRun> {
    [
        "Die Muster GmbH liefert die bestellte Ware an den",
        "Kunden. Erika Mustermann zahlt den Betrag von",
        "1.234,56 innerhalb von zehn Tagen nach dem Erhalt",
        "der Rechnung auf das genannte Konto der Firma.",
    ]
    .iter()
    .enumerate()
    .map(|(k, t)| run(t, 72.0, y - 11.0 * k as f32, 9.2, first + k as u32))
    .collect()
}

/// A page of the file `pages` (number `at`) and the lines of it that make one segment.
struct Case {
    pages: Vec<PageIn>,
    at: usize,
    lines: Vec<u32>,
}

impl Case {
    /// Every page scaled by `k`: places, sizes and the page itself.
    fn scaled(&self, k: f32) -> Case {
        let pages = self
            .pages
            .iter()
            .map(|p| PageIn {
                size: [p.size[0] * k, p.size[1] * k],
                texts: p
                    .texts
                    .iter()
                    .map(|r| TextRun {
                        x: r.x * k,
                        y: r.y * k,
                        w: r.w * k,
                        h: r.h * k,
                        size: r.size * k,
                        gaps: r
                            .gaps
                            .iter()
                            .map(|g| GapIn {
                                from: g.from * k,
                                to: g.to * k,
                                ..*g
                            })
                            .collect(),
                        ..r.clone()
                    })
                    .collect(),
                ..p.clone()
            })
            .collect();
        Case {
            pages,
            at: self.at,
            lines: self.lines.clone(),
        }
    }

    /// What every typer says of the segment.
    fn typed(&self) -> Vec<Typed> {
        let page = &self.pages[self.at];
        let prims: Vec<PrimId> = page
            .texts
            .iter()
            .enumerate()
            .filter(|(_, r)| self.lines.contains(&r.line))
            .map(|(i, _)| PrimId::Text(i as u32))
            .collect();
        self.typed_prims(prims)
    }

    fn typed_prims(&self, prims: Vec<PrimId>) -> Vec<Typed> {
        let file: Vec<FilePage<'_, TextRun>> = self.pages.iter().map(FilePage::of).collect();
        let facts = learn(&file, false, &no_amount);
        let index = FileIndex::new(&facts);
        let page = &self.pages[self.at];
        let structure = drawn(page, &|_| true);
        let segs = segment_page(page, &Limits::default(), &|| false);
        let view = PageView::new(page, &structure, &segs, &facts, &index, &no_amount);
        let seg = Segment {
            id: SegId(u32::MAX),
            bounds: Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: page.size[0],
                y1: page.size[1],
            },
            prims,
            source: Source::Rest,
            parent: None,
            children: Vec::new(),
            alternatives: Vec::new(),
        };
        TYPERS
            .iter()
            .filter_map(|(kind, typer)| {
                let t = typer(&view, &seg)?;
                assert_eq!(t.kind, *kind, "a typer gives its own kind");
                Some(t)
            })
            .collect()
    }
}

fn of(typed: &[Typed], kind: Kind) -> &Typed {
    typed
        .iter()
        .find(|t| t.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} in {typed:#?}"))
}

fn ids(t: &Typed) -> Vec<&'static str> {
    t.signals.iter().map(|s| s.id).collect()
}

/// The best kind of `typed`, and its lead over the next.
fn winner(typed: &[Typed]) -> (Kind, u16) {
    let mut scores: Vec<(u16, Kind)> = typed.iter().map(|t| (t.score.0, t.kind)).collect();
    scores.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let second = scores.get(1).map_or(0, |s| s.0);
    (scores[0].1, scores[0].0 - second)
}

/// A heading of 14.2 points over prose of 9.2 points.
fn heading_case(lines: &[u32]) -> Case {
    let mut texts = vec![run("Einleitung", 72.0, 780.0, 14.2, 0)];
    texts.extend(prose(1, 740.0));
    Case {
        pages: vec![page(2, texts)],
        at: 0,
        lines: lines.to_vec(),
    }
}

/// Prose and a list of two items under it.
fn list_case() -> Case {
    let mut texts = prose(0, 740.0);
    texts.push(run("- Erster Punkt", 72.0, 690.0, 9.2, 4));
    texts.push(run("- Zweiter Punkt", 72.0, 679.0, 9.2, 5));
    Case {
        pages: vec![page(1, texts)],
        at: 0,
        lines: vec![4, 5],
    }
}

/// Three pages of prose with a footer line at one height; the footer of the second page.
fn footer_case() -> Case {
    let pages = (1..=3)
        .map(|n| {
            let mut texts = prose(0, 500.0);
            texts.push(run(&format!("Muster GmbH Seite {n}"), 72.0, 30.0, 9.2, 4));
            page(n, texts)
        })
        .collect();
    Case {
        pages,
        at: 1,
        lines: vec![4],
    }
}

/// Prose with two raised references and the legend of 6.2 points at the foot of the page.
fn legend_case() -> Case {
    let mut texts = vec![
        run("Einheit", 40.0, 720.0, 9.2, 0),
        run(" 2", 72.0, 724.0, 6.2, 0),
        run(" Beitrag", 200.0, 720.0, 9.2, 0),
        run(" 3", 236.0, 724.0, 6.2, 0),
    ];
    texts.extend(prose(1, 690.0));
    texts.push(run("2 Std = Stunden", 40.0, 70.0, 6.2, 5));
    texts.push(run("3 Gegebenenfalls anteilig", 40.0, 62.0, 6.2, 6));
    Case {
        pages: vec![page(2, texts)],
        at: 0,
        lines: vec![5, 6],
    }
}

/// A legend whose markers are a number and a bracket, referenced after words above: its lines
/// also read as list items.
fn paren_legend_case() -> Case {
    let mut texts = vec![run(
        "Der Betrag1) und die Summe2) gelten",
        72.0,
        760.0,
        9.2,
        0,
    )];
    texts.extend(prose(1, 740.0));
    texts.push(run("1) Erste Fussnote zum Betrag", 72.0, 70.0, 6.2, 5));
    texts.push(run("2) Zweite Fussnote zur Summe", 72.0, 62.0, 6.2, 6));
    Case {
        pages: vec![page(2, texts)],
        at: 0,
        lines: vec![5, 6],
    }
}

/// Bold lines of an address a little larger than the prose of 10.2 points under them, which
/// read as a paragraph: their lines are short items of one column.
fn address_case() -> Case {
    let mut texts = vec![
        bold(run("Muster GmbH", 72.0, 780.0, 13.2, 0)),
        bold(run("Musterweg 1", 72.0, 765.0, 13.2, 1)),
        bold(run("12345 Musterstadt", 72.0, 750.0, 13.2, 2)),
    ];
    texts.extend(prose(3, 700.0).into_iter().map(|r| TextRun {
        size: 10.2,
        h: 10.2,
        w: r.text.chars().count() as f32 * 5.1,
        ..r
    }));
    Case {
        pages: vec![page(2, texts)],
        at: 0,
        lines: vec![0, 1, 2],
    }
}

/// The title of a report at the top of page 1, over prose.
fn title_case() -> Case {
    let mut texts = vec![run("Jahresbericht", 72.0, 800.0, 20.2, 0)];
    texts.extend(prose(1, 740.0));
    Case {
        pages: vec![page(1, texts)],
        at: 0,
        lines: vec![0],
    }
}

#[test]
fn heading_has_size_ratio_signal() {
    let typed = heading_case(&[0]).typed();
    let heading = of(&typed, Kind::Heading);
    assert!(ids(heading).contains(&"heading.size_ratio"), "{heading:#?}");
    assert_eq!(winner(&typed).0, Kind::Heading);
}

#[test]
fn prose_lines_are_paragraph() {
    let typed = heading_case(&[1, 2, 3, 4]).typed();
    let paragraph = of(&typed, Kind::Paragraph);
    assert!(
        ids(paragraph).contains(&"paragraph.joins"),
        "{paragraph:#?}"
    );
    assert!(
        typed.iter().all(|t| t.kind == Kind::Paragraph),
        "{typed:#?}"
    );
}

/// One line of body text reads as a paragraph by its size alone.
#[test]
fn a_body_line_is_a_paragraph_by_its_size() {
    let typed = heading_case(&[3]).typed();
    assert_eq!(ids(of(&typed, Kind::Paragraph)), ["paragraph.text_size"]);
    assert_eq!(winner(&typed).0, Kind::Paragraph);
}

#[test]
fn list_marker_is_list_evidence() {
    let typed = list_case().typed();
    let list = of(&typed, Kind::List);
    assert!(ids(list).contains(&"list.marker"), "{list:#?}");
    assert!(ids(list).contains(&"list.items"), "{list:#?}");
    assert!(typed.iter().all(|t| t.kind == Kind::List), "{typed:#?}");
}

#[test]
fn running_footer_is_footer_band() {
    let typed = footer_case().typed();
    let footer = of(&typed, Kind::FooterBand);
    assert_eq!(ids(footer), ["footer_band.running"]);
    // It is set in the size of the body, so it is a paragraph too, far behind.
    of(&typed, Kind::Paragraph);
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::FooterBand);
    assert!(lead >= MARGIN, "{typed:#?}");
}

/// A running header is a header band; a footer of a file of two pages says so.
#[test]
fn running_header_and_two_page_footer() {
    let mut header = footer_case();
    for p in &mut header.pages {
        let last = p.texts.len() - 1;
        p.texts[last].y = 812.0;
    }
    let typed = header.typed();
    assert_eq!(ids(of(&typed, Kind::HeaderBand)), ["header_band.running"]);
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::HeaderBand);
    assert!(lead >= MARGIN, "{typed:#?}");
    let mut two = footer_case();
    two.pages.truncate(2);
    let typed = two.typed();
    assert_eq!(
        ids(of(&typed, Kind::FooterBand)),
        ["footer_band.running", "footer_band.two_page"]
    );
}

#[test]
fn legend_under_raised_references() {
    let typed = legend_case().typed();
    let legend = of(&typed, Kind::FootnoteLegend);
    assert_eq!(
        ids(legend),
        ["footnote_legend.entries", "footnote_legend.raised_refs"]
    );
    assert_eq!(winner(&typed).0, Kind::FootnoteLegend);
}

/// A legend that also reads as a list stays a legend, by the margin.
#[test]
fn a_legend_is_ahead_of_a_list() {
    let typed = paren_legend_case().typed();
    of(&typed, Kind::List);
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::FootnoteLegend, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
}

/// Lines of a heading size that read as an address are a paragraph, by the margin over the
/// heading their size speaks for.
#[test]
fn a_demoted_heading_is_a_paragraph_ahead_of_the_heading() {
    let typed = address_case().typed();
    let heading = of(&typed, Kind::Heading);
    assert!(
        !ids(heading).contains(&"heading.title_lines"),
        "{heading:#?}"
    );
    let paragraph = of(&typed, Kind::Paragraph);
    assert!(
        ids(paragraph).contains(&"paragraph.lines_apart"),
        "{paragraph:#?}"
    );
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Paragraph);
    assert!(lead >= MARGIN, "{typed:#?}");
}

/// The largest heading in the head of page 1 is the title block, ahead of the heading.
#[test]
fn the_title_is_ahead_of_its_heading() {
    let typed = title_case().typed();
    let title = of(&typed, Kind::TitleBlock);
    assert_eq!(
        ids(title),
        ["title_block.largest_heading", "title_block.top_third"]
    );
    assert!(ids(of(&typed, Kind::Heading)).contains(&"heading.title_lines"));
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::TitleBlock);
    assert!(lead >= MARGIN, "{typed:#?}");
    // The same heading on another page is no title.
    let other = Case {
        pages: vec![PageIn {
            page: 2,
            ..title_case().pages[0].clone()
        }],
        ..title_case()
    };
    assert_eq!(winner(&other.typed()).0, Kind::Heading);
}

#[test]
fn no_signal_no_candidate() {
    for case in [
        heading_case(&[0]),
        footer_case(),
        legend_case(),
        title_case(),
    ] {
        assert_eq!(case.typed_prims(Vec::new()), []);
    }
    // A line of white space only is no text either.
    let mut blank = heading_case(&[9]);
    blank.pages[0].texts.push(run("   ", 72.0, 600.0, 9.2, 9));
    assert_eq!(blank.typed(), []);
}

#[test]
fn scores_are_scale_free() {
    let cases = [
        heading_case(&[0]),
        heading_case(&[1, 2, 3, 4]),
        heading_case(&[3]),
        list_case(),
        footer_case(),
        legend_case(),
        paren_legend_case(),
        address_case(),
        title_case(),
    ];
    for case in cases {
        let scores = |c: &Case| -> Vec<(Kind, u16, Vec<&'static str>)> {
            c.typed()
                .iter()
                .map(|t| (t.kind, t.score.0, ids(t)))
                .collect()
        };
        let at = scores(&case);
        assert!(!at.is_empty());
        for k in [0.75, 0.8, 1.3, 1.5] {
            assert_eq!(scores(&case.scaled(k)), at, "at {k}x of {:?}", case.lines);
        }
    }
}

/// A score is the sum of its signals' weights, at most 1000, and no typer gives a kind
/// without a signal.
#[test]
fn a_score_is_the_sum_of_its_signals() {
    for case in [
        heading_case(&[0]),
        list_case(),
        footer_case(),
        legend_case(),
        address_case(),
        title_case(),
    ] {
        for t in case.typed() {
            assert!(!t.signals.is_empty());
            let sum: u32 = t.signals.iter().map(|s| u32::from(s.weight)).sum();
            assert_eq!(u32::from(t.score.0), sum.min(1000), "{t:#?}");
        }
    }
}

/// The worst cases of the kinds today's rules decide between, by the weights alone: each
/// winner's least score is ahead of the loser's best by the margin.
#[test]
fn worst_case_margins_hold_by_the_weights() {
    let heading_kept = HEADING_SIZE_LARGE + HEADING_BOLD + HEADING_TITLE_LINES;
    let heading_demoted = HEADING_SIZE_LARGE + HEADING_BOLD;
    let list = LIST_MARKER + LIST_ITEMS + LIST_HANGING;
    let paragraph = PARAGRAPH_TEXT_SIZE + PARAGRAPH_JOINS + PARAGRAPH_SENTENCES;
    let pairs = [
        ("demoted heading", PARAGRAPH_DEMOTED, heading_demoted),
        (
            "title over heading",
            TITLE_HEADING + TITLE_TOP,
            heading_kept,
        ),
        ("legend over list", LEGEND_ENTRIES, list),
        ("legend over paragraph", LEGEND_ENTRIES, paragraph),
        ("band over paragraph", BAND_RUNNING, paragraph),
        ("band over list", BAND_RUNNING, list),
    ];
    for (what, winner, loser) in pairs {
        assert!(winner >= loser + MARGIN, "{what}: {winner} against {loser}");
    }
    // The least paragraph evidence alone loses to the first signal of every other kind.
    for first in [
        HEADING_SIZE_LARGE.min(layoutregions::typers::text::HEADING_SIZE),
        LIST_MARKER,
        BAND_RUNNING,
        LEGEND_ENTRIES,
        TITLE_HEADING,
    ] {
        assert!(PARAGRAPH_TEXT_SIZE + MARGIN <= first);
    }
}

/// A segment of the view's tree is walked once for all typers: a second round does no work
/// and gives the same as a copy of the segment, which is walked again.
#[test]
fn a_segment_of_the_tree_is_walked_once() {
    let case = heading_case(&[0]);
    let file: Vec<FilePage<'_, TextRun>> = case.pages.iter().map(FilePage::of).collect();
    let facts = learn(&file, false, &no_amount);
    let index = FileIndex::new(&facts);
    let page = &case.pages[0];
    let structure = drawn(page, &|_| true);
    let segs = segment_page(page, &Limits::default(), &|| false);
    let view = PageView::new(page, &structure, &segs, &facts, &index, &no_amount);
    let all = |seg: &Segment| -> Vec<Typed> {
        TYPERS
            .iter()
            .filter_map(|(_, typer)| typer(&view, seg))
            .collect()
    };
    for seg in &segs.all {
        let before = work_steps();
        let first = all(seg);
        let between = work_steps();
        let second = all(seg);
        let after = work_steps();
        assert_eq!(first, second);
        assert_eq!(all(&seg.clone()), first);
        if let (Some(a), Some(b), Some(c)) = (before, between, after) {
            if seg.prims.iter().any(|p| matches!(p, PrimId::Text(_))) {
                assert!(b > a, "the first round walks the text");
            }
            assert_eq!(c, b, "the second round is cached");
        }
    }
    // The root holds every line: a heading over a paragraph is no text kind of its own.
    assert!(all(&segs.all[segs.root.0 as usize]).is_empty());
}

// ---- Tables, totals and label/value groups (Task 17) ----

/// A rule from (x0, y0) to (x1, y1).
fn rule(x0: f32, y0: f32, x1: f32, y1: f32) -> RuleIn {
    RuleIn {
        x0,
        y0,
        x1,
        y1,
        thick: 0.5,
        dashed: false,
        rect: false,
    }
}

/// The rules of a grid of column edges `xs` and row edges `ys`.
fn grid_rules(xs: &[f32], ys: &[f32]) -> Vec<RuleIn> {
    let (l, r) = (xs[0], xs[xs.len() - 1]);
    let (b, t) = (ys[ys.len() - 1], ys[0]);
    let mut out: Vec<RuleIn> = ys.iter().map(|&y| rule(l, y, r, y)).collect();
    out.extend(xs.iter().map(|&x| rule(x, b, x, t)));
    out
}

/// A drawn box (x0, y0, x1, y1): its four sides as rules.
fn box_rules(b: [f32; 4]) -> Vec<RuleIn> {
    grid_rules(&[b[0], b[2]], &[b[3], b[1]])
}

/// A page with prose above and below (the body size) and `texts` and `rules` between.
fn sheet(texts: Vec<TextRun>, rules: Vec<RuleIn>) -> PageIn {
    let mut all = prose(0, 800.0);
    all.extend(texts);
    all.extend(prose(900, 120.0));
    let mut p = page(2, all);
    p.rules = rules;
    p
}

/// What the typers say of each segment of a page's tree.
struct Tree {
    page: PageIn,
}

impl Tree {
    fn scaled(&self, k: f32) -> Tree {
        let c = Case {
            pages: vec![self.page.clone()],
            at: 0,
            lines: Vec::new(),
        };
        let mut page = c.scaled(k).pages.remove(0);
        page.rules = self
            .page
            .rules
            .iter()
            .map(|r| RuleIn {
                x0: r.x0 * k,
                y0: r.y0 * k,
                x1: r.x1 * k,
                y1: r.y1 * k,
                thick: r.thick * k,
                ..r.clone()
            })
            .collect();
        Tree { page }
    }

    /// `f` of the view of the page.
    fn with<T>(&self, f: impl FnOnce(&PageView<'_>) -> T) -> T {
        let file = [FilePage::of(&self.page)];
        let facts = learn(&file, false, &no_amount);
        let index = FileIndex::new(&facts);
        let structure = drawn(&self.page, &|_| true);
        let segs = segment_page(&self.page, &Limits::default(), &|| false);
        let view = PageView::new(&self.page, &structure, &segs, &facts, &index, &no_amount);
        f(&view)
    }

    /// Every typer's answer for the segment of the tree that `pick` finds first.
    fn typed(&self, pick: &dyn Fn(&PageView<'_>, &Segment) -> bool) -> Vec<Typed> {
        self.with(|view| {
            let seg = view
                .segs
                .all
                .iter()
                .find(|s| pick(view, s))
                .unwrap_or_else(|| panic!("no such segment in {:#?}", view.segs.all));
            all_typed(view, seg)
        })
    }
}

impl Tree {
    /// Every typer's answer for the smallest segment of the tree that holds the text of
    /// every line of `lines` and no text of other lines.
    fn typed_holding(&self, lines: &[u32]) -> Vec<Typed> {
        self.with(|view| {
            let lines_of = |s: &Segment| -> Vec<u32> {
                let mut l: Vec<u32> = s
                    .prims
                    .iter()
                    .filter_map(|p| match p {
                        PrimId::Text(i) => Some(view.page.texts[*i as usize].line),
                        _ => None,
                    })
                    .collect();
                l.sort_unstable();
                l.dedup();
                l
            };
            let seg = view
                .segs
                .all
                .iter()
                .filter(|s| lines_of(s) == lines)
                .max_by_key(|s| s.prims.len())
                .unwrap_or_else(|| {
                    let all: Vec<(Source, Vec<u32>)> = view
                        .segs
                        .all
                        .iter()
                        .map(|s| (s.source, lines_of(s)))
                        .collect();
                    panic!("no segment of exactly {lines:?}: {all:?}")
                });
            all_typed(view, seg)
        })
    }
}

fn all_typed(view: &PageView<'_>, seg: &Segment) -> Vec<Typed> {
    TYPERS
        .iter()
        .filter_map(|(kind, typer)| {
            let t = typer(view, seg)?;
            assert_eq!(t.kind, *kind, "a typer gives its own kind");
            Some(t)
        })
        .collect()
}

fn find(typed: &[Typed], kind: Kind) -> Option<&Typed> {
    typed.iter().find(|t| t.kind == kind)
}

/// The grid segment of the page.
fn grid(_: &PageView<'_>, s: &Segment) -> bool {
    matches!(s.source, Source::Grid { .. })
}

/// The segment that holds exactly the text runs of `lines` (and maybe other primitives).
fn text_of_lines(lines: Vec<u32>) -> impl Fn(&PageView<'_>, &Segment) -> bool {
    move |view, s| {
        let mut own: Vec<u32> = s
            .prims
            .iter()
            .filter_map(|p| match p {
                PrimId::Text(i) => Some(view.page.texts[*i as usize].line),
                _ => None,
            })
            .collect();
        own.sort_unstable();
        own.dedup();
        own == lines
    }
}

/// Cells of a ruled grid: the column edges `xs`, rows `rows` (each its cell texts, the first
/// the header) from `top` down, 14 points a row, header bold when `bold`.
fn ruled_table(xs: &[f32], top: f32, rows: &[&[&str]], bold: bool, first_line: u32) -> Tree {
    let ys: Vec<f32> = (0..=rows.len()).map(|k| top - 14.0 * k as f32).collect();
    let mut texts = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            let r = run(t, xs[c] + 3.0, ys[k] - 10.2, 9.2, first_line + k as u32);
            texts.push(if bold && k == 0 { self::bold(r) } else { r });
        }
    }
    Tree {
        page: sheet(texts, grid_rules(xs, &ys)),
    }
}

const ITEMS: &[&[&str]] = &[
    &["Artikel", "Menge", "Preis"],
    &["Muster A", "2", "12,50"],
    &["Muster B", "1", "3,00"],
    &["Muster C", "4", "8,75"],
];

const ITEM_XS: &[f32] = &[72.0, 220.0, 320.0, 420.0];

#[test]
fn ruled_grid_is_table_evidence() {
    let tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    let typed = tree.typed(&grid);
    let table = of(&typed, Kind::Table);
    assert_eq!(
        ids(table),
        [
            "table.drawn_cells",
            "table.shared_columns",
            "table.header_row"
        ]
    );
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Table, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
}

/// Long sentences in a grid of two rows and two columns: the cells hold prose.
fn prose_grid() -> Tree {
    let rows: &[&[&str]] = &[
        &[
            "die Ware wird nach dem Erhalt der Rechnung geliefert.",
            "der Betrag wird innerhalb von zehn Tagen bezahlt.",
        ],
        &[
            "die Muster GmbH sendet die Ware an den Kunden.",
            "der Kunde prueft die Ware nach dem Erhalt genau.",
        ],
    ];
    ruled_table(&[40.0, 300.0, 560.0], 700.0, rows, false, 100)
}

#[test]
fn prose_cells_speak_for_paragraph() {
    let typed = prose_grid().typed(&grid);
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
    let paragraph = of(&typed, Kind::Paragraph);
    assert!(
        ids(paragraph).contains(&"paragraph.prose_cells"),
        "{typed:#?}"
    );
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Paragraph);
    assert!(lead >= MARGIN);
}

/// The item grid with totals under it: labels in the description column, amounts right
/// under the price column.
fn totals_tree(rule_over: bool) -> Tree {
    let mut tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    let lines: &[(&str, &str)] = &[
        ("Summe netto", "24,25"),
        ("MwSt 19 %", "4,61"),
        ("Gesamt", "28,86"),
    ];
    for (k, (label, value)) in lines.iter().enumerate() {
        let y = 630.0 - 13.0 * k as f32;
        tree.page
            .texts
            .push(run(label, 75.0, y, 9.2, 200 + k as u32));
        let v = run(value, 0.0, y, 9.2, 200 + k as u32);
        let x = 417.0 - v.w;
        tree.page.texts.push(TextRun { x, ..v });
    }
    if rule_over {
        tree.page.rules.push(rule(320.0, 641.0, 420.0, 641.0));
    }
    tree.page.texts.sort_by_key(|r| r.line);
    tree
}

#[test]
fn amounts_under_money_columns_are_totals() {
    let tree = totals_tree(false);
    let typed = tree.typed(&text_of_lines(vec![200, 201, 202]));
    let totals = of(&typed, Kind::Totals);
    assert_eq!(
        ids(totals),
        ["totals.under_money_columns", "totals.labels_in_description"]
    );
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Totals, "{typed:#?}");
    assert!(lead >= MARGIN);
    // A rule over the amounts, across the money column, is one more signal.
    let typed = totals_tree(true).typed(&text_of_lines(vec![200, 201, 202]));
    assert!(ids(of(&typed, Kind::Totals)).contains(&"totals.end_rule"));
}

/// A row of three boxes side by side, each a small label over its value.
fn boxed_fields() -> Tree {
    let fields = [
        ("Rechnungsnummer", "RE-2026-0001"),
        ("Datum", "01.01.2026"),
        ("Kundennummer", "12345"),
    ];
    let mut texts = Vec::new();
    let mut rules = Vec::new();
    for (k, (label, value)) in fields.iter().enumerate() {
        let x = 72.0 + 150.0 * k as f32;
        texts.push(run(label, x + 3.0, 683.0, 6.2, 100));
        texts.push(run(value, x + 3.0, 668.0, 9.2, 101));
        rules.extend(box_rules([x, 660.0, x + 150.0, 692.0]));
    }
    texts.sort_by_key(|r| r.line);
    Tree {
        page: sheet(texts, rules),
    }
}

#[test]
fn boxed_fields_are_label_value() {
    let typed = boxed_fields().typed(&grid);
    let pairs = of(&typed, Kind::LabelValue);
    let found = ids(pairs);
    assert!(found.contains(&"label_value.field_cells"), "{typed:#?}");
    assert!(found.contains(&"label_value.boxes_touch"), "{typed:#?}");
    assert!(found.contains(&"label_value.value_shape"), "{typed:#?}");
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::LabelValue, "{typed:#?}");
    assert!(lead >= MARGIN);
}

#[test]
fn size_alone_seeds_no_style() {
    // A firm name over its address in a smaller size, in a box: no colon, no value shape.
    let texts = vec![
        run("Muster GmbH", 75.0, 683.0, 10.2, 100),
        run("Musterweg 1", 75.0, 671.0, 8.2, 101),
        run("12345 Musterstadt", 75.0, 661.0, 8.2, 102),
    ];
    let tree = Tree {
        page: sheet(texts, box_rules([72.0, 655.0, 250.0, 695.0])),
    };
    let typed = tree.typed(&|_, s| s.source == Source::Box);
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
}

/// Labels and values in lines: bold labels, with a colon and values shaped as numbers when
/// `evidence`, else words; and a box whose label and value differ in weight only.
fn seeded(evidence: bool) -> Tree {
    let rows: &[(&str, &str)] = if evidence {
        &[
            ("Kundennummer:", "12345"),
            ("Vertragsnummer:", "67890"),
            ("Rechnungsdatum:", "01.01.2026"),
        ]
    } else {
        &[
            ("Abteilung", "Einkauf"),
            ("Bereich", "Verwaltung"),
            ("Standort", "Musterstadt"),
        ]
    };
    let mut texts = Vec::new();
    for (k, (label, value)) in rows.iter().enumerate() {
        let y = 700.0 - 13.0 * k as f32;
        texts.push(bold(run(label, 72.0, y, 9.2, 100 + k as u32)));
        texts.push(run(value, 200.0, y, 9.2, 100 + k as u32));
    }
    texts.push(bold(run("Ansprechpartner", 335.0, 560.0, 9.2, 150)));
    texts.push(run("Erika Mustermann", 335.0, 547.0, 9.2, 151));
    Tree {
        page: sheet(texts, box_rules([330.0, 540.0, 520.0, 572.0])),
    }
}

#[test]
fn a_colon_pair_seeds_a_weight_only_box() {
    let typed = seeded(true).typed(&|_, s| s.source == Source::Box);
    let pairs = of(&typed, Kind::LabelValue);
    assert!(
        ids(pairs).contains(&"label_value.known_style"),
        "{typed:#?}"
    );
}

#[test]
fn a_size_pair_seeds_no_weight_only_box() {
    let typed = seeded(false).typed(&|_, s| s.source == Source::Box);
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
}

/// Eight columns without rules: a bold header and four rows.
fn eight_columns() -> Vec<TextRun> {
    let titles = [
        "Pos", "Nr", "Text", "Menge", "Einh", "Preis", "MwSt", "Summe",
    ];
    let mut texts = Vec::new();
    for (c, t) in titles.iter().enumerate() {
        texts.push(bold(run(t, 40.0 + 64.0 * c as f32, 700.0, 9.2, 100)));
    }
    for k in 0..4u32 {
        let row = [
            format!("{}", k + 1),
            format!("A-10{k}"),
            "Muster".to_string(),
            format!("{}", k + 2),
            "Stk".to_string(),
            "12,50".to_string(),
            "19%".to_string(),
            "25,00".to_string(),
        ];
        for (c, t) in row.iter().enumerate() {
            let y = 687.0 - 13.0 * k as f32;
            texts.push(run(t, 40.0 + 64.0 * c as f32, y, 9.2, 101 + k));
        }
    }
    texts
}

#[test]
fn eight_aligned_columns_read_as_a_table() {
    let tree = Tree {
        page: sheet(eight_columns(), Vec::new()),
    };
    let lines: Vec<u32> = (100..105).collect();
    let pick = text_of_lines(lines.clone());
    let typed = tree.typed(&pick);
    let (kind, _) = winner(&typed);
    assert_eq!(kind, Kind::Table, "{typed:#?}");
    tree.with(|view| {
        let seg = view.segs.all.iter().find(|s| pick(view, s)).unwrap();
        let cols = table_columns(view, seg).expect("columns");
        assert_eq!(cols.edges.len(), 9, "{cols:#?}");
        assert_eq!(cols.header.len(), 8);
        assert!(cols.header.iter().all(|h| h.len() == 1), "{cols:#?}");
        assert_eq!(cols.header_rows, 1);
    });
    // One region the file states (a tagged table) that holds every run: its columns read by
    // geometry alone, and no table candidate for it (it is decided as given).
    let mut page = sheet(eight_columns(), Vec::new());
    let prims: Vec<PrimId> = page
        .texts
        .iter()
        .enumerate()
        .filter(|(_, r)| (100..105).contains(&r.line))
        .map(|(i, _)| PrimId::Text(i as u32))
        .collect();
    page.given.push(GivenRegion {
        kind: Kind::Table,
        prims,
        order: 0,
        level: None,
    });
    let given = Tree { page };
    given.with(|view| {
        let seg = view
            .segs
            .all
            .iter()
            .find(|s| s.source == Source::Given(Kind::Table))
            .unwrap();
        let cols = table_columns(view, seg).expect("columns of a given table");
        assert_eq!(cols.edges.len(), 9);
        assert_eq!(cols.header.len(), 8);
        assert!(type_table(view, seg).is_none());
    });
}

/// The item grid with a sentence printed over its last row, from its first column into its
/// third, in runs `parts` (word counts per run).
fn sentence_over_row(parts: &[usize]) -> (Tree, Vec<u32>) {
    let mut tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    let words: Vec<&str> = "die Ware wird nach dem Erhalt der Rechnung geliefert und bezahlt"
        .split(' ')
        .collect();
    let mut x = 75.0;
    let mut at = 0;
    let y = 700.0 - 14.0 * 4.0 - 10.2;
    // One more row of the grid for the sentence.
    tree.page.rules = grid_rules(ITEM_XS, &[700.0, 686.0, 672.0, 658.0, 644.0, 630.0]);
    for &n in parts {
        let text = words[at..at + n].join(" ");
        at += n;
        let r = run(&text, x, y, 9.2, 104);
        // The gap between runs falls on a column rule when the run ends before it.
        x = r.x + r.w + 4.6;
        if x > 215.0 && x < 225.0 {
            x = 223.0;
        }
        tree.page.texts.push(r);
    }
    tree.page.texts.sort_by_key(|r| r.line);
    let ids: Vec<u32> = tree
        .page
        .texts
        .iter()
        .enumerate()
        .filter(|(_, r)| r.line == 104)
        .map(|(i, _)| i as u32)
        .collect();
    (tree, ids)
}

#[test]
fn a_sentence_over_a_ruled_row_is_paragraph_evidence() {
    // Its word gap at a column rule; then the same sentence in three runs.
    for parts in [&[5usize, 6][..], &[4, 4, 3][..]] {
        let (tree, sentence) = sentence_over_row(parts);
        tree.with(|view| {
            let seg = Segment {
                id: SegId(u32::MAX),
                bounds: Bounds {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 595.0,
                    y1: 842.0,
                },
                prims: sentence.iter().map(|&i| PrimId::Text(i)).collect(),
                source: Source::Rest,
                parent: None,
                children: Vec::new(),
                alternatives: Vec::new(),
            };
            let typed = all_typed(view, &seg);
            let paragraph = of(&typed, Kind::Paragraph);
            assert!(
                ids(paragraph).contains(&"paragraph.prose_cells"),
                "{typed:#?}"
            );
            let (kind, lead) = winner(&typed);
            assert_eq!(kind, Kind::Paragraph);
            assert!(lead >= MARGIN);
            // The grid's reading names the line.
            let g = view.segs.all.iter().find(|s| grid(view, s)).unwrap();
            let table = type_table(view, g).expect("the grid is a table");
            assert!(
                table.signals[0].detail.contains("sentence"),
                "{:#?}",
                table.signals
            );
        });
    }
}

#[test]
fn table_pair_and_totals_scores_are_scale_free() {
    type Pick = Box<dyn Fn(&PageView<'_>, &Segment) -> bool>;
    let cases: Vec<(Tree, Pick)> = vec![
        (
            ruled_table(ITEM_XS, 700.0, ITEMS, true, 100),
            Box::new(grid),
        ),
        (prose_grid(), Box::new(grid)),
        (
            totals_tree(true),
            Box::new(text_of_lines(vec![200, 201, 202])),
        ),
        (boxed_fields(), Box::new(grid)),
        (
            seeded(true),
            Box::new(|_: &PageView<'_>, s: &Segment| s.source == Source::Box),
        ),
        (
            Tree {
                page: sheet(eight_columns(), Vec::new()),
            },
            Box::new(text_of_lines((100..105).collect())),
        ),
    ];
    for (tree, pick) in &cases {
        let scores = |t: &Tree| -> Vec<(Kind, u16, Vec<&'static str>)> {
            t.typed(pick.as_ref())
                .iter()
                .map(|t| (t.kind, t.score.0, ids(t)))
                .collect()
        };
        let at = scores(tree);
        assert!(!at.is_empty());
        for k in [0.75, 0.8, 1.3, 1.5] {
            assert_eq!(scores(&tree.scaled(k)), at, "at {k}x");
        }
    }
}

/// The kinds of this task against their rivals, by the weights alone: each winner's least
/// score is ahead of the loser's best on such text by the margin.
#[test]
fn worst_case_margins_of_tables_pairs_and_totals() {
    use layoutregions::typers::pairs::{
        LABEL_BOXES_TOUCH, LABEL_COLON, LABEL_FIELD_CELLS, LABEL_FIELDS, LABEL_KEY_VALUES,
        LABEL_KNOWN_STYLE, LABEL_LINE_PAIRS, LABEL_STRIP_FIELDS, LABEL_STYLE, LABEL_VALUE_SHAPE,
    };
    use layoutregions::typers::table::{
        PARAGRAPH_PROSE_CELLS, TABLE_HEADER, TABLE_READING, TABLE_SHARED, TOTALS_UNDER,
    };
    // Paragraph on any text, without a demotion. Inside a table reading the demotions of lines
    // apart, unwrapped or in cells are the table's, and an address or sentences refuse the
    // table (a content demotion against a refused table, below).
    let paragraph = PARAGRAPH_TEXT_SIZE + PARAGRAPH_JOINS + PARAGRAPH_SENTENCES;
    // A heading on lines in two pieces or more is demoted as cells: no title lines.
    let heading_in_cells = HEADING_SIZE_LARGE + HEADING_BOLD;
    let list = LIST_MARKER + LIST_ITEMS + LIST_HANGING;
    // Paragraph on the lines of a table: they stand in pieces, so they join no paragraph.
    let paragraph_in_cells = PARAGRAPH_TEXT_SIZE + PARAGRAPH_SENTENCES;
    let table = TABLE_READING + TABLE_SHARED + TABLE_HEADER;
    let pairs = [
        (
            "bare table over paragraph in its cells",
            TABLE_READING,
            paragraph_in_cells,
        ),
        ("prose cells over a refused table", PARAGRAPH_PROSE_CELLS, 0),
        ("field cells over paragraph", LABEL_FIELD_CELLS, paragraph),
        ("key values over paragraph", LABEL_KEY_VALUES, paragraph),
        ("key values over list", LABEL_KEY_VALUES, list),
        (
            "key values over a heading in cells",
            LABEL_KEY_VALUES,
            heading_in_cells,
        ),
        (
            "content demotion over a refused table",
            PARAGRAPH_DEMOTED,
            0,
        ),
        (
            "touching boxed fields over paragraph",
            LABEL_FIELDS + LABEL_STYLE + LABEL_BOXES_TOUCH,
            paragraph,
        ),
        ("totals over paragraph", TOTALS_UNDER, paragraph),
        // Line pairs carry a colon or a shaped value on every line (Task 18b, ruling Q2).
        (
            "line pairs over paragraph",
            LABEL_LINE_PAIRS + LABEL_COLON.min(LABEL_VALUE_SHAPE),
            paragraph,
        ),
        (
            "line pairs over a large bold heading that keeps its lines",
            LABEL_LINE_PAIRS + LABEL_COLON.min(LABEL_VALUE_SHAPE),
            HEADING_SIZE_LARGE + HEADING_BOLD + HEADING_TITLE_LINES,
        ),
        // A box whose label differs in weight only, read by a seeded style, alone.
        (
            "lone weight-only box over paragraph",
            LABEL_FIELDS + LABEL_STYLE + LABEL_KNOWN_STYLE,
            paragraph,
        ),
        // Fields in strips between rules (Task 18b, round 2).
        ("strip fields over a table", LABEL_STRIP_FIELDS, table),
        ("strip fields over paragraph", LABEL_STRIP_FIELDS, paragraph),
        // Refused readings give nothing: a table on field cells (see
        // `field_cells_refuse_a_table`), key values or totals, and label/value pairs on totals
        // (see `amounts_under_money_columns_are_totals`).
        ("field cells over a refused table", LABEL_FIELD_CELLS, 0),
        ("totals over refused pairs", TOTALS_UNDER, 0),
        // A table's reading alone is ahead of the least evidence of a paragraph.
        (
            "table over the least paragraph",
            TABLE_READING,
            PARAGRAPH_TEXT_SIZE,
        ),
    ];
    for (what, winner, loser) in pairs {
        assert!(winner >= loser + MARGIN, "{what}: {winner} against {loser}");
    }
    assert!(table <= 1000);
}

#[test]
fn a_header_band_over_lines_is_table_evidence() {
    // A row of three drawn cells with column titles, and four lines without rules under it in
    // its columns.
    let xs = [72.0, 220.0, 320.0, 420.0];
    let mut texts = Vec::new();
    for (c, t) in ["Artikel", "Menge", "Preis"].iter().enumerate() {
        texts.push(run(t, xs[c] + 3.0, 689.8, 9.2, 100));
    }
    for (k, row) in ITEMS[1..]
        .iter()
        .chain([&["Muster D", "3", "1,00"][..]].iter())
        .enumerate()
    {
        for (c, t) in row.iter().enumerate() {
            let y = 672.0 - 13.0 * k as f32;
            texts.push(run(t, xs[c] + 3.0, y, 9.2, 101 + k as u32));
        }
    }
    let tree = Tree {
        page: sheet(texts, grid_rules(&xs, &[700.0, 686.0])),
    };
    let typed = tree.typed(&|_, s| s.source == Source::Page);
    let table = of(&typed, Kind::Table);
    assert_eq!(
        ids(table),
        [
            "table.header_band",
            "table.shared_columns",
            "table.header_row"
        ]
    );
    for k in [0.75, 1.5] {
        let scaled = tree.scaled(k).typed(&|_, s| s.source == Source::Page);
        assert_eq!(of(&scaled, Kind::Table).score, table.score, "at {k}x");
    }
}

/// A grid of two rows and three columns of boxes, each a small label over a value of words:
/// no colon, no value shape.
fn field_grid() -> Tree {
    let cells = [
        [
            ("Firma", "Muster GmbH"),
            ("Name", "Erika Mustermann"),
            ("Ort", "Musterstadt"),
        ],
        [
            ("Abteilung", "Einkauf"),
            ("Bereich", "Verwaltung"),
            ("Land", "Musterland"),
        ],
    ];
    let xs = [72.0, 222.0, 372.0, 522.0];
    let ys = [700.0, 668.0, 636.0];
    let mut texts = Vec::new();
    for (r, row) in cells.iter().enumerate() {
        for (c, (label, value)) in row.iter().enumerate() {
            let top = ys[r];
            texts.push(run(label, xs[c] + 3.0, top - 9.0, 6.2, 100 + 2 * r as u32));
            texts.push(run(value, xs[c] + 3.0, top - 24.0, 9.2, 101 + 2 * r as u32));
        }
    }
    texts.sort_by_key(|r| r.line);
    Tree {
        page: sheet(texts, grid_rules(&xs, &ys)),
    }
}

#[test]
fn field_cells_refuse_a_table() {
    let tree = field_grid();
    let typed = tree.typed(&grid);
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
    let pairs = of(&typed, Kind::LabelValue);
    assert!(
        ids(pairs).contains(&"label_value.field_cells"),
        "{typed:#?}"
    );
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::LabelValue);
    assert!(lead >= MARGIN, "{typed:#?}");
    tree.with(|view| {
        let g = view.segs.all.iter().find(|s| grid(view, s)).unwrap();
        let r = refusal(view, g).expect("a refusal");
        assert_eq!(r.kind, Kind::LabelValue);
        assert_eq!(r.signal.id, "label_value.refuses_table");
    });
}

/// The item grid with rows of the same table under it, out of its rules: a description, a
/// quantity and an amount each, in its columns.
fn continued_rows() -> Tree {
    let mut tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    let rows = [
        ("Muster D", "3", "1,00"),
        ("Muster E", "2", "5,00"),
        ("Muster F", "1", "2,50"),
    ];
    for (k, (d, q, a)) in rows.iter().enumerate() {
        let y = 630.0 - 13.0 * k as f32;
        let line = 200 + k as u32;
        tree.page.texts.push(run(d, 75.0, y, 9.2, line));
        tree.page.texts.push(run(q, 223.0, y, 9.2, line));
        let v = run(a, 0.0, y, 9.2, line);
        let x = 417.0 - v.w;
        tree.page.texts.push(TextRun { x, ..v });
    }
    tree.page.texts.sort_by_key(|r| r.line);
    tree
}

#[test]
fn rows_of_a_table_are_no_totals() {
    let typed = continued_rows().typed(&text_of_lines(vec![200, 201, 202]));
    assert!(find(&typed, Kind::Totals).is_none(), "{typed:#?}");
    // The rows keep their table reading.
    assert!(find(&typed, Kind::Table).is_some(), "{typed:#?}");
}

/// A table of a description and an amount only, with three more such lines under it, and a
/// rule between them when `rule`.
fn two_column_stack(rule: bool) -> Tree {
    let rows: &[&[&str]] = &[
        &["Artikel", "Preis"],
        &["Muster A", "12,50"],
        &["Muster B", "3,00"],
        &["Muster C", "8,75"],
    ];
    let xs = [72.0, 320.0, 420.0];
    let mut tree = ruled_table(&xs, 700.0, rows, true, 100);
    for (k, (d, a)) in [
        ("Muster D", "1,00"),
        ("Muster E", "5,00"),
        ("Muster F", "2,50"),
    ]
    .iter()
    .enumerate()
    {
        let y = 630.0 - 13.0 * k as f32;
        tree.page.texts.push(run(d, 75.0, y, 9.2, 200 + k as u32));
        let v = run(a, 0.0, y, 9.2, 200 + k as u32);
        let x = 417.0 - v.w;
        tree.page.texts.push(TextRun { x, ..v });
    }
    if rule {
        tree.page.rules.push(rule_at(641.0));
    }
    tree.page.texts.sort_by_key(|r| r.line);
    tree
}

fn rule_at(y: f32) -> RuleIn {
    rule(320.0, y, 420.0, y)
}

#[test]
fn a_table_of_descriptions_and_amounts_needs_a_rule_for_totals() {
    let lines = || text_of_lines(vec![200, 201, 202]);
    let typed = two_column_stack(false).typed(&lines());
    assert!(find(&typed, Kind::Totals).is_none(), "{typed:#?}");
    // The lines keep their own reading: lines of two pieces, a description and an amount,
    // flush at both edges, are what the key/value rule reads (it refuses a table of them).
    let pairs = of(&typed, Kind::LabelValue);
    assert!(ids(pairs).contains(&"label_value.key_values"), "{typed:#?}");
    let typed = two_column_stack(true).typed(&lines());
    assert!(
        ids(of(&typed, Kind::Totals)).contains(&"totals.end_rule"),
        "{typed:#?}"
    );
}

#[test]
fn one_total_under_a_table() {
    let mut tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    tree.page.texts.push(run("Gesamt", 75.0, 630.0, 9.2, 200));
    let v = run("28,86", 0.0, 630.0, 9.2, 200);
    let x = 417.0 - v.w;
    tree.page.texts.push(TextRun { x, ..v });
    tree.page.texts.sort_by_key(|r| r.line);
    let typed = tree.typed(&text_of_lines(vec![200]));
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Totals, "{typed:#?}");
    assert!(lead >= MARGIN);
}

#[test]
fn a_lone_boxed_field_is_label_value() {
    let texts = vec![
        run("Rechnungsnummer", 75.0, 683.0, 6.2, 100),
        run("RE-2026-0001", 75.0, 668.0, 9.2, 101),
    ];
    let tree = Tree {
        page: sheet(texts, box_rules([72.0, 660.0, 222.0, 692.0])),
    };
    let typed = tree.typed(&|_, s| s.source == Source::Box);
    let pairs = of(&typed, Kind::LabelValue);
    assert!(
        !ids(pairs).contains(&"label_value.boxes_touch"),
        "{typed:#?}"
    );
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::LabelValue, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
}

#[test]
fn the_seed_lines_are_pairs_themselves() {
    // Without a colon or a value shape the lines still read as labels and values: only the
    // seed rule keeps them from seeding the weight-only box.
    for evidence in [true, false] {
        let typed = seeded(evidence).typed(&text_of_lines(vec![100, 101, 102]));
        let pairs = of(&typed, Kind::LabelValue);
        assert!(ids(pairs).contains(&"label_value.key_values"), "{typed:#?}");
    }
    // The seeded box alone (no box beside it) leads the paragraph its two lines join into
    // by the margin: its seeded style is the evidence (950 against 600).
    let typed = seeded(true).typed(&|_, s| s.source == Source::Box);
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::LabelValue, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
}

#[test]
fn too_little_text_is_no_table() {
    // One line in four pieces; one column of lines; three runs.
    let cases: Vec<Vec<TextRun>> = vec![
        vec![
            run("Muster", 72.0, 700.0, 9.2, 100),
            run("2", 220.0, 700.0, 9.2, 100),
            run("Stk", 270.0, 700.0, 9.2, 100),
            run("12,50", 320.0, 700.0, 9.2, 100),
        ],
        (0..4)
            .map(|k| run("Muster", 72.0, 700.0 - 13.0 * k as f32, 9.2, 100 + k))
            .collect(),
        vec![
            run("Muster", 72.0, 700.0, 9.2, 100),
            run("2", 220.0, 700.0, 9.2, 100),
            run("Muster", 72.0, 687.0, 9.2, 101),
        ],
    ];
    for texts in cases {
        let lines: Vec<u32> = {
            let mut l: Vec<u32> = texts.iter().map(|r| r.line).collect();
            l.dedup();
            l
        };
        let tree = Tree {
            page: sheet(texts, Vec::new()),
        };
        let typed = tree.typed(&text_of_lines(lines));
        assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
    }
}

#[test]
fn a_page_of_tables_and_totals_is_typed_once() {
    let tree = totals_tree(true);
    tree.with(|view| {
        let all = |view: &PageView<'_>| -> Vec<Vec<Typed>> {
            view.segs.all.iter().map(|s| all_typed(view, s)).collect()
        };
        let first = all(view);
        let before = work_steps();
        let second = all(view);
        let after = work_steps();
        assert_eq!(first, second);
        if let (Some(b), Some(a)) = (before, after) {
            assert_eq!(a, b, "the second round does no work");
        }
        assert!(first.iter().flatten().any(|t| t.kind == Kind::Totals));
        assert!(first.iter().flatten().any(|t| t.kind == Kind::Table));
    });
}

/// An item table `Pos | Bezeichnung | Menge | Preis` (Pos and Menge numbers in every row) with
/// one line under it: `label` at `x`, its amount right under Preis, a rule over the amount
/// when `rule`. `pos` puts a Pos number before the label (a row of the table).
fn pos_table(label: &str, x: f32, rule_over: bool, pos: Option<&str>) -> Tree {
    let rows: &[&[&str]] = &[
        &["Pos", "Bezeichnung", "Menge", "Preis"],
        &["1", "Muster A", "2", "12,50"],
        &["2", "Muster B", "1", "3,00"],
        &["3", "Muster C", "4", "8,75"],
    ];
    let xs = [72.0, 100.0, 260.0, 340.0, 440.0];
    let mut tree = ruled_table(&xs, 700.0, rows, true, 100);
    let y = 630.0;
    if let Some(n) = pos {
        tree.page.texts.push(run(n, 75.0, y, 9.2, 200));
    }
    tree.page.texts.push(run(label, x, y, 9.2, 200));
    if pos.is_some() {
        tree.page.texts.push(run("3", 263.0, y, 9.2, 200));
    }
    let v = run("28,86", 0.0, y, 9.2, 200);
    let vx = 437.0 - v.w;
    tree.page.texts.push(TextRun { x: vx, ..v });
    if rule_over {
        tree.page.rules.push(rule(340.0, 641.0, 440.0, 641.0));
    }
    tree.page
        .texts
        .sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
    tree
}

#[test]
fn a_total_label_may_stand_in_a_column_the_rows_fill() {
    // Flush left under Pos, and right-aligned against its amount under Menge.
    let w = |t: &str| t.chars().count() as f32 * 4.6;
    let cases = [("Netto", 75.0), ("Summe netto", 335.0 - w("Summe netto"))];
    for (label, x) in cases {
        for rule_over in [false, true] {
            let typed = pos_table(label, x, rule_over, None).typed(&text_of_lines(vec![200]));
            let totals = of(&typed, Kind::Totals);
            assert_eq!(
                ids(totals).contains(&"totals.end_rule"),
                rule_over,
                "{label}: {typed:#?}"
            );
            let (kind, lead) = winner(&typed);
            assert_eq!(kind, Kind::Totals, "{label}: {typed:#?}");
            assert!(lead >= MARGIN, "{label}: {typed:#?}");
        }
    }
    // A label of words right-aligned under a column of words (Einheit: Stk, Std).
    let rows: &[&[&str]] = &[
        &["Pos", "Bezeichnung", "Einheit", "Preis"],
        &["1", "Muster A", "Stk", "12,50"],
        &["2", "Muster B", "Std", "3,00"],
        &["3", "Muster C", "Stk", "8,75"],
    ];
    let xs = [72.0, 100.0, 260.0, 340.0, 440.0];
    for rule_over in [false, true] {
        let mut tree = ruled_table(&xs, 700.0, rows, true, 100);
        let label = run("Summe netto", 0.0, 630.0, 9.2, 200);
        let lx = 335.0 - label.w;
        assert!(
            lx + label.w / 2.0 > 260.0,
            "its middle stands under Einheit"
        );
        tree.page.texts.push(TextRun { x: lx, ..label });
        let v = run("28,86", 0.0, 630.0, 9.2, 200);
        let vx = 437.0 - v.w;
        tree.page.texts.push(TextRun { x: vx, ..v });
        if rule_over {
            tree.page.rules.push(rule(340.0, 641.0, 440.0, 641.0));
        }
        let typed = tree.typed(&text_of_lines(vec![200]));
        let (kind, lead) = winner(&typed);
        assert_eq!(kind, Kind::Totals, "Einheit: {typed:#?}");
        assert!(lead >= MARGIN, "Einheit: {typed:#?}");
    }
    // A line with a number under Pos and a quantity under Menge is a row of the table.
    let typed = pos_table("Muster D", 103.0, true, Some("4")).typed(&text_of_lines(vec![200]));
    assert!(find(&typed, Kind::Totals).is_none(), "{typed:#?}");
}

#[test]
fn touching_boxes_cost_the_boxes_not_their_square() {
    // Two columns of boxes, the second's left edges 2.5 points right of the first's right
    // edges (apart for the drawn lines, within a box's reach): each box looks only at the boxes
    // near it, not at every box of the other column.
    let work = |k: usize| -> Option<u64> {
        let mut texts = Vec::new();
        let mut rules = Vec::new();
        for i in 0..k {
            let top = 1950.0 - 17.0 * i as f32;
            for (c, x) in [(0u32, 72.0f32), (1, 224.5)] {
                let line = 10 + 4 * i as u32 + 2 * c;
                texts.push(run("Feld", x + 2.0, top - 6.0, 6.2, line));
                texts.push(run("Muster", x + 2.0, top - 12.5, 9.2, line + 1));
                rules.extend(box_rules([x, top - 14.0, x + 150.0, top]));
            }
        }
        texts.sort_by_key(|r| r.line);
        let mut page = sheet(texts, rules);
        page.size = [595.0, 2000.0];
        let tree = Tree { page };
        tree.with(|view| {
            let seg = view.segs.all.iter().find(|s| s.source == Source::Box)?;
            let before = work_steps()?;
            let _ = layoutregions::typers::type_label_value(view, seg);
            Some(work_steps()? - before)
        })
    };
    let (Some(small), Some(large)) = (work(25), work(100)) else {
        // A release build counts no work.
        assert!(work_steps().is_none(), "the page has its boxes");
        return;
    };
    // Four times the boxes: about four times the work, far from sixteen.
    assert!(large < 8 * small, "{small} then {large}");
}

// ---- Figures and codes (Task 18) ----

fn bounds(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
    Bounds { x0, y0, x1, y1 }
}

/// `tree` scaled by `k`, its pictures and codes too.
fn scaled_all(tree: &Tree, k: f32) -> Tree {
    let mut out = tree.scaled(k);
    let s = |b: &Bounds| bounds(b.x0 * k, b.y0 * k, b.x1 * k, b.y1 * k);
    out.page.pictures = tree
        .page
        .pictures
        .iter()
        .map(|p| PictureIn {
            bounds: s(&p.bounds),
        })
        .collect();
    out.page.codes = tree
        .page
        .codes
        .iter()
        .map(|c| CodeIn {
            bounds: s(&c.bounds),
        })
        .collect();
    out
}

/// The segment that holds exactly `prims`.
fn holding(prims: Vec<PrimId>) -> impl Fn(&PageView<'_>, &Segment) -> bool {
    move |_, s| s.prims == prims
}

/// Prose, a picture and, when `caption`, a figure caption under it.
fn picture_page(caption: &str) -> Tree {
    let mut texts = Vec::new();
    if !caption.is_empty() {
        texts.push(run(caption, 72.0, 385.0, 9.2, 50));
    }
    let mut page = sheet(texts, Vec::new());
    page.pictures.push(PictureIn {
        bounds: bounds(72.0, 400.0, 372.0, 600.0),
    });
    Tree { page }
}

#[test]
fn a_picture_is_a_figure_and_its_caption_adds_to_it() {
    for (caption, want) in [
        (
            "Abb. 1: Umsatz der Muster GmbH",
            &["figure.picture", "figure.caption"][..],
        ),
        (
            "Abbildung 2 Muster",
            &["figure.picture", "figure.caption"][..],
        ),
        (
            "Figure 3: Muster",
            &["figure.picture", "figure.caption"][..],
        ),
        ("Die Muster GmbH zeigt hier", &["figure.picture"][..]),
        ("", &["figure.picture"][..]),
    ] {
        let tree = picture_page(caption);
        let typed = tree.typed(&holding(vec![PrimId::Picture(0)]));
        assert_eq!(typed.len(), 1, "{caption}: {typed:#?}");
        assert_eq!(ids(of(&typed, Kind::Figure)), want, "{caption}");
        for k in [0.75, 0.8, 1.3, 1.5] {
            let t = scaled_all(&tree, k).typed(&holding(vec![PrimId::Picture(0)]));
            assert_eq!(t, typed, "{caption} at {k}x");
        }
    }
}

/// A drawn chart: a box of rules with no text, a figure caption under it.
#[test]
fn drawn_rules_with_a_figure_caption_are_a_figure() {
    let caption = run("Abb. 4: Werte der Muster GmbH", 72.0, 385.0, 9.2, 50);
    let mut rules = box_rules([72.0, 400.0, 372.0, 600.0]);
    rules.push(rule(100.0, 420.0, 340.0, 580.0));
    let tree = Tree {
        page: sheet(vec![caption], rules),
    };
    let typed = tree.with(|view| {
        let seg = view
            .segs
            .all
            .iter()
            .find(|s| s.children.is_empty() && s.prims.iter().all(|p| matches!(p, PrimId::Rule(_))))
            .expect("a leaf of the rules");
        all_typed(view, seg)
    });
    assert_eq!(ids(of(&typed, Kind::Figure)), ["figure.drawn_caption"]);
    // A lone rule under a caption line is no drawn chart: its box is no figure's size.
    let caption = run("Abb. 4: Werte der Muster GmbH", 72.0, 385.0, 9.2, 50);
    let tree = Tree {
        page: sheet(vec![caption], vec![rule(72.0, 380.0, 372.0, 380.0)]),
    };
    tree.with(|view| {
        for seg in &view.segs.all {
            assert!(type_figure(view, seg).is_none(), "{seg:#?}");
        }
    });
    // Without a caption, rules are no figure.
    let tree = Tree {
        page: sheet(Vec::new(), box_rules([72.0, 400.0, 372.0, 600.0])),
    };
    tree.with(|view| {
        for seg in &view.segs.all {
            assert!(type_figure(view, seg).is_none(), "{seg:#?}");
        }
    });
}

/// A code with its value printed under it, and a picture beside it: each is its own leaf and
/// its own kind; their parent is neither.
fn code_page(readable: &str) -> Tree {
    let mut texts = Vec::new();
    if !readable.is_empty() {
        texts.push(run(readable, 80.0, 288.0, 9.2, 50));
    }
    let mut page = sheet(texts, Vec::new());
    page.codes.push(CodeIn {
        bounds: bounds(72.0, 300.0, 222.0, 340.0),
    });
    page.pictures.push(PictureIn {
        bounds: bounds(300.0, 300.0, 400.0, 340.0),
    });
    Tree { page }
}

#[test]
fn a_code_and_its_readable_line() {
    for (line, want) in [
        (
            "4 012345 678901",
            &["code.symbol", "code.readable_line"][..],
        ),
        ("MUSTER-12345", &["code.symbol", "code.readable_line"][..]),
        ("die Ware der Muster GmbH", &["code.symbol"][..]),
        ("Muster", &["code.symbol"][..]),
        ("SCAN ME", &["code.symbol", "code.readable_line"][..]),
        ("", &["code.symbol"][..]),
    ] {
        let tree = code_page(line);
        let typed = tree.typed(&holding(vec![PrimId::Code(0)]));
        assert_eq!(typed.len(), 1, "{line}: {typed:#?}");
        assert_eq!(ids(of(&typed, Kind::Code)), want, "{line}");
        for k in [0.75, 0.8, 1.3, 1.5] {
            let t = scaled_all(&tree, k).typed(&holding(vec![PrimId::Code(0)]));
            assert_eq!(t, typed, "{line} at {k}x");
        }
        let picture = tree.typed(&holding(vec![PrimId::Picture(0)]));
        assert_eq!(ids(of(&picture, Kind::Figure)), ["figure.picture"]);
    }
    // A segment of a code and a picture is neither: it is the parent of both.
    let mut tree = code_page("");
    tree.page.texts.clear();
    let typed = tree.typed(&holding(vec![PrimId::Picture(0), PrimId::Code(0)]));
    assert!(typed.iter().all(|t| t.kind == Kind::Spatial), "{typed:#?}");
    assert!(find(&typed, Kind::Figure).is_none(), "{typed:#?}");
    assert!(find(&typed, Kind::Code).is_none(), "{typed:#?}");
}

/// A ruled table that lies on a picture is a chart: the refusal is the figure's evidence,
/// ahead of every other kind by the margin. A grid of fields on a picture stays fields: the
/// table rules refuse it for labels and values before they look at the area.
#[test]
fn a_grid_on_a_picture_is_a_figure() {
    let mut tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    tree.page.pictures.push(PictureIn {
        bounds: bounds(60.0, 620.0, 440.0, 710.0),
    });
    let typed = tree.typed(&grid);
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
    assert_eq!(ids(of(&typed, Kind::Figure)), ["figure.on_picture"]);
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Figure, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
    let mut fields = field_grid();
    fields.page.pictures.push(PictureIn {
        bounds: bounds(60.0, 630.0, 530.0, 705.0),
    });
    let typed = fields.typed(&grid);
    assert!(find(&typed, Kind::Figure).is_none(), "{typed:#?}");
    assert_eq!(winner(&typed).0, Kind::LabelValue);
}

/// Figures and codes against their rivals, by the weights alone. A picture and a code never
/// meet on one segment (each is a leaf of its own and their parent is neither), and no text
/// kind fires where there is no text, so their own signals meet no rival: they only need the
/// least score. A figure refusal is on a drawn grid's text and meets the text kinds there; the
/// pairs it cannot lead by the margin (a demoted heading-size paragraph, a heading kept with its
/// title lines, a running band, a legend or a title) are listed in the Task 18 report for 18b.
#[test]
fn worst_case_margins_of_figures_and_codes() {
    use layoutregions::typers::code::{CODE_READABLE, CODE_SYMBOL};
    use layoutregions::typers::figure::{
        FIGURE_CAPTION, FIGURE_DRAWN_CAPTION, FIGURE_PICTURE, FIGURE_REFUSED,
    };
    let min = 300;
    for least in [FIGURE_PICTURE, FIGURE_DRAWN_CAPTION, CODE_SYMBOL] {
        assert!(least >= min && least >= MARGIN);
    }
    const {
        assert!(FIGURE_PICTURE + FIGURE_CAPTION <= 1000 && CODE_SYMBOL + CODE_READABLE <= 1000);
    }
    let paragraph = PARAGRAPH_TEXT_SIZE + PARAGRAPH_JOINS + PARAGRAPH_SENTENCES;
    let list = LIST_MARKER + LIST_ITEMS + LIST_HANGING;
    let heading_in_cells = HEADING_SIZE_LARGE + HEADING_BOLD;
    for (what, loser) in [
        ("paragraph", paragraph),
        ("list", list),
        ("heading in cells", heading_in_cells),
        ("a refused table", 0),
    ] {
        assert!(
            FIGURE_REFUSED >= loser + MARGIN,
            "figure refusal over {what}: {FIGURE_REFUSED} against {loser}"
        );
    }
}

/// A segment of two pictures is no figure of its own: each picture is.
#[test]
fn two_pictures_are_two_figures() {
    let mut page = page(2, Vec::new());
    page.pictures = vec![
        PictureIn {
            bounds: bounds(72.0, 400.0, 272.0, 600.0),
        },
        PictureIn {
            bounds: bounds(300.0, 400.0, 500.0, 600.0),
        },
    ];
    let tree = Tree { page };
    let both = tree.typed(&holding(vec![PrimId::Picture(0), PrimId::Picture(1)]));
    assert!(find(&both, Kind::Figure).is_none(), "{both:#?}");
    for k in 0..2 {
        let one = tree.typed(&holding(vec![PrimId::Picture(k)]));
        assert_eq!(ids(of(&one, Kind::Figure)), ["figure.picture"]);
    }
}

/// A ruled table on a picture of the whole page (a letterhead or a scan under the text) and
/// a ruled table with a cell that opens with "Abb. 3" are no figures: the refusal names why
/// no table was read, but a page frame under a grid or a caption in a cell is no chart.
#[test]
fn a_grid_on_a_page_picture_or_with_a_caption_cell_is_no_figure() {
    let mut tree = ruled_table(ITEM_XS, 700.0, ITEMS, true, 100);
    tree.page.pictures.push(PictureIn {
        bounds: bounds(0.0, 0.0, 595.0, 842.0),
    });
    let typed = tree.typed(&grid);
    assert!(find(&typed, Kind::Figure).is_none(), "{typed:#?}");
    tree.with(|view| {
        let g = view.segs.all.iter().find(|s| grid(view, s)).unwrap();
        assert_eq!(refusal(view, g).unwrap().signal.id, "figure.on_picture");
    });
    let rows: &[&[&str]] = &[
        &["Teil", "Menge", "Zeichnung"],
        &["Muster A", "2", "Abb. 3"],
        &["Muster B", "1", "Abb. 4"],
        &["Muster C", "4", "Abb. 5"],
    ];
    let tree = ruled_table(ITEM_XS, 700.0, rows, true, 100);
    let typed = tree.typed(&grid);
    assert!(find(&typed, Kind::Figure).is_none(), "{typed:#?}");
    tree.with(|view| {
        let g = view.segs.all.iter().find(|s| grid(view, s)).unwrap();
        assert_eq!(refusal(view, g).unwrap().signal.id, "figure.caption_near");
    });
}

/// Lines of `rows` (pieces at `xs`), one line each from line `first` on, 12 points apart.
fn rows_at(rows: &[&[&str]], xs: &[f32], top: f32, size: f32, first: u32) -> Vec<TextRun> {
    let mut texts = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            if !t.is_empty() {
                texts.push(run(t, xs[c], top - 12.0 * k as f32, size, first + k as u32));
            }
        }
    }
    texts
}

/// Task 18b (ruling Q2): a short label with a colon and its value in one run reads as a pair,
/// line by line, at every scale; prose after a colon does not.
#[test]
fn a_colon_label_and_its_value_in_one_run_is_a_pair() {
    let rows: [&[&str]; 3] = [
        &["Kontonummer: 1234 5678 9012"],
        &["Inhaber: Erika Mustermann"],
        &["Bank: Musterbank Musterstadt"],
    ];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0], 600.0, 9.2, 100), Vec::new()),
    };
    for k in [0.75f32, 1.0, 1.5] {
        let typed = tree.scaled(k).typed(&text_of_lines(vec![100, 101, 102]));
        let pairs = of(&typed, Kind::LabelValue);
        assert!(ids(pairs).contains(&"label_value.line_pairs"), "{typed:#?}");
        assert!(ids(pairs).contains(&"label_value.colon"), "{typed:#?}");
        let (kind, lead) = winner(&typed);
        assert_eq!(kind, Kind::LabelValue, "at {k}x: {typed:#?}");
        assert!(lead >= MARGIN, "at {k}x: {typed:#?}");
    }
    // A sentence after a colon is prose.
    let rows: [&[&str]; 1] = [&["Hinweis: die Ware bleibt bis zur Zahlung unser Eigentum."]];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0], 600.0, 9.2, 100), Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100]));
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
}

/// Task 18b (ruling Q2): a lone line of a short label and a value shaped as a code or a
/// number, apart by a gap, is a pair; a lone line of two words is none.
#[test]
fn a_lone_line_of_a_label_and_a_shaped_value_is_a_pair() {
    let rows: [&[&str]; 1] = [&["Rechnungs-Nr.", "2026004711"]];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0, 250.0], 600.0, 9.2, 100), Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100]));
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::LabelValue, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
    let rows: [&[&str]; 1] = [&["Muster GmbH", "Beispiel AG"]];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0, 250.0], 600.0, 9.2, 100), Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100]));
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
}

/// Task 18b (ruling Q5): a block of two columns in one style without a colon or a shaped value
/// on its lines (names beside names) is no block of labels and values.
#[test]
fn key_values_need_evidence_on_every_pair() {
    let rows: [&[&str]; 3] = [
        &["Erika Mustermann", "Musterstadt"],
        &["Max Muster", "Beispielstadt"],
        &["Anna Probe", "Probedorf"],
    ];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0, 250.0], 600.0, 9.2, 100), Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100, 101, 102]));
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
}

/// Task 18b (ruling Q5): blocks side by side that share no column structure are no table: an
/// address down one column beside a field, an address beside a bank. Rows of numbers in
/// columns (lines the detectors without rules left as text) stay a table.
#[test]
fn blocks_side_by_side_are_no_table() {
    let field_and_address: [&[&str]; 3] = [
        &["Personalnummer", "Beispiel AG"],
        &["00000", "Musterweg 1"],
        &["", "12345 Musterstadt"],
    ];
    let address_and_bank: [&[&str]; 3] = [
        &["Muster GmbH", "Musterbank"],
        &["Musterweg 1", "IBAN DE00 0000 0000 0000 0000 00"],
        &["12345 Musterstadt", "BIC MUSTDEXXXXX"],
    ];
    for rows in [&field_and_address[..], &address_and_bank[..]] {
        let tree = Tree {
            page: sheet(rows_at(rows, &[72.0, 250.0], 600.0, 9.2, 100), Vec::new()),
        };
        for k in [0.75f32, 1.0, 1.5] {
            let typed = tree.scaled(k).typed(&text_of_lines(vec![100, 101, 102]));
            assert!(find(&typed, Kind::Table).is_none(), "at {k}x: {typed:#?}");
        }
    }
    let numbers: [&[&str]; 4] = [
        &["113,35", "4120", "64", "30.09", "Gehalt"],
        &["360,00", "4190", "70", "30.09", "Aushilfe"],
        &["238,00", "4145", "71", "30.09", "Zulage"],
        &["30,00", "4145", "72", "30.09", "Fahrgeld"],
    ];
    let tree = Tree {
        page: sheet(
            rows_at(
                &numbers,
                &[72.0, 140.0, 200.0, 250.0, 320.0],
                600.0,
                9.2,
                100,
            ),
            Vec::new(),
        ),
    };
    let typed = tree.typed(&text_of_lines(vec![100, 101, 102, 103]));
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Table, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
}

/// Task 18b (ruling Q7): a line of titles (every word capitalised, stops only after
/// abbreviations) reads as no sentence and no prose.
#[test]
fn a_line_of_titles_is_no_sentence() {
    let rows: [&[&str]; 1] = [&["Nr. Art Tage Std. Satz Brutto"]];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0], 600.0, 9.2, 100), Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100]));
    let paragraph = of(&typed, Kind::Paragraph);
    assert!(
        !ids(paragraph).contains(&"paragraph.sentences"),
        "{typed:#?}"
    );
    assert!(
        !ids(paragraph).contains(&"paragraph.prose_cells"),
        "{typed:#?}"
    );
    // A sentence keeps its evidence.
    let rows: [&[&str]; 1] = [&["Die Ware wird geliefert. Der Betrag ist sofort zu zahlen."]];
    let tree = Tree {
        page: sheet(rows_at(&rows, &[72.0], 600.0, 9.2, 100), Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100]));
    assert!(ids(of(&typed, Kind::Paragraph)).contains(&"paragraph.sentences"));
}

/// Task 18b (ruling Q3): an address in a size that is no text size of the page reads as a
/// paragraph by its joined lines alone, and the walk keeps a line break before each of its
/// lines.
#[test]
fn an_address_in_small_print_is_a_paragraph_with_its_breaks() {
    let rows: [&[&str]; 3] = [&["Muster GmbH"], &["Musterweg 1"], &["12345 Musterstadt"]];
    let mut texts = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        texts.push(run(
            row[0],
            72.0,
            600.0 - 8.0 * k as f32,
            6.7,
            100 + k as u32,
        ));
    }
    let tree = Tree {
        page: sheet(texts, Vec::new()),
    };
    let typed = tree.typed(&text_of_lines(vec![100, 101, 102]));
    let paragraph = of(&typed, Kind::Paragraph);
    assert_eq!(ids(paragraph), ["paragraph.joins"], "{typed:#?}");
    let (kind, _) = winner(&typed);
    assert_eq!(kind, Kind::Paragraph);
    let breaks = tree.with(|view| {
        use layoutregions::extract::text::{TextBlock, TextCx, next_block};
        let runs: Vec<&TextRun> = view
            .page
            .texts
            .iter()
            .filter(|r| r.line >= 100 && r.line < 200)
            .collect();
        let lines = layoutregions::line::text_lines(&runs);
        let levels = view.levels();
        let block = vec![None; lines.len()];
        let cx = TextCx {
            levels: &levels,
            block: &block,
        };
        match next_block(&lines, 0, false, &cx) {
            (TextBlock::Paragraph { breaks }, 3) => breaks,
            other => panic!("{other:?}"),
        }
    });
    assert_eq!(breaks.len(), 2, "{breaks:?}");
}

/// Task 18b (ruling Q6): a text leaf whose walk reads list items and a paragraph is cut at the
/// walk's block boundaries, so each part gets its own kind.
#[test]
fn a_leaf_of_mixed_text_kinds_is_cut_by_the_walk() {
    let rows: [&[&str]; 3] = [
        &["1. Erster Punkt der Liste"],
        &["2. Zweiter Punkt der Liste"],
        &["Die Firma liefert die bestellte Ware."],
    ];
    let page = sheet(rows_at(&rows, &[72.0], 600.0, 9.2, 100), Vec::new());
    let file = [FilePage::of(&page)];
    let facts = learn(&file, false, &no_amount);
    let index = FileIndex::new(&facts);
    let structure = drawn(&page, &|_| true);
    let segs = segment_page(&page, &Limits::default(), &|| false);
    let (segs, typed) =
        layoutregions::typers::type_page(&page, &structure, segs, &facts, &index, &no_amount);
    let lines_of = |s: &Segment| -> Vec<u32> {
        let mut l: Vec<u32> = s
            .prims
            .iter()
            .filter_map(|p| match p {
                PrimId::Text(i) => Some(page.texts[*i as usize].line),
                _ => None,
            })
            .collect();
        l.sort_unstable();
        l.dedup();
        l
    };
    let walks: Vec<(Vec<u32>, Kind)> = segs
        .all
        .iter()
        .filter(|s| s.source == Source::Walk)
        .map(|s| (lines_of(s), winner(&typed[&s.id]).0))
        .collect();
    assert_eq!(
        walks,
        [(vec![100, 101], Kind::List), (vec![102], Kind::Paragraph)],
        "{:#?}",
        segs.all
    );
    let out = arbitrate(&segs, &typed, &Margins::default());
    let mut got: Vec<PrimId> = out
        .iter()
        .flat_map(|d| segs.all[d.seg.0 as usize].prims.iter().copied())
        .collect();
    got.sort_unstable();
    assert_eq!(got, segs.all[segs.root.0 as usize].prims, "the cover");
}

/// Task 18b (ruling Q7): the walk of a segment looks at the page's lines just under it, as the
/// walk over the page does: a bold line at a heading size over more such lines of fields
/// (a box cuts it off them) is no heading but lines apart.
#[test]
fn the_walk_of_a_segment_sees_the_lines_past_its_edge() {
    let lines = ["Kundennummer 12345", "Seite 1 von 2", "Bearbeiter Muster"];
    let texts: Vec<TextRun> = lines
        .iter()
        .enumerate()
        .map(|(k, t)| bold(run(t, 75.0, 600.0 - 16.0 * k as f32, 13.8, 100 + k as u32)))
        .collect();
    // A box around the last two lines parts the first line off them.
    let tree = Tree {
        page: sheet(texts, box_rules([72.0, 560.0, 260.0, 592.0])),
    };
    let typed = tree.typed(&text_of_lines(vec![100]));
    let (kind, _) = winner(&typed);
    assert_eq!(kind, Kind::Paragraph, "{typed:#?}");
    assert!(
        ids(of(&typed, Kind::Paragraph)).contains(&"paragraph.lines_apart"),
        "{typed:#?}"
    );
}

/// Horizontal rules from `x0` to `x1` at each of `ys`.
fn h_rules(x0: f32, x1: f32, ys: &[f32]) -> Vec<RuleIn> {
    ys.iter().map(|&y| rule(x0, y, x1, y)).collect()
}

/// Strips between rules: in each, small labels over their values in columns.
fn strips_page(strips: &[[(&str, &str); 3]], k: f32) -> Tree {
    let xs = [72.0, 180.0, 288.0];
    let mut texts = Vec::new();
    let mut ys = vec![700.0 * k];
    for (s, strip) in strips.iter().enumerate() {
        let top = 700.0 - 24.0 * s as f32;
        for (c, (label, value)) in strip.iter().enumerate() {
            texts.push(run(
                label,
                xs[c] * k,
                (top - 8.0) * k,
                6.2 * k,
                100 + 2 * s as u32,
            ));
            if !value.is_empty() {
                texts.push(run(
                    value,
                    xs[c] * k,
                    (top - 19.0) * k,
                    9.2 * k,
                    101 + 2 * s as u32,
                ));
            }
        }
        ys.push((top - 24.0) * k);
    }
    texts.sort_by_key(|r| r.line);
    let mut page = sheet(texts, h_rules(70.0 * k, 400.0 * k, &ys));
    for r in &mut page.texts {
        if r.line < 100 || r.line >= 900 {
            r.x *= k;
            r.y *= k;
            r.size *= k;
            r.w *= k;
            r.h *= k;
        }
    }
    Tree { page }
}

/// Task 18b round 2: strips of small labels over their values between rules, without
/// dividers, read as fields, ahead of a table and a paragraph by the margin, at every scale;
/// an empty field keeps its label. A value with no label in its strip reads no fields.
#[test]
fn fields_in_strips_between_rules_are_label_value() {
    let strips = [
        [
            ("Personalnummer", "00123"),
            ("Eintritt", "01.01.2020"),
            ("Klasse", "3"),
        ],
        [("Kasse", "Muster BKK"), ("Austritt", ""), ("Kinder", "0,5")],
    ];
    for k in [0.75f32, 1.0, 1.5] {
        let tree = strips_page(&strips, k);
        for strip in [[100, 101], [102, 103]] {
            let typed = tree.typed_holding(&strip);
            let pairs = of(&typed, Kind::LabelValue);
            assert!(
                ids(pairs).contains(&"label_value.strip_fields"),
                "at {k}x: {typed:#?}"
            );
            assert!(find(&typed, Kind::Table).is_none(), "at {k}x: {typed:#?}");
            let (kind, lead) = winner(&typed);
            assert_eq!(kind, Kind::LabelValue, "at {k}x");
            assert!(lead >= MARGIN, "at {k}x: {typed:#?}");
        }
    }
    // A value in the label size alone in a strip beside a field: no strip reading.
    let strips = [[
        ("Personalnummer", "00123"),
        ("Beispielwert", ""),
        ("Eintritt", "01.01.2020"),
    ]];
    let mut tree = strips_page(&strips, 1.0);
    for r in &mut tree.page.texts {
        if r.text == "Beispielwert" {
            r.size = 9.2;
            r.y -= 11.0;
            r.line = 101;
        }
    }
    tree.page.texts.sort_by_key(|r| r.line);
    let typed = tree.typed(&text_of_lines(vec![100, 101]));
    assert!(
        find(&typed, Kind::LabelValue)
            .is_none_or(|t| !ids(t).contains(&"label_value.strip_fields")),
        "{typed:#?}"
    );
}

/// Task 18b round 2: lines that alternate between values and their smaller labels (values
/// over labels) are no rows of a table.
#[test]
fn alternating_labels_and_values_are_no_table() {
    let rows: [(&str, &str, f32); 4] = [
        ("Erika Mustermann", "Musterstadt", 9.2),
        ("Name", "Ort", 6.2),
        ("Musterweg 1", "12", 9.2),
        ("Strasse", "Hausnr", 6.2),
    ];
    let mut texts = Vec::new();
    for (k, (a, b, size)) in rows.iter().enumerate() {
        let y = 650.0 - 9.0 * k as f32;
        texts.push(run(a, 72.0, y, *size, 100 + k as u32));
        texts.push(run(b, 250.0, y, *size, 100 + k as u32));
    }
    let tree = Tree {
        page: sheet(texts, Vec::new()),
    };
    let typed = tree.typed_holding(&[100, 101, 102, 103]);
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
}

/// Task 18b round 2: lines in cells never join into a paragraph, and the pieces of a row in a
/// ruled grid are cells, not a label and its value: the grid reads as a table.
#[test]
fn cells_are_no_paragraph_and_no_pair() {
    let rows: [&[&str]; 2] = [
        &["Steuer A", "1.000,00", "10 %"],
        &["Steuer B", "500,00", "5 %"],
    ];
    let tree = Tree {
        page: sheet(
            rows_at(&rows, &[72.0, 200.0, 320.0], 600.0, 9.2, 100),
            Vec::new(),
        ),
    };
    let typed = tree.typed(&text_of_lines(vec![100, 101]));
    assert!(find(&typed, Kind::Paragraph).is_none(), "{typed:#?}");
    let xs = [72.0, 220.0, 370.0];
    let mut texts = Vec::new();
    for (k, row) in [["Art", "Betrag"], ["Gehalt", "1.234,56"]]
        .iter()
        .enumerate()
    {
        for (c, t) in row.iter().enumerate() {
            texts.push(run(
                t,
                xs[c] + 3.0,
                689.8 - 14.0 * k as f32,
                9.2,
                100 + k as u32,
            ));
        }
    }
    let tree = Tree {
        page: sheet(texts, grid_rules(&xs, &[700.0, 686.0, 672.0])),
    };
    let typed = tree.typed(&text_of_lines(vec![101]));
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
}
