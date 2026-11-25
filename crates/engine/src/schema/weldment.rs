//! Contracts for the weldment commands in `cmd/weldment.rs` (docs/design/weldments.md): Trim/Extend,
//! gussets, welds, and member exports for tube lasers and beam lines (End Cap is `end_cap`).

use schemars::JsonSchema;
use serde::Deserialize;

/// The call's parameters read as `T`, with the contract's message on the key that is wrong.
pub fn typed<T: serde::de::DeserializeOwned>(cmd: &str, p: &serde_json::Value) -> crate::Result<T> {
    let v = if p.is_null() { serde_json::Value::Object(Default::default()) } else { p.clone() };
    serde_path_to_error::deserialize(&v).map_err(|e| crate::params::bad(cmd, super::check::located(&e)))
}

use super::types::{FeatureCommon, LengthExpr, PlaneParam, ToolFace, feature_common};

/// Which end of a member's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndParam {
    Start,
    End,
}

/// One member end to trim or extend.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrimEndParam {
    /// The member's body, by name (a body a frame member feature made).
    pub body: String,
    /// Which end of the member's line. Default: the end nearer `near`.
    #[serde(default)]
    pub end: Option<EndParam>,
    /// A point near the end to work on (mm), as a pick near that end gives; read when `end` is
    /// not given, and then one of the two is required.
    #[serde(default)]
    pub near: Option<[f64; 3]>,
}

/// Trim or extend frame member ends to a boundary: a plane, a flat face of a body (each end cut on
/// it, the gap short of it), or a body (each end coped by it). With `extend`, an end short of the
/// boundary is lengthened to meet it first. The members keep their names, and the cut list shows
/// the new ends.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct TrimParams {
    /// The member ends (1…1000).
    pub members: Vec<TrimEndParam>,
    /// A plane to cut on.
    #[serde(default)]
    pub plane: Option<PlaneParam>,
    /// A flat face of a body to cut on: its plane, extended.
    #[serde(default)]
    pub face: Option<ToolFace>,
    /// A body to cope the ends by, by name.
    #[serde(default)]
    pub tool: Option<String>,
    /// How far short of the boundary each end stops (a length; default 0). Not for a body.
    #[serde(default)]
    pub gap: Option<LengthExpr>,
    /// Lengthen an end that stops short of the boundary to meet it. Default: false.
    #[serde(default)]
    pub extend: Option<bool>,
    #[serde(flatten, deserialize_with = "feature_common")]
    pub common: FeatureCommon,
}

/// A gusset's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GussetShapeParam {
    /// A triangle, legs `d1` and `d2` along the faces.
    Triangle,
    /// A triangle with each tip cut back `chamfer` along its leg, the cut parallel to the other face.
    Chamfered,
}

/// A gusset plate between two flat faces of members that meet: in the plane square to the line
/// where the faces' planes meet, its legs along the faces, centred on the picks. One body,
/// `Gusset`.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GussetParams {
    /// The first face: a point on a flat face of a body. Leg `d1` runs along it.
    pub first: ToolFace,
    /// The second face. Leg `d2` runs along it.
    pub second: ToolFace,
    /// Default: triangle.
    #[serde(default)]
    pub shape: Option<GussetShapeParam>,
    /// The leg along the first face, a length.
    pub d1: LengthExpr,
    /// The leg along the second face, a length. Default: `d1`.
    #[serde(default)]
    pub d2: Option<LengthExpr>,
    /// The plate's thickness, a length.
    pub thickness: LengthExpr,
    /// With `shape: chamfered`, how far back along its leg each tip is cut (required then).
    #[serde(default)]
    pub chamfer: Option<LengthExpr>,
    /// The corner cut back this far along each leg, clear of the weld (default 0).
    #[serde(default)]
    pub clip: Option<LengthExpr>,
    /// The plate moved this far along the line where the faces meet (default 0: centred on the
    /// picks).
    #[serde(default)]
    pub offset: Option<LengthExpr>,
    #[serde(flatten, deserialize_with = "feature_common")]
    pub common: FeatureCommon,
}

/// An edge a weld runs along: a point on an edge of a body.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeldEdgeParam {
    /// The body, by name.
    pub body: String,
    /// A point on the edge (mm).
    pub point: [f64; 3],
}

/// Fillet welds along edges of a weldment: a leg `size`, on one side of each edge or both,
/// continuous or stitched. A weld changes no body; it is drawn along its edges and counts towards
/// mass and the cut list (its volume is size² / 2 times the length welded).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct WeldParams {
    /// The edges (1…1000).
    pub edges: Vec<WeldEdgeParam>,
    /// The fillet weld's leg, a length.
    pub size: LengthExpr,
    /// Weld both sides of each edge. Default: false.
    #[serde(default)]
    pub both_sides: Option<bool>,
    /// A stitch weld's stitch length (with `stitch_pitch`). Default: continuous.
    #[serde(default)]
    pub stitch_length: Option<LengthExpr>,
    /// A stitch weld's pitch, start to start (at least `stitch_length`).
    #[serde(default)]
    pub stitch_pitch: Option<LengthExpr>,
    #[serde(flatten, deserialize_with = "feature_common")]
    pub common: FeatureCommon,
}

/// Export one frame member as a STEP file: its solid alone, its line along +X from x = 0 (where
/// the member starts), the section's x and y along Y and Z.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportMemberStepParams {
    /// The member's body, by name.
    pub body: String,
    /// The `.step` file to write; without it the STEP text is returned.
    #[serde(default)]
    pub path: Option<String>,
}

/// Export one frame member as a DSTV NC1 file for a beam line, saw, drill or tube laser: its
/// header (profile, length, grade, skew cuts), each face's outline and its holes, read exactly
/// off the member's body.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportMemberNc1Params {
    /// The member's body, by name (made from a built-in section).
    pub body: String,
    /// The `.nc1` file to write; without it the NC1 text is returned.
    #[serde(default)]
    pub path: Option<String>,
    /// The order the piece is for. Default: `-`.
    #[serde(default)]
    pub order: Option<String>,
    /// The drawing the piece is on. Default: `-`.
    #[serde(default)]
    pub drawing: Option<String>,
    /// The phase. Default: `-`.
    #[serde(default)]
    pub phase: Option<String>,
    /// The piece mark. Default: the body's name.
    #[serde(default)]
    pub piece: Option<String>,
    /// The steel grade. Default: the member's material.
    #[serde(default)]
    pub grade: Option<String>,
    /// How many to make (1…1000000). Default: 1.
    #[serde(default)]
    pub quantity: Option<u32>,
}

/// Export one member as a DXF of three views, Top, Front and Bottom, drawn exactly from its edges
/// (lines and arcs), stacked with its length along X so each face's cuts and holes line up.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportMemberDxfParams {
    /// The body, by name (a frame member, or any body with two flat faces square to each other).
    pub body: String,
    /// A point on the face the Top view looks at (a flat face).
    pub top: [f64; 3],
    /// A point on the face the Front view looks at (a flat face square to the top one).
    pub front: [f64; 3],
    /// Also draw hidden edges, dashed on layer HIDDEN. Default: false.
    #[serde(default)]
    pub hidden: Option<bool>,
    /// The `.dxf` file to write; without it the DXF text is returned.
    #[serde(default)]
    pub path: Option<String>,
}

/// One view of a member DXF.
#[derive(Clone, Debug, PartialEq, serde::Serialize, JsonSchema)]
pub struct MemberView {
    /// `top`, `front` or `bottom`.
    pub view: String,
    /// The view's size on the sheet (mm).
    pub width: f64,
    pub height: f64,
    /// Where its lower left corner is on the sheet (mm).
    pub at: [f64; 2],
}

/// A member export's result.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, JsonSchema)]
pub struct MemberExportResult {
    /// The file written, with `path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file's text, without `path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The member's length along X (mm).
    pub length: f64,
    /// The DXF's views.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<MemberView>,
    /// Pieces of edge whose visibility couldn't be decided, drawn on layer UNCERTAIN.
    #[serde(skip_serializing_if = "is_zero")]
    pub uncertain: usize,
    /// The NC1 profile code (`I`, `U`, `L`, `M`, `B`, `RO`, `RU`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_code: Option<String>,
    /// The NC1 faces given an outline (`v`, `o`, `u`, `h`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<String>,
    /// The NC1 file's round holes.
    #[serde(skip_serializing_if = "is_zero")]
    pub holes: usize,
    /// The NC1 file's other openings, as inner outlines.
    #[serde(skip_serializing_if = "is_zero")]
    pub openings: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}
