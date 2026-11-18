//! Weldment commands that work on frame members after they are made (docs/design/weldments.md):
//! `frame.trim` trims or extends member ends to a boundary;
//! `frame.gusset` adds a gusset plate between two faces; `frame.weld` records fillet welds.

use solidcraft_geom::prelude::*;

use serde_json::Value;
use solidcraft_doc::frame::{self, TrimEnd};
use solidcraft_doc::frame_corner::End;
use solidcraft_doc::{BodyName, FaceAt, FeatureKind, Position};

use super::CommandSpec;
use super::features::{add_feature, check_expr, plane_param};
use super::not_in_sketch;
use crate::params::bad;
use crate::schema::features::FeatureResult;
use crate::schema::weldment::{EndParam, GussetParams, GussetShapeParam, TrimEndParam, TrimParams, WeldParams};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

#[cfg(test)]
#[path = "weldment_tests.rs"]
mod tests;

const TRIM_CONTRACT: TypedContract<TrimParams> = TypedContract::new();
const GUSSET_CONTRACT: TypedContract<GussetParams> = TypedContract::new();
const WELD_CONTRACT: TypedContract<WeldParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("frame.trim", "Trim/Extend", crate::checked!("frame.trim", trim))
    .at("WELDMENTS", "CREATE")
    .icon("trim")
    .enabled(not_in_sketch)
    .contract(&TRIM_CONTRACT)
    .output(output_schema_for::<FeatureResult>)
    .params(
        "members: [{body: a frame member's body name, end?: start|end, near?: [x,y,z] (a point near the end; read when end is not given)}] (1…1000); and one boundary: plane: XY|XZ|YZ|plane name|{origin, normal} — or face: {body, point} (a flat face, its plane extended) — or tool: a body name (the ends are coped by it); gap?: length expr (how far short of a plane or face each end stops; default 0); extend?: bool (an end short of the boundary is lengthened to meet it first; default false). The members keep their names; the cut list shows their new ends. Returns the feature",
    ),
    CommandSpec::new("frame.gusset", "Gusset", crate::checked!("frame.gusset", gusset))
        .at("WELDMENTS", "CREATE")
        .icon("frame_member")
        .enabled(not_in_sketch)
        .contract(&GUSSET_CONTRACT)
        .output(output_schema_for::<FeatureResult>)
        .params(
            "first: {body, point} (a point on a flat face; leg d1 runs along it), second: {body, point} (another flat face, not parallel; leg d2); d1: length expr; d2?: length expr (default d1); thickness: length expr; shape?: triangle|chamfered (default triangle); chamfer?: length expr (with chamfered: each tip cut back this far along its leg); clip?: length expr (the corner cut back along each leg, clear of the weld; default 0); offset?: length expr (moved along the line where the faces meet; default 0: centred on the picks). The plate lies square to that line. One body, `Gusset`. Returns the feature and its body",
        ),
    CommandSpec::new("frame.weld", "Weld", crate::checked!("frame.weld", weld))
        .at("WELDMENTS", "CREATE")
        .icon("frame_member")
        .enabled(not_in_sketch)
        .contract(&WELD_CONTRACT)
        .output(output_schema_for::<FeatureResult>)
        .params(
            "edges: [{body, point: [x,y,z] on an edge}] (1…1000); size: length expr (the fillet's leg); both_sides?: bool (default false); stitch_length?, stitch_pitch?: length exprs (a stitch weld: stitches from each edge's start, one every pitch while a whole one fits; default continuous). Changes no body: the weld is drawn along its edges and counts in the cut list (`welds`: length and mass per size; volume size²/2 × length welded) and its total mass. Returns the feature",
        ),
];

fn trim(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.trim";
    let a: TrimParams = crate::schema::frame_library::typed(cmd, p)?;
    if a.members.is_empty() || a.members.len() > frame::MAX_MEMBERS {
        return Err(bad(cmd, format!("`members` takes 1…{} member ends", frame::MAX_MEMBERS)));
    }
    let given = [a.plane.is_some(), a.face.is_some(), a.tool.is_some()].iter().filter(|g| **g).count();
    if given != 1 {
        return Err(bad(cmd, "give one boundary: `plane`, `face` or `tool`"));
    }
    let members = member_ends(s, cmd, &a.members)?;
    let st = s.model.state();
    let face = match &a.face {
        Some(t) => {
            let tb = st.body(&t.body).ok_or_else(|| bad(cmd, format!("no body `{}`", t.body)))?;
            let k = solidcraft_doc::appearance::face_index_at(tb, t.point).ok_or_else(|| bad(cmd, "`face.point` is not on a face of the body"))?;
            if tb.body.planar_face(k).is_none() {
                return Err(bad(cmd, "the face is not flat (give it as `tool` to cope by its body)"));
            }
            Some(FaceAt { body: BodyName(t.body.clone()), point: Position(t.point) })
        }
        None => None,
    };
    if let Some(t) = &a.tool {
        if st.body(t).is_none() {
            return Err(bad(cmd, format!("no body `{t}`")));
        }
        if members.iter().any(|m| m.body.0 == *t) {
            return Err(bad(cmd, format!("`{t}` can't be trimmed by itself")));
        }
    }
    drop(st);
    let plane = match &a.plane {
        Some(v) => Some(plane_param(s, Some(&v.0), cmd)?),
        None => None,
    };
    let gap = match a.gap {
        Some(g) => {
            check_expr(s, &g.0, solidcraft_doc::expr::Kind::Length, cmd, "`gap`")?;
            Some(g.0)
        }
        None => None,
    };
    let kind = FeatureKind::FrameTrim { members, plane, face, tool: a.tool.map(BodyName), gap, extend: a.extend.unwrap_or(false) };
    add_feature(s, p, kind)
}

/// The member ends a weldment command takes: each body a frame member, its end given or nearest
/// `near`, none twice.
fn member_ends(s: &Session, cmd: &str, given: &[TrimEndParam]) -> Result<Vec<TrimEnd>> {
    if given.is_empty() || given.len() > frame::MAX_MEMBERS {
        return Err(bad(cmd, format!("`members` takes 1…{} member ends", frame::MAX_MEMBERS)));
    }
    let st = s.model.state();
    let mut members = Vec::with_capacity(given.len());
    for (k, m) in given.iter().enumerate() {
        let (start, end) = frame::member_line(&s.doc, &st, &m.body).map_err(|e| bad(cmd, format!("`members[{k}]`: {e}")))?;
        let end = match (m.end, m.near) {
            (Some(EndParam::Start), _) => End::Start,
            (Some(EndParam::End), _) => End::End,
            (None, Some(n)) => {
                let n = solidcore_record::Vec3::new(n[0], n[1], n[2]);
                if n.dist(start) <= n.dist(end) { End::Start } else { End::End }
            }
            (None, None) => return Err(bad(cmd, format!("`members[{k}]` needs `end` or `near`"))),
        };
        let te = TrimEnd { body: BodyName(m.body.clone()), end };
        if members.contains(&te) {
            return Err(bad(cmd, format!("`members[{k}]`: that end of `{}` is listed twice", m.body)));
        }
        members.push(te);
    }
    Ok(members)
}

/// A face pick as a weldment feature saves it, checked to be on a flat face.
fn flat_face(s: &Session, cmd: &str, t: &crate::schema::ToolFace, which: &str) -> Result<FaceAt> {
    let st = s.model.state();
    let tb = st.body(&t.body).ok_or_else(|| bad(cmd, format!("no body `{}`", t.body)))?;
    let k = solidcraft_doc::appearance::face_index_at(tb, t.point)
        .ok_or_else(|| bad(cmd, format!("`{which}.point` is not on a face of `{}`", t.body)))?;
    if tb.body.planar_face(k).is_none() {
        return Err(bad(cmd, format!("the `{which}` face is not flat")));
    }
    Ok(FaceAt { body: BodyName(t.body.clone()), point: Position(t.point) })
}

fn gusset(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.gusset";
    let a: GussetParams = crate::schema::frame_library::typed(cmd, p)?;
    let first = flat_face(s, cmd, &a.first, "first")?;
    let second = flat_face(s, cmd, &a.second, "second")?;
    let length = |s: &Session, e: Option<crate::schema::LengthExpr>, what: &str| -> Result<Option<solidcraft_doc::Expr>> {
        match e {
            Some(e) => {
                check_expr(s, &e.0, solidcraft_doc::expr::Kind::Length, cmd, what)?;
                Ok(Some(e.0))
            }
            None => Ok(None),
        }
    };
    let d1 = length(s, Some(a.d1), "`d1`")?.unwrap_or_default();
    let d2 = length(s, a.d2, "`d2`")?.unwrap_or_else(|| d1.clone());
    let thickness = length(s, Some(a.thickness), "`thickness`")?.unwrap_or_default();
    let shape = match a.shape {
        Some(GussetShapeParam::Chamfered) => frame::GussetShape::Chamfered,
        _ => frame::GussetShape::Triangle,
    };
    let chamfer = length(s, a.chamfer, "`chamfer`")?;
    if shape == frame::GussetShape::Chamfered && chamfer.is_none() {
        return Err(bad(cmd, "`shape: chamfered` needs `chamfer`"));
    }
    let kind = FeatureKind::FrameGusset {
        first,
        second,
        shape,
        d1,
        d2,
        thickness,
        chamfer,
        clip: length(s, a.clip, "`clip`")?,
        offset: length(s, a.offset, "`offset`")?,
    };
    add_feature(s, p, kind)
}

fn weld(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.weld";
    let a: WeldParams = crate::schema::frame_library::typed(cmd, p)?;
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
