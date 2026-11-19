// Adapted from oar-ocr-vl 0.10.3 src/models/pp_doclayout/model.rs (Apache-2.0, Copyright the
// oar-ocr authors, https://github.com/GreatV/oar-ocr). Changes: V3 boxes only (no V2 reading
// order, masks, polygons, layout filter, clip or element cap), every query returned as a
// LayoutBox with its rank, errors as PageError. Licensed under the Apache License, Version 2.0;
// see LICENSE-APACHE in this crate.
//
// oar-ocr-vl ports this code from Hugging Face transformers (Apache-2.0,
// https://github.com/huggingface/transformers): src/transformers/models/rt_detr (Copyright 2024
// Baidu Inc and The HuggingFace Inc. team) and src/transformers/models/pp_doclayout_v3
// (Copyright 2026 The PaddlePaddle Team and The HuggingFace Inc. team).

//! PP-DocLayoutV3 assembly: backbone, hybrid encoder, query selection,
//! decoder and the order head, plus the [`PpDocLayout`] entry point.

use super::config::PpDocLayoutConfig;
use super::decoder::{Decoder, LevelShape, MlpPredictionHead, SharedHeads, inverse_sigmoid};
use super::encoder::HybridEncoder;
use super::err::{MODEL_NAME, config_err, infer_err};
use super::hgnetv2::HGNetV2;
use super::layers::SequentialConvNorm;
use super::mask_head::mask_to_box_coordinate;
use super::order::order_sequence;
use super::processing::{PreprocessedImage, preprocess};
use crate::PageError as Error;
use crate::layoutpost::LayoutBox;
use candle_core::{DType, Device, Tensor};
use candle_nn::{LayerNorm, Linear, Module, VarBuilder, layer_norm, linear};
use image::RgbImage;
use std::path::Path;

/// The V3-only order head: a per-layer projection feeding a shared global
/// pointer.
#[derive(Debug)]
struct V3OrderHead {
    norm: LayerNorm,
    order_heads: Vec<Linear>,
    pointer: Linear,
    head_size: usize,
    /// Projects decoder queries onto the mask prototypes. With
    /// `mask_enhanced`, the resulting masks set the decoder's initial boxes.
    mask_query_head: MlpPredictionHead,
    mask_enhanced: bool,
}

impl V3OrderHead {
    fn load(cfg: &PpDocLayoutConfig, vb: VarBuilder) -> Result<Self, Error> {
        let order_vb = vb.pp("decoder_order_head");
        let mut order_heads = Vec::with_capacity(cfg.decoder_layers);
        for i in 0..cfg.decoder_layers {
            order_heads.push(
                linear(cfg.d_model, cfg.d_model, order_vb.pp(i))
                    .map_err(|e| infer_err("load v3 order head", e))?,
            );
        }
        Ok(Self {
            norm: layer_norm(cfg.d_model, cfg.layer_norm_eps, vb.pp("decoder_norm"))
                .map_err(|e| infer_err("load v3 decoder norm", e))?,
            order_heads,
            pointer: linear(
                cfg.d_model,
                cfg.global_pointer_head_size * 2,
                vb.pp("decoder_global_pointer").pp("dense"),
            )
            .map_err(|e| infer_err("load v3 global pointer", e))?,
            head_size: cfg.global_pointer_head_size,
            mask_query_head: MlpPredictionHead::load(
                cfg.d_model,
                cfg.d_model,
                cfg.num_prototypes,
                3,
                vb.pp("mask_query_head"),
            )?,
            mask_enhanced: cfg.mask_enhanced,
        })
    }

    /// The layer norm applied to decoder states before the class and order
    /// heads.
    fn norm(&self) -> &LayerNorm {
        &self.norm
    }
}

/// The detector trunk shared by both versions.
#[derive(Debug)]
struct Detector {
    backbone: HGNetV2,
    encoder_input_proj: Vec<SequentialConvNorm>,
    encoder: HybridEncoder,
    decoder_input_proj: Vec<SequentialConvNorm>,
    enc_output_dense: Linear,
    enc_output_norm: LayerNorm,
    enc_score_head: Linear,
    enc_bbox_head: MlpPredictionHead,
    decoder: Decoder,
    num_queries: usize,
}

impl Detector {
    fn load(cfg: &PpDocLayoutConfig, vb: VarBuilder) -> Result<Self, Error> {
        let vb = vb.pp("model");
        let backbone = HGNetV2::load(
            &cfg.backbone_return_idx(),
            cfg.batch_norm_eps,
            vb.pp("backbone"),
        )?;

        let encoder_proj_vb = vb.pp("encoder_input_proj");
        let mut encoder_input_proj = Vec::with_capacity(cfg.encoder_in_channels.len());
        for (i, &in_channels) in cfg.encoder_in_channels.iter().enumerate() {
            encoder_input_proj.push(SequentialConvNorm::load(
                in_channels,
                cfg.encoder_hidden_dim,
                1,
                1,
                cfg.batch_norm_eps,
                encoder_proj_vb.pp(i),
            )?);
        }

        let decoder_proj_vb = vb.pp("decoder_input_proj");
        let mut decoder_input_proj = Vec::with_capacity(cfg.decoder_in_channels.len());
        for (i, &in_channels) in cfg.decoder_in_channels.iter().enumerate() {
            decoder_input_proj.push(SequentialConvNorm::load(
                in_channels,
                cfg.d_model,
                1,
                1,
                cfg.batch_norm_eps,
                decoder_proj_vb.pp(i),
            )?);
        }

        let enc_output_vb = vb.pp("enc_output");
        Ok(Self {
            backbone,
            encoder_input_proj,
            encoder: HybridEncoder::load(cfg, true, vb.pp("encoder"))?,
            decoder_input_proj,
            enc_output_dense: linear(cfg.d_model, cfg.d_model, enc_output_vb.pp(0))
                .map_err(|e| infer_err("load enc_output dense", e))?,
            enc_output_norm: layer_norm(cfg.d_model, cfg.layer_norm_eps, enc_output_vb.pp(1))
                .map_err(|e| infer_err("load enc_output norm", e))?,
            enc_score_head: linear(cfg.d_model, cfg.num_labels(), vb.pp("enc_score_head"))
                .map_err(|e| infer_err("load enc_score_head", e))?,
            enc_bbox_head: MlpPredictionHead::load(
                cfg.d_model,
                cfg.d_model,
                4,
                3,
                vb.pp("enc_bbox_head"),
            )?,
            decoder: Decoder::load(cfg, vb.pp("decoder"))?,
            num_queries: cfg.num_queries,
        })
    }
}

/// Anchor priors in logit space, plus the validity mask that zeroes out anchors
/// too close to the image border.
fn generate_anchors(shapes: &[LevelShape], device: &Device) -> Result<(Tensor, Tensor), Error> {
    const GRID_SIZE: f32 = 0.05;
    const EPS: f32 = 1e-2;

    let total: usize = shapes.iter().map(|s| s.height * s.width).sum();
    let mut anchors = Vec::with_capacity(total * 4);
    let mut valid = Vec::with_capacity(total);
    for (level, shape) in shapes.iter().enumerate() {
        let wh = GRID_SIZE * 2f32.powi(level as i32);
        for y in 0..shape.height {
            for x in 0..shape.width {
                let cx = (x as f32 + 0.5) / shape.width as f32;
                let cy = (y as f32 + 0.5) / shape.height as f32;
                let coords = [cx, cy, wh, wh];
                let is_valid = coords.iter().all(|&c| c > EPS && c < 1.0 - EPS);
                valid.push(if is_valid { 1.0f32 } else { 0.0 });
                for &c in &coords {
                    anchors.push(if is_valid {
                        (c / (1.0 - c)).ln()
                    } else {
                        f32::MAX
                    });
                }
            }
        }
    }

    let anchors = Tensor::from_vec(anchors, (1, total, 4), device)
        .map_err(|e| infer_err("build anchors", e))?;
    let valid = Tensor::from_vec(valid, (1, total, 1), device)
        .map_err(|e| infer_err("build anchor mask", e))?;
    Ok((anchors, valid))
}

/// The PP-DocLayoutV3 layout detector (CPU, F32).
#[derive(Debug)]
pub(super) struct PpDocLayout {
    detector: Detector,
    v3_order: V3OrderHead,
    config: PpDocLayoutConfig,
    device: Device,
    dtype: DType,
}

/// Queries whose best class has a sigmoid score above this (strict) are kept as raw
/// boxes: in practice all of them. The PaddleX filter applies the real threshold.
const RAW_SCORE_THRESHOLD: f32 = 0.0;

impl PpDocLayout {
    /// Loads the safetensors checkpoint with the given configuration.
    pub(super) fn load(weights: &Path, config: PpDocLayoutConfig) -> Result<Self, Error> {
        let device = Device::Cpu;
        // The checkpoint is float32 and small enough that keeping full
        // precision costs little, and the detector is sensitive to it.
        let dtype = DType::F32;
        // SAFETY: the file is mapped read-only; it must not change while it is mapped,
        // which holds for a model file in the model folder.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights], dtype, &device)
                .map_err(|e| infer_err("map safetensors", e))?
        };
        let detector = Detector::load(&config, vb.clone())?;
        let v3_order = V3OrderHead::load(&config, vb.pp("model"))?;
        Ok(Self {
            detector,
            v3_order,
            config,
            device,
            dtype,
        })
    }

    /// Every query with its best class, score and box in page pixels, in reading order.
    pub(super) fn raw(&self, image: &RgbImage) -> Result<Vec<LayoutBox>, Error> {
        let prepared = preprocess(image, &self.device, self.dtype)?;
        let output = self.forward_detector(&prepared)?;
        let mut boxes =
            self.postprocess(&output.logits, &output.boxes, &output.hidden, &prepared)?;
        boxes.sort_by_key(|b| b.rank);
        Ok(boxes)
    }

    /// Runs backbone, encoder and decoder, returning last-layer logits, boxes
    /// (cxcywh, normalized) and decoder hidden states. The final instance
    /// masks are not computed: only boxes are used.
    fn forward_detector(&self, prepared: &PreprocessedImage) -> Result<DetectorOutput, Error> {
        let detector = &self.detector;
        let features = detector.backbone.forward(&prepared.pixel_values)?;

        // V3's backbone also returns the stride-4 map, which feeds the mask
        // branch rather than the encoder.
        let skip = features
            .len()
            .saturating_sub(detector.encoder_input_proj.len());
        let x4_feature = (skip > 0).then(|| features[0].clone());
        let projected: Result<Vec<Tensor>, Error> = features[skip..]
            .iter()
            .zip(detector.encoder_input_proj.iter())
            .map(|(feature, proj)| proj.forward(feature))
            .collect();
        let encoded = detector.encoder.forward(&projected?)?;
        let mask_features = match &x4_feature {
            Some(x4) => detector.encoder.mask_features(&encoded, x4)?,
            None => None,
        };

        let mut shapes = Vec::with_capacity(encoded.len());
        let mut flattened = Vec::with_capacity(encoded.len());
        for (level, source) in encoded.iter().enumerate() {
            let source = detector.decoder_input_proj[level].forward(source)?;
            let (_, _, height, width) = source
                .dims4()
                .map_err(|e| infer_err("decoder source shape", e))?;
            shapes.push(LevelShape { height, width });
            flattened.push(
                source
                    .flatten_from(2)
                    .and_then(|t| t.transpose(1, 2))
                    .and_then(|t| t.contiguous())
                    .map_err(|e| infer_err("decoder source flatten", e))?,
            );
        }
        let source =
            Tensor::cat(&flattened, 1).map_err(|e| infer_err("decoder source concat", e))?;

        // Only the encoder head sees the anchor-masked features; the decoder
        // cross-attends to the unmasked ones.
        let (anchors, valid_mask) = generate_anchors(&shapes, &self.device)?;
        let memory = source
            .broadcast_mul(&valid_mask)
            .map_err(|e| infer_err("anchor masking", e))?;
        let output_memory = detector
            .enc_output_dense
            .forward(&memory)
            .and_then(|t| detector.enc_output_norm.forward(&t))
            .map_err(|e| infer_err("encoder output head", e))?;

        let enc_class = detector
            .enc_score_head
            .forward(&output_memory)
            .map_err(|e| infer_err("encoder score head", e))?;
        let enc_coords = detector
            .enc_bbox_head
            .forward(&output_memory)?
            .broadcast_add(&anchors)
            .map_err(|e| infer_err("encoder box head", e))?;

        let topk = self.topk_indices(&enc_class, detector.num_queries)?;
        let index = Tensor::from_vec(topk, (1, detector.num_queries), &self.device)
            .map_err(|e| infer_err("topk index tensor", e))?;

        let reference_points = enc_coords
            .index_select(
                &index
                    .flatten_all()
                    .map_err(|e| infer_err("index flatten", e))?,
                1,
            )
            .map_err(|e| infer_err("gather reference points", e))?;
        let queries = output_memory
            .index_select(
                &index
                    .flatten_all()
                    .map_err(|e| infer_err("index flatten", e))?,
                1,
            )
            .map_err(|e| infer_err("gather queries", e))?;

        // PP-DocLayoutV3 replaces the encoder's initial boxes with tight boxes
        // around the prototype masks of the selected queries.
        let order = &self.v3_order;
        let reference_points = match &mask_features {
            Some(mask_features) if order.mask_enhanced => {
                let normed = order
                    .norm
                    .forward(&queries)
                    .map_err(|e| infer_err("v3 query norm", e))?;
                let mask_query = order.mask_query_head.forward(&normed)?;
                let (_, prototypes, mask_height, mask_width) = mask_features
                    .dims4()
                    .map_err(|e| infer_err("mask feature shape", e))?;
                let flat = mask_features
                    .reshape((1, prototypes, mask_height * mask_width))
                    .map_err(|e| infer_err("mask feature flatten", e))?;
                let masks = mask_query
                    .matmul(&flat)
                    .and_then(|t| t.reshape((1, detector.num_queries, mask_height, mask_width)))
                    .map_err(|e| infer_err("mask logits", e))?;
                let positive = masks
                    .gt(0f64)
                    .and_then(|t| t.to_dtype(masks.dtype()))
                    .map_err(|e| infer_err("mask threshold", e))?;
                inverse_sigmoid(&mask_to_box_coordinate(&positive)?)?
            }
            _ => reference_points,
        };

        // V3 reuses the encoder heads inside the decoder.
        let shared = SharedHeads {
            class_embed: &detector.enc_score_head,
            bbox_embed: &detector.enc_bbox_head,
            norm: order.norm(),
        };
        let output =
            detector
                .decoder
                .forward(&queries, &source, &reference_points, &shapes, shared)?;
        Ok(DetectorOutput {
            logits: output.logits,
            boxes: output.reference_points,
            hidden: output.last_hidden_state,
        })
    }

    /// Indices of the `k` highest per-position maximum class logits, in
    /// descending score order, matching `torch.topk`.
    fn topk_indices(&self, enc_class: &Tensor, k: usize) -> Result<Vec<u32>, Error> {
        let scores = enc_class
            .max(candle_core::D::Minus1)
            .map_err(|e| infer_err("encoder class max", e))?
            .flatten_all()
            .and_then(|t| t.to_vec1::<f32>())
            .map_err(|e| infer_err("encoder class to host", e))?;
        let mut order: Vec<u32> = (0..scores.len() as u32).collect();
        order.sort_by(|&a, &b| {
            scores[b as usize]
                .total_cmp(&scores[a as usize])
                .then(a.cmp(&b))
        });
        order.truncate(k);
        Ok(order)
    }

    /// Applies the raw threshold and the reading order, mapping boxes back to
    /// the original image size.
    fn postprocess(
        &self,
        logits: &Tensor,
        boxes: &Tensor,
        hidden: &Tensor,
        prepared: &PreprocessedImage,
    ) -> Result<Vec<LayoutBox>, Error> {
        let logit_values = logits
            .flatten_all()
            .and_then(|t| t.to_vec1::<f32>())
            .map_err(|e| infer_err("logits to host", e))?;
        let box_values = boxes
            .flatten_all()
            .and_then(|t| t.to_vec1::<f32>())
            .map_err(|e| infer_err("boxes to host", e))?;
        let num_queries = logits.dim(1).map_err(|e| infer_err("logits shape", e))?;
        let num_labels = self.config.num_labels();

        // V3 reads the order head straight off the decoder states, which stay
        // in query order, so `sequence[query]` is the rank of a query.
        let order_logits = self.v3_order_logits(&self.v3_order, hidden)?;
        let sequence = order_sequence(&order_logits)?;

        let mut detections = Vec::with_capacity(num_queries);
        for query in 0..num_queries {
            // One box per query: its best class. Upstream ranks every
            // (query, class) pair instead; the two agree whenever a query has a
            // single class above the threshold.
            let row = &logit_values[query * num_labels..(query + 1) * num_labels];
            let Some((class, &logit)) = row
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
            else {
                return Err(config_err("PP-DocLayout: no class logits".to_string()));
            };
            let score = sigmoid(logit);
            if score <= RAW_SCORE_THRESHOLD {
                continue;
            }
            let corners = scaled_corners_f32(&box_values[query * 4..query * 4 + 4], 1.0);
            let (w, h) = (
                prepared.original_width as f32,
                prepared.original_height as f32,
            );
            let bbox = [
                corners[0] * w,
                corners[1] * h,
                corners[2] * w,
                corners[3] * h,
            ];
            detections.push(LayoutBox {
                class,
                score,
                bbox: bbox.map(f64::from),
                rank: sequence[query],
            });
        }
        Ok(detections)
    }

    fn v3_order_logits(&self, order: &V3OrderHead, hidden: &Tensor) -> Result<Tensor, Error> {
        let normed = order
            .norm
            .forward(hidden)
            .map_err(|e| infer_err("v3 decoder norm", e))?;
        let last = order.order_heads.last().ok_or_else(|| {
            config_err(format!("{MODEL_NAME}: PP-DocLayoutV3 has no order heads"))
        })?;
        let projected = last
            .forward(&normed)
            .map_err(|e| infer_err("v3 order projection", e))?;

        let (batch, seq, _) = projected
            .dims3()
            .map_err(|e| infer_err("v3 order shape", e))?;
        let pair = order
            .pointer
            .forward(&projected)
            .and_then(|t| t.reshape((batch, seq, 2, order.head_size)))
            .map_err(|e| infer_err("v3 global pointer", e))?;
        let queries = pair
            .narrow(2, 0, 1)
            .and_then(|t| t.squeeze(2))
            .and_then(|t| t.contiguous())
            .map_err(|e| infer_err("v3 pointer queries", e))?;
        let keys = pair
            .narrow(2, 1, 1)
            .and_then(|t| t.squeeze(2))
            .and_then(|t| t.contiguous())
            .map_err(|e| infer_err("v3 pointer keys", e))?;
        let logits = queries
            .matmul(
                &keys
                    .transpose(1, 2)
                    .and_then(|t| t.contiguous())
                    .map_err(|e| infer_err("v3 pointer transpose", e))?,
            )
            .map_err(|e| infer_err("v3 pointer logits", e))?;
        let logits = (logits / (order.head_size as f64).sqrt())
            .map_err(|e| infer_err("v3 pointer scale", e))?;

        let keep: Vec<u8> = (0..seq)
            .flat_map(|i| (0..seq).map(move |j| u8::from(i < j)))
            .collect();
        let keep = Tensor::from_vec(keep, (1, seq, seq), logits.device())
            .and_then(|t| t.broadcast_as(logits.shape()))
            .and_then(|t| t.contiguous())
            .map_err(|e| infer_err("v3 pointer mask", e))?;
        let masked = Tensor::full(-1e4f32, logits.shape(), logits.device())
            .and_then(|t| t.to_dtype(logits.dtype()))
            .map_err(|e| infer_err("v3 pointer fill", e))?;
        keep.where_cond(&logits, &masked)
            .map_err(|e| infer_err("v3 pointer masking", e))
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn scaled_corners_f32(box_cxcywh: &[f32], scale: f32) -> [f32; 4] {
    let (cx, cy, w, h) = (box_cxcywh[0], box_cxcywh[1], box_cxcywh[2], box_cxcywh[3]);
    [
        (cx - 0.5 * w) * scale,
        (cy - 0.5 * h) * scale,
        (cx + 0.5 * w) * scale,
        (cy + 0.5 * h) * scale,
    ]
}

struct DetectorOutput {
    logits: Tensor,
    boxes: Tensor,
    hidden: Tensor,
}
