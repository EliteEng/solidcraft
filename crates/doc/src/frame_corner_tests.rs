//! Junction and corner-cut tests. Expected values are worked out by hand from the lines and a
//! 40 × 40 square section on its centre.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use solidcore_record::Loop2;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn line(id: &str, a: Vec3, b: Vec3, ends: [&str; 2]) -> MemberLine {
    MemberLine { id: id.into(), a, b, ends: [ends[0].into(), ends[1].into()] }
}

fn square(side: f64) -> Region2 {
    let h = side / 2.0;
    Region2 { outer: Loop2::polygon(&[Vec2::new(-h, -h), Vec2::new(h, -h), Vec2::new(h, h), Vec2::new(-h, h)]), holes: vec![] }
}

/// A member on `l` with its section's y as near world up (+z) as the line allows, as
/// `frame::member_axes` places it.
fn placed<'a>(l: &MemberLine, regions: &'a [Region2]) -> Placed<'a> {
    let (x, y) = crate::frame::member_axes(l.b - l.a, 0.0).unwrap();
    Placed { a: l.a, b: l.b, x, y, at: Vec2::ZERO, regions }
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() <= 1e-9 * (1.0 + want.abs()), "{what}: got {got}, want {want}");
}

fn close3(got: Vec3, want: Vec3, what: &str) {
    assert!(got.dist(want) <= 1e-9 * (1.0 + want.len()), "{what}: got {got:?}, want {want:?}");
}

/// A 200 × 100 rectangle drawn as a chain: p1 (0,0) → p2 (200,0) → p3 (200,100) → p4 (0,100).
fn rectangle() -> Vec<MemberLine> {
    vec![
        line("l1", v(0.0, 0.0, 0.0), v(200.0, 0.0, 0.0), ["p1", "p2"]),
        line("l2", v(200.0, 0.0, 0.0), v(200.0, 100.0, 0.0), ["p2", "p3"]),
        line("l3", v(200.0, 100.0, 0.0), v(0.0, 100.0, 0.0), ["p3", "p4"]),
        line("l4", v(0.0, 100.0, 0.0), v(0.0, 0.0, 0.0), ["p4", "p1"]),
    ]
}

#[test]
fn junctions_are_found_and_named_after_their_points() {
    let mut lines = rectangle();
    // A cross bar from the bottom's middle (its own point p5) to the top's, landing on both spans.
    lines.push(line("l5", v(100.0, 0.0, 0.0), v(100.0, 100.0, 0.0), ["p5", "p6"]));
    // Two collinear lines continuing the right side upwards, and a third end there.
    lines.push(line("l6", v(200.0, 100.0, 0.0), v(200.0, 150.0, 0.0), ["p3", "p7"]));
    // A line crossing the cross bar between its ends, landing nowhere.
    lines.push(line("l7", v(50.0, 50.0, 0.0), v(150.0, 60.0, 0.0), ["p8", "p9"]));
    let found = junctions(&lines, 1e-6);
    let got: Vec<(&str, JunctionKind, Vec<(usize, End)>)> = found.iter().map(|j| (j.id.as_str(), j.kind, j.ends.clone())).collect();
    assert_eq!(
        got,
        vec![
            ("p1", JunctionKind::L, vec![(0, End::Start), (3, End::End)]),
            ("p2", JunctionKind::L, vec![(0, End::End), (1, End::Start)]),
            ("p3", JunctionKind::Multi, vec![(1, End::End), (2, End::Start), (5, End::Start)]),
            ("p4", JunctionKind::L, vec![(2, End::End), (3, End::Start)]),
            ("p5@l1", JunctionKind::T, vec![(4, End::Start)]),
            ("p6@l3", JunctionKind::T, vec![(4, End::End)]),
        ]
    );
    assert_eq!(found.iter().find(|j| j.id == "p5@l1").and_then(|j| j.span), Some(0));
    close3(found[2].at, v(200.0, 100.0, 0.0), "p3 position");
}

#[test]
fn ends_join_by_position_and_collinear_ends_are_an_i() {
    // Two lines drawn apart (no shared point), end to end and in line; then one at an angle whose
    // end lands a hair from them.
    let lines = vec![
        line("a", v(0.0, 0.0, 0.0), v(100.0, 0.0, 0.0), ["p1", "p2"]),
        line("b", v(100.0, 0.0, 0.0), v(250.0, 0.0, 0.0), ["p3", "p4"]),
        line("c", v(250.0, 1e-8, 0.0), v(250.0, 100.0, 0.0), ["p5", "p6"]),
    ];
    let found = junctions(&lines, 1e-6);
    let got: Vec<(&str, JunctionKind)> = found.iter().map(|j| (j.id.as_str(), j.kind)).collect();
    assert_eq!(got, vec![("p2", JunctionKind::I), ("p4", JunctionKind::L)]);
}

#[test]
fn a_section_reaches_exactly_as_far_as_its_lines_and_arcs() {
    let sq = [square(40.0)];
    let l = line("x", v(0.0, 0.0, 0.0), v(100.0, 0.0, 0.0), ["p1", "p2"]);
    let p = placed(&l, &sq);
    close(p.reach(v(0.0, 1.0, 0.0)), 20.0, "square, square on");
    close(p.reach(v(0.0, 1.0, 1.0).normalized().unwrap()), 40.0 / 2f64.sqrt(), "square, corner on");
    close(p.reach(v(1.0, 0.0, 0.0)), 0.0, "along the line");
    let round = [Region2 { outer: Loop2::circle(Vec2::new(5.0, 0.0), 10.0), holes: vec![] }];
    let p = Placed { regions: &round, ..p };
    for (dir, want) in [(v(0.0, 1.0, 0.0), 10.0), (v(0.0, -1.0, 0.0), 10.0), (v(0.0, 0.0, 1.0), 10.0), (v(0.0, 0.0, -1.0), 10.0)] {
        // The section's x is along -y here (member_axes on a line along +x), so its centre at x = 5
        // sits 5 off the line one way.
        let off = 5.0 * p.x.dot(dir);
        close(p.reach(dir), want + off, "circle");
    }
    let cubic = Seg2::Cubic { p0: Vec2::new(0.0, 0.0), p1: Vec2::new(0.0, 4.0), p2: Vec2::new(10.0, 4.0), p3: Vec2::new(10.0, 0.0) };
    // B(t).y = 12 t (1 − t) peaks at t = 1/2: 3.
    close(seg_reach(&cubic, Vec2::new(0.0, 1.0)), 3.0, "cubic's peak");
}

#[test]
fn an_l_mitres_on_the_bisecting_plane_with_half_the_gap_each_side() {
    let lines = rectangle();
    let sq = [square(40.0)];
    let members: Vec<Placed> = lines.iter().map(|l| placed(l, &sq)).collect();
    let found = junctions(&lines, 1e-6);
    let set = Settings { treatment: Treatment::Mitre, gap: 2.0, ..Settings::default() };
    let cuts = member_cuts(&members, &found, |_| set.clone()).unwrap();
    // Member l1 (along +x) at p1 meets l4 (coming down the y axis): the mitre normal is
    // (+x − +y)/√2, moved 1 mm along it.
    let n = v(1.0, -1.0, 0.0).normalized().unwrap();
    let (end, id, cut) = cuts[0].cuts.iter().find(|c| c.1 == "p1").cloned().unwrap();
    assert_eq!((end, id.as_str()), (End::Start, "p1"));
    close3(cut.normal, n, "l1's mitre normal");
    close3(cut.origin, n * 1.0, "l1's mitre plane, 1 mm off the node");
    let other = cuts[3].cuts.iter().find(|c| c.1 == "p1").unwrap().2;
    close3(other.normal, -n, "l4's mitre normal");
    close3(other.origin, -n, "l4's mitre plane");
    // Each end runs past its line far enough for its outer side to reach the plane: the section's
    // reach along the normal (its side 20 from the line, along −y: 20/√2) less the gap's offset (1),
    // over the normal's slope along the member (1/√2), plus a small margin.
    let need = (20.0 / 2f64.sqrt() - 1.0) * 2f64.sqrt();
    assert!(cuts[0].ext[0] > need && cuts[0].ext[0] < need + 1.0, "l1 runs back {} (needs {need})", cuts[0].ext[0]);
}

#[test]
fn a_butt_stops_at_the_through_members_face_and_the_through_member_covers_the_corner() {
    let lines = rectangle();
    let sq = [square(40.0)];
    let members: Vec<Placed> = lines.iter().map(|l| placed(l, &sq)).collect();
    let found = junctions(&lines, 1e-6);
    let p1: Vec<Junction> = found.into_iter().filter(|j| j.id == "p1").collect();
    let set = Settings { treatment: Treatment::Butt, gap: 3.0, rank: vec![0, 3], ..Settings::default() };
    let cuts = member_cuts(&members, &p1, |_| set.clone()).unwrap();
    // l4 (butting, running in +y from p1 into itself... its end is p1, going up) stops square at
    // y = 20 + 3.
    let c4 = cuts[3].cuts[0].2;
    close3(c4.normal, v(0.0, 1.0, 0.0), "butting cut is square to l4");
    close(c4.origin.y, 23.0, "butting cut at the through face plus the gap");
    // l1 runs on to l4's outer face, x = −20, keeping what is towards +x.
    let c1 = cuts[0].cuts[0].2;
    close3(c1.normal, v(1.0, 0.0, 0.0), "through member's end cut");
    close(c1.origin.x, -20.0, "through member reaches the far face");
    assert!(cuts[0].ext[0] >= 20.0, "the through member runs back over the corner");
    assert_eq!(cuts[3].ext[0], 0.0);
    assert_eq!(cuts[3].ext[1], 0.0, "the butting member is only shortened");
}

#[test]
fn a_t_butts_against_the_span_square_or_flush() {
    let lines = vec![
        line("rail", v(0.0, 0.0, 0.0), v(200.0, 0.0, 0.0), ["p1", "p2"]),
        // A brace at 45° landing on the rail at x = 100.
        line("brace", v(100.0, 0.0, 0.0), v(200.0, 100.0, 0.0), ["p3", "p4"]),
    ];
    let sq = [square(40.0)];
    let members: Vec<Placed> = lines.iter().map(|l| placed(l, &sq)).collect();
    let found = junctions(&lines, 1e-6);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].id.as_str(), found[0].kind), ("p3@rail", JunctionKind::T));
    let flush = member_cuts(&members, &found, |_| Settings { treatment: Treatment::Butt, butt_cut: ButtCut::Flush, ..Settings::default() }).unwrap();
    assert!(flush[0].is_plain(), "the rail runs through");
    let c = flush[1].cuts[0].2;
    close3(c.normal, v(0.0, 1.0, 0.0), "flush: on the rail's face");
    close(c.origin.y, 20.0, "flush: at the rail's top");
    let square_cut = member_cuts(&members, &found, |_| Settings { treatment: Treatment::Butt, ..Settings::default() }).unwrap();
    let c = square_cut[1].cuts[0].2;
    let ub = v(1.0, 1.0, 0.0).normalized().unwrap();
    close3(c.normal, ub, "square: across the brace");
    // The brace's lower side (20 below its line, square to it) clears y = 20 when the line is at
    // s with s/√2 − 20/√2 = 20: s = 20√2 + 20.
    close((c.origin - v(100.0, 0.0, 0.0)).len(), 20.0 * 2f64.sqrt() + 20.0, "square: where its lower corner meets the face");
}

#[test]
fn a_3d_corner_mitres_and_impossible_cuts_are_refused() {
    let lines = vec![line("a", v(0.0, 0.0, 0.0), v(100.0, 0.0, 0.0), ["p1", "p2"]), line("b", v(0.0, 0.0, 0.0), v(0.0, 70.0, 70.0), ["p1", "p3"])];
    let sq = [square(20.0)];
    let members: Vec<Placed> = lines.iter().map(|l| placed(l, &sq)).collect();
    let found = junctions(&lines, 1e-6);
    let cuts = member_cuts(&members, &found, |_| Settings { treatment: Treatment::Mitre, ..Settings::default() }).unwrap();
    let n = (v(1.0, 0.0, 0.0) - v(0.0, 1.0, 1.0).normalized().unwrap()).normalized().unwrap();
    close3(cuts[0].cuts[0].2.normal, n, "the mitre plane of lines in 3D");
    // A member shorter than the corner cut needs is refused, naming the junction.
    let short = vec![line("a", v(0.0, 0.0, 0.0), v(5.0, 0.0, 0.0), ["p1", "p2"]), line("b", v(0.0, 0.0, 0.0), v(100.0, 1.0, 0.0), ["p1", "p3"])];
    let members: Vec<Placed> = short.iter().map(|l| placed(l, &sq)).collect();
    let found = junctions(&short, 1e-6);
    let err = member_cuts(&members, &found, |_| Settings { treatment: Treatment::Mitre, ..Settings::default() }).unwrap_err().to_string();
    assert!(err.contains("junction `p1`") && err.contains("butt or none"), "{err}");
}

#[test]
fn settings_follow_the_override_and_mitre_is_only_for_an_l() {
    let lines = rectangle();
    let found = junctions(&lines, 1e-6);
    let defaults = CornerDefaults::mitre();
    let s = settings(&found[0], &defaults, 1.5, None, &lines);
    assert_eq!((s.treatment, s.gap, s.rank.clone()), (Treatment::Mitre, 1.5, vec![0, 3]), "the longer line first");
    let o = CornerOverride { treatment: Some(Treatment::Butt), through: Some("l4".into()), gap: None, butt_cut: Some(ButtCut::Flush) };
    let s = settings(&found[0], &defaults, 1.5, Some((&o, Some(0.5))), &lines);
    assert_eq!((s.treatment, s.gap, s.butt_cut, s.rank.clone()), (Treatment::Butt, 0.5, ButtCut::Flush, vec![3, 0]));
    // A through line that isn't at the junction is ignored.
    let o = CornerOverride { through: Some("l2".into()), ..CornerOverride::default() };
    assert_eq!(settings(&found[0], &defaults, 0.0, Some((&o, None)), &lines).rank, vec![0, 3]);
    let t = Junction { id: "p9@l1".into(), kind: JunctionKind::T, at: Vec3::ZERO, ends: vec![(1, End::Start)], span: Some(0) };
    assert_eq!(settings(&t, &defaults, 0.0, None, &lines).treatment, Treatment::Butt);
    // An old design's corners (none) read as none, and say nothing when saved.
    let old: CornerDefaults = serde_json::from_str("{}").unwrap();
    assert!(old.is_none() && settings(&found[0], &old, 0.0, None, &lines).treatment.is_none());
    assert_eq!(serde_json::to_string(&CornerDefaults::mitre()).unwrap(), r#"{"within_group":"mitre"}"#);
}

#[test]
fn the_longer_or_the_shorter_lines_run_through_at_every_corner() {
    let ranks = |lines: &[MemberLine], through: ButtThrough| -> Vec<Vec<usize>> {
        let defaults = CornerDefaults { within_group: Treatment::Butt, butt_through: through, ..CornerDefaults::default() };
        junctions(lines, 1e-6).iter().map(|j| settings(j, &defaults, 0.0, None, lines).rank).collect()
    };
    // The 200 × 100 rectangle: l1 and l3 (the long sides) run through at all four corners, or l2
    // and l4 do.
    let r = rectangle();
    assert_eq!(ranks(&r, ButtThrough::Longer), vec![vec![0, 3], vec![0, 1], vec![2, 1], vec![2, 3]]);
    assert_eq!(ranks(&r, ButtThrough::Shorter), vec![vec![3, 0], vec![1, 0], vec![1, 2], vec![3, 2]]);
    // A square's sides are equal: the ones along X count as longer, so the corners still agree.
    let sq = vec![
        line("l1", v(0.0, 0.0, 0.0), v(100.0, 0.0, 0.0), ["p1", "p2"]),
        line("l2", v(100.0, 0.0, 0.0), v(100.0, 100.0, 0.0), ["p2", "p3"]),
        line("l3", v(100.0, 100.0, 0.0), v(0.0, 100.0, 0.0), ["p3", "p4"]),
        line("l4", v(0.0, 100.0, 0.0), v(0.0, 0.0, 0.0), ["p4", "p1"]),
    ];
    assert_eq!(ranks(&sq, ButtThrough::Longer), vec![vec![0, 3], vec![0, 1], vec![2, 1], vec![2, 3]]);
    assert_eq!(ranks(&sq, ButtThrough::Shorter), vec![vec![3, 0], vec![1, 0], vec![1, 2], vec![3, 2]]);
    // An override's line runs through whatever the rule says.
    let defaults = CornerDefaults { within_group: Treatment::Butt, ..CornerDefaults::default() };
    let o = CornerOverride { through: Some("l4".into()), ..CornerOverride::default() };
    assert_eq!(settings(&junctions(&r, 1e-6)[0], &defaults, 0.0, Some((&o, None)), &r).rank, vec![3, 0]);
    assert_eq!(
        serde_json::to_string(&CornerDefaults { butt_through: ButtThrough::Shorter, ..CornerDefaults::mitre() }).unwrap(),
        r#"{"within_group":"mitre","butt_through":"shorter"}"#
    );
}
