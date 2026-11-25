//! A frame member as a DSTV NC1 file (`.nc1`), the plain-text format beam lines, saws, drills and
//! tube lasers read: one file per piece. Written from the public DSTV description ("Standard
//! description for steel structure pieces for the numerical controls", 7th edition, July 1998).
//!
//! The piece lies on the standard's axes: X along its length from x = 0 at its nearest point, Y
//! across and Z up, the zero point at the section's lowest Y and Z. An I beam lies with its web
//! flat (flanges `o` at the top Y and `u` at Y = 0), a channel with its web on the bottom and its
//! flanges up, an angle with one leg flat (`v`) and the other standing at Y = 0 (`u`), a tube with
//! its walls `v` on top, `h` underneath, `o` and `u` at the sides. Each face is laid out as a plate,
//! x along the piece and y across it (from Y = 0 on `v` and `h`, from Z = 0 on `o` and `u`).
//!
//! Every outline is read exactly off the member's body: each face's outline (`AK`) is the body cut
//! just inside that face's outer surface (the outside edge of the material; an I beam's web through
//! its middle), its long edges carried out to the section's full width
//! as the standard lays a profile out in plates; a round hole through the wall is a hole (`BO`),
//! another opening an inner outline (`IK`). A round section's face is its outside unrolled (the
//! perimeter across `v`), read off where its body starts and ends along each line of its outside.
//! The header's skew cuts are each end's angle, in the front view for the web and the bottom view
//! for the flange, when that end is one flat cut; an end that is coped or cut in more than one
//! plane gives 0 there and is described by the outlines.

use solidcraft_geom::prelude::*;
use std::fmt::Write as _;

use solidcore_record::{Plane, Seg2, Vec2, Vec3};
use solidcraft_develop::Query;
use solidcraft_kernel as kernel;

use super::ModelState;
use crate::frame::{MemberProfile, Profile};
use crate::frame_library::LibrarySection;
use crate::{DocError, Document, FeatureKind, Result};

/// How far a curved edge's chords may stray from it in an outline (mm).
const CHORD: f64 = 0.01;
/// How far inside a face's outer surface its outline is read (mm): the outside edge of the
/// material, as the standard's outlines are, well under the file's hundredths.
const SKIN: f64 = 1e-3;
/// The longest free text the standard takes in a header line.
const TEXT_MAX: usize = 80;

/// The header's free text: the order, drawing, phase and piece the file is for, the steel grade
/// and how many pieces to make. An empty field is written `-`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nc1Header {
    pub order: String,
    pub drawing: String,
    pub phase: String,
    /// Empty: the member's body name.
    pub piece: String,
    /// Empty: the member's material.
    pub grade: String,
    pub quantity: u32,
}

/// A member written as NC1.
#[derive(Clone, Debug, PartialEq)]
pub struct Nc1 {
    pub text: String,
    /// Its length along X (mm).
    pub length: f64,
    /// The standard's profile code (`I`, `U`, `L`, `M`, `B`, `RO`, `RU`).
    pub code: &'static str,
    /// The faces given an outline, in the order written.
    pub faces: Vec<char>,
    /// Holes written to the `BO` block.
    pub holes: usize,
    /// Openings written as inner outlines (`IK`).
    pub openings: usize,
}

/// One face of the section laid out as a plate: its letter, whether y across it runs along Y
/// (`v`, `h`; else along Z: `o`, `u`) and the level it is read at on the other axis: just inside
/// its outer surface ([`SKIN`]), an I beam's web at its middle.
#[derive(Clone, Copy, Debug)]
struct Face {
    letter: char,
    along_y: bool,
    at: f64,
}

/// How a built-in profile lies on the standard's axes and what its header says.
struct Layout {
    code: &'static str,
    /// The profile's own x and y (as placed) that become Y and Z.
    y: Vec3,
    z: Vec3,
    /// The section's extent along Y and along Z.
    extent: [f64; 2],
    /// Header: profile height, flange width, flange thickness, web thickness, radius.
    header: [f64; 5],
    faces: Vec<Face>,
    /// A round section: its diameter and its wall (the radius for a bar).
    round: Option<(f64, f64)>,
}

fn invalid(msg: impl Into<String>) -> DocError {
    DocError::Invalid(msg.into())
}

/// The built-in profile member `name` is made of; a sketch or a drawn library profile has none.
fn builtin_profile(doc: &Document, vals: &std::collections::BTreeMap<String, crate::expr::Value>, f: &crate::Feature) -> Result<Option<Profile>> {
    let FeatureKind::FrameMember { profile, .. } = &f.kind else { return Ok(None) };
    let spec = match profile {
        MemberProfile::Library { profile } => profile,
        MemberProfile::LibraryProfile { id } => match doc.library_profile(id).map(|e| &e.section) {
            Some(LibrarySection::BuiltIn { profile }) => profile,
            _ => return Ok(None),
        },
        MemberProfile::Sketch { .. } => return Ok(None),
    };
    let p = spec.resolve(|name, e, kind| super::val(vals, e, kind).map_err(|err| invalid(format!("profile `{name}`: {err}"))))?;
    Ok(Some(p.filled()))
}

/// Where `p` lies on the standard's axes, given the profile's own x and y as the member places
/// them (`ex` already turned to where the profile's x points, a mirror included).
fn layout(p: &Profile, ex: Vec3, ey: Vec3) -> Layout {
    let plate = |letter, along_y, at| Face { letter, along_y, at };
    match *p {
        Profile::IBeam { height: h, width: b, web, flange, root_radius } => Layout {
            code: "I",
            y: ey,
            z: -ex,
            extent: [h, b],
            header: [h, b, flange, web, root_radius.unwrap_or(0.0)],
            faces: vec![plate('v', true, b / 2.0), plate('o', false, h - SKIN), plate('u', false, SKIN)],
            round: None,
        },
        Profile::Channel { height: h, width: b, web, flange, root_radius, .. } => Layout {
            code: "U",
            y: -ey,
            z: ex,
            extent: [h, b],
            header: [h, b, flange, web, root_radius.unwrap_or(0.0)],
            faces: vec![plate('v', true, SKIN), plate('o', false, h - SKIN), plate('u', false, SKIN)],
            round: None,
        },
        Profile::Angle { width: w, height: h, thickness: t, root_radius, .. } => Layout {
            code: "L",
            y: ex,
            z: ey,
            extent: [w, h],
            header: [w, h, t, t, root_radius.unwrap_or(0.0)],
            faces: vec![plate('v', true, SKIN), plate('u', false, SKIN)],
            round: None,
        },
        Profile::RectTube { width: w, height: h, wall: t, outer_radius, .. } => Layout {
            code: "M",
            y: ey,
            z: -ex,
            extent: [h, w],
            header: [h, w, t, t, outer_radius.unwrap_or(0.0)],
            faces: vec![plate('v', true, w - SKIN), plate('o', false, h - SKIN), plate('u', false, SKIN), plate('h', true, SKIN)],
            round: None,
        },
        Profile::Bar { width: w, height: h, .. } => {
            // A plate lies flat: its wider side along Y.
            let (y, z, wide, thick) = if w >= h { (ex, ey, w, h) } else { (ey, -ex, h, w) };
            Layout { code: "B", y, z, extent: [wide, thick], header: [wide, 0.0, 0.0, thick, 0.0], faces: vec![plate('v', true, SKIN)], round: None }
        }
        Profile::RoundTube { diameter: d, wall } => Layout {
            code: "RO",
            y: ey,
            z: -ex,
            extent: [d, d],
            header: [d, d, wall, wall, 0.0],
            faces: vec![plate('v', true, d)],
            round: Some((d, wall)),
        },
        Profile::RoundBar { diameter: d } => Layout {
            code: "RU",
            y: ey,
            z: -ex,
            extent: [d, d],
            header: [d, d, 0.0, 0.0, 0.0],
            faces: vec![plate('v', true, d)],
            round: Some((d, d / 2.0)),
        },
    }
}

/// `body` turned so `rows` (unit, square to each other) become X, Y and Z about `origin`.
fn turned(body: &kernel::Body, rows: [Vec3; 3], origin: Vec3) -> Result<kernel::Body> {
    let [x, y, z] = rows;
    let m = [[x.x, y.x, z.x, 0.0], [x.y, y.y, z.y, 0.0], [x.z, y.z, z.z, 0.0], [-x.dot(origin), -y.dot(origin), -z.dot(origin), 1.0]];
    Ok(kernel::transform_matrix(body, m)?)
}

/// Member body `name` as NC1.
pub fn member_nc1(doc: &Document, st: &ModelState, name: &str, header: &Nc1Header) -> Result<Nc1> {
    let vals = doc.evaluation_values();
    let body = st.body(name).ok_or_else(|| DocError::Unknown(format!("body `{name}`")))?;
    let f = doc.feature(body.feature).filter(|f| matches!(f.kind, FeatureKind::FrameMember { .. }));
    let f = f.ok_or_else(|| invalid(format!("`{name}` is not a frame member")))?;
    let profile = builtin_profile(doc, &vals, f)?.ok_or_else(|| {
        invalid(format!(
            "`{name}` is made from a drawn profile: NC1 export takes a built-in section (bar, round bar, tube, angle, channel or I beam)"
        ))
    })?;
    let m = super::weldment::member_at(doc, &vals, st, name)?;
    let description = match &f.kind {
        FeatureKind::FrameMember { profile: mp, .. } => {
            super::frame_member::profile_regions(doc, &vals, st, mp).map(|r| r.description).unwrap_or_else(|_| profile.description())
        }
        _ => profile.description(),
    };
    // A channel's web and an angle's heel are on the profile's -x side unless it is mirrored:
    // the section's centroid says which side they are on as placed.
    let section = crate::frame::section(&m.region);
    let ex = if section.centroid[0] > 1e-9 * section.width.max(1.0) { -m.plane.x } else { m.plane.x };
    let lay = layout(&profile, ex, m.plane.y);
    let x = lay.y.cross(lay.z).normalized().ok_or_else(|| invalid("the member's section has no direction"))?;
    let rotated = turned(&body.body, [x, lay.y, lay.z], Vec3::ZERO)?;
    let bb = rotated.exact_bounds()?;
    let ident = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
    let placed = turned(&rotated, ident, bb.min)?;
    let length = bb.max.x - bb.min.x;
    let [ey, ez] = [bb.max.y - bb.min.y, bb.max.z - bb.min.z];
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-4 * b.abs().max(1.0);
    if !(length > 0.0 && length.is_finite()) || !near(ey, lay.extent[0]) || !near(ez, lay.extent[1]) {
        return Err(invalid(format!(
            "`{name}` is not its profile's full section along its length ({ey:.2} x {ez:.2} mm, the profile {:.2} x {:.2}): NC1 describes a piece cut from whole stock",
            lay.extent[0], lay.extent[1]
        )));
    }
    let mut out = String::new();
    let cuts = end_angles(&placed, length, lay.extent[0].max(lay.extent[1]))?;
    let density = doc.density(name);
    let kg_per_m = section.area * density / 1000.0;
    let paint = perimeter(&m.region.outer.segs) / 1000.0;
    let material = doc.body_material(name);
    let text = |t: &str| {
        let t: String = t.chars().map(|c| if c.is_control() { ' ' } else { c }).take(TEXT_MAX).collect();
        let t = t.trim().to_string();
        if t.is_empty() { "-".to_string() } else { t }
    };
    let piece = if header.piece.trim().is_empty() { name } else { header.piece.as_str() };
    let grade = if header.grade.trim().is_empty() { material.as_str() } else { header.grade.as_str() };
    let mut lines = vec!["ST".to_string(), "** SolidCraft".to_string()];
    for t in [&header.order, &header.drawing, &header.phase, &piece.to_string(), &grade.to_string()] {
        lines.push(format!("  {}", text(t)));
    }
    lines.push(format!("  {}", header.quantity.max(1)));
    lines.push(format!("  {}", text(&description)));
    lines.push(format!("  {}", lay.code));
    lines.push(format!("  {}", f2(length)));
    for v in lay.header {
        lines.push(format!("  {}", f2(v)));
    }
    lines.push(format!("  {}", f3(kg_per_m)));
    lines.push(format!("  {}", f3(paint)));
    for v in [cuts[0][0], cuts[0][1], cuts[1][0], cuts[1][1]] {
        lines.push(format!("  {}", f3(v)));
    }
    for _ in 0..4 {
        lines.push("  -".to_string());
    }
    for l in lines {
        let _ = writeln!(out, "{l}");
    }
    // Each face's outlines and holes.
    let mut faces = Vec::new();
    let (mut holes, mut openings) = (Vec::new(), 0usize);
    for face in &lay.faces {
        let (outers, inner, round_holes) = match lay.round {
            Some((d, wall)) => (vec![unrolled(&placed, length, d, wall)?], Vec::new(), Vec::new()),
            None => plate_loops(&placed, face, lay.extent)?,
        };
        if outers.is_empty() {
            continue;
        }
        faces.push(face.letter);
        for o in &outers {
            contour(&mut out, "AK", face.letter, o);
        }
        for i in &inner {
            contour(&mut out, "IK", face.letter, i);
            openings += 1;
        }
        holes.extend(round_holes.into_iter().map(|(c, d)| (face.letter, c, d)));
    }
    if !holes.is_empty() {
        let _ = writeln!(out, "BO");
        for (letter, c, d) in &holes {
            let _ = writeln!(out, "  {letter}{:>10}u{:>10}{:>10}{:>10}", f2(c.x), f2(c.y), f2(*d), f2(0.0));
        }
    }
    let _ = writeln!(out, "EN");
    Ok(Nc1 { text: out, length, code: lay.code, faces, holes: holes.len(), openings })
}

/// A length to two decimals, a tiny one as 0.
fn f2(v: f64) -> String {
    let v = if v.abs() < 0.005 { 0.0 } else { v };
    format!("{v:.2}")
}

/// A value to three decimals, a tiny one as 0.
fn f3(v: f64) -> String {
    let v = if v.abs() < 0.0005 { 0.0 } else { v };
    format!("{v:.3}")
}

/// One closed outline as a block: the face's letter on its first point, the first point again
/// at the end.
fn contour(out: &mut String, block: &str, letter: char, pts: &[Vec2]) {
    let Some(first) = pts.first() else { return };
    let _ = writeln!(out, "{block}");
    let _ = writeln!(out, "  {letter}{:>10}u{:>10}{:>10}", f2(first.x), f2(first.y), f2(0.0));
    for p in pts.iter().skip(1).chain(std::iter::once(first)) {
        let _ = writeln!(out, "   {:>10} {:>10}{:>10}", f2(p.x), f2(p.y), f2(0.0));
    }
}

/// The length of a loop (mm): lines and arcs exactly, other curves by chords.
fn perimeter(segs: &[Seg2]) -> f64 {
    segs.iter()
        .map(|g| match *g {
            Seg2::Line { a, b } => (b - a).len(),
            Seg2::Arc { radius, sweep, .. } => radius.abs() * sweep.abs(),
            _ => g.polyline(CHORD).windows(2).map(|w| (w[1] - w[0]).len()).sum(),
        })
        .sum()
}

/// A loop's points, its chords within [`CHORD`], the first not repeated at the end.
fn loop_points(segs: &[Seg2]) -> Vec<Vec2> {
    let mut pts: Vec<Vec2> = Vec::new();
    for g in segs {
        for p in g.polyline(CHORD) {
            if pts.last().is_none_or(|l| (p - *l).len() > 1e-6) {
                pts.push(p);
            }
        }
    }
    if pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(a, b)| (*a - *b).len() <= 1e-6) {
        pts.pop();
    }
    pts
}

/// Twice the signed area of a closed polygon: above 0 counter-clockwise.
fn signed_area(pts: &[Vec2]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].x * pts[(i + 1) % n].y - pts[(i + 1) % n].x * pts[i].y).sum()
}

/// `pts` turned to run counter-clockwise (`ccw`) or clockwise.
fn oriented(mut pts: Vec<Vec2>, ccw: bool) -> Vec<Vec2> {
    if (signed_area(&pts) > 0.0) != ccw {
        pts.reverse();
    }
    pts
}

/// The centre and diameter of a loop that is one whole circle.
fn circle(segs: &[Seg2]) -> Option<(Vec2, f64)> {
    let mut centre: Option<(Vec2, f64)> = None;
    let mut turned = 0.0;
    for g in segs {
        let Seg2::Arc { center, radius, sweep, .. } = *g else { return None };
        match centre {
            Some((c, r)) if (c - center).len() > 1e-6 * r.max(1.0) || (r - radius.abs()).abs() > 1e-6 * r.max(1.0) => return None,
            Some(_) => {}
            None => centre = Some((center, radius.abs())),
        }
        turned += sweep.abs();
    }
    let (c, r) = centre?;
    ((turned - std::f64::consts::TAU).abs() < 1e-6).then_some((c, 2.0 * r))
}

/// The outlines (counter-clockwise), the openings (clockwise) and the round holes of `face`, from
/// the body cut just inside its outer surface (an I beam's web through its middle).
type PlateLoops = (Vec<Vec<Vec2>>, Vec<Vec<Vec2>>, Vec<(Vec2, f64)>);

fn plate_loops(placed: &kernel::Body, face: &Face, extent: [f64; 2]) -> Result<PlateLoops> {
    let (origin, q) = if face.along_y {
        (Vec3::new(0.0, 0.0, face.at), Vec3::new(0.0, 1.0, 0.0))
    } else {
        (Vec3::new(0.0, face.at, 0.0), Vec3::new(0.0, 0.0, 1.0))
    };
    let plane = Plane::new(origin, Vec3::new(1.0, 0.0, 0.0), q).ok_or_else(|| invalid("a face of the section has no plane"))?;
    let n = plane.normal().normalized().ok_or_else(|| invalid("a face of the section has no plane"))?;
    // Keep the bulk of the piece, not the skin: a plane's normal is +Z on `v`/`h` and −Y on `o`/`u`.
    let positive = if face.along_y { face.at < extent[1] / 2.0 } else { face.at >= extent[0] / 2.0 };
    let (kept, _) = kernel::half_space(placed, &plane, positive)?;
    let Some(kept) = kept else { return Ok((Vec::new(), Vec::new(), Vec::new())) };
    let rec = Query::new(kept.record());
    let tol = (kept.size() * 1e-7).max(1e-6);
    let mut regions = Vec::new();
    for k in 0..rec.record().faces.len() {
        let Some((fp, out)) = crate::sheet_surface_convert::planar_face(&kept, &rec, k, tol) else { continue };
        if out.cross(n).len() > 1e-7 || (fp.origin - origin).dot(n).abs() > tol * 10.0 {
            continue;
        }
        let r = crate::sheet_surface_convert::face_region(&kept, &rec, &plane, out, k, tol)
            .ok_or_else(|| invalid(format!("the cut through face `{}` can't be read", face.letter)))?;
        regions.push(r);
    }
    // The section's edges across this face (the cut's long edges, lying along the piece: no cut
    // takes a whole edge away along its length, the piece is its full section), and its full
    // width there.
    let qs = regions.iter().flat_map(|r| loop_points(&r.outer.segs)).map(|p| p.y);
    let natural = qs.fold(None, |a: Option<(f64, f64)>, q| Some(a.map_or((q, q), |(lo, hi)| (lo.min(q), hi.max(q)))));
    let width = if face.along_y { extent[0] } else { extent[1] };
    let snap = (width * 1e-6).max(1e-6);
    let mut outers = Vec::new();
    let mut inner = Vec::new();
    let mut holes = Vec::new();
    for r in regions {
        let mut o = oriented(loop_points(&r.outer.segs), true);
        if let Some((lo, hi)) = natural {
            o = widened(o, lo, 0.0, snap);
            o = widened(o, hi, width, snap);
        }
        outers.push(simplified(o));
        for h in &r.holes {
            match circle(&h.segs) {
                Some(c) => holes.push(c),
                None => inner.push(simplified(oriented(loop_points(&h.segs), false))),
            }
        }
    }
    Ok((outers, inner, holes))
}

/// `pts` with each point on the section's edge at `edge` across the face moved to `to`, the face's
/// full width there: along the other edge it lies on, so a straight cut stays straight as it is
/// carried out past a tube's rounded corner.
fn widened(pts: Vec<Vec2>, edge: f64, to: f64, snap: f64) -> Vec<Vec2> {
    if (edge - to).abs() <= snap {
        return pts;
    }
    let n = pts.len();
    let on = |p: Vec2| (p.y - edge).abs() <= snap;
    let mut out = pts.clone();
    for (i, p) in pts.iter().enumerate() {
        if !on(*p) {
            continue;
        }
        let (Some(prev), Some(next)) = (pts.get((i + n - 1) % n), pts.get((i + 1) % n)) else { continue };
        let other = match (on(*prev), on(*next)) {
            (true, false) => Some(*next),
            (false, true) => Some(*prev),
            _ => None,
        };
        let x = match other {
            Some(o) if (o.y - p.y).abs() > 1e-12 => p.x + (o.x - p.x) * (to - p.y) / (o.y - p.y),
            _ => p.x,
        };
        if let Some(slot) = out.get_mut(i) {
            *slot = Vec2::new(x, to);
        }
    }
    out
}

/// `pts` without points that lie on the line through their neighbours (within a micrometre) or
/// repeat the one before at the file's two decimals.
fn simplified(pts: Vec<Vec2>) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_some_and(|l| (p - *l).len() < 0.005) {
            continue;
        }
        while out.len() >= 2 {
            let (Some(a), Some(b)) = (out.get(out.len() - 2), out.last()) else { break };
            let (ab, ap) = (*b - *a, p - *a);
            let len = ab.len();
            if len > 0.0 && (ab.x * ap.y - ab.y * ap.x).abs() / len < 1e-3 && ab.x * ap.x + ab.y * ap.y > len * len {
                out.pop();
            } else {
                break;
            }
        }
        out.push(p);
    }
    while out.len() > 3 && out.first().zip(out.last()).is_some_and(|(a, b)| (*a - *b).len() < 0.005) {
        out.pop();
    }
    out
}

/// A round section's outside unrolled: x along the piece, y the distance round its outside from
/// its top (the side the front view looks at), turning towards `o`. Read where the body starts
/// and ends along lines of its outside a little inside its skin, one about every millimetre.
fn unrolled(placed: &kernel::Body, length: f64, d: f64, wall: f64) -> Result<Vec<Vec2>> {
    let r = d / 2.0;
    let skin = (wall * 0.25).min(0.01);
    let rr = r - skin;
    let round = std::f64::consts::PI * d;
    let n = (round.ceil() as usize).clamp(90, 1440);
    let (mut starts, mut ends) = (Vec::with_capacity(n + 1), Vec::with_capacity(n + 1));
    for i in 0..=n {
        let t = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        let (y, z) = (r + rr * t.sin(), r + rr * t.cos());
        let q = round * i as f64 / n as f64;
        let a = placed.fire_ray(Vec3::new(-1.0, y, z), Vec3::new(1.0, 0.0, 0.0))?;
        let b = placed.fire_ray(Vec3::new(length + 1.0, y, z), Vec3::new(-1.0, 0.0, 0.0))?;
        if let (Some(a), Some(b)) = (a, b) {
            starts.push(Vec2::new(a.point.x.max(0.0), q));
            ends.push(Vec2::new(b.point.x.min(length), q));
        }
    }
    if starts.len() < 3 {
        return Err(invalid("the round member's outside can't be read"));
    }
    // Along the end side as y rises, back along the start side: counter-clockwise.
    let mut pts = ends;
    pts.extend(starts.into_iter().rev());
    Ok(simplified(oriented(pts, true)))
}

/// Each end's skew cut, `[[web start, web end], [flange start, flange end]]` in degrees: the
/// web's in the front view (X and Y), the flange's in the bottom view (X and Z), positive turning
/// counter-clockwise. An end cut by more than one plane, or coped, gives 0.
fn end_angles(placed: &kernel::Body, length: f64, size: f64) -> Result<[[f64; 2]; 2]> {
    let near = (size * 2.0).min(length * 0.5);
    let mut planes: [Vec<Vec3>; 2] = [Vec::new(), Vec::new()];
    let mut curved = [false, false];
    for k in 0..placed.face_count() {
        let pts: Vec<Vec3> = placed.face_edge_polylines(k).into_iter().flatten().collect();
        let (lo, hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, z), p| (a.min(p.x), z.max(p.x)));
        if !lo.is_finite() || hi - lo >= length * 0.5 {
            continue;
        }
        let end = usize::from(lo + hi > length);
        let Some(pl) = placed.planar_face(k) else {
            if (hi > length - near || lo < near)
                && let Some(c) = curved.get_mut(end)
            {
                *c = true;
            }
            continue;
        };
        let Some(n) = pl.normal().normalized() else { continue };
        if n.x.abs() < 1e-9 {
            continue;
        }
        // Pointing out of its end.
        let n = if (n.x > 0.0) == (end == 1) { n } else { -n };
        if let Some(seen) = planes.get_mut(end)
            && !seen.iter().any(|m| m.dot(n) > 1.0 - 1e-9)
        {
            seen.push(n);
        }
    }
    let mut out = [[0.0; 2]; 2];
    for e in 0..2 {
        let (Some(ps), Some(c)) = (planes.get(e), curved.get(e)) else { continue };
        if *c || ps.len() != 1 {
            continue;
        }
        let Some(n) = ps.first() else { continue };
        if let Some(row) = out.get_mut(0).and_then(|r| r.get_mut(e)) {
            *row = (n.y / n.x).atan().to_degrees();
        }
        if let Some(row) = out.get_mut(1).and_then(|r| r.get_mut(e)) {
            *row = (n.z / n.x).atan().to_degrees();
        }
    }
    Ok(out)
}
