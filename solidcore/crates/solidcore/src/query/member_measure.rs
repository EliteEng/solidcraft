//! The measures a frame member's cut list, welds and fabrication exports are worked out on: unit
//! normals, whether two directions are parallel, how far a point lies from a
//! path's line or arc, an arc's centroid, how wide a section is across a direction, and the
//! angles an end face is cut at. The caller compares them with its tolerances.
//!
//! Upstream: none (not in SMLib).
//!
//! | Rust                 | Upstream |
//! |----------------------|----------|
//! | [`unit_normal`]      | (none)   |
//! | [`parallel`]         | (none)   |
//! | [`segment_distance`] | (none)   |
//! | [`arc_distance`]     | (none)   |
//! | [`arc_centroid`]     | (none)   |
//! | [`width_across`]     | (none)   |
//! | [`end_cut`]          | (none)   |
//!
//! Deviations: no upstream counterpart.

use crate::error::{Result, SolidError};
use crate::math::{Point2, Point3, Vec2, Vec3};
use crate::prism::profile::{Loop2, Seg2};

/// The unit normal of directions `a` and `b`, `a × b` at unit length: right-handed, so the unit
/// normal of a plane's x and y axes is its normal.
///
/// # Errors
///
/// [`SolidError::InvalidInput`]-style refusal of [`Vec3::unitize`] when `a` and `b` are parallel,
/// either has no length, or a number is not finite.
pub fn unit_normal(a: Vec3, b: Vec3) -> Result<Vec3> {
    a.cross(b).unitize()
}

/// Whether `a` and `b` run the same way or opposite ways: the sine of the angle between them,
/// `|â × b̂|`, at most `tol`. `false` when either has no length or a number is not finite.
#[must_use]
pub fn parallel(a: Vec3, b: Vec3, tol: f64) -> bool {
    match (a.unitize(), b.unitize()) {
        (Ok(a), Ok(b)) => a.cross(b).length() <= tol,
        _ => false,
    }
}

/// How far `p` lies from the line segment from `a` to `b`: from its nearest point, an end where
/// the foot of the perpendicular falls outside it (`a` itself when `a` and `b` are one point).
#[must_use]
pub fn segment_distance(a: Point3, b: Point3, p: Point3) -> f64 {
    let d = b - a;
    let l = d.dot(d);
    let t = if l > 0.0 {
        ((p - a).dot(d) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - (a + d * t)).length()
}

/// The arc from `a` turning `angle` radians right-handed about the line through `center` along
/// `axis`: its radius, and the unit vectors along `a − center` and a quarter turn on.
fn arc_frame(a: Point3, center: Point3, axis: Vec3) -> Result<(f64, Vec3, Vec3)> {
    let k = axis.unitize()?;
    let r0 = a - center;
    let radius = r0.length();
    let x = r0.unitize()?;
    let y = k.cross(x).unitize()?;
    Ok((radius, x, y))
}

/// How far `p` lies from the arc from `a` turning `angle` radians right-handed about the line
/// through `center` along `axis`: radially from where `p` lies within its sweep (seen along the
/// axis), else from the nearer end.
///
/// # Errors
///
/// [`SolidError::InvalidInput`] for a zero or non-finite `axis`, an `a` at `center`, or a
/// non-finite `angle` or `p`.
pub fn arc_distance(a: Point3, center: Point3, axis: Vec3, angle: f64, p: Point3) -> Result<f64> {
    if !(angle.is_finite() && p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
        return Err(SolidError::InvalidInput {
            operation: "arc_distance",
            reason: "the sweep and the point must be finite",
        });
    }
    let (radius, x, y) = arc_frame(a, center, axis)?;
    let k = x.cross(y);
    let q = p - center;
    let flat = q - k * q.dot(k);
    let mut t = flat.dot(y).atan2(flat.dot(x));
    // The angle within the arc's sweep, else the nearer end.
    let (lo, hi) = if angle >= 0.0 {
        (0.0, angle)
    } else {
        (angle, 0.0)
    };
    let tau = std::f64::consts::TAU;
    while t < lo {
        t += tau;
    }
    while t > hi + 1e-12 && t - tau >= lo - 1e-12 {
        t -= tau;
    }
    let at = |t: f64| center + (x * t.cos() + y * t.sin()) * radius;
    Ok(if t <= hi {
        (p - at(t)).length()
    } else {
        (p - at(lo)).length().min((p - at(hi)).length())
    })
}

/// The length and the centroid of the arc from `a` turning `angle` radians right-handed about the
/// line through `center` along `axis`. The centroid lies on the arc's bisector,
/// `r·sin(θ/2)/(θ/2)` from the centre.
///
/// # Errors
///
/// [`SolidError::InvalidInput`] for a zero or non-finite `axis`, an `a` at `center`, or a
/// non-finite `angle`.
pub fn arc_centroid(a: Point3, center: Point3, axis: Vec3, angle: f64) -> Result<(f64, Point3)> {
    if !angle.is_finite() {
        return Err(SolidError::InvalidInput {
            operation: "arc_centroid",
            reason: "the sweep must be finite",
        });
    }
    let (radius, x, y) = arc_frame(a, center, axis)?;
    let half = angle / 2.0;
    let reach = if half.abs() > 1e-12 {
        radius * half.sin() / half
    } else {
        radius
    };
    let mid = x * half.cos() + y * half.sin();
    Ok((radius * angle.abs(), center + mid * reach))
}

/// How wide `outline` is across the unit direction `w` in its plane: the spread of `p · w` over
/// it. Exact for lines and arcs (an arc's ends, and where it reaches furthest along ±`w` when
/// that lies on it); a curve by its control points, which bound it. 0 for no finite point.
#[must_use]
pub fn width_across(outline: &Loop2, w: Vec2) -> f64 {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut add = |p: Point2| {
        let v = p.dot(w);
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    };
    for g in &outline.segs {
        match *g {
            Seg2::Line { a, b } => {
                add(a);
                add(b);
            }
            Seg2::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let at =
                    |t: f64| Point2::new(center.x + radius * t.cos(), center.y + radius * t.sin());
                add(at(start));
                add(at(start + sweep));
                let face = w.y.atan2(w.x);
                for t in [face, face + std::f64::consts::PI] {
                    let mut d = (t - start).rem_euclid(std::f64::consts::TAU);
                    if sweep < 0.0 {
                        d -= std::f64::consts::TAU;
                    }
                    if (sweep.abs() - d.abs()) >= 0.0 && d * sweep >= 0.0 {
                        add(at(t));
                    }
                }
            }
            Seg2::Cubic { p0, p1, p2, p3 } => [p0, p1, p2, p3].into_iter().for_each(&mut add),
            Seg2::Conic { a, apex, b, .. } => [a, apex, b].into_iter().for_each(&mut add),
            Seg2::Bezier { n, p } => p
                .iter()
                .take(usize::from(n) + 1)
                .copied()
                .for_each(&mut add),
        }
    }
    if hi >= lo {
        hi - lo
    } else {
        0.0
    }
}

/// How a flat end face of a member is cut ([`end_cut`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndCut {
    /// The face's unit normal.
    pub normal: Vec3,
    /// Whether it faces along the member (its far end), not back (its near end).
    pub at_end: bool,
    /// The cosine of its tilt from square: `|n · along|`, at most 1.
    pub cos: f64,
    /// Its tilt from square to the member, in radians.
    pub angle: f64,
    /// Which way across the section it leans, in radians from the section's x axis: the
    /// direction its normal's part across the section points away from.
    pub towards: f64,
    /// The length of its normal's part across the section, `|n − (n · along) along|`.
    pub side: f64,
    /// That part as a unit direction in the section (x, y), where it has a length (above 1e-12).
    pub across: Option<Point2>,
}

/// How the face of normal `normal` (any length) cuts a member running along unit `along`, its
/// section's axes the unit `x` and `y`. `None` when the face runs along the member (its normal
/// within 1e-9 of square to `along`) or `normal` has no length.
#[must_use]
pub fn end_cut(normal: Vec3, along: Vec3, x: Vec3, y: Vec3) -> Option<EndCut> {
    let n = normal.unitize().ok()?;
    let lean = n.dot(along);
    if lean.abs() <= 1e-9 || lean.is_nan() {
        return None;
    }
    let cos = lean.abs().min(1.0);
    let (nx, ny) = (n.dot(x), n.dot(y));
    let side = nx.hypot(ny);
    let across = (side > 1e-12).then(|| Point2::new(nx / side, ny / side));
    Some(EndCut {
        normal: n,
        at_end: lean > 0.0,
        cos,
        angle: cos.acos(),
        towards: (-ny).atan2(-nx),
        side,
        across,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(x, y, z)
    }

    #[test]
    fn unit_normals_and_parallels() {
        assert!(
            (unit_normal(v(2.0, 0.0, 0.0), v(0.0, 5.0, 0.0)).unwrap() - v(0.0, 0.0, 1.0)).length()
                < 1e-15
        );
        assert!(unit_normal(v(1.0, 0.0, 0.0), v(-3.0, 0.0, 0.0)).is_err());
        assert!(parallel(v(1.0, 2.0, 3.0), v(-2.0, -4.0, -6.0), 1e-12));
        assert!(!parallel(v(1.0, 0.0, 0.0), v(1.0, 1e-3, 0.0), 1e-7));
        assert!(!parallel(v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), 1.0));
    }

    #[test]
    fn a_point_is_measured_to_a_segment_and_an_arc() {
        let (a, b) = (v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0));
        assert!((segment_distance(a, b, v(5.0, 3.0, 4.0)) - 5.0).abs() < 1e-12);
        assert!((segment_distance(a, b, v(13.0, 4.0, 0.0)) - 5.0).abs() < 1e-12);
        assert!((segment_distance(a, a, v(0.0, 2.0, 0.0)) - 2.0).abs() < 1e-12);
        // A quarter circle of radius 10 about Z from (10, 0, 0) to (0, 10, 0).
        let quarter = |p| {
            arc_distance(
                v(10.0, 0.0, 0.0),
                v(0.0, 0.0, 0.0),
                v(0.0, 0.0, 1.0),
                std::f64::consts::FRAC_PI_2,
                p,
            )
            .unwrap()
        };
        // Within the sweep, 3 out and 4 up: 5. Past it (behind its start), from the nearer end.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        assert!((quarter(v(13.0 * s, 13.0 * s, 4.0)) - 5.0).abs() < 1e-12);
        assert!((quarter(v(10.0, -3.0, 0.0)) - 3.0).abs() < 1e-12);
        assert!(arc_distance(
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
            1.0,
            v(1.0, 0.0, 0.0)
        )
        .is_err());
        assert!(arc_distance(
            v(1.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
            f64::NAN,
            v(1.0, 0.0, 0.0)
        )
        .is_err());
    }

    #[test]
    fn a_half_circle_has_its_centroid_two_r_over_pi_out() {
        let (length, c) = arc_centroid(
            v(10.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
            std::f64::consts::PI,
        )
        .unwrap();
        assert!((length - 10.0 * std::f64::consts::PI).abs() < 1e-12);
        assert!(
            (c - v(0.0, 20.0 / std::f64::consts::PI, 0.0)).length() < 1e-12,
            "{c:?}"
        );
        // No sweep: the start point itself.
        let (length, c) =
            arc_centroid(v(10.0, 0.0, 0.0), v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), 0.0).unwrap();
        assert!(length == 0.0 && (c - v(10.0, 0.0, 0.0)).length() < 1e-12);
    }

    #[test]
    fn a_square_and_a_circle_have_their_widths_across() {
        let p = |x: f64, y: f64| Point2::new(x, y);
        let square = Loop2 {
            segs: vec![
                Seg2::Line {
                    a: p(0.0, 0.0),
                    b: p(10.0, 0.0),
                },
                Seg2::Line {
                    a: p(10.0, 0.0),
                    b: p(10.0, 10.0),
                },
                Seg2::Line {
                    a: p(10.0, 10.0),
                    b: p(0.0, 10.0),
                },
                Seg2::Line {
                    a: p(0.0, 10.0),
                    b: p(0.0, 0.0),
                },
            ],
        };
        assert!((width_across(&square, Vec2::new(1.0, 0.0)) - 10.0).abs() < 1e-12);
        let s = std::f64::consts::FRAC_1_SQRT_2;
        assert!((width_across(&square, Vec2::new(s, s)) - 20.0 * s).abs() < 1e-12);
        // A whole circle of radius 5 starting at angle 0: 10 across any way, its extremes inside the arc.
        let circle = Loop2 {
            segs: vec![Seg2::Arc {
                center: p(0.0, 0.0),
                radius: 5.0,
                start: 0.0,
                sweep: std::f64::consts::TAU,
            }],
        };
        assert!((width_across(&circle, Vec2::new(0.0, 1.0)) - 10.0).abs() < 1e-12);
        assert_eq!(
            width_across(&Loop2 { segs: Vec::new() }, Vec2::new(1.0, 0.0)),
            0.0
        );
    }

    #[test]
    fn an_end_face_is_cut_at_its_tilt_and_leans_across_the_section() {
        // A member along X, its section's axes Y and Z; the far end cut at 30°, leaning away from +Y (±π).
        let (along, x, y) = (v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0), v(0.0, 0.0, 1.0));
        let t = 30f64.to_radians();
        let cut = end_cut(v(t.cos(), t.sin(), 0.0) * 2.0, along, x, y).unwrap();
        assert!(cut.at_end && (cut.angle - t).abs() < 1e-12 && (cut.cos - t.cos()).abs() < 1e-12);
        assert!(
            (cut.side - t.sin()).abs() < 1e-12
                && (cut.towards.abs() - std::f64::consts::PI).abs() < 1e-12
        );
        assert!((cut.across.unwrap() - Point2::new(1.0, 0.0)).length() < 1e-12);
        // A square near end: no tilt, no lean.
        let square = end_cut(v(-1.0, 0.0, 0.0), along, x, y).unwrap();
        assert!(!square.at_end && square.angle.abs() < 1e-12 && square.across.is_none());
        // A face along the member is no end face.
        assert!(end_cut(v(0.0, 1.0, 0.0), along, x, y).is_none());
        assert!(end_cut(v(0.0, 0.0, 0.0), along, x, y).is_none());
    }
}
