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
/// (the runs in the order of their places, facts from every page, the segments on the page's
/// grids, typed and arbitrated), with no amount test. The decisions name the runs by the ids
/// of the pages handed in.
fn decide(pages: &[PageIn]) -> Vec<Decisions> {
    let none = |_: &str| false;
    let ordered: Vec<Option<RunOrder>> = pages.iter().map(RunOrder::of).collect();
    let placed: Vec<&PageIn> = pages
        .iter()
        .zip(&ordered)
        .map(|(p, o)| o.as_ref().map_or(p, |o| &o.page))
        .collect();
    let file: Vec<FilePage<'_, TextRun>> = placed.iter().map(|p| FilePage::of(p)).collect();
    let facts = learn(&file, false, &none);
    let index = FileIndex::new(&facts);
    let limits = Limits::default();
    placed
        .iter()
        .zip(&ordered)
        .map(|(p, order)| {
            let (segs, drawn) = segment_page_drawn(p, &grids(&p.rules), &limits, &|| false);
            let (mut segs, typed) = match drawn {
                Some(d) => type_page(p, &d, segs, &facts, &index, &none, &limits),
                None => (segs, BTreeMap::new()),
            };
            if let Some(order) = order {
                order.segments_back(&mut segs);
            }
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
    // The order of two regions side by side is free: the same decisions on a page whose
    // last two lines stand right of the others, in any order, break nothing.
    let mut side = p.clone();
    for t in &mut side.texts[4..] {
        t.x += 300.0;
    }
    assert!(meta::compare(&side, &base, 6, &o, &same).broken.is_empty());
    // A heading over those two lines is of their flow: read after them, the order is broken.
    side.texts[0].x += 300.0;
    assert_eq!(
        meta::compare(&side, &base, 6, &o, &same)
            .broken
            .iter()
            .map(|b| b.what)
            .collect::<Vec<_>>(),
        [Break::Order]
    );
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
    assert_eq!(owner(Transform::ReorderRuns, Break::Members, "table"), "31");
    assert_eq!(
        owner(Transform::RuleAsBox, Break::Kind, "pairs"),
        "after 0.1.0"
    );
    assert_eq!(owner(Transform::ShiftGrow, Break::Kind, "band"), "31");
    assert_eq!(owner(Transform::Scale(50), Break::Order, "band"), "31");
    assert_eq!(owner(Transform::SplitRules, Break::Members, "pairs"), "31");
    assert_eq!(owner(Transform::Scale(200), Break::Members, "pairs"), "31");
    assert_eq!(
        owner(Transform::Jitter, Break::Members, "pairs"),
        "after 0.1.0"
    );
    assert_eq!(owner(Transform::Jitter, Break::Kind, "text"), "after 0.1.0");
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
    // Nothing is pinned. With the runs in another order the page read the table with a rule
    // more, until the runs were read in the order of their places (`RunOrder`, Task 31c).
    assert_eq!(broken, [""; 0], "{:#?}", findings);
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
        "# head\nother.pdf\t3\tjitter\tmembers\ttext\t2\t31\nthis.pdf\t1\tjitter\tkind\ttext\t1\t36b\n",
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

/// A cause that names an open owner sets the owner column of its line: the computed owner
/// (by the class of the break) gives way to the traced one, when the pins are checked and
/// when they are written. A cause that names none leaves the column as computed.
#[test]
fn a_traced_cause_names_the_owner_of_its_pin() {
    assert_eq!(meta::traced_owner("owner Task 36b. Traced: x"), Some("36b"));
    assert_eq!(
        meta::traced_owner("owner after 0.1.0. Traced"),
        Some("after 0.1.0")
    );
    assert_eq!(
        meta::traced_owner("owner after 0.1.0, kept. Traced"),
        Some("after 0.1.0")
    );
    assert_eq!(meta::traced_owner("owner Task 31c, kept"), Some("31c"));
    assert_eq!(
        meta::traced_owner("owner Task 37 (named 31c before)"),
        Some("37")
    );
    assert_eq!(meta::traced_owner("Traced: the owner is Task 31"), None);
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("meta-owner");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pins.tsv");
    std::fs::write(
        &path,
        "# head\nthis.pdf\t1\tjitter\tmembers\ttext\t1\t36b\towner Task 36b. Traced: a cause\n\
         this.pdf\t2\tjitter\tmembers\ttext\t1\t31\tTraced: a cause with no owner\n",
    )
    .unwrap();
    let names = ["this.pdf".to_string()];
    // The lines as they are found, with the computed owner 31.
    let found = vec![
        "this.pdf\t1\tjitter\tmembers\ttext\t1\t31".to_string(),
        "this.pdf\t2\tjitter\tmembers\ttext\t1\t31".to_string(),
    ];
    meta::check_or_write_pins(&path, &names, &found, false);
    // Written again, the line keeps the owner of its cause, and a pin whose column names
    // another owner than its cause is set right.
    std::fs::write(
        &path,
        "# head\nthis.pdf\t1\tjitter\tmembers\ttext\t1\t31\towner Task 36b. Traced: a cause\n",
    )
    .unwrap();
    let wrong = std::panic::catch_unwind(|| {
        meta::check_or_write_pins(&path, &["this.pdf".to_string()], &found[..1], false)
    });
    assert!(wrong.is_err());
    meta::check_or_write_pins(&path, &names, &found[..1], true);
    assert_eq!(
        meta::pinned(&path),
        ["this.pdf\t1\tjitter\tmembers\ttext\t1\t36b"]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A move to or from spatial is pinned with its page, as a break is: the same move on another
/// page is another line, so a move that goes on one page and comes on another shows.
#[test]
fn moves_are_pinned_per_page() {
    let moved = |page: usize| meta::Finding {
        transform: Transform::ALL[0],
        page,
        verdict: meta::Verdict {
            to_spatial: vec![(Kind::Paragraph, "paragraph: a line".to_string())],
            from_spatial: Vec::new(),
            broken: Vec::new(),
        },
    };
    let name = Transform::ALL[0].name();
    assert_eq!(
        meta::pin_lines("this.pdf", &[moved(0), moved(0), moved(4)]),
        [
            format!("this.pdf\t1\t{name}\tto-spatial\ttext\t2\t-"),
            format!("this.pdf\t5\t{name}\tto-spatial\ttext\t1\t-"),
        ]
    );
    assert_ne!(
        meta::pin_lines("this.pdf", &[moved(0)]),
        meta::pin_lines("this.pdf", &[moved(4)])
    );
}

/// One flow is half the width of the narrower region or more over the other: moved both ways
/// at the limit, with a region without width and regions that only touch.
#[test]
fn one_flow_is_half_the_narrower_width_over_the_other() {
    // A column of 100 and a block of 40 that stands 20.5 and 19.5 points over it.
    assert!(meta::one_flow([0.0, 100.0], [79.5, 119.5]));
    assert!(!meta::one_flow([0.0, 100.0], [80.5, 120.5]));
    // The same at twice the size: a share, not points.
    assert!(meta::one_flow([0.0, 200.0], [159.0, 239.0]));
    assert!(!meta::one_flow([0.0, 200.0], [161.0, 241.0]));
    // A block inside the width of another, and a block across both columns.
    assert!(meta::one_flow([0.0, 100.0], [30.0, 60.0]));
    assert!(meta::one_flow([0.0, 500.0], [300.0, 500.0]));
    // Side by side, with a gutter or touching.
    assert!(!meta::one_flow([0.0, 100.0], [120.0, 220.0]));
    assert!(!meta::one_flow([0.0, 100.0], [100.0, 200.0]));
    // A region without width is in the flow it stands in, and in no flow beside it.
    assert!(meta::one_flow([0.0, 100.0], [50.0, 50.0]));
    assert!(!meta::one_flow([0.0, 100.0], [150.0, 150.0]));
    assert_eq!(meta::FLOW_SHARE, 0.5);
}

/// Whether `changed` breaks the order of `base` on `p`.
fn order_breaks(p: &PageIn, base: &Decisions, changed: &Decisions) -> bool {
    let runs = p.texts.len();
    let v = meta::compare(p, base, runs, changed, &meta::Origin::same(p));
    assert!(
        v.broken.iter().all(|b| b.what == Break::Order),
        "{:?}",
        v.broken
    );
    !v.broken.is_empty()
}

/// `base` with its decisions in the order `to` (places in `base`).
fn reordered(base: &Decisions, to: &[usize]) -> Decisions {
    to.iter().map(|&k| base[k].clone()).collect()
}

/// A line of `chars` letters that ends in a full stop: 5 points a letter at size 10.
fn line_of(chars: usize, x: f32, y: f32, line: u32) -> TextRun {
    run(&format!("{}.", "m".repeat(chars - 1)), x, y, 10.0, line)
}

/// Relation (a) across a column break: the foot of a column of prose and the head of the
/// next column of its set keep their order, so two columns read by rows break. The shares
/// that make a column (a full line, a like width) are moved both ways, and two blocks of
/// short lines side by side (an address beside an address) are free.
#[test]
fn the_foot_of_a_column_and_the_head_of_the_next_keep_their_order() {
    // `second` letters in the second line of the left column, `right` in the lines of the
    // right one. Left: two paragraphs of 40 letters a line (200 points); right: two.
    let columns = |second: usize, right: usize| {
        page(
            vec![
                line_of(40, 50.0, 700.0, 0),
                line_of(second, 50.0, 688.0, 1),
                line_of(20, 50.0, 676.0, 2),
                line_of(40, 50.0, 652.0, 3),
                line_of(second, 50.0, 640.0, 4),
                line_of(20, 50.0, 628.0, 5),
                line_of(right, 300.0, 700.0, 6),
                line_of(20, 300.0, 688.0, 7),
                line_of(right, 300.0, 664.0, 8),
                line_of(20, 300.0, 652.0, 9),
            ],
            Vec::new(),
        )
    };
    let base = ds(&[
        (Kind::Paragraph, &[0, 1, 2]),
        (Kind::Paragraph, &[3, 4, 5]),
        (Kind::Paragraph, &[6, 7]),
        (Kind::Paragraph, &[8, 9]),
    ]);
    let p = columns(40, 40);
    assert!(!order_breaks(&p, &base, &base));
    // Read by rows, and the right column first.
    assert!(order_breaks(&p, &base, &reordered(&base, &[0, 2, 1, 3])));
    assert!(order_breaks(&p, &base, &reordered(&base, &[2, 3, 0, 1])));
    // The right column a little narrower: 33 of 40 letters is of the set, 31 is not.
    assert!(order_breaks(
        &columns(40, 33),
        &base,
        &reordered(&base, &[0, 2, 1, 3])
    ));
    assert!(!order_breaks(
        &columns(40, 31),
        &base,
        &reordered(&base, &[0, 2, 1, 3])
    ));
    // The second line of the left paragraphs: at 33 letters both lines over the last are
    // full, at 31 one of two is, and the block is no prose.
    assert!(order_breaks(
        &columns(33, 40),
        &base,
        &reordered(&base, &[0, 2, 1, 3])
    ));
    assert!(!order_breaks(
        &columns(31, 40),
        &base,
        &reordered(&base, &[0, 2, 1, 3])
    ));
    assert_eq!(meta::COLUMN_SHARE, 0.8);
    // A known limit of the check (owner: after 0.1.0; a limit of the check, it changes no read): a
    // foot of two lines has one line over its last, which is the widest by itself and says nothing.
    // Such a column is not seen as prose. When the sign is taken from the column and not from its
    // foot block, this assertion turns. Part 2b-1 of Task 31c found no such sign in the check: it
    // compares regions, and the lines of the column over the foot are in other regions or in none.
    let short = ds(&[
        (Kind::Paragraph, &[0, 1, 2]),
        (Kind::Paragraph, &[3, 4]),
        (Kind::Spatial, &[5]),
        (Kind::Paragraph, &[6, 7]),
        (Kind::Paragraph, &[8, 9]),
    ]);
    assert!(!order_breaks(
        &p,
        &short,
        &reordered(&short, &[0, 3, 1, 2, 4])
    ));
    // A column that starts under the end of the left one is no next column.
    let mut low = columns(40, 40);
    for t in &mut low.texts[6..] {
        t.y -= 200.0;
    }
    assert!(!order_breaks(&low, &base, &reordered(&base, &[0, 2, 1, 3])));
}

/// Relation (a) when the next column starts with a heading: the heading opens the next part
/// of the same flow, so the foot of the left column is read before it. The sign of a column
/// set (a like width) is taken from the text under the heading, and is moved both ways. A
/// heading over a block of short lines, and one under the end of the left column, are free.
#[test]
fn a_column_headed_by_a_heading_is_the_next_column() {
    // `right` letters in the full lines under the heading.
    let columns = |right: usize| {
        page(
            vec![
                line_of(40, 50.0, 700.0, 0),
                line_of(40, 50.0, 688.0, 1),
                line_of(20, 50.0, 676.0, 2),
                line_of(40, 50.0, 652.0, 3),
                line_of(40, 50.0, 640.0, 4),
                line_of(20, 50.0, 628.0, 5),
                run("Results", 300.0, 700.0, 12.0, 6),
                line_of(right, 300.0, 684.0, 7),
                line_of(right, 300.0, 672.0, 8),
                line_of(20, 300.0, 660.0, 9),
            ],
            Vec::new(),
        )
    };
    let base = ds(&[
        (Kind::Paragraph, &[0, 1, 2]),
        (Kind::Paragraph, &[3, 4, 5]),
        (Kind::Heading, &[6]),
        (Kind::Paragraph, &[7, 8, 9]),
    ]);
    let p = columns(40);
    assert!(!order_breaks(&p, &base, &base));
    // The right column before the foot of the left one, and the columns read by rows.
    let right_first = reordered(&base, &[0, 2, 3, 1]);
    let by_rows = reordered(&base, &[0, 2, 1, 3]);
    assert!(order_breaks(&p, &base, &right_first));
    assert!(order_breaks(&p, &base, &by_rows));
    // The text under the heading a little narrower: 33 of 40 letters is of the set, 31 is
    // not, and a block of short lines under a heading is no column of the text at the left.
    assert!(order_breaks(&columns(33), &base, &by_rows));
    assert!(!order_breaks(&columns(31), &base, &by_rows));
    assert!(!order_breaks(&columns(20), &base, &right_first));
    // The heading alone, with no text under it that the page reads: nothing says that it
    // heads a column.
    let alone = ds(&[
        (Kind::Paragraph, &[0, 1, 2]),
        (Kind::Paragraph, &[3, 4, 5]),
        (Kind::Heading, &[6]),
        (Kind::Spatial, &[7, 8, 9]),
    ]);
    assert!(!order_breaks(&p, &alone, &reordered(&alone, &[0, 2, 1, 3])));
    // What stands under the heading is a table: no running text, no column.
    let table = ds(&[
        (Kind::Paragraph, &[0, 1, 2]),
        (Kind::Paragraph, &[3, 4, 5]),
        (Kind::Heading, &[6]),
        (Kind::Table, &[7, 8, 9]),
    ]);
    assert!(!order_breaks(&p, &table, &reordered(&table, &[0, 2, 1, 3])));
    // A heading that starts under the end of the left column opens no next column.
    let mut low = columns(40);
    for t in &mut low.texts[6..] {
        t.y -= 200.0;
    }
    assert!(!order_breaks(&low, &base, &by_rows));
}

/// Relation (b): a heading left of its block, on the block's rows, is read before it. A
/// block of text that is no heading in the same place (a head block against a date box) is
/// free. The share of the heading's height on the rows is moved both ways.
#[test]
fn a_side_heading_and_its_block_keep_their_order() {
    let side = |y: f32| {
        page(
            vec![
                run("Sum due", 50.0, y, 10.0, 0),
                run("Take the sum of the lines.", 200.0, 700.0, 10.0, 1),
                run("Write it in the last box.", 200.0, 688.0, 10.0, 2),
                run("Send the sheet on.", 200.0, 676.0, 10.0, 3),
            ],
            Vec::new(),
        )
    };
    let base = ds(&[(Kind::Heading, &[0]), (Kind::Paragraph, &[1, 2, 3])]);
    let swapped = reordered(&base, &[1, 0]);
    for y in [700.0, 688.0, 676.0] {
        assert!(order_breaks(&side(y), &base, &swapped), "{y}");
    }
    // The block reaches up to 710: a heading from 705 to 715 has half its height on the
    // rows, one from 706 has less.
    assert!(order_breaks(&side(705.0), &base, &swapped));
    assert!(!order_breaks(&side(706.0), &base, &swapped));
    assert_eq!(meta::ROW_SHARE, 0.5);
    // The same two blocks, neither a heading: free.
    let plain = ds(&[(Kind::Paragraph, &[0]), (Kind::Paragraph, &[1, 2, 3])]);
    assert!(!order_breaks(
        &side(700.0),
        &plain,
        &reordered(&plain, &[1, 0])
    ));
    // A heading right of the block heads nothing left of it.
    let mut right = side(700.0);
    right.texts[0].x = 450.0;
    assert!(!order_breaks(&right, &base, &swapped));
    // A title beside a number box heads nothing: the box is narrower than the title, and the
    // two are a free pair. A side heading stands in the margin of a block that is as wide as
    // it or wider: a block of 7 letters beside a heading of 7 is headed, one of 6 is not.
    let beside = |title: &str, text: &str| {
        page(
            vec![
                run(title, 50.0, 700.0, 10.0, 0),
                run(text, 300.0, 700.0, 10.0, 0),
            ],
            Vec::new(),
        )
    };
    let pair = ds(&[(Kind::Heading, &[0]), (Kind::Paragraph, &[1])]);
    let turned = reordered(&pair, &[1, 0]);
    assert!(!order_breaks(
        &beside("Sample Works Parts Return", "Sheet No. 4411"),
        &pair,
        &turned
    ));
    assert!(order_breaks(&beside("Sum due", "Take it"), &pair, &turned));
    assert!(!order_breaks(&beside("Sum due", "Take 4"), &pair, &turned));
    // The box typed a field or a table: the title is no label of it. A heading that ends in
    // a colon is one.
    for kind in [Kind::LabelValue, Kind::Totals, Kind::Table] {
        let boxed = ds(&[(Kind::Heading, &[0]), (kind, &[1])]);
        let turned = reordered(&boxed, &[1, 0]);
        let title = beside("Sample Works Parts Return", "Sheet No. 4411");
        assert!(!order_breaks(&title, &boxed, &turned), "{kind:?}");
        let label = beside("Sample Works Parts Return:", "Sheet No. 4411");
        assert!(order_breaks(&label, &boxed, &turned), "{kind:?}");
    }
    // A short title beside a wider box typed a field: the width says side heading, the type
    // says box, and a side heading never heads a field beside a title.
    let wide = beside("Invoice", "Sheet No. 4411-0074");
    for kind in [Kind::LabelValue, Kind::Totals] {
        let boxed = ds(&[(Kind::Heading, &[0]), (kind, &[1])]);
        assert!(
            !order_breaks(&wide, &boxed, &reordered(&boxed, &[1, 0])),
            "{kind:?}"
        );
    }
    // A known limit of the check (owner: after 0.1.0; a limit of the check, it changes no read): the
    // same wider box typed a paragraph. A heading beside a wider block of one line on its row is
    // the shape of a side heading, and geometry cannot tell a number line from a block of one line,
    // so the pair is held though it is free. When a sign tells the two apart, this assertion turns.
    // Part 2b-1 of Task 31c found none: a box or a number field is a sign of the engine's read
    // (its label and value groups), not of the geometry the check has.
    assert!(order_breaks(&wide, &pair, &turned));
    // A box typed a field left of a title: the title is no value of it either. A field left
    // of a paragraph on its row is still read first.
    let left_box = beside("Sheet No. 4411", "Sample Works Parts Return");
    for kind in [Kind::LabelValue, Kind::Totals] {
        let boxed = ds(&[(kind, &[0]), (Kind::Heading, &[1])]);
        assert!(
            !order_breaks(&left_box, &boxed, &reordered(&boxed, &[1, 0])),
            "{kind:?}"
        );
        let field = ds(&[(kind, &[0]), (Kind::Paragraph, &[1])]);
        assert!(
            order_breaks(&left_box, &field, &reordered(&field, &[1, 0])),
            "{kind:?}"
        );
    }
}

/// Relation (c): a label and its value on one row, and a column of labels beside the column
/// of their values, are read label first. The sign of a label is a colon at the end of each
/// line or a region typed label/value or totals; two blocks without one (two names side by
/// side) are free, as are the columns of a footer band. The row test is moved both ways.
#[test]
fn a_label_and_its_value_keep_their_order() {
    let fields = |labels: [&str; 2], down: f32| {
        page(
            vec![
                run(labels[0], 50.0, 700.0, 10.0, 0),
                run(labels[1], 50.0, 688.0, 10.0, 1),
                run("Sample part", 200.0, 700.0 - down, 10.0, 0),
                run("12", 200.0, 688.0 - down, 10.0, 1),
            ],
            Vec::new(),
        )
    };
    let two = |a: Kind, b: Kind| ds(&[(a, &[0, 1]), (b, &[2, 3])]);
    let base = two(Kind::Paragraph, Kind::Paragraph);
    let swapped = reordered(&base, &[1, 0]);
    let colons = ["Name:", "Count:"];
    assert!(order_breaks(&fields(colons, 0.0), &base, &swapped));
    // No sign of a label: free. Typed as fields: the order counts again.
    let plain = ["Name", "Count"];
    assert!(!order_breaks(&fields(plain, 0.0), &base, &swapped));
    for (a, b) in [
        (Kind::LabelValue, Kind::Paragraph),
        (Kind::Paragraph, Kind::LabelValue),
        (Kind::Totals, Kind::Paragraph),
    ] {
        let typed = two(a, b);
        assert!(order_breaks(
            &fields(plain, 0.0),
            &typed,
            &reordered(&typed, &[1, 0])
        ));
    }
    // The values 4.9 points under the labels are on their rows, 5.1 points are not.
    assert!(order_breaks(&fields(colons, 4.9), &base, &swapped));
    assert!(!order_breaks(&fields(colons, 5.1), &base, &swapped));
    // One row each.
    let one = ds(&[
        (Kind::Paragraph, &[0]),
        (Kind::Paragraph, &[2]),
        (Kind::Spatial, &[1, 3]),
    ]);
    assert!(order_breaks(
        &fields(colons, 0.0),
        &one,
        &reordered(&one, &[1, 0, 2])
    ));
    // Two labels against one value: not row by row.
    let uneven = ds(&[
        (Kind::Paragraph, &[0, 1]),
        (Kind::Paragraph, &[2]),
        (Kind::Spatial, &[3]),
    ]);
    assert!(!order_breaks(
        &fields(colons, 0.0),
        &uneven,
        &reordered(&uneven, &[1, 0, 2])
    ));
    // A label with a colon left of a title on its row: a title is no value, whatever the
    // label is typed. The same label left of a paragraph is read before it.
    let subject = page(
        vec![
            run("Subject:", 50.0, 700.0, 10.0, 0),
            run("Sample Works Report", 300.0, 700.0, 10.0, 0),
        ],
        Vec::new(),
    );
    for label in [Kind::Paragraph, Kind::LabelValue, Kind::Totals] {
        let title = ds(&[(label, &[0]), (Kind::Heading, &[1])]);
        assert!(
            !order_breaks(&subject, &title, &reordered(&title, &[1, 0])),
            "{label:?}"
        );
        let value = ds(&[(label, &[0]), (Kind::Paragraph, &[1])]);
        assert!(
            order_breaks(&subject, &value, &reordered(&value, &[1, 0])),
            "{label:?}"
        );
    }
    // The columns of a footer band.
    let band = two(Kind::FooterBand, Kind::FooterBand);
    assert!(!order_breaks(
        &fields(colons, 0.0),
        &band,
        &reordered(&band, &[1, 0])
    ));
}

/// Relation (c) when each label and each value is a region of its own: a value is read
/// right after its label. All labels first and all values after keeps every label before its
/// value and still parts each pair, so it is a break. Without the sign of a label the four
/// blocks are free.
#[test]
fn a_label_and_its_value_are_not_parted() {
    let fields = |labels: [&str; 2]| {
        page(
            vec![
                run(labels[0], 50.0, 700.0, 10.0, 0),
                run(labels[1], 50.0, 688.0, 10.0, 1),
                run("Sample part", 200.0, 700.0, 10.0, 0),
                run("12", 200.0, 688.0, 10.0, 1),
            ],
            Vec::new(),
        )
    };
    let rows = |label: Kind| {
        ds(&[
            (label, &[0]),
            (Kind::Paragraph, &[2]),
            (label, &[1]),
            (Kind::Paragraph, &[3]),
        ])
    };
    let base = rows(Kind::Paragraph);
    let colons = fields(["Name:", "Count:"]);
    let plain = fields(["Name", "Count"]);
    assert!(!order_breaks(&colons, &base, &base));
    // All labels, then all values.
    let parted = [0, 2, 1, 3];
    assert!(order_breaks(&colons, &base, &reordered(&base, &parted)));
    // The same with typed labels and no colon.
    let typed = rows(Kind::LabelValue);
    assert!(order_breaks(&plain, &typed, &reordered(&typed, &parted)));
    // One pair whole, the label of the other read first on the page and its value last.
    assert!(order_breaks(
        &colons,
        &base,
        &reordered(&base, &[2, 0, 1, 3])
    ));
    // The rows in the other order: each value still right after its label, and the rows
    // are of one flow, so this breaks by the flow and not by the bond. Two names side by
    // side on each row, with no sign of a label, are free to be read by columns.
    assert!(order_breaks(
        &colons,
        &base,
        &reordered(&base, &[2, 3, 0, 1])
    ));
    assert!(!order_breaks(&plain, &base, &reordered(&base, &parted)));
}

/// Relation (d): a caption beside its picture stays with it in the order the page has. Text
/// beside a picture that does not open as a caption is free. The share of the caption's
/// height on the rows of the picture is moved both ways.
#[test]
fn a_caption_and_its_picture_keep_their_order() {
    let beside = |text: &str, y: f32| {
        let mut p = page(vec![run(text, 300.0, y, 10.0, 0)], Vec::new());
        p.pictures.push(PictureIn {
            bounds: Bounds {
                x0: 50.0,
                y0: 500.0,
                x1: 250.0,
                y1: 700.0,
            },
        });
        p
    };
    let base: Decisions = vec![
        (Kind::Figure, vec![PrimId::Picture(0)], String::new()),
        (Kind::Paragraph, vec![PrimId::Text(0)], String::new()),
    ];
    let swapped = reordered(&base, &[1, 0]);
    let caption = "Figure 3 shows the part";
    assert!(order_breaks(&beside(caption, 600.0), &base, &swapped));
    // The caption first on the page: that order is kept too.
    assert!(order_breaks(&beside(caption, 600.0), &swapped, &base));
    assert!(!order_breaks(
        &beside("The part is shown here", 600.0),
        &base,
        &swapped
    ));
    // The picture ends at 700: a caption from 695 to 705 is on its rows, one from 696 not.
    assert!(order_breaks(&beside(caption, 695.0), &base, &swapped));
    assert!(!order_breaks(&beside(caption, 696.0), &base, &swapped));
}

/// Relation (d) with two pictures, each with its caption beside it: a caption is read right
/// after its picture. All pictures first and all captions after keeps every caption behind
/// its picture and still parts each pair, so it is a break. The same texts without a caption
/// word are free to be read by columns.
#[test]
fn a_caption_and_its_picture_are_not_parted() {
    let beside = |texts: [&str; 2]| {
        let mut p = page(
            vec![
                run(texts[0], 300.0, 600.0, 10.0, 0),
                run(texts[1], 300.0, 300.0, 10.0, 1),
            ],
            Vec::new(),
        );
        for (y0, y1) in [(500.0, 700.0), (200.0, 400.0)] {
            p.pictures.push(PictureIn {
                bounds: Bounds {
                    x0: 50.0,
                    y0,
                    x1: 250.0,
                    y1,
                },
            });
        }
        p
    };
    let base: Decisions = vec![
        (Kind::Figure, vec![PrimId::Picture(0)], String::new()),
        (Kind::Paragraph, vec![PrimId::Text(0)], String::new()),
        (Kind::Figure, vec![PrimId::Picture(1)], String::new()),
        (Kind::Paragraph, vec![PrimId::Text(1)], String::new()),
    ];
    let captions = beside(["Figure 1 shows the part", "Figure 2 shows the cover"]);
    assert!(!order_breaks(&captions, &base, &base));
    // All pictures, then all captions.
    let parted = reordered(&base, &[0, 2, 1, 3]);
    assert!(order_breaks(&captions, &base, &parted));
    // No caption word: the texts beside the pictures are free.
    let plain = beside(["The part is shown here", "The cover is shown here"]);
    assert!(!order_breaks(&plain, &base, &parted));
}

/// Relation (e): a sentence that runs from one region into the next. The second region is
/// read right after the first: a block read between the two stands inside the sentence, even
/// when it stands beside both. When the first region ends its sentence the block is free.
#[test]
fn nothing_stands_inside_a_sentence_that_runs_on() {
    let text = |end: &str, start: &str| {
        page(
            vec![
                run("The cover is held by four", 50.0, 700.0, 10.0, 0),
                run(end, 50.0, 688.0, 10.0, 1),
                run(start, 50.0, 664.0, 10.0, 2),
                run("Side note", 400.0, 690.0, 10.0, 3),
            ],
            Vec::new(),
        )
    };
    let base = ds(&[
        (Kind::Paragraph, &[0, 1]),
        (Kind::Paragraph, &[2]),
        (Kind::Paragraph, &[3]),
    ]);
    let between = reordered(&base, &[0, 2, 1]);
    let first = reordered(&base, &[2, 0, 1]);
    let p = text("screws, and each of them", "takes a washer.");
    assert!(order_breaks(&p, &base, &between));
    // The block before both parts or after both: inside nothing.
    assert!(!order_breaks(&p, &base, &first));
    // The sentence ends in the first region, or the next region opens a new one: free.
    assert!(!order_breaks(
        &text("screws of one size.", "takes a washer."),
        &base,
        &between
    ));
    assert!(!order_breaks(
        &text("screws, and each of them", "Each takes a washer."),
        &base,
        &between
    ));
    // A last line that is not full ends the text (19 of 25 letters; 20 are four fifths), and
    // so does a line alone.
    assert!(order_breaks(
        &text("screws, and each one", "takes a washer."),
        &base,
        &between
    ));
    assert!(!order_breaks(
        &text("screws, and each on", "takes a washer."),
        &base,
        &between
    ));
    let alone = ds(&[
        (Kind::Spatial, &[0]),
        (Kind::Paragraph, &[1]),
        (Kind::Paragraph, &[2]),
        (Kind::Paragraph, &[3]),
    ]);
    assert!(!order_breaks(&p, &alone, &reordered(&alone, &[0, 1, 3, 2])));
    // The second part at the head of another column, short lines, no prose: still held.
    let mut over = text("screws, and each of them", "takes a washer.");
    over.texts[2].x = 220.0;
    over.texts[2].y = 700.0;
    let two = ds(&[
        (Kind::Paragraph, &[0, 1]),
        (Kind::Paragraph, &[2]),
        (Kind::Spatial, &[3]),
    ]);
    assert!(order_breaks(&over, &two, &reordered(&two, &[1, 0, 2])));
    // A block to the left under the first part goes on nothing: the pair is free.
    over.texts[2].x = 50.0;
    over.texts[2].y = 664.0;
    for t in &mut over.texts[..2] {
        t.x = 300.0;
    }
    over.texts[3].y = 500.0;
    assert!(!order_breaks(&over, &two, &reordered(&two, &[1, 0, 2])));
}

/// The free case: blocks that do not depend on each other. A head block against a date box,
/// two boxes of short lines side by side and a note beside a paragraph may be read in any
/// order, and no order of them is a break.
#[test]
fn independent_blocks_side_by_side_are_free() {
    let p = page(
        vec![
            // A head block at the left, three lines.
            run("Sample Works and Sons", 50.0, 760.0, 10.0, 0),
            run("4 Mill Lane.", 50.0, 748.0, 10.0, 1),
            run("Lowtown.", 50.0, 736.0, 10.0, 2),
            // A date box at the right, two lines.
            run("Sheet 4", 420.0, 760.0, 10.0, 0),
            run("Third of May.", 420.0, 748.0, 10.0, 1),
            // Two boxes of like width side by side, short lines.
            run("Sent to the north store.", 50.0, 600.0, 10.0, 3),
            run("Dock 2.", 50.0, 588.0, 10.0, 4),
            run("Sent to the south store.", 300.0, 600.0, 10.0, 3),
            run("Dock 7.", 300.0, 588.0, 10.0, 4),
        ],
        Vec::new(),
    );
    let base = ds(&[
        (Kind::Paragraph, &[0, 1, 2]),
        (Kind::Paragraph, &[3, 4]),
        (Kind::Paragraph, &[5, 6]),
        (Kind::Paragraph, &[7, 8]),
    ]);
    for to in [[1, 0, 2, 3], [0, 1, 3, 2], [1, 0, 3, 2]] {
        assert!(!order_breaks(&p, &base, &reordered(&base, &to)), "{to:?}");
    }
    // The boxes under the head block are of its flow: read before it, the order is broken.
    assert!(order_breaks(&p, &base, &reordered(&base, &[2, 0, 1, 3])));
}
