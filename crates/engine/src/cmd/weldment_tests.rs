#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn tube() -> Value {
    json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3})
}

/// One 40 mm tube member `Rail` along X from 0 to 600 (sketch `Frame`).
fn rail(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(s, "sketch.line", json!({"points": [[0, 0], [600, 0]]}));
    run(s, "sketch.finish", json!({}));
    run(s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "body_name": "Rail"}));
}

/// A 600 × 400 rectangle of 40 mm tubes (mitred corners).
fn rectangle(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(s, "sketch.finish", json!({}));
    run(s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "body_name": "Side", "corner": "mitre"}));
}

fn volume(s: &mut Session, name: &str) -> f64 {
    let r = run(s, "inspect.measure", json!({"bodies": [name]}));
    f(&r["bodies"][0]["volume_mm3"])
}

/// The name of the body holding the point `p` on its surface (the one whose box holds it).
fn body_at(s: &mut Session, p: [f64; 3]) -> String {
    let r = run(s, "inspect.measure", json!({}));
    r["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| (0..3).all(|k| f(&b["bbox"]["min"][k]) - 1e-6 <= p[k] && p[k] <= f(&b["bbox"]["max"][k]) + 1e-6))
        .map(|b| b["name"].as_str().unwrap().to_string())
        .unwrap()
}

#[test]
fn a_weld_runs_along_edges_and_counts_in_the_cut_list() {
    let mut s = Session::default();
    rail(&mut s);
    let bare = run(&mut s, "frame.cut_list", json!({}));
    // The edge where the rail's top face meets its rounded front corner (radius 6), y = −14, z = 20,
    // from x = 0 to 600: 600 mm, on both sides, 5 mm leg.
    let made = run(&mut s, "frame.weld", json!({"edges": [{"body": "Rail", "point": [300, -14, 20]}], "size": 5, "both_sides": true}));
    assert!(made["created_bodies"].as_array().unwrap().is_empty(), "a weld makes no body: {made}");
    let list = run(&mut s, "frame.cut_list", json!({"format": "csv"}));
    let w = &list["welds"][0];
    assert!((f(&w["size"]) - 5.0).abs() < 1e-9 && (f(&w["length"]) - 1200.0).abs() < 1e-6, "{list}");
    let density = (f(&bare["items"][0]["mass_each_g"]) * 1000.0) / volume(&mut s, "Rail");
    assert!((f(&w["mass_g"]) - 12.5 * 1200.0 * density / 1000.0).abs() < 1e-6, "{list}");
    assert!((f(&list["total_mass_g"]) - f(&bare["total_mass_g"]) - f(&w["mass_g"])).abs() < 1e-6, "{list}");
    assert!(list["csv"].as_str().unwrap().contains("Weld size (mm),Length welded (mm),Mass (kg)\n5.00,1200.00,"), "{list}");
    // The design's physical properties count it too: its mass at the middle of the edge (where the
    // top face meets the rounded corner, y = −14), so the centre of mass moves towards (300, −14, 20).
    let props = run(&mut s, "component.properties", json!({}));
    let t = &props["total"];
    let rail_mass = f(&bare["total_mass_g"]);
    assert!((f(&t["weld_mass_g"]) - f(&w["mass_g"])).abs() < 1e-6 && (f(&t["mass_g"]) - rail_mass - f(&w["mass_g"])).abs() < 1e-6, "{props}");
    let all = f(&t["mass_g"]);
    let want = [300.0, -14.0 * f(&w["mass_g"]) / all, 20.0 * f(&w["mass_g"]) / all];
    for k in 0..3 {
        assert!((f(&t["center_of_mass"][k]) - want[k]).abs() < 1e-6, "{k}: {props}");
    }
    assert!((f(&t["volume_mm3"]) - volume(&mut s, "Rail") - 12.5 * 1200.0).abs() < 1e-6, "{props}");
    // Stitched 50 every 150 on one side: 600 mm holds stitches at 0, 150, 300, 450 and 550 doesn't fit: 4 × 50.
    let fid = made["feature"].clone();
    let p = json!({"edges": [{"body": "Rail", "point": [300, -14, 20]}], "size": 5, "stitch_length": 50, "stitch_pitch": 150});
    run(&mut s, "timeline.redefine", json!({"feature": fid, "command": "frame.weld", "params": p}));
    let list = run(&mut s, "frame.cut_list", json!({}));
    assert!((f(&list["welds"][0]["length"]) - 200.0).abs() < 1e-6, "{list}");
}

#[test]
fn bad_welds_are_refused_with_the_reason() {
    let mut s = Session::default();
    rail(&mut s);
    let err = |s: &mut Session, p: Value| s.execute("frame.weld", &p).unwrap_err().to_string();
    let edge = json!([{"body": "Rail", "point": [300, -14, 20]}]);
    assert!(err(&mut s, json!({"edges": [], "size": 5})).contains("edges"));
    assert!(err(&mut s, json!({"edges": [{"body": "Nope", "point": [0, 0, 0]}], "size": 5})).contains("no body"));
    assert!(err(&mut s, json!({"edges": edge.clone(), "size": 5, "stitch_length": 50})).contains("stitch"));
    assert!(err(&mut s, json!({"edges": edge.clone(), "size": 5, "stitch_length": 50, "stitch_pitch": 20})).contains("pitch"));
    assert!(err(&mut s, json!({"edges": edge, "size": 0})).contains("size"));
    assert!(err(&mut s, json!({"edges": [{"body": "Rail", "point": [300, 0, 0]}], "size": 5})).contains("no edge"));
}

/// A post exported as STEP lies along X from 0, its section across Y and Z; the file reads back.
#[test]
fn a_member_exports_as_step_along_x_from_zero() {
    let mut s = Session::default();
    rectangle(&mut s);
    let post = body_at(&mut s, [580.0, 200.0, 0.0]);
    let out = run(&mut s, "frame.export_member_step", json!({"body": post}));
    assert!((f(&out["length"]) - 440.0).abs() < 1e-6, "a mitred 400 mm post of 40 mm tube: {}", out["length"]);
    let text = out["text"].as_str().unwrap();
    assert!(text.starts_with("ISO-10303-21"), "{}", &text[..40.min(text.len())]);
    let back = crate::kernel::step_import(text).unwrap();
    let bb = back.bodies[0].body.exact_bounds().unwrap();
    assert!((bb.min.x).abs() < 1e-6 && (bb.max.x - 440.0).abs() < 1e-6, "{bb:?}");
    assert!((bb.max.y - bb.min.y - 40.0).abs() < 1e-6 && (bb.max.z - bb.min.z - 40.0).abs() < 1e-6, "{bb:?}");
    assert!(s.execute("frame.export_member_step", &json!({"body": "nope"})).unwrap_err().to_string().contains("no body"));
}

/// A rail with a hole drilled through its top: the DXF has Top, Front and Bottom views stacked,
/// the length along X, the hole's circle in the top and bottom views, and the mitres' slopes in the
/// front view; lines and arcs only, no mesh.
#[test]
fn a_member_exports_as_a_dxf_of_its_top_front_and_bottom() {
    let mut s = Session::default();
    rectangle(&mut s);
    let rail = body_at(&mut s, [300.0, 20.0, 0.0]);
    run(&mut s, "solid.hole", json!({"position": [300, 0, 20], "direction": [0, 0, -1], "diameter": 10, "depth": 40}));
    let out = s.execute("frame.export_member_dxf", &json!({"body": rail, "top": [300, 0, 20], "front": [300, -20, 0]}));
    let out = match out {
        Ok(v) => v,
        Err(e) => panic!("{e}"),
    };
    assert!((f(&out["length"]) - 640.0).abs() < 1e-6, "{}", out["length"]);
    let views: Vec<&str> = out["views"].as_array().unwrap().iter().map(|v| v["view"].as_str().unwrap()).collect();
    assert_eq!(views, ["top", "front", "bottom"]);
    // Stacked: the bottom view lowest, the top highest.
    let at = |k: usize| f(&out["views"][k]["at"][1]);
    assert!(at(2) < at(1) && at(1) < at(0), "{}", out["views"]);
    let text = out["text"].as_str().unwrap();
    let kinds: std::collections::BTreeMap<&str, usize> =
        text.lines().collect::<Vec<_>>().windows(2).filter(|w| w[0].trim() == "0").fold(Default::default(), |mut m, w| {
            *m.entry(w[1].trim()).or_default() += 1;
            m
        });
    assert!(kinds.contains_key("CIRCLE") || kinds.contains_key("ARC"), "the hole is drawn as a circle: {kinds:?}");
    assert!(!kinds.contains_key("ELLIPSE") && !kinds.contains_key("SPLINE"), "lines and arcs only: {kinds:?}");
    assert!(text.contains("TOP") && text.contains("FRONT") && text.contains("BOTTOM"));
    let e = s.execute("frame.export_member_dxf", &json!({"body": rail, "top": [300, 0, 20], "front": [300, 0, 20]})).unwrap_err().to_string();
    assert!(e.contains("square"), "{e}");
}

/// The NC1 file's blocks: each block's code and its data lines split into words.
fn nc1_blocks(text: &str) -> Vec<(String, Vec<Vec<String>>)> {
    let mut out: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    for line in text.lines() {
        if line.starts_with("**") {
            continue;
        }
        if !line.starts_with(' ') {
            out.push((line.trim().to_string(), Vec::new()));
        } else if let Some(b) = out.last_mut() {
            b.1.push(line.split_whitespace().map(str::to_string).collect());
        }
    }
    out
}

/// An outline block's face and its points, the face letter and the `u` reference taken off the
/// first point.
fn nc1_points(lines: &[Vec<String>]) -> (char, Vec<(f64, f64)>) {
    let face = lines[0][0].chars().next().unwrap();
    let pts = lines
        .iter()
        .enumerate()
        .map(|(k, w)| {
            let text = w.join(" ");
            let text = if k == 0 { &text[1..] } else { &text[..] };
            let nums: Vec<f64> = text.replace('u', " ").split_whitespace().map(|n| n.parse().unwrap()).collect();
            (nums[0], nums[1])
        })
        .collect();
    (face, pts)
}

/// A mitred rail of 40 mm square tube drilled through its top: an `M` piece 640 mm long, its four
/// walls laid out as plates, the walls the mitres cross trapezoids 640 long at one edge and 560 at
/// the other (the side walls square at 0 and 640, and at 40 and 600), the two ends' skew cuts 45° the
/// opposite ways round, and the hole through both walls.
#[test]
fn a_member_exports_as_nc1_with_its_outlines_skew_cuts_and_holes() {
    let mut s = Session::default();
    rectangle(&mut s);
    let rail = body_at(&mut s, [300.0, 20.0, 0.0]);
    run(&mut s, "solid.hole", json!({"position": [300, 0, 20], "direction": [0, 0, -1], "diameter": 10, "depth": 40}));
    run(&mut s, "material.assign", json!({"bodies": [rail], "material": "Steel"}));
    let out = run(&mut s, "frame.export_member_nc1", json!({"body": rail, "order": "J1", "drawing": "D7", "quantity": 4}));
    assert!((f(&out["length"]) - 640.0).abs() < 1e-6, "{}", out["length"]);
    assert_eq!(out["profile_code"], "M");
    assert_eq!(out["faces"], json!(["v", "o", "u", "h"]));
    assert_eq!(out["holes"], 2, "the hole goes through the top wall and the bottom one");
    let text = out["text"].as_str().unwrap();
    let blocks = nc1_blocks(text);
    let (st, header) = &blocks[0];
    assert_eq!(st, "ST");
    let words: Vec<&str> = header.iter().map(|w| w.first().map_or("", String::as_str)).collect();
    // Order, drawing, phase, piece, grade, quantity, profile, code, length, height, width, the
    // two thicknesses and the radius.
    assert_eq!(&words[..3], &["J1", "D7", "-"]);
    assert_eq!(words[3], rail);
    assert_eq!(words[4], "Steel", "the grade is the member's material");
    assert_eq!(words[5], "4");
    assert_eq!(words[7], "M");
    assert_eq!(&words[8..14], &["640.00", "40.00", "40.00", "3.00", "3.00", "6.00"]);
    // kg/m of steel (7.85 g/cm³) and paint area: the section's area and outside.
    let kg: f64 = words[14].parse().unwrap();
    assert!(kg > 3.0 && kg < 3.5, "40x40x3 tube is about 3.3 kg/m: {kg}");
    let cuts: Vec<f64> = words[16..20].iter().map(|w| w.parse().unwrap()).collect();
    let (web, flange) = ((cuts[0], cuts[1]), (cuts[2], cuts[3]));
    let mitred = if web.0.abs() > 1.0 { web } else { flange };
    let square = if web.0.abs() > 1.0 { flange } else { web };
    assert!((mitred.0.abs() - 45.0).abs() < 1e-3 && (mitred.0 + mitred.1).abs() < 1e-3, "{cuts:?}");
    assert!(square.0.abs() < 1e-6 && square.1.abs() < 1e-6, "{cuts:?}");
    let mut trapezoids = 0;
    for (code, lines) in blocks.iter().filter(|b| b.0 == "AK") {
        let (face, pts) = nc1_points(lines);
        assert_eq!(pts.first(), pts.last(), "{code} on {face} is closed");
        let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
        assert!(ys.iter().all(|y| y.abs() < 1e-9 || (y - 40.0).abs() < 1e-9), "each wall is laid out its full 40 mm wide: {pts:?}");
        let mut xs: Vec<i64> = pts.iter().map(|p| p.0.round() as i64).collect();
        xs.sort_unstable();
        xs.dedup();
        if xs == [0, 40, 600, 640] {
            trapezoids += 1;
        } else {
            // The side walls: the mitres are square to them, at the outer corner and the inner one.
            assert!(xs == [0, 640] || xs == [40, 600], "a wall's outline: {pts:?}");
        }
    }
    assert_eq!(trapezoids, 2, "the mitres show on two walls");
    let bo = &blocks.iter().find(|b| b.0 == "BO").unwrap().1;
    // 320 along the piece: its outer corner starts 20 mm before the hole's x = 300 (the mitre).
    for hole in bo {
        assert!(hole[1] == "320.00u" && hole[2] == "20.00" && hole[3] == "10.00", "{hole:?}");
    }
    assert_eq!(blocks.last().unwrap().0, "EN");
}

/// An angle lies with one leg flat (`v`) and the other standing at the zero point (`u`); a round
/// tube's outside unrolls on `v`, its perimeter across.
#[test]
fn open_and_round_sections_export_as_nc1_on_the_standards_faces() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [500, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    let angle = json!({"kind": "angle", "width": 60, "height": 40, "thickness": 5});
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": angle, "body_name": "Leg"}));
    let leg = body_at(&mut s, [250.0, 0.0, 0.0]);
    let out = run(&mut s, "frame.export_member_nc1", json!({"body": leg}));
    assert_eq!(out["profile_code"], "L");
    assert_eq!(out["faces"], json!(["v", "u"]));
    let blocks = nc1_blocks(out["text"].as_str().unwrap());
    let words: Vec<&str> = blocks[0].1.iter().map(|w| w.first().map_or("", String::as_str)).collect();
    assert_eq!(&words[8..13], &["500.00", "60.00", "40.00", "5.00", "5.00"]);
    for (_, lines) in blocks.iter().filter(|b| b.0 == "AK") {
        let (face, pts) = nc1_points(lines);
        let wide = if face == 'v' { 60.0 } else { 40.0 };
        assert!(
            pts.iter().all(|p| (p.0.abs() < 1e-9 || (p.0 - 500.0).abs() < 1e-9) && (p.1.abs() < 1e-9 || (p.1 - wide).abs() < 1e-9)),
            "{face}: {pts:?}"
        );
        assert_eq!(pts.len(), 5, "{face}: a rectangle, closed: {pts:?}");
    }

    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [300, 0]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": {"kind": "round_tube", "diameter": 48.3, "wall": 3.2}, "body_name": "Pipe"}));
    let pipe = body_at(&mut s, [150.0, 0.0, 0.0]);
    let out = run(&mut s, "frame.export_member_nc1", json!({"body": pipe}));
    assert_eq!(out["profile_code"], "RO");
    let blocks = nc1_blocks(out["text"].as_str().unwrap());
    let (_, lines) = blocks.iter().find(|b| b.0 == "AK").unwrap();
    let (face, pts) = nc1_points(lines);
    assert_eq!(face, 'v');
    let round = std::f64::consts::PI * 48.3;
    assert!(pts.iter().all(|p| p.0.abs() < 1e-9 || (p.0 - 300.0).abs() < 1e-9), "{pts:?}");
    let top = pts.iter().map(|p| p.1).fold(0.0, f64::max);
    assert!((top - round).abs() < 0.01, "the perimeter across: {top} vs {round}");
    let e = s.execute("frame.export_member_nc1", &json!({"body": "nope"})).unwrap_err().to_string();
    assert!(e.contains("nope"), "{e}");
}
