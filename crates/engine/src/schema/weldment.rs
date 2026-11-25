//! Contracts for the weldment commands in `cmd/weldment.rs` (docs/design/weldments.md): Trim/Extend,
//! gussets, welds, and member exports for tube lasers and beam lines (End Cap is `end_cap`).

use schemars::JsonSchema;
use serde::Deserialize;

/// The call's parameters read as `T`, with the contract's message on the key that is wrong.
pub fn typed<T: serde::de::DeserializeOwned>(cmd: &str, p: &serde_json::Value) -> crate::Result<T> {
    let v = if p.is_null() { serde_json::Value::Object(Default::default()) } else { p.clone() };
    serde_path_to_error::deserialize(&v).map_err(|e| crate::params::bad(cmd, super::check::located(&e)))
}

use super::types::{FeatureCommon, LengthExpr, feature_common};

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
