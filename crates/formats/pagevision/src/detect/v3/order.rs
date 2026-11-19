// Adapted from oar-ocr-vl 0.10.3 src/models/pp_doclayout/order.rs (Apache-2.0, Copyright the
// oar-ocr authors, https://github.com/GreatV/oar-ocr). Changes: errors as PageError. Licensed
// under the Apache License, Version 2.0; see LICENSE-APACHE in this crate.
//
// oar-ocr-vl ports this code from Hugging Face transformers (Apache-2.0,
// https://github.com/huggingface/transformers): src/transformers/models/pp_doclayout_v3
// (Copyright 2026 The PaddlePaddle Team and The HuggingFace Inc. team).

//! Turning pairwise order logits into a reading sequence.
//!
//! Ported from `_get_order_seqs` in
//! `transformers/models/pp_doclayout_v3/image_processing_pp_doclayout_v3.py`,
//! which both PP-DocLayout versions share.

use super::err::{config_err, infer_err};
use crate::PageError as Error;
use candle_core::Tensor;

/// Ranks queries by how many pairwise comparisons place them late in the
/// document, returning `rank[query]` for a single-image `(1, n, n)` logit
/// tensor.
pub(super) fn order_sequence(order_logits: &Tensor) -> Result<Vec<usize>, Error> {
    let (batch, rows, cols) = order_logits
        .dims3()
        .map_err(|e| infer_err("order logits shape", e))?;
    if batch != 1 || rows != cols {
        return Err(config_err(format!(
            "PP-DocLayout: expected square single-image order logits, got ({batch}, {rows}, {cols})"
        )));
    }
    let scores = order_logits
        .flatten_all()
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(|e| infer_err("order logits to host", e))?;

    let mut votes = vec![0f64; rows];
    for (j, vote) in votes.iter_mut().enumerate() {
        let mut total = 0f64;
        for i in 0..j {
            total += sigmoid(scores[i * cols + j]) as f64;
        }
        for i in (j + 1)..rows {
            total += 1.0 - sigmoid(scores[j * cols + i]) as f64;
        }
        *vote = total;
    }

    let mut pointers: Vec<usize> = (0..rows).collect();
    pointers.sort_by(|&a, &b| votes[a].total_cmp(&votes[b]).then(a.cmp(&b)));

    let mut sequence = vec![0usize; rows];
    for (rank, &query) in pointers.iter().enumerate() {
        sequence[query] = rank;
    }
    Ok(sequence)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{Device, Tensor};

    /// A strictly increasing "i before j" signal must rank queries in index
    /// order.
    #[test]
    fn ranks_a_confident_forward_chain_in_order() {
        let n = 4;
        let mut data = vec![0f32; n * n];
        for i in 0..n {
            for j in 0..n {
                data[i * n + j] = if i < j { 10.0 } else { -10.0 };
            }
        }
        let logits = Tensor::from_vec(data, (1, n, n), &Device::Cpu).unwrap();
        assert_eq!(order_sequence(&logits).unwrap(), vec![0, 1, 2, 3]);
    }

    /// Reversing the signal reverses the sequence.
    #[test]
    fn ranks_a_reversed_chain_backwards() {
        let n = 3;
        let mut data = vec![0f32; n * n];
        for i in 0..n {
            for j in 0..n {
                data[i * n + j] = if i < j { -10.0 } else { 10.0 };
            }
        }
        let logits = Tensor::from_vec(data, (1, n, n), &Device::Cpu).unwrap();
        assert_eq!(order_sequence(&logits).unwrap(), vec![2, 1, 0]);
    }
}
