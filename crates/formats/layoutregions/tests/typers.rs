// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typers of text regions: each kind from named signals, positive evidence only, the
//! same scores at every scale, and the current decisions ahead of their rivals by the margin.

use layoutregions::file::{FilePage, learn};
use layoutregions::segment::drawn::drawn;
use layoutregions::segment::{SegId, Segment, Segments, Source, segment_page};
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

/// The worst cases of the kinds the current rules decide between, by the weights alone: each
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

// ---- Tables, totals and label/value groups ----

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

/// A row of boxed values under one title across all their boxes (a date in boxes
/// under `Month Day Year`) has no column titles and no second row: no table. With a second row
/// of values the grid is a table.
#[test]
fn one_row_under_a_title_across_its_boxes_is_no_table() {
    let xs = [300.0f32, 330.0, 360.0, 390.0];
    for rows in [1usize, 2] {
        let ys: Vec<f32> = (0..=rows + 1).map(|k| 600.0 - 14.0 * k as f32).collect();
        let (b, t) = (ys[ys.len() - 1], ys[0]);
        let mut rules: Vec<RuleIn> = ys.iter().map(|&y| rule(xs[0], y, xs[3], y)).collect();
        rules.extend([rule(xs[0], b, xs[0], t), rule(xs[3], b, xs[3], t)]);
        // The inner box edges stand under the title only.
        rules.extend(xs[1..3].iter().map(|&x| rule(x, b, x, ys[1])));
        let mut texts = vec![run("Month Day Year", xs[0] + 3.0, ys[0] - 10.2, 9.2, 100)];
        for r in 0..rows {
            for (c, v) in ["08", "31", "84"].iter().enumerate() {
                texts.push(run(v, xs[c] + 3.0, ys[r + 1] - 10.2, 9.2, 101 + r as u32));
            }
        }
        let tree = Tree {
            page: sheet(texts, rules),
        };
        let typed = tree.typed(&grid);
        assert_eq!(
            find(&typed, Kind::Table).is_some(),
            rows == 2,
            "{rows} rows: {typed:#?}"
        );
    }
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
        role: None,
        group: None,
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
        // Line pairs carry a colon or a shaped value on every line.
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
        // Fields in strips between rules.
        ("strip fields over a table", LABEL_STRIP_FIELDS, table),
        ("strip fields over paragraph", LABEL_STRIP_FIELDS, paragraph),
        (
            "strip fields over a large bold heading that keeps its lines",
            LABEL_STRIP_FIELDS,
            HEADING_SIZE_LARGE + HEADING_BOLD + HEADING_TITLE_LINES,
        ),
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

// ---- Figures and codes ----

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
/// title lines, a running band, a legend or a title) are left to the later typers.
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

/// A ruled table on a picture of the whole page (a letterhead or a scan under the text) is no
/// figure: the refusal names why no table was read, but a page frame under a grid is no chart.
/// A ruled table with cells that hold only "Abb. 3" beside the other cells of their rows is a
/// table (such a cell points to a figure, it captions none); a grid with a caption
/// alone on its line inside it, or with caption words, is refused for it.
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
    assert_eq!(winner(&typed).0, Kind::Table, "{typed:#?}");
    tree.with(|view| {
        let g = view.segs.all.iter().find(|s| grid(view, s)).unwrap();
        assert!(refusal(view, g).is_none());
    });
    // A caption alone on its line in a cell of the grid, and one with its caption words beside
    // other text on its line.
    for last in [
        &["Abb. 3", "", ""][..],
        &["Abb. 3: Muster Teile", "4", "Nord"][..],
    ] {
        let rows: &[&[&str]] = &[
            &["Teil", "Menge", "Zeichnung"],
            &["Muster A", "2", "Nord"],
            &["Muster B", "1", "West"],
            last,
        ];
        let tree = ruled_table(ITEM_XS, 700.0, rows, true, 100);
        tree.with(|view| {
            let g = view.segs.all.iter().find(|s| grid(view, s)).unwrap();
            assert_eq!(
                refusal(view, g).map(|r| r.signal.id),
                Some("figure.caption_near"),
                "{last:?}"
            );
        });
    }
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

/// Lines of a label that ends with a colon and its value
/// a cell gap after it read as pairs among other lines, in the rest of a drawn grid of two
/// columns or more too (the colon says the first piece is a label), but not right under a row
/// of smaller labels (the row of the values of those labels). The size of that row is moved
/// both ways around the size step. A line of two pieces without a colon label, a pair alone
/// by the shape of its value, is cells in such a grid.
#[test]
fn colon_lines_pair_in_a_grid_but_not_under_smaller_labels() {
    let rows: [&[&str]; 2] = [&["Sheet:", "03.04.2026"], &["Pages:", "4"]];
    let block_of = |rows: &[&[&str]]| rows_at(rows, &[72.0, 160.0], 600.0, 10.0, 100);
    let block = || block_of(&rows);
    // What the typers say of the two lines as a segment under `parent`.
    let typed = |page: PageIn, in_grid: bool| {
        Tree { page }.with(|view| {
            let parent = view
                .segs
                .all
                .iter()
                .find(|s| matches!(s.source, Source::Grid { cols, .. } if cols >= 2))
                .map(|s| s.id);
            assert_eq!(parent.is_some(), in_grid, "{:#?}", view.segs.all);
            let prims: Vec<PrimId> = (0..view.page.texts.len())
                .filter(|&i| matches!(view.page.texts[i].line, 100 | 101))
                .map(|i| PrimId::Text(i as u32))
                .collect();
            let seg = Segment {
                id: SegId(u32::MAX),
                bounds: Bounds {
                    x0: 72.0,
                    y0: 580.0,
                    x1: 220.0,
                    y1: 612.0,
                },
                prims,
                source: Source::Rest,
                parent,
                children: Vec::new(),
                alternatives: Vec::new(),
            };
            all_typed(view, &seg)
        })
    };
    let plain = typed(sheet(block(), Vec::new()), false);
    let pairs = of(&plain, Kind::LabelValue);
    assert!(ids(pairs).contains(&"label_value.line_pairs"), "{plain:#?}");
    assert!(ids(pairs).contains(&"label_value.colon"), "{plain:#?}");
    // The same lines as a leaf in a drawn grid of two columns (drawn lower on the page).
    let mut texts = block();
    texts.extend(rows_at(
        &[&["Parcels", "12"], &["Crates", "7"]],
        &[76.0, 206.0],
        410.0,
        10.0,
        200,
    ));
    let below = texts.split_off(block().len());
    let grid = || grid_rules(&[72.0, 200.0, 330.0], &[422.0, 406.0, 392.0]);
    let in_grid = |rows: &[&[&str]]| {
        let mut texts = block_of(rows);
        texts.extend(below.iter().cloned());
        typed(sheet(texts, grid()), true)
    };
    let held = in_grid(&rows);
    let pairs = of(&held, Kind::LabelValue);
    assert!(ids(pairs).contains(&"label_value.line_pairs"), "{held:#?}");
    assert!(ids(pairs).contains(&"label_value.colon"), "{held:#?}");
    // A lone line with a colon label pairs there, the same line without the colon does
    // not, though it pairs on a page without the grid (a label and a number).
    let lone = in_grid(&[&["Sheet:", "4711"]]);
    assert!(find(&lone, Kind::LabelValue).is_some(), "{lone:#?}");
    let bare: [&[&str]; 1] = [&["Sheet", "4711"]];
    let cells = in_grid(&bare);
    assert!(find(&cells, Kind::LabelValue).is_none(), "{cells:#?}");
    let free = typed(sheet(block_of(&bare), Vec::new()), false);
    assert!(find(&free, Kind::LabelValue).is_some(), "{free:#?}");
    // Right under a row of smaller labels: 9.2 under 10 is a size step (1.08), 9.3 is none.
    let under = |size: f32| {
        let mut texts = block();
        texts.extend(rows_at(
            &[&["Kind", "Issued"]],
            &[72.0, 160.0],
            611.0,
            size,
            99,
        ));
        typed(sheet(texts, Vec::new()), false)
    };
    assert!(find(&under(9.2), Kind::LabelValue).is_none());
    assert!(find(&under(9.3), Kind::LabelValue).is_some());
}

/// Two lines alone in one size, the upper one with an underline
/// of its own, are a label over its value. The underline is the evidence: it lies at most a
/// quarter of the size under the baseline and is as long as the words (each end within half
/// a size). Each bound is moved both ways; a longer rule (a line to fill in), an underline
/// under both lines, a net address, a sentence, a smaller lower line and a lower line from
/// another edge give no pair.
#[test]
fn a_label_with_its_own_underline_over_a_line_is_a_pair() {
    // `Checked by` at 72..122 on 600 in size 10, the value on 588.
    let typed = |label: &str, value: (&str, f32, f32), rules: Vec<RuleIn>| {
        let texts = vec![
            run(label, 72.0, 600.0, 10.0, 100),
            run(value.0, value.1, 588.0, value.2, 101),
        ];
        let end = 72.0 + 5.0 * label.chars().count() as f32;
        Tree {
            page: sheet(texts, rules),
        }
        .with(|view| {
            let prims: Vec<PrimId> = (0..view.page.texts.len())
                .filter(|&i| matches!(view.page.texts[i].line, 100 | 101))
                .map(|i| PrimId::Text(i as u32))
                .collect();
            let seg = Segment {
                id: SegId(u32::MAX),
                bounds: Bounds {
                    x0: 72.0,
                    y0: 586.0,
                    x1: end,
                    y1: 610.0,
                },
                prims,
                source: Source::Rest,
                parent: None,
                children: Vec::new(),
                alternatives: Vec::new(),
            };
            all_typed(view, &seg)
        })
    };
    let pairs = |t: &[Typed]| {
        find(t, Kind::LabelValue).is_some_and(|p| ids(p) == ["label_value.underlined_label"])
    };
    let value = ("Rowan/Birch", 72.0, 10.0);
    let under = |y: f32, x0: f32, x1: f32| vec![rule(x0, y, x1, y)];
    let own = typed("Checked by", value, under(599.0, 72.0, 122.0));
    assert!(pairs(&own), "{own:#?}");
    let (kind, lead) = winner(&own);
    assert_eq!(kind, Kind::LabelValue, "{own:#?}");
    assert!(lead >= MARGIN, "{own:#?}");
    // No rule: two lines.
    assert!(!pairs(&typed("Checked by", value, Vec::new())));
    // The drop: 2.4 under the baseline of text of 10 is an underline, 2.6 is none; a rule
    // over the baseline is none.
    assert!(pairs(&typed(
        "Checked by",
        value,
        under(597.6, 72.0, 122.0)
    )));
    assert!(!pairs(&typed(
        "Checked by",
        value,
        under(597.4, 72.0, 122.0)
    )));
    assert!(!pairs(&typed(
        "Checked by",
        value,
        under(600.5, 72.0, 122.0)
    )));
    // The ends: 4.9 past the words on either side is their underline, 5.1 is a longer rule.
    assert!(pairs(&typed(
        "Checked by",
        value,
        under(599.0, 72.0, 126.9)
    )));
    assert!(!pairs(&typed(
        "Checked by",
        value,
        under(599.0, 72.0, 127.1)
    )));
    assert!(pairs(&typed(
        "Checked by",
        value,
        under(599.0, 67.1, 122.0)
    )));
    assert!(!pairs(&typed(
        "Checked by",
        value,
        under(599.0, 66.9, 122.0)
    )));
    assert!(!pairs(&typed(
        "Checked by",
        value,
        under(599.0, 72.0, 116.9)
    )));
    // An underline under both lines says nothing about which is the label.
    let mut both = under(599.0, 72.0, 122.0);
    both.extend(under(587.0, 72.0, 127.0));
    assert!(!pairs(&typed("Checked by", value, both)));
    // The label is plain words: a net address or a code with an underline is none.
    let link = "www.sample.net";
    assert!(!pairs(&typed(link, value, under(599.0, 72.0, 142.0))));
    assert!(!pairs(&typed("AB-2048", value, under(599.0, 72.0, 107.0))));
    assert!(pairs(&typed("Order no.", value, under(599.0, 72.0, 117.0))));
    // The value: no sentence, no label, at most six words.
    let rule_of = || under(599.0, 72.0, 122.0);
    for (text, pair) in [
        ("the crate was sound.", false),
        ("Passed on:", false),
        ("lot 1 lot 2 lot 3", true),
        ("lot 1 lot 2 lot 3 lot 4", false),
        // The value reads as one: at least half of its words hold a digit, a slash or an
        // at sign, or the line is a number, a date or a code. Plain words under an
        // underlined line say nothing about which of the two is the label (M2), and one
        // number among plain words is text under a heading.
        ("call back Monday", false),
        ("call back 12 May", false),
        ("back 12 May", false),
        ("12 May", true),
        ("Room 12", true),
        ("Budget 2026 review", false),
        ("Budget 2026", true),
        ("shelf 3 lamp 4", true),
        ("shelf 3 lamp green desk", false),
        ("shelf 3 lamp 4 desk", false),
        ("shelf 3 lamp 4 desk 5", true),
        ("A/B", true),
        ("N/A", true),
        ("N/A or A/B", true),
        ("see N/A below", false),
        ("03.04.2026", true),
        ("Rowan Birch", false),
        ("rowan@sample.net", true),
        ("write to rowan@sample.net", false),
    ] {
        let t = typed("Checked by", (text, 72.0, 10.0), rule_of());
        assert_eq!(pairs(&t), pair, "{text}: {t:#?}");
    }
    // An underline as long as its words stands under a filled value, a
    // name, a link and a heading as well. With plain words under it the two lines are no
    // pair, on a segment made by hand and on the one the cut makes; with a value under
    // the same upper line they are one.
    let cut = |label: &str, value: &str| {
        let end = 72.0 + 5.0 * label.chars().count() as f32;
        let texts = vec![
            run(label, 72.0, 600.0, 10.0, 100),
            run(value, 72.0, 588.0, 10.0, 101),
        ];
        Tree {
            page: sheet(texts, vec![rule(72.0, 599.0, end, 599.0)]),
        }
        .typed_holding(&[100, 101])
    };
    for (label, value) in [
        ("Alex Sample", "Signature"),
        ("Alex Sample", "Name of the holder"),
        ("Rowan Birch", "Head of Sales"),
        ("Unsubscribe", "Sent by Sample Ltd"),
        ("Background", "The frame and its parts"),
        ("Agenda", "Budget review and staffing"),
    ] {
        let end = 72.0 + 5.0 * label.chars().count() as f32;
        let by_hand = |value: &str| typed(label, (value, 72.0, 10.0), under(599.0, 72.0, end));
        let t = by_hand(value);
        assert!(!pairs(&t), "{label} over {value}: {t:#?}");
        let t = cut(label, value);
        assert!(!pairs(&t), "{label} over {value}, cut: {t:#?}");
        for filled in ["N/A", "A/B", "Room 12"] {
            assert!(pairs(&by_hand(filled)), "{label} over {filled}");
            assert!(pairs(&cut(label, filled)), "{label} over {filled}, cut");
        }
        // One number among plain words is still text under the upper line.
        let text = "Budget 2026 review";
        assert!(!pairs(&by_hand(text)), "{label} over {text}");
        assert!(!pairs(&cut(label, text)), "{label} over {text}, cut");
    }
    // A lower line a size step smaller is a note under a title (9.2 under 10; 9.3 is none).
    assert!(!pairs(&typed(
        "Checked by",
        ("Rowan/Birch", 72.0, 9.2),
        rule_of()
    )));
    assert!(pairs(&typed(
        "Checked by",
        ("Rowan/Birch", 72.0, 9.3),
        rule_of()
    )));
    // The value starts at the label's left edge (a tenth of the size: 1 at 10).
    assert!(pairs(&typed(
        "Checked by",
        ("Rowan/Birch", 72.9, 10.0),
        rule_of()
    )));
    assert!(!pairs(&typed(
        "Checked by",
        ("Rowan/Birch", 73.1, 10.0),
        rule_of()
    )));
}

/// A short label with a colon and its value in one run reads as a pair,
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

/// A lone line of a short label and a value shaped as a code or a
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

/// Items of a list that read `label: value`, each cut into a
/// band of its own by the equal white space between them, are items of their list and no
/// tables of one row: with a bullet set as a letter that every item opens with, or with an
/// enumerator in brackets. The same lines without a marker are pairs, line by line.
#[test]
fn a_list_item_of_a_label_and_its_value_is_no_pair_of_its_own() {
    let lines = |marks: [&str; 4]| -> Vec<TextRun> {
        [
            "shelf height: 120",
            "lamp colour: green",
            "desk width: 80",
            "chair count: 4",
        ]
        .iter()
        .zip(marks)
        .enumerate()
        .map(|(k, (t, m))| {
            run(
                &format!("{m}{t}"),
                72.0,
                600.0 - 18.0 * k as f32,
                9.2,
                100 + k as u32,
            )
        })
        .collect()
    };
    for (marks, pair) in [
        (["o ", "o ", "o ", "o "], false),
        (["(a) ", "(b) ", "(c) ", "(d) "], false),
        (["(1) ", "(2) ", "(3) ", "(4) "], false),
        (["(i) ", "(ii) ", "(iii) ", "(iv) "], false),
        // A short name in brackets is no enumerator.
        (["(EIN) ", "(GLN) ", "(GTIN) ", "(USt) "], true),
        (["", "", "", ""], true),
    ] {
        let tree = Tree {
            page: sheet(lines(marks), Vec::new()),
        };
        for line in [100, 101, 103] {
            let typed = tree.typed(&text_of_lines(vec![line]));
            assert_eq!(
                find(&typed, Kind::LabelValue).is_some(),
                pair,
                "{marks:?} line {line}: {typed:#?}"
            );
        }
    }
}

/// A lone line of a name and a code set a size step smaller beside it (the bureau
/// name of a sample check and its printed `000`) is no pair; the code in the label's size is.
#[test]
fn a_lone_name_beside_a_smaller_code_is_no_pair() {
    let rows: [&[&str]; 1] = [&["United States Treasury", "000"]];
    for (value_size, pair) in [(6.0f32, false), (10.0, true)] {
        let mut texts = rows_at(&rows, &[72.0, 250.0], 600.0, 10.0, 100);
        texts[1].size = value_size;
        texts[1].h = value_size;
        let tree = Tree {
            page: sheet(texts, Vec::new()),
        };
        let typed = tree.typed(&text_of_lines(vec![100]));
        assert_eq!(
            find(&typed, Kind::LabelValue).is_some(),
            pair,
            "{value_size}: {typed:#?}"
        );
    }
}

/// A block of two columns in one style without a colon or a shaped value
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

/// Blocks side by side that share no column structure are no table: an
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

/// A line of titles (every word capitalised, stops only after
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

/// An address in a size that is no text size of the page reads as a
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
            own: &[],
        };
        match next_block(&lines, 0, false, &cx) {
            (TextBlock::Paragraph { breaks }, 3) => breaks,
            other => panic!("{other:?}"),
        }
    });
    assert_eq!(breaks.len(), 2, "{breaks:?}");
}

/// A text leaf whose walk reads list items and a paragraph is cut at the
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
    let (segs, typed) = layoutregions::typers::type_page(
        &page,
        &structure,
        segs,
        &facts,
        &index,
        &no_amount,
        &Limits::default(),
    );
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

/// The walk of a segment looks at the page's lines just under it, as the
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

/// Strips of small labels over their values between rules, without
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

/// Lines that alternate between values and their smaller labels (values
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

/// Lines in cells never join into a paragraph, and the pieces of a row in a
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

/// Running lines, footnote legends and the page title keep their own kinds:
/// a running footer of a label with a colon and its value, and a title line of one, read no
/// label and value.
#[test]
fn running_lines_and_the_title_read_no_pairs() {
    let pages = (1..=3)
        .map(|n| {
            let mut texts = prose(0, 500.0);
            texts.push(run(&format!("Seite: {n} von 3"), 72.0, 30.0, 9.2, 4));
            page(n, texts)
        })
        .collect();
    let footer = Case {
        pages,
        at: 1,
        lines: vec![4],
    };
    let typed = footer.typed();
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
    assert_eq!(winner(&typed).0, Kind::FooterBand, "{typed:#?}");
    let mut texts = vec![run("Rechnung: 2026-0001", 72.0, 800.0, 20.2, 0)];
    texts.extend(prose(1, 740.0));
    let title = Case {
        pages: vec![page(1, texts)],
        at: 0,
        lines: vec![0],
    };
    let typed = title.typed();
    assert!(find(&typed, Kind::LabelValue).is_none(), "{typed:#?}");
    assert_eq!(winner(&typed).0, Kind::TitleBlock, "{typed:#?}");
}

/// Lines the table rules read as a table are no heading, however large
/// they are set: a borderless table in a heading size.
#[test]
fn a_table_in_a_heading_size_is_no_heading() {
    let rows: [&[&str]; 4] = [
        &["Pos", "Artikel", "Betrag"],
        &["1", "Schrauben", "50,00"],
        &["2", "Muttern", "40,00"],
        &["3", "Scheiben", "10,00"],
    ];
    let mut texts = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            texts.push(bold(run(
                t,
                72.0 + 150.0 * c as f32,
                650.0 - 18.0 * k as f32,
                14.2,
                100 + k as u32,
            )));
        }
    }
    let tree = Tree {
        page: sheet(texts, Vec::new()),
    };
    let typed = tree.typed_holding(&[100, 101, 102, 103]);
    assert!(find(&typed, Kind::Table).is_some(), "{typed:#?}");
    assert!(find(&typed, Kind::Heading).is_none(), "{typed:#?}");
}

/// A header and a total row set larger than the rows between them change
/// the size there and back once: the lines stay a table by the margin. Lines whose sizes go
/// and come back more often (values over their labels, a second value line on a label's line)
/// are no table.
#[test]
fn a_header_and_a_total_in_one_size_keep_their_table() {
    let rows: [(&[&str], f32); 5] = [
        (&["Pos", "Artikel", "Betrag"], 10.2),
        (&["1", "Schrauben", "50,00"], 9.2),
        (&["2", "Muttern", "40,00"], 9.2),
        (&["3", "Scheiben", "10,00"], 9.2),
        (&["", "Summe", "100,00"], 10.2),
    ];
    let mut texts = Vec::new();
    for (k, (row, size)) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            if !t.is_empty() {
                texts.push(run(
                    t,
                    72.0 + 150.0 * c as f32,
                    650.0 - 13.0 * k as f32,
                    *size,
                    100 + k as u32,
                ));
            }
        }
    }
    let tree = Tree {
        page: sheet(texts, Vec::new()),
    };
    let typed = tree.typed_holding(&[100, 101, 102, 103, 104]);
    let (kind, lead) = winner(&typed);
    assert_eq!(kind, Kind::Table, "{typed:#?}");
    assert!(lead >= MARGIN, "{typed:#?}");
    // Values, labels, labels, values, labels, values: no strict alternation, but the sizes
    // turn four times.
    let rows: [(&str, &str, f32); 6] = [
        ("Erika Mustermann", "Musterstadt", 9.2),
        ("Name", "Ort", 6.2),
        ("Strasse", "Hausnr", 6.2),
        ("Musterweg 1", "12", 9.2),
        ("PLZ", "Land", 6.2),
        ("12345", "DE", 9.2),
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
    let typed = tree.typed_holding(&[100, 101, 102, 103, 104, 105]);
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
    let spatial = find(&typed, Kind::Spatial).expect("the refusal is named");
    assert_eq!(ids(spatial), ["spatial.interleaved"], "{typed:#?}");
}

/// The decisions of `page` by the typers and the arbitration, with its segments.
fn decide(page: &PageIn) -> (Segments, Vec<Decision>) {
    let file = [FilePage::of(page)];
    let facts = learn(&file, false, &no_amount);
    let index = FileIndex::new(&facts);
    let structure = drawn(page, &|_| true);
    let segs = segment_page(page, &Limits::default(), &|| false);
    let (segs, typed) = layoutregions::typers::type_page(
        page,
        &structure,
        segs,
        &facts,
        &index,
        &no_amount,
        &Limits::default(),
    );
    let out = arbitrate(&segs, &typed, &Margins::default());
    (segs, out)
}

/// The kind of the decision that holds the text of `line`.
fn kind_of_line(page: &PageIn, segs: &Segments, out: &[Decision], line: u32) -> Option<Kind> {
    out.iter()
        .find(|d| {
            segs.all[d.seg.0 as usize]
                .prims
                .iter()
                .any(|p| matches!(*p, PrimId::Text(i) if page.texts[i as usize].line == line))
        })
        .map(|d| d.kind)
}

/// An address under a table in one box stays a paragraph at every scale:
/// the box's table reading only spreads the rows over it. A regression test of the class: it
/// does not fail under the old score rule of `arbitrate` (the box does not outscore its table
/// child here); `a_parent_that_only_spreads_a_child_folds_no_sibling` pins the rule.
#[test]
fn an_address_under_a_table_stays_a_paragraph() {
    for k in [0.75f32, 1.0, 1.5] {
        let rows: [&[&str]; 4] = [
            &["Pos", "Artikel", "Betrag"],
            &["1", "Schrauben", "50,00"],
            &["2", "Muttern", "40,00"],
            &["3", "Scheiben", "10,00"],
        ];
        let mut texts = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (c, t) in row.iter().enumerate() {
                let y = 650.0 - 12.0 * r as f32;
                texts.push(run(
                    t,
                    (80.0 + 150.0 * c as f32) * k,
                    y * k,
                    9.2 * k,
                    100 + r as u32,
                ));
            }
        }
        for (r, t) in ["Muster GmbH", "Musterweg 2", "54321 Musterdorf"]
            .iter()
            .enumerate()
        {
            let y = 580.0 - 11.0 * r as f32;
            texts.push(run(t, 80.0 * k, y * k, 9.2 * k, 110 + r as u32));
        }
        let mut page = sheet(
            texts,
            box_rules([75.0 * k, 540.0 * k, 520.0 * k, 665.0 * k]),
        );
        for r in &mut page.texts {
            if r.line < 100 || r.line >= 900 {
                (r.x, r.y, r.size, r.w, r.h) = (r.x * k, r.y * k, r.size * k, r.w * k, r.h * k);
            }
        }
        let (segs, out) = decide(&page);
        for line in [110, 111, 112] {
            assert_eq!(
                kind_of_line(&page, &segs, &out, line),
                Some(Kind::Paragraph),
                "at {k}x: {out:#?}"
            );
        }
        assert_eq!(
            kind_of_line(&page, &segs, &out, 101),
            Some(Kind::Table),
            "at {k}x"
        );
    }
}

/// A bold header row over the rows of a table without rules is the
/// table's header, not a heading: the parent's table reading names its line as the header row,
/// so the heading typer gives it no candidate and the table holds it, at sizes where a bold
/// line alone would read as a heading. The address under a table stays a paragraph (see
/// [`an_address_under_a_table_stays_a_paragraph`]): the rule is no exception to arbitration.
#[test]
fn a_bold_header_over_its_rows_folds_into_the_table() {
    for k in [1.0f32, 1.25, 1.5] {
        let rows: [&[&str]; 4] = [
            &["Lohnart", "Bezeichnung", "Betrag"],
            &["1000", "Gehalt", "3.000,00"],
            &["1010", "Zulage", "150,00"],
            &["1020", "Fahrgeld", "45,67"],
        ];
        let mut texts = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (c, t) in row.iter().enumerate() {
                let y = 650.0 - 12.0 * r as f32;
                let cell = run(t, 80.0 + 150.0 * c as f32, y, 9.2, 100 + r as u32);
                texts.push(if r == 0 { bold(cell) } else { cell });
            }
        }
        let mut page = sheet(texts, Vec::new());
        // Scaled about the table's top left corner, so the table keeps its place between the
        // prose over it and under it.
        for r in &mut page.texts {
            if r.line < 100 || r.line >= 900 {
                continue;
            }
            (r.x, r.y) = (80.0 + (r.x - 80.0) * k, 650.0 + (r.y - 650.0) * k);
            (r.size, r.w, r.h) = (r.size * k, r.w * k, r.h * k);
        }
        let (segs, out) = decide(&page);
        for line in [100, 101, 103] {
            assert_eq!(
                kind_of_line(&page, &segs, &out, line),
                Some(Kind::Table),
                "line {line} at {k}x: {out:#?}"
            );
        }
    }
}

/// A segment between two rules (a strip) is never a paragraph: its lines
/// are cells or fields.
#[test]
fn a_strip_is_no_paragraph() {
    let rows: [&[&str]; 2] = [
        &["Muster GmbH", "Beispiel AG"],
        &["Musterstadt", "Probedorf"],
    ];
    let mut page = sheet(rows_at(&rows, &[72.0, 250.0], 640.0, 9.2, 100), Vec::new());
    page.rules = vec![
        rule(70.0, 650.0, 400.0, 650.0),
        rule(70.0, 620.0, 400.0, 620.0),
    ];
    let tree = Tree { page };
    let typed = tree.typed(&|_, s| s.source == Source::Strip);
    assert!(find(&typed, Kind::Paragraph).is_none(), "{typed:#?}");
}

/// A label over its value read cell by cell (a label line and a value line,
/// as segments of their own) is no paragraph, so the pairing that is not read is never lost
/// without a word: it stays as laid out. Two words of one style one over the other are.
#[test]
fn a_label_over_its_value_is_no_paragraph() {
    for k in [0.75f32, 1.0, 1.5] {
        let texts = vec![
            run("Folge", 72.0 * k, 650.0 * k, 6.2 * k, 100),
            run("4/9", 72.0 * k, 641.0 * k, 9.2 * k, 101),
        ];
        let mut page = sheet(texts, Vec::new());
        for r in &mut page.texts {
            if r.line < 100 || r.line >= 900 {
                (r.x, r.y, r.size, r.w, r.h) = (r.x * k, r.y * k, r.size * k, r.w * k, r.h * k);
            }
        }
        let tree = Tree { page };
        for line in [100, 101] {
            let typed = tree.typed_holding(&[line]);
            assert!(
                find(&typed, Kind::Paragraph).is_none(),
                "at {k}x: {typed:#?}"
            );
        }
    }
}

/// In a row of boxed fields a box of one label alone (an empty field) is no
/// paragraph, and a box of a company over its address beside the fields stays a paragraph of
/// its own: the grid's field reading leaves it out and never folds it.
#[test]
fn boxes_beside_fields_keep_their_kinds() {
    let xs = [72.0, 222.0, 372.0, 522.0];
    let mut texts = vec![
        run("Personalnummer", 75.0, 683.0, 6.2, 100),
        run("00000", 75.0, 668.0, 9.2, 101),
        run("Steuerklasse", 225.0, 683.0, 6.2, 100),
    ];
    for (k, t) in ["Muster GmbH", "Musterweg 1", "12345 Musterstadt"]
        .iter()
        .enumerate()
    {
        texts.push(run(t, 375.0, 685.0 - 9.0 * k as f32, 7.2, 100 + k as u32));
    }
    texts.sort_by_key(|r| r.line);
    let page = sheet(texts, grid_rules(&xs, &[692.0, 660.0]));
    let (segs, out) = decide(&page);
    let decided = |text: &str| {
        out.iter()
            .find(|d| {
                segs.all[d.seg.0 as usize]
                    .prims
                    .iter()
                    .any(|p| matches!(*p, PrimId::Text(i) if page.texts[i as usize].text == text))
            })
            .map(|d| d.kind)
    };
    assert_ne!(decided("Steuerklasse"), Some(Kind::Paragraph), "{out:#?}");
    assert_eq!(decided("Musterweg 1"), Some(Kind::Paragraph), "{out:#?}");
    assert_eq!(decided("00000"), Some(Kind::LabelValue), "{out:#?}");
}

/// `texts` and `rules` on a sheet, all scaled by `k` (the prose too).
fn sheet_at(texts: Vec<TextRun>, rules: Vec<RuleIn>, k: f32) -> Tree {
    Tree {
        page: sheet(texts, rules),
    }
    .scaled(k)
}

/// A ruled table whose header of two lines (titles over their units) stands
/// between its top rule and its header rule stays a table by the margin: the header strip
/// reads as nothing else.
#[test]
fn a_table_with_a_header_of_two_lines_between_rules_stays_a_table() {
    let rows: [&[&str]; 5] = [
        &["Artikel", "Menge", "Preis"],
        &["", "Stk", "EUR"],
        &["Schrauben", "2", "12,50"],
        &["Muttern", "1", "3,00"],
        &["Scheiben", "4", "8,75"],
    ];
    for k in [0.75f32, 1.0, 1.5] {
        // The table alone on its page: the strip and the rows under it are one segment.
        let mut page = page(2, rows_at(&rows, &[72.0, 220.0, 320.0], 690.0, 9.2, 100));
        page.rules = h_rules(70.0, 420.0, &[700.0, 672.0]);
        let tree = Tree { page }.scaled(k);
        let typed = tree.typed(&|view, s| {
            s.source != Source::Page
                && text_of_lines(vec![100, 101, 102, 103, 104])(view, s)
                && s.children
                    .iter()
                    .any(|c| view.segs.all[c.0 as usize].source == Source::Strip)
        });
        let (kind, lead) = winner(&typed);
        assert_eq!(kind, Kind::Table, "at {k}x: {typed:#?}");
        assert!(lead >= MARGIN, "at {k}x: {typed:#?}");
    }
}

/// A table reading over lines that take in a strip of fields between rules
/// is refused (`spatial.strip_inside`): the strip's fields are its own reading.
#[test]
fn a_table_over_a_strip_of_fields_is_refused() {
    let strips = [[
        ("Personalnummer", "00123"),
        ("Eintritt", "01.01.2020"),
        ("Klasse", "3"),
    ]];
    let rows: [&[&str]; 3] = [
        &["Schrauben", "2", "12,50"],
        &["Muttern", "1", "3,00"],
        &["Scheiben", "4", "8,75"],
    ];
    for k in [0.75f32, 1.0, 1.5] {
        let mut tree = strips_page(&strips, k);
        let under = rows_at(&rows, &[72.0, 180.0, 288.0], 666.0, 9.2, 110);
        tree.page.texts.extend(under.into_iter().map(|r| TextRun {
            x: r.x * k,
            y: r.y * k,
            w: r.w * k,
            h: r.h * k,
            size: r.size * k,
            ..r
        }));
        tree.page.texts.sort_by_key(|r| r.line);
        let wide = tree.with(|view| {
            let seg = view
                .segs
                .all
                .iter()
                .filter(|s| {
                    s.source != Source::Page
                        && s.children
                            .iter()
                            .any(|c| view.segs.all[c.0 as usize].source == Source::Strip)
                })
                .min_by_key(|s| s.prims.len())
                .unwrap_or_else(|| panic!("no parent of the strip: {:#?}", view.segs.all));
            all_typed(view, seg)
        });
        assert!(find(&wide, Kind::Table).is_none(), "at {k}x: {wide:#?}");
        let spatial = find(&wide, Kind::Spatial).expect("the refusal is named");
        assert!(
            ids(spatial).contains(&"spatial.strip_inside"),
            "at {k}x: {wide:#?}"
        );
    }
}

/// A grouped table, its section row of one piece in the header's size
/// between the rows, stays a table by the margin: a section row is no turn of the sizes. A
/// second value line of one piece among values and their labels is no section row.
#[test]
fn a_grouped_table_stays_a_table() {
    let rows: [(&[&str], f32); 8] = [
        (&["Pos", "Artikel", "Betrag"], 10.2),
        (&["1", "Schrauben", "50,00"], 9.2),
        (&["2", "Muttern", "40,00"], 9.2),
        (&["Gruppe B"], 10.2),
        (&["3", "Scheiben", "10,00"], 9.2),
        (&["4", "Bolzen", "20,00"], 9.2),
        (&["5", "Stifte", "5,00"], 9.2),
        (&["", "Summe", "125,00"], 10.2),
    ];
    for k in [0.75f32, 1.0, 1.5] {
        let mut texts = Vec::new();
        for (r, (row, size)) in rows.iter().enumerate() {
            for (c, t) in row.iter().enumerate() {
                if !t.is_empty() {
                    texts.push(run(
                        t,
                        72.0 + 150.0 * c as f32,
                        650.0 - 13.0 * r as f32,
                        *size,
                        100 + r as u32,
                    ));
                }
            }
        }
        let tree = sheet_at(texts, Vec::new(), k);
        let lines: Vec<u32> = (100..108).collect();
        let typed = tree.typed_holding(&lines);
        let (kind, lead) = winner(&typed);
        assert_eq!(kind, Kind::Table, "at {k}x: {typed:#?}");
        assert!(lead >= MARGIN, "at {k}x: {typed:#?}");
    }
    // Values over their labels with a second value line of one piece: no section row, the
    // sizes still turn too often.
    let rows: [(&[&str], f32); 6] = [
        (&["Erika Mustermann", "Musterstadt"], 9.2),
        (&["Name", "Ort"], 6.2),
        (&["Musterweg 1", "12"], 9.2),
        (&["Hinterhaus"], 9.2),
        (&["Strasse", "Hausnr"], 6.2),
        (&["12345", "DE"], 9.2),
    ];
    let mut texts = Vec::new();
    for (r, (row, size)) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            texts.push(run(
                t,
                72.0 + 178.0 * c as f32,
                650.0 - 9.0 * r as f32,
                *size,
                100 + r as u32,
            ));
        }
    }
    let tree = Tree {
        page: sheet(texts, Vec::new()),
    };
    let lines: Vec<u32> = (100..106).collect();
    let typed = tree.typed_holding(&lines);
    assert!(find(&typed, Kind::Table).is_none(), "{typed:#?}");
}

/// The lines of a footnote legend read no label and value, even where an
/// entry holds a label with a colon and its value.
#[test]
fn a_legend_reads_no_pairs() {
    let mut case = legend_case();
    for r in &mut case.pages[0].texts {
        match r.line {
            5 => r.text = "2 Quelle: Musterstadt 2020".into(),
            6 => r.text = "3 Stand: 01.01.2020".into(),
            _ => {}
        }
    }
    for k in [0.75f32, 1.0, 1.5] {
        let typed = case.scaled(k).typed();
        assert!(
            find(&typed, Kind::LabelValue).is_none(),
            "at {k}x: {typed:#?}"
        );
        assert_eq!(
            winner(&typed).0,
            Kind::FootnoteLegend,
            "at {k}x: {typed:#?}"
        );
    }
}

/// A sentence across the columns of a ruled table, set in a heading size,
/// gives no heading candidate: its line is the paragraph's evidence.
#[test]
fn prose_cells_in_a_heading_size_are_no_heading() {
    let (mut tree, sentence) = sentence_over_row(&[5, 3]);
    for r in &mut tree.page.texts {
        if r.line == 104 {
            let x = if r.x > 100.0 { 223.0 } else { r.x };
            *r = bold(run(&r.text, x, 631.0, 12.2, 104));
        }
    }
    for k in [0.75f32, 1.0, 1.5] {
        tree.scaled(k).with(|view| {
            let seg = Segment {
                id: SegId(u32::MAX),
                bounds: Bounds {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 595.0 * k,
                    y1: 842.0 * k,
                },
                prims: sentence.iter().map(|&i| PrimId::Text(i)).collect(),
                source: Source::Rest,
                parent: None,
                children: Vec::new(),
                alternatives: Vec::new(),
            };
            let typed = all_typed(view, &seg);
            assert!(
                ids(of(&typed, Kind::Paragraph)).contains(&"paragraph.prose_cells"),
                "at {k}x: {typed:#?}"
            );
            assert!(find(&typed, Kind::Heading).is_none(), "at {k}x: {typed:#?}");
        });
    }
}

/// Two lines in cells that no table reads never join into a paragraph.
#[test]
fn lines_in_cells_join_no_paragraph() {
    // The pieces of the second line stand off those of the first: no cells aligned.
    let mut texts = rows_at(
        &[&["Muster GmbH", "Beispiel AG"]],
        &[72.0, 250.0],
        640.0,
        9.2,
        100,
    );
    texts.extend(rows_at(
        &[&["Musterstadt", "Probedorf"]],
        &[110.0, 320.0],
        628.0,
        9.2,
        101,
    ));
    for k in [0.75f32, 1.0, 1.5] {
        let tree = sheet_at(texts.clone(), Vec::new(), k);
        let typed = tree.typed(&text_of_lines(vec![100, 101]));
        assert!(find(&typed, Kind::Table).is_none(), "at {k}x: {typed:#?}");
        assert!(
            find(&typed, Kind::Paragraph).is_none(),
            "at {k}x: {typed:#?}"
        );
    }
}

/// A row of a table read on its own (a segment of one line in two cells,
/// inside a segment that reads as a table) is no paragraph.
#[test]
fn a_row_inside_a_table_is_no_paragraph() {
    // The amounts stand right-aligned: the total's amount, wider, shares no left edge with
    // the amount over it, so the total's line is no line of cells aligned with a near line.
    let rows: [&[&str]; 5] = [
        &["Pos", "Artikel", "Betrag"],
        &["1", "Schrauben", "500,00"],
        &["2", "Muttern", "400,00"],
        &["3", "Scheiben", "100,00"],
        &["Summe", "", "1.000,00"],
    ];
    let mut texts = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            if !t.is_empty() {
                let mut x = 72.0 + 150.0 * c as f32;
                let piece = run(t, x, 650.0 - 13.0 * r as f32, 9.2, 100 + r as u32);
                if c == 2 {
                    x = 420.0 - piece.w;
                }
                texts.push(TextRun { x, ..piece });
            }
        }
    }
    for k in [0.75f32, 1.0, 1.5] {
        let tree = sheet_at(texts.clone(), Vec::new(), k);
        let typed = tree.with(|view| {
            let table = view
                .segs
                .all
                .iter()
                .filter(|s| text_of_lines(vec![100, 101, 102, 103, 104])(view, s))
                .max_by_key(|s| s.prims.len())
                .unwrap_or_else(|| panic!("no table segment: {:#?}", view.segs.all));
            assert!(type_table(view, table).is_some(), "at {k}x");
            // The total row, a line of two pieces, as a segment of its own under the table.
            let prims: Vec<PrimId> = table
                .prims
                .iter()
                .copied()
                .filter(
                    |p| matches!(*p, PrimId::Text(i) if view.page.texts[i as usize].line == 104),
                )
                .collect();
            let row = Segment {
                id: SegId(u32::MAX),
                bounds: table.bounds,
                prims,
                source: Source::Rest,
                parent: Some(table.id),
                children: Vec::new(),
                alternatives: Vec::new(),
            };
            all_typed(view, &row)
        });
        assert!(
            find(&typed, Kind::Paragraph).is_none(),
            "at {k}x: {typed:#?}"
        );
    }
}

/// A segment that holds a strip between rules is no text alone: the line
/// readings (key values, line pairs) never read over the strip and the lines under it, so the
/// strip keeps its own decision.
#[test]
fn a_segment_over_a_strip_reads_no_pairs_of_its_own() {
    let lines = [
        ("Konto: 12345", 690.0, 100),
        ("Datum: 01.02.2020", 680.0, 101),
        ("Name: Erika Mustermann", 660.0, 110),
        ("Ort: Musterstadt", 650.0, 111),
    ];
    for k in [0.75f32, 1.0, 1.5] {
        let texts = lines
            .iter()
            .map(|&(t, y, l)| run(t, 72.0, y, 9.2, l))
            .collect();
        let mut page = page(2, texts);
        page.rules = h_rules(70.0, 400.0, &[700.0, 672.0]);
        let tree = Tree { page }.scaled(k);
        let typed = tree.with(|view| {
            view.segs
                .all
                .iter()
                .filter(|s| {
                    s.source != Source::Page
                        && s.children
                            .iter()
                            .any(|c| view.segs.all[c.0 as usize].source == Source::Strip)
                })
                .map(|s| all_typed(view, s))
                .collect::<Vec<_>>()
        });
        assert!(!typed.is_empty(), "at {k}x: no segment over the strip");
        for t in &typed {
            assert!(find(t, Kind::LabelValue).is_none(), "at {k}x: {t:#?}");
        }
    }
}

/// A rule of the left column at a height between the two rules of a strip on
/// the right does not part the strip: each rule pairs with the nearest rule above it that lies
/// over it, and the strip reads as fields.
#[test]
fn a_rule_of_another_column_keeps_a_strip() {
    let fields = [("Personalnummer", "00123"), ("Eintritt", "01.01.2020")];
    for k in [0.75f32, 1.0, 1.5] {
        let mut texts = Vec::new();
        for (c, (label, value)) in fields.iter().enumerate() {
            let x = 330.0 + 100.0 * c as f32;
            texts.push(run(label, x, 690.0, 6.2, 100));
            texts.push(run(value, x, 678.0, 9.2, 101));
        }
        let mut page = sheet(texts, Vec::new());
        page.rules = vec![
            rule(320.0, 700.0, 520.0, 700.0),
            rule(320.0, 670.0, 520.0, 670.0),
            // The left column's rule, between the heights of the strip's rules.
            rule(70.0, 685.0, 250.0, 685.0),
        ];
        let tree = Tree { page }.scaled(k);
        let typed = tree.typed_holding(&[100, 101]);
        let pairs = of(&typed, Kind::LabelValue);
        assert!(
            ids(pairs).contains(&"label_value.strip_fields"),
            "at {k}x: {typed:#?}"
        );
    }
}

/// A strip of fields whose value line is set in runs that stand across its fields: small
/// labels over one line of values between two rules, with a divider of `share` of the strip's
/// height at each field edge, rising from the lower rule. The labels are more words than one
/// label holds (`LABEL_WORDS`), so without dividers nothing shows the line of labels to be
/// fields before the value line is cut.
fn padded_strip(share: f32) -> Tree {
    let (top, bottom) = (700.0, 676.0);
    let mut texts: Vec<TextRun> = [
        ("Sheet", 44.0),
        ("Depot name", 118.0),
        ("Rate %", 289.0),
        ("Gr C", 323.0),
        ("Gr D", 346.0),
        ("Days", 373.0),
    ]
    .iter()
    .map(|(t, x)| run(t, *x, 692.0, 6.6, 100))
    .collect();
    // A print run sets the line in a monospaced face, 5.7 points a character.
    let value = |text: &str, x: f32, gaps: &[(u32, usize, usize)]| TextRun {
        w: 5.7 * text.chars().count() as f32,
        gaps: gaps
            .iter()
            .map(|&(words, from, to)| GapIn {
                words,
                from: x + 5.7 * from as f32,
                to: x + 5.7 * to as f32,
            })
            .collect(),
        ..run(text, x, 679.0, 9.5, 101)
    };
    texts.push(value(
        "12345678X012 Tall Crate",
        42.5,
        &[(1, 12, 13), (2, 17, 18)],
    ));
    texts.push(value(
        "12,34 567 8901 30",
        287.6,
        &[(1, 5, 6), (2, 9, 10), (3, 14, 15)],
    ));
    let mut rules = h_rules(40.0, 400.0, &[top, bottom]);
    for x in [114.0, 285.0, 319.0, 342.0, 370.0] {
        rules.push(rule(x, bottom, x, bottom + share * (top - bottom)));
    }
    Tree {
        page: sheet(texts, rules),
    }
}

/// The dividers a page
/// draws in a strip between rules part its fields. A value line in runs that stand across
/// the fields is cut at the dividers and each piece is the value of the label over it, as
/// the same line reads when each word is a run of its own. A mark shorter than
/// `DIVIDER` of the strip's height is no divider: the strip reads as it does without one.
#[test]
fn drawn_dividers_part_the_fields_of_a_strip() {
    use layoutregions::extract::pairs::strips::DIVIDER;
    let fields = |tree: &Tree| {
        let typed = tree.typed_holding(&[100, 101]);
        find(&typed, Kind::LabelValue).is_some_and(|t| ids(t).contains(&"label_value.strip_fields"))
    };
    for share in [0.75, 0.5, DIVIDER + 0.01] {
        assert!(fields(&padded_strip(share)), "dividers of {share}");
    }
    for share in [DIVIDER - 0.01, 0.1] {
        assert!(!fields(&padded_strip(share)), "marks of {share}");
    }
}

/// Lines that stand in two drawn boxes or more (fields in boxes beside a box of
/// a bold title) are no one paragraph: the box sides part them.
#[test]
fn lines_in_two_boxes_are_no_paragraph() {
    let xs = [72.0, 222.0, 372.0, 522.0];
    let texts = vec![
        run("Personalnummer", 75.0, 683.0, 6.2, 100),
        bold(run("Angaben zum", 225.0, 682.0, 9.2, 100)),
        run("Eintritt", 375.0, 683.0, 6.2, 100),
        bold(run("Betrieb", 225.0, 671.0, 9.2, 101)),
        run("00000", 75.0, 668.0, 9.2, 102),
        run("01.01.2020", 375.0, 668.0, 9.2, 102),
    ];
    let tree = Tree {
        page: sheet(texts, grid_rules(&xs, &[692.0, 660.0])),
    };
    for k in [0.75f32, 1.0, 1.5] {
        let typed = tree.scaled(k).typed(&grid);
        assert!(
            find(&typed, Kind::Paragraph).is_none(),
            "at {k}x: {typed:#?}"
        );
    }
}

/// Lines of `rows` (each its pieces and its size), 150 points a column, `step` points apart,
/// from line 100 on, on a sheet.
fn sized_rows(rows: &[(&[&str], f32)], step: f32) -> Tree {
    let mut texts = Vec::new();
    for (r, (row, size)) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            if !t.is_empty() {
                texts.push(run(
                    t,
                    72.0 + 150.0 * c as f32,
                    650.0 - step * r as f32,
                    *size,
                    100 + r as u32,
                ));
            }
        }
    }
    Tree {
        page: sheet(texts, Vec::new()),
    }
}

/// Values and labels whose sizes turn four times stay refused when a line
/// in a third size follows them: the longest streak of turns counts, not the last one.
#[test]
fn a_line_in_a_third_size_keeps_the_interleaving_refused() {
    let rows: [(&[&str], f32); 7] = [
        (&["Erika Mustermann", "Musterstadt"], 9.2),
        (&["Name", "Ort"], 6.2),
        (&["Strasse", "Hausnr"], 6.2),
        (&["Musterweg 1", "12"], 9.2),
        (&["PLZ", "Land"], 6.2),
        (&["12345", "DE"], 9.2),
        (&["Gesamt", "100,00"], 12.2),
    ];
    for k in [0.75f32, 1.0, 1.5] {
        // The lines as one segment (a block the segmenter may part by its sizes).
        let typed = Case {
            pages: vec![sized_rows(&rows, 10.0).page],
            at: 0,
            lines: (100..107).collect(),
        }
        .scaled(k)
        .typed();
        assert!(find(&typed, Kind::Table).is_none(), "at {k}x: {typed:#?}");
        let spatial = find(&typed, Kind::Spatial).expect("the refusal is named");
        assert_eq!(ids(spatial), ["spatial.interleaved"], "at {k}x: {typed:#?}");
    }
}

/// A line of one piece is a section row only in the header's size. Lines of
/// one piece in a third size between the rows (a size step off the rows and off the header)
/// are turns of the sizes: the lines are refused as interleaved, by that refusal alone.
#[test]
fn a_line_of_one_piece_off_the_header_size_is_no_section_row() {
    let rows: [(&[&str], f32); 9] = [
        (&["Pos", "Artikel", "Betrag"], 9.2),
        (&["1", "Schrauben", "50,00"], 6.2),
        (&["2", "Muttern", "40,00"], 6.2),
        (&["Gruppe B"], 10.2),
        (&["3", "Scheiben", "10,00"], 6.2),
        (&["4", "Bolzen", "20,00"], 6.2),
        (&["Gruppe C"], 10.2),
        (&["5", "Stifte", "5,00"], 6.2),
        (&["6", "Klammern", "3,00"], 6.2),
    ];
    for k in [0.75f32, 1.0, 1.5] {
        // The lines as one segment (a block the segmenter may part by its sizes).
        let typed = Case {
            pages: vec![sized_rows(&rows, 12.0).page],
            at: 0,
            lines: (100..109).collect(),
        }
        .scaled(k)
        .typed();
        assert!(find(&typed, Kind::Table).is_none(), "at {k}x: {typed:#?}");
        let spatial = find(&typed, Kind::Spatial).expect("the refusal is named");
        assert_eq!(ids(spatial), ["spatial.interleaved"], "at {k}x: {typed:#?}");
        assert!(
            find(&typed, Kind::LabelValue).is_none(),
            "at {k}x: {typed:#?}"
        );
    }
}

/// Two lines of one sentence, each in a drawn box of its own, one box
/// over the other, are no paragraph: the box sides part them. The same lines without the
/// boxes read as a paragraph of sentences. One piece a line, of more words than a value, in one size and
/// weight: no rule of cells, labels or fields refuses them.
#[test]
fn a_sentence_over_two_boxes_is_no_paragraph() {
    let lines = [
        "die Ware wird nach dem Erhalt der Rechnung an den Kunden",
        "geliefert und der Betrag wird in zehn Tagen bezahlt.",
    ];
    for boxed in [true, false] {
        let mut texts = prose(0, 800.0);
        texts.push(run(lines[0], 75.0, 690.0, 9.2, 100));
        texts.push(run(lines[1], 75.0, 670.0, 9.2, 101));
        texts.extend(prose(900, 120.0));
        let mut p = page(2, texts);
        if boxed {
            p.rules = box_rules([72.0, 686.0, 400.0, 702.0]);
            p.rules.extend(box_rules([72.0, 666.0, 400.0, 682.0]));
        }
        let case = Case {
            pages: vec![p],
            at: 0,
            lines: vec![100, 101],
        };
        for k in [0.75f32, 1.0, 1.5] {
            let mut scaled = case.scaled(k);
            for r in &mut scaled.pages[0].rules {
                (r.x0, r.y0, r.x1, r.y1) = (r.x0 * k, r.y0 * k, r.x1 * k, r.y1 * k);
            }
            let typed = scaled.typed();
            if boxed {
                assert!(
                    find(&typed, Kind::Paragraph).is_none(),
                    "at {k}x: {typed:#?}"
                );
            } else {
                let paragraph = of(&typed, Kind::Paragraph);
                assert!(
                    ids(paragraph)
                        .iter()
                        .any(|i| *i == "paragraph.joins" || *i == "paragraph.sentences"),
                    "at {k}x: {typed:#?}"
                );
            }
        }
    }
}

/// A heading in two pieces a line next to a figure caption keeps its heading
/// candidate: the table rule refuses its lines for the figure (`figure.caption_near`), but
/// that refusal weighs nothing and claims no text.
#[test]
fn a_heading_by_a_figure_caption_stays_a_heading() {
    let mut texts = prose(0, 800.0);
    texts.push(bold(run("3.1", 72.0, 700.0, 14.2, 100)));
    texts.push(bold(run("Ergebnisse", 160.0, 700.0, 14.2, 100)));
    texts.push(bold(run("der", 72.0, 682.0, 14.2, 101)));
    texts.push(bold(run("Messung", 160.0, 682.0, 14.2, 101)));
    texts.push(run("Abb. 3: Verlauf der Messung", 72.0, 664.0, 9.2, 102));
    texts.extend(prose(900, 120.0));
    let tree = Tree {
        page: page(2, texts),
    };
    for k in [0.75f32, 1.0, 1.5] {
        tree.scaled(k).with(|view| {
            let prims: Vec<PrimId> = view
                .page
                .texts
                .iter()
                .enumerate()
                .filter(|(_, r)| r.line == 100 || r.line == 101)
                .map(|(i, _)| PrimId::Text(i as u32))
                .collect();
            let seg = Segment {
                id: SegId(u32::MAX),
                bounds: Bounds {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 595.0 * k,
                    y1: 842.0 * k,
                },
                prims,
                source: Source::Rest,
                parent: None,
                children: Vec::new(),
                alternatives: Vec::new(),
            };
            let r = refusal(view, &seg).expect("the table rule refuses the lines");
            assert_eq!(r.signal.id, "figure.caption_near", "at {k}x");
            assert_eq!(r.signal.weight, 0, "at {k}x");
            let typed = all_typed(view, &seg);
            assert!(find(&typed, Kind::Heading).is_some(), "at {k}x: {typed:#?}");
        });
    }
}

/// A name with three lines under it, and what stands at the right of the name's line: on
/// line 10 the name and `beside` at `x` in 8 points, on lines 11 to 13 the lines under the
/// name in 6.6 points (a name over its address, as on the page of the public file), the
/// middle one `middle`. Lines of text in both sizes stand elsewhere on the page, so both are
/// text sizes of it and each line has the evidence of a paragraph by its size.
fn name_block(beside: &str, x: f32, middle: &str, under: usize) -> Case {
    let mut texts = Vec::new();
    let t = "The works at the harbor send the parts on the first day of the week.";
    // More ink in 8 points: the body size of the file, so the name is no heading.
    for k in 0..6u32 {
        texts.push(run(t, 40.0, 800.0 - 10.0 * k as f32, 8.0, k));
    }
    for k in 0..4u32 {
        texts.push(run(t, 40.0, 500.0 - 8.0 * k as f32, 6.6, 6 + k));
    }
    texts.push(run("Alex Sample", 72.0, 700.0, 8.0, 10));
    texts.push(run(beside, x, 700.0, 8.0, 10));
    let lines = ["Oakfield Works", middle, "alex@works.example"];
    for (k, t) in lines.iter().take(under).enumerate() {
        texts.push(run(t, 72.0, 692.0 - 8.0 * k as f32, 6.6, 11 + k as u32));
    }
    Case {
        pages: vec![page(1, texts)],
        at: 0,
        lines: (10..11 + under as u32).collect(),
    }
}

/// Two blocks side by side are not joined into one line (page 7 of the public file
/// arxiv-2408.09869, a row of two author names over the lines of the first): a second name
/// that stands right of every line under the first is another block, and the lines are no
/// paragraph, at every scale. The paragraph stands when a line under the name reaches under
/// the second piece (a value over the text of its block), when one line only stands under
/// the name, and when the second piece is a word of the line.
#[test]
fn a_block_beside_a_block_is_no_line_of_its_paragraph() {
    let paragraph = |c: &Case, k: f32| find(&c.scaled(k).typed(), Kind::Paragraph).is_some();
    for k in [0.5f32, 1.0, 2.0] {
        // The lines under the name end at 72 + 18 * 3.3 = 131.4; the second name starts
        // at 250.
        let apart = name_block("Robin Example", 250.0, "Harbor Road 12", 3);
        assert!(!paragraph(&apart, k), "{k}: {:#?}", apart.scaled(k).typed());
        // A line under the name that reaches under the second piece: one block.
        let long = "Harbor Road 12 in the town of Oakfield West by the river and the mill";
        assert!(
            paragraph(&name_block("Robin Example", 250.0, long, 3), k),
            "{k}"
        );
        // The middle line ends at 72 + 52 * 3.3 = 243.6 with this text, less than a cell
        // gap (one size of the name's line, 8 points) before the second name at 250, and at
        // 240.3 with one letter less, more than a cell gap before it: the limit both ways.
        let near = "Harbor Road 12 in the town of Oakfield West by river";
        assert_eq!(near.chars().count(), 52);
        assert!(
            paragraph(&name_block("Robin Example", 250.0, near, 3), k),
            "{k}"
        );
        assert!(
            !paragraph(&name_block("Robin Example", 250.0, &near[..51], 3), k),
            "{k}"
        );
        // One line under the name is no column, two are: lines in the name's size, each a
        // paragraph of its own three sizes under the line before (a size step apart, a name
        // over one line is a label over its value and no paragraph either way).
        let spaced = |n: usize| {
            let mut c = name_block("Robin Example", 250.0, "Harbor Road 12", 0);
            for k in 0..n {
                let (y, line) = (676.0 - 24.0 * k as f32, 11 + k as u32);
                c.pages[0]
                    .texts
                    .push(run("Oakfield Works", 72.0, y, 8.0, line));
                c.lines.push(line);
            }
            c
        };
        assert!(paragraph(&spaced(1), k), "{k}");
        assert!(!paragraph(&spaced(2), k), "{k}");
        // The next word of the line, a word space on: one line of one block.
        assert!(
            paragraph(&name_block("of Oakfield", 119.0, "Harbor Road 12", 3), k),
            "{k}"
        );
    }
}

/// A colon that opens the second piece of a
/// line as a word of its own (`Currency` and `: EUR`) is the colon of the label only when
/// another line of the block has a colon in that column: a listing in a fixed-width face
/// pads its labels up to a column of colons, and one colon before a cell is chance. The
/// other line may be padded too, or have its colon inside its one run, as a word or at the
/// end of its label. Without the column the line is a pair only as a lone line whose value
/// is shaped as a number, a date or a code, and then not by its colon.
#[test]
fn a_colon_that_opens_the_value_needs_a_column_of_colons() {
    /// One run: its text, its left edge, its baseline and its line. Text of 10 points, 5
    /// points a character.
    type Part = (&'static str, f32, f32, u32);
    // The signals of the label and value typer for the leaf that holds line 1.
    let read = |parts: &[Part]| -> Option<Vec<&'static str>> {
        let mut texts: Vec<TextRun> = parts
            .iter()
            .map(|&(t, x, y, line)| run(t, x, y, 10.0, line))
            .collect();
        texts.extend(prose(20, 500.0));
        let tree = Tree {
            page: page(1, texts),
        };
        tree.with(|view| {
            let seg = view
                .segs
                .all
                .iter()
                .find(|s| {
                    s.children.is_empty()
                        && s.prims.iter().any(
                            |p| matches!(p, PrimId::Text(i) if view.page.texts[*i as usize].line == 1),
                        )
                })
                .expect("a leaf with line 1");
            find(&all_typed(view, seg), Kind::LabelValue).map(ids)
        })
    };
    let colon = |signals: Option<Vec<&'static str>>| {
        signals.is_some_and(|s| s.contains(&"label_value.colon"))
    };
    // The line: its colon at 115.
    let (label, value): (Part, Part) = (("Currency ", 40.0, 700.0, 1), (": EUR", 115.0, 700.0, 1));
    // Alone on the page: one colon is chance, and `EUR` no value by its shape.
    assert_eq!(read(&[label, value]), None);
    // With a number it is a pair as a lone line, by the shape of the value and no colon.
    let number = read(&[label, (": 549910", 115.0, 700.0, 1)]).expect("a lone pair");
    assert!(number.contains(&"label_value.value_shape"), "{number:?}");
    assert!(!number.contains(&"label_value.colon"), "{number:?}");
    // Another padded line with its colon in the column: under it and over it.
    let other = |x: f32, y: f32| -> [Part; 2] { [("Region ", 40.0, y, 2), (": north", x, y, 2)] };
    for y in [688.0, 712.0] {
        let [a, b] = other(115.0, y);
        assert!(colon(read(&[label, value, a, b])), "{y}");
        assert!(colon(read(&[a, b, label, value])), "{y}");
    }
    // A line in one run with its colon there: a word of its own (15 characters before
    // it), or at the end of its label.
    assert!(colon(read(&[
        label,
        value,
        ("Profile in use : basic", 40.0, 688.0, 2)
    ])));
    assert!(colon(read(&[
        label,
        value,
        ("Profile in use: basic", 45.0, 688.0, 2)
    ])));
    // A colon inside a word there (a time) is no colon of a label; nor is a line with no
    // colon, with which the line is one of several and no pair at all.
    assert_eq!(
        read(&[label, value, ("Opens at nine or 10:30", 20.0, 688.0, 2)]),
        None
    );
    assert_eq!(
        read(&[label, value, ("Region north", 40.0, 688.0, 2)]),
        None
    );
    // The column: within `EDGE_SIZES` of the text size (1 point at 10).
    let [a, b] = other(115.9, 688.0);
    assert!(colon(read(&[label, value, a, b])));
    let [a, b] = other(116.1, 688.0);
    assert_eq!(read(&[label, value, a, b]), None);
    let [a, b] = other(113.9, 688.0);
    assert_eq!(read(&[label, value, a, b]), None);
    // The block: a line at most `ITEM_PITCH` sizes (20 points) off.
    let [a, b] = other(115.0, 680.1);
    assert!(colon(read(&[label, value, a, b])));
    let [a, b] = other(115.0, 679.9);
    assert_eq!(read(&[label, value, a, b]), None);
}

/// The decisions of the engine on a page of `parts` (each its text, left edge, baseline,
/// size and line) over a block of prose, as the PDF reader's layout makes them: the kind of
/// each region that holds a part, with its parts in the order of the region. Every part
/// stands in exactly one region.
fn decided(parts: &[(&'static str, f32, f32, f32, u32)]) -> Vec<(Kind, Vec<&'static str>)> {
    let texts = parts
        .iter()
        .map(|&(t, x, y, size, line)| run(t, x, y, size, line))
        .collect();
    decided_on(texts, Vec::new())
        .into_iter()
        .map(|(kind, ids)| (kind, ids.into_iter().map(|t| parts[t].0).collect()))
        .collect()
}

/// The decisions of the engine on a page of the runs `own` and the `rules` over a block of
/// prose (see [`decided`]): the kind of each region that holds a run of `own`, with the
/// places of its runs in `own` in the order of the region. Every run of `own` stands in
/// exactly one region.
fn decided_on(own: Vec<TextRun>, rules: Vec<RuleIn>) -> Vec<(Kind, Vec<usize>)> {
    use layoutregions::segment::drawn::grids;
    use layoutregions::segment::segment_page_drawn;
    use layoutregions::typers::type_page;
    let n = own.len();
    let mut texts = own;
    // Prose far under the parts: they are a group of the page, not the page.
    texts.extend(prose(50, 500.0));
    let mut p = page(1, texts);
    p.rules = rules;
    let file = [FilePage::of(&p)];
    let facts = learn(&file, false, &no_amount);
    let index = FileIndex::new(&facts);
    let limits = Limits::default();
    let (segs, drawn) = segment_page_drawn(&p, &grids(&p.rules), &limits, &|| false);
    let drawn = drawn.expect("no stop");
    let (segs, typed) = type_page(&p, &drawn, segs, &facts, &index, &no_amount, &limits);
    let mut seen = vec![0; n];
    let out: Vec<(Kind, Vec<usize>)> = arbitrate(&segs, &typed, &Margins::of(&limits))
        .into_iter()
        .map(|d| {
            let texts = segs.all[d.seg.0 as usize]
                .prims
                .iter()
                .filter_map(|p| match *p {
                    PrimId::Text(t) if (t as usize) < n => Some(t as usize),
                    _ => None,
                })
                .inspect(|&t| seen[t] += 1)
                .collect();
            (d.kind, texts)
        })
        .filter(|d: &(Kind, Vec<usize>)| !d.1.is_empty())
        .collect();
    assert!(seen.iter().all(|&n| n == 1), "{seen:?} in {out:?}");
    out
}

/// Asserts that no region of `read` is a label with its value, and that `label` stands
/// right before `after` in one region.
#[track_caller]
fn plain_line(read: &[(Kind, Vec<&str>)], label: &str, after: &str) {
    assert!(read.iter().all(|r| r.0 != Kind::LabelValue), "{read:?}");
    let together = read
        .iter()
        .any(|r| r.1.windows(2).any(|w| w == [label, after]));
    assert!(together, "{label} {after} in {read:?}");
}

/// A small colon label on a data line whose own
/// value stands under it, a little farther than a line pitch. The text after the label on
/// its line was read as its value (`Staff no.: | Page 1`, the wrong value). Now no pair is
/// claimed: the label and the text after it are one plain line, at every drop of the value
/// under the label.
#[test]
fn a_label_on_a_data_line_with_its_value_under_it_claims_no_pair() {
    for drop in [11.0f32, 14.0, 15.5, 16.0, 18.0, 20.0] {
        let read = decided(&[
            ("Ledger sheet", 50.0, 700.0, 10.0, 0),
            ("Staff no.:", 260.0, 700.0, 7.0, 0),
            ("Page 1", 310.0, 700.0, 10.0, 0),
            ("4711", 260.0, 700.0 - drop, 10.0, 1),
        ]);
        plain_line(&read, "Staff no.:", "Page 1");
        // The number under the label is in no region with the label.
        assert!(
            read.iter()
                .all(|r| !(r.1.contains(&"4711") && r.1.contains(&"Staff no.:"))),
            "{drop}: {read:?}"
        );
        let read = decided(&[
            ("AB 12", 50.0, 700.0, 10.0, 0),
            ("Amount:", 200.0, 700.0, 7.0, 0),
            ("4711/0815", 232.0, 700.0, 10.0, 0),
            ("03.04.2026", 50.0, 700.0 - drop, 10.0, 1),
            ("12,50", 200.0, 700.0 - drop, 10.0, 1),
        ]);
        plain_line(&read, "Amount:", "4711/0815");
        // A head of two stretches (a title or a code, then a date): no pair of them either.
        for head in ["Ledger", "AB 12"] {
            let read = decided(&[
                (head, 50.0, 700.0, 10.0, 0),
                ("03.04.2026", 120.0, 700.0, 10.0, 0),
                ("Staff no.:", 260.0, 700.0, 7.0, 0),
                ("Page 1", 310.0, 700.0, 10.0, 0),
                ("4711", 260.0, 700.0 - drop, 10.0, 1),
            ]);
            plain_line(&read, "Staff no.:", "Page 1");
            plain_line(&read, head, "03.04.2026");
        }
    }
}

/// Rows of a table with a blank
/// labelled field before the amount column. The amount of each row stands under the column
/// title, and was read as the value of the label (`Part no.: | 12,50`). No pair is claimed,
/// and the row is not parted at its label either: with a second row in its leaf, a line
/// that shares its columns, each row stays one row, its cells in their order in one region,
/// with and without the row of titles. A row alone in its leaf, with nothing there that
/// shows its columns, is a data line: the label and the amount are one plain line.
#[test]
fn a_blank_labelled_field_in_a_table_row_claims_no_pair_and_parts_no_row() {
    type Part = (&'static str, f32, f32, f32, u32);
    let titles: [Part; 3] = [
        ("Pos", 50.0, 714.0, 10.0, 0),
        ("Description", 90.0, 714.0, 10.0, 0),
        ("Amount", 300.0, 714.0, 10.0, 0),
    ];
    let row = |pos: &'static str, what: &'static str, amount: &'static str, y: f32, line| {
        let row: [Part; 4] = [
            (pos, 50.0, y, 10.0, line),
            (what, 90.0, y, 10.0, line),
            ("Part no.:", 250.0, y, 7.0, line),
            (amount, 300.0, y, 10.0, line),
        ];
        row
    };
    // The cells of a row stand in their order in one region.
    #[track_caller]
    fn one_row(read: &[(Kind, Vec<&str>)], row: &[(&'static str, f32, f32, f32, u32); 4]) {
        let cells = row.map(|c| c.0);
        assert!(read.iter().all(|r| r.0 != Kind::LabelValue), "{read:?}");
        let whole = read.iter().any(|r| r.1.windows(4).any(|w| w == cells));
        assert!(whole, "{cells:?} in {read:?}");
    }
    let first = row("1", "Widget", "12,50", 700.0, 1);
    let second = row("2", "Gadget", "8,00", 686.0, 2);
    // The row alone.
    let read = decided(&first);
    plain_line(&read, "Part no.:", "12,50");
    assert!(
        read.iter()
            .all(|r| !(r.1.contains(&"Widget") && r.1.contains(&"Part no.:"))),
        "{read:?}"
    );
    // Over a second row, with their titles and without.
    let mut parts: Vec<Part> = titles.to_vec();
    parts.extend(first);
    // One row under its titles: the cut makes the titles a leaf of their own, so the row is
    // alone in its leaf and is a data line as the row alone is.
    plain_line(&decided(&parts), "Part no.:", "12,50");
    parts.extend(second);
    let read = decided(&parts);
    one_row(&read, &first);
    one_row(&read, &second);
    let rows: Vec<Part> = first.into_iter().chain(second).collect();
    let read = decided(&rows);
    one_row(&read, &first);
    one_row(&read, &second);
    // A line over the row that shares one column with it (the amount's) shows no table: a
    // line of codes over a data line ends where the data line ends.
    let mut parts: Vec<Part> = vec![("AB/12 345", 280.0, 714.0, 10.0, 0)];
    parts.extend(first);
    let read = decided(&parts);
    plain_line(&read, "Part no.:", "12,50");
    assert!(
        read.iter()
            .all(|r| !(r.1.contains(&"Widget") && r.1.contains(&"Part no.:"))),
        "{read:?}"
    );
}

/// Two small colon labels that touch, the first
/// field blank, before one number. The number was read into the value of the first label
/// (`Sheet: | Page: 2`). Now no pair is claimed: the two labels and the number are one
/// plain line in their order. With the labels far apart, each a stretch of its own, the
/// last label stands with the number and claims no pair either.
#[test]
fn two_labels_before_one_number_on_a_data_line_claim_no_pair() {
    let read = decided(&[
        ("03.04.2026", 50.0, 700.0, 10.0, 0),
        ("Sheet:", 270.0, 700.0, 7.0, 0),
        ("Page:", 294.0, 700.0, 7.0, 0),
        ("2", 316.0, 700.0, 10.0, 0),
    ]);
    plain_line(&read, "Sheet:", "Page:");
    plain_line(&read, "Page:", "2");
    let read = decided(&[
        ("03.04.2026", 50.0, 700.0, 10.0, 0),
        ("Sheet:", 200.0, 700.0, 7.0, 0),
        ("Page:", 294.0, 700.0, 7.0, 0),
        ("2", 316.0, 700.0, 10.0, 0),
    ]);
    plain_line(&read, "Page:", "2");
    // A head of two stretches (a title or a code, then a date) before the labels: the date
    // is no value of the title or of the code.
    for head in ["Ledger", "AB 12"] {
        for sheet in [270.0, 200.0] {
            let read = decided(&[
                (head, 50.0, 700.0, 10.0, 0),
                ("03.04.2026", 110.0, 700.0, 10.0, 0),
                ("Sheet:", sheet, 700.0, 7.0, 0),
                ("Page:", 294.0, 700.0, 7.0, 0),
                ("2", 316.0, 700.0, 10.0, 0),
            ]);
            plain_line(&read, "Page:", "2");
            plain_line(&read, head, "03.04.2026");
        }
    }
}

/// The head of a data line that is parted at a
/// small colon label claims no pair either. A title or a code and then a date before the
/// label were read as a label with its value (`Ledger | 03.04.2026`, `AB 12 | 03.04.2026`):
/// the date is no value of the title. Now the head stands alone as a plain line, its
/// stretches in their order, on every page of the kind: with text beside the line, with a
/// value under the label at each drop, with two labels before one number and with a line of
/// codes over the data line.
#[test]
fn the_head_of_a_parted_data_line_claims_no_pair() {
    type Part = (&'static str, f32, f32, f32, u32);
    #[track_caller]
    fn head_alone(parts: &[Part], head: [&str; 2]) {
        let read = decided(parts);
        assert!(read.iter().all(|r| r.0 != Kind::LabelValue), "{read:?}");
        let together = read.iter().any(|r| r.1.windows(2).any(|w| w == head));
        assert!(together, "{head:?} in {read:?}");
    }
    let date: Part = ("03.04.2026", 110.0, 700.0, 10.0, 0);
    let code: Part = ("AB 12", 50.0, 700.0, 10.0, 0);
    let title: Part = ("Ledger", 50.0, 700.0, 10.0, 0);
    let sheet: [Part; 2] = [
        ("Sheet:", 270.0, 700.0, 7.0, 0),
        ("1", 300.0, 700.0, 10.0, 0),
    ];
    let staff: [Part; 2] = [
        ("Staff no.:", 260.0, 700.0, 7.0, 0),
        ("Page 1", 310.0, 700.0, 10.0, 0),
    ];
    // E1: text beside the line on its baseline, another line of the host.
    let mut e1 = vec![code, date];
    e1.extend(sheet);
    e1.push(("Page", 500.0, 700.0, 10.0, 1));
    head_alone(&e1, ["AB 12", "03.04.2026"]);
    // E2 and E3: a value under the label.
    let mut e2 = vec![title, date];
    e2.extend(sheet);
    e2.push(("4", 270.0, 688.0, 10.0, 1));
    head_alone(&e2, ["Ledger", "03.04.2026"]);
    let mut e3 = vec![code, date];
    e3.extend(staff);
    e3.push(("4711", 260.0, 688.0, 10.0, 1));
    head_alone(&e3, ["AB 12", "03.04.2026"]);
    // D1: the value under the label at each drop.
    for drop in [12.0f32, 14.0, 16.0] {
        let mut d1 = vec![title, ("03.04.2026", 120.0, 700.0, 10.0, 0)];
        d1.extend(staff);
        d1.push(("4711", 260.0, 700.0 - drop, 10.0, 1));
        head_alone(&d1, ["Ledger", "03.04.2026"]);
    }
    // E4: two labels that touch before one number.
    for head in [title, code] {
        head_alone(
            &[
                head,
                date,
                ("Sheet:", 270.0, 700.0, 7.0, 0),
                ("Page:", 294.0, 700.0, 7.0, 0),
                ("2", 316.0, 700.0, 10.0, 0),
            ],
            [head.0, "03.04.2026"],
        );
    }
    // C6: a line of codes over the data line, which shares one column with it.
    let mut c6 = vec![("CD/34 567", 280.0, 714.0, 10.0, 0)];
    c6.extend([
        ("AB 12", 50.0, 700.0, 10.0, 1),
        ("03.04.2026", 110.0, 700.0, 10.0, 1),
        ("Sheet:", 270.0, 700.0, 7.0, 1),
        ("1", 300.0, 700.0, 10.0, 1),
    ]);
    head_alone(&c6, ["AB 12", "03.04.2026"]);
}

/// A strip between rules with a drawn divider in the digit group
/// gap of a number claims no pair: the digits say one number and the divider says two fields.
/// Every run stands in one region, and none is a label with its value. With the divider out
/// of reach of the gap the strip reads as fields, the amount whole.
#[test]
fn a_strip_with_a_divider_in_a_digit_group_gap_claims_no_pair() {
    // Small labels over one value line between two rules from 40 to 220, parted by a divider;
    // the number starts at 130, its group gap 1.6 wide after its first group.
    let read = |number: &str, divider: f32, count: Option<&str>| {
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            w: 0.6 * size * text.chars().count() as f32,
            font_kind: FontKind::Monospace,
            ..run(text, x, y, size, line)
        };
        let first = number.chars().take_while(char::is_ascii_digit).count() as f32;
        let mut amount = sized(number, 130.0, 679.0, 9.5, 1);
        amount.w -= 4.1;
        amount.gaps = vec![GapIn {
            words: 1,
            from: 130.0 + 5.7 * first,
            to: 130.0 + 5.7 * first + 1.6,
        }];
        let mut texts = vec![
            sized("Count", 44.0, 692.0, 6.6, 0),
            sized("Amount", divider + 12.0, 692.0, 6.6, 0),
        ];
        texts.extend(count.map(|c| sized(c, 44.0, 679.0, 9.5, 1)));
        texts.push(amount);
        let mut rules = h_rules(40.0, 220.0, &[700.0, 675.0]);
        rules.push(rule(divider, 675.0, divider, 700.0));
        let names: Vec<String> = texts.iter().map(|t| t.text.clone()).collect();
        let out: Vec<(Kind, Vec<String>)> = decided_on(texts, rules)
            .into_iter()
            .map(|(kind, ids)| (kind, ids.iter().map(|&t| names[t].clone()).collect()))
            .collect();
        out
    };
    let amount = "1\u{2009}234,56";
    for count in [None, Some("7")] {
        for divider in [135.0, 136.0] {
            let out = read(amount, divider, count);
            assert!(
                out.iter().all(|r| r.0 != Kind::LabelValue),
                "divider at {divider}: {out:?}"
            );
        }
        let out = read("123\u{2009}456", 148.0, count);
        assert!(out.iter().all(|r| r.0 != Kind::LabelValue), "{out:?}");
    }
    // Out of reach of the gap, with a value in both fields: fields, the amount whole.
    let out = read(amount, 133.0, Some("7"));
    let pairs: Vec<&Vec<String>> = out
        .iter()
        .filter(|r| r.0 == Kind::LabelValue)
        .map(|r| &r.1)
        .collect();
    assert_eq!(pairs.len(), 1, "{out:?}");
    assert!(pairs[0].iter().any(|t| t == amount), "{out:?}");
    assert_eq!(pairs[0].len(), 4, "{out:?}");
}

/// A strip between rules with no divider, whose second label
/// starts over the digit group gap of a number or over the number, claims no pair: the digits
/// say one number and the labels say two fields. Every run stands in one region. The other
/// way: with no group after the gap the strip reads as two fields.
#[test]
fn a_strip_with_a_label_start_in_a_number_in_digit_groups_claims_no_pair() {
    // Two small labels on one line, one value run from 44 with one gap after its first word,
    // its second word under the second label.
    let read = |labels: [&str; 2], second: f32, text: &str, gap: f32| {
        let sized = |text: &str, x: f32, y: f32, size: f32, line: u32| TextRun {
            w: 0.6 * size * text.chars().count() as f32,
            font_kind: FontKind::Monospace,
            ..run(text, x, y, size, line)
        };
        let first = text.chars().take_while(|c| *c != ' ').count() as f32;
        let mut value = sized(text, 44.0, 679.0, 9.5, 1);
        value.w += gap - 5.7;
        value.gaps = vec![GapIn {
            words: 1,
            from: 44.0 + 5.7 * first,
            to: 44.0 + 5.7 * first + gap,
        }];
        let texts = vec![
            sized(labels[0], 44.0, 692.0, 6.6, 0),
            sized(labels[1], second, 692.0, 6.6, 0),
            value,
        ];
        let rules = h_rules(40.0, 220.0, &[700.0, 675.0]);
        let kinds: Vec<Kind> = decided_on(texts, rules).into_iter().map(|r| r.0).collect();
        kinds
    };
    for (labels, second, text, gap) in [
        (["A", "B"], 58.0, "12 345", 2.6),
        (["A", "B"], 56.0, "7 500,00", 6.3),
        (["No", "Qty"], 60.0, "12 345", 4.6),
        // The label start over the middle of "345", away from the group gap.
        (["A", "B"], 68.0, "12 345", 2.6),
        (["No", "Qty"], 68.0, "12 345", 4.6),
    ] {
        let out = read(labels, second, text, gap);
        assert!(!out.contains(&Kind::LabelValue), "{text}: {out:?}");
    }
    let out = read(["No", "Qty"], 60.0, "12 34,5", 4.6);
    assert_eq!(out, [Kind::LabelValue], "{out:?}");
}

/// A value in a row of fields is no title of the page. Two small labels side
/// by side, each over its value in bold print larger than the body: the value that is no
/// number alone (`03.04.2026 / 10:30`) is a heading by its style, and was the title of the
/// page, so its row was no row of pairs. The cut finds the row of labels over values, and the
/// title is looked for among the other lines. The same line alone, with no row of fields
/// around it, is the title, as before.
#[test]
fn a_value_in_a_row_of_fields_is_no_title() {
    let fields = |labels: bool| {
        let mut texts = Vec::new();
        if labels {
            texts.push(run("Staff number", 324.6, 744.9, 8.0, 0));
            texts.push(run("Office and time", 406.1, 744.9, 8.0, 1));
            texts.push(bold(run("20481", 327.7, 732.0, 12.0, 2)));
        }
        texts.push(bold(run("03.04.2026 / 10:30", 409.2, 732.0, 12.0, 3)));
        // A line across both fields under them: no gutter parts the two fields.
        texts.push(run(
            "Member of a pension scheme [see note 4]",
            324.6,
            706.0,
            8.0,
            4,
        ));
        texts.extend(prose(5, 600.0));
        Tree {
            page: page(1, texts),
        }
    };
    let value = |view: &PageView<'_>, s: &Segment| {
        s.children.is_empty()
            && s.prims.iter().any(|p| {
                matches!(*p, PrimId::Text(i) if view.page.texts[i as usize].text.starts_with("03.04"))
            })
    };
    let row = |_: &PageView<'_>, s: &Segment| s.source == Source::Cell;
    for k in [0.5f32, 1.0, 2.0] {
        let with = fields(true).scaled(k);
        let typed = with.typed(&row);
        let pairs = of(&typed, Kind::LabelValue);
        assert!(
            ids(pairs).contains(&"label_value.label_over_value"),
            "{k}: {typed:#?}"
        );
        assert_eq!(winner(&typed).0, Kind::LabelValue, "{k}");
        let leaf = with.typed(&value);
        assert!(
            leaf.iter().all(|t| t.kind != Kind::TitleBlock),
            "{k}: {leaf:#?}"
        );
        let alone = fields(false).scaled(k).typed(&value);
        assert_eq!(winner(&alone).0, Kind::TitleBlock, "{k}: {alone:#?}");
    }

    // Only what looks like a field's value is kept out of the title
    // search. A page title under a small kicker line stands like a value under its label
    // when a small label over its value stands at the right on the same two baselines; it
    // holds no digit and is much wider than the kicker over it, so it stays the title. With
    // the value at the right in bold of the title's size, in plain print of another size,
    // and with a line of prose under the row.
    let leaf_of = |text: &'static str| {
        move |view: &PageView<'_>, s: &Segment| {
            s.children.is_empty()
                && s.prims.iter().any(|p| {
                    matches!(*p, PrimId::Text(i) if view.page.texts[i as usize].text.starts_with(text))
                })
        }
    };
    let kicker = |title: &str, label: &str, value: TextRun, note: bool| {
        let mut texts = vec![
            run("Department of Samples", 50.0, 790.0, 8.0, 0),
            run(label, 420.0, 790.0, 8.0, 1),
            bold(run(title, 50.0, 776.0, 16.0, 2)),
            value,
        ];
        if note {
            texts.push(run(
                "Please fill in this form in block letters and send it to the office",
                50.0,
                757.0,
                8.0,
                4,
            ));
        }
        texts.extend(prose(5, 600.0));
        Tree {
            page: page(1, texts),
        }
    };
    for k in [0.5f32, 1.0, 2.0] {
        for (label, value, note) in [
            ("Form", bold(run("12", 420.0, 776.0, 16.0, 3)), false),
            ("Form number", run("SL-12", 420.0, 776.0, 10.0, 3), false),
            ("Form number", run("SL-12", 420.0, 776.0, 10.0, 3), true),
        ] {
            // A title that holds a year or a number is a title
            // all the same: it is mostly words, and more words than the kicker over it.
            // And so is a title of as many words as the
            // kicker has, or of fewer: it is the largest text of the top of the page.
            for title in [
                "Request for Sample Leave",
                "Request for Sample Leave 2026",
                "Form 1040 Request for Leave",
                "Sample Leave Request",
                "Annual Report",
            ] {
                let typed = kicker(title, label, value.clone(), note)
                    .scaled(k)
                    .typed(&leaf_of(&title[..6]));
                assert_eq!(
                    winner(&typed).0,
                    Kind::TitleBlock,
                    "{k} {title} {label} {note}: {typed:#?}"
                );
            }
        }
    }
    // Mostly digits and signs (more of them than letters) is a field's value, however many
    // words: `Leave 2026 12 34` under the kicker of three words has 5 letters and 8 digits
    // and is no title. `Leaves 2026 12` (6 letters, 6 digits) is not mostly digits: it is
    // the largest text of the top of the page and may be the title, with as many words as
    // the kicker has (the count of words is no cue).
    let titled = |title: &'static str| {
        let value = bold(run("12", 420.0, 776.0, 16.0, 3));
        let typed = kicker(title, "Form", value, false).typed(&leaf_of(&title[..5]));
        winner(&typed).0 == Kind::TitleBlock
    };
    assert!(!titled("Leave 2026 12 34"));
    assert!(titled("Leaves 2026 12"));
    assert!(titled("Leaves of 2026 12"));
    // A value of words is kept out of the title search unless it is the largest text of the
    // top of the page (the count of words
    // against the label was the measure before, and the width of the ink before that).
    // `other` is another line of the page, its size and baseline. With no other line as
    // large in the top of the page the value may be the title, however few words it has.
    // With a line of its size there (sizes by half points: 11.8 is 12, 11.7 is not), or a
    // larger one, it is a value among the texts of a form and no title. A line as large in
    // the lower part of the page (the page is 842 high, its top third ends at 561) is not
    // counted.
    let wordy = |text: &'static str, other: Option<(f32, f32)>| {
        let mut texts = vec![
            run("Staff number", 324.6, 744.9, 8.0, 0),
            run("Office and time", 406.1, 744.9, 8.0, 1),
            bold(run("20481", 327.7, 732.0, 12.0, 2)),
            bold(run(text, 406.1, 732.0, 12.0, 3)),
            run(
                "Member of a pension scheme [see note 4]",
                324.6,
                706.0,
                8.0,
                4,
            ),
        ];
        if let Some((size, y)) = other {
            texts.push(bold(run("Leave of the staff", 50.0, y, size, 5)));
        }
        texts.extend(prose(6, 600.0));
        let tree = Tree {
            page: page(1, texts),
        };
        tree.typed(&leaf_of(text))
            .iter()
            .any(|t| t.kind == Kind::TitleBlock)
    };
    for text in ["Sales room", "Sales room north", "Sales room north wing"] {
        assert!(wordy(text, None), "{text}");
        assert!(!wordy(text, Some((12.0, 680.0))), "{text}");
        assert!(!wordy(text, Some((11.8, 680.0))), "{text}");
        assert!(wordy(text, Some((11.7, 680.0))), "{text}");
        assert!(!wordy(text, Some((14.0, 680.0))), "{text}");
        assert!(wordy(text, Some((12.0, 300.0))), "{text}");
    }
}

/// A table without rules whose columns start at one pitch,
/// with lowercase descriptions that fill their column and a last row that wraps, is not
/// refused as blocks side by side: its rows are rows (every other column has a line on each
/// row but the wrapped one). With a column of words and with a column of prices.
#[test]
fn rows_of_a_table_with_a_wrapped_last_row_are_no_side_blocks() {
    let cell = |text: &str, x: f32, w: f32, line: u32| {
        let mut r = run(text, x, 700.0 - 11.0 * (line - 100) as f32, 9.2, line);
        r.w = w;
        if line == 100 { bold(r) } else { r }
    };
    // The same with the last cell of the word column left
    // empty (the other columns end on different rows), and with the last description
    // wrapped over two lines then.
    for (third, more) in [
        (["Source", "Mill", "Forge", "Yard"], false),
        (["Price", "1.20", "0.40", "0.10"], false),
        (["Source", "Mill", "Forge", ""], false),
        (["Source", "Mill", "Forge", ""], true),
    ] {
        let mut rows = vec![
            ("Item", "Description", 50.0, third[0]),
            (
                "Bolt",
                "steel bolt for the frame, zinc plated",
                142.0,
                third[1],
            ),
            (
                "Nut",
                "nut for the bolt, zinc plated, size M8",
                141.0,
                third[2],
            ),
            (
                "Washer",
                "washer for the nut and the bolt, set",
                140.0,
                third[3],
            ),
            ("", "twelve pieces", 60.0, ""),
        ];
        if more {
            rows[3].2 = 142.0;
            rows[4] = ("", "twelve pieces, zinc plated, in a paper", 142.0, "");
            rows.push(("", "bag", 18.0, ""));
        }
        let mut texts = Vec::new();
        for (k, (a, b, wb, c)) in rows.iter().enumerate() {
            let line = 100 + k as u32;
            for (text, x, w) in [
                (a, 50.0, 5.0 * a.len() as f32),
                (b, 200.0, *wb),
                (c, 350.0, 5.0 * c.len() as f32),
            ] {
                if !text.is_empty() {
                    texts.push(cell(text, x, w, line));
                }
            }
        }
        let tree = Tree {
            page: sheet(texts, Vec::new()),
        };
        let mut seen = 0;
        tree.with(|view| {
            for s in &view.segs.all {
                let table = s.prims.iter().any(|p| {
                    matches!(*p, PrimId::Text(i) if (100..=105).contains(&view.page.texts[i as usize].line))
                });
                if !table {
                    continue;
                }
                seen += 1;
                for t in all_typed(view, s) {
                    assert!(
                        !ids(&t).contains(&"spatial.side_blocks"),
                        "{}: {t:#?}",
                        third[0]
                    );
                }
            }
        });
        assert!(seen > 0);
    }
}
