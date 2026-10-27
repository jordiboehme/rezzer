// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the splitter.

use super::{
    Names, Naming, OWN_TITLES, Unit, capped, finish_titles, part_budget, part_measure, slug, split,
    title_key,
};
use crate::RenderOptions;
use crate::describe::body_tokens;
use rezzer_model::{
    Anchor, Asset, Block, Confidence, Detection, Document, FormatId, Inline, SectionKind,
    SourceInfo,
};

fn doc() -> Document {
    Document::new(SourceInfo {
        path: None,
        name: "a report with a long name (final).md".into(),
        sha256: "00".into(),
        size: 1,
        format: FormatId::Markdown,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "test".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
        encoding: None,
        reader: None,
    })
}

/// A part file as the part writer will make it at worst: the longest title, the
/// description and both relations around the unit's body.
/// The main name is percent-encoded as the renderer writes a destination.
fn part_file(title: &str, doc_title: &str, main: &str, body: &str) -> String {
    let long = "y".repeat(super::TITLE_CAP);
    let main = main
        .replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29");
    format!(
        "---\ntype: source\n---\n\n# {doc_title}: {title}, part 9999\n\nPart 9999 of 9999 of \"{doc_title}\": {long}.\n\n{body}\n\n## Relations\n\n- part_of [{doc_title}](../{main})\n- follows [{doc_title}: {long}, part 9999]({}.md)\n",
        "z".repeat(60)
    )
}

#[test]
fn units_with_their_part_overhead_stay_within_the_part_size() {
    // Pictures (import length) and anchor links to other units (file names) in every
    // chapter: the finished part file is within the size by Crystalline's measure.
    let mut d = doc();
    for i in 0..40 {
        let id = format!("p{i}");
        d.assets.push(Asset {
            id: id.clone(),
            path: Some(format!("b.assets/{id}.png")),
            sha256: "00".into(),
            mime: "image/png".into(),
            caption: None,
            alt: Some("p".into()),
            needs_vision: false,
            codes: Vec::new(),
            code_unread: false,
        });
    }
    for c in 0..8 {
        d.body.push(Block::Heading {
            level: 2,
            inlines: vec![Inline::plain(format!("Chapter {c}"))],
            anchor: Some(Anchor(format!("h:chapter-{c}"))),
        });
        for k in 0..60 {
            let mut inlines = vec![Inline::plain("see ")];
            inlines.push(Inline::Link {
                target: format!("#chapter-{}", (c + 3) % 8),
                title: None,
                internal: true,
                wiki: false,
                inlines: vec![Inline::plain("there")],
            });
            inlines.push(Inline::plain(
                " and some words to fill the line up a bit more",
            ));
            d.body.push(Block::Paragraph {
                inlines,
                role: None,
                anchor: None,
            });
            if k % 10 == 0 {
                d.body.push(Block::Media {
                    asset: format!("p{}", (c * 6 + k / 10) % 40),
                    alt: None,
                    anchor: None,
                });
            }
        }
    }
    d.body.push(Block::Section {
        kind: SectionKind::Other,
        title: Some("Tail".into()),
        anchor: None,
        blocks: vec![],
    });
    let opts = RenderOptions::default();
    let bundle = "a-report-with-a-long-name-final-0123abcd";
    let list = split(&d, &opts, 2_500, bundle).expect("over the size");
    assert!(list.len() > 2);
    let budget = part_budget(&d, &opts, 2_500);
    let main = "a report with a long name (final).md-0123abcd.md";
    for u in &list {
        assert!(
            part_measure(bundle).chars(&u.markdown) <= budget,
            "{}",
            u.file
        );
        let file = part_file(
            &u.title,
            "a report with a long name (final).md",
            main,
            &u.markdown,
        );
        let note = bundle.len() + 1 + u.file.len();
        assert!(
            body_tokens(&file, note) <= 2_500,
            "{}: {}",
            u.file,
            body_tokens(&file, note)
        );
    }
}

#[test]
fn slugs_and_names() {
    assert_eq!(slug("Über alles!"), "ber-alles");
    assert_eq!(slug("  --A_b--  "), "a-b");
    let mut names = Names::new();
    assert_eq!(names.take("Index", "", 1), "index-2");
    assert_eq!(names.take("Contents", "", 2), "contents-2");
    assert_eq!(names.take("!!!", "", 3), "part-3");
    assert_eq!(names.take("Big", "-rows-2-10", 4), "big-rows-2-10");
    assert_eq!(names.take("Big", "-rows-2-10", 5), "big-rows-2-10-2");
    let long = "x".repeat(80);
    assert_eq!(names.take(&long, "-rows-1-2", 6).len(), 60);
}

#[test]
fn capped_cuts_at_a_space() {
    assert_eq!(capped("short", 10), "short");
    assert_eq!(capped("one two three four", 12), "one two...");
    assert_eq!(capped(&"x".repeat(20), 10), "xxxxxxx...");
}

#[test]
fn logic_parts_read_their_heading() {
    use super::{PartStart, logic_part};
    let begins = |p: &str| match logic_part(p) {
        Some(PartStart::Begins(t)) => Some(t),
        _ => None,
    };
    assert_eq!(
        begins("## Logic\n\n### Q1 \\*\n\ntext").as_deref(),
        Some("Q1 *")
    );
    assert!(matches!(
        logic_part("### Data (continued)\n\nrows"),
        Some(PartStart::Continues(t)) if t == "Data"
    ));
    assert!(logic_part("## Logic\n\n> [!WARNING] Old\n\n### Data").is_none());
}

fn unit(title: &str, location: &str, own: bool, piece: usize) -> Unit {
    Unit {
        file: String::new(),
        title: String::new(),
        location: location.into(),
        markdown: String::new(),
        naming: Naming {
            own,
            base: title.into(),
            location: location.into(),
            piece,
        },
    }
}

fn titled(units: &mut [Unit], extra: &[&str]) -> Vec<String> {
    finish_titles(units, extra);
    units.iter().map(|u| u.title.clone()).collect()
}

/// Every title Rezzer gives its own parts (ruling P5-AF): a source unit titled like it,
/// like a piece of it, or in another letter case gets its location, else `(section)`,
/// but only when the document really has that part (as one of its units or as a part
/// the main document adds). Rezzer's own unit keeps its title.
#[test]
fn every_own_title_is_checked() {
    for own in OWN_TITLES {
        let lower = own.to_lowercase();
        let piece = format!("{own}, part 2");
        for title in [own, lower.as_str(), piece.as_str()] {
            // The part is one of the units.
            assert_eq!(
                titled(
                    &mut [unit(own, own, true, 0), unit(title, title, false, 0)],
                    &[]
                ),
                [own.to_string(), format!("{title} (section)")]
            );
            assert_eq!(
                titled(
                    &mut [
                        unit(own, own, true, 0),
                        unit(title, "pages 1 to 2", false, 0)
                    ],
                    &[]
                ),
                [own.to_string(), format!("{title} (pages 1 to 2)")]
            );
            // The main document writer adds the part.
            assert_eq!(
                titled(&mut [unit(title, title, false, 0)], &[own]),
                [format!("{title} (section)")]
            );
            // No such part: the title stays.
            assert_eq!(
                titled(&mut [unit(title, title, false, 0)], &[]),
                [title.to_string()]
            );
        }
    }
    // The pieces of a renamed unit keep one name.
    assert_eq!(
        titled(
            &mut [
                unit("Logic", "Logic", false, 1),
                unit("Logic", "Logic", false, 2),
                unit("Logic", "Logic", true, 0),
            ],
            &[]
        ),
        ["Logic (section)", "Logic (section), part 2", "Logic"]
    );
    assert_eq!(title_key(" Start "), "start");
    // The last guard keeps a unit of the source off a part the main document adds and
    // off its numbered pieces; a title that only starts like one stays.
    let mut taken = unit("", "", false, 0);
    taken.title = "List of parts, part 2".into();
    let mut similar = unit("", "", false, 0);
    similar.title = "List of parts, Sheet1".into();
    let mut own = unit("", "", true, 0);
    own.title = "Overview".into();
    assert_eq!(
        titled(&mut [taken, similar, own], &["List of parts", "Overview"]),
        [
            "List of parts, part 2 (2)",
            "List of parts, Sheet1",
            "Overview"
        ]
    );
    assert_eq!(
        titled(&mut [unit("Startup", "Startup", false, 0)], &["Start"]),
        ["Startup"]
    );
}
