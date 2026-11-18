//! Frame members (#4003): one body per straight line of a frame sketch, its cross-section a
//! profile laid square to the line (`frame::member_axes`) and run from one end to the other.

use solidcraft_geom::prelude::*;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};

use solidcore_record::{Plane, Region2, Vec2, Vec3};
use solidcraft_geom::{Aabb2, Ring};
use solidcraft_kernel as kernel;
use solidcraft_sketch::CurveKind;

use super::{ModelBody, ModelState, SolvedSketch, new_name, path_in_frame, select_profiles, sketch_in_feature_frame, val};
use crate::expr::{Expr, Kind, Value};
use crate::frame::{self, MAX_MEMBERS, MemberProfile};
use crate::frame_corner::{self, CornerDefaults, CornerOverride, MemberCuts, MemberLine};
use crate::frame_library;
use crate::{DocError, Document, Feature, FeatureId, FeatureKind, ProfileSel, Result};

/// Most profiles a profile sketch may have for its material to be worked out.
const MAX_PROFILE_PIECES: usize = 256;
/// Shortest member (mm).
const MIN_LENGTH: f64 = 1e-6;
/// How far (mm) a 3D sketch curve may stray from the line between its ends and still be a line.
const STRAIGHT_TOL: f64 = 1e-3;

/// Evaluate a `FrameMember` feature: one new body per line, each named
/// ([`member_names`], or by position for a copy whose `body_names` were set).
pub(super) fn evaluate(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &mut ModelState, warning: &mut Option<String>) -> Result<()> {
    let FeatureKind::FrameMember { names, corners, corner_overrides, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let prepared = prepare(doc, vals, f, st)?;
    let Prepared { segments, group_of, sections, .. } = &prepared;
    let placed = prepared.placed()?;
    let cuts = corner_cuts(vals, segments, group_of, &placed, corners, corner_overrides, warning)?;
    let mut made = Vec::with_capacity(segments.len());
    for ((l, p), c) in segments.iter().zip(&placed).zip(cuts) {
        let id = &l.id;
        let plane = Plane::new(l.a - p.x * p.at.x - p.y * p.at.y, p.x, p.y)
            .ok_or_else(|| DocError::Invalid(format!("line `{id}`: no plane square to it")))?;
        let mut bodies = kernel::extrude(&plane, p.regions, -c.ext[0], l.a.dist(l.b) + c.ext[1])?;
        let mut body = match (bodies.pop(), bodies.is_empty()) {
            (Some(body), true) => body,
            _ => return Err(DocError::Invalid(format!("the member on line `{id}` is not one solid"))),
        };
        for (_, junction, cut) in &c.cuts {
            let plane = Plane::from_normal(cut.origin, cut.normal)
                .ok_or_else(|| DocError::Invalid(format!("junction `{junction}`: the cut on line `{id}` has no direction")))?;
            body = match kernel::half_space(&body, &plane, true)?.0 {
                Some(b) if b.lumps().map_or(1, |l| l.len()) <= 1 => b,
                Some(_) => return Err(DocError::Invalid(format!("junction `{junction}` cuts the member on line `{id}` in two (try butt or none)"))),
                None => return Err(DocError::Invalid(format!("junction `{junction}` cuts away the whole member on line `{id}` (try butt or none)"))),
            };
        }
        for cope in &c.copes {
            body = cope_member(body, cope, &placed, id)?;
        }
        made.push(crate::lineage::numbered_tool(body));
    }
    if !f.body_names.is_empty() {
        // A copy (pasted, or in a copied component) names its members by position, as any
        // feature's bodies.
        for (k, t) in made.into_iter().enumerate() {
            let name = new_name(st, f, k);
            st.bodies.push(ModelBody::traced_new(name, t, f.id));
        }
        return Ok(());
    }
    // Named per group, after its name or profile; a pinned name holds across groups.
    let ids: Vec<String> = segments.iter().map(|l| l.id.clone()).collect();
    let bases: Vec<&str> = sections.iter().map(|g| g.base.as_str()).collect();
    let taken = grouped_member_names(st, &ids, group_of, &bases, names);
    for (name, t) in taken.into_iter().zip(made) {
        st.bodies.push(ModelBody::traced_new(name, t, f.id));
    }
    Ok(())
}

/// Member `body` (on line `id`) trimmed by `cope`: the through member's outline, holes filled and
/// grown by the gap, run along its line either side of the junction.
fn cope_member(body: kernel::Body, cope: &frame_corner::Cope, placed: &[frame_corner::Placed], id: &str) -> Result<kernel::Body> {
    let j = &cope.junction;
    let t = placed.get(cope.through).ok_or_else(|| DocError::Invalid(format!("junction `{j}`: no member to cope round")))?;
    let along = (t.b - t.a).normalized().ok_or_else(|| DocError::Invalid(format!("junction `{j}`: the member to cope round has no length")))?;
    let foot = t.a + along * (cope.at - t.a).dot(along);
    let plane = Plane::new(foot - t.x * t.at.x - t.y * t.at.y, t.x, t.y)
        .ok_or_else(|| DocError::Invalid(format!("junction `{j}`: no plane square to the member to cope round")))?;
    let outline = t
        .regions
        .iter()
        .map(|r| kernel::grown_loop(&r.outer, cope.gap).map(|outer| Region2 { outer, holes: Vec::new() }))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| DocError::Invalid(format!("junction `{j}`: the cope's outline: {e}")))?;
    let tools = kernel::extrude(&plane, &outline, -cope.reach, cope.reach)?;
    let mut body = body;
    for tool in &tools {
        body = match kernel::boolean(&body, tool, kernel::BoolOp::Cut)? {
            Some(b) if b.lumps().map_or(1, |l| l.len()) <= 1 => b,
            Some(_) => return Err(DocError::Invalid(format!("junction `{j}`: the cope cuts the member on line `{id}` in two (try butt)"))),
            None => return Err(DocError::Invalid(format!("junction `{j}`: the cope cuts away the whole member on line `{id}` (try butt)"))),
        };
    }
    Ok(body)
}

/// A group's section as its members take it.
struct GroupSection {
    regions: Vec<Region2>,
    at: Vec2,
    turn: f64,
    /// The profile point a Rotate step turns its members about: the centre of its bounding box.
    centre: Vec2,
    /// What its members are named after.
    base: String,
    /// The profile's own description (`Rect Tube 40x40x3`).
    description: String,
    /// Members of groups with the same key have the same section: the library profile, the
    /// built-in profile as resolved, or the sketch profiles; and the mirror.
    key: String,
}

/// A frame member feature's lines, groups and sections, ready to place.
pub(crate) struct Prepared {
    pub(crate) segments: Vec<MemberLine>,
    pub(crate) group_of: Vec<usize>,
    sections: Vec<GroupSection>,
    /// Each line's further turn (radians) from the Rotate steps after the feature
    /// ([`member_turns`]: a step whose angle fails is passed over).
    turns: BTreeMap<String, f64>,
    normal: Option<Vec3>,
}

impl Prepared {
    pub(crate) fn placed(&self) -> Result<Vec<frame_corner::Placed<'_>>> {
        let mut placed = Vec::with_capacity(self.segments.len());
        for (l, g) in self.segments.iter().zip(&self.group_of) {
            let s = self.sections.get(*g).ok_or_else(|| DocError::Invalid(format!("line `{}`: no group {g}", l.id)))?;
            let plane = member_plane(l.a, l.b, s.turn, s.at, s.centre, self.turns.get(&l.id).copied(), self.normal)
                .ok_or_else(|| DocError::Invalid(format!("line `{}`: no plane square to it", l.id)))?;
            placed.push(placed_on(l, &plane, &s.regions));
        }
        Ok(placed)
    }
}

/// Work out feature `f`'s member lines (by group) and each group's section as placed.
pub(crate) fn prepare(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<Prepared> {
    let FeatureKind::FrameMember { path_sketch, lines, profile, anchor, locate, point, angle, mirror, body_name, posts_world_x, groups, .. } =
        &f.kind
    else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let ps = st.sketch(path_sketch.0).ok_or_else(|| DocError::Unknown(format!("sketch {path_sketch} (it must come earlier in the timeline)")))?;
    // The frame sketch in this feature's component frame: its plane, and its 3D lines.
    let mapped =
        if ps.sketch.wires.is_empty() { Cow::Owned(sketch_in_feature_frame(doc, ps, f.component)?) } else { path_in_frame(doc, ps, f.component)? };
    let (segments, group_of) = grouped_lines(mapped.as_ref(), lines, groups)?;
    // Each group's section as placed, its angle and what its members are named after.
    let own = GroupSpec {
        name: body_name.as_deref(),
        profile,
        anchor: *anchor,
        point: point.as_deref(),
        locate: *locate,
        angle: angle.as_ref(),
        mirror: *mirror,
    };
    let mut sections = Vec::with_capacity(groups.len() + 1);
    for (k, g) in std::iter::once(own).chain(groups.iter().map(GroupSpec::of)).enumerate() {
        let label = |e: DocError| if k == 0 { e } else { DocError::Invalid(format!("group {}: {e}", k + 1)) };
        let section = profile_regions(doc, vals, st, g.profile).map_err(label)?;
        let description = section.description.clone();
        let base = g.name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| description.clone(), str::to_string);
        let source = match g.profile {
            MemberProfile::LibraryProfile { id } => format!("library:{id}"),
            MemberProfile::Library { .. } => format!("builtin:{description}"),
            MemberProfile::Sketch { sketch, profiles } => format!("sketch:{}:{}", sketch.0, serde_json::to_string(profiles).unwrap_or_default()),
        };
        let key = format!("{source}|{}", if g.mirror { "mirrored" } else { "" });
        let (regions, at) = placed_section(section, g.anchor, g.point, g.locate, g.mirror).map_err(label)?;
        let turn = match g.angle {
            Some(e) => val(vals, e, Kind::Angle).map_err(label)?,
            None => 0.0,
        };
        let centre = frame::anchor_point(&regions, frame::Anchor::Center);
        sections.push(GroupSection { regions, at, turn, centre, base, description, key });
    }
    let turns = member_turns(doc, vals, FeatureId(f.id)).into_iter().map(|(line, (_, a))| (line, a)).collect();
    // A flat sketch's normal sets which way its posts face; a 3D sketch has none.
    let normal = (ps.sketch.wires.is_empty() && !*posts_world_x).then(|| mapped.plane.x.cross(mapped.plane.y));
    Ok(Prepared { segments, group_of, sections, turns, normal })
}

/// The plane a member's profile is laid in at `a`, square to the line on to `b`: profile point
/// `at` on the line, the profile turned `turn` about it. A Rotate step's turn `own` turns it on
/// about the profile point `centre`, which stays where it was. `normal` is a flat frame sketch's
/// ([`frame::member_axes_on`]).
fn member_plane(a: Vec3, b: Vec3, turn: f64, at: Vec2, centre: Vec2, own: Option<f64>, normal: Option<Vec3>) -> Option<Plane> {
    let (x, y) = frame::member_axes_on(b - a, turn, normal)?;
    let origin = a - x * at.x - y * at.y;
    let Some(t) = own else { return Plane::new(origin, x, y) };
    let pivot = origin + x * centre.x + y * centre.y;
    let (x, y) = frame::member_axes_on(b - a, turn + t.rem_euclid(std::f64::consts::TAU), normal)?;
    Plane::new(pivot - x * centre.x - y * centre.y, x, y)
}

/// Member line `l` laid in `plane` (from [`member_plane`]) as the corners take it: the plane's axes,
/// and the profile point on the line (where the line's start is in the plane), exactly.
fn placed_on<'a>(l: &MemberLine, plane: &Plane, regions: &'a [Region2]) -> frame_corner::Placed<'a> {
    let d = l.a - plane.origin;
    frame_corner::Placed { a: l.a, b: l.b, x: plane.x, y: plane.y, at: Vec2::new(d.dot(plane.x), d.dot(plane.y)), regions }
}

/// The member of feature `f` on line `line`, as its body was made before its corners are cut: the
/// plane square to the line at its start (x × y along the line), its length, and its profile in
/// that plane.
pub(crate) fn member_section(
    doc: &Document,
    vals: &BTreeMap<String, Value>,
    f: &Feature,
    st: &ModelState,
    line: &str,
) -> Result<(Plane, f64, Region2)> {
    let prepared = prepare(doc, vals, f, st)?;
    let placed = prepared.placed()?;
    let k = prepared.segments.iter().position(|l| l.id == line).ok_or_else(|| DocError::Unknown(format!("line `{line}` of the member feature")))?;
    let (Some(l), Some(p)) = (prepared.segments.get(k), placed.get(k)) else {
        return Err(DocError::Unknown(format!("line `{line}` of the member feature")));
    };
    let plane =
        Plane::new(l.a - p.x * p.at.x - p.y * p.at.y, p.x, p.y).ok_or_else(|| DocError::Invalid(format!("line `{line}`: no plane square to it")))?;
    let region = p.regions.first().cloned().ok_or_else(|| DocError::Invalid("the member profile has no area".into()))?;
    Ok((plane, l.a.dist(l.b), region))
}

/// One end cut of a member as the cut list gives it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EndCut {
    /// How far the cut leans from square to the member (degrees; 0: a square end).
    pub angle: f64,
    /// Which way it leans, as a direction in the profile's own x and y (degrees from +x; 0 for a
    /// square end): the cut reaches furthest along the member on that side.
    pub towards: f64,
}

/// One member of a frame member feature as the cut list takes it, worked out from its line, its
/// section and the exact cut planes at its ends.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MemberCut {
    pub line: String,
    /// Its group (0: the feature's own fields).
    pub group: usize,
    /// Members with the same key have the same section (see the cut list's grouping).
    pub profile_key: String,
    pub description: String,
    /// The longest extent along its line (mm).
    pub length: f64,
    /// The shortest extent along its line (mm), what a saw marks for a mitre.
    pub short_length: f64,
    /// The cuts at its start and at its end, the most leaning first; none: a square end.
    pub ends: [Vec<EndCut>; 2],
    /// Whether its start and its end are coped (fitted round another member).
    pub coped: [bool; 2],
}

/// The members of frame member feature `f` as built in `st`: each one's lengths and end cuts, in
/// the order the feature makes its bodies.
pub fn member_cuts_report(doc: &Document, st: &ModelState, f: &Feature) -> Result<Vec<MemberCut>> {
    let FeatureKind::FrameMember { corners, corner_overrides, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let (vals, _) = doc.param_values();
    let prepared = prepare(doc, &vals, f, st)?;
    let placed = prepared.placed()?;
    let mut ignored = None;
    let cuts = corner_cuts(&vals, &prepared.segments, &prepared.group_of, &placed, corners, corner_overrides, &mut ignored)?;
    // The members' bodies, in order, and the trims made to them later in the timeline.
    let names: Vec<&str> = st.bodies.iter().filter(|b| b.feature == f.id).map(|b| b.name.as_str()).collect();
    let trims = super::weldment::later_trims(doc, &vals, st, f);
    let mut out = Vec::with_capacity(placed.len());
    for (k, (((l, p), c), g)) in prepared.segments.iter().zip(&placed).zip(&cuts).zip(&prepared.group_of).enumerate() {
        let name = names.get(k).copied().unwrap_or_default();
        let len = l.a.dist(l.b);
        let d = (l.b - l.a).normalized().ok_or_else(|| DocError::Invalid(format!("line `{}` has no length", l.id)))?;
        // Each end's planes; a square end is the plane square to the line at the end.
        let planes = |end: frame_corner::End| -> Vec<frame_corner::Cut> {
            let mine: Vec<frame_corner::Cut> = c.cuts.iter().filter(|(e, _, _)| *e == end).map(|(_, _, cut)| *cut).collect();
            if !mine.is_empty() {
                return mine;
            }
            match end {
                frame_corner::End::Start => vec![frame_corner::Cut { origin: l.a, normal: d }],
                frame_corner::End::End => vec![frame_corner::Cut { origin: l.b, normal: -d }],
            }
        };
        let (starts, ends) = (planes(frame_corner::End::Start), planes(frame_corner::End::End));
        // A coped end is shortest at the near face of what it fits round, not at the plane
        // through its middle.
        let near = |end: frame_corner::End, planes: &[frame_corner::Cut]| -> Vec<frame_corner::Cut> {
            let copes: Vec<&frame_corner::Cope> = c.copes.iter().filter(|k| k.end == end).collect();
            if copes.is_empty() {
                return planes.to_vec();
            }
            let mut out: Vec<frame_corner::Cut> =
                c.cuts.iter().filter(|(e, j, _)| *e == end && !copes.iter().any(|k| k.junction == *j)).map(|(_, _, cut)| *cut).collect();
            out.extend(copes.iter().map(|k| k.near));
            out
        };
        let (mut short_starts, mut short_ends) = (near(frame_corner::End::Start, &starts), near(frame_corner::End::End, &ends));
        // A trim's plane takes over its end; a cope by a body is measured on the body.
        let (mut starts, mut ends) = (starts, ends);
        let mut coped = [frame_corner::End::Start, frame_corner::End::End].map(|e| c.copes.iter().any(|x| x.end == e));
        let mut measured = false;
        for t in trims.iter().filter(|t| t.body == name) {
            let (long, short, cope) = match t.end {
                frame_corner::End::Start => (&mut starts, &mut short_starts, &mut coped[0]),
                frame_corner::End::End => (&mut ends, &mut short_ends, &mut coped[1]),
            };
            match t.cut {
                Some(cut) => {
                    *long = vec![cut];
                    *short = vec![cut];
                    *cope = false;
                }
                None => {
                    *cope = true;
                    measured = true;
                }
            }
        }
        // Along the line, a section point r (square to it) is kept from t_s(r) to t_e(r), each
        // affine in r: the length there is C + r·w, so its extremes are the section's reach.
        let (mut longest, mut shortest) = (f64::INFINITY, f64::INFINITY);
        let length_between = |s: &frame_corner::Cut, e: &frame_corner::Cut| -> Option<(f64, Vec3)> {
            let (ds, de) = (d.dot(s.normal), d.dot(e.normal));
            if ds.abs() < 1e-12 || de.abs() < 1e-12 {
                return None;
            }
            let c0 = (e.origin - l.a).dot(e.normal) / de - (s.origin - l.a).dot(s.normal) / ds;
            Some((c0, s.normal * (1.0 / ds) - e.normal * (1.0 / de)))
        };
        for s in &starts {
            for e in &ends {
                if let Some((c0, w)) = length_between(s, e) {
                    longest = longest.min(c0 + p.reach(w));
                }
            }
        }
        for s in &short_starts {
            for e in &short_ends {
                if let Some((c0, w)) = length_between(s, e) {
                    shortest = shortest.min(c0 - p.reach(-w));
                }
            }
        }
        if !longest.is_finite() || !shortest.is_finite() {
            longest = len;
            shortest = len;
        }
        if measured && let Some(b) = st.body(name) {
            longest = super::weldment::extent_along(&b.body, d)?;
            shortest = longest;
        }
        let end_cuts = |cuts: &[frame_corner::Cut], square: Vec3| -> Vec<EndCut> {
            let mut out: Vec<EndCut> = cuts
                .iter()
                .filter_map(|cut| {
                    let along = cut.normal.dot(square).clamp(-1.0, 1.0);
                    let angle = along.acos().to_degrees();
                    if angle < 1e-9 {
                        return None;
                    }
                    // The cut reaches furthest along the member where its normal leans away.
                    let lean = -(cut.normal - d * cut.normal.dot(d));
                    let towards = lean.dot(p.y).atan2(lean.dot(p.x)).to_degrees().rem_euclid(360.0) + 0.0;
                    Some(EndCut { angle, towards })
                })
                .collect();
            out.sort_by(|a, b| b.angle.total_cmp(&a.angle));
            out
        };
        let section = prepared.sections.get(*g);
        out.push(MemberCut {
            line: l.id.clone(),
            group: *g,
            profile_key: section.map(|s| s.key.clone()).unwrap_or_default(),
            description: section.map(|s| s.description.clone()).unwrap_or_default(),
            length: longest,
            short_length: shortest,
            ends: [end_cuts(&starts, d), end_cuts(&ends, -d)],
            coped,
        });
    }
    Ok(out)
}

/// A group's member settings: the feature's own fields or a `MemberGroup`'s.
struct GroupSpec<'a> {
    name: Option<&'a str>,
    profile: &'a MemberProfile,
    anchor: frame::Anchor,
    point: Option<&'a str>,
    /// Only the feature's own group: groups have no `locate`.
    locate: Option<[f64; 2]>,
    angle: Option<&'a crate::Expr>,
    mirror: bool,
}

impl<'a> GroupSpec<'a> {
    fn of(g: &'a frame::MemberGroup) -> GroupSpec<'a> {
        GroupSpec {
            name: g.name.as_deref(),
            profile: &g.profile,
            anchor: g.anchor,
            point: g.point.as_deref(),
            locate: None,
            angle: g.angle.as_ref(),
            mirror: g.mirror,
        }
    }
}

/// The member lines of a feature's groups and the group of each (0: the feature's own fields,
/// whose empty `lines` takes every line no other group has). A line in two groups, a group with
/// no lines and too many groups or members are refused.
pub(super) fn grouped_lines(ss: &SolvedSketch, lines: &[String], groups: &[frame::MemberGroup]) -> Result<(Vec<MemberLine>, Vec<usize>)> {
    if groups.is_empty() {
        let out = member_lines(ss, lines)?;
        let n = out.len();
        return Ok((out, vec![0; n]));
    }
    if groups.len() + 1 > frame::MAX_GROUPS {
        return Err(DocError::Invalid(format!("at most {} groups in one feature", frame::MAX_GROUPS)));
    }
    let mut owner: BTreeMap<&str, usize> = BTreeMap::new();
    for (k, g) in groups.iter().enumerate() {
        if g.lines.is_empty() {
            return Err(DocError::Invalid(format!("group {} (`{}`) has no lines", k + 2, g.id)));
        }
        for id in &g.lines {
            if let Some(other) = owner.insert(id.as_str(), k + 1) {
                return Err(DocError::Invalid(format!("line `{id}` is in group {} and group {}", other + 1, k + 2)));
            }
        }
    }
    for id in lines {
        if let Some(other) = owner.get(id.as_str()) {
            return Err(DocError::Invalid(format!("line `{id}` is in group 1 and group {}", other + 1)));
        }
    }
    let mut out = Vec::new();
    let mut group_of = Vec::new();
    let first: Vec<MemberLine> = if lines.is_empty() {
        let all = member_lines(ss, &[])?;
        all.into_iter().filter(|l| !owner.contains_key(l.id.as_str())).collect()
    } else {
        member_lines(ss, lines)?
    };
    group_of.extend(std::iter::repeat_n(0, first.len()));
    out.extend(first);
    for (k, g) in groups.iter().enumerate() {
        let these = member_lines(ss, &g.lines).map_err(|e| DocError::Invalid(format!("group {}: {e}", k + 2)))?;
        group_of.extend(std::iter::repeat_n(k + 1, these.len()));
        out.extend(these);
    }
    if out.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature")));
    }
    Ok((out, group_of))
}

/// The corner cuts of frame member feature `f`'s members as placed (`placed`, from `prepared`).
pub(crate) fn feature_cuts(
    vals: &BTreeMap<String, Value>,
    f: &Feature,
    prepared: &Prepared,
    placed: &[frame_corner::Placed],
) -> Result<Vec<MemberCuts>> {
    let FeatureKind::FrameMember { corners, corner_overrides, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    let mut ignored = None;
    corner_cuts(vals, &prepared.segments, &prepared.group_of, placed, corners, corner_overrides, &mut ignored)
}

/// Each member's corner cuts: none when the feature has no corner settings.
fn corner_cuts(
    vals: &BTreeMap<String, Value>,
    lines: &[MemberLine],
    group_of: &[usize],
    placed: &[frame_corner::Placed],
    corners: &CornerDefaults,
    overrides: &BTreeMap<String, CornerOverride>,
    warning: &mut Option<String>,
) -> Result<Vec<MemberCuts>> {
    if corners.is_none() && overrides.is_empty() {
        return Ok(vec![MemberCuts::default(); lines.len()]);
    }
    let gap = |label: &str, e: &Option<crate::Expr>| -> Result<Option<f64>> {
        let Some(e) = e else { return Ok(None) };
        let g = val(vals, e, Kind::Length)?;
        if !(g >= 0.0) || !g.is_finite() {
            return Err(DocError::Invalid(format!("{label} must not be negative")));
        }
        Ok(Some(g))
    };
    let gaps = (gap("the weld gap", &corners.gap_within)?.unwrap_or(0.0), gap("the weld gap between groups", &corners.gap_between)?.unwrap_or(0.0));
    let found = junctions_of(lines);
    let mut over = BTreeMap::new();
    let mut gone = Vec::new();
    for (id, o) in overrides {
        if found.iter().any(|j| j.id == *id) {
            over.insert(id.as_str(), (o, gap(&format!("the gap at junction `{id}`"), &o.gap)?));
        } else {
            gone.push(id.as_str());
        }
    }
    if !gone.is_empty() {
        let shown: Vec<String> = gone.iter().take(5).map(|g| format!("`{g}`")).collect();
        let more = if gone.len() > 5 { format!(" and {} more", gone.len() - 5) } else { String::new() };
        super::append_warning(
            warning,
            format!("the corner settings of junction {}{more} are kept but not used: no members meet there now", shown.join(", ")),
        );
    }
    frame_corner::member_cuts(placed, &found, |j| {
        let o = over.get(j.id.as_str()).map(|(o, g)| (*o, *g));
        frame_corner::settings(j, corners, gaps, o, lines, group_of)
    })
}

/// One junction of a frame member feature as it builds: where, which member ends meet there
/// (by line id), and what it gets.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct JunctionReport {
    pub id: String,
    pub kind: frame_corner::JunctionKind,
    pub at: [f64; 3],
    pub ends: Vec<(String, frame_corner::End)>,
    /// Each of `ends`' unit direction from the junction into its member.
    #[serde(skip)]
    pub inward: Vec<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<String>,
    pub treatment: frame_corner::Treatment,
    pub gap: f64,
    pub butt_cut: frame_corner::ButtCut,
    /// The line that runs through at a butt where two or more ends meet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub through: Option<String>,
    /// It has settings of its own (`corner_overrides`).
    pub overridden: bool,
    /// Members of different groups meet here (the between-groups settings apply).
    pub between: bool,
}

/// The junctions of frame member feature `f`, in its component's coordinates, with what each gets.
pub fn feature_junctions(doc: &Document, st: &ModelState, f: &Feature) -> Result<Vec<JunctionReport>> {
    let FeatureKind::FrameMember { path_sketch, lines, groups, corners, corner_overrides, .. } = &f.kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    junctions_on(doc, st, path_sketch.0, (lines, groups), corners, corner_overrides, f.component)
}

/// The junctions frame members on the lines of sketch `sketch` (the first group's `lines`, none: all
/// lines no other group has, and the further `groups`) would have in
/// component `component`'s coordinates, with what each gets from `corners` and `overrides`: what
/// Frame Member's corner list shows before the feature is made.
pub fn junctions_on(
    doc: &Document,
    st: &ModelState,
    sketch: u64,
    (lines, groups): (&[String], &[frame::MemberGroup]),
    corners: &CornerDefaults,
    overrides: &BTreeMap<String, CornerOverride>,
    component: u64,
) -> Result<Vec<JunctionReport>> {
    let ps = st.sketch(sketch).ok_or_else(|| DocError::Unknown(format!("sketch {sketch}")))?;
    let mapped =
        if ps.sketch.wires.is_empty() { Cow::Owned(sketch_in_feature_frame(doc, ps, component)?) } else { path_in_frame(doc, ps, component)? };
    let (segments, group_of) = grouped_lines(mapped.as_ref(), lines, groups)?;
    let (vals, _) = doc.param_values();
    let gap = |e: &Option<crate::Expr>| e.as_ref().and_then(|e| val(&vals, e, Kind::Length).ok()).filter(|g| *g >= 0.0);
    let gaps = (gap(&corners.gap_within).unwrap_or(0.0), gap(&corners.gap_between).unwrap_or(0.0));
    let line_id = |i: usize| segments.get(i).map(|l| l.id.clone()).unwrap_or_default();
    Ok(junctions_of(&segments)
        .into_iter()
        .map(|j| {
            let over = overrides.get(&j.id);
            let set = frame_corner::settings(&j, corners, gaps, over.map(|o| (o, gap(&o.gap))), &segments, &group_of);
            let group = |i: usize| group_of.get(i).copied().unwrap_or(0);
            let first = j.ends.first().map_or(0, |e| group(e.0));
            let between = j.ends.iter().map(|e| e.0).chain(j.span).any(|i| group(i) != first);
            JunctionReport {
                id: j.id.clone(),
                kind: j.kind,
                at: [j.at.x, j.at.y, j.at.z],
                ends: j.ends.iter().map(|(i, e)| (line_id(*i), *e)).collect(),
                inward: j.ends.iter().map(|(i, e)| segments.get(*i).and_then(|l| l.inward(*e)).map_or([0.0; 3], |u| [u.x, u.y, u.z])).collect(),
                span: j.span.map(line_id),
                treatment: set.treatment,
                gap: set.gap,
                butt_cut: set.butt_cut,
                through: (set.treatment.runs_through() && matches!(j.kind, frame_corner::JunctionKind::L | frame_corner::JunctionKind::Multi))
                    .then(|| set.rank.first().map(|i| line_id(*i)))
                    .flatten(),
                overridden: over.is_some(),
                between,
            }
        })
        .collect())
}

/// The junctions of a feature's member lines, matching ends within a tolerance of the frame's size.
pub(super) fn junctions_of(lines: &[MemberLine]) -> Vec<frame_corner::Junction> {
    let size = lines.iter().flat_map(|l| [l.a, l.b]).map(|p| p.x.abs().max(p.y.abs()).max(p.z.abs())).fold(1.0_f64, f64::max);
    frame_corner::junctions(lines, frame_corner::JOIN_TOL * size)
}

/// The point member `line` of `f` turns about, halfway along it: the centre of the profile's
/// bounding box, which a Rotate step's turn leaves where it is ([`member_plane`]).
pub(crate) fn member_pivot(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState, line: &str) -> Result<Vec3> {
    let Prepared { segments, group_of, sections, normal, .. } = prepare(doc, vals, f, st)?;
    let k = segments.iter().position(|l| l.id == line).ok_or_else(|| DocError::Unknown(format!("line `{line}` of the member feature")))?;
    let (l, g) = (&segments[k], group_of.get(k).copied().unwrap_or(0));
    let s = sections.get(g).ok_or_else(|| DocError::Invalid(format!("line `{line}`: no group {g}")))?;
    let plane = member_plane(l.a, l.b, s.turn, s.at, s.centre, None, normal)
        .ok_or_else(|| DocError::Invalid(format!("line `{line}`: no plane square to it")))?;
    Ok(plane.origin + plane.x * s.centre.x + plane.y * s.centre.y + (l.b - l.a) * 0.5)
}

/// The body name of the member on each of `lines`: its own name from `pinned` (by line id) when no
/// other body has it, else the first free one of `base`, `base (2)`, `base (3)`… that no line has
/// pinned. A member therefore keeps its name when other lines are added, deleted or reordered.
pub fn member_names(st: &ModelState, lines: &[String], pinned: &BTreeMap<String, String>, base: &str) -> Vec<String> {
    member_names_reserving(st, lines, pinned, &BTreeMap::new(), std::iter::empty(), base)
}

/// The body name of the member on each of `lines`, whose groups `group_of` gives: as
/// [`member_names`] for each group with its `bases` entry, a name `pinned` to a line of another
/// group reserved for it, so names hold across groups too.
pub fn grouped_member_names(st: &ModelState, lines: &[String], group_of: &[usize], bases: &[&str], pinned: &BTreeMap<String, String>) -> Vec<String> {
    let mut taken: Vec<Option<String>> = vec![None; lines.len()];
    for (k, base) in bases.iter().enumerate() {
        let picked: Vec<usize> = (0..lines.len()).filter(|i| group_of.get(*i) == Some(&k)).collect();
        let ids: Vec<String> = picked.iter().filter_map(|i| lines.get(*i).cloned()).collect();
        let (mine, others): (BTreeMap<String, String>, BTreeMap<String, String>) =
            pinned.iter().map(|(l, n)| (l.clone(), n.clone())).partition(|(l, _)| ids.contains(l));
        let used: Vec<String> = taken.iter().flatten().cloned().collect();
        for (i, name) in picked.into_iter().zip(member_names_reserving(st, &ids, &mine, &others, used.iter(), base)) {
            if let Some(slot) = taken.get_mut(i) {
                *slot = Some(name);
            }
        }
    }
    taken.into_iter().map(Option::unwrap_or_default).collect()
}

/// The member lines of sketch `sketch` for the first group's `lines` and further `groups`, each
/// with its group (0: the first).
pub fn line_groups_on(st: &ModelState, sketch: u64, lines: &[String], groups: &[frame::MemberGroup]) -> Result<(Vec<String>, Vec<usize>)> {
    let ps = st.sketch(sketch).ok_or_else(|| DocError::Unknown(format!("sketch {sketch}")))?;
    let (out, group_of) = grouped_lines(ps, lines, groups)?;
    Ok((out.into_iter().map(|l| l.id).collect(), group_of))
}

/// [`member_names`] for one group of a feature: `others` (the other groups' pinned names) are
/// reserved too, and `used` (names given to members already) taken.
fn member_names_reserving<'a>(
    st: &ModelState,
    lines: &[String],
    pinned: &BTreeMap<String, String>,
    others: &BTreeMap<String, String>,
    used: impl Iterator<Item = &'a String>,
    base: &str,
) -> Vec<String> {
    let reserved: HashSet<&str> = pinned.values().chain(others.values()).map(String::as_str).collect();
    let mut used: HashSet<String> = used.cloned().collect();
    let mut out: Vec<Option<String>> = lines
        .iter()
        .map(|line| {
            let n = pinned.get(line).map(|n| n.trim()).filter(|n| !n.is_empty() && st.body(n).is_none())?;
            (!others.values().any(|o| o == n) && used.insert(n.to_string())).then(|| n.to_string())
        })
        .collect();
    let base = if base.trim().is_empty() { "Member" } else { base.trim() };
    // Each candidate is refused at most once per body, used or reserved name: past that many,
    // a free one has been found for every member.
    let limit = st.bodies.len() + reserved.len() + lines.len() + 2;
    let mut next = 1usize;
    for slot in out.iter_mut().filter(|n| n.is_none()) {
        while next <= limit {
            let candidate = if next == 1 { base.to_string() } else { format!("{base} ({next})") };
            next += 1;
            if st.body(&candidate).is_none() && !used.contains(&candidate) && !reserved.contains(candidate.as_str()) {
                used.insert(candidate.clone());
                *slot = Some(candidate);
                break;
            }
        }
    }
    out.into_iter().map(|n| n.unwrap_or_else(|| base.to_string())).collect()
}

/// The angle each line of the frame member `frame` is turned to by active Rotate steps after
/// it: line id to the angle's expression and its value (radians), a later step replacing an
/// earlier one. A step whose angle doesn't evaluate with `vals` turns nothing (it reports why
/// itself, `check_turn`): an earlier step for the line, if any, stands.
pub fn member_turns(doc: &Document, vals: &BTreeMap<String, Value>, frame: FeatureId) -> BTreeMap<String, (Expr, f64)> {
    turn_steps(doc, frame).filter_map(|(line, e)| Some((line.to_string(), (e.clone(), turn_angle(vals, e).ok()?)))).collect()
}

/// The active Rotate steps of the frame member `frame`, in timeline order: each one's line id and
/// angle, whether it evaluates or not. Active: after the frame, before the marker, not suppressed.
pub(super) fn turn_steps(doc: &Document, frame: FeatureId) -> impl Iterator<Item = (&str, &Expr)> {
    let at = doc.feature_index(frame.0);
    let end = doc.marker.unwrap_or(doc.features.len()).min(doc.features.len());
    let after = at.map_or(end, |at| at.saturating_add(1));
    doc.features.iter().take(end).skip(after).filter_map(move |f| match &f.kind {
        FeatureKind::FrameMemberTurn { member, angle } if !f.suppressed && member.feature == frame => Some((member.line.as_str(), angle)),
        _ => None,
    })
}

/// A Rotate step's angle (radians), refused unless it evaluates to a finite number.
fn turn_angle(vals: &BTreeMap<String, Value>, angle: &Expr) -> Result<f64> {
    let a = val(vals, angle, Kind::Angle)?;
    if !a.is_finite() {
        return Err(DocError::Invalid("The angle must be a finite number".into()));
    }
    Ok(a)
}

/// A Rotate step's checks: its frame member is there, earlier, and still has the line, and its
/// angle evaluates.
pub(super) fn check_turn(doc: &Document, vals: &BTreeMap<String, Value>, f: &Feature, st: &ModelState) -> Result<()> {
    let FeatureKind::FrameMemberTurn { member, angle } = &f.kind else {
        return Err(DocError::Invalid("not a rotate member feature".into()));
    };
    let gone = || DocError::Invalid("Its frame member is gone".into());
    let source = doc.feature(member.feature.0).filter(|m| !m.suppressed && matches!(m.kind, FeatureKind::FrameMember { .. })).ok_or_else(gone)?;
    let (Some(from), Some(here)) = (doc.feature_index(source.id), doc.feature_index(f.id)) else { return Err(gone()) };
    if from >= here {
        return Err(DocError::Invalid("Its frame member must come earlier in the timeline".into()));
    }
    if !member_line_ids(st, &source.kind)?.contains(&member.line) {
        return Err(DocError::Invalid(format!("Line `{}` of its frame member is gone", member.line)));
    }
    turn_angle(vals, angle).map(|_| ())
}

/// The ids of the lines of sketch `sketch` members would be made on (`lines`, or none: every
/// straight line), in the order they are made.
pub fn line_ids_on(st: &ModelState, sketch: u64, lines: &[String], groups: &[frame::MemberGroup]) -> Result<Vec<String>> {
    let ps = st.sketch(sketch).ok_or_else(|| DocError::Unknown(format!("sketch {sketch}")))?;
    Ok(grouped_lines(ps, lines, groups)?.0.into_iter().map(|l| l.id).collect())
}

/// The ids of a `FrameMember` feature's lines in `st`, in the order its members are made.
pub fn member_line_ids(st: &ModelState, kind: &FeatureKind) -> Result<Vec<String>> {
    let FeatureKind::FrameMember { path_sketch, lines, groups, .. } = kind else {
        return Err(DocError::Invalid("not a frame member feature".into()));
    };
    line_ids_on(st, path_sketch.0, lines, groups)
}

/// The section of profile sketch `sketch` as a member would take it (its material, or the
/// `profiles` picked), in the sketch's own x and y: one piece of material, holes and all. This is
/// what a library profile copies when it is made from a sketch.
pub fn sketch_section(doc: &Document, st: &ModelState, sketch: crate::SketchId, profiles: &ProfileSel) -> Result<Vec<Region2>> {
    let profile = MemberProfile::Sketch { sketch, profiles: profiles.clone() };
    Ok(profile_regions(doc, &BTreeMap::new(), st, &profile)?.regions)
}

/// Where point `id` of sketch `sketch` is, in the sketch's own x and y.
pub fn sketch_point(st: &ModelState, sketch: crate::SketchId, id: &str) -> Option<Vec2> {
    let ss = st.sketch(sketch.0)?;
    ss.sketch.point(ss.sketch.point_index(id)?)
}

/// The member lines of the frame sketch, each as (curve id, start, end) in its frame: the lines
/// asked for, or every straight line drawn in it (not construction, not projected or otherwise
/// linked, and with a length).
pub(super) fn member_lines(ss: &SolvedSketch, ids: &[String]) -> Result<Vec<MemberLine>> {
    let sk = &ss.sketch;
    let mut out = Vec::new();
    if ids.is_empty() {
        for (ci, c) in sk.curves.iter().enumerate() {
            if !c.construction
                && !c.centerline
                && c.link.is_none()
                && matches!(c.kind, CurveKind::Line { .. })
                && let Some((a, b, ends)) = planar_line(ss, ci)
                && a.dist(b) > MIN_LENGTH
            {
                out.push(MemberLine { id: c.id.clone(), a, b, ends });
            }
        }
        for w in sk.wires.iter().filter(|w| w.link.is_none()) {
            if let Some((a, b)) = straight(&w.pts)
                && a.dist(b) > MIN_LENGTH
            {
                out.push(MemberLine { id: w.id.clone(), a, b, ends: wire_ends(&w.id) });
            }
        }
        if out.is_empty() {
            return Err(DocError::Invalid(format!("{} has no lines for members", ss.name)));
        }
        if out.len() > MAX_MEMBERS {
            return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature (pick the lines)")));
        }
        return Ok(out);
    }
    if ids.len() > MAX_MEMBERS {
        return Err(DocError::Invalid(format!("at most {MAX_MEMBERS} members in one feature")));
    }
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) {
            return Err(DocError::Invalid(format!("line `{id}` is listed twice")));
        }
        let line = if let Some(wi) = sk.wire_index(id) {
            sk.wires.get(wi).and_then(|w| straight(&w.pts)).map(|(a, b)| (a, b, wire_ends(id)))
        } else {
            let ci = sk.curve_index(id).ok_or_else(|| DocError::Unknown(format!("line `{id}` in {}", ss.name)))?;
            planar_line(ss, ci)
        };
        let (a, b, ends) = line.ok_or_else(|| DocError::Invalid(format!("`{id}` is not a straight line (members run along lines)")))?;
        if !(a.dist(b) > MIN_LENGTH) {
            return Err(DocError::Invalid(format!("line `{id}` has no length")));
        }
        out.push(MemberLine { id: id.clone(), a, b, ends });
    }
    Ok(out)
}

/// A sketch line's ends in the sketch's frame, and their point ids.
fn planar_line(ss: &SolvedSketch, ci: usize) -> Option<(Vec3, Vec3, [String; 2])> {
    let CurveKind::Line { a, b } = ss.sketch.curves.get(ci)?.kind else { return None };
    let (pa, pb) = (ss.sketch.points.get(a)?, ss.sketch.points.get(b)?);
    Some((ss.plane.to_world(pa.pos), ss.plane.to_world(pb.pos), [pa.id.clone(), pb.id.clone()]))
}

/// A 3D sketch line's end ids: it has no end points of its own.
fn wire_ends(id: &str) -> [String; 2] {
    [format!("{id}.start"), format!("{id}.end")]
}

/// The ends of a 3D polyline that runs straight from its first point to its last: every point
/// within [`STRAIGHT_TOL`] of the line between them, none behind the start, past the end or back
/// from a point before it.
fn straight(pts: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let (a, b) = (*pts.first()?, *pts.last()?);
    let (len, d) = (a.dist(b), (b - a).normalized()?);
    let mut reached = 0.0_f64;
    for p in pts {
        let q = *p - a;
        let t = q.dot(d);
        if (q - d * t).len() > STRAIGHT_TOL || t < reached - STRAIGHT_TOL || t > len + STRAIGHT_TOL {
            return None;
        }
        reached = reached.max(t);
    }
    Some((a, b))
}

/// A section as Frame Member's preview draws it: its outlines as polylines (outer loops and holes,
/// in the profile's own coordinates), its placement points by name and its default point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionPreview {
    pub outlines: Vec<Vec<Vec2>>,
    pub points: Vec<(String, Vec2)>,
    pub default_point: Option<String>,
}

impl SectionPreview {
    fn of(regions: &[Region2], points: Vec<(String, Vec2)>, default_point: Option<String>) -> SectionPreview {
        let size = regions.iter().flat_map(|r| r.outer.polyline(1.0).to_vec()).map(|p| p.x.abs().max(p.y.abs())).fold(1.0_f64, f64::max);
        let outlines = regions.iter().flat_map(Region2::loops).map(|l| l.polyline(size * 1e-3).to_vec()).collect();
        SectionPreview { outlines, points, default_point }
    }
}

/// Profile `profile`'s section as a member takes it before mirroring and placing, worked out with
/// the design's parameters.
pub fn section_preview(doc: &Document, st: &ModelState, profile: &MemberProfile) -> Result<SectionPreview> {
    let (vals, _) = doc.param_values();
    let s = profile_regions(doc, &vals, st, profile)?;
    Ok(SectionPreview::of(&s.regions, s.points, s.default_point))
}

/// Library entry `entry` (the design's or a personal one) worked out with the design's
/// parameters.
pub fn library_entry_preview(doc: &Document, entry: &frame_library::LibraryProfile) -> Result<SectionPreview> {
    let (vals, _) = doc.param_values();
    let r = entry.resolve(|label, e, kind| val(&vals, e, kind).map_err(|err| DocError::Invalid(format!("profile `{label}`: {err}"))))?;
    Ok(SectionPreview::of(&r.regions, r.points, entry.default_point.clone()))
}

/// A member's section before it is placed on a line: its regions in the profile's own
/// coordinates, its placement points, the point a library profile starts on, what its members
/// are named after when they have no name of their own, and the profile's own origin (the point
/// a built-in profile's dimensions are measured from, which `locate` is measured from).
pub(super) struct MemberSection {
    pub regions: Vec<Region2>,
    pub points: Vec<(String, Vec2)>,
    pub default_point: Option<String>,
    pub description: String,
    pub origin: Vec2,
}

/// The section's regions as they are placed (mirrored across the profile's y axis when asked), and
/// the point of them that sits on the line: the placement point named `point`; else the `locate`d
/// point, measured from the profile's origin; else `anchor` when it isn't the origin; else a library
/// profile's default point; else the origin. A mirrored profile's bounding-box points are those of
/// the mirrored outline (its `top_left` is still at the top left); the points its kind generates or
/// its entry names move with the material (its `heel` stays the heel).
pub(super) fn placed_section(
    section: MemberSection,
    anchor: frame::Anchor,
    point: Option<&str>,
    locate: Option<[f64; 2]>,
    mirror: bool,
) -> Result<(Vec<Region2>, Vec2)> {
    let MemberSection { regions, points, default_point, origin, .. } = section;
    let (regions, points) = if mirror {
        let flipped: Vec<Region2> = regions.iter().map(frame::mirrored).collect();
        let mut pts = frame_library::placement_points(&flipped, None, &[]);
        pts.extend(points.into_iter().filter(|(n, _)| !frame_library::COMMON_POINTS.contains(&n.as_str())).map(|(n, v)| (n, Vec2::new(-v.x, v.y))));
        (flipped, pts)
    } else {
        (regions, points)
    };
    let at = match (point, locate, default_point) {
        (Some(name), _, _) => frame_library::find_point(&points, name)?,
        (None, Some([x, y]), _) => {
            // A design file is hostile input: the command checks the point, a file may not.
            if !(x.is_finite() && y.is_finite() && x.abs() <= frame::MAX_PROFILE_SIZE && y.abs() <= frame::MAX_PROFILE_SIZE) {
                return Err(DocError::Invalid(format!("frame member `locate` must be two finite lengths within {} mm", frame::MAX_PROFILE_SIZE)));
            }
            // Measured from the profile's own origin (`Profile::origin`): the built-in region is
            // centred on its bounding box, so that origin moves when the section's size changes,
            // and the located point moves with it.
            let p = origin + Vec2::new(x, y);
            if mirror { Vec2::new(-p.x, p.y) } else { p }
        }
        (None, None, _) if !anchor.is_origin() => frame::anchor_point(&regions, anchor),
        (None, None, Some(name)) => frame_library::find_point(&points, &name)?,
        (None, None, None) => Vec2::ZERO,
    };
    Ok((regions, at))
}

/// The section of a member's profile, in the profile's own coordinates: one piece of material,
/// holes and all.
pub(super) fn profile_regions(doc: &Document, vals: &BTreeMap<String, Value>, st: &ModelState, profile: &MemberProfile) -> Result<MemberSection> {
    let (regions, description) = match profile {
        MemberProfile::Library { profile: lp } => {
            let p = lp.resolve(|name, e, kind| val(vals, e, kind).map_err(|err| DocError::Invalid(format!("profile `{name}`: {err}"))))?;
            let regions = vec![p.region()?];
            let points = frame_library::placement_points(&regions, Some(&p), &[]);
            let description = lp.size.clone().unwrap_or_else(|| p.description());
            return Ok(MemberSection { regions, points, default_point: None, description, origin: p.origin() });
        }
        MemberProfile::LibraryProfile { id } => {
            let entry = doc
                .library_profile(id)
                .ok_or_else(|| DocError::Unknown(format!("library profile `{id}` (it was removed from the profile library)")))?;
            let r = entry
                .resolve(|label, e, kind| val(vals, e, kind).map_err(|err| DocError::Invalid(format!("profile `{label}`: {err}"))))
                .map_err(|e| DocError::Invalid(format!("library profile {}: {e}", entry.name)))?;
            if r.regions.len() != 1 {
                return Err(DocError::Invalid(format!("library profile {} is {} separate pieces", entry.name, r.regions.len())));
            }
            return Ok(MemberSection {
                regions: r.regions,
                points: r.points,
                default_point: entry.default_point.clone(),
                description: entry.name.clone(),
                origin: Vec2::ZERO,
            });
        }
        MemberProfile::Sketch { sketch, profiles } => {
            let ss = st.sketch(sketch.0).ok_or_else(|| DocError::Unknown(format!("sketch {sketch} (it must come earlier in the timeline)")))?;
            let picked = match profiles {
                ProfileSel::All => material(ss)?,
                sel => select_profiles(ss, sel)?,
            };
            (solidcraft_sketch::merge_regions(&picked), ss.name.clone())
        }
    };
    match regions.len() {
        1 => {
            let points = frame_library::placement_points(&regions, None, &[]);
            Ok(MemberSection { regions, points, default_point: None, description, origin: Vec2::ZERO })
        }
        0 => Err(DocError::Invalid("the member profile has no area".into())),
        n => Err(DocError::Invalid(format!("the member profile is {n} separate pieces (pick one with `profiles`)"))),
    }
}

/// The material of a profile sketch: of what was drawn (as `solid.extrude` takes all profiles:
/// projected reference geometry only when the drawn curves make no profile, letters without
/// their counters filled in), each profile inside an even number of other profiles' outlines,
/// so nested loops alternate between material and hole.
fn material(ss: &SolvedSketch) -> Result<Vec<Region2>> {
    let drawn = solidcraft_sketch::find_drawn_profiles(&ss.sketch);
    let ps = solidcraft_sketch::without_text_counters(&ss.sketch, if drawn.is_empty() { ss.profiles.clone() } else { drawn });
    if ps.is_empty() {
        return Err(DocError::Invalid(format!("{} has no closed profiles", ss.name)));
    }
    if ps.len() > MAX_PROFILE_PIECES {
        return Err(DocError::Invalid(format!("{} has more than {MAX_PROFILE_PIECES} profiles (pick the member's with `profiles`)", ss.name)));
    }
    // Outer polylines and their boxes, once.
    let polys: Vec<Ring<Vec2>> = ps.iter().map(|p| p.region.outer.polyline(1e-3)).collect();
    let boxes: Vec<Aabb2> = polys.iter().map(|poly| poly.bounds()).collect();
    Ok(ps
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            let at = p.region.interior_point();
            let depth = polys.iter().zip(&boxes).enumerate().filter(|(j, (poly, bb))| j != i && bb.contains(at) && poly.contains(at)).count();
            depth % 2 == 0
        })
        .map(|(_, p)| p.region.clone())
        .collect())
}
