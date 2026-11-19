// Adapted from oar-ocr-vl 0.10.3 src/models/pp_doclayout/config.rs (Apache-2.0, Copyright the
// oar-ocr authors, https://github.com/GreatV/oar-ocr). Changes: V3 only (the V2 fields, the
// reading-order config and file loading removed), parsed from the embedded config.json, errors
// as PageError. Licensed under the Apache License, Version 2.0; see LICENSE-APACHE in this
// crate.
//
// oar-ocr-vl ports this code from Hugging Face transformers (Apache-2.0,
// https://github.com/huggingface/transformers): src/transformers/models/pp_doclayout_v3
// (Copyright 2026 The PaddlePaddle Team and The HuggingFace Inc. team).

//! Configuration for PP-DocLayoutV3.
//!
//! Mirrors `config.json` of the `PaddlePaddle/PP-DocLayoutV3_safetensors`
//! checkpoint. Defaults follow
//! `transformers/models/pp_doclayout_v3/configuration_pp_doclayout_v3.py`,
//! so fields the checkpoint omits still resolve to the upstream values.

use super::err::config_err;
use crate::PageError as Error;
use serde::Deserialize;
use std::collections::HashMap;

/// Backbone selection. Only the fields that vary between the two checkpoints
/// are read; the `L` stage geometry is baked into the backbone module.
#[derive(Debug, Clone, Deserialize)]
pub struct BackboneConfig {
    /// HGNetV2 architecture preset. The checkpoint uses `L`.
    #[serde(default = "default_arch")]
    pub arch: String,
    /// Stage indices to return, e.g. `[1, 2, 3]` for stage2/3/4.
    #[serde(default = "default_return_idx")]
    pub return_idx: Vec<usize>,
}

fn default_arch() -> String {
    "L".to_string()
}

fn default_return_idx() -> Vec<usize> {
    vec![1, 2, 3]
}

fn default_global_pointer_head_size() -> usize {
    64
}

/// Top-level PP-DocLayout configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PpDocLayoutConfig {
    pub model_type: String,
    #[serde(default)]
    pub backbone_config: Option<BackboneConfig>,

    #[serde(default = "default_d_model")]
    pub d_model: usize,
    #[serde(default = "default_encoder_hidden_dim")]
    pub encoder_hidden_dim: usize,
    #[serde(default = "default_encoder_in_channels")]
    pub encoder_in_channels: Vec<usize>,
    /// V3 spells this `feature_strides`.
    #[serde(default, alias = "feature_strides")]
    pub feat_strides: Option<Vec<usize>>,
    #[serde(default = "default_encoder_layers")]
    pub encoder_layers: usize,
    #[serde(default = "default_ffn_dim")]
    pub encoder_ffn_dim: usize,
    #[serde(default = "default_attention_heads")]
    pub encoder_attention_heads: usize,
    #[serde(default = "default_encode_proj_layers")]
    pub encode_proj_layers: Vec<usize>,
    #[serde(default = "default_positional_encoding_temperature")]
    pub positional_encoding_temperature: f64,
    #[serde(default = "default_encoder_activation")]
    pub encoder_activation_function: String,
    #[serde(default = "default_activation")]
    pub activation_function: String,
    /// Pre-LN encoder layers. Only `false` (post-LN) is implemented, which is
    /// what the checkpoint ships; loading rejects `true` rather than quietly
    /// running the wrong layer order.
    #[serde(default)]
    pub normalize_before: bool,
    #[serde(default = "default_hidden_expansion")]
    pub hidden_expansion: f64,

    #[serde(default = "default_num_queries")]
    pub num_queries: usize,
    #[serde(default = "default_decoder_in_channels")]
    pub decoder_in_channels: Vec<usize>,
    #[serde(default = "default_ffn_dim")]
    pub decoder_ffn_dim: usize,
    #[serde(default = "default_num_feature_levels")]
    pub num_feature_levels: usize,
    #[serde(default = "default_decoder_n_points")]
    pub decoder_n_points: usize,
    #[serde(default = "default_decoder_layers")]
    pub decoder_layers: usize,
    #[serde(default = "default_attention_heads")]
    pub decoder_attention_heads: usize,
    #[serde(default = "default_decoder_activation")]
    pub decoder_activation_function: String,

    #[serde(default = "default_layer_norm_eps")]
    pub layer_norm_eps: f64,
    #[serde(default = "default_batch_norm_eps")]
    pub batch_norm_eps: f64,

    /// Class id to label name. The checkpoints map several ids to the same
    /// name (for example both `footer` entries), which is intentional.
    pub id2label: HashMap<String, String>,
    /// Global pointer head size for the V3 order head.
    #[serde(default = "default_global_pointer_head_size")]
    pub global_pointer_head_size: usize,
    /// Channel widths of the V3 mask feature head.
    #[serde(default = "default_mask_feature_channels")]
    pub mask_feature_channels: Vec<usize>,
    /// Channel width of the V3 stage1 (stride 4) feature map.
    #[serde(default = "default_x4_feat_dim")]
    pub x4_feat_dim: usize,
    /// Number of V3 mask prototypes.
    #[serde(default = "default_num_prototypes")]
    pub num_prototypes: usize,
    /// Whether V3 derives the decoder's initial boxes from the predicted
    /// masks. Upstream defaults to `true`.
    #[serde(default = "default_true")]
    pub mask_enhanced: bool,
}

fn default_true() -> bool {
    true
}
fn default_d_model() -> usize {
    256
}
fn default_encoder_hidden_dim() -> usize {
    256
}
fn default_encoder_in_channels() -> Vec<usize> {
    vec![512, 1024, 2048]
}
fn default_encoder_layers() -> usize {
    1
}
fn default_ffn_dim() -> usize {
    1024
}
fn default_attention_heads() -> usize {
    8
}
fn default_encode_proj_layers() -> Vec<usize> {
    vec![2]
}
fn default_positional_encoding_temperature() -> f64 {
    10000.0
}
fn default_encoder_activation() -> String {
    "gelu".to_string()
}
fn default_activation() -> String {
    "silu".to_string()
}
fn default_hidden_expansion() -> f64 {
    1.0
}
fn default_num_queries() -> usize {
    300
}
fn default_decoder_in_channels() -> Vec<usize> {
    vec![256, 256, 256]
}
fn default_num_feature_levels() -> usize {
    3
}
fn default_decoder_n_points() -> usize {
    4
}
fn default_decoder_layers() -> usize {
    6
}
fn default_decoder_activation() -> String {
    "relu".to_string()
}
fn default_layer_norm_eps() -> f64 {
    1e-5
}
fn default_batch_norm_eps() -> f64 {
    1e-5
}
fn default_mask_feature_channels() -> Vec<usize> {
    vec![64, 64]
}
fn default_x4_feat_dim() -> usize {
    128
}
fn default_num_prototypes() -> usize {
    32
}

impl PpDocLayoutConfig {
    /// Parses and checks `config.json`.
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        let cfg: Self = serde_json::from_slice(bytes)
            .map_err(|e| config_err(format!("PP-DocLayout: config.json: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Number of detection classes.
    pub fn num_labels(&self) -> usize {
        self.id2label.len()
    }

    /// Feature-map strides, defaulting to the RT-DETR `[8, 16, 32]`.
    pub fn strides(&self) -> Vec<usize> {
        self.feat_strides.clone().unwrap_or_else(|| vec![8, 16, 32])
    }

    /// Backbone stage indices to return.
    pub fn backbone_return_idx(&self) -> Vec<usize> {
        self.backbone_config
            .as_ref()
            .map(|b| b.return_idx.clone())
            .unwrap_or_else(default_return_idx)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.model_type != "pp_doclayout_v3" {
            return Err(config_err(format!(
                "PP-DocLayout: unsupported model_type '{}', expected 'pp_doclayout_v3'",
                self.model_type
            )));
        }

        if let Some(backbone) = &self.backbone_config
            && backbone.arch != "L"
        {
            return Err(config_err(format!(
                "PP-DocLayout: unsupported backbone arch '{}', only 'L' is implemented",
                backbone.arch
            )));
        }

        if self.normalize_before {
            return Err(config_err(
                "PP-DocLayout: normalize_before=true (pre-LN encoder) is not implemented"
                    .to_string(),
            ));
        }

        if self.id2label.is_empty() {
            return Err(config_err(
                "PP-DocLayout: config.json has an empty id2label map".to_string(),
            ));
        }

        if self.encoder_in_channels.len() != self.num_feature_levels {
            return Err(config_err(format!(
                "PP-DocLayout: encoder_in_channels has {} entries but num_feature_levels is {}",
                self.encoder_in_channels.len(),
                self.num_feature_levels
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// V3 spells the stride list `feature_strides` and returns four backbone
    /// stages, the first of which feeds the mask branch.
    #[test]
    fn parses_a_v3_configuration() {
        let json = r#"{
            "model_type": "pp_doclayout_v3",
            "backbone_config": {"arch": "L", "return_idx": [0, 1, 2, 3]},
            "feature_strides": [8, 16, 32],
            "id2label": {"0": "text"},
            "mask_feature_channels": [64, 64],
            "x4_feat_dim": 128
        }"#;
        let cfg = PpDocLayoutConfig::from_json(json.as_bytes()).unwrap();
        assert_eq!(cfg.strides(), vec![8, 16, 32]);
        assert_eq!(cfg.backbone_return_idx(), vec![0, 1, 2, 3]);
        assert_eq!(cfg.num_prototypes, 32);
        assert!(cfg.mask_enhanced);
    }

    #[test]
    fn parses_the_embedded_configuration() {
        let cfg = PpDocLayoutConfig::from_json(crate::detect::V3_CONFIG_JSON).unwrap();
        assert_eq!(cfg.num_labels(), 25);
        assert_eq!(cfg.num_queries, 300);
        assert_eq!(cfg.backbone_return_idx(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn rejects_other_model_types() {
        let json = r#"{"model_type": "pp_doclayout_v2", "id2label": {"0": "text"}}"#;
        assert!(PpDocLayoutConfig::from_json(json.as_bytes()).is_err());
    }

    /// Post-LN is baked into the encoder, so a pre-LN checkpoint must be
    /// refused rather than silently mis-run.
    #[test]
    fn rejects_pre_layer_norm_encoders() {
        let json = r#"{
            "model_type": "pp_doclayout_v3",
            "id2label": {"0": "text"},
            "normalize_before": true
        }"#;
        assert!(PpDocLayoutConfig::from_json(json.as_bytes()).is_err());
    }

    #[test]
    fn rejects_unsupported_backbone_architectures() {
        let json = r#"{
            "model_type": "pp_doclayout_v3",
            "backbone_config": {"arch": "X"},
            "id2label": {"0": "text"}
        }"#;
        assert!(PpDocLayoutConfig::from_json(json.as_bytes()).is_err());
    }
}
