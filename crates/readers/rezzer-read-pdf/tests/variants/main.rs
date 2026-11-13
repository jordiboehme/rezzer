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

#[path = "../../../../formats/layoutregions/tests/meta/mod.rs"]
mod meta;

mod axes;
mod classes;
mod docs;
mod forms;
mod inv;
mod known;
mod metamorphic;
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
    all.extend(classes::shapes());
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
    // The region invariants of Task 20 are pinned the same way, with the task that owns the
    // fix: a pinned kind break holds, the same variant without it or with a forced type more
    // is reported.
    let classes = [Known {
        shape: "s",
        variants: &["k"],
        broken: &[Inv::Table, Inv::RegionKind],
        cause: Cause::Class("Task 29"),
        why: "a reason",
    }];
    let kinds = vec![b(Inv::Table), b(Inv::RegionKind)];
    assert!(unexpected_in("s", &read("k", kinds), &classes).is_empty());
    assert_eq!(
        unexpected_in("s", &read("k", vec![b(Inv::Table)]), &classes).len(),
        1
    );
    let more = vec![b(Inv::Table), b(Inv::RegionKind), b(Inv::NoForcedType)];
    assert_eq!(unexpected_in("s", &read("k", more), &classes).len(), 1);
    let spatial = vec![b(Inv::Table), b(Inv::RegionKind), b(Inv::SpatialSaid)];
    assert_eq!(unexpected_in("s", &read("k", spatial), &classes).len(), 1);
}

/// A text the shape marks unclear that stands in a table or a label and value region breaks
/// `NoForcedType`, a break of ruling P6-X; in a paragraph or a block kept as laid out it holds.
#[test]
fn forced_type_is_wrong() {
    let region = |kind: &'static str, texts: &[&str]| inv::Seen {
        kind,
        host: 0,
        texts: texts.iter().map(|t| t.to_string()).collect(),
    };
    let table = [
        region("paragraph", &["Abrechnung"]),
        region("table", &["Kundennr:", "4711", "Satz %", "19"]),
    ];
    let broken = inv::forced(&["Kundennr", "Satz %"], &table, 1);
    assert_eq!(broken.len(), 2, "{broken:?}");
    assert!(
        broken
            .iter()
            .all(|b| b.0 == Inv::NoForcedType && b.0.wrong())
    );
    let pairs = [region("label_value", &["Kundennr", "4711"])];
    assert_eq!(inv::forced(&["Kundennr"], &pairs, 1).len(), 1);
    let said = [
        region("spatial", &["Kundennr", "4711"]),
        region("paragraph", &["Satz %", "19"]),
    ];
    assert!(inv::forced(&["Kundennr", "Satz %"], &said, 1).is_empty());
    // A word is matched whole: `Satz` is not `Satzung`.
    let other = [region("table", &["Satzung", "%"])];
    assert!(inv::forced(&["Satz %"], &other, 1).is_empty());
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
    a_picture_under_a_body_size_title => "a picture at the top of the right column under a title in the body size",
    f1_to_f3 => "F1 to F3",
    head_line_over_fields => "head line over fields",
    padded_period_beside_a_title_over_fields => "padded period beside a title over fields",
    header_grid => "header grid",
    column_of_totals => "column of totals",
    boxed_fields => "boxed fields",
    boxed_fields_beside_a_company => "boxed fields beside a company over its address",
    boxed_fields_beside_a_bold_title => "boxed fields beside a bold title",
    boxed_fields_beside_a_box_at_the_value_size => "boxed fields beside a box at the value size",
    boxed_fields_beside_a_name_in_one_size_but_for_noise => "boxed fields beside a name over its department in one size but for float noise",
    a_numbered_label_over_a_line_of_codes => "a numbered label over a line of codes in a strip",
    a_strip_without_dividers_beside_a_smaller_name => "a strip without dividers beside a smaller name over its address",
    a_lone_field_in_a_strip_beside_a_smaller_name => "a lone field in a strip beside a smaller name over its address",
    a_strip_without_dividers_after_a_smaller_name => "a strip without dividers after a smaller name over its address",
    a_strip_of_a_smaller_label_beside_a_smaller_name => "a strip of a smaller label beside a smaller name over its address",
    a_refused_strip_with_a_value_of_two_lines => "a refused strip with a value of two lines",
    a_column_of_boxes_of_one_style => "a column of boxes of one style",
    a_strip_with_dividers_and_no_ends => "a strip with dividers and no ends",
    two_named_addresses_between_rules => "two named addresses between rules",
    a_lone_field_in_a_strip_beside_a_value => "a lone field in a strip beside a value",
    a_strip_without_dividers_beside_a_bold_title => "a strip without dividers beside a bold title",
    a_strip_without_dividers_whose_labels_are_a_little_smaller => "a strip without dividers whose labels are 3 % smaller than their values",
    boxed_fields_beside_an_empty_box => "boxed fields beside an empty box",
    boxed_fields_beside_an_empty_field => "boxed fields beside an empty field",
    boxed_fields_beside_a_company_in_another_font => "boxed fields beside a company in another font",
    boxed_fields_beside_an_address => "boxed fields beside an address",
    label_block_and_bank_row => "label block and bank row",
    overflow_into_an_empty_field => "overflow into an empty field",
    a_short_value_after_a_full_field => "a short value after a full field",
    rows_of_fields => "rows of fields",
    labels_under_values => "labels under values",
    labels_under_values_with_a_value_line_on_a_label_line => "labels under values with a second value line on a label's line",
    title_block_over_a_header_grid => "title block over a header grid",
    grids_side_by_side => "grids side by side",
    a_lone_field => "a lone field",
    separator_marks => "separator marks",
    section_grids => "section grids",
    band_table_with_empty_cells => "band table with empty cells",
    band_table_with_spaced_header_cells => "band table with spaced header cells",
    a_head_block_beside_a_logo_over_a_letter => "a head block beside a logo over a letter",
    a_framed_label_and_value_column_at_a_close_pitch => "a framed label column and value column, rows 1.36 sizes apart beside a picture",
    a_framed_label_and_value_column_at_a_wide_pitch => "a framed label column and value column, rows 1.55 sizes apart",
    a_head_block_of_two_lines_beside_a_logo => "a head block of two lines beside a logo over a letter",
    a_letter_paragraph_that_runs_on_under_a_box_on_its_right => "a letter paragraph that runs on under a box on its right",
    a_paragraph_of_full_width_under_two_columns_of_prose => "a paragraph of full width under two columns of prose, the right one a line shorter",
    a_small_bold_label_over_a_rule_with_two_lines => "a small bold label over a rule with two lines",
    two_boxes_side_by_side_each_with_its_label => "two boxes side by side, each with its label",
    two_fields_beside_a_ruled_table_with_empty_rows => "two fields of a label over a rule beside a ruled table with empty rows",
    two_fields_beside_a_ruled_table_with_empty_rows_and_a_third_rule => "two fields of a label over a rule beside a ruled table with empty rows and the rule of a third field",
    two_fields_left_of_a_ruled_table_with_empty_rows => "two fields of a label over a rule left of a ruled table with empty rows and the rule of a third field",
    two_fields_beside_a_ruled_table_with_filled_rows => "two fields of a label over a rule beside a ruled table with filled rows and the rule of a third field",
    labels_on_touching_underlines_over_two_rows_of_fields => "labels on touching underlines over two rows of fields",
    letter_head => "letter head",
    letter_head_with_a_larger_name => "letter head with a larger name",
    letter_head_with_the_date_beside_the_name => "letter head with the date beside the name",
    line_items_with_a_larger_total_between_rules => "line items with a larger total between rules over a note",
    line_items_with_a_smaller_first_total_between_rules => "line items with a smaller first total between rules over a note",
    totals_currency => "totals with a currency column between rules over a note",
    totals_currency_larger => "totals with a currency column and a larger total between rules over a note",
    totals_label_right => "totals with the label right of its amount between rules over a note",
    totals_label_right_larger => "totals with the label right of its amount and a larger total between rules over a note",
    totals_whole => "totals with whole amounts between rules over a note",
    totals_whole_larger => "totals with whole amounts and a larger total between rules over a note",
    totals_percent => "totals with a percent column between rules over a note",
    totals_percent_larger => "totals with a percent column and a larger total between rules over a note",
    totals_percent_one_row => "totals with a percent column in one row between rules over a note",
    totals_percent_one_row_larger => "totals with a percent column in one row and a larger total between rules over a note",
    totals_larger_under_a_rule => "totals with a larger total under a rule",
    totals_percent_larger_under_a_rule => "totals with a percent column and a larger total under a rule",
    net_tax_gross_titles => "net, tax and gross titles over their amounts between rules over a note",
    lines_of_a_label_and_its_bold_value => "lines of a label and its bold value",
    lines_of_a_label_and_its_bold_value_two_boxed => "lines of a label and its bold value, two of them boxed",
    lines_of_a_label_and_its_bold_value_middle_boxed => "lines of a label and its bold value, the middle two boxed",
    a_column_of_bold_lines_boxed_at_the_end => "a column of bold lines at heading size with a box around two of them",
    a_column_of_bold_lines_boxed_in_the_middle => "a column of bold lines at heading size with a box around the middle two",
    a_column_of_lines_of_one_face => "a column of lines of one face",
    a_column_of_lines_of_one_face_boxed => "a column of lines of one face with a box around two of them",
    a_column_of_long_field_lines => "a column of one-face field lines of 5 to 7 words",
    a_column_of_long_field_lines_boxed => "a column of one-face field lines of 5 to 7 words with a box around two of them",
    a_column_of_bold_long_field_lines_boxed => "a column of bold field lines of 5 to 7 words at heading size, two boxed",
    slogan_lines => "slogan lines, each a sentence ending in an exclamation mark",
    bold_slogan_lines => "bold slogan lines, each a sentence ending in an exclamation mark",
    bold_lines_of_starred_items => "bold lines of starred items",
    plain_lines_of_starred_items => "plain lines of starred items",
    a_cut_row_label_and_a_row_of_numbers_over_prose => "a row label cut by a hyphen and a row of numbers over prose",
    two_lines_in_three_columns => "two lines in three columns alone in the body",
    a_value_printed_over_its_label => "a value printed over its label",
    a_value_that_touches_its_label => "a value that touches its label",
    prose_with_a_number_at_a_line_end => "prose with a number at a line end",
    two_lines_of_three_words_set_full => "two lines of three words set full",
    numbered_labels_beside_an_address => "numbered labels beside an address",
    prose_right_under_field_lines => "prose right under a column of field lines",
    a_paragraph_set_full_in_a_narrow_column => "a paragraph without a stop set full in a narrow column",
    paragraphs_across_a_frame_of_two_rules => "paragraphs with a short last line across a frame of two rules",
    pair_rows_over_a_larger_total_row => "rows of a label with a colon and its value over a larger total row",
    a_subtotal_over_a_larger_total => "a subtotal over a larger total without colons",
    name_rows_over_a_larger_last_row => "rows of a name and a word over a larger last row",
    contents_lines_with_page_numbers => "lines of a table of contents with their pages at one edge",
    line_items_with_a_hanging_credit_minus => "line items with a hanging credit minus over a total",
    line_items_with_a_deduction_under_an_amount => "line items with a deduction with a minus under an amount",
    line_items_with_a_credit_over_a_total => "line items with a credit over a total",
    an_accent_drawn_on_its_own => "an accent drawn on its own over its word",
    cells_of_two_lines_lining_up_by_chance => "cells of two lines lining up by chance between rules",
    cells_of_two_lines_lining_up_by_chance_with_no_divider => "cells of two lines lining up by chance between rules with no divider",
    cells_or_rows_of_two_lines_with_no_divider => "cells or rows of two lines between rules with no divider",
    labels_beside_a_street_with_no_divider => "labels beside a street and its town between rules with no divider",
    abbreviated_labels_beside_a_street_with_no_divider => "abbreviated labels beside a street and its town between rules with no divider",
    two_rows_of_items_with_no_divider => "two rows of items between rules with no divider",
    words_in_two_weights => "words in two weights",
    a_range_that_wraps_after_its_hyphen => "a range that wraps after its hyphen in prose",
    two_lines_in_three_columns_between_two_rules => "two lines in three columns between two rules",
    titles_over_a_row_that_ends_in_a_stop => "titles over a row that ends in a stop",
    short_field_lines_after_a_sentence => "a column of short field lines after a sentence",
    line_items_with_wrapped_date_ranges => "line items with wrapped date ranges",
    line_items_whose_last_period_wraps => "line items whose last period wraps under the table",
    line_items_over_two_lone_amounts => "line items over two amounts alone in two columns",
    a_notice_band_between_rules => "a notice band between rules",
    line_items_with_totals_between_rules => "line items with totals between rules over a note",
    line_items_with_an_end_rule => "line items with an end rule",
    line_items_with_row_rules => "line items with row rules",
    close_column_titles => "close column titles",
    title_beside_a_field_list => "title beside a field list",
    item_table_between_two_rules_with_a_picture_column => "item table between two rules with a picture column",
    description_wrapping_to_four_lines => "description wrapping to four lines",
    cells_of_several_lines_that_pair_up_line_by_line_in_a_ruled_grid => "cells of several lines that pair up line by line in a ruled grid",
    two_line_cells_with_one_value_on_the_second_line => "two-line cells with one value on the second line",
    cells_of_several_lines_under_a_cell_merged_over_two_rows => "cells of several lines under a cell merged over two rows",
    wrapped_text_beside_stacked_values_in_a_ruled_grid => "wrapped text beside stacked values in a ruled grid",
    an_item_beside_net_over_gross_amounts => "an item beside net over gross amounts under titles of two lines",
    columns_bounded_by_breaks_in_the_rules => "columns bounded by breaks in the rules",
    a_sentence_across_the_breaks_in_the_rules => "a sentence across the breaks in the rules",
    a_note_where_a_detail_line_would_stand => "a note where the detail line of the last item would stand",
    bold_item_names_over_plain_detail_lines_without_a_header => "bold item names over plain detail lines without a header",
    a_stamp_over_an_item_table_without_rules => "a stamp over an item table without rules",
    a_stamp_over_a_ruled_item_table => "a stamp over a ruled item table",
    flat_pictures_across_a_column_bound_in_item_rows => "flat pictures across a column bound in item rows",
    a_picture_over_the_gap_to_the_next_table => "a picture over the gap to the next table",
    item_rows_with_a_picture_in_the_description_column => "item rows with a picture in the description column",
    totals_between_two_rules_in_pieces_under_one_item_row => "totals between two rules in pieces under one item row",
    item_rows_under_a_rule_in_pieces_beside_a_block_of_lines => "item rows under a rule in pieces on a page with a block of lines on the right",
    item_rows_under_a_rule_in_pieces_and_a_row_carried_over => "item rows under a rule in pieces and a row carried over, on a page with a block of lines on the right",
    address_columns_in_the_body => "address columns in the body",
    an_address_beside_two_banks => "an address beside two banks",
    an_address_beside_two_phone_lines => "an address beside two phone lines and a register",
    an_address_beside_a_colon_heading => "an address beside a bank under a colon heading",
    a_second_table_after_a_row_set_apart => "a second table under a plain header after a row set apart",
    a_note_row_after_a_blank_row => "a note row after a blank row in a table",
    a_second_table_under_a_plain_header => "a second table under a plain header after a bold total",
    a_second_table_of_one_row => "a second table of one row under a plain header after a bold total",
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
    a_note_column_beside_rows_of_fields => "a note column beside rows of fields",
    an_amount_in_its_box_before_a_note_column => "an amount in its box before a note column",
    a_row_of_labels_over_two_blocks_of_fields => "a row of labels over two blocks of fields",
    a_row_of_labels_in_drawn_boxes_over_two_blocks_of_fields => "a row of labels over two blocks of fields in drawn boxes",
    template_logos => "template logos",
    a_ruler_of_short_ticks => "a ruler of short ticks",
    codes_in_a_barcode_font_named_for_codes_size_10 => "codes in a barcode font named for codes, size 10, gap 5",
    codes_in_a_barcode_font_named_for_codes_size_16 => "codes in a barcode font named for codes, size 16, gap 12",
    codes_in_a_barcode_font_known_by_its_bars_size_12 => "codes in a barcode font known by its bars, size 12, gap 5",
    codes_in_a_barcode_font_known_by_its_bars_size_16 => "codes in a barcode font known by its bars, size 16, gap 12",
    codes_in_a_barcode_font_known_by_its_bars_beside_a_photo => "codes in a barcode font known by its bars beside a photo, size 12, gap 5",
    codes_in_a_barcode_font_named_for_codes_beside_a_photo => "codes in a barcode font named for codes beside a photo, size 10, gap 5",
    codes_in_a_bar_a_glyph_truetype_subset_font => "codes in a bar-a-glyph TrueType subset font named BC C128 HD Wide, size 30",
    codes_in_a_bar_a_glyph_type1_font => "codes in a bar-a-glyph Type1 font known by its bars, size 30",
    codes_in_a_bar_a_glyph_cff_font => "codes in a bar-a-glyph CFF font named BC C128 HD Wide, size 36",
    codes_in_a_bar_a_glyph_full_truetype_font => "codes in a bar-a-glyph full TrueType font known by its bars, size 24",
    codes_in_a_bar_a_glyph_font_beside_a_photo => "codes in a bar-a-glyph TrueType subset font beside a photo, size 36",
    codes_in_the_real_shop_font_named => "codes in the real shop font's glyph program named BC C128 HD Wide, size 30",
    codes_in_the_real_shop_font_beside_a_photo => "codes in the real shop font's glyph program known by its bars beside a photo, size 34",
    two_codes_3_modules_apart_one_show => "two codes in a bar-a-glyph font 3 modules apart, one show",
    two_codes_3_modules_apart_two_shows => "two codes in a bar-a-glyph font 3 modules apart, two shows",
    two_codes_4_modules_apart_one_show => "two codes in a bar-a-glyph font 4 modules apart, one show",
    two_codes_4_modules_apart_two_shows => "two codes in a bar-a-glyph font 4 modules apart, two shows",
    two_codes_5_modules_apart_one_show => "two codes in a bar-a-glyph font 5 modules apart, one show",
    two_codes_5_modules_apart_two_shows => "two codes in a bar-a-glyph font 5 modules apart, two shows",
    two_codes_6_modules_apart_one_show => "two codes in a bar-a-glyph font 6 modules apart, one show",
    two_codes_6_modules_apart_two_shows => "two codes in a bar-a-glyph font 6 modules apart, two shows",
    two_codes_7_modules_apart_one_show => "two codes in a bar-a-glyph font 7 modules apart, one show",
    two_codes_7_modules_apart_two_shows => "two codes in a bar-a-glyph font 7 modules apart, two shows",
    two_codes_12_modules_apart_one_show => "two codes in a bar-a-glyph font 12 modules apart, one show",
    two_codes_12_modules_apart_two_shows => "two codes in a bar-a-glyph font 12 modules apart, two shows",
    a_coding_line_in_a_letter_font_named_idautomationocrb => "a coding line in a letter font named IDAutomationOCRb",
    a_coding_line_in_a_letter_font_named_ocean13 => "a coding line in a letter font named Ocean13",
    a_code_128_in_a_human_readable_bar_font => "a Code 128 in a human-readable bar font, size 12",
    a_code_39_in_a_bar_a_glyph_font => "a Code 39 in a bar-a-glyph font known by its bars, size 30",
    an_itf_in_a_bar_a_glyph_type1_font => "an ITF in a bar-a-glyph Type1 font known by its bars, size 30",
    a_code_128_shown_a_glyph_at_a_time => "a Code 128 shown a glyph at a time in a bar-a-glyph font known by its bars",
    an_ean_13_drawn_as_paths_module_0_62 => "an EAN-13 drawn as paths, module 0.62",
    an_ean_13_drawn_as_paths_beside_a_photo => "an EAN-13 drawn as paths beside a photo, module 0.57",
    a_ruler_of_short_ticks_beside_a_photo => "a ruler of short ticks beside a photo",
    barcode_module_08_height_20 => "barcode, module 0.8, height 20",
    barcode_module_10_height_30 => "barcode, module 1.0, height 30",
    barcode_module_12_height_20 => "barcode, module 1.2, height 20",
    barcode_module_12_height_30 => "barcode, module 1.2, height 30",
    barcode_module_12_height_40 => "barcode, module 1.2, height 40",
    barcode_module_12_height_80 => "barcode, module 1.2, height 80",
    barcode_module_08_height_60 => "barcode, module 0.8, height 60",
    barcode_module_05_height_3 => "barcode, module 0.5, height 3",
    barcode_module_05_height_5 => "barcode, module 0.5, height 5",
    barcode_module_05_height_8 => "barcode, module 0.5, height 8",
    barcode_module_05_height_20 => "barcode, module 0.5, height 20",
    barcode_module_05_height_30 => "barcode, module 0.5, height 30",
    barcode_module_05_height_40 => "barcode, module 0.5, height 40",
    barcode_module_05_height_80 => "barcode, module 0.5, height 80",
    barcode_module_05_height_150 => "barcode, module 0.5, height 150",
    barcode_module_08_height_3 => "barcode, module 0.8, height 3",
    barcode_module_08_height_5 => "barcode, module 0.8, height 5",
    barcode_module_08_height_8 => "barcode, module 0.8, height 8",
    barcode_module_08_height_30 => "barcode, module 0.8, height 30",
    barcode_module_08_height_40 => "barcode, module 0.8, height 40",
    barcode_module_08_height_80 => "barcode, module 0.8, height 80",
    barcode_module_08_height_150 => "barcode, module 0.8, height 150",
    barcode_module_12_height_3 => "barcode, module 1.2, height 3",
    barcode_module_12_height_5 => "barcode, module 1.2, height 5",
    barcode_module_12_height_8 => "barcode, module 1.2, height 8",
    barcode_module_12_height_150 => "barcode, module 1.2, height 150",
    photo_barcode_module_05_height_3 => "barcode beside a photo, module 0.5, height 3",
    photo_barcode_module_05_height_5 => "barcode beside a photo, module 0.5, height 5",
    photo_barcode_module_05_height_8 => "barcode beside a photo, module 0.5, height 8",
    photo_barcode_module_05_height_20 => "barcode beside a photo, module 0.5, height 20",
    photo_barcode_module_05_height_30 => "barcode beside a photo, module 0.5, height 30",
    photo_barcode_module_05_height_40 => "barcode beside a photo, module 0.5, height 40",
    photo_barcode_module_05_height_80 => "barcode beside a photo, module 0.5, height 80",
    photo_barcode_module_05_height_150 => "barcode beside a photo, module 0.5, height 150",
    photo_barcode_module_08_height_3 => "barcode beside a photo, module 0.8, height 3",
    photo_barcode_module_08_height_5 => "barcode beside a photo, module 0.8, height 5",
    photo_barcode_module_08_height_8 => "barcode beside a photo, module 0.8, height 8",
    photo_barcode_module_08_height_20 => "barcode beside a photo, module 0.8, height 20",
    photo_barcode_module_08_height_30 => "barcode beside a photo, module 0.8, height 30",
    photo_barcode_module_08_height_40 => "barcode beside a photo, module 0.8, height 40",
    photo_barcode_module_08_height_80 => "barcode beside a photo, module 0.8, height 80",
    photo_barcode_module_08_height_150 => "barcode beside a photo, module 0.8, height 150",
    photo_barcode_module_12_height_3 => "barcode beside a photo, module 1.2, height 3",
    photo_barcode_module_12_height_5 => "barcode beside a photo, module 1.2, height 5",
    photo_barcode_module_12_height_8 => "barcode beside a photo, module 1.2, height 8",
    photo_barcode_module_12_height_20 => "barcode beside a photo, module 1.2, height 20",
    photo_barcode_module_12_height_30 => "barcode beside a photo, module 1.2, height 30",
    photo_barcode_module_12_height_80 => "barcode beside a photo, module 1.2, height 80",
    photo_barcode_module_12_height_150 => "barcode beside a photo, module 1.2, height 150",
    twin_barcodes_1_pt_apart => "twin barcodes 1 pt apart",
    twin_barcodes_3_pt_apart => "twin barcodes 3 pt apart",
    twin_barcodes_4_pt_apart => "twin barcodes 4 pt apart",
    twin_barcodes_5_pt_apart => "twin barcodes 5 pt apart",
    short_twin_barcodes_3_pt_high_one_over_the_other => "twin barcodes 3 pt high 1 pt apart, one over the other",
    short_twin_barcodes_5_pt_high_one_over_the_other => "twin barcodes 5 pt high 1 pt apart, one over the other",
    twin_barcodes_1_pt_apart_one_over_the_other => "twin barcodes 1 pt apart, one over the other",
    twin_barcodes_3_pt_apart_one_over_the_other => "twin barcodes 3 pt apart, one over the other",
    twin_photo_barcodes_1_pt_apart => "twin barcodes beside a photo 1 pt apart",
    twin_photo_barcodes_3_pt_apart => "twin barcodes beside a photo 3 pt apart",
    twin_photo_barcodes_4_pt_apart => "twin barcodes beside a photo 4 pt apart",
    twin_photo_barcodes_5_pt_apart => "twin barcodes beside a photo 5 pt apart",
    short_twin_photo_barcodes_3_pt_high_one_over_the_other => "twin barcodes 3 pt high beside a photo 1 pt apart, one over the other",
    short_twin_photo_barcodes_5_pt_high_one_over_the_other => "twin barcodes 5 pt high beside a photo 1 pt apart, one over the other",
    twin_photo_barcodes_1_pt_apart_one_over_the_other => "twin barcodes beside a photo 1 pt apart, one over the other",
    twin_photo_barcodes_3_pt_apart_one_over_the_other => "twin barcodes beside a photo 3 pt apart, one over the other",
    photo_barcode_module_12_height_40 => "barcode beside a photo, module 1.2, height 40",
    many_photo_barcodes => "21 barcodes beside a photo",
    a_halftone_beside_a_photo => "a halftone beside a photo",
    scan_dots_2_5_px_0_deg_crisp => "a scan of dots 2.5 px apart at 0 degrees, crisp",
    scan_dots_2_5_px_0_deg_crisp_band => "a scan of dots 2.5 px apart at 0 degrees, crisp, a band",
    scan_dots_2_5_px_0_deg_blur => "a scan of dots 2.5 px apart at 0 degrees, blur 1",
    scan_dots_2_5_px_0_deg_blur_band => "a scan of dots 2.5 px apart at 0 degrees, blur 1, a band",
    scan_dots_3_6_px_45_deg_crisp => "a scan of dots 3.6 px apart at 45 degrees, crisp",
    scan_dots_3_6_px_45_deg_crisp_band => "a scan of dots 3.6 px apart at 45 degrees, crisp, a band",
    scan_dots_3_6_px_45_deg_blur => "a scan of dots 3.6 px apart at 45 degrees, blur 1",
    scan_dots_3_6_px_45_deg_blur_band => "a scan of dots 3.6 px apart at 45 degrees, blur 1, a band",
    scan_dots_4_2_px_45_deg_crisp => "a scan of dots 4.2 px apart at 45 degrees, crisp",
    scan_dots_4_2_px_45_deg_crisp_band => "a scan of dots 4.2 px apart at 45 degrees, crisp, a band",
    scan_dots_4_2_px_45_deg_blur => "a scan of dots 4.2 px apart at 45 degrees, blur 1",
    scan_dots_4_2_px_45_deg_blur_band => "a scan of dots 4.2 px apart at 45 degrees, blur 1, a band",
    scan_dots_5_0_px_15_deg_crisp => "a scan of dots 5.0 px apart at 15 degrees, crisp",
    scan_dots_5_0_px_15_deg_crisp_band => "a scan of dots 5.0 px apart at 15 degrees, crisp, a band",
    scan_dots_5_0_px_15_deg_blur => "a scan of dots 5.0 px apart at 15 degrees, blur 1",
    scan_dots_5_0_px_15_deg_blur_band => "a scan of dots 5.0 px apart at 15 degrees, blur 1, a band",
    scan_dots_6_0_px_75_deg_crisp => "a scan of dots 6.0 px apart at 75 degrees, crisp",
    scan_dots_6_0_px_75_deg_crisp_band => "a scan of dots 6.0 px apart at 75 degrees, crisp, a band",
    scan_dots_6_0_px_75_deg_blur => "a scan of dots 6.0 px apart at 75 degrees, blur 1",
    scan_dots_6_0_px_75_deg_blur_band => "a scan of dots 6.0 px apart at 75 degrees, blur 1, a band",
    scan_dots_3_0_px_30_deg_crisp => "a scan of dots 3.0 px apart at 30 degrees, crisp",
    scan_dots_3_0_px_30_deg_crisp_band => "a scan of dots 3.0 px apart at 30 degrees, crisp, a band",
    scan_dots_3_0_px_30_deg_blur => "a scan of dots 3.0 px apart at 30 degrees, blur 1",
    scan_dots_3_0_px_30_deg_blur_band => "a scan of dots 3.0 px apart at 30 degrees, blur 1, a band",
    scan_lines_2_2_px_0_deg_crisp => "a scan of lines 2.2 px apart at 0 degrees, crisp",
    scan_lines_2_2_px_0_deg_crisp_band => "a scan of lines 2.2 px apart at 0 degrees, crisp, a band",
    scan_lines_2_2_px_0_deg_blur => "a scan of lines 2.2 px apart at 0 degrees, blur 1",
    scan_lines_2_2_px_0_deg_blur_band => "a scan of lines 2.2 px apart at 0 degrees, blur 1, a band",
    scan_lines_3_3_px_90_deg_crisp => "a scan of lines 3.3 px apart at 90 degrees, crisp",
    scan_lines_3_3_px_90_deg_crisp_band => "a scan of lines 3.3 px apart at 90 degrees, crisp, a band",
    scan_lines_3_3_px_90_deg_blur => "a scan of lines 3.3 px apart at 90 degrees, blur 1",
    scan_lines_3_3_px_90_deg_blur_band => "a scan of lines 3.3 px apart at 90 degrees, blur 1, a band",
    scan_lines_4_0_px_45_deg_crisp => "a scan of lines 4.0 px apart at 45 degrees, crisp",
    scan_lines_4_0_px_45_deg_crisp_band => "a scan of lines 4.0 px apart at 45 degrees, crisp, a band",
    scan_lines_4_0_px_45_deg_blur => "a scan of lines 4.0 px apart at 45 degrees, blur 1",
    scan_lines_4_0_px_45_deg_blur_band => "a scan of lines 4.0 px apart at 45 degrees, blur 1, a band",
    scan_lines_5_5_px_90_deg_crisp => "a scan of lines 5.5 px apart at 90 degrees, crisp",
    scan_lines_5_5_px_90_deg_crisp_band => "a scan of lines 5.5 px apart at 90 degrees, crisp, a band",
    scan_lines_5_5_px_90_deg_blur => "a scan of lines 5.5 px apart at 90 degrees, blur 1",
    scan_lines_5_5_px_90_deg_blur_band => "a scan of lines 5.5 px apart at 90 degrees, blur 1, a band",
    scan_square_dots_3_5_px_0_deg_crisp => "a scan of square dots 3.5 px apart at 0 degrees, crisp",
    scan_square_dots_3_5_px_0_deg_crisp_band => "a scan of square dots 3.5 px apart at 0 degrees, crisp, a band",
    scan_square_dots_3_5_px_0_deg_blur => "a scan of square dots 3.5 px apart at 0 degrees, blur 1",
    scan_square_dots_3_5_px_0_deg_blur_band => "a scan of square dots 3.5 px apart at 0 degrees, blur 1, a band",
    scan_square_dots_5_0_px_30_deg_crisp => "a scan of square dots 5.0 px apart at 30 degrees, crisp",
    scan_square_dots_5_0_px_30_deg_crisp_band => "a scan of square dots 5.0 px apart at 30 degrees, crisp, a band",
    scan_square_dots_5_0_px_30_deg_blur => "a scan of square dots 5.0 px apart at 30 degrees, blur 1",
    scan_square_dots_5_0_px_30_deg_blur_band => "a scan of square dots 5.0 px apart at 30 degrees, blur 1, a band",
    halftone_square_dots_1_px_crisp => "halftone square dots, 1 px a point, crisp",
    halftone_square_dots_1_px_blur_1 => "halftone square dots, 1 px a point, blur 1",
    halftone_square_dots_1_px_blur_2 => "halftone square dots, 1 px a point, blur 2",
    halftone_square_dots_2_px_crisp => "halftone square dots, 2 px a point, crisp",
    halftone_square_dots_2_px_blur_1 => "halftone square dots, 2 px a point, blur 1",
    halftone_square_dots_2_px_blur_2 => "halftone square dots, 2 px a point, blur 2",
    halftone_square_dot_bands_1_px_crisp => "halftone square dot bands, 1 px a point, crisp",
    halftone_square_dot_bands_1_px_blur_1 => "halftone square dot bands, 1 px a point, blur 1",
    halftone_square_dot_bands_1_px_blur_2 => "halftone square dot bands, 1 px a point, blur 2",
    halftone_square_dot_bands_2_px_crisp => "halftone square dot bands, 2 px a point, crisp",
    halftone_square_dot_bands_2_px_blur_1 => "halftone square dot bands, 2 px a point, blur 1",
    halftone_square_dot_bands_2_px_blur_2 => "halftone square dot bands, 2 px a point, blur 2",
    halftone_dots_1_px_crisp => "halftone dots, 1 px a point, crisp",
    halftone_dots_1_px_blur_1 => "halftone dots, 1 px a point, blur 1",
    halftone_dots_1_px_blur_2 => "halftone dots, 1 px a point, blur 2",
    halftone_dots_2_px_crisp => "halftone dots, 2 px a point, crisp",
    halftone_dots_2_px_blur_1 => "halftone dots, 2 px a point, blur 1",
    halftone_dots_2_px_blur_2 => "halftone dots, 2 px a point, blur 2",
    halftone_dot_bands_1_px_crisp => "halftone dot bands, 1 px a point, crisp",
    halftone_dot_bands_1_px_blur_1 => "halftone dot bands, 1 px a point, blur 1",
    halftone_dot_bands_1_px_blur_2 => "halftone dot bands, 1 px a point, blur 2",
    halftone_dot_bands_2_px_crisp => "halftone dot bands, 2 px a point, crisp",
    halftone_dot_bands_2_px_blur_1 => "halftone dot bands, 2 px a point, blur 1",
    halftone_dot_bands_2_px_blur_2 => "halftone dot bands, 2 px a point, blur 2",
    halftone_lines_1_px_crisp => "halftone lines, 1 px a point, crisp",
    halftone_lines_1_px_blur_1 => "halftone lines, 1 px a point, blur 1",
    halftone_lines_1_px_blur_2 => "halftone lines, 1 px a point, blur 2",
    halftone_lines_2_px_crisp => "halftone lines, 2 px a point, crisp",
    halftone_lines_2_px_blur_1 => "halftone lines, 2 px a point, blur 1",
    halftone_lines_2_px_blur_2 => "halftone lines, 2 px a point, blur 2",
    halftone_line_bands_1_px_crisp => "halftone line bands, 1 px a point, crisp",
    halftone_line_bands_1_px_blur_1 => "halftone line bands, 1 px a point, blur 1",
    halftone_line_bands_1_px_blur_2 => "halftone line bands, 1 px a point, blur 2",
    halftone_line_bands_2_px_crisp => "halftone line bands, 2 px a point, crisp",
    halftone_line_bands_2_px_blur_1 => "halftone line bands, 2 px a point, blur 1",
    halftone_line_bands_2_px_blur_2 => "halftone line bands, 2 px a point, blur 2",
    scan_dots_3_6_px_52_deg_crisp => "a scan of dots 3.6 px apart at 52 degrees, crisp",
    scan_dots_2_7_px_83_deg_crisp => "a scan of dots 2.7 px apart at 83 degrees, crisp",
    scan_lines_3_3_px_standing => "a scan of lines 3.3 px apart standing up",
    scan_dots_3_5_px_2_px_blur_1 => "a scan of dots 3.5 px apart, 2 px a point, blur 1",
    scan_dots_3_6_px_2_px_blur_1 => "a scan of dots 3.6 px apart, 2 px a point, blur 1",
    scan_dots_3_8_px_2_px_blur_1 => "a scan of dots 3.8 px apart, 2 px a point, blur 1",
    letters_times => "stems and strokes in Times-Roman, twice as large",
    ruled_code_1_px => "a printed barcode crossed by a dark line 1 px wide",
    ruled_code_4_px => "a printed barcode crossed by a dark line 4 px wide",
    ruled_code_lying_2_px => "a printed barcode lying, crossed by a dark line 2 px wide",
    printed_code_cut_30 => "a printed barcode cut by the edge of the page to 30 modules",
    printed_code_cut_45_blur => "a printed barcode cut by the edge of the page to 45 modules, blurred",
    printed_code_short_bars_blur => "a printed barcode with bars 6 modules long, blurred",
    printed_qr_touching => "two printed QR codes touching",
    printed_qr_1_module_apart => "two printed QR codes 1 module apart",
    printed_qr_cut => "a printed QR code cut in half by the edge of the page",
}

/// Task 15b part 2d fix round 3 (M1): the cell check counts the texts it leaves out, and a list
/// of cells whose every text is left out (drawn twice, or inside another text) is caught, so the
/// check never passes empty; a glued value is still found.
#[test]
fn the_cell_check_never_passes_empty() {
    let units = vec!["Name | Beispiel".to_string(), "Musterweg 14711".to_string()];
    let cells = vec![
        vec!["Name".to_string(), "Beispiel".to_string()],
        vec!["Musterweg 1".to_string()],
        vec!["4711".to_string()],
    ];
    let none = |_: &str| false;
    let found = common::cells_check(&units, &cells, &[true, true, true], &none);
    assert_eq!(found.skipped, [0, 0, 0]);
    assert!(
        found.wrong.iter().any(|w| w.contains("holds texts")),
        "{:?}",
        found.wrong
    );
    let twice = |_: &str| true;
    let found = common::cells_check(&units, &cells, &[true, true, true], &twice);
    assert_eq!(found.skipped, [2, 1, 1]);
    assert!(found.all_skipped(&cells, &[true, true, true]));
    assert!(!found.all_skipped(&cells, &[false, false, false]));
}

/// A name as a file name: a slash would make a folder.
fn file_name(name: &str) -> String {
    name.replace(['/', '\\'], "_")
}

/// Writes the Markdown and the JSON of every sampled variant of every shape to
/// `<dir>/<shape name>/<variant name>.md` and `.json`, for the dir in `REZZER_DUMP_DIR`; to
/// compare the reads of two builds byte for byte. Without the variable it does nothing. The
/// time limit is one hour, so the dump does not depend on the load of the machine. With
/// `REZZER_DUMP_SHAPE` set, only the shapes whose name holds its text are written.
#[test]
#[ignore = "writes files when REZZER_DUMP_DIR is set"]
fn dump_generated_reads() {
    let Some(dir) = std::env::var_os("REZZER_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    // With `REZZER_ENGINE_DIR` set, the engine report of each variant goes there too, at the
    // path of its JSON (see `rezzer_read_pdf::engine_report`).
    let engine = std::env::var_os("REZZER_ENGINE_DIR").map(std::path::PathBuf::from);
    let options = rezzer_render::RenderOptions {
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        rezzer_version: "dump".to_string(),
        ..rezzer_render::RenderOptions::default()
    };
    let limits = rezzer_limits::Limits {
        max_time: std::time::Duration::from_secs(3600),
        ..rezzer_limits::Limits::default()
    };
    // With `REZZER_DUMP_SHAPE` set, only the shapes whose name holds it are written.
    let only = std::env::var("REZZER_DUMP_SHAPE").ok();
    let mut folders = std::collections::HashMap::new();
    for shape in all_shapes() {
        if only
            .as_ref()
            .is_some_and(|o| !shape.name.contains(o.as_str()))
        {
            continue;
        }
        let folder = dir.join(file_name(shape.name));
        if let Some(first) = folders.insert(folder.clone(), shape.name) {
            panic!(
                "the shapes {first:?} and {:?} share the folder {folder:?}",
                shape.name
            );
        }
        std::fs::create_dir_all(&folder).unwrap();
        let mut files = std::collections::HashMap::new();
        for page in axes::variants(&shape) {
            if let Some(first) = files.insert(file_name(&page.name), page.name.clone()) {
                panic!(
                    "the variants {first:?} and {:?} of {:?} share a file name",
                    page.name, shape.name
                );
            }
            // With `REZZER_DUMP_PDF` set, the file of the variant is written too: to read it
            // with another build, or to look at it.
            if std::env::var_os("REZZER_DUMP_PDF").is_some() {
                let name = file_name(&page.name);
                std::fs::write(folder.join(format!("{name}.pdf")), inv::file(&page)).unwrap();
            }
            let (out, pages) = rezzer_read_pdf::read_engine_pages(
                &common::source(&inv::file(&page)),
                &rezzer_model::ReadOptions::default(),
                &rezzer_limits::Budget::new(limits.clone()),
            );
            let markdown = rezzer_render::render(&out.document, out.completeness, &options);
            let json = serde_json::to_string_pretty(&out.document).unwrap();
            let name = file_name(&page.name);
            std::fs::write(folder.join(format!("{name}.md")), markdown).unwrap();
            std::fs::write(folder.join(format!("{name}.json")), json).unwrap();
            if let Some(engine) = &engine {
                let folder = engine.join(file_name(shape.name));
                std::fs::create_dir_all(&folder).unwrap();
                std::fs::write(
                    folder.join(format!("{name}.json")),
                    rezzer_read_pdf::engine_report(&pages),
                )
                .unwrap();
            }
        }
    }
}

/// Prints the path of one variant through the read, for a task report: the text runs of each
/// page, the segments of the engine's own segmentation (what made each, its bounds and its
/// first words), the regions with their kind and evidence, and the Markdown. The shape is the
/// one whose name is `REZZER_TRACE_SHAPE`, the variant `REZZER_TRACE_VARIANT` (default
/// `base`); or the file at `REZZER_TRACE_FILE` (with `REZZER_TRACE_PAGE`, only that page).
/// Without these variables it does nothing. Run with `--no-capture`.
#[test]
#[ignore = "prints the path of one variant when REZZER_TRACE_SHAPE is set"]
fn trace_shape() {
    // A file instead of a shape: `REZZER_TRACE_FILE`, and `REZZER_TRACE_PAGE` for one page.
    let bytes = if let Ok(file) = std::env::var("REZZER_TRACE_FILE") {
        std::fs::read(file).unwrap()
    } else {
        let Ok(name) = std::env::var("REZZER_TRACE_SHAPE") else {
            return;
        };
        let variant = std::env::var("REZZER_TRACE_VARIANT").unwrap_or_else(|_| "base".into());
        let shape = shape(&name);
        let page = axes::variants(&shape)
            .find(|p| p.name == variant)
            .unwrap_or_else(|| panic!("no variant {variant}"));
        inv::file(&page)
    };
    let only: Option<u32> = std::env::var("REZZER_TRACE_PAGE")
        .ok()
        .and_then(|p| p.parse().ok());
    let (out, pages) = common::read_engine_pages(&bytes);
    let words = |t: &str| t.chars().take(40).collect::<String>();
    for p in &pages {
        if only.is_some_and(|o| o != p.input.page) {
            continue;
        }
        let texts = &p.input.texts;
        println!("PAGE {} ({} runs)", p.input.page, texts.len());
        for (i, t) in texts.iter().enumerate() {
            println!(
                "  run {i}: x {:.1}..{:.1} y {:.1} size {:.1} line {}{} {:?}",
                t.x,
                t.x + t.w,
                t.y,
                t.size,
                t.line,
                if t.bold { " bold" } else { "" },
                words(&t.text)
            );
        }
        for r in &p.input.rules {
            println!("  rule {r:?}");
        }
        for b in &p.input.boxes {
            println!("  box {b:?}");
        }
        for b in &p.input.pictures {
            println!("  picture {:?}", b.bounds);
        }
        let first = |prims: &[layoutregions::PrimId]| {
            let ids: Vec<usize> = prims
                .iter()
                .filter_map(|p| match p {
                    layoutregions::PrimId::Text(i) => Some(*i as usize),
                    _ => None,
                })
                .collect();
            let text: Vec<&str> = ids
                .iter()
                .filter_map(|&i| texts.get(i))
                .map(|t| t.text.trim())
                .filter(|t| !t.is_empty())
                .collect();
            format!("{} runs {:?}", ids.len(), words(&text.join(" | ")))
        };
        // The engine alone: without the regions the reader's own finders gave it.
        let mut alone = p.input.clone();
        if !alone.given.is_empty() {
            println!(
                " GIVEN {} regions of the reader's finders, left out below",
                alone.given.len()
            );
        }
        alone.given.clear();
        let segs = layoutregions::segment::segment_page(
            &alone,
            &layoutregions::Limits::default(),
            &|| false,
        );
        fn walk(
            segs: &layoutregions::segment::Segments,
            id: layoutregions::segment::SegId,
            depth: usize,
            first: &dyn Fn(&[layoutregions::PrimId]) -> String,
        ) {
            let s = &segs.all[id.0 as usize];
            let b = s.bounds;
            println!(
                "  {}seg {} {:?} [{:.1} {:.1} {:.1} {:.1}] {}",
                "  ".repeat(depth),
                id.0,
                s.source,
                b.x0,
                b.y0,
                b.x1,
                b.y1,
                first(&s.prims)
            );
            for c in &s.children {
                walk(segs, *c, depth + 1, first);
            }
        }
        println!(" SEGMENTS (the engine alone, without the reader's given regions)");
        walk(&segs, segs.root, 0, &first);
        println!(" REGIONS");
        for r in &p.regions.regions {
            let b = r.bounds;
            let ids: Vec<&str> = r.signals.iter().map(|s| s.id).collect();
            let alts: Vec<&str> = r.alternatives.iter().map(|a| a.kind.name()).collect();
            println!(
                "  {} {:?} [{:.1} {:.1} {:.1} {:.1}] {} | also {} | {}",
                r.kind.name(),
                r.certainty,
                b.x0,
                b.y0,
                b.x1,
                b.y1,
                ids.join(","),
                alts.join(","),
                first(&r.prims)
            );
        }
    }
    if only.is_none() {
        println!("{}", common::markdown(&out));
    }
}

/// What a variant must read as, as facts of the scorecard (`crates/devtools/scorecard`): the
/// rows of its tables, its headings, its paragraphs, the order of its texts, the lines of its
/// cells and each field's label with its value. The same fact of two pages is written once.
fn want_facts(page: &axes::Page) -> Vec<scorecard::facts::Fact> {
    use scorecard::facts::{Fact, Term};
    let mut facts: Vec<Fact> = Vec::new();
    for sheet in &page.sheets {
        let want = &sheet.want;
        for table in &want.tables {
            for row in table {
                let cells: Vec<Term> = row
                    .iter()
                    .filter(|c| !c.trim().is_empty())
                    .map(|c| Term::exact(c))
                    .collect();
                if !cells.is_empty() {
                    facts.push(Fact::row(cells));
                }
            }
        }
        for lines in &want.cell_lines {
            for (n, line) in lines.iter().enumerate() {
                if !line.trim().is_empty() {
                    facts.push(Fact::row(vec![Term::exact(line).at_line(n + 1)]));
                }
            }
        }
        for h in &want.headings {
            facts.push(Fact::heading(h, None));
        }
        for p in &want.paragraphs {
            facts.push(Fact::text(Term::contains(p)));
        }
        for chain in std::iter::once(&want.order).chain(&want.orders) {
            for pair in chain.windows(2) {
                facts.push(Fact::order(
                    Term::contains(&pair[0]),
                    Term::contains(&pair[1]),
                ));
            }
        }
        for (label, value) in &sheet.built.fields {
            if let Ok(Some(value)) = value
                && !label.trim().is_empty()
                && !value.trim().is_empty()
            {
                facts.push(Fact::pair(Term::exact(label), Term::exact(value)));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    facts.retain(|f| seen.insert(f.to_string()));
    facts
}

/// Writes what every sampled variant of every shape must read as to `<dir>/<shape name>/<variant
/// name>.facts`, for the dir in `REZZER_WANTS_DIR`: the assertion files of the generated set,
/// beside the reads of [`dump_generated_reads`], so that the scorecard checks this set as it
/// checks the others. A variant with no fact gets no file. Without the variable it does
/// nothing.
#[test]
#[ignore = "writes files when REZZER_WANTS_DIR is set"]
fn dump_generated_wants() {
    let Some(dir) = std::env::var_os("REZZER_WANTS_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for shape in all_shapes() {
        let folder = dir.join(file_name(shape.name));
        for page in axes::variants(&shape) {
            let facts = want_facts(&page);
            if facts.is_empty() {
                continue;
            }
            let lines: Vec<String> = facts.iter().map(|f| f.to_string()).collect();
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join(format!("{}.facts", file_name(&page.name))),
                format!(
                    "# what the generated variant must read as\n{}\n",
                    lines.join("\n")
                ),
            )
            .unwrap();
        }
    }
}

/// The exported facts say what the variant's own checks say: on a shape that reads right
/// today, the read holds every fact, and the facts are of more than one class.
#[test]
fn the_exported_wants_hold_on_a_right_read() {
    use scorecard::facts::Class;
    let shape = shape("boxed fields beside a bold title");
    let page = axes::page_of(&shape, axes::BASE);
    let facts = want_facts(&page);
    for class in [Class::Pair, Class::Heading] {
        assert!(facts.iter().any(|f| f.class == class), "{facts:?}");
    }
    let out = inv::read(&page);
    let view = scorecard::doc::View::of(&out.document);
    // Through the written form, as the scorecard reads them.
    let written: Vec<String> = facts.iter().map(|f| f.to_string()).collect();
    for w in scorecard::facts::parse(&written.join("\n")).unwrap() {
        assert!(scorecard::check::check(&view, &w.fact).pass, "{}", w.line);
    }
}

/// The regions of each kind over every sampled variant of every shape, against
/// `tests/variants/region-counts.tsv`: one line `kind\tcount` per kind in `Kind` order, then
/// `total` (as `region-report counts` prints one set). A moved count fails with its old and
/// new line. With `REZZER_WRITE_COUNTS` set, the file is written from the counts instead.
#[test]
#[ignore = "reads every sampled variant; run for the region counts"]
fn generated_region_counts() {
    let limits = rezzer_limits::Limits {
        max_time: std::time::Duration::from_secs(3600),
        ..rezzer_limits::Limits::default()
    };
    let mut counts: std::collections::BTreeMap<layoutregions::Kind, usize> =
        std::collections::BTreeMap::new();
    for shape in all_shapes() {
        for page in axes::variants(&shape) {
            let (_, pages) = rezzer_read_pdf::read_with_regions(
                &common::source(&inv::file(&page)),
                &rezzer_model::ReadOptions::default(),
                &rezzer_limits::Budget::new(limits.clone()),
            );
            for p in &pages {
                for (kind, n) in layoutregions::count_by_kind(&p.regions) {
                    *counts.entry(kind).or_insert(0) += n;
                }
            }
        }
    }
    let new = layoutregions::count_lines(&[counts]);
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/variants/region-counts.tsv");
    if std::env::var_os("REZZER_WRITE_COUNTS").is_some() {
        std::fs::write(&path, new.join("\n") + "\n").unwrap();
        return;
    }
    let old: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let moved = layoutregions::moved_lines(&old, &new);
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// The detector without rules names its regions by class: a shape of each class, at its first
/// variant, has a region of that class's signal and kind holding the shape's first word.
#[test]
fn borderless_classes_name_their_regions() {
    use layoutregions::Kind;
    for (name, id, kind, word) in [
        (
            "blocks side by side under their headings",
            "pdf.borderless.side_blocks",
            Kind::LabelValue,
            "Anschrift",
        ),
        (
            "titles over a row that ends in a stop",
            "pdf.borderless.labels_over_values",
            Kind::LabelValue,
            "Artikel",
        ),
        (
            "title beside a field list",
            "pdf.borderless.key_values",
            Kind::LabelValue,
            "Rechnungsnummer",
        ),
        (
            "a headerless table of records",
            "pdf.borderless.grid",
            Kind::Table,
            "Muster GmbH",
        ),
        (
            "line items with an end rule",
            "pdf.borderless.totals_under",
            Kind::Totals,
            "Nettobetrag",
        ),
    ] {
        let page = axes::variants(&shape(name)).next().unwrap();
        let (_, pages) = common::read_engine_pages(&inv::file(&page));
        let found = pages.iter().any(|p| {
            p.regions.regions.iter().any(|r| {
                r.signals[0].id == id
                    && r.kind == kind
                    && common::region_texts(p, r).iter().any(|t| t.contains(word))
            })
        });
        assert!(found, "{name}: no {id} region of {kind:?} holds {word:?}");
    }
}

/// A second table after a blank row under a table is no totals, though the region search
/// parted it from the table right over it: only the rule that ends a table parts its totals
/// from it. Every sampled variant.
#[test]
fn a_second_table_after_a_blank_row_is_no_totals() {
    for name in [
        "a second table under a plain header after a bold total",
        "a second table of one row under a plain header after a bold total",
    ] {
        for page in axes::variants(&shape(name)) {
            let (_, pages) = common::read_engine_pages(&inv::file(&page));
            let totals: Vec<&str> = pages
                .iter()
                .flat_map(|p| &p.regions.regions)
                .filter(|r| r.kind == layoutregions::Kind::Totals)
                .map(|r| r.signals[0].id)
                .collect();
            assert!(totals.is_empty(), "{}: {totals:?}", page.name);
        }
    }
}

/// A heading decision whose lines the walk writes as a paragraph for their geometry (bold lines
/// at heading size, each a short item of one column) is a paragraph region that says why, with
/// the heading as its alternative: the region says what is written. Every region of kind
/// heading is written as a heading.
#[test]
fn a_heading_written_as_a_paragraph_is_a_paragraph_region() {
    use layoutregions::Kind;
    let name = "a column of bold lines at heading size with a box around the middle two";
    let mut demoted = 0;
    for page in axes::variants(&shape(name)) {
        let (out, pages) = common::read_engine_pages(&inv::file(&page));
        let md = rezzer_render::render(
            &out.document,
            out.completeness,
            &rezzer_render::RenderOptions::default(),
        );
        let in_heading = |t: &str| {
            md.lines()
                .any(|l| l.starts_with('#') && l.contains(t.trim()))
        };
        for p in &pages {
            for r in &p.regions.regions {
                let texts = common::region_texts(p, r);
                if r.kind == Kind::Heading {
                    assert!(
                        texts.iter().all(|t| in_heading(t)),
                        "{name}: heading region {texts:?} not written as a heading in\n{md}"
                    );
                }
                if r.signals.iter().any(|s| s.id == "pdf.walk.demoted") {
                    demoted += 1;
                    assert_eq!(r.kind, Kind::Paragraph);
                    assert_eq!(r.alternatives[0].kind, Kind::Heading);
                    assert!(
                        texts.iter().all(|t| !in_heading(t)),
                        "{name}: {texts:?} in\n{md}"
                    );
                }
            }
        }
    }
    assert!(demoted > 0, "{name}: no heading was written as a paragraph");
}

/// Task 29 fix round 1 (review Important 6): the figure region of a picture written in a table
/// cell stands at its table: its host is the table's block, which has an anchor of its own.
#[test]
fn a_cell_picture_stands_at_its_table() {
    let page = axes::page_of(
        &shape("item table between two rules with a picture column"),
        axes::BASE,
    );
    let (_, pages) = inv::read_engine(&page);
    let p = &pages[0];
    let hosts: Vec<Option<&String>> = p
        .regions
        .regions
        .iter()
        .filter(|r| r.kind == layoutregions::Kind::Figure)
        .map(|r| p.anchors.get(r.host as usize).and_then(Option::as_ref))
        .collect();
    assert_eq!(hosts.len(), 3);
    assert!(
        hosts.iter().all(|h| h.is_some_and(|a| a == "p1:t1")),
        "{hosts:?}"
    );
}
