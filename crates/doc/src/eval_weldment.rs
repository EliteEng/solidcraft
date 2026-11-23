//! Trim/Extend, gussets and welds (end caps are `crate::end_cap`), on frame members as Frame
//! Member builds them.

use solidcraft_geom::prelude::*;
use std::collections::BTreeMap;

use solidcore_record::{Plane, Region2, Vec2, Vec3};
use solidcraft_kernel as kernel;

use super::{ModelBody, ModelState, val};
use crate::expr::{Kind, Value};
use crate::frame::{MAX_MEMBERS, TrimEnd, TrimEndSide};
use crate::lineage::Traced;
use crate::{DocError, Document, Feature, FeatureKind, Result};

/// The largest length a gusset, weld or trim works with (mm).
const BIG: f64 = 1e6;

/// A frame member body's member as its Frame Member made it, in that feature's component
/// coordinates: the plane square to its line at its start (x × y along the line), its length
/// there, its profile in that plane and how far that reaches from the line.
pub(crate) struct MemberAt {
    pub(crate) plane: Plane,
    pub(crate) len: f64,
    pub(crate) region: Region2,
    /// The axis the member runs along: the centre of its profile's box at its start.
    pub(crate) start: Vec3,
    pub(crate) dir: Vec3,
    pub(crate) reach: f64,
}

impl MemberAt {
    pub(crate) fn end(&self) -> Vec3 {
        self.start + self.dir * self.len
    }
}

/// The member that body `name` is.
pub(crate) fn member_at(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, name: &str) -> Result<MemberAt> {
    let body = st.body(name).ok_or_else(|| DocError::Unknown(format!("body `{name}`")))?;
    let not_member = || DocError::Invalid(format!("`{name}` is not a frame member"));
    let f = doc.feature(body.feature).filter(|f| matches!(f.kind, FeatureKind::FrameMember { .. })).ok_or_else(not_member)?;
    member_of_feature(doc, vals, st, f, name)
}

/// [`member_at`] for a body of Frame Member `f`.
fn member_of_feature(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, f: &Feature, name: &str) -> Result<MemberAt> {
    let not_member = || DocError::Invalid(format!("`{name}` is not a frame member"));
    let line = crate::frame::member_of_body(doc, st, f, name)?.ok_or_else(not_member)?;
    let (plane, len, region, _) = super::frame_member::member_section(doc, vals, f, st, &line)?;
    let dir = plane.normal().normalized().ok_or_else(|| DocError::Invalid(format!("the member `{name}` has no direction")))?;
    let (lo, hi) = outline_box(&region);
    let c = Vec2::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
    let reach = ((hi.x - lo.x).hypot(hi.y - lo.y) / 2.0).max(1e-6);
    let start = plane.origin + plane.x * c.x + plane.y * c.y;
    Ok(MemberAt { plane, len, region, start, dir, reach })
}

/// A box round `region`'s outline: its line ends, arcs' centres give or take their radii and
/// curves' control points (a curve lies in their hull).
fn outline_box(region: &Region2) -> (Vec2, Vec2) {
    use solidcore_record::Seg2;
    let (mut lo, mut hi) = (Vec2::new(f64::INFINITY, f64::INFINITY), Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut add = |p: Vec2| {
        if p.x.is_finite() && p.y.is_finite() {
            lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
        }
    };
    for g in &region.outer.segs {
        match g {
            Seg2::Line { a, b } => [*a, *b].into_iter().for_each(&mut add),
            Seg2::Arc { center, radius, .. } => {
                let r = radius.abs();
                add(Vec2::new(center.x - r, center.y - r));
                add(Vec2::new(center.x + r, center.y + r));
            }
            Seg2::Cubic { p0, p1, p2, p3 } => [*p0, *p1, *p2, *p3].into_iter().for_each(&mut add),
            Seg2::Conic { a, apex, b, .. } => [*a, *apex, *b].into_iter().for_each(&mut add),
            Seg2::Bezier { n, p } => p.get(..=usize::from(*n)).unwrap_or(p).iter().copied().for_each(&mut add),
        }
    }
    if lo.x > hi.x { (Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0)) } else { (lo, hi) }
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
    if !(0.0..BIG).contains(&g) {
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
        let m = member_at(doc, vals, st, &te.body)?;
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

/// Member `m`'s axis ends: the end worked on, the other end, and the outward direction there.
fn ends(m: &MemberAt, end: TrimEndSide) -> (Vec3, Vec3, Vec3) {
    match end {
        TrimEndSide::Start => (m.start, m.end(), -m.dir),
        TrimEndSide::End => (m.end(), m.start, m.dir),
    }
}

/// Member body `body` with end `te.end` trimmed at `boundary`, `gap` short of it, lengthened to it
/// first when `extend` is on.
fn trim_end(m: &MemberAt, body: &kernel::Body, te: &TrimEnd, boundary: &Boundary, gap: f64, extend: bool) -> Result<kernel::Body> {
    let name = &te.body.0;
    let (e, q, u) = ends(m, te.end);
    let len = m.len;
    let margin = 0.01 * (m.reach + 1.0);
    // How far past its axis end the boundary needs the member (≤ 0: it is reached already).
    let ahead = match boundary {
        Boundary::Plane(pl) => {
            let n = pl.normal().normalized().ok_or_else(|| DocError::Invalid("the boundary plane has no direction".into()))?;
            let along = u.dot(n);
            if along.abs() < 1e-9 {
                return Err(DocError::Invalid(format!("`{name}` runs along the boundary")));
            }
            let t = (pl.origin - e).dot(n) / along;
            if t <= -len {
                return Err(DocError::Invalid(format!("the boundary is past the other end of `{name}`")));
            }
            t + m.reach * (1.0 - along * along).max(0.0).sqrt() / along.abs() + margin
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
        if ahead > BIG {
            return Err(DocError::Invalid(format!("the boundary is too far from `{name}` to extend to")));
        }
        // From far enough back that a corner's cut (a mitre, a butt, a cope) at this end is
        // covered, to past the boundary: the whole profile, so the old end goes.
        let back = (4.0 * m.reach + margin).min(0.5 * len);
        // Distances along the section plane's normal (the line, start to end) from its origin.
        let at = (e - m.plane.origin).dot(m.dir);
        let (from, to) = match te.end {
            TrimEndSide::End => (at - back, at + ahead),
            TrimEndSide::Start => (at - ahead, at + back),
        };
        for piece in kernel::extrude(&m.plane, std::slice::from_ref(&m.region), from, to)? {
            body = kernel::boolean(&body, &piece, kernel::BoolOp::Union)?
                .ok_or_else(|| DocError::Invalid(format!("extending `{name}` left nothing")))?;
        }
    }
    let kept = match boundary {
        Boundary::Plane(pl) => {
            let n = pl.normal().normalized().ok_or_else(|| DocError::Invalid("the boundary plane has no direction".into()))?;
            // Keep the side the other end is on, the gap short of the plane.
            let keep = if (q - pl.origin).dot(n) >= 0.0 { n } else { -n };
            let cut =
                Plane::from_normal(pl.origin + keep * gap, keep).ok_or_else(|| DocError::Invalid("the boundary plane has no direction".into()))?;
            kernel::half_space(&body, &cut, true)?.0
        }
        Boundary::Body(b) => {
            // With a gap, cope by the body grown that far all round.
            let tool = if gap > 0.0 {
                kernel::grown(b, gap).map_err(|err| DocError::Invalid(format!("the trim's gap round the body: {err}")))?
            } else {
                b.clone()
            };
            kernel::boolean(&body, &tool, kernel::BoolOp::Cut)?
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

/// The exact extent of `body` along direction `d`: the lowest and highest of `p·d` over it, from
/// its exact box once turned so `d` runs along X.
pub(crate) fn span_along(body: &kernel::Body, d: Vec3) -> Result<(f64, f64)> {
    let x = d.normalized().ok_or_else(|| DocError::Invalid("no direction to measure along".into()))?;
    let helper = if x.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let y = x.cross(helper).normalized().ok_or_else(|| DocError::Invalid("no direction to measure along".into()))?;
    let z = x.cross(y);
    // Column-major, its rows the new X, Y and Z axes: a point p goes to (x·p, y·p, z·p).
    let m = [[x.x, y.x, z.x, 0.0], [x.y, y.y, z.y, 0.0], [x.z, y.z, z.z, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let turned = kernel::transform_matrix(body, m)?;
    let bb = turned.exact_bounds()?;
    Ok((bb.min.x, bb.max.x))
}


/// The axis of the frame member that body `name` is: its start and its end as made, in its
/// feature's component coordinates.
pub fn member_line(doc: &Document, st: &ModelState, name: &str) -> Result<(Vec3, Vec3)> {
    let vals = doc.evaluation_values();
    let m = member_at(doc, &vals, st, name)?;
    Ok((m.start, m.end()))
}

/// The frame of the member that body `name` is: its axis's start, the unit direction along it,
/// and the section's x and y as placed (so `(along, x, y)` is right-handed), in its feature's
/// component coordinates.
pub fn member_frame(doc: &Document, st: &ModelState, name: &str) -> Result<(Vec3, Vec3, Vec3, Vec3)> {
    let vals = doc.evaluation_values();
    let m = member_at(doc, &vals, st, name)?;
    Ok((m.start, m.dir, m.plane.x, m.plane.y))
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
    let big = BIG;
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
    /// The edges' pieces, each once (not doubled for both sides): its exact length, its centre
    /// (the centroid of the curve) and, for a straight piece, its direction.
    pub pieces: Vec<WeldPiece>,
}

/// A piece of a weld's edge, for where its mass is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeldPiece {
    pub length: f64,
    pub centre: Vec3,
    pub along: Option<Vec3>,
}

impl Weld {
    /// The weld's volume (mm³): its triangular section, `size`² / 2, times the length welded.
    pub fn volume(&self) -> f64 {
        self.size * self.size / 2.0 * self.welded
    }
}

/// Segment `s` as weld pieces: a line or an arc exactly, another curve by its polyline within
/// 0.01 mm.
fn weld_pieces(s: &kernel::PathSeg) -> Vec<WeldPiece> {
    let line = |a: Vec3, b: Vec3| WeldPiece { length: a.dist(b), centre: (a + b) * 0.5, along: (b - a).normalized() };
    match *s {
        kernel::PathSeg::Line { a, b } => vec![line(a, b)],
        kernel::PathSeg::Arc { a, center, axis, angle } => {
            let r0 = a - center;
            let radius = r0.len();
            let (Some(k), Some(x)) = (axis.normalized(), r0.normalized()) else { return Vec::new() };
            let y = k.cross(x);
            let half = angle / 2.0;
            // An arc's centroid is on its bisector, r·sin(θ/2)/(θ/2) from the centre.
            let reach = if half.abs() > 1e-12 { radius * half.sin() / half } else { radius };
            let mid = x * half.cos() + y * half.sin();
            vec![WeldPiece { length: radius * angle.abs(), centre: center + mid * reach, along: None }]
        }
        _ => kernel::path_polyline(std::slice::from_ref(s)).map(|pts| pts.windows(2).map(|w| line(w[0], w[1])).collect()).unwrap_or_default(),
    }
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
    let big = BIG;
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
    let mut pieces = Vec::new();
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
        pieces.extend(path.iter().flat_map(weld_pieces));
        paths.push(kernel::path_polyline(&path)?);
    }
    if *both_sides {
        welded *= 2.0;
    }
    Ok(Weld { feature: f.id, size, welded, bodies, paths, pieces })
}

/// Every weld feature of the design (not suppressed) as built in `st`; one whose edges can't be
/// found is left out, as its feature fails too.
pub fn welds(doc: &Document, st: &ModelState) -> Vec<Weld> {
    let vals = doc.evaluation_values();
    doc.features
        .iter()
        .filter(|f| !f.suppressed && matches!(f.kind, FeatureKind::FrameWeld { .. }))
        .filter_map(|f| weld_of(&vals, f, st).ok())
        .collect()
}

/// A cut at one end of a member, as the cut list gives it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct EndCut {
    /// How far the cut leans from square to the member (degrees; 0: a square end).
    pub angle: f64,
    /// Which way it leans, as a direction in the profile's own x and y (degrees from +x; 0 for a
    /// square end): the cut reaches furthest along the member on that side.
    pub towards: f64,
}

/// One member of a Frame Member feature as the cut list takes it, measured on its body as built
/// (corners, trims and later cuts included).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MemberCut {
    /// Its body's name.
    pub body: String,
    /// Members with the same key have the same section (see the cut list's grouping).
    pub profile_key: String,
    pub description: String,
    /// The longest extent along its line (mm).
    pub length: f64,
    /// The shortest extent along its line (mm), what a saw marks for a mitre.
    pub short_length: f64,
    /// The flat cuts at its start and at its end, the most leaning first; none: a square end.
    pub ends: [Vec<EndCut>; 2],
    /// Whether its start and its end are coped (fitted round another member).
    pub coped: [bool; 2],
}

/// How wide `region` is across direction `w` (a unit vector in its plane): exact for lines and
/// arcs, a curve by its control points.
fn width_across(region: &Region2, w: Vec2) -> f64 {
    use solidcore_record::Seg2;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut add = |v: f64| {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    };
    let dot = |p: Vec2| p.x * w.x + p.y * w.y;
    for g in &region.outer.segs {
        match g {
            Seg2::Line { a, b } => {
                add(dot(*a));
                add(dot(*b));
            }
            Seg2::Arc { center, radius, start, sweep } => {
                // The arc's ends, and where it reaches furthest along ±w if that lies on it.
                let at = |t: f64| Vec2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
                add(dot(at(*start)));
                add(dot(at(start + sweep)));
                let face = w.y.atan2(w.x);
                for t in [face, face + std::f64::consts::PI] {
                    let mut d = (t - start).rem_euclid(std::f64::consts::TAU);
                    if *sweep < 0.0 {
                        d -= std::f64::consts::TAU;
                    }
                    if (sweep.abs() - d.abs()) >= 0.0 && d * sweep >= 0.0 {
                        add(dot(at(t)));
                    }
                }
            }
            Seg2::Cubic { p0, p1, p2, p3 } => [*p0, *p1, *p2, *p3].into_iter().for_each(|p| add(dot(p))),
            Seg2::Conic { a, apex, b, .. } => [*a, *apex, *b].into_iter().for_each(|p| add(dot(p))),
            Seg2::Bezier { n, p } => p.get(..=usize::from(*n)).unwrap_or(p).iter().for_each(|p| add(dot(*p))),
        }
    }
    if hi >= lo { hi - lo } else { 0.0 }
}

/// The members of Frame Member `f` as built in `st`, measured on their bodies: each one's lengths
/// and end cuts, in the order the feature makes its bodies. A body that is no member of `f` any
/// more (an End Cap's) is passed over.
pub fn member_cuts_report(doc: &Document, st: &ModelState, f: &Feature) -> Result<Vec<MemberCut>> {
    let FeatureKind::FrameMember { profile, mirror, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let vals = doc.evaluation_values();
    let description = super::frame_member::profile_regions(doc, &vals, st, profile).map(|r| r.description).unwrap_or_else(|_| f.name.clone());
    let groups = crate::frame::member_groups(&f.kind);
    let mut out = Vec::new();
    for b in st.bodies.iter().filter(|b| b.feature == f.id) {
        let Ok(Some(line)) = crate::frame::member_of_body(doc, st, f, &b.name) else { continue };
        let Ok(m) = member_of_feature(doc, &vals, st, f, &b.name) else { continue };
        // A group's own mirror makes the other hand.
        let mirrored = groups.iter().find(|g| g.lines.contains(&line)).map_or(*mirror, |g| g.mirror);
        let profile_key = if mirrored { format!("{description}|mirrored") } else { description.clone() };
        let (lo, hi) = span_along(&b.body, m.dir)?;
        let length = hi - lo;
        if !(length > 0.0 && length.is_finite()) {
            continue;
        }
        let near = (m.reach * 2.0).min(length * 0.5);
        let mut ends: [Vec<EndCut>; 2] = [Vec::new(), Vec::new()];
        let mut coped = [false, false];
        let mut drop = [0.0f64, 0.0];
        let mut normals: [Vec<Vec3>; 2] = [Vec::new(), Vec::new()];
        for k in 0..b.body.face_count() {
            // Where the face lies along the member, from its edges' exact points.
            let pts: Vec<Vec3> = b.body.face_edge_polylines(k).into_iter().flatten().collect();
            let (flo, fhi) = pts.iter().map(|p| p.dot(m.dir)).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, z), t| (a.min(t), z.max(t)));
            let plane = b.body.planar_face(k);
            let leaning = plane.and_then(|pl| pl.normal().normalized()).filter(|n| n.dot(m.dir).abs() > 1e-9);
            match leaning {
                Some(n) => {
                    // An end face: its outward normal points out of the end it is on.
                    let e = usize::from(n.dot(m.dir) > 0.0);
                    if normals[e].iter().any(|x| x.dot(n) > 1.0 - 1e-9) {
                        continue;
                    }
                    normals[e].push(n);
                    let c = n.dot(m.dir).abs().min(1.0);
                    let angle = c.acos().to_degrees();
                    if angle < 1e-7 {
                        continue;
                    }
                    let (nx, ny) = (n.dot(m.plane.x), n.dot(m.plane.y));
                    let side = nx.hypot(ny);
                    let towards = (-ny).atan2(-nx).to_degrees();
                    ends[e].push(EndCut { angle, towards });
                    if side > 1e-12 {
                        let w = Vec2::new(nx / side, ny / side);
                        drop[e] = drop[e].max(width_across(&m.region, w) * side / c);
                    }
                }
                None if flo.is_finite() && fhi - flo < length * 0.5 => {
                    // A short face along the member at one end: a cope's or a notch's.
                    if fhi > hi - near {
                        coped[1] = true;
                    } else if flo < lo + near {
                        coped[0] = true;
                    }
                }
                None => {}
            }
        }
        for e in &mut ends {
            e.sort_by(|x, y| y.angle.total_cmp(&x.angle));
        }
        let short = |e: usize| if coped.get(e).copied().unwrap_or(false) { 0.0 } else { drop.get(e).copied().unwrap_or(0.0) };
        let short_length = (length - short(0) - short(1)).max(0.0);
        out.push(MemberCut { body: b.name.clone(), profile_key, description: description.clone(), length, short_length, ends, coped });
    }
    Ok(out)
}
