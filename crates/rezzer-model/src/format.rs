// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A file format Rezzer can detect. Serialized in snake_case, for example `three_mf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FormatId {
    Empty,
    Text,
    Markdown,
    Csv,
    Tsv,
    Html,
    Epub,
    Pdf,
    PostScript,
    Eps,
    Docx,
    Xlsx,
    Xlsm,
    Xlsb,
    Pptx,
    Odt,
    Ods,
    Odp,
    Doc,
    Xls,
    Ppt,
    Rtf,
    Eml,
    Mbox,
    Zip,
    Fcstd,
    Step,
    Iges,
    Ifc,
    Dxf,
    Dwg,
    ThreeMf,
    Stl,
    Obj,
    Gltf,
    Glb,
    Gcode,
    Bgcode,
    KicadSch,
    KicadPcb,
    Gerber,
    Excellon,
    Openscad,
    Sldprt,
    CreoPrt,
    NxPrt,
    Catpart,
    Ipt,
    F3d,
    Rhino3dm,
    Ole,
    Unknown,
}

/// How much of a file a reader could read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial,
    Failed,
}
