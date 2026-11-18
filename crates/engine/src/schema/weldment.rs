//! Contracts for the weldment commands in `cmd/weldment.rs` (docs/design/weldments.md): Trim/Extend,
//! end caps, gussets and welds.

use schemars::JsonSchema;
use serde::Deserialize;

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
