//! Weldment features that work on frame members after they are made (docs/design/weldments.md):
//! Trim/Extend, gussets and welds (end caps are `crate::end_cap`).

use solidcraft_geom::prelude::*;
use std::collections::BTreeMap;

use solidcore_record::{Plane, Vec3};
use solidcraft_kernel as kernel;

use super::frame_member::{Prepared, feature_cuts, prepare};
use super::{ModelBody, ModelState, val};
use crate::expr::{Kind, Value};
use crate::frame::{MAX_MEMBERS, TrimEnd};
use crate::frame_corner::{self, End, MemberCuts};
use crate::lineage::Traced;
use crate::{DocError, Document, Feature, FeatureKind, Result};

/// A frame member body's member: the frame member feature that made it, worked out, and the
/// member's place among its members.
pub(crate) struct MemberOf {
    pub(crate) prepared: Prepared,
    pub(crate) cuts: Vec<MemberCuts>,
    pub(crate) index: usize,
}

/// The member that body `name` is, from the frame member feature that made it.
pub(crate) fn member_of(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, name: &str) -> Result<MemberOf> {
    let body = st.body(name).ok_or_else(|| DocError::Unknown(format!("body `{name}`")))?;
    let not_member = || DocError::Invalid(format!("`{name}` is not a frame member"));
    let f = doc.feature(body.feature).filter(|f| matches!(f.kind, FeatureKind::FrameMember { .. })).ok_or_else(not_member)?;
    let index = st.bodies.iter().filter(|b| b.feature == f.id).position(|b| b.name == name).ok_or_else(not_member)?;
    let prepared = prepare(doc, vals, f, st)?;
    let cuts = {
        let placed = prepared.placed()?;
        feature_cuts(vals, f, &prepared, &placed)?
    };
    if index >= prepared.segments.len() {
        return Err(not_member());
    }
    Ok(MemberOf { prepared, cuts, index })
}

/// What a trim cuts to.
pub(crate) enum Boundary {
    /// A plane; the member keeps the side its other end is on.
    Plane(Plane),
    /// A body the member is coped by.
    Body(kernel::Body),
}

/// The boundary of trim feature `f`.
pub(crate) fn trim_boundary(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, f: &Feature) -> Result<Boundary> {
    let FeatureKind::FrameTrim { plane, face, tool, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame trim feature".into()));
    };
    match (plane, face, tool) {
        (Some(p), None, None) => Ok(Boundary::Plane(doc.resolve_plane_in(vals, p, 0, f.component, Some(st))?)),
        (None, Some(fa), None) => {
            let b = st.body(&fa.body).ok_or_else(|| DocError::Unknown(format!("body `{}`", fa.body)))?;
            let k = crate::appearance::face_index_at(b, fa.point.0)
                .ok_or_else(|| DocError::Invalid(format!("no face of `{}` at the boundary point", fa.body)))?;
            let plane = b
                .body
                .planar_face(k)
                .ok_or_else(|| DocError::Invalid(format!("the boundary face of `{}` is not flat (use a body boundary to cope)", fa.body)))?;
            Ok(Boundary::Plane(plane))
        }
        (None, None, Some(name)) => Ok(Boundary::Body(st.body(name).ok_or_else(|| DocError::Unknown(format!("body `{name}`")))?.body.clone())),
        _ => Err(DocError::Invalid("a trim needs one boundary: a plane, a flat face or a body".into())),
    }
}

/// A trim's gap (mm, 0 when not given), refused when negative.
fn trim_gap(vals: &BTreeMap<String, Value>, gap: &Option<crate::Expr>) -> Result<f64> {
    let g = match gap {
        Some(e) => val(vals, e, Kind::Length)?,
        None => 0.0,
    };
    if !(g >= 0.0) {
        return Err(DocError::Invalid("the trim's gap can't be negative".into()));
    }
    Ok(g)
}

/// Evaluate a `FrameTrim` feature: each listed member end cut (or coped) at the boundary, first
/// lengthened to it when `extend` is on.
pub(super) fn evaluate_trim(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &mut ModelState) -> Result<()> {
    let FeatureKind::FrameTrim { members, gap, extend, tool, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame trim feature".into()));
    };
    if members.is_empty() {
        return Err(DocError::Invalid("pick the member ends to trim".into()));
    }
    if members.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("a trim takes at most {MAX_MEMBERS} member ends")));
    }
    let gap = trim_gap(vals, gap)?;
    let boundary = trim_boundary(doc, vals, st, f)?;
    for te in members {
        if tool.as_ref().is_some_and(|t| *t == te.body) {
            return Err(DocError::Invalid(format!("`{}` can't be trimmed by itself", te.body)));
        }
        let m = member_of(doc, vals, st, &te.body)?;
        let i = st.bodies.iter().position(|b| b.name == te.body.0).ok_or_else(|| DocError::Unknown(format!("body `{}`", te.body)))?;
        let Some(mb) = st.bodies.get(i).cloned() else { continue };
        let input = mb.traced();
        let body = trim_end(&m, &mb.body, te, &boundary, gap, *extend)?;
        if let Some(slot) = st.bodies.get_mut(i) {
            *slot = ModelBody::traced_new(mb.name.clone(), Traced::step(body, &[&input]), mb.feature);
        }
    }
    Ok(())
}

/// Member `m`'s line ends: the end worked on, the other end, and the outward direction there.
fn ends(m: &MemberOf, end: End, name: &str) -> Result<(Vec3, Vec3, Vec3)> {
    let l = m.prepared.segments.get(m.index).ok_or_else(|| DocError::Invalid(format!("`{name}` is not a frame member")))?;
    let (e, q) = match end {
        End::Start => (l.a, l.b),
        End::End => (l.b, l.a),
    };
    let u = (e - q).normalized().ok_or_else(|| DocError::Invalid(format!("the member `{name}` has no length")))?;
    Ok((e, q, u))
}

/// How far back from its line end (along the line) member end `end` is cut by its corners: the
/// most any section point is cut back.
fn retreat(p: &frame_corner::Placed, c: &MemberCuts, end: End, e: Vec3, u: Vec3) -> f64 {
    let inward = -u;
    let back = |cut: &frame_corner::Cut| {
        let along = inward.dot(cut.normal);
        if along > 1e-9 { ((cut.origin - e).dot(cut.normal) + p.reach(-cut.normal)) / along } else { 0.0 }
    };
    let planes = c.cuts.iter().filter(|(x, _, _)| *x == end).map(|(_, _, cut)| back(cut));
    let copes = c.copes.iter().filter(|k| k.end == end).map(|k| back(&k.near));
    planes.chain(copes).fold(0.0, f64::max)
}

/// Member body `body` with end `te.end` trimmed at `boundary`, `gap` short of it, lengthened to it
/// first when `extend` is on.
fn trim_end(m: &MemberOf, body: &kernel::Body, te: &TrimEnd, boundary: &Boundary, gap: f64, extend: bool) -> Result<kernel::Body> {
    let name = &te.body.0;
    let placed = m.prepared.placed()?;
    let p = placed.get(m.index).ok_or_else(|| DocError::Invalid(format!("`{name}` is not a frame member")))?;
    let cuts = m.cuts.get(m.index).cloned().unwrap_or_default();
    let (e, q, u) = ends(m, te.end, name)?;
    let len = e.dist(q);
    let margin = 0.01 * (p.radius() + 1.0);
    // How far past its line end the boundary needs the member (≤ 0: it is reached already).
    let ahead = match boundary {
        Boundary::Plane(pl) => {
            let n = pl.x.cross(pl.y).normalized().ok_or_else(|| DocError::Invalid("the boundary plane has no direction".into()))?;
            let along = u.dot(n);
            if along.abs() < 1e-9 {
                return Err(DocError::Invalid(format!("`{name}` runs along the boundary")));
            }
            let t = (pl.origin - e).dot(n) / along;
            if t <= -len {
                return Err(DocError::Invalid(format!("the boundary is past the other end of `{name}`")));
            }
            t + p.radius() * (1.0 - along * along).max(0.0).sqrt() / along.abs() + margin
        }
        Boundary::Body(b) => {
            let bb = b.exact_bounds()?;
            let corners = [bb.min.x, bb.max.x]
                .into_iter()
                .flat_map(|x| [bb.min.y, bb.max.y].into_iter().flat_map(move |y| [bb.min.z, bb.max.z].into_iter().map(move |z| Vec3::new(x, y, z))));
            corners.map(|c| (c - e).dot(u)).fold(f64::NEG_INFINITY, f64::max) + margin
        }
    };
    let mut body = body.clone();
    if extend && ahead > 0.0 {
        if ahead > frame_corner::MAX_EXTENSION {
            return Err(DocError::Invalid(format!("the boundary is too far from `{name}` to extend to")));
        }
        // From where the corner cut the end back (so its old end goes) to past the boundary.
        let back = (retreat(p, &cuts, te.end, e, u) + margin).min(0.9 * len);
        let plane = Plane::new(e - p.x * p.at.x - p.y * p.at.y, p.x, p.y).ok_or_else(|| DocError::Invalid(format!("no plane square to `{name}`")))?;
        // The plane's normal is p.x × p.y, along the line from start to end.
        let sign = if te.end == End::End { 1.0 } else { -1.0 };
        let (from, to) = if sign > 0.0 { (-back, ahead) } else { (-ahead, back) };
        for piece in kernel::extrude(&plane, p.regions, from, to)? {
            body = kernel::boolean(&body, &piece, kernel::BoolOp::Union)?
                .ok_or_else(|| DocError::Invalid(format!("extending `{name}` left nothing")))?;
        }
    }
    let kept = match boundary {
        Boundary::Plane(pl) => {
            let n = pl.x.cross(pl.y).normalized().ok_or_else(|| DocError::Invalid("the boundary plane has no direction".into()))?;
            // Keep the side the other end is on, the gap short of the plane.
            let keep = if (q - pl.origin).dot(n) >= 0.0 { n } else { -n };
            let cut =
                Plane::from_normal(pl.origin + keep * gap, keep).ok_or_else(|| DocError::Invalid("the boundary plane has no direction".into()))?;
            kernel::half_space(&body, &cut, true)?.0
        }
        Boundary::Body(b) => {
            if gap > 0.0 {
                return Err(DocError::Invalid("a cope by a body has no gap yet (set the gap to 0)".into()));
            }
            kernel::boolean(&body, b, kernel::BoolOp::Cut)?
        }
    };
    let kept = kept.ok_or_else(|| DocError::Invalid(format!("the boundary cuts away the whole of `{name}`")))?;
    // A cope can leave pieces past the boundary: keep the one the member's other end is in.
    let mut pieces = kept.lumps()?;
    if pieces.len() > 1 {
        let inside = |b: &kernel::Body| {
            b.exact_bounds()
                .map(|bb| {
                    bb.min.x <= q.x + margin
                        && q.x - margin <= bb.max.x
                        && bb.min.y <= q.y + margin
                        && q.y - margin <= bb.max.y
                        && bb.min.z <= q.z + margin
                        && q.z - margin <= bb.max.z
                })
                .unwrap_or(false)
        };
        let k = pieces.iter().position(inside).ok_or_else(|| DocError::Invalid(format!("the boundary cuts `{name}` in pieces")))?;
        return Ok(pieces.swap_remove(k));
    }
    pieces.pop().ok_or_else(|| DocError::Invalid(format!("the boundary cuts away the whole of `{name}`")))
}

/// A trim made to a member end later in the timeline, as the cut list takes it.
pub(crate) struct LaterTrim {
    pub(crate) body: String,
    pub(crate) end: End,
    /// The end's cut plane (the member keeps the side its normal points to); none: coped by a body.
    pub(crate) cut: Option<frame_corner::Cut>,
}

/// The trims made after frame member feature `f` (not suppressed) to its members, in timeline
/// order. A trim whose boundary can't be worked out now is left out, as its feature fails too.
pub(crate) fn later_trims(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, f: &Feature) -> Vec<LaterTrim> {
    trims_between(doc, vals, st, f, None)
}

/// [`later_trims`] made before feature `until` (none: all of them).
fn trims_between(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, f: &Feature, until: Option<u64>) -> Vec<LaterTrim> {
    let mut out = Vec::new();
    let Some(at) = doc.features.iter().position(|x| x.id == f.id) else { return out };
    for t in doc.features.iter().skip(at + 1).take_while(|t| Some(t.id) != until).filter(|t| !t.suppressed) {
        let FeatureKind::FrameTrim { members, gap, .. } = &t.kind else { continue };
        let (Ok(boundary), Ok(gap)) = (trim_boundary(doc, vals, st, t), trim_gap(vals, gap)) else { continue };
        for te in members {
            let cut = match &boundary {
                Boundary::Body(_) => None,
                Boundary::Plane(pl) => {
                    let Ok(m) = member_of_feature(doc, vals, st, f, &te.body.0) else { continue };
                    let Ok((_, q, _)) = ends(&m, te.end, &te.body.0) else { continue };
                    let Some(n) = pl.x.cross(pl.y).normalized() else { continue };
                    let keep = if (q - pl.origin).dot(n) >= 0.0 { n } else { -n };
                    Some(frame_corner::Cut { origin: pl.origin + keep * gap, normal: keep })
                }
            };
            out.push(LaterTrim { body: te.body.0.clone(), end: te.end, cut });
        }
    }
    out
}

/// [`member_of`] for a body of feature `f` (the body's own feature may be another by now).
fn member_of_feature(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, f: &Feature, name: &str) -> Result<MemberOf> {
    let not_member = || DocError::Invalid(format!("`{name}` is not a frame member"));
    let index = st.bodies.iter().filter(|b| b.feature == f.id).position(|b| b.name == name).ok_or_else(not_member)?;
    let prepared = prepare(doc, vals, f, st)?;
    if index >= prepared.segments.len() {
        return Err(not_member());
    }
    Ok(MemberOf { prepared, cuts: Vec::new(), index })
}

/// The exact extent of `body` along direction `d` (a unit vector): its exact box once turned so
/// `d` runs along X.
pub(crate) fn extent_along(body: &kernel::Body, d: Vec3) -> Result<f64> {
    let x = d.normalized().ok_or_else(|| DocError::Invalid("no direction to measure along".into()))?;
    let helper = if x.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let y = x.cross(helper).normalized().ok_or_else(|| DocError::Invalid("no direction to measure along".into()))?;
    let z = x.cross(y);
    // Column-major, its rows the new X, Y and Z axes: a point p goes to (x·p, y·p, z·p).
    let m = [[x.x, y.x, z.x, 0.0], [x.y, y.y, z.y, 0.0], [x.z, y.z, z.z, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let turned = kernel::transform_matrix(body, m)?;
    let bb = turned.exact_bounds()?;
    Ok(bb.max.x - bb.min.x)
}

/// The line of the frame member that body `name` is: its start and its end, in its feature's
/// component coordinates.
pub fn member_line(doc: &Document, st: &ModelState, name: &str) -> Result<(Vec3, Vec3)> {
    let (vals, _) = doc.param_values();
    let m = member_of(doc, &vals, st, name)?;
    let l = m.prepared.segments.get(m.index).ok_or_else(|| DocError::Invalid(format!("`{name}` is not a frame member")))?;
    Ok((l.a, l.b))
}

/// A picked flat face: a point on it and its plane's normal.
fn flat_face(st: &ModelState, fa: &crate::FaceAt, which: &str) -> Result<(Vec3, Vec3)> {
    let b = st.body(&fa.body).ok_or_else(|| DocError::Unknown(format!("body `{}`", fa.body)))?;
    let k =
        crate::appearance::face_index_at(b, fa.point.0).ok_or_else(|| DocError::Invalid(format!("no face of `{}` at the {which} pick", fa.body)))?;
    let pl = b.body.planar_face(k).ok_or_else(|| DocError::Invalid(format!("the {which} face is not flat")))?;
    let n = pl.x.cross(pl.y).normalized().ok_or_else(|| DocError::Invalid(format!("the {which} face has no direction")))?;
    // The pick moved onto the plane.
    let p = fa.point.0 - n * (fa.point.0 - pl.origin).dot(n);
    Ok((p, n))
}

/// Evaluate a `FrameGusset` feature: one plate in the corner between two flat faces.
pub(super) fn evaluate_gusset(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &mut ModelState) -> Result<()> {
    let _ = doc;
    let FeatureKind::FrameGusset { first, second, shape, d1, d2, thickness, chamfer, clip, offset } = &f.kind else {
        return Err(DocError::Invalid("not a gusset feature".into()));
    };
    let len = |e: &crate::Expr| val(vals, e, Kind::Length);
    let opt = |e: &Option<crate::Expr>| e.as_ref().map(|e| val(vals, e, Kind::Length)).transpose().map(|v| v.unwrap_or(0.0));
    let (d1, d2, t) = (len(d1)?, len(d2)?, len(thickness)?);
    let (chamfer, clip, offset) = (opt(chamfer)?, opt(clip)?, opt(offset)?);
    let big = frame_corner::MAX_EXTENSION;
    if !(d1 > 1e-6 && d2 > 1e-6 && d1 < big && d2 < big) {
        return Err(DocError::Invalid("the gusset's legs must be more than 0".into()));
    }
    if !(t > 1e-6 && t < big) {
        return Err(DocError::Invalid("the gusset's thickness must be more than 0".into()));
    }
    if !(0.0..d1.min(d2)).contains(&clip) {
        return Err(DocError::Invalid("the gusset's corner clip must be 0 or more and shorter than its legs".into()));
    }
    let chamfered = *shape == crate::frame::GussetShape::Chamfered;
    if chamfered && !(chamfer > 1e-6 && chamfer < d1.min(d2)) {
        return Err(DocError::Invalid("the gusset's chamfer must be more than 0 and shorter than its legs".into()));
    }
    if !offset.is_finite() || offset.abs() >= big {
        return Err(DocError::Invalid("the gusset's offset must be a length".into()));
    }
    let (p1, n1) = flat_face(st, first, "first")?;
    let (p2, n2) = flat_face(st, second, "second")?;
    let along = n1.cross(n2);
    let t_dir = along
        .normalized()
        .filter(|_| along.len() > 1e-6)
        .ok_or_else(|| DocError::Invalid("the two faces are parallel: a gusset needs faces that meet".into()))?;
    // The point on both planes nearest the picks' midpoint.
    let m = (p1 + p2) * 0.5;
    let (c1, c2) = (n1.dot(p1) - n1.dot(m), n2.dot(p2) - n2.dot(m));
    let g = n1.dot(n2);
    let det = 1.0 - g * g;
    let (a, b) = ((c1 - g * c2) / det, (c2 - g * c1) / det);
    let o = m + n1 * a + n2 * b + t_dir * offset;
    // Each leg runs along its face, square to the line, towards the pick on it.
    let leg = |n: Vec3, p: Vec3| -> Result<Vec3> {
        let l = t_dir.cross(n).normalized().ok_or_else(|| DocError::Invalid("the gusset's faces have no direction".into()))?;
        let toward = p - o;
        if toward.dot(l).abs() < 1e-9 {
            return Err(DocError::Invalid("pick each face away from the line where the faces meet".into()));
        }
        Ok(if toward.dot(l) > 0.0 { l } else { -l })
    };
    let (a1, a2) = (leg(n1, p1)?, leg(n2, p2)?);
    let e1 = a1;
    let e2 =
        (a2 - e1 * a2.dot(e1)).normalized().ok_or_else(|| DocError::Invalid("the two faces are parallel: a gusset needs faces that meet".into()))?;
    let q = |w: Vec3| solidcore_record::Vec2::new((w - o).dot(e1), (w - o).dot(e2));
    let (tip1, tip2) = (o + a1 * d1, o + a2 * d2);
    let mut pts = Vec::new();
    if clip > 0.0 {
        pts.extend([o + a2 * clip, o + a1 * clip]);
    } else {
        pts.push(o);
    }
    if chamfered {
        // Each tip cut back `chamfer` along its leg, the cut parallel to the other face, as far
        // as the hypotenuse.
        let (c1, c2) = (o + a1 * (d1 - chamfer), o + a2 * (d2 - chamfer));
        pts.extend([c1, c1 + a2 * (d2 * chamfer / d1), c2 + a1 * (d1 * chamfer / d2), c2]);
    } else {
        pts.extend([tip1, tip2]);
    }
    let mut flat: Vec<solidcore_record::Vec2> = pts.into_iter().map(q).collect();
    let area: f64 = flat.iter().zip(flat.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    if area < 0.0 {
        flat.reverse();
    }
    let region = solidcore_record::Region2 { outer: solidcore_record::Loop2::polygon(&flat), holes: Vec::new() };
    let plane = Plane::new(o, e1, e2).ok_or_else(|| DocError::Invalid("the gusset's plane has no direction".into()))?;
    let mut bodies = kernel::extrude(&plane, &[region], -t / 2.0, t / 2.0)?;
    let body = bodies.pop().ok_or_else(|| DocError::Invalid("the gusset came out empty".into()))?;
    let name = match f.body_names.first().filter(|n| !n.trim().is_empty() && st.body(n).is_none()) {
        Some(n) => n.clone(),
        None => super::unique_body_name(st, "Gusset"),
    };
    st.bodies.push(ModelBody::traced_new(name, crate::lineage::numbered_tool(body), f.id));
    Ok(())
}

/// Most edges one weld feature runs along.
pub const MAX_WELD_EDGES: usize = 1000;

/// A weld feature worked out: its leg size, the length welded (both sides counted, stitches
/// only), and each edge as a polyline for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Weld {
    pub feature: u64,
    pub size: f64,
    pub welded: f64,
    /// The bodies its edges are on, in order, each once.
    pub bodies: Vec<String>,
    pub paths: Vec<Vec<Vec3>>,
}

/// Exact distance from `p` to segment `s` (a line or an arc in closed form; another curve by its
/// polyline within 0.01 mm).
fn seg_distance(s: &kernel::PathSeg, p: Vec3) -> f64 {
    let to_line = |a: Vec3, b: Vec3| {
        let d = b - a;
        let l = d.dot(d);
        let t = if l > 0.0 { ((p - a).dot(d) / l).clamp(0.0, 1.0) } else { 0.0 };
        p.dist(a + d * t)
    };
    match *s {
        kernel::PathSeg::Line { a, b } => to_line(a, b),
        kernel::PathSeg::Arc { a, center, axis, angle } => {
            let Some(k) = axis.normalized() else { return p.dist(a) };
            let r0 = a - center;
            let radius = r0.len();
            let q = p - center;
            let flat = q - k * q.dot(k);
            let (x, y) = (r0.normalized(), k.cross(r0).normalized());
            let (Some(x), Some(y)) = (x, y) else { return p.dist(a) };
            let mut t = flat.dot(y).atan2(flat.dot(x));
            // The angle within the arc's sweep, else the nearer end.
            let (lo, hi) = if angle >= 0.0 { (0.0, angle) } else { (angle, 0.0) };
            let tau = std::f64::consts::TAU;
            while t < lo {
                t += tau;
            }
            while t > hi + 1e-12 && t - tau >= lo - 1e-12 {
                t -= tau;
            }
            let at = |t: f64| center + (x * t.cos() + y * t.sin()) * radius;
            if t <= hi { p.dist(at(t)) } else { p.dist(at(lo)).min(p.dist(at(hi))) }
        }
        _ => kernel::path_polyline(std::slice::from_ref(s))
            .map(|pts| pts.windows(2).map(|w| to_line(w[0], w[1])).fold(f64::INFINITY, f64::min))
            .unwrap_or(f64::INFINITY),
    }
}

/// The edge of `body` that point `p` is on, exactly, and its curve: none when no edge is within
/// a thousandth of the body's size.
fn edge_at(body: &kernel::Body, p: Vec3) -> Option<Vec<kernel::PathSeg>> {
    let tol = (body.size() * 1e-3).max(1e-3);
    let mut best: Option<(f64, Vec<kernel::PathSeg>)> = None;
    for k in 0..body.edge_count() {
        let Ok(path) = kernel::edge_path(body, k) else { continue };
        let d = path.iter().map(|s| seg_distance(s, p)).fold(f64::INFINITY, f64::min);
        if d <= tol && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Weld feature `f` worked out in `st`: every edge found again, the welded length summed exactly.
pub(crate) fn weld_of(vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Weld> {
    let FeatureKind::FrameWeld { edges, size, both_sides, stitch_length, stitch_pitch } = &f.kind else {
        return Err(DocError::Invalid("not a weld feature".into()));
    };
    if edges.is_empty() {
        return Err(DocError::Invalid("pick the edges to weld".into()));
    }
    if edges.len() > MAX_WELD_EDGES {
        return Err(DocError::Invalid(format!("a weld runs along at most {MAX_WELD_EDGES} edges")));
    }
    let big = frame_corner::MAX_EXTENSION;
    let size = val(vals, size, Kind::Length)?;
    if !(size > 1e-6 && size < big) {
        return Err(DocError::Invalid("the weld's size must be more than 0".into()));
    }
    let stitch = match (stitch_length, stitch_pitch) {
        (None, None) => None,
        (Some(l), Some(p)) => {
            let (l, p) = (val(vals, l, Kind::Length)?, val(vals, p, Kind::Length)?);
            if !(l > 1e-6 && p >= l && p < big) {
                return Err(DocError::Invalid("a stitch weld's pitch must be at least its length, both more than 0".into()));
            }
            Some((l, p))
        }
        _ => return Err(DocError::Invalid("a stitch weld needs both its length and its pitch".into())),
    };
    let mut welded = 0.0;
    let mut bodies: Vec<String> = Vec::new();
    let mut paths = Vec::with_capacity(edges.len());
    for (k, e) in edges.iter().enumerate() {
        let b = st.body(&e.body).ok_or_else(|| DocError::Unknown(format!("body `{}`", e.body)))?;
        let path =
            edge_at(&b.body, e.point.0).ok_or_else(|| DocError::Invalid(format!("weld edge {}: no edge of `{}` there any more", k + 1, e.body)))?;
        let length: f64 = path.iter().map(|s| s.length().unwrap_or(0.0)).sum();
        welded += match stitch {
            // Stitches from the start, one every pitch while a whole one fits.
            Some((l, p)) if length >= l => (((length - l) / p).floor() + 1.0) * l,
            Some(_) | None => length,
        };
        if !bodies.contains(&e.body.0) {
            bodies.push(e.body.0.clone());
        }
        paths.push(kernel::path_polyline(&path)?);
    }
    if *both_sides {
        welded *= 2.0;
    }
    Ok(Weld { feature: f.id, size, welded, bodies, paths })
}

/// Every weld feature of the design (not suppressed) as built in `st`; one whose edges can't be
/// found is left out, as its feature fails too.
pub fn welds(doc: &Document, st: &ModelState) -> Vec<Weld> {
    let (vals, _) = doc.param_values();
    doc.features
        .iter()
        .filter(|f| !f.suppressed && matches!(f.kind, FeatureKind::FrameWeld { .. }))
        .filter_map(|f| weld_of(&vals, f, st).ok())
        .collect()
}
