//! Weldment commands that work on frame members after they are made (docs/design/weldments.md):
//! `frame.trim` trims or extends member ends to a boundary;
//! `frame.gusset` adds a gusset plate between two faces; `frame.weld` records fillet welds;
//! `frame.export_member_step`, `frame.export_member_dxf` and `frame.export_member_nc1` write one
//! member for a tube laser or a beam line.

use solidcraft_geom::prelude::*;

use serde_json::Value;
use solidcraft_doc::frame;
use solidcraft_doc::{BodyName, FeatureKind, Position};
use solidcraft_io::{DxfFile, DxfLinetype, DxfShape};

use super::CommandSpec;
use super::features::{add_feature, check_expr};
use super::not_in_sketch;
use crate::params::bad;
use crate::schema::features::FeatureResult;
use crate::schema::weldment::{ExportMemberDxfParams, ExportMemberNc1Params, ExportMemberStepParams, MemberExportResult, MemberView, WeldParams};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

#[cfg(test)]
#[path = "weldment_tests.rs"]
mod tests;

const WELD_CONTRACT: TypedContract<WeldParams> = TypedContract::new();
const STEP_CONTRACT: TypedContract<ExportMemberStepParams> = TypedContract::new();
const DXF_CONTRACT: TypedContract<ExportMemberDxfParams> = TypedContract::new();
const NC1_CONTRACT: TypedContract<ExportMemberNc1Params> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("frame.weld", "Weld", crate::checked!("frame.weld", weld))
        .at("WELDMENTS", "CREATE")
        .icon("frame_member")
        .enabled(not_in_sketch)
        .contract(&WELD_CONTRACT)
        .output(output_schema_for::<FeatureResult>)
        .params(
            "edges: [{body, point: [x,y,z] on an edge}] (1…1000); size: length expr (the fillet's leg); both_sides?: bool (default false); stitch_length?, stitch_pitch?: length exprs (a stitch weld: stitches from each edge's start, one every pitch while a whole one fits; default continuous). Changes no body: the weld is drawn along its edges and counts in the cut list (`welds`: length and mass per size; volume size²/2 × length welded) and its total mass. Returns the feature",
        ),
    CommandSpec::new("frame.export_member_step", "Export Member STEP", crate::checked!("frame.export_member_step", export_member_step))
        .at("WELDMENTS", "INSPECT")
        .icon("export")
        .noundo()
        .doc_only()
        .contract(&STEP_CONTRACT)
        .output(output_schema_for::<MemberExportResult>)
        .params("body: a frame member's body name; path?: .step file (else the STEP text is returned). The member's solid alone, its line along +X from x = 0 where the member starts, the section's x and y along Y and Z: ready for a tube laser or a beam line. Returns path or text, and length (mm)"),
    CommandSpec::new("frame.export_member_dxf", "Export Member DXF", crate::checked!("frame.export_member_dxf", export_member_dxf))
        .at("WELDMENTS", "INSPECT")
        .icon("export")
        .noundo()
        .doc_only()
        .contract(&DXF_CONTRACT)
        .output(output_schema_for::<MemberExportResult>)
        .params("body: a body name (a frame member, or any body with flat faces square to each other); top: [x,y,z] on the face the Top view looks at; front: [x,y,z] on the face the Front view looks at (square to the top one); hidden?: bool (hidden edges dashed on layer HIDDEN; default false); path?: .dxf file (else the DXF text is returned). Three views, Top, Front and Bottom (looking up at the face opposite the top), drawn exactly from the body's edges as lines, arcs and ellipse arcs (no mesh) on layer VISIBLE, stacked with the length along X from x = 0 so each face's cuts line up; view names on layer LABELS. Returns path or text, length, views [{view, width, height, at}], uncertain (pieces whose visibility couldn't be decided, on layer UNCERTAIN)"),
    CommandSpec::new("frame.export_member_nc1", "Export Member NC1", crate::checked!("frame.export_member_nc1", export_member_nc1))
        .at("WELDMENTS", "INSPECT")
        .icon("export")
        .noundo()
        .doc_only()
        .contract(&NC1_CONTRACT)
        .output(output_schema_for::<MemberExportResult>)
        .params("body: a frame member's body name (made from a built-in section: bar, round bar, tube, angle, channel or I beam); path?: .nc1 file (else the NC1 text is returned); order?, drawing?, phase?: the header's text (default `-`); piece?: the piece mark (default the body name); grade?: the steel grade (default the member's material); quantity?: 1…1000000 (default 1). A DSTV NC1 file for a beam line, saw, drill or tube laser: the header (profile and its code, length, sizes, kg/m, paint area/m, each end's skew cut when it is one flat cut), each face's outline (AK; a round section's outside unrolled on v) taken exactly through the middle of its wall, round holes (BO) and other openings (IK), the piece on the standard's axes (an I beam's web flat, a channel's web down, an angle's heel at the zero point). Returns path or text, length (mm), profile_code, faces, holes, openings"),
];

fn weld(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.weld";
    let a: WeldParams = crate::schema::weldment::typed(cmd, p)?;
    if a.edges.is_empty() || a.edges.len() > frame::MAX_WELD_EDGES {
        return Err(bad(cmd, format!("`edges` takes 1…{} edges", frame::MAX_WELD_EDGES)));
    }
    {
        let st = s.model.state();
        for (k, e) in a.edges.iter().enumerate() {
            if st.body(&e.body).is_none() {
                return Err(bad(cmd, format!("`edges[{k}]`: no body `{}`", e.body)));
            }
        }
    }
    let length = |s: &Session, e: Option<crate::schema::LengthExpr>, what: &str| -> Result<Option<solidcraft_doc::Expr>> {
        match e {
            Some(e) => {
                check_expr(s, &e.0, solidcraft_doc::expr::Kind::Length, cmd, what)?;
                Ok(Some(e.0))
            }
            None => Ok(None),
        }
    };
    let size = length(s, Some(a.size), "`size`")?.unwrap_or_default();
    let (stitch_length, stitch_pitch) = (length(s, a.stitch_length, "`stitch_length`")?, length(s, a.stitch_pitch, "`stitch_pitch`")?);
    if stitch_length.is_some() != stitch_pitch.is_some() {
        return Err(bad(cmd, "a stitch weld needs both `stitch_length` and `stitch_pitch`"));
    }
    let edges = a
        .edges
        .into_iter()
        .map(|e| frame::WeldEdge { body: BodyName(e.body), point: Position(solidcore_record::Vec3::new(e.point[0], e.point[1], e.point[2])) })
        .collect();
    let kind = FeatureKind::FrameWeld { edges, size, both_sides: a.both_sides.unwrap_or(false), stitch_length, stitch_pitch };
    add_feature(s, p, kind)
}

/// `body` turned so `rows` (unit, square to each other) become X, Y and Z about `origin`.
fn turned(body: &crate::kernel::Body, rows: [solidcore_record::Vec3; 3], origin: solidcore_record::Vec3) -> Result<crate::kernel::Body> {
    let [x, y, z] = rows;
    // Column-major: a point p goes to (x·(p−origin), y·(p−origin), z·(p−origin)).
    let m = [[x.x, y.x, z.x, 0.0], [x.y, y.y, z.y, 0.0], [x.z, y.z, z.z, 0.0], [-x.dot(origin), -y.dot(origin), -z.dot(origin), 1.0]];
    Ok(crate::kernel::transform_matrix(body, m)?)
}

/// Write `text` to `path` (checked) or hand it back.
fn written(cmd: &str, path: Option<String>, text: String) -> Result<(Option<String>, Option<String>)> {
    match path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        Some(p) => {
            if p.len() > 4096 {
                return Err(bad(cmd, "`path` is too long"));
            }
            solidcraft_io::vfs::write(&p, text.as_bytes()).map_err(|e| crate::EngineError::Other(format!("{p}: {e}")))?;
            Ok((Some(p), None))
        }
        None => Ok((None, Some(text))),
    }
}

fn export_member_step(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.export_member_step";
    let a: ExportMemberStepParams = crate::schema::weldment::typed(cmd, p)?;
    let st = s.model.state();
    let body = st.body(&a.body).ok_or_else(|| bad(cmd, format!("no body `{}`", a.body)))?;
    let (start, along, x, y) = frame::member_frame(&s.doc, &st, &a.body).map_err(|e| bad(cmd, e.to_string()))?;
    let placed = turned(&body.body, [along, x, y], start)?;
    // Moved along X so the member starts at x = 0 (a mitre can reach back past its line's start).
    let bb = placed.exact_bounds()?;
    let shift = solidcore_record::Vec3::new(bb.min.x, 0.0, 0.0);
    let placed = turned(
        &placed,
        [solidcore_record::Vec3::new(1.0, 0.0, 0.0), solidcore_record::Vec3::new(0.0, 1.0, 0.0), solidcore_record::Vec3::new(0.0, 0.0, 1.0)],
        shift,
    )?;
    let text = crate::kernel::step_export(&[&placed], "SolidCraft")?;
    let (path, text) = written(cmd, a.path, text)?;
    let out = MemberExportResult { path, text, length: bb.max.x - bb.min.x, views: Vec::new(), uncertain: 0, ..MemberExportResult::default() };
    serde_json::to_value(out).map_err(|e| bad(cmd, e.to_string()))
}

fn export_member_nc1(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.export_member_nc1";
    let a: ExportMemberNc1Params = crate::schema::weldment::typed(cmd, p)?;
    let quantity = a.quantity.unwrap_or(1);
    if !(1..=1_000_000).contains(&quantity) {
        return Err(bad(cmd, "`quantity` must be 1…1000000"));
    }
    let header = frame::Nc1Header {
        order: a.order.unwrap_or_default(),
        drawing: a.drawing.unwrap_or_default(),
        phase: a.phase.unwrap_or_default(),
        piece: a.piece.unwrap_or_default(),
        grade: a.grade.unwrap_or_default(),
        quantity,
    };
    let st = s.model.state();
    let nc = frame::member_nc1(&s.doc, &st, &a.body, &header).map_err(|e| bad(cmd, e.to_string()))?;
    drop(st);
    let (path, text) = written(cmd, a.path, nc.text)?;
    let out = MemberExportResult {
        path,
        text,
        length: nc.length,
        profile_code: Some(nc.code.to_string()),
        faces: nc.faces.iter().map(|c| c.to_string()).collect(),
        holes: nc.holes,
        openings: nc.openings,
        ..MemberExportResult::default()
    };
    serde_json::to_value(out).map_err(|e| bad(cmd, e.to_string()))
}

/// The outward normal of the flat face of `body` at `point`.
fn face_normal(body: &solidcraft_doc::ModelBody, point: [f64; 3], cmd: &str, which: &str) -> Result<solidcore_record::Vec3> {
    let p = solidcore_record::Vec3::new(point[0], point[1], point[2]);
    let k = solidcraft_doc::appearance::face_index_at(body, p).ok_or_else(|| bad(cmd, format!("`{which}` is not on a face of `{}`", body.name)))?;
    let pl = body.body.planar_face(k).ok_or_else(|| bad(cmd, format!("the `{which}` face is not flat")))?;
    pl.x.cross(pl.y).normalized().ok_or_else(|| bad(cmd, format!("the `{which}` face has no direction")))
}

/// `s` moved by `v` on the sheet.
fn moved(s: &solidcore_record::Seg2, v: solidcore_record::Vec2) -> solidcore_record::Seg2 {
    match *s {
        solidcore_record::Seg2::Arc { center, radius, start, sweep } => solidcore_record::Seg2::Arc { center: center + v, radius, start, sweep },
        other => other.map_controls(|p| p + v),
    }
}

fn export_member_dxf(s: &mut Session, p: &Value) -> Result<Value> {
    use solidcraft_kernel::hlr::{Options, Placement, View, Visibility, hidden_lines, prepare};
    let cmd = "frame.export_member_dxf";
    let a: ExportMemberDxfParams = crate::schema::weldment::typed(cmd, p)?;
    let st = s.model.state();
    let body = st.body(&a.body).ok_or_else(|| bad(cmd, format!("no body `{}`", a.body)))?;
    let (top, front) = (face_normal(body, a.top, cmd, "top")?, face_normal(body, a.front, cmd, "front")?);
    if top.dot(front).abs() > 1e-6 {
        return Err(bad(cmd, "the top and front faces must be square to each other"));
    }
    let record = crate::kernel::brep_record(&body.body)?;
    let opts = Options::default();
    let def = prepare(&record, &opts).map_err(|e| bad(cmd, e.to_string()))?;
    let identity =
        [solidcore_record::Vec3::new(1.0, 0.0, 0.0), solidcore_record::Vec3::new(0.0, 1.0, 0.0), solidcore_record::Vec3::new(0.0, 0.0, 1.0)];
    let placed = Placement { body: 0, rotation: identity, translation: solidcore_record::Vec3::ZERO };
    // Every view's screen x runs along top × front, so the three line up along the member.
    let views = [("top", -top, -front), ("front", -front, top), ("bottom", top, front)];
    let hidden = a.hidden.unwrap_or(false);
    let mut drawn: Vec<(&str, Vec<(&'static str, solidcore_record::Seg2)>)> = Vec::new();
    let mut uncertain = 0;
    for (name, d, up) in views {
        let view = View::new(d, up).map_err(|e| bad(cmd, e.to_string()))?;
        let drawing = hidden_lines(std::slice::from_ref(&def), &[placed], &view, &opts, &|| false).map_err(|e| bad(cmd, e.to_string()))?;
        let mut segs = Vec::new();
        for piece in drawing.lines.iter().flat_map(|l| &l.pieces).filter(|p| !p.covered) {
            let layer = match piece.visibility {
                Visibility::Visible => "VISIBLE",
                Visibility::Hidden if hidden => "HIDDEN",
                Visibility::Hidden => continue,
                Visibility::Uncertain => {
                    uncertain += 1;
                    "UNCERTAIN"
                }
            };
            // Lines and arcs only, as lasers and beam lines read them: a circular arc comes back
            // as its exact arc, another curve as arcs within a thousandth of a millimetre.
            for g in &piece.geometry {
                use solidcraft_geom::Seg2ArcFit as _;
                match g.fit_arcs(1e-3) {
                    Some(arcs) => segs.extend(arcs.into_iter().map(|a| (layer, a))),
                    None => segs.push((layer, *g)),
                }
            }
        }
        drawn.push((name, segs));
    }
    // Each view's box on its own sheet coordinates.
    let bounds = |segs: &[(&str, solidcore_record::Seg2)]| {
        let pts = segs.iter().flat_map(|(_, g)| g.polyline(0.01));
        pts.fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(a, b, c, d), q| (a.min(q.x), b.min(q.y), c.max(q.x), d.max(q.y)))
    };
    let boxes: Vec<(f64, f64, f64, f64)> = drawn.iter().map(|(_, segs)| bounds(segs)).collect();
    if boxes.iter().any(|b| !(b.0 <= b.2 && b.1 <= b.3)) {
        return Err(bad(cmd, "a view came out empty"));
    }
    let x0 = boxes.iter().map(|b| b.0).fold(f64::MAX, f64::min);
    let length = boxes.iter().map(|b| b.2).fold(f64::MIN, f64::max) - x0;
    let tallest = boxes.iter().map(|b| b.3 - b.1).fold(0.0, f64::max);
    let gap = 20.0 + tallest * 0.5;
    // Bottom view at the foot of the sheet, the front above it, the top above that.
    let mut file = DxfFile::<DxfShape>::new().map_err(|e| bad(cmd, e.to_string()))?;
    let layer = |file: &mut DxfFile<DxfShape>, name: &str, color: i16, line: DxfLinetype| {
        file.declare_layer(name, color, line, None).map_err(|e| bad(cmd, e.to_string()))
    };
    let visible = layer(&mut file, "VISIBLE", 7, DxfLinetype::Continuous)?;
    let hidden_layer = layer(&mut file, "HIDDEN", 8, DxfLinetype::Hidden)?;
    let uncertain_layer = layer(&mut file, "UNCERTAIN", 1, DxfLinetype::Continuous)?;
    let labels = layer(&mut file, "LABELS", 3, DxfLinetype::Continuous)?;
    let mut out_views = Vec::new();
    let mut y = 0.0;
    for k in [2usize, 1, 0] {
        let (Some((name, segs)), Some(b)) = (drawn.get(k), boxes.get(k)) else { continue };
        let v = solidcore_record::Vec2::new(-x0, y - b.1);
        for (name, g) in segs {
            let on = match *name {
                "HIDDEN" => hidden_layer,
                "UNCERTAIN" => uncertain_layer,
                _ => visible,
            };
            file.push(on, shape_of(&moved(g, v)), None).map_err(|e| bad(cmd, e.to_string()))?;
        }
        let label = name.to_uppercase();
        let text = DxfShape::Text { at: solidcore_record::Vec2::new(-gap.min(60.0) - 10.0, y), height: 5.0, angle_deg: 0.0, text: label };
        file.push(labels, text, None).map_err(|e| bad(cmd, e.to_string()))?;
        out_views.push(MemberView { view: (*name).to_string(), width: b.2 - b.0, height: b.3 - b.1, at: [b.0 - x0, y] });
        y += b.3 - b.1 + gap;
    }
    out_views.reverse();
    let text = solidcraft_io::write_dxf_file(&file).map_err(|e| bad(cmd, e.to_string()))?;
    let (path, text) = written(cmd, a.path, text)?;
    let out = MemberExportResult { path, text, length, views: out_views, uncertain, ..MemberExportResult::default() };
    serde_json::to_value(out).map_err(|e| bad(cmd, e.to_string()))
}

/// A drawn segment as a DXF entity: a line, an arc (counter-clockwise, in degrees) or a whole
/// circle; any other curve (none reach here: they are fitted to arcs first) as a polyline within
/// a thousandth of a millimetre.
fn shape_of(g: &solidcore_record::Seg2) -> DxfShape {
    match *g {
        solidcore_record::Seg2::Line { a, b } => DxfShape::Line(a, b),
        solidcore_record::Seg2::Arc { center, radius, start: _, sweep } if sweep.abs() >= std::f64::consts::TAU - 1e-9 => {
            DxfShape::Circle { c: center, r: radius }
        }
        solidcore_record::Seg2::Arc { center, radius, start, sweep } => {
            let (from, to) = if sweep >= 0.0 { (start, start + sweep) } else { (start + sweep, start) };
            DxfShape::Arc { c: center, r: radius, a0: from.to_degrees(), a1: to.to_degrees() }
        }
        _ => DxfShape::Polyline { points: g.polyline(1e-3) },
    }
}
