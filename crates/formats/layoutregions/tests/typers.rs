// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typers of text regions: each kind from named signals, positive evidence only, the
//! same scores at every scale, and today's decisions ahead of their rivals by the margin.

use layoutregions::file::{FilePage, learn};
use layoutregions::segment::drawn::drawn;
use layoutregions::segment::{SegId, Segment, Source, segment_page};
use layoutregions::typers::{FileIndex, PageView, TYPERS, Typed};
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

/// Four lines of prose in 9 points from line `first` on, the first at `y`.
fn prose(first: u32, y: f32) -> Vec<TextRun> {
    [
        "Die Muster GmbH liefert die bestellte Ware an den",
        "Kunden. Erika Mustermann zahlt den Betrag von",
        "1.234,56 innerhalb von zehn Tagen nach dem Erhalt",
        "der Rechnung auf das genannte Konto der Firma.",
    ]
    .iter()
    .enumerate()
    .map(|(k, t)| run(t, 72.0, y - 11.0 * k as f32, 9.0, first + k as u32))
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

/// A heading of 14 points over prose of 9 points.
fn heading_case(lines: &[u32]) -> Case {
    let mut texts = vec![run("Einleitung", 72.0, 780.0, 14.0, 0)];
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
    texts.push(run("- Erster Punkt", 72.0, 690.0, 9.0, 4));
    texts.push(run("- Zweiter Punkt", 72.0, 679.0, 9.0, 5));
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
            texts.push(run(&format!("Muster GmbH Seite {n}"), 72.0, 30.0, 9.0, 4));
            page(n, texts)
        })
        .collect();
    Case {
        pages,
        at: 1,
        lines: vec![4],
    }
}

/// Prose with two raised references and the legend of 6 points at the foot of the page.
fn legend_case() -> Case {
    let mut texts = vec![
        run("Einheit", 40.0, 720.0, 9.0, 0),
        run(" 2", 72.0, 724.0, 6.0, 0),
        run(" Beitrag", 200.0, 720.0, 9.0, 0),
        run(" 3", 236.0, 724.0, 6.0, 0),
    ];
    texts.extend(prose(1, 690.0));
    texts.push(run("2 Std = Stunden", 40.0, 70.0, 6.0, 5));
    texts.push(run("3 Gegebenenfalls anteilig", 40.0, 62.0, 6.0, 6));
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
        9.0,
        0,
    )];
    texts.extend(prose(1, 740.0));
    texts.push(run("1) Erste Fussnote zum Betrag", 72.0, 70.0, 6.0, 5));
    texts.push(run("2) Zweite Fussnote zur Summe", 72.0, 62.0, 6.0, 6));
    Case {
        pages: vec![page(2, texts)],
        at: 0,
        lines: vec![5, 6],
    }
}

/// Bold lines of an address a little larger than the prose of 10 points under them.
fn address_case() -> Case {
    let mut texts = vec![
        bold(run("Muster GmbH", 72.0, 780.0, 13.0, 0)),
        bold(run("Musterweg 1", 72.0, 765.0, 13.0, 1)),
        bold(run("12345 Musterstadt", 72.0, 750.0, 13.0, 2)),
    ];
    texts.extend(prose(3, 700.0).into_iter().map(|r| TextRun {
        size: 10.0,
        h: 10.0,
        w: r.text.chars().count() as f32 * 5.0,
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
    let mut texts = vec![run("Jahresbericht", 72.0, 800.0, 20.0, 0)];
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
    assert_eq!(winner(&typed).0, Kind::HeaderBand);
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
    blank.pages[0].texts.push(run("   ", 72.0, 600.0, 9.0, 9));
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
        let scores = |c: &Case| -> Vec<(Kind, u16)> {
            c.typed().iter().map(|t| (t.kind, t.score.0)).collect()
        };
        let at = scores(&case);
        assert!(!at.is_empty());
        for k in [0.75, 1.5] {
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
