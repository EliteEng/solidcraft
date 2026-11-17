//! Frame member corners (docs/design/weldments.md, section 3): where a feature's members meet
//! (its junctions), what is done there (mitre, butt or none, with a weld gap), and the cut planes
//! that does it, from the members' exact lines and sections. No mesh is used: every cut is a plane
//! worked out from the line directions and the sections' exact extents.

use serde::{Deserialize, Serialize};
use solidcraft_geom::prelude::*;
use std::collections::BTreeMap;

use solidcore_record::{Region2, Seg2, Vec2, Vec3};

use crate::{DocError, Expr, Result};

/// Two member ends this far apart (mm) meet; an end this near another member's line lands on it.
pub const JOIN_TOL: f64 = 1e-6;
/// Two members whose lines are within this angle (radians) of each other are collinear.
pub const COLLINEAR_TOL: f64 = 1e-6;
/// Longest extension (mm) an end may need to reach its cut: past this the corner is too near
/// parallel for its cut to be made.
pub const MAX_EXTENSION: f64 = 1e6;
/// Most corner overrides one feature may have.
pub const MAX_OVERRIDES: usize = 4000;

/// What is done where members meet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    /// Square ends at the line ends: members overlap at a corner, as before corners existed.
    #[default]
    None,
    /// Both members cut on the plane that mirrors one onto the other (two ends meeting at an
    /// angle); elsewhere it is a butt.
    Mitre,
    /// One member runs through; the other stops at its face.
    Butt,
}

impl Treatment {
    pub fn is_none(&self) -> bool {
        *self == Treatment::None
    }
}

/// How a butting member is cut where it meets the member it butts against at an angle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtCut {
    /// Square to the butting member, stopping where it first touches the other's face.
    #[default]
    Square,
    /// On the face of the member it butts against, at an angle.
    Flush,
}

impl ButtCut {
    pub fn is_square(&self) -> bool {
        *self == ButtCut::Square
    }
}

/// Which member runs through at a butt where nothing says which: the longer or the shorter one.
/// Equal lengths go by direction (the one nearer the X axis, then Y, counts as longer), so a
/// rectangle's or a square's corners all go the same way and the other choice swaps them all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtThrough {
    #[default]
    Longer,
    Shorter,
}

impl ButtThrough {
    pub fn is_longer(&self) -> bool {
        *self == ButtThrough::Longer
    }
}

/// A feature's corner settings: what every junction gets unless an override says otherwise.
/// The default (what a design saved before corners existed reads) is no treatment, so it rebuilds
/// as it did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, solidcraft_derive::Walk)]
pub struct CornerDefaults {
    /// Where two ends meet at an angle (an L). Ends landing on another member (a T), three or
    /// more ends at one point, and two collinear ends butt unless this is `none`.
    #[serde(default, skip_serializing_if = "Treatment::is_none")]
    pub within_group: Treatment,
    /// The weld gap left at each junction. None: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[walk(length = "Weld gap")]
    pub gap_within: Option<Expr>,
    #[serde(default, skip_serializing_if = "ButtCut::is_square")]
    pub butt_cut: ButtCut,
    #[serde(default, skip_serializing_if = "ButtThrough::is_longer")]
    pub butt_through: ButtThrough,
}

impl CornerDefaults {
    pub fn is_none(&self) -> bool {
        *self == CornerDefaults::default()
    }

    /// New members' defaults: mitred corners, no gap.
    pub fn mitre() -> CornerDefaults {
        CornerDefaults { within_group: Treatment::Mitre, ..CornerDefaults::default() }
    }
}

/// One junction's own settings, each in place of the feature's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, solidcraft_derive::Walk)]
#[serde(deny_unknown_fields)]
pub struct CornerOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<Treatment>,
    /// The line id of the member that runs through (a butt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[walk(length = "Corner gap")]
    pub gap: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub butt_cut: Option<ButtCut>,
}

/// Which end of a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Start,
    End,
}

/// What kind of junction members make.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum JunctionKind {
    /// Two ends at a point, at an angle.
    L,
    /// Two ends at a point, in line.
    I,
    /// Ends landing on another member between its ends.
    T,
    /// Three or more ends at a point.
    Multi,
}

/// A member's line as junctions are found from it: its id, its ends, and each end's id (its
/// sketch point's id, or `<line>.start`/`<line>.end` for a 3D sketch line).
#[derive(Clone, Debug, PartialEq)]
pub struct MemberLine {
    pub id: String,
    pub a: Vec3,
    pub b: Vec3,
    pub ends: [String; 2],
}

impl MemberLine {
    fn at(&self, e: End) -> Vec3 {
        match e {
            End::Start => self.a,
            End::End => self.b,
        }
    }

    /// The unit direction from end `e` into the member.
    pub fn inward(&self, e: End) -> Option<Vec3> {
        match e {
            End::Start => (self.b - self.a).normalized(),
            End::End => (self.a - self.b).normalized(),
        }
    }
}

/// Where members meet.
#[derive(Clone, Debug, PartialEq)]
pub struct Junction {
    /// Stable through sketch edits: the shared end point's id (`p12`), or for a T the end point and
    /// the line it lands on (`p12@l7`).
    pub id: String,
    pub kind: JunctionKind,
    pub at: Vec3,
    /// The member ends that meet here, by member index, in member order.
    pub ends: Vec<(usize, End)>,
    /// For a T, the member the ends land on.
    pub span: Option<usize>,
}

/// The junctions of `lines`, sorted by id. Lines that cross between their ends are no junction.
pub fn junctions(lines: &[MemberLine], tol: f64) -> Vec<Junction> {
    let ends: Vec<(usize, End)> = (0..lines.len()).flat_map(|i| [(i, End::Start), (i, End::End)]).collect();
    let pos = |(i, e): (usize, End)| lines.get(i).map(|l| l.at(e));
    let end_id = |(i, e): (usize, End)| lines.get(i).and_then(|l| l.ends.get(e as usize).cloned()).unwrap_or_default();
    // Ends landing on another member's span.
    let mut on_span: Vec<Option<usize>> = vec![None; ends.len()];
    for (k, &end) in ends.iter().enumerate() {
        let Some(p) = pos(end) else { continue };
        on_span[k] = lines.iter().enumerate().position(|(j, l)| {
            if j == end.0 {
                return false;
            }
            let len = l.a.dist(l.b);
            let Some(d) = (l.b - l.a).normalized() else { return false };
            let t = (p - l.a).dot(d);
            t > tol && t < len - tol && (p - l.a - d * t).len() <= tol
        });
    }
    let mut out = Vec::new();
    // T junctions: the ends on one span at one point.
    let mut done = vec![false; ends.len()];
    for k in 0..ends.len() {
        let (Some(span), Some(&end)) = (on_span.get(k).copied().flatten(), ends.get(k)) else { continue };
        if done.get(k).copied().unwrap_or(true) {
            continue;
        }
        let Some(p) = pos(end) else { continue };
        let mut group = vec![end];
        for m in k + 1..ends.len() {
            if let (Some(Some(s)), Some(&other)) = (on_span.get(m), ends.get(m))
                && *s == span
                && pos(other).is_some_and(|q| q.dist(p) <= tol)
            {
                group.push(other);
                if let Some(d) = done.get_mut(m) {
                    *d = true;
                }
            }
        }
        let first = group.iter().map(|e| end_id(*e)).min().unwrap_or_default();
        let line = lines.get(span).map(|l| l.id.clone()).unwrap_or_default();
        group.sort();
        out.push(Junction { id: format!("{first}@{line}"), kind: JunctionKind::T, at: p, ends: group, span: Some(span) });
    }
    // The other ends, joined where they share a point id or a position.
    let free: Vec<usize> = (0..ends.len()).filter(|k| on_span.get(*k).is_some_and(Option::is_none)).collect();
    let mut cluster: Vec<usize> = (0..ends.len()).collect();
    fn root(c: &mut [usize], mut k: usize) -> usize {
        while let Some(&up) = c.get(k) {
            if up == k {
                break;
            }
            k = up;
        }
        k
    }
    for (n, &k) in free.iter().enumerate() {
        for &m in free.iter().skip(n + 1) {
            let (Some(&ek), Some(&em)) = (ends.get(k), ends.get(m)) else { continue };
            let same = end_id(ek) == end_id(em) && !end_id(ek).is_empty();
            let near = matches!((pos(ek), pos(em)), (Some(p), Some(q)) if p.dist(q) <= tol);
            if same || near {
                let (rk, rm) = (root(&mut cluster, k), root(&mut cluster, m));
                if let Some(c) = cluster.get_mut(rk.max(rm)) {
                    *c = rk.min(rm);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<(usize, End)>> = BTreeMap::new();
    for &k in &free {
        if let Some(&end) = ends.get(k) {
            groups.entry(root(&mut cluster, k)).or_default().push(end);
        }
    }
    for (_, mut group) in groups {
        if group.len() < 2 {
            continue;
        }
        group.sort();
        let id = group.iter().map(|e| end_id(*e)).min().unwrap_or_default();
        let at = group.first().and_then(|e| pos(*e)).unwrap_or(Vec3::ZERO);
        let kind = match group.as_slice() {
            [p, q] => {
                let dirs = (lines.get(p.0).and_then(|l| l.inward(p.1)), lines.get(q.0).and_then(|l| l.inward(q.1)));
                match dirs {
                    (Some(u), Some(v)) if u.dot(v) <= -COLLINEAR_TOL.cos() => JunctionKind::I,
                    _ => JunctionKind::L,
                }
            }
            _ => JunctionKind::Multi,
        };
        out.push(Junction { id, kind, at, ends: group, span: None });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// A member as placed: its line, the section's axes square to it, the section's point on the line,
/// and the section in its own coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Placed<'a> {
    pub a: Vec3,
    pub b: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub at: Vec2,
    pub regions: &'a [Region2],
}

impl Placed<'_> {
    /// How far the section reaches from the line in direction `n` (the largest `r·n` over its
    /// material, `r` measured square to the line from it): exact for lines, arcs and cubic
    /// segments, the control polygon's reach (never less than the curve's) for other curves.
    pub fn reach(&self, n: Vec3) -> f64 {
        let d = Vec2::new(self.x.dot(n), self.y.dot(n));
        let most = self.regions.iter().flat_map(|r| r.outer.segs.iter()).map(|s| seg_reach(s, d)).fold(f64::NEG_INFINITY, f64::max);
        if most.is_finite() { most - self.at.dot(d) } else { 0.0 }
    }

    /// The section's largest distance from the line, a bound on its reach in any direction.
    pub fn radius(&self) -> f64 {
        let r = |d: Vec2| {
            self.regions.iter().flat_map(|r| r.outer.segs.iter()).map(|s| seg_reach(s, d)).fold(f64::NEG_INFINITY, f64::max) - self.at.dot(d)
        };
        let (hx, lx, hy, ly) = (r(Vec2::new(1.0, 0.0)), r(Vec2::new(-1.0, 0.0)), r(Vec2::new(0.0, 1.0)), r(Vec2::new(0.0, -1.0)));
        let (w, h) = (hx.max(lx).max(0.0), hy.max(ly).max(0.0));
        let out = (w * w + h * h).sqrt();
        if out.is_finite() { out } else { 0.0 }
    }

    fn line(&self) -> MemberLine {
        MemberLine { id: String::new(), a: self.a, b: self.b, ends: [String::new(), String::new()] }
    }
}

/// The largest `p·d` over a segment.
fn seg_reach(s: &Seg2, d: Vec2) -> f64 {
    match *s {
        Seg2::Line { a, b } => a.dot(d).max(b.dot(d)),
        Seg2::Arc { center, radius, start, sweep } => {
            let at = |t: f64| center + Vec2::new(t.cos(), t.sin()) * radius;
            let mut most = at(start).dot(d).max(at(start + sweep).dot(d));
            if d.len() > 0.0 {
                let toward = d.y.atan2(d.x);
                let (lo, hi) = if sweep >= 0.0 { (start, start + sweep) } else { (start + sweep, start) };
                let tau = std::f64::consts::TAU;
                let k = ((lo - toward) / tau).ceil();
                if toward + k * tau <= hi {
                    most = most.max(center.dot(d) + radius.abs() * d.len());
                }
            }
            most
        }
        Seg2::Cubic { p0, p1, p2, p3 } => {
            let bez = |t: f64| {
                let u = 1.0 - t;
                p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
            };
            let (a0, a1, a2) = ((p1 - p0).dot(d), (p2 - p1).dot(d), (p3 - p2).dot(d));
            let (qa, qb, qc) = (a0 - 2.0 * a1 + a2, 2.0 * (a1 - a0), a0);
            let mut most = p0.dot(d).max(p3.dot(d));
            let mut roots = Vec::with_capacity(2);
            if qa.abs() > 1e-15 {
                let disc = qb * qb - 4.0 * qa * qc;
                if disc >= 0.0 {
                    let s = disc.sqrt();
                    roots.extend([(-qb + s) / (2.0 * qa), (-qb - s) / (2.0 * qa)]);
                }
            } else if qb.abs() > 1e-15 {
                roots.push(-qc / qb);
            }
            for t in roots.into_iter().filter(|t| *t > 0.0 && *t < 1.0) {
                most = most.max(bez(t).dot(d));
            }
            most
        }
        Seg2::Conic { a, apex, b, .. } => a.dot(d).max(apex.dot(d)).max(b.dot(d)),
        Seg2::Bezier { n, p } => p.iter().take(usize::from(n) + 1).map(|q| q.dot(d)).fold(f64::NEG_INFINITY, f64::max),
    }
}

/// A plane cut: a member keeps what is on the side `normal` points to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    pub origin: Vec3,
    pub normal: Vec3,
}

/// What a junction gets, worked out from the feature's settings and its override.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub treatment: Treatment,
    pub gap: f64,
    pub butt_cut: ButtCut,
    /// The junction's member indices in the order they run through at a butt: the first runs
    /// through, each later one stops at those before it. Empty: in the junction's own order.
    pub rank: Vec<usize>,
}

/// How a member is made: run from `ext[0]` before its start to `ext[1]` past its end, then cut by
/// each of `cuts`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemberCuts {
    pub ext: [f64; 2],
    pub cuts: Vec<(End, String, Cut)>,
}

impl MemberCuts {
    pub fn is_plain(&self) -> bool {
        self.cuts.is_empty() && self.ext == [0.0, 0.0]
    }
}

/// The cuts every member gets at `junctions`, with `settings` for each.
pub fn member_cuts(members: &[Placed], junctions: &[Junction], settings: impl Fn(&Junction) -> Settings) -> Result<Vec<MemberCuts>> {
    let mut out = vec![MemberCuts::default(); members.len()];
    for j in junctions {
        let set = settings(j);
        if set.treatment.is_none() {
            continue;
        }
        for (i, end, cut) in junction_cuts(members, j, &set)? {
            if let Some(m) = out.get_mut(i) {
                m.cuts.push((end, j.id.clone(), cut));
            }
        }
    }
    for (i, m) in out.iter_mut().enumerate() {
        let Some(p) = members.get(i) else { continue };
        let line = p.line();
        for &(end, ref id, cut) in &m.cuts {
            let (Some(u), e0, e1) = (line.inward(end), line.at(end), line.at(other(end))) else {
                return Err(DocError::Invalid(format!("junction `{id}`: a member there has no length")));
            };
            let along = u.dot(cut.normal);
            if !(along > 1e-9) {
                return Err(DocError::Invalid(format!("junction `{id}`: the cut runs along a member (try butt or none)")));
            }
            // The far end must be wholly kept.
            let kept = (e1 - cut.origin).dot(cut.normal) - p.reach(-cut.normal);
            if !(kept > JOIN_TOL) {
                return Err(DocError::Invalid(format!("junction `{id}`: the cut reaches past the member's other end (try butt or none)")));
            }
            let need = ((e0 - cut.origin).dot(cut.normal) + p.reach(cut.normal)) / along;
            let margin = 0.01 * (p.radius() + 1.0);
            let ext = if need > -margin { need.max(0.0) + margin } else { 0.0 };
            if !(ext <= MAX_EXTENSION) {
                return Err(DocError::Invalid(format!("junction `{id}`: the members are too near parallel for this corner (try butt or none)")));
            }
            let slot = &mut m.ext[end as usize];
            *slot = slot.max(ext);
        }
    }
    Ok(out)
}

fn other(e: End) -> End {
    match e {
        End::Start => End::End,
        End::End => End::Start,
    }
}

/// The cuts one junction makes, as (member, end, cut).
fn junction_cuts(members: &[Placed], j: &Junction, set: &Settings) -> Result<Vec<(usize, End, Cut)>> {
    let into = |(i, e): (usize, End)| -> Result<Vec3> {
        members.get(i).and_then(|p| p.line().inward(e)).ok_or_else(|| DocError::Invalid(format!("junction `{}`: a member there has no length", j.id)))
    };
    let member = |i: usize| members.get(i).ok_or_else(|| DocError::Invalid(format!("junction `{}`: no member {i}", j.id)));
    let n0 = j.at;
    let g = set.gap.max(0.0);
    let mut out = Vec::new();
    match j.kind {
        JunctionKind::I => {
            if g > 0.0 {
                for &end in &j.ends {
                    let u = into(end)?;
                    out.push((end.0, end.1, Cut { origin: n0 + u * (g / 2.0), normal: u }));
                }
            }
        }
        JunctionKind::L if set.treatment == Treatment::Mitre => {
            let [p, q] = j.ends.as_slice() else { return Ok(out) };
            let (u, v) = (into(*p)?, into(*q)?);
            let n = (u - v).normalized().ok_or_else(|| DocError::Invalid(format!("junction `{}`: the members run back along each other", j.id)))?;
            out.push((p.0, p.1, Cut { origin: n0 + n * (g / 2.0), normal: n }));
            out.push((q.0, q.1, Cut { origin: n0 - n * (g / 2.0), normal: -n }));
        }
        JunctionKind::L => {
            let [p, q] = j.ends.as_slice() else { return Ok(out) };
            let (t, b) = if set.rank.first() == Some(&q.0) { (*q, *p) } else { (*p, *q) };
            let (ut, ub) = (into(t)?, into(b)?);
            let (mt, mb) = (member(t.0)?, member(b.0)?);
            out.push((b.0, b.1, butt(mt, ut, mb, ub, n0, g, set.butt_cut, &j.id)?));
            // The through member runs on to the butting member's far face.
            let away =
                (-ut - ub * (-ut).dot(ub)).normalized().ok_or_else(|| DocError::Invalid(format!("junction `{}`: the members are in line", j.id)))?;
            out.push((t.0, t.1, Cut { origin: n0 + away * mb.reach(away), normal: -away }));
        }
        JunctionKind::T => {
            let span = j.span.and_then(|s| members.get(s)).ok_or_else(|| DocError::Invalid(format!("junction `{}`: no member it lands on", j.id)))?;
            let along = (span.b - span.a)
                .normalized()
                .ok_or_else(|| DocError::Invalid(format!("junction `{}`: the member it lands on has no length", j.id)))?;
            for &end in &j.ends {
                let ub = into(end)?;
                out.push((end.0, end.1, butt(span, along, member(end.0)?, ub, n0, g, set.butt_cut, &j.id)?));
            }
        }
        JunctionKind::Multi => {
            // In `rank` order; each end butts against all above it.
            let mut ranked = j.ends.clone();
            ranked.sort_by_key(|e| set.rank.iter().position(|r| *r == e.0).unwrap_or(usize::MAX));
            for (k, &end) in ranked.iter().enumerate().skip(1) {
                let ub = into(end)?;
                for &above in ranked.iter().take(k) {
                    let ua = into(above)?;
                    if ub.dot(ua).abs() >= COLLINEAR_TOL.cos() {
                        continue;
                    }
                    out.push((end.0, end.1, butt(member(above.0)?, ua, member(end.0)?, ub, n0, g, set.butt_cut, &j.id)?));
                }
            }
        }
    }
    Ok(out)
}

/// The cut that stops member `b` (direction `ub` from `n0` into it) at the face of member `t`
/// (along `ut`, either way) towards it, `gap` short of it.
#[allow(clippy::too_many_arguments)]
fn butt(t: &Placed, ut: Vec3, b: &Placed, ub: Vec3, n0: Vec3, gap: f64, cut: ButtCut, id: &str) -> Result<Cut> {
    let m = (ub - ut * ub.dot(ut)).normalized().ok_or_else(|| DocError::Invalid(format!("junction `{id}`: the members are in line")))?;
    let h = t.reach(m) + gap;
    Ok(match cut {
        ButtCut::Flush => Cut { origin: n0 + m * h, normal: m },
        ButtCut::Square => {
            let s = (h + b.reach(-m)) / ub.dot(m);
            Cut { origin: n0 + ub * s, normal: ub }
        }
    })
}

/// The settings junction `j` gets: the feature's `defaults` (with its gap `gap`), changed by `over`
/// (with its gap `over_gap`), its `through` line found among `lines`; the rest ranked by the
/// feature's `butt_through`.
pub fn settings(j: &Junction, defaults: &CornerDefaults, gap: f64, over: Option<(&CornerOverride, Option<f64>)>, lines: &[MemberLine]) -> Settings {
    let mut s = Settings { treatment: defaults.within_group, gap, butt_cut: defaults.butt_cut, rank: Vec::new() };
    let mut through = None;
    if let Some((o, g)) = over {
        if let Some(t) = o.treatment {
            s.treatment = t;
        }
        if let Some(g) = g {
            s.gap = g;
        }
        if let Some(c) = o.butt_cut {
            s.butt_cut = c;
        }
        through = o.through.as_deref().and_then(|t| lines.iter().position(|l| l.id == t)).filter(|i| j.ends.iter().any(|e| e.0 == *i));
    }
    let mut rank: Vec<usize> = j.ends.iter().map(|e| e.0).collect();
    rank.dedup();
    rank.sort_by(|a, b| {
        let longer = longer_first(lines.get(*a), lines.get(*b)).then(a.cmp(b));
        match defaults.butt_through {
            ButtThrough::Longer => longer,
            ButtThrough::Shorter => longer.reverse(),
        }
    });
    if let Some(t) = through
        && let Some(k) = rank.iter().position(|r| *r == t)
    {
        let top = rank.remove(k);
        rank.insert(0, top);
    }
    s.rank = rank;
    // Mitre is for two ends at an angle; elsewhere it is a butt.
    if s.treatment == Treatment::Mitre && j.kind != JunctionKind::L {
        s.treatment = Treatment::Butt;
    }
    s
}

/// `Less` when `a` counts as the longer line: by length (equal within a part in a million), then
/// by how nearly it runs along X, then Y, then Z.
fn longer_first(a: Option<&MemberLine>, b: Option<&MemberLine>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (Some(a), Some(b)) = (a, b) else { return Ordering::Equal };
    let (da, db) = (a.b - a.a, b.b - b.a);
    let (la, lb) = (da.len(), db.len());
    if (la - lb).abs() > JOIN_TOL * la.max(lb).max(1.0) {
        return lb.partial_cmp(&la).unwrap_or(Ordering::Equal);
    }
    let axis = |d: Vec3, l: f64| if l > 0.0 { [d.x.abs() / l, d.y.abs() / l, d.z.abs() / l] } else { [0.0; 3] };
    let (aa, ab) = (axis(da, la), axis(db, lb));
    for k in 0..3 {
        if (aa[k] - ab[k]).abs() > COLLINEAR_TOL {
            return ab[k].partial_cmp(&aa[k]).unwrap_or(Ordering::Equal);
        }
    }
    Ordering::Equal
}

#[cfg(test)]
#[path = "frame_corner_tests.rs"]
mod tests;
