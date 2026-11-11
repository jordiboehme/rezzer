// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The changes of the metamorphic tests (see `meta`): each keeps the meaning of a page, and
//! the comparison of decisions names what a change must not do. Then a few small pages through
//! the engine under every change. The PDF reader's tests run the same changes on every
//! generated shape and on the corpus pages.

mod meta;

use std::collections::BTreeMap;

use layoutregions::file::{FilePage, learn};
use layoutregions::segment::drawn::grids;
use layoutregions::segment::segment_page_drawn;
use layoutregions::typers::{FileIndex, type_page};
use layoutregions::*;
use meta::{Break, Decisions, Transform};

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

/// `run` with a word gap stated after each word but the last: words of `size / 2` per
/// character, a gap of `size / 4` between them.
fn spaced(text: &str, x: f32, y: f32, size: f32, line: u32) -> TextRun {
    let words: Vec<&str> = text.split(' ').collect();
    let mut gaps = Vec::new();
    let mut at = x;
    for (k, w) in words.iter().enumerate() {
        at += w.chars().count() as f32 * size * 0.5;
        if k + 1 < words.len() {
            gaps.push(GapIn {
                words: k as u32 + 1,
                from: at,
                to: at + size * 0.25,
            });
            at += size * 0.25;
        }
    }
    TextRun {
        w: at - x,
        gaps,
        ..run(text, x, y, size, line)
    }
}

fn page(texts: Vec<TextRun>, rules: Vec<RuleIn>) -> PageIn {
    PageIn {
        page: 0,
        size: [600.0, 800.0],
        turn: 0,
        texts,
        rules,
        boxes: Vec::new(),
        pictures: Vec::new(),
        codes: Vec::new(),
        given: Vec::new(),
    }
}

fn rule(x0: f32, y0: f32, x1: f32, y1: f32) -> RuleIn {
    RuleIn {
        x0,
        y0,
        x1,
        y1,
        thick: 0.0,
        dashed: false,
        rect: false,
    }
}

/// The engine's decisions on the pages of one file, as the PDF reader's layout makes them
/// (facts from every page, the segments on the page's grids, typed and arbitrated), with no
/// amount test.
fn decide(pages: &[PageIn]) -> Vec<Decisions> {
    let none = |_: &str| false;
    let file: Vec<FilePage<'_, TextRun>> = pages.iter().map(FilePage::of).collect();
    let facts = learn(&file, false, &none);
    let index = FileIndex::new(&facts);
    let limits = Limits::default();
    pages
        .iter()
        .map(|p| {
            let (segs, drawn) = segment_page_drawn(p, &grids(&p.rules), &limits, &|| false);
            let (segs, typed) = match drawn {
                Some(d) => type_page(p, &d, segs, &facts, &index, &none, &limits),
                None => (segs, BTreeMap::new()),
            };
            arbitrate(&segs, &typed, &Margins::of(&limits))
                .into_iter()
                .map(|d| {
                    (
                        d.kind,
                        segs.all[d.seg.0 as usize].prims.clone(),
                        String::new(),
                    )
                })
                .collect()
        })
        .collect()
}

#[test]
fn a_scale_multiplies_every_place_and_size() {
    let mut p = page(
        vec![spaced("Erika Mustermann wohnt", 72.0, 700.0, 10.0, 0)],
        vec![rule(72.0, 690.0, 300.0, 690.0)],
    );
    p.boxes.push(BoxIn {
        bounds: Bounds {
            x0: 70.0,
            y0: 680.0,
            x1: 310.0,
            y1: 715.0,
        },
        filled: false,
        stroked: true,
        widget: true,
    });
    let (s, origin) = Transform::Scale(200).apply(&p);
    assert_eq!(origin, meta::Origin::same(&p));
    assert_eq!(s.size, [1200.0, 1600.0]);
    let (a, b) = (&p.texts[0], &s.texts[0]);
    assert_eq!(
        [b.x, b.y, b.w, b.h, b.size],
        [a.x * 2.0, a.y * 2.0, a.w * 2.0, a.h * 2.0, a.size * 2.0]
    );
    assert_eq!(b.gaps[1].from, a.gaps[1].from * 2.0);
    assert_eq!(s.rules[0].x1, 600.0);
    assert_eq!(s.boxes[0].bounds.y1, 1430.0);
    // A shift that grows the page moves everything and grows the frame by as much.
    let (m, _) = Transform::ShiftGrow.apply(&p);
    let [dx, dy] = meta::SHIFT_GROW;
    assert_eq!(m.size, [600.0 + dx, 800.0 + dy]);
    assert_eq!(m.texts[0].gaps[0].to, p.texts[0].gaps[0].to + dx);
    assert_eq!(m.rules[0].y0, 690.0 + dy);
    // A plain shift keeps the frame and moves the content right and down within it.
    let (k, _) = Transform::Shift.apply(&p);
    assert_eq!(k.size, p.size);
    assert_eq!(k.texts[0].x, p.texts[0].x + meta::SHIFT[0]);
    assert_eq!(k.rules[0].y0, 690.0 - meta::SHIFT[1]);
    // Content at the right edge of the frame is not moved past it.
    let mut edge = p.clone();
    edge.boxes[0].bounds.x1 = 595.0;
    let (e, _) = Transform::Shift.apply(&edge);
    assert_eq!(e.texts[0].x, p.texts[0].x + 5.0);
}

#[test]
fn runs_split_at_their_gaps_join_again() {
    let p = page(
        vec![
            spaced("Erika Mustermann wohnt hier", 72.0, 700.0, 10.0, 0),
            run("Muster GmbH", 72.0, 680.0, 10.0, 1),
        ],
        Vec::new(),
    );
    let (s, origin) = Transform::SplitRuns.apply(&p);
    let texts: Vec<&str> = s.texts.iter().map(|t| t.text.as_str()).collect();
    // The second run states no gap, so it stays whole.
    assert_eq!(
        texts,
        ["Erika", "Mustermann", "wohnt", "hier", "Muster GmbH"]
    );
    assert_eq!(origin.texts, [vec![0], vec![0], vec![0], vec![0], vec![1]]);
    // Each word starts where the gap before it ends and ends where the gap after it starts.
    let g = &p.texts[0].gaps;
    assert_eq!(
        (s.texts[1].x, s.texts[1].x + s.texts[1].w),
        (g[0].to, g[1].from)
    );
    assert_eq!(s.texts[3].x + s.texts[3].w, p.texts[0].x + p.texts[0].w);
    // Joined again: the same run, its gaps stated.
    let (j, origin) = Transform::MergeRuns.apply(&s);
    assert_eq!(origin.texts, [vec![0, 1, 2, 3], vec![4]]);
    assert_eq!(j.texts[0].text, p.texts[0].text);
    assert_eq!(j.texts[0].gaps, p.texts[0].gaps);
    assert!((j.texts[0].w - p.texts[0].w).abs() < 1e-3);
}

#[test]
fn runs_far_apart_or_in_another_style_are_not_joined() {
    let mut bold = run("4711", 130.0, 700.0, 10.0, 0);
    bold.bold = true;
    let p = page(
        vec![
            run("Kundennr", 72.0, 700.0, 10.0, 0),
            // A column gap: wider than a word space.
            run("12345", 200.0, 700.0, 10.0, 0),
            bold,
            run("Muster", 72.0, 680.0, 10.0, 1),
            // A word the producer cut with no gap: no word space, so not joined with one.
            run("GmbH", 102.0, 680.0, 10.0, 1),
        ],
        Vec::new(),
    );
    let (j, origin) = Transform::MergeRuns.apply(&p);
    assert_eq!(j.texts.len(), 5);
    assert_eq!(origin.texts, [vec![0], vec![1], vec![2], vec![3], vec![4]]);
}

#[test]
fn jitter_stays_within_its_share_and_repeats() {
    let p = page(
        (0..20)
            .map(|k| spaced("Muster GmbH", 72.0, 700.0 - 14.0 * k as f32, 10.0, k))
            .collect(),
        Vec::new(),
    );
    let (a, _) = Transform::Jitter.apply(&p);
    let (b, _) = Transform::Jitter.apply(&p);
    assert_eq!(a, b);
    let most = meta::JITTER * 10.0 + 1e-4;
    let mut moved = 0;
    for (t, u) in p.texts.iter().zip(&a.texts) {
        for (x, y) in [(t.x, u.x), (t.y, u.y), (t.w, u.w), (t.h, u.h)] {
            assert!((x - y).abs() <= most, "{x} {y}");
            moved += usize::from(x != y);
        }
        // The gaps move with the run.
        assert!((u.gaps[0].from - t.gaps[0].from - (u.x - t.x)).abs() < 1e-4);
    }
    assert!(moved > 60, "{moved}");
}

#[test]
fn rules_in_pieces_or_as_bars_cover_the_rule() {
    let p = page(
        Vec::new(),
        vec![
            rule(72.0, 690.0, 300.0, 690.0),
            rule(100.0, 600.0, 100.0, 690.0),
            RuleIn {
                rect: true,
                ..rule(50.0, 50.0, 80.0, 80.0)
            },
        ],
    );
    let (s, origin) = Transform::SplitRules.apply(&p);
    // Each line in three touching pieces, each from its rule; the outline stays.
    assert_eq!(
        origin.rules,
        [0, 0, 0, 1, 1, 1, 2].map(PrimId::Rule).to_vec()
    );
    assert_eq!(s.rules.len(), 7);
    assert_eq!((s.rules[0].x0, s.rules[2].x1), (72.0, 300.0));
    assert_eq!(s.rules[0].x1, s.rules[1].x0);
    assert_eq!(s.rules[4].y1, s.rules[5].y0);
    assert!(s.rules[6].rect);
    let (b, origin) = Transform::RuleAsBox.apply(&p);
    assert_eq!(origin.rules, [PrimId::Rule(2)]);
    assert_eq!(origin.boxes, [PrimId::Rule(0), PrimId::Rule(1)]);
    assert_eq!(b.rules.len(), 1);
    assert_eq!(b.boxes.len(), 2);
    let h = b.boxes[0].bounds;
    assert_eq!([h.x0, h.x1], [72.0, 300.0]);
    assert!((h.y1 - h.y0 - meta::BAR).abs() < 1e-4 && b.boxes[0].filled);
    let v = b.boxes[1].bounds;
    assert!((v.x1 - v.x0 - meta::BAR).abs() < 1e-4 && v.y0 == 600.0);
}

#[test]
fn a_reorder_keeps_every_primitive_once() {
    let mut p = page(
        (0..30)
            .map(|k| run("Muster", 72.0, 700.0 - 12.0 * k as f32, 10.0, k))
            .collect(),
        (0..10)
            .map(|k| rule(72.0, 100.0 + k as f32, 300.0, 100.0 + k as f32))
            .collect(),
    );
    for (k, t) in p.texts.iter_mut().enumerate() {
        t.text = format!("Muster {k}");
    }
    let (r, origin) = Transform::ReorderRuns.apply(&p);
    assert_ne!(r.texts, p.texts);
    for (t, o) in r.texts.iter().zip(&origin.texts) {
        assert_eq!(*t, p.texts[o[0] as usize]);
    }
    let mut seen: Vec<u32> = origin.texts.iter().flatten().copied().collect();
    seen.sort_unstable();
    assert_eq!(seen, (0..30).collect::<Vec<u32>>());
    let (d, origin) = Transform::ReorderDrawn.apply(&p);
    assert_eq!(d.texts, p.texts);
    assert_ne!(d.rules, p.rules);
    for (k, r) in d.rules.iter().enumerate() {
        let PrimId::Rule(o) = origin.rules[k] else {
            panic!("{:?}", origin.rules[k])
        };
        assert_eq!(*r, p.rules[o as usize]);
    }
    let mut a: Vec<f32> = d.rules.iter().map(|r| r.y0).collect();
    a.sort_by(f32::total_cmp);
    assert_eq!(a, p.rules.iter().map(|r| r.y0).collect::<Vec<f32>>());
}

/// Decisions of kinds and runs, as `compare` reads them.
fn ds(list: &[(Kind, &[u32])]) -> Decisions {
    list.iter()
        .map(|(k, ids)| {
            (
                *k,
                ids.iter().map(|&i| PrimId::Text(i)).collect(),
                String::new(),
            )
        })
        .collect()
}

#[test]
fn the_comparison_names_what_a_change_must_not_do() {
    let p = page(
        (0..6)
            .map(|k| run("Muster", 72.0, 700.0 - 12.0 * k as f32, 10.0, k))
            .collect(),
        Vec::new(),
    );
    let same = meta::Origin::same(&p);
    let base = ds(&[
        (Kind::Heading, &[0]),
        (Kind::Table, &[1, 2, 3]),
        (Kind::Paragraph, &[4, 5]),
    ]);
    let whats = |changed: &Decisions| -> Vec<Break> {
        meta::compare(&p, &base, 6, changed, &same)
            .broken
            .iter()
            .map(|b| b.what)
            .collect()
    };
    assert!(meta::compare(&p, &base, 6, &base, &same).same());
    // A typed region kept as laid out is allowed.
    let v = meta::compare(
        &p,
        &base,
        6,
        &ds(&[
            (Kind::Heading, &[0]),
            (Kind::Spatial, &[1, 2, 3]),
            (Kind::Paragraph, &[4, 5]),
        ]),
        &same,
    );
    assert!(v.broken.is_empty() && v.to_spatial.len() == 1);
    // Another kind, other runs, another order, a run lost or read twice are not.
    let k = ds(&[
        (Kind::Heading, &[0]),
        (Kind::LabelValue, &[1, 2, 3]),
        (Kind::Paragraph, &[4, 5]),
    ]);
    assert_eq!(whats(&k), [Break::Kind]);
    let m = ds(&[
        (Kind::Heading, &[0]),
        (Kind::Table, &[1, 2]),
        (Kind::Paragraph, &[3, 4, 5]),
    ]);
    assert_eq!(whats(&m), [Break::Members, Break::Members]);
    let o = ds(&[
        (Kind::Paragraph, &[4, 5]),
        (Kind::Heading, &[0]),
        (Kind::Table, &[1, 2, 3]),
    ]);
    assert_eq!(whats(&o), [Break::Order]);
    let lost = ds(&[
        (Kind::Heading, &[0]),
        (Kind::Table, &[1, 2, 3]),
        (Kind::Paragraph, &[5]),
    ]);
    let v = meta::compare(&p, &base, 6, &lost, &same);
    assert!(v.broken.iter().any(|b| b.what == Break::Lost));
    let twice = ds(&[
        (Kind::Heading, &[0]),
        (Kind::Table, &[1, 2, 3]),
        (Kind::Paragraph, &[3, 4, 5]),
    ]);
    let v = meta::compare(&p, &base, 6, &twice, &same);
    assert!(v.broken.iter().any(|b| b.what == Break::Twice));
    // A typed region whose runs were all spatial is allowed too, and said.
    let spatial = ds(&[(Kind::Spatial, &[0, 1, 2, 3, 4, 5])]);
    let v = meta::compare(&p, &spatial, 6, &base, &same);
    assert!(v.broken.is_empty() && v.from_spatial.len() == 3);
    // Through an origin: two runs joined into one stand for both.
    let joined = meta::Origin {
        texts: vec![vec![0], vec![1, 2], vec![3], vec![4], vec![5]],
        ..meta::Origin::same(&p)
    };
    let changed: Decisions = vec![
        (Kind::Heading, vec![PrimId::Text(0)], String::new()),
        (
            Kind::Table,
            vec![PrimId::Text(1), PrimId::Text(2)],
            String::new(),
        ),
        (
            Kind::Paragraph,
            vec![PrimId::Text(3), PrimId::Text(4)],
            String::new(),
        ),
    ];
    assert!(meta::compare(&p, &base, 5, &changed, &joined).same());
}

#[test]
fn the_owner_of_a_break_follows_its_class() {
    use meta::owner;
    assert_eq!(
        owner(Transform::ReorderRuns, Break::Members, "table"),
        "after 0.1.0"
    );
    assert_eq!(
        owner(Transform::RuleAsBox, Break::Kind, "pairs"),
        "after 0.1.0"
    );
    assert_eq!(owner(Transform::ShiftGrow, Break::Kind, "band"), "31");
    assert_eq!(owner(Transform::Scale(50), Break::Order, "band"), "31");
    assert_eq!(owner(Transform::SplitRules, Break::Members, "pairs"), "31");
    assert_eq!(owner(Transform::Scale(200), Break::Members, "pairs"), "31");
    assert_eq!(owner(Transform::Jitter, Break::Members, "pairs"), "26");
    assert_eq!(owner(Transform::Jitter, Break::Kind, "text"), "36a");
    assert_eq!(owner(Transform::Scale(50), Break::Members, "text"), "31");
    assert_eq!(owner(Transform::SplitRuns, Break::Members, "table"), "31");
}

/// A heading over two paragraphs of prose and a ruled table of three rows, in one file of
/// two pages.
fn prose_and_table() -> Vec<PageIn> {
    let mut texts = vec![spaced("Bericht der Muster GmbH", 72.0, 740.0, 16.0, 0)];
    let line = "die Muster GmbH in Musterstadt zeigt hier die Werte des Jahres";
    for k in 0..4u32 {
        texts.push(spaced(line, 72.0, 710.0 - 12.0 * k as f32, 9.0, 1 + k));
    }
    for k in 0..3u32 {
        texts.push(spaced(line, 72.0, 640.0 - 12.0 * k as f32, 9.0, 5 + k));
    }
    let mut rules = Vec::new();
    for (r, y) in [500.0f32, 480.0, 460.0, 440.0].iter().enumerate() {
        rules.push(rule(72.0, *y, 372.0, *y));
        if r < 3 {
            for (c, x) in [76.0f32, 176.0, 276.0].iter().enumerate() {
                let t = if r == 0 {
                    ["Name", "Ort", "Betrag"][c].to_string()
                } else {
                    [
                        "Erika".to_string(),
                        "Musterstadt".to_string(),
                        format!("1.234,5{r}"),
                    ][c]
                        .clone()
                };
                texts.push(run(&t, *x, y - 14.0, 9.0, 8 + r as u32));
            }
        }
    }
    for x in [72.0f32, 172.0, 272.0, 372.0] {
        rules.push(rule(x, 440.0, x, 500.0));
    }
    let first = page(texts, rules);
    let mut second = first.clone();
    second.page = 1;
    vec![first, second]
}

#[test]
fn a_page_of_prose_and_a_ruled_table_reads_the_same_under_every_change_but_known_ones() {
    let pages = prose_and_table();
    let base = decide(&pages);
    let kinds: Vec<Kind> = base[0].iter().map(|d| d.0).collect();
    assert!(kinds.contains(&Kind::Table), "{kinds:?}");
    assert!(kinds.contains(&Kind::Paragraph), "{kinds:?}");
    let findings = meta::sweep(&pages, &Transform::ALL, &decide);
    let broken: Vec<String> = findings
        .iter()
        .filter(|f| !f.verdict.broken.is_empty())
        .map(|f| format!("{} {}", f.transform.name(), f.page + 1))
        .collect();
    // Pinned: with the runs in another order (input the PDF reader never hands over, after
    // 0.1.0) the table holds a rule more than on the page, which reads it among the drawing.
    let pinned = ["reorder-runs 1", "reorder-runs 2"];
    assert_eq!(broken, pinned, "{:#?}", findings);
}

/// Regions without text are compared too, by their pictures, rules and boxes through the
/// origin: a figure that loses a picture to another region breaks, the same figure with its
/// pictures in another order does not.
#[test]
fn regions_without_text_are_compared_by_their_primitives() {
    let mut p = page(vec![run("Muster", 72.0, 700.0, 10.0, 0)], Vec::new());
    for k in 0..2 {
        p.pictures.push(PictureIn {
            bounds: Bounds {
                x0: 72.0 + 100.0 * k as f32,
                y0: 400.0,
                x1: 150.0 + 100.0 * k as f32,
                y1: 500.0,
            },
        });
    }
    let base: Decisions = vec![
        (Kind::Paragraph, vec![PrimId::Text(0)], String::new()),
        (
            Kind::Figure,
            vec![PrimId::Picture(0), PrimId::Picture(1)],
            String::new(),
        ),
    ];
    let swapped = meta::Origin {
        pictures: vec![PrimId::Picture(1), PrimId::Picture(0)],
        ..meta::Origin::same(&p)
    };
    assert!(meta::compare(&p, &base, 1, &base, &swapped).same());
    let parted: Decisions = vec![
        (Kind::Paragraph, vec![PrimId::Text(0)], String::new()),
        (Kind::Figure, vec![PrimId::Picture(0)], String::new()),
        (Kind::Figure, vec![PrimId::Picture(1)], String::new()),
    ];
    let v = meta::compare(&p, &base, 1, &parted, &meta::Origin::same(&p));
    assert_eq!(
        v.broken.iter().map(|b| b.what).collect::<Vec<_>>(),
        [Break::Members, Break::Members]
    );
}

/// Writing the pins keeps the lines of every name that was not swept, and a page's breaks
/// are pinned on their own line.
#[test]
fn writing_pins_keeps_the_other_names() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("meta-pins");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pins.tsv");
    std::fs::write(
        &path,
        "# head\nother.pdf\t3\tjitter\tmembers\ttext\t2\t31\nthis.pdf\t1\tjitter\tkind\ttext\t1\t36a\n",
    )
    .unwrap();
    let lines = vec!["this.pdf\t2\tjitter\tmembers\ttext\t1\t31".to_string()];
    meta::check_or_write_pins(&path, &["this.pdf".to_string()], &lines, true);
    let written = meta::pinned(&path);
    assert_eq!(
        written,
        [
            "other.pdf\t3\tjitter\tmembers\ttext\t2\t31",
            "this.pdf\t2\tjitter\tmembers\ttext\t1\t31"
        ]
    );
    // Checked again: the same lines hold; a break moved to another page does not.
    meta::check_or_write_pins(&path, &["this.pdf".to_string()], &lines, false);
    let moved = vec!["this.pdf\t5\tjitter\tmembers\ttext\t1\t31".to_string()];
    let failed = std::panic::catch_unwind(|| {
        meta::check_or_write_pins(&path, &["this.pdf".to_string()], &moved, false)
    });
    assert!(failed.is_err());
    std::fs::remove_dir_all(&dir).ok();
}
