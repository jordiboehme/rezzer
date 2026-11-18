// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A minimal reader for ONNX model files: the graph, its nodes and attributes, and the
//! initializer tensors. It reads the protobuf wire format directly, with the field numbers
//! of onnx.proto, so no protobuf crate is needed. Unknown fields are skipped.

use crate::PageError;

/// An ONNX model: only its graph.
#[derive(Debug, Clone, PartialEq)]
pub struct OnnxModel {
    pub graph: Graph,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub initializers: Vec<TensorData>,
    /// The names of the graph inputs (initializers may be listed too).
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub op_type: String,
    pub name: String,
    /// Input names in order. An omitted optional input is an empty string in its place.
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: AttrValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Int(i64),
    Float(f32),
    Ints(Vec<i64>),
    Floats(Vec<f32>),
    Bytes(Vec<u8>),
    Tensor(TensorData),
}

/// A tensor. FLOAT data is in `floats`; INT64 and INT32 data are in `ints`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TensorData {
    pub name: String,
    pub dims: Vec<i64>,
    pub data_type: i32,
    pub floats: Vec<f32>,
    pub ints: Vec<i64>,
}

/// TensorProto data types this reader decodes.
const FLOAT: i32 = 1;
const INT32: i32 = 6;
const INT64: i32 = 7;

/// AttributeProto types this reader keeps; other attribute types are skipped.
const ATTR_FLOAT: u64 = 1;
const ATTR_INT: u64 = 2;
const ATTR_STRING: u64 = 3;
const ATTR_TENSOR: u64 = 4;
const ATTR_FLOATS: u64 = 6;
const ATTR_INTS: u64 = 7;

/// Parses an ONNX model file.
pub fn parse(bytes: &[u8]) -> Result<OnnxModel, PageError> {
    let mut graph = None;
    let mut r = Reader::new(bytes, 0);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (7, LEN) => graph = Some(parse_graph(r.bytes()?)?),
            _ => r.skip(wire)?,
        }
    }
    let graph = graph.ok_or_else(|| engine("onnx: the model has no graph".into()))?;
    Ok(OnnxModel { graph })
}

fn engine(message: String) -> PageError {
    PageError::Engine(message)
}

const VARINT: u8 = 0;
const I64: u8 = 1;
const LEN: u8 = 2;
const I32: u8 = 5;

/// A slice of the file, with the absolute offset of its first byte for error messages.
#[derive(Clone, Copy)]
struct Sub<'a> {
    data: &'a [u8],
    base: usize,
}

struct Reader<'a> {
    data: &'a [u8],
    base: usize,
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], base: usize) -> Self {
        Self { data, base, pos: 0 }
    }

    fn of(sub: Sub<'a>) -> Self {
        Self::new(sub.data, sub.base)
    }

    fn truncated(&self) -> PageError {
        engine(format!(
            "onnx: truncated at byte {}",
            self.base + self.data.len()
        ))
    }

    fn varint(&mut self) -> Result<u64, PageError> {
        let start = self.base + self.pos;
        let mut value = 0u64;
        for i in 0..10 {
            let Some(&b) = self.data.get(self.pos) else {
                return Err(self.truncated());
            };
            self.pos += 1;
            value |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(engine(format!(
            "onnx: varint longer than 10 bytes at byte {start}"
        )))
    }

    /// The next field number and wire type, or `None` at the end.
    fn key(&mut self) -> Result<Option<(u64, u8)>, PageError> {
        if self.pos == self.data.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        Ok(Some((key >> 3, (key & 7) as u8)))
    }

    fn take(&mut self, n: usize) -> Result<Sub<'a>, PageError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            return Err(self.truncated());
        };
        let sub = Sub {
            data: &self.data[self.pos..end],
            base: self.base + self.pos,
        };
        self.pos = end;
        Ok(sub)
    }

    fn bytes(&mut self) -> Result<Sub<'a>, PageError> {
        let n = self.varint()?;
        let n = usize::try_from(n).map_err(|_| self.truncated())?;
        self.take(n)
    }

    fn string(&mut self) -> Result<String, PageError> {
        let sub = self.bytes()?;
        String::from_utf8(sub.data.to_vec())
            .map_err(|_| engine(format!("onnx: a name is not UTF-8 at byte {}", sub.base)))
    }

    fn fixed32(&mut self) -> Result<[u8; 4], PageError> {
        let sub = self.take(4)?;
        Ok([sub.data[0], sub.data[1], sub.data[2], sub.data[3]])
    }

    fn skip(&mut self, wire: u8) -> Result<(), PageError> {
        match wire {
            VARINT => self.varint().map(drop),
            I64 => self.take(8).map(drop),
            LEN => self.bytes().map(drop),
            I32 => self.take(4).map(drop),
            other => Err(engine(format!(
                "onnx: unsupported wire type {other} at byte {}",
                self.base + self.pos
            ))),
        }
    }

    /// A repeated varint field, packed or not, as `i64` (plain varint, two's complement).
    fn ints_into(&mut self, wire: u8, out: &mut Vec<i64>) -> Result<(), PageError> {
        match wire {
            VARINT => out.push(self.varint()? as i64),
            LEN => {
                let mut packed = Reader::of(self.bytes()?);
                while packed.pos < packed.data.len() {
                    out.push(packed.varint()? as i64);
                }
            }
            other => return self.skip(other),
        }
        Ok(())
    }

    /// A repeated float field, packed or not.
    fn floats_into(&mut self, wire: u8, out: &mut Vec<f32>) -> Result<(), PageError> {
        match wire {
            I32 => out.push(f32::from_le_bytes(self.fixed32()?)),
            LEN => {
                let mut packed = Reader::of(self.bytes()?);
                while packed.pos < packed.data.len() {
                    out.push(f32::from_le_bytes(packed.fixed32()?));
                }
            }
            other => return self.skip(other),
        }
        Ok(())
    }
}

fn parse_graph(sub: Sub<'_>) -> Result<Graph, PageError> {
    let mut graph = Graph::default();
    let mut r = Reader::of(sub);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (1, LEN) => graph.nodes.push(parse_node(r.bytes()?)?),
            (5, LEN) => graph.initializers.push(parse_tensor(r.bytes()?)?),
            (11, LEN) => graph.inputs.push(value_info_name(r.bytes()?)?),
            (12, LEN) => graph.outputs.push(value_info_name(r.bytes()?)?),
            _ => r.skip(wire)?,
        }
    }
    Ok(graph)
}

fn value_info_name(sub: Sub<'_>) -> Result<String, PageError> {
    let mut name = String::new();
    let mut r = Reader::of(sub);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (1, LEN) => name = r.string()?,
            _ => r.skip(wire)?,
        }
    }
    Ok(name)
}

fn parse_node(sub: Sub<'_>) -> Result<Node, PageError> {
    let mut node = Node::default();
    let mut r = Reader::of(sub);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (1, LEN) => node.inputs.push(r.string()?),
            (2, LEN) => node.outputs.push(r.string()?),
            (3, LEN) => node.name = r.string()?,
            (4, LEN) => node.op_type = r.string()?,
            (5, LEN) => {
                if let Some(a) = parse_attribute(r.bytes()?)? {
                    node.attributes.push(a);
                }
            }
            _ => r.skip(wire)?,
        }
    }
    Ok(node)
}

/// An attribute, typed by its `type` field (proto2 leaves out a value equal to the default,
/// so `i = 0` has no field of its own). Attribute types other than the six kept are `None`.
fn parse_attribute(sub: Sub<'_>) -> Result<Option<Attribute>, PageError> {
    let mut name = String::new();
    let mut kind = 0u64;
    let (mut f, mut i) = (0f32, 0i64);
    let mut s = Vec::new();
    let mut t = None;
    let (mut floats, mut ints) = (Vec::new(), Vec::new());
    let mut r = Reader::of(sub);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (1, LEN) => name = r.string()?,
            (2, I32) => f = f32::from_le_bytes(r.fixed32()?),
            (3, VARINT) => i = r.varint()? as i64,
            (4, LEN) => s = r.bytes()?.data.to_vec(),
            (5, LEN) => t = Some(parse_tensor(r.bytes()?)?),
            (7, _) => r.floats_into(wire, &mut floats)?,
            (8, _) => r.ints_into(wire, &mut ints)?,
            (20, VARINT) => kind = r.varint()?,
            _ => r.skip(wire)?,
        }
    }
    let value = match kind {
        ATTR_FLOAT => AttrValue::Float(f),
        ATTR_INT => AttrValue::Int(i),
        ATTR_STRING => AttrValue::Bytes(s),
        ATTR_TENSOR => AttrValue::Tensor(t.unwrap_or_default()),
        ATTR_FLOATS => AttrValue::Floats(floats),
        ATTR_INTS => AttrValue::Ints(ints),
        _ => return Ok(None),
    };
    Ok(Some(Attribute { name, value }))
}

fn parse_tensor(sub: Sub<'_>) -> Result<TensorData, PageError> {
    let mut t = TensorData::default();
    let mut raw: Option<&[u8]> = None;
    let mut int32 = Vec::new();
    let mut int64 = Vec::new();
    let mut r = Reader::of(sub);
    while let Some((field, wire)) = r.key()? {
        match (field, wire) {
            (1, _) => r.ints_into(wire, &mut t.dims)?,
            (2, VARINT) => t.data_type = r.varint()? as i32,
            (4, _) => r.floats_into(wire, &mut t.floats)?,
            (5, _) => r.ints_into(wire, &mut int32)?,
            (7, _) => r.ints_into(wire, &mut int64)?,
            (8, LEN) => t.name = r.string()?,
            (9, LEN) => raw = Some(r.bytes()?.data),
            _ => r.skip(wire)?,
        }
    }
    let raw = raw.unwrap_or_default();
    let size = match t.data_type {
        FLOAT | INT32 => 4,
        INT64 => 8,
        other => return Err(engine(format!("onnx: unsupported tensor type {other}"))),
    };
    if !raw.len().is_multiple_of(size) {
        return Err(engine(format!(
            "onnx: tensor {} has {} raw bytes, not a multiple of {size}",
            t.name,
            raw.len()
        )));
    }
    match t.data_type {
        FLOAT => {
            if !raw.is_empty() {
                let (chunks, _) = raw.as_chunks::<4>();
                t.floats = chunks.iter().map(|c| f32::from_le_bytes(*c)).collect();
            }
        }
        INT64 => {
            t.ints = if raw.is_empty() {
                int64
            } else {
                let (chunks, _) = raw.as_chunks::<8>();
                chunks.iter().map(|c| i64::from_le_bytes(*c)).collect()
            };
        }
        _ => {
            t.ints = if raw.is_empty() {
                // int32_data holds sign-extended varints.
                int32.into_iter().map(|v| i64::from(v as i32)).collect()
            } else {
                let (chunks, _) = raw.as_chunks::<4>();
                chunks
                    .iter()
                    .map(|c| i64::from(i32::from_le_bytes(*c)))
                    .collect()
            };
        }
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny protobuf encoder for the tests.
    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }
    fn key(field: u64, wire: u8) -> Vec<u8> {
        varint(field << 3 | u64::from(wire))
    }
    fn vi(field: u64, v: i64) -> Vec<u8> {
        [key(field, VARINT), varint(v as u64)].concat()
    }
    fn ld(field: u64, bytes: &[u8]) -> Vec<u8> {
        [key(field, LEN), varint(bytes.len() as u64), bytes.to_vec()].concat()
    }
    fn st(field: u64, s: &str) -> Vec<u8> {
        ld(field, s.as_bytes())
    }
    fn f32f(field: u64, v: f32) -> Vec<u8> {
        [key(field, I32), v.to_le_bytes().to_vec()].concat()
    }
    fn packed(vals: &[i64]) -> Vec<u8> {
        vals.iter().flat_map(|&v| varint(v as u64)).collect()
    }
    fn model(graph: &[u8]) -> Vec<u8> {
        [vi(1, 8), ld(7, graph)].concat()
    }

    fn sample_node() -> Vec<u8> {
        let group = [st(1, "group"), vi(20, 2), vi(3, 2)].concat();
        let pads = [st(1, "pads"), vi(20, 7), ld(8, &packed(&[1, 1, 1, 1]))].concat();
        let alpha = [st(1, "alpha"), vi(20, 1), f32f(2, 0.2)].concat();
        let mode = [st(1, "mode"), vi(20, 3), st(4, "nearest")].concat();
        // ceil_mode = 0: proto2 writes no `i` field.
        let ceil = [st(1, "ceil_mode"), vi(20, 2)].concat();
        [
            st(1, "x"),
            st(1, ""),
            st(1, "w"),
            st(2, "y"),
            st(3, "n0"),
            st(4, "Conv"),
            ld(5, &group),
            ld(5, &pads),
            ld(5, &alpha),
            ld(5, &mode),
            ld(5, &ceil),
        ]
        .concat()
    }

    #[test]
    fn reads_a_node_with_attributes() {
        let graph = [
            ld(1, &sample_node()),
            ld(11, &st(1, "x")),
            ld(12, &st(1, "y")),
        ]
        .concat();
        let m = parse(&model(&graph)).unwrap();
        assert_eq!(m.graph.inputs, ["x"]);
        assert_eq!(m.graph.outputs, ["y"]);
        let n = &m.graph.nodes[0];
        assert_eq!(n.op_type, "Conv");
        assert_eq!(n.name, "n0");
        assert_eq!(n.inputs, ["x", "", "w"]);
        assert_eq!(n.outputs, ["y"]);
        let attrs: Vec<(&str, &AttrValue)> = n
            .attributes
            .iter()
            .map(|a| (a.name.as_str(), &a.value))
            .collect();
        assert_eq!(
            attrs,
            [
                ("group", &AttrValue::Int(2)),
                ("pads", &AttrValue::Ints(vec![1, 1, 1, 1])),
                ("alpha", &AttrValue::Float(0.2)),
                ("mode", &AttrValue::Bytes(b"nearest".to_vec())),
                ("ceil_mode", &AttrValue::Int(0)),
            ]
        );
    }

    #[test]
    fn reads_raw_float_initializer() {
        let floats: Vec<u8> = [1.5f32, -2.25, 0.0, 3e-8]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let w = [
            ld(1, &packed(&[2, 2])),
            vi(2, 1),
            st(8, "w"),
            ld(9, &floats),
        ]
        .concat();
        let i64s: Vec<u8> = [-1i64, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
        let shape = [vi(1, 2), vi(2, 7), st(8, "shape"), ld(9, &i64s)].concat();
        let i32s: Vec<u8> = [-7i32, 9].iter().flat_map(|v| v.to_le_bytes()).collect();
        let small = [vi(1, 2), vi(2, 6), st(8, "small"), ld(9, &i32s)].concat();
        let m = parse(&model(&[ld(5, &w), ld(5, &shape), ld(5, &small)].concat())).unwrap();
        let t = &m.graph.initializers;
        assert_eq!(t[0].name, "w");
        assert_eq!(t[0].dims, [2, 2]);
        assert_eq!(t[0].data_type, 1);
        assert_eq!(t[0].floats, [1.5, -2.25, 0.0, 3e-8]);
        assert_eq!(t[1].ints, [-1, 3]);
        assert_eq!(t[2].ints, [-7, 9]);
    }

    #[test]
    fn reads_packed_and_unpacked_ints() {
        // dims unpacked, int64_data unpacked with a negative (a 10-byte varint).
        let a = [vi(1, 3), vi(2, 7), vi(7, -5), vi(7, 1 << 40), vi(7, 0)].concat();
        // dims packed, int64_data packed, float_data unpacked and packed.
        let b = [ld(1, &packed(&[1, 2])), vi(2, 7), ld(7, &packed(&[-1, 2]))].concat();
        let floats: Vec<u8> = [0.5f32, 0.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let c = [vi(1, 3), vi(2, 1), f32f(4, -1.0), ld(4, &floats)].concat();
        // int32_data: negatives are sign-extended varints.
        let d = [vi(1, 2), vi(2, 6), vi(5, -3), vi(5, 4)].concat();
        let ints_unpacked = [st(1, "axes"), vi(20, 7), vi(8, -1), vi(8, 2)].concat();
        let node = [st(4, "ReduceMean"), ld(5, &ints_unpacked)].concat();
        let graph = [ld(5, &a), ld(5, &b), ld(5, &c), ld(5, &d), ld(1, &node)].concat();
        let m = parse(&model(&graph)).unwrap();
        let t = &m.graph.initializers;
        assert_eq!(t[0].dims, [3]);
        assert_eq!(t[0].ints, [-5, 1 << 40, 0]);
        assert_eq!(t[1].dims, [1, 2]);
        assert_eq!(t[1].ints, [-1, 2]);
        assert_eq!(t[2].floats, [-1.0, 0.5, 0.25]);
        assert_eq!(t[3].ints, [-3, 4]);
        assert_eq!(
            m.graph.nodes[0].attributes[0].value,
            AttrValue::Ints(vec![-1, 2])
        );
    }

    #[test]
    fn skips_unknown_fields() {
        let unknown = [
            vi(99, 12345),
            [key(98, I64), vec![0; 8]].concat(),
            ld(97, b"junk"),
            [key(96, I32), vec![0; 4]].concat(),
        ]
        .concat();
        let node = [unknown.clone(), sample_node(), unknown.clone()].concat();
        let graph = [unknown.clone(), ld(1, &node), st(2, "graph name")].concat();
        let file = [unknown.clone(), model(&graph), unknown].concat();
        let m = parse(&file).unwrap();
        assert_eq!(m.graph.nodes.len(), 1);
        assert_eq!(m.graph.nodes[0].attributes.len(), 5);
    }

    #[test]
    fn truncated_input_is_an_error() {
        let w = [vi(1, 1), vi(2, 1), st(8, "w"), ld(9, &1f32.to_le_bytes())].concat();
        let file = model(&[ld(1, &sample_node()), ld(5, &w)].concat());
        assert!(parse(&file).is_ok());
        for len in 0..file.len() {
            assert!(parse(&file[..len]).is_err(), "cut at {len}");
        }
        let cut = file.len() - 1;
        match parse(&file[..cut]) {
            Err(PageError::Engine(m)) => assert_eq!(m, format!("onnx: truncated at byte {cut}")),
            other => panic!("{other:?}"),
        }
        // A varint that never ends.
        let bad = [key(7, LEN), vec![0xff; 11]].concat();
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn zero_length_initializer_is_kept() {
        let roi = [vi(1, 0), vi(2, 1), st(8, "roi")].concat();
        let m = parse(&model(&ld(5, &roi))).unwrap();
        let t = &m.graph.initializers[0];
        assert_eq!(t.name, "roi");
        assert_eq!(t.dims, [0]);
        assert!(t.floats.is_empty());
    }

    #[test]
    fn unsupported_tensor_type_is_an_error() {
        let half = [vi(1, 1), vi(2, 10), st(8, "h"), ld(9, &[0, 0])].concat();
        match parse(&model(&ld(5, &half))) {
            Err(PageError::Engine(m)) => assert_eq!(m, "onnx: unsupported tensor type 10"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    #[ignore = "needs PAGEVISION_MODEL_DIR with the PP-OCRv6 small det ONNX file"]
    fn reads_the_official_v6_graph() {
        let dir = std::env::var_os("PAGEVISION_MODEL_DIR")
            .expect("set PAGEVISION_MODEL_DIR to the folder with the model files");
        let path = std::path::Path::new(&dir).join(crate::model::TEXT_DET_V6.local);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let m = parse(&bytes).unwrap();
        assert_eq!(m.graph.nodes.len(), 242);
        let mut ops: Vec<&str> = m.graph.nodes.iter().map(|n| n.op_type.as_str()).collect();
        ops.sort_unstable();
        ops.dedup();
        assert_eq!(
            ops,
            [
                "Add",
                "Concat",
                "Conv",
                "ConvTranspose",
                "Div",
                "Erf",
                "GlobalAveragePool",
                "HardSigmoid",
                "MaxPool",
                "Mul",
                "ReduceMean",
                "Relu",
                "Resize",
                "Sigmoid",
            ]
        );
        let convs = m.graph.nodes.iter().filter(|n| n.op_type == "Conv").count();
        assert_eq!(convs, 83);
        assert!(!m.graph.initializers.is_empty());
        assert_eq!(m.graph.outputs.len(), 1);
    }
}
