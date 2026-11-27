//! Trim/Extend, gussets and welds (end caps are `crate::end_cap`), on frame members as Frame
//! Member builds them.

use solidcraft_geom::prelude::*;
use std::collections::BTreeMap;

use solidcore_record::{Plane, Region2, Vec2, Vec3};
use solidcraft_kernel as kernel;
use solidcraft_kernel::services::member_measure as measure;

use super::{ModelState, val};
use crate::expr::{Kind, Value};
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

/// `v` at unit length; `None` for no length or a number not finite.
fn unit(v: Vec3) -> Option<Vec3> {
    kernel::services::unit_direction(v.into()).map(Vec3::from)
}

/// [`member_at`] for a body of Frame Member `f`.
fn member_of_feature(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, f: &Feature, name: &str) -> Result<MemberAt> {
    let not_member = || DocError::Invalid(format!("`{name}` is not a frame member"));
    let line = crate::frame::member_of_body(doc, st, f, name)?.ok_or_else(not_member)?;
    let (plane, len, region, _) = super::frame_member::member_section(doc, vals, f, st, &line)?;
    let dir = unit(plane.normal()).ok_or_else(|| DocError::Invalid(format!("the member `{name}` has no direction")))?;
    let (lo, hi) = outline_box(&region);
    let c = Vec2::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
    // Half the section box's diagonal.
    let diagonal = kernel::points::point_distance([lo.x, lo.y, 0.0], [hi.x, hi.y, 0.0]).map_or(0.0, |d| d.value);
    let reach = (diagonal / 2.0).max(1e-6);
    let start = plane.origin + plane.x * c.x + plane.y * c.y;
    Ok(MemberAt { plane, len, region, start, dir, reach })
}

/// A box round `region`'s outline: its line ends, arcs' centres give or take their radii and
/// curves' control points (a curve lies in their hull).
pub(crate) fn outline_box(region: &Region2) -> (Vec2, Vec2) {
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

/// The exact extent of `body` along direction `d`: the lowest and highest of `p·d` over it, from
/// its exact box once turned so `d` runs along X.
pub(crate) fn span_along(body: &kernel::Body, d: Vec3) -> Result<(f64, f64)> {
    let none = || DocError::Invalid("no direction to measure along".into());
    let x = unit(d).ok_or_else(none)?;
    let helper = if x.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let y = measure::unit_normal(x, helper).ok_or_else(none)?;
    let z = measure::unit_normal(x, y).ok_or_else(none)?;
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
    let line = |a: Vec3, b: Vec3| WeldPiece { length: a.dist(b), centre: (a + b) * 0.5, along: unit(b - a) };
    match *s {
        kernel::PathSeg::Line { a, b } => vec![line(a, b)],
        kernel::PathSeg::Arc { a, center, axis, angle } => match measure::arc_centroid(a, center, axis, angle) {
            Some((length, centre)) => vec![WeldPiece { length, centre, along: None }],
            None => Vec::new(),
        },
        _ => kernel::path_polyline(std::slice::from_ref(s)).map(|pts| pts.windows(2).map(|w| line(w[0], w[1])).collect()).unwrap_or_default(),
    }
}

/// Exact distance from `p` to segment `s` (a line or an arc in closed form; another curve by its
/// polyline within 0.01 mm).
fn seg_distance(s: &kernel::PathSeg, p: Vec3) -> f64 {
    match *s {
        kernel::PathSeg::Line { a, b } => measure::segment_distance(a, b, p),
        kernel::PathSeg::Arc { a, center, axis, angle } => measure::arc_distance(a, center, axis, angle, p).unwrap_or_else(|| p.dist(a)),
        _ => kernel::path_polyline(std::slice::from_ref(s))
            .map(|pts| pts.windows(2).map(|w| measure::segment_distance(w[0], w[1], p)).fold(f64::INFINITY, f64::min))
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
            let along = |p: Vec3| kernel::affine::distance_along([0.0; 3], m.dir.into(), p.into());
            let (flo, fhi) = pts.iter().map(|p| along(*p)).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, z), t| (a.min(t), z.max(t)));
            let plane = b.body.planar_face(k);
            match plane.and_then(|pl| measure::end_cut(pl.normal(), m.dir, m.plane.x, m.plane.y)) {
                Some(cut) => {
                    // An end face: its outward normal points out of the end it is on.
                    let e = usize::from(cut.at_end);
                    // Unit normals: their projection on each other is the cosine between them.
                    if normals[e].iter().any(|x| kernel::affine::distance_along([0.0; 3], (*x).into(), cut.normal.into()) > 1.0 - 1e-9) {
                        continue;
                    }
                    normals[e].push(cut.normal);
                    let angle = cut.angle.to_degrees();
                    if angle < 1e-7 {
                        continue;
                    }
                    ends[e].push(EndCut { angle, towards: cut.towards.to_degrees() });
                    if let Some(w) = cut.across {
                        drop[e] = drop[e].max(measure::width_across(&m.region.outer, w) * cut.side / cut.cos);
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
