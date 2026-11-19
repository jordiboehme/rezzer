// Adapted from oar-ocr-vl 0.10.3 src/pipeline/geometry/layout_filter.rs, filter_candidates
// with LayoutPostprocessOptions::paddlex() (Apache-2.0, Copyright the oar-ocr authors,
// https://github.com/GreatV/oar-ocr), which follows PaddleX layout_analysis and
// object_detection processors (Apache-2.0, Copyright the PaddlePaddle Authors). Changes: the
// PaddleX options fixed as constants, a strict 0.5 score threshold in front, the clip and
// the minimum size behind. Licensed under the Apache License, Version 2.0; see LICENSE-APACHE
// in this crate.

//! The PaddleX post-processing of PP-DocLayoutV3 boxes. Default code; the model runs in
//! `detect::v3`.

use crate::rules::Rect;

/// One layout box in page pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBox {
    /// PP-DocLayoutV3 class id.
    pub class: usize,
    pub score: f32,
    /// `[x0, y0, x1, y1]` in page pixels.
    pub bbox: Rect,
    /// The reading-order rank from the model's order head; the output keeps this order. It is
    /// not the query index: sorting by the query index changes the order on 7 of the 23 test
    /// pages, while this rank gives the spike's Candle order on all of them.
    pub rank: usize,
}

/// The classes that count as a drawn region: image (14), chart (3) and seal (20).
pub const DRAWN_CLASSES: [usize; 3] = [14, 3, 20];
/// A box is kept when its score is above this (strict).
pub const V3_THRESHOLD: f32 = 0.5;

/// PP-DocLayoutV3 class ids used by the filter.
const IMAGE: usize = 14;
/// NMS between boxes of the same class.
const NMS_SAME_CLASS: f64 = 0.6;
/// NMS between boxes of different classes.
const NMS_CROSS_CLASS: f64 = 0.98;
/// An image box larger than this share of a portrait page is dropped.
const PORTRAIT_IMAGE_RATIO: f64 = 0.93;
/// An image box larger than this share of a landscape page is dropped.
const LANDSCAPE_IMAGE_RATIO: f64 = 0.82;
/// A box of one of these classes removes the boxes it contains.
const CONTAINMENT_CLASSES: [usize; 5] = [3, 5, 6, 15, 17];
/// A box counts as contained when this share of its area is inside the other box.
const CONTAINMENT_RATIO: f64 = 0.9;
/// Boxes with a side shorter than this (pixels) are dropped.
const MIN_SIDE: f64 = 6.;

fn area(b: Rect) -> f64 {
    (b[2] - b[0]).max(0.) * (b[3] - b[1]).max(0.)
}

fn inter(a: Rect, b: Rect) -> f64 {
    (a[2].min(b[2]) - a[0].max(b[0])).max(0.) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.)
}

/// PaddleX IoU with the +1 pixel convention.
fn iou_plus_one(a: Rect, b: Rect) -> f64 {
    let i = (a[2].min(b[2]) - a[0].max(b[0]) + 1.).max(0.)
        * (a[3].min(b[3]) - a[1].max(b[1]) + 1.).max(0.);
    let side = |x: Rect| (x[2] - x[0] + 1.).max(0.) * (x[3] - x[1] + 1.).max(0.);
    let u = side(a) + side(b) - i;
    if u > 0. { i / u } else { 0. }
}

/// Threshold (strict 0.5), PaddleX NMS, the image size rule, containment, clip and minimum
/// size, on the raw boxes of a `width` x `height` page. The result is in rank order.
pub fn v3_filter(raw: &[LayoutBox], width: u32, height: u32) -> Vec<LayoutBox> {
    let mut boxes: Vec<LayoutBox> = raw
        .iter()
        .filter(|c| c.score > V3_THRESHOLD)
        .copied()
        .collect();
    for c in &mut boxes {
        c.bbox = c.bbox.map(f64::round_ties_even);
    }
    // NMS: highest score first, ties to the later box.
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| boxes[b].score.total_cmp(&boxes[a].score).then(b.cmp(&a)));
    let mut kept: Vec<usize> = Vec::new();
    for i in order {
        let suppressed = kept.iter().any(|&j| {
            let t = if boxes[i].class == boxes[j].class {
                NMS_SAME_CLASS
            } else {
                NMS_CROSS_CLASS
            };
            iou_plus_one(boxes[i].bbox, boxes[j].bbox) >= t
        });
        if !suppressed {
            kept.push(i);
        }
    }
    boxes = kept.into_iter().map(|i| boxes[i]).collect();
    // The image size rule; it never removes every box.
    if boxes.len() > 1 {
        let limit = if width > height {
            LANDSCAPE_IMAGE_RATIO
        } else {
            PORTRAIT_IMAGE_RATIO
        };
        let (w, h) = (f64::from(width), f64::from(height));
        let retained: Vec<LayoutBox> = boxes
            .iter()
            .filter(|c| {
                let b = c.bbox;
                c.class != IMAGE
                    || area([b[0].max(0.), b[1].max(0.), b[2].min(w), b[3].min(h)]) <= limit * w * h
            })
            .copied()
            .collect();
        if !retained.is_empty() {
            boxes = retained;
        }
    }
    // Containment: a box inside a box of one of the classes is dropped (the large one stays).
    let mut keep = vec![true; boxes.len()];
    for class in CONTAINMENT_CLASSES {
        for (i, inner) in boxes.iter().enumerate() {
            let inner_area = area(inner.bbox);
            if inner_area == 0. {
                continue;
            }
            let contained = boxes.iter().enumerate().any(|(j, outer)| {
                i != j
                    && outer.class == class
                    && inter(inner.bbox, outer.bbox) / inner_area >= CONTAINMENT_RATIO
            });
            keep[i] &= !contained;
        }
    }
    let mut boxes: Vec<LayoutBox> = boxes
        .into_iter()
        .zip(keep)
        .filter_map(|(b, k)| k.then_some(b))
        .collect();
    boxes.sort_by_key(|b| b.rank);
    let (w, h) = (f64::from(width), f64::from(height));
    boxes
        .into_iter()
        .filter_map(|mut c| {
            let b = c.bbox;
            c.bbox = [
                b[0].clamp(0., w).trunc(),
                b[1].clamp(0., h).trunc(),
                b[2].clamp(0., w).trunc(),
                b[3].clamp(0., h).trunc(),
            ];
            let big = c.bbox[2] - c.bbox[0] >= MIN_SIDE && c.bbox[3] - c.bbox[1] >= MIN_SIDE;
            big.then_some(c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(class: usize, score: f32, bbox: Rect, rank: usize) -> LayoutBox {
        LayoutBox {
            class,
            score,
            bbox,
            rank,
        }
    }

    const PAGE: (u32, u32) = (1404, 1872);

    fn filter(raw: &[LayoutBox]) -> Vec<LayoutBox> {
        v3_filter(raw, PAGE.0, PAGE.1)
    }

    #[test]
    fn filter_threshold_is_strict_half() {
        let raw = [
            b(22, 0.5, [10., 10., 100., 50.], 0),
            b(22, 0.5001, [10., 200., 100., 250.], 1),
        ];
        let out = filter(&raw);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rank, 1);
    }

    #[test]
    fn boxes_round_half_to_even_before_nms() {
        let out = filter(&[b(14, 0.9, [10.5, 11.5, 100.5, 101.5], 0)]);
        assert_eq!(out[0].bbox, [10., 12., 100., 102.]);
        // Two same-class boxes whose IoU reaches 0.6 only after rounding: 58.6 rounds to
        // 59, which gives 60 / 100 rows with the +1 convention (0.596 without rounding).
        // 58.4 rounds to 58: IoU 0.59, both stay.
        let raw = [
            b(22, 0.9, [0., 0., 99., 99.], 0),
            b(22, 0.8, [0., 0., 99., 58.6], 1),
        ];
        assert_eq!(filter(&raw).len(), 1);
        let raw = [
            b(22, 0.9, [0., 0., 99., 99.], 0),
            b(22, 0.8, [0., 0., 99., 58.4], 1),
        ];
        assert_eq!(filter(&raw).len(), 2);
    }

    #[test]
    fn nms_same_class_at_0_6() {
        // With the +1 convention, [0, 0, 99, 99] and [0, 0, 99, y1] overlap
        // (y1 + 1) / 100.
        let pair = |class: usize, y1: f64| {
            filter(&[
                b(22, 0.9, [0., 0., 99., 99.], 0),
                b(class, 0.8, [0., 0., 99., y1], 1),
            ])
            .len()
        };
        assert_eq!(pair(22, 59.), 1, "IoU 0.60 is suppressed");
        assert_eq!(pair(22, 58.), 2, "IoU 0.59 stays");
        assert_eq!(pair(21, 59.), 2, "another class needs 0.98");
    }

    #[test]
    fn cross_class_nms_at_0_98() {
        let pair = |y1: f64| {
            filter(&[
                b(22, 0.9, [0., 0., 99., 99.], 0),
                b(21, 0.8, [0., 0., 99., y1], 1),
            ])
            .len()
        };
        assert_eq!(pair(97.), 1, "IoU 0.98 is suppressed");
        assert_eq!(pair(96.), 2, "IoU 0.97 stays");
    }

    #[test]
    fn landscape_image_rule_uses_0_82() {
        // A 1872 x 1404 page: 0.82 of it is 2,155,196.16 square pixels.
        let (w, h) = (1872u32, 1404u32);
        let image = |x1: f64| b(14, 0.9, [0., 0., x1, 1404.], 0);
        let text = b(22, 0.9, [1800., 10., 1860., 50.], 1);
        // 1535 * 1404 = 2,155,140: kept. 1536 * 1404 = 2,156,544: dropped.
        assert_eq!(v3_filter(&[image(1535.), text], w, h).len(), 2);
        assert_eq!(v3_filter(&[image(1536.), text], w, h).len(), 1);
        // On a portrait page the same box is under 0.93.
        let low = b(22, 0.9, [10., 1500., 60., 1540.], 1);
        assert_eq!(v3_filter(&[image(1536.), low], 1404, 2072).len(), 2);
    }

    #[test]
    fn portrait_image_rule_uses_0_93() {
        // 0.93 of 1404 x 1872 is 2,444,307.84; 1305 * 1872 = 2,442,960 is kept,
        // 1306 * 1872 = 2,444,832 is dropped.
        let image = |x1: f64| b(14, 0.9, [0., 0., x1, 1872.], 0);
        let text = b(22, 0.9, [1340., 10., 1400., 50.], 1);
        assert_eq!(filter(&[image(1305.), text]).len(), 2);
        assert_eq!(filter(&[image(1306.), text]).len(), 1);
        // A lone image box is never removed.
        assert_eq!(filter(&[image(1404.)]).len(), 1);
    }

    #[test]
    fn containment_keeps_large_box_for_class_3() {
        // A text box 90 % inside a chart is dropped, 89 % inside stays.
        let chart = b(3, 0.9, [0., 0., 500., 500.], 0);
        let inside = |x0: f64| b(22, 0.8, [x0, 100., x0 + 100., 150.], 1);
        assert_eq!(filter(&[chart, inside(410.)]).len(), 1);
        assert_eq!(filter(&[chart, inside(411.)]).len(), 2);
        // An image (14) is no containment class: both stay.
        let image = b(14, 0.9, [0., 0., 500., 500.], 0);
        assert_eq!(filter(&[image, inside(300.)]).len(), 2);
    }

    #[test]
    fn min_side_6() {
        let out = filter(&[
            b(22, 0.9, [10., 10., 16., 100.], 0),
            b(22, 0.9, [100., 10., 105., 100.], 1),
            b(22, 0.9, [200., 10., 300., 15.4], 2),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rank, 0);
    }

    #[test]
    fn output_is_clipped_truncated_and_in_rank_order() {
        let out = filter(&[
            b(22, 0.6, [-5., 1800., 50., 1900.], 1),
            b(22, 0.9, [100., 100., 200., 200.], 0),
        ]);
        assert_eq!(out[0].rank, 0);
        assert_eq!(out[1].bbox, [0., 1800., 50., 1872.]);
    }
}
