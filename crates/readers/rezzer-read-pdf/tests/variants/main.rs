// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Variants of every print form shape of plan 6 (ruling P6-Y: Rezzer reads similar forms as
//! well as the one sample). Each shape is drawn along the axes of [`axes`]: the page scale, the
//! text sizes, the data font (monospaced or proportional), the label font, the look of the
//! rules, the turn of the page, a neighbour over, under or beside it, and one or two pages.
//! A deterministic sample of their combinations (each value alone, then a pairwise cover) is
//! read, and every read is held to the invariants of [`inv`]: never a wrong pair, every value
//! once, pairs where the layout is clear, a named loss where it is not, the same reading at
//! every scale and font, and no loss where the read at scale 1 has none. What breaks today is
//! pinned by name in [`known`]. All text and values are invented.

#[path = "../common/mod.rs"]
mod common;

#[path = "../form_fields/check.rs"]
#[allow(dead_code)]
mod check;
#[path = "../form_fields/draw.rs"]
#[allow(dead_code)]
mod draw;
#[path = "../form_fields/shapes.rs"]
#[allow(dead_code)]
mod shapes;

mod axes;
mod docs;
mod forms;
mod inv;
mod known;
mod pictures;

use axes::{ALL, Shape};
use inv::{Broken, Inv, hold, unexpected_in};
use known::{Cause, Known};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// Every shape of every family.
fn all_shapes() -> Vec<Shape> {
    let mut all = forms::shapes();
    all.extend(docs::shapes());
    all.extend(pictures::shapes());
    all
}

/// The shape named `name`.
fn shape(name: &str) -> Shape {
    all_shapes()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no shape {name}"))
}

/// The sample holds every pair of values of two axes, the base first and each value alone.
#[test]
fn the_sample_covers_every_pair_of_axis_values() {
    let sample = ALL.sample();
    assert_eq!(sample[0], axes::BASE);
    let axes: Vec<Vec<String>> = sample
        .iter()
        .map(|v| {
            vec![
                format!("{:?}", v.scale),
                format!("{:?}", v.size),
                format!("{:?}", v.data),
                format!("{:?}", v.labels),
                format!("{:?}", v.look),
                format!("{:?}", v.turn),
                format!("{:?}", v.next),
                format!("{:?}", v.pages),
            ]
        })
        .collect();
    let values: Vec<Vec<String>> = (0..8)
        .map(|a| {
            let mut vs: Vec<String> = axes.iter().map(|r| r[a].clone()).collect();
            vs.sort();
            vs.dedup();
            vs
        })
        .collect();
    for a in 0..8 {
        for b in a + 1..8 {
            for va in &values[a] {
                for vb in &values[b] {
                    assert!(
                        axes.iter().any(|r| r[a] == *va && r[b] == *vb),
                        "{va} with {vb}"
                    );
                }
            }
        }
    }
    // Every value of the axes stands in the sample.
    assert_eq!(
        values.iter().map(Vec::len).collect::<Vec<_>>(),
        [4, 3, 3, 2, 5, 3, 5, 2]
    );
    // The same sample on every run, and small enough to read.
    assert_eq!(sample, ALL.sample());
    assert!(sample.len() < 90, "{}", sample.len());
    let names: Vec<String> = sample.iter().map(|v| v.name()).collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "{names:?}");
}

/// The check of pinned breaks bites: a pinned variant that breaks another invariant, or
/// none, is reported; so is a variant that is not pinned and breaks one, and an entry whose
/// variant is not in the sample.
#[test]
fn pinned_breaks_are_held_exactly() {
    let known = [Known {
        shape: "s",
        variants: &["k"],
        broken: &[Inv::ClearPairs],
        cause: Cause::Threshold,
        why: "a reason",
    }];
    let b = |inv: Inv| Broken(inv, String::new());
    let read = |name: &str, broken: Vec<Broken>| vec![(name.to_string(), broken)];
    assert!(unexpected_in("s", &read("k", vec![b(Inv::ClearPairs)]), &known).is_empty());
    assert_eq!(
        unexpected_in(
            "s",
            &read("k", vec![b(Inv::ClearPairs), b(Inv::Table)]),
            &known
        )
        .len(),
        1
    );
    assert_eq!(unexpected_in("s", &read("k", Vec::new()), &known).len(), 1);
    // A variant that is not pinned, and the pinned one missing from the sample.
    assert_eq!(
        unexpected_in("s", &read("other", vec![b(Inv::Heading)]), &known).len(),
        2
    );
    // Another shape's entry does not pin this shape's variant.
    assert_eq!(
        unexpected_in("t", &read("k", vec![b(Inv::ClearPairs)]), &known).len(),
        1
    );
}

/// Every pinned entry names a shape that exists, its variants, what they break and a reason;
/// a break of ruling P6-X says so in its reason, and no variant is pinned twice.
#[test]
fn every_pinned_entry_names_a_shape_and_a_reason() {
    let names: Vec<&str> = all_shapes().iter().map(|s| s.name).collect();
    let mut pinned = Vec::new();
    for k in known::KNOWN {
        assert!(names.contains(&k.shape), "{}", k.shape);
        assert!(!k.why.is_empty() && !k.variants.is_empty() && !k.broken.is_empty());
        if k.broken.iter().any(|i| i.wrong()) {
            assert!(k.why.starts_with("P6-X"), "{}: {}", k.shape, k.why);
        }
        for v in k.variants {
            pinned.push((k.shape, *v));
        }
    }
    let n = pinned.len();
    pinned.sort();
    pinned.dedup();
    assert_eq!(pinned.len(), n, "a variant is pinned twice");
}

/// A value drawn more than once on a page is to be found as many times as it is drawn
/// (`20` stands in both rows of a section, `Muster` in a cell and in the address under it),
/// and a read that holds it fewer or more times breaks `ValueOnce`.
#[test]
fn a_value_drawn_twice_is_expected_twice() {
    let page = axes::page_of(&shape("section grids"), axes::BASE);
    let values = &page.sheets[0].built.values;
    let times = |t: &str| values.iter().filter(|v| *v == t).count();
    assert_eq!((times("20"), times("Muster"), times("1001")), (2, 2, 1));
    let out = inv::read(&page);
    assert!(inv::check_read(&out, &page).is_empty());
    let mut more = axes::page_of(&shape("section grids"), axes::BASE);
    more.sheets[0].built.values.push("20".into());
    let broken = inv::check_read(&out, &more);
    assert!(
        broken
            .iter()
            .any(|b| b.0 == Inv::ValueOnce && b.1.contains("\"20\" stands 2 times, drawn 3")),
        "{broken:?}"
    );
}

/// A value drawn in two parts (each word its own text) is to be found once, joined as the
/// read joins it; it is not left out.
#[test]
fn a_value_drawn_in_parts_is_expected_joined() {
    let mut sheet = axes::Sheet::new(shapes::Built::new(&axes::BASE.geo()));
    sheet.built.page.template = common::show("FL", 9.0, 40.0, 700.0, "Erika")
        + &common::show("FL", 9.0, 70.0, 700.0, "Mustermann");
    sheet.once = vec!["Erika Mustermann".into(), "Erika".into()];
    axes::expect_drawn(&mut sheet, 1);
    let times = |t: &str| sheet.built.values.iter().filter(|v| *v == t).count();
    assert_eq!((times("Erika Mustermann"), times("Erika")), (1, 1));
}

/// Two reads that differ only in their losses do not read the same: a loss where the
/// reference has none is `NoLoss`, another loss where it has one is `SameReading`.
#[test]
fn reads_that_differ_only_in_their_losses_differ() {
    let page = axes::page_of(&shape("section grids"), axes::BASE);
    let theirs = inv::read(&page);
    assert!(theirs.document.losses.is_empty());
    let loss = |what: &str| rezzer_model::Loss {
        location: "p1".into(),
        what: what.into(),
        why: rezzer_model::LossReason::Uncertain,
        severity: rezzer_model::Severity::Info,
    };
    let held = |mine: &rezzer_model::ReadOutcome, theirs: &rezzer_model::ReadOutcome| {
        let mut broken = Vec::new();
        inv::compare(mine, theirs, 0, &mut broken);
        broken.into_iter().map(|b| b.0).collect::<Vec<Inv>>()
    };
    assert!(held(&theirs, &theirs).is_empty());
    let mut mine = theirs.clone();
    mine.document
        .losses
        .push(loss("the field Name is not read"));
    assert_eq!(held(&mine, &theirs), [Inv::NoLoss]);
    let mut other = theirs.clone();
    other
        .document
        .losses
        .push(loss("the field Art is not read"));
    assert_eq!(held(&mine, &other), [Inv::SameReading]);
    // The numbers of a loss are masked: the same loss with another number reads the same.
    let (mut at, mut moved) = (theirs.clone(), theirs.clone());
    at.document.losses.push(loss("the number 12,34 is kept"));
    moved.document.losses.push(loss("the number 56,78 is kept"));
    assert!(held(&at, &moved).is_empty());
}

macro_rules! shape_tests {
    ($($test:ident => $name:literal,)*) => {
        $(
            #[test]
            fn $test() {
                hold(&shape($name));
            }
        )*

        /// Every shape has its test.
        #[test]
        fn every_shape_has_a_test() {
            let tested = [$($name),*];
            for s in all_shapes() {
                assert!(tested.contains(&s.name), "{}", s.name);
            }
        }
    };
}

shape_tests! {
    f1_to_f3 => "F1 to F3",
    header_grid => "header grid",
    column_of_totals => "column of totals",
    boxed_fields => "boxed fields",
    boxed_fields_beside_a_company => "boxed fields beside a company over its address",
    boxed_fields_beside_a_bold_title => "boxed fields beside a bold title",
    boxed_fields_beside_a_box_at_the_value_size => "boxed fields beside a box at the value size",
    a_strip_without_dividers_beside_a_smaller_name => "a strip without dividers beside a smaller name over its address",
    a_lone_field_in_a_strip_beside_a_smaller_name => "a lone field in a strip beside a smaller name over its address",
    a_strip_without_dividers_after_a_smaller_name => "a strip without dividers after a smaller name over its address",
    a_strip_of_a_smaller_label_beside_a_smaller_name => "a strip of a smaller label beside a smaller name over its address",
    a_lone_field_in_a_strip_beside_a_value => "a lone field in a strip beside a value",
    a_strip_without_dividers_beside_a_bold_title => "a strip without dividers beside a bold title",
    boxed_fields_beside_a_company_in_another_font => "boxed fields beside a company in another font",
    boxed_fields_beside_an_address => "boxed fields beside an address",
    label_block_and_bank_row => "label block and bank row",
    overflow_into_an_empty_field => "overflow into an empty field",
    a_short_value_after_a_full_field => "a short value after a full field",
    rows_of_fields => "rows of fields",
    labels_under_values => "labels under values",
    title_block_over_a_header_grid => "title block over a header grid",
    grids_side_by_side => "grids side by side",
    a_lone_field => "a lone field",
    separator_marks => "separator marks",
    section_grids => "section grids",
    band_table_with_empty_cells => "band table with empty cells",
    band_table_with_spaced_header_cells => "band table with spaced header cells",
    letter_head => "letter head",
    letter_head_with_a_larger_name => "letter head with a larger name",
    letter_head_with_the_date_beside_the_name => "letter head with the date beside the name",
    line_items_with_totals_between_rules => "line items with totals between rules over a note",
    line_items_with_an_end_rule => "line items with an end rule",
    line_items_with_row_rules => "line items with row rules",
    close_column_titles => "close column titles",
    title_beside_a_field_list => "title beside a field list",
    address_columns_in_the_body => "address columns in the body",
    an_address_beside_two_banks => "an address beside two banks",
    an_address_beside_two_phone_lines => "an address beside two phone lines and a register",
    an_address_beside_a_colon_heading => "an address beside a bank under a colon heading",
    a_second_table_after_a_row_set_apart => "a second table under a plain header after a row set apart",
    a_note_row_after_a_blank_row => "a note row after a blank row in a table",
    a_second_table_under_a_plain_header => "a second table under a plain header after a bold total",
    a_second_table_under_a_header_of_years => "a second table under a header of years after a bold total",
    a_small_table_of_records_under_a_plain_header => "a small table of records under a plain header",
    a_headerless_table_of_records => "a headerless table of records",
    labels_beside_values_with_a_last_label_alone => "labels beside values with a last label alone",
    blocks_side_by_side_under_their_headings => "blocks side by side under their headings",
    line_items_without_rules_over_an_end_rule => "line items without rules over an end rule",
    a_heading_over_a_rule_on_a_prose_page => "a heading over a rule on a prose page",
    a_heading_over_a_rule_beside_a_ruled_table => "a heading over a rule beside a ruled table",
    a_heading_over_a_rule_beside_a_boxed_note => "a heading over a rule beside a boxed note",
    weight_only_boxes => "weight-only boxes",
    a_column_flowing_past_a_framed_figure => "a column flowing past a framed figure",
    footer_columns => "footer columns",
    template_logos => "template logos",
    barcode_module_08_height_20 => "barcode, module 0.8, height 20",
    barcode_module_10_height_30 => "barcode, module 1.0, height 30",
    barcode_module_12_height_20 => "barcode, module 1.2, height 20",
    barcode_module_12_height_30 => "barcode, module 1.2, height 30",
    barcode_module_12_height_40 => "barcode, module 1.2, height 40",
    barcode_module_12_height_80 => "barcode, module 1.2, height 80",
    barcode_module_08_height_60 => "barcode, module 0.8, height 60",
}
