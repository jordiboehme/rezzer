// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PP-OCRv6 small det on Candle: a small evaluator for the 14 op types of the official fused
//! ONNX graph. The weights are the graph's initializers, read by [`crate::onnx`].

use std::collections::HashMap;
use std::path::Path;

use candle_core::{D, DType, Device, Tensor};
use image::RgbImage;

use crate::PageError;
use crate::onnx::{AttrValue, Node, OnnxModel};
use crate::rules::Rect;
use crate::textdet::{db_lines, v6_input};

fn err(e: impl std::fmt::Display) -> PageError {
    PageError::Engine(format!("v6: {e}"))
}

fn attr<'a>(n: &'a Node, name: &str) -> Option<&'a AttrValue> {
    n.attributes
        .iter()
        .find(|a| a.name == name)
        .map(|a| &a.value)
}

fn attr_i(n: &Node, name: &str, default: i64) -> i64 {
    match attr(n, name) {
        Some(AttrValue::Int(v)) => *v,
        _ => default,
    }
}

fn attr_f(n: &Node, name: &str, default: f32) -> f32 {
    match attr(n, name) {
        Some(AttrValue::Float(v)) => *v,
        _ => default,
    }
}

fn attr_ints(n: &Node, name: &str) -> Option<Vec<i64>> {
    match attr(n, name) {
        Some(AttrValue::Ints(v)) => Some(v.clone()),
        _ => None,
    }
}

fn attr_bytes<'a>(n: &'a Node, name: &str) -> Option<&'a [u8]> {
    match attr(n, name) {
        Some(AttrValue::Bytes(v)) => Some(v),
        _ => None,
    }
}

/// Explicit `[top, left, bottom, right]` pads, resolving auto_pad SAME_UPPER and SAME_LOWER.
fn pads_of(n: &Node, h: usize, w: usize, k: &[i64], s: &[i64], d: &[i64]) -> Vec<i64> {
    match attr_bytes(n, "auto_pad") {
        Some(mode @ (b"SAME_UPPER" | b"SAME_LOWER")) => {
            let upper = mode == b"SAME_UPPER";
            let mut p = vec![0i64; 4];
            for (i, size) in [h as i64, w as i64].into_iter().enumerate() {
                let out = (size + s[i] - 1) / s[i];
                let total = ((out - 1) * s[i] + (k[i] - 1) * d[i] + 1 - size).max(0);
                let small = total / 2;
                let (b, e) = if upper {
                    (small, total - small)
                } else {
                    (total - small, small)
                };
                p[i] = b;
                p[i + 2] = e;
            }
            p
        }
        _ => attr_ints(n, "pads").unwrap_or(vec![0, 0, 0, 0]),
    }
}

/// The PP-OCRv6 small det text line detector (CPU).
#[derive(Debug)]
pub struct TextLineDetector {
    nodes: Vec<Node>,
    weights: HashMap<String, Tensor>,
    ints: HashMap<String, Vec<i64>>,
    floats: HashMap<String, Vec<f32>>,
    input: String,
    output: String,
    device: Device,
}

impl TextLineDetector {
    /// Loads the official ONNX file (`model::TEXT_DET_V6`).
    pub fn load(path: &Path) -> Result<Self, PageError> {
        if !path.exists() {
            return Err(PageError::ModelMissing {
                path: path.to_path_buf(),
            });
        }
        let bytes = std::fs::read(path)?;
        Self::from_model(crate::onnx::parse(&bytes)?)
    }

    /// Builds the evaluator from a parsed graph.
    pub fn from_model(model: OnnxModel) -> Result<Self, PageError> {
        let device = Device::Cpu;
        let g = model.graph;
        let mut weights = HashMap::new();
        let mut ints = HashMap::new();
        let mut floats = HashMap::new();
        for t in &g.initializers {
            let dims: Vec<usize> = t.dims.iter().map(|&d| d as usize).collect();
            if t.data_type == 1 {
                floats.insert(t.name.clone(), t.floats.clone());
                // A zero-length tensor (a Resize roi placeholder) is never an operand.
                if !t.floats.is_empty() {
                    let tensor = Tensor::from_vec(t.floats.clone(), dims, &device).map_err(err)?;
                    weights.insert(t.name.clone(), tensor);
                }
            } else {
                ints.insert(t.name.clone(), t.ints.clone());
            }
        }
        let input = g
            .inputs
            .iter()
            .find(|name| !floats.contains_key(*name) && !ints.contains_key(*name))
            .cloned()
            .ok_or_else(|| err("the graph has no input"))?;
        let output = g
            .outputs
            .first()
            .cloned()
            .ok_or_else(|| err("the graph has no output"))?;
        Ok(Self {
            nodes: g.nodes,
            weights,
            ints,
            floats,
            input,
            output,
            device,
        })
    }

    /// Runs the graph on an input tensor.
    fn run(&self, x: Tensor) -> Result<Tensor, PageError> {
        let mut env: HashMap<&str, Tensor> = HashMap::new();
        env.insert(self.input.as_str(), x);
        for n in &self.nodes {
            let y = self.eval(n, &env)?;
            let out = n
                .outputs
                .first()
                .ok_or_else(|| err(format!("{} has no output", n.op_type)))?;
            env.insert(out.as_str(), y);
        }
        env.remove(self.output.as_str())
            .ok_or_else(|| err("the output was never computed"))
    }

    fn eval(&self, n: &Node, env: &HashMap<&str, Tensor>) -> Result<Tensor, PageError> {
        let get = |i: usize| -> Result<Tensor, PageError> {
            let name = n.inputs.get(i).map(String::as_str).unwrap_or_default();
            env.get(name)
                .or_else(|| self.weights.get(name))
                .cloned()
                .ok_or_else(|| err(format!("missing input {name:?} of {}", n.op_type)))
        };
        let y = match n.op_type.as_str() {
            "Conv" => {
                let x = get(0)?;
                let w = get(1)?;
                let group = attr_i(n, "group", 1) as usize;
                let strides = attr_ints(n, "strides").unwrap_or(vec![1, 1]);
                let dil = attr_ints(n, "dilations").unwrap_or(vec![1, 1]);
                let (_, _, ih, iw) = x.dims4().map_err(err)?;
                let (_, _, kh, kw) = w.dims4().map_err(err)?;
                let pads = pads_of(n, ih, iw, &[kh as i64, kw as i64], &strides, &dil);
                if strides[0] != strides[1] || dil[0] != dil[1] {
                    return Err(err("Conv with a non-square stride or dilation"));
                }
                let (x, pad) = if pads.iter().all(|&p| p == pads[0]) {
                    (x, pads[0] as usize)
                } else {
                    let x = x
                        .pad_with_zeros(2, pads[0] as usize, pads[2] as usize)
                        .and_then(|x| x.pad_with_zeros(3, pads[1] as usize, pads[3] as usize))
                        .map_err(err)?;
                    (x, 0)
                };
                let mut y = x
                    .conv2d(&w, pad, strides[0] as usize, dil[0] as usize, group)
                    .map_err(err)?;
                if n.inputs.len() > 2 {
                    let b = get(2)?.reshape((1, (), 1, 1)).map_err(err)?;
                    y = y.broadcast_add(&b).map_err(err)?;
                }
                y
            }
            "ConvTranspose" => {
                let x = get(0)?;
                let w = get(1)?;
                if attr_i(n, "group", 1) != 1 {
                    return Err(err("grouped ConvTranspose"));
                }
                let pads = attr_ints(n, "pads").unwrap_or(vec![0, 0, 0, 0]);
                let strides = attr_ints(n, "strides").unwrap_or(vec![1, 1]);
                let opad = attr_ints(n, "output_padding").unwrap_or(vec![0, 0]);
                if pads.iter().any(|&p| p != pads[0])
                    || strides[0] != strides[1]
                    || opad[0] != opad[1]
                {
                    return Err(err("asymmetric ConvTranspose"));
                }
                let mut y = x
                    .conv_transpose2d(
                        &w,
                        pads[0] as usize,
                        opad[0] as usize,
                        strides[0] as usize,
                        1,
                    )
                    .map_err(err)?;
                if n.inputs.len() > 2 {
                    let b = get(2)?.reshape((1, (), 1, 1)).map_err(err)?;
                    y = y.broadcast_add(&b).map_err(err)?;
                }
                y
            }
            "Add" => get(0)?.broadcast_add(&get(1)?).map_err(err)?,
            "Mul" => get(0)?.broadcast_mul(&get(1)?).map_err(err)?,
            "Div" => get(0)?.broadcast_div(&get(1)?).map_err(err)?,
            "Erf" => get(0)?.erf().map_err(err)?,
            "Relu" => get(0)?.relu().map_err(err)?,
            "Sigmoid" => candle_nn::ops::sigmoid(&get(0)?).map_err(err)?,
            "HardSigmoid" => {
                let a = f64::from(attr_f(n, "alpha", 0.2));
                let b = f64::from(attr_f(n, "beta", 0.5));
                get(0)?
                    .affine(a, b)
                    .and_then(|t| t.clamp(0f32, 1f32))
                    .map_err(err)?
            }
            "GlobalAveragePool" => get(0)?
                .mean_keepdim(D::Minus1)
                .and_then(|t| t.mean_keepdim(D::Minus2))
                .map_err(err)?,
            "ReduceMean" => {
                let x = get(0)?;
                let axes = attr_ints(n, "axes").ok_or_else(|| err("ReduceMean without axes"))?;
                let keep = attr_i(n, "keepdims", 1) == 1;
                let rank = x.rank() as i64;
                let mut axes: Vec<usize> =
                    axes.iter().map(|&a| ((a + rank) % rank) as usize).collect();
                axes.sort_unstable_by(|a, b| b.cmp(a));
                let mut y = x;
                for a in axes {
                    y = if keep { y.mean_keepdim(a) } else { y.mean(a) }.map_err(err)?;
                }
                y
            }
            "Resize" => {
                let x = get(0)?;
                let (_, _, h, w) = x.dims4().map_err(err)?;
                if attr_bytes(n, "mode") != Some(b"nearest") {
                    return Err(err("Resize mode other than nearest"));
                }
                let sizes = n.inputs.get(3).filter(|s| !s.is_empty());
                let (th, tw) = if let Some(sizes) = sizes {
                    let s = self
                        .ints
                        .get(sizes)
                        .ok_or_else(|| err("Resize sizes are not constant"))?;
                    (s[2] as usize, s[3] as usize)
                } else {
                    let name = n.inputs.get(2).map(String::as_str).unwrap_or_default();
                    let s = self
                        .floats
                        .get(name)
                        .ok_or_else(|| err("Resize scales are not constant"))?;
                    (
                        (h as f32 * s[2]).floor() as usize,
                        (w as f32 * s[3]).floor() as usize,
                    )
                };
                x.upsample_nearest2d(th, tw).map_err(err)?
            }
            "MaxPool" => {
                let x = get(0)?;
                let k = attr_ints(n, "kernel_shape")
                    .ok_or_else(|| err("MaxPool without kernel_shape"))?;
                let s = attr_ints(n, "strides").unwrap_or(vec![1, 1]);
                let (b, c, h, w) = x.dims4().map_err(err)?;
                let mut p = pads_of(n, h, w, &k, &s, &[1, 1]);
                if attr_i(n, "ceil_mode", 0) == 1 {
                    // Extend bottom and right so the output size rounds up, as ONNX ceil_mode.
                    for (dim, size) in [(0usize, h), (1, w)] {
                        let span = size as i64 + p[dim] + p[dim + 2] - k[dim];
                        let rem = span % s[dim];
                        if rem != 0 {
                            p[dim + 2] += s[dim] - rem;
                        }
                    }
                }
                let x = if p.iter().any(|&v| v != 0) {
                    // Pad with a very low value, so padding never wins the max.
                    let (t, l, bo, r) =
                        (p[0] as usize, p[1] as usize, p[2] as usize, p[3] as usize);
                    Tensor::full(-1e30f32, (b, c, h + t + bo, w + l + r), x.device())
                        .and_then(|canvas| {
                            canvas.slice_assign(&[0..b, 0..c, t..t + h, l..l + w], &x)
                        })
                        .map_err(err)?
                } else {
                    x
                };
                x.max_pool2d_with_stride(
                    (k[0] as usize, k[1] as usize),
                    (s[0] as usize, s[1] as usize),
                )
                .map_err(err)?
            }
            "Concat" => {
                let axis = attr_i(n, "axis", 1);
                let xs: Vec<Tensor> = (0..n.inputs.len()).map(get).collect::<Result<_, _>>()?;
                let rank = xs.first().map_or(1, Tensor::rank) as i64;
                Tensor::cat(&xs, ((axis + rank) % rank) as usize).map_err(err)?
            }
            op => return Err(err(format!("unsupported op {op}"))),
        };
        Ok(y)
    }

    /// The probability map of a page: data (row-major), width and height (the model input
    /// size, [`crate::textdet::det_size`]).
    pub fn probability_map(&self, rgb: &RgbImage) -> Result<(Vec<f32>, usize, usize), PageError> {
        let (data, w, h) = v6_input(rgb);
        let (w, h) = (w as usize, h as usize);
        let x = Tensor::from_vec(data, (1, 3, h, w), &self.device).map_err(err)?;
        let y = self.run(x)?;
        let dims = y.dims().to_vec();
        if dims.len() != 4 || dims[2] != h || dims[3] != w {
            return Err(err(format!("unexpected output shape {dims:?}")));
        }
        let prob = y
            .to_dtype(DType::F32)
            .and_then(|t| t.flatten_all())
            .and_then(|t| t.to_vec1::<f32>())
            .map_err(err)?;
        Ok((prob, w, h))
    }

    /// The text line boxes of a page, in page pixels.
    pub fn lines(&self, rgb: &RgbImage) -> Result<Vec<Rect>, PageError> {
        let (prob, w, h) = self.probability_map(rgb)?;
        Ok(db_lines(&prob, w, h, rgb.dimensions()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onnx::{Attribute, Graph, TensorData};

    fn node(op: &str, inputs: &[&str], output: &str, attributes: Vec<Attribute>) -> Node {
        Node {
            op_type: op.into(),
            name: String::new(),
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: vec![output.into()],
            attributes,
        }
    }

    fn graph(nodes: Vec<Node>, initializers: Vec<TensorData>, output: &str) -> OnnxModel {
        OnnxModel {
            graph: Graph {
                nodes,
                initializers,
                inputs: vec!["x".into()],
                outputs: vec![output.into()],
            },
        }
    }

    fn input(values: &[f32], h: usize, w: usize) -> Tensor {
        Tensor::from_vec(values.to_vec(), (1, 1, h, w), &Device::Cpu).unwrap()
    }

    #[test]
    fn evaluates_a_small_graph() {
        let bias = TensorData {
            name: "b".into(),
            dims: vec![1],
            data_type: 1,
            floats: vec![0.5],
            ints: vec![],
        };
        let hs = vec![
            Attribute {
                name: "alpha".into(),
                value: AttrValue::Float(0.25),
            },
            Attribute {
                name: "beta".into(),
                value: AttrValue::Float(0.5),
            },
        ];
        let m = graph(
            vec![
                node("Relu", &["x"], "r", vec![]),
                node("Add", &["r", "b"], "a", vec![]),
                node("HardSigmoid", &["a"], "y", hs),
            ],
            vec![bias],
            "y",
        );
        let det = TextLineDetector::from_model(m).unwrap();
        let y = det.run(input(&[-4., 0., 1., 8.], 2, 2)).unwrap();
        let y: Vec<f32> = y.flatten_all().unwrap().to_vec1().unwrap();
        // relu, + 0.5, then clamp(0.25 x + 0.5, 0, 1).
        assert_eq!(y, [0.625, 0.625, 0.875, 1.0]);
    }

    #[test]
    fn max_pool_ceil_mode_rounds_up() {
        let attrs = vec![
            Attribute {
                name: "kernel_shape".into(),
                value: AttrValue::Ints(vec![2, 2]),
            },
            Attribute {
                name: "strides".into(),
                value: AttrValue::Ints(vec![2, 2]),
            },
            Attribute {
                name: "ceil_mode".into(),
                value: AttrValue::Int(1),
            },
        ];
        let m = graph(vec![node("MaxPool", &["x"], "y", attrs)], vec![], "y");
        let det = TextLineDetector::from_model(m).unwrap();
        let x = input(&[1., 2., 3., 4., 5., 6., 7., 8., 9.], 3, 3);
        let y = det.run(x).unwrap();
        assert_eq!(y.dims(), [1, 1, 2, 2]);
        let y: Vec<f32> = y.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(y, [5., 6., 8., 9.]);
    }

    #[test]
    fn unknown_op_is_an_error() {
        let m = graph(vec![node("Foo", &["x"], "y", vec![])], vec![], "y");
        let det = TextLineDetector::from_model(m).unwrap();
        match det.run(input(&[1.], 1, 1)) {
            Err(PageError::Engine(m)) => assert_eq!(m, "v6: unsupported op Foo"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_model_file_is_named() {
        let path = Path::new("/nonexistent/pp-ocrv6-small-det.onnx");
        match TextLineDetector::load(path) {
            Err(PageError::ModelMissing { path: p }) => assert_eq!(p, path),
            other => panic!("{other:?}"),
        }
    }
}
