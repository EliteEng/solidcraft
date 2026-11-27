//! A frame member's cut list, weld and export measures: the kernel's
//! [`solidcore::query::member_measure`] on the record's types.

use solidcore::query::member_measure as k;
use solidcore_convert::{loop2, record_vec2, record_vec3, vec2, vec3};
use solidcore_record::{Loop2, Vec2, Vec3};

/// `a × b` at unit length (a plane's x and y axes give its normal); `None` when they are parallel,
/// either has no length, or a number is not finite.
#[must_use]
pub fn unit_normal(a: Vec3, b: Vec3) -> Option<Vec3> {
    k::unit_normal(vec3(a), vec3(b)).ok().map(record_vec3)
}

/// Whether `a` and `b` run the same way or opposite ways, the sine of the angle between them at
/// most `tol`; `false` when either has no length.
#[must_use]
pub fn parallel(a: Vec3, b: Vec3, tol: f64) -> bool {
    k::parallel(vec3(a), vec3(b), tol)
}

/// How far `p` lies from the segment from `a` to `b` (mm).
#[must_use]
pub fn segment_distance(a: Vec3, b: Vec3, p: Vec3) -> f64 {
    k::segment_distance(vec3(a), vec3(b), vec3(p))
}

/// How far `p` lies from the arc from `a` turning `angle` radians about the line through `center`
/// along `axis` (mm); `None` for a degenerate arc or a number not finite.
#[must_use]
pub fn arc_distance(a: Vec3, center: Vec3, axis: Vec3, angle: f64, p: Vec3) -> Option<f64> {
    k::arc_distance(vec3(a), vec3(center), vec3(axis), angle, vec3(p)).ok()
}

/// The length (mm) and the centroid of the arc from `a` turning `angle` radians about the line
/// through `center` along `axis`; `None` for a degenerate arc or a number not finite.
#[must_use]
pub fn arc_centroid(a: Vec3, center: Vec3, axis: Vec3, angle: f64) -> Option<(f64, Vec3)> {
    k::arc_centroid(vec3(a), vec3(center), vec3(axis), angle)
        .ok()
        .map(|(l, c)| (l, record_vec3(c)))
}

/// How wide `outline` is across the unit direction `w` in its plane (mm).
#[must_use]
pub fn width_across(outline: &Loop2, w: Vec2) -> f64 {
    k::width_across(&loop2(outline), vec2(w))
}

/// How a flat end face cuts a member ([`end_cut`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndCut {
    /// The face's unit normal.
    pub normal: Vec3,
    /// Whether it faces along the member (its far end).
    pub at_end: bool,
    /// The cosine of its tilt from square.
    pub cos: f64,
    /// Its tilt from square, in radians.
    pub angle: f64,
    /// Which way across the section it leans, in radians from the section's x axis.
    pub towards: f64,
    /// The length of its normal's part across the section.
    pub side: f64,
    /// That part as a unit direction in the section, where it has a length.
    pub across: Option<Vec2>,
}

/// How the face of normal `normal` cuts a member running along unit `along`, its section's axes
/// the unit `x` and `y`; `None` when the face runs along the member.
#[must_use]
pub fn end_cut(normal: Vec3, along: Vec3, x: Vec3, y: Vec3) -> Option<EndCut> {
    k::end_cut(vec3(normal), vec3(along), vec3(x), vec3(y)).map(|c| EndCut {
        normal: record_vec3(c.normal),
        at_end: c.at_end,
        cos: c.cos,
        angle: c.angle,
        towards: c.towards,
        side: c.side,
        across: c.across.map(record_vec2),
    })
}
