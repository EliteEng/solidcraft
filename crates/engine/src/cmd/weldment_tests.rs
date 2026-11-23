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

/// Body `name`'s exact box: min and max.
fn bbox(s: &mut Session, name: &str) -> ([f64; 3], [f64; 3]) {
    let r = run(s, "inspect.measure", json!({"bodies": [name]}));
    let b = &r["bodies"][0]["bbox"];
    let v = |v: &Value| [f(&v[0]), f(&v[1]), f(&v[2])];
    (v(&b["min"]), v(&b["max"]))
}

#[test]
fn a_member_end_is_trimmed_on_a_plane_with_its_gap_and_keeps_its_name() {
    let mut s = Session::default();
    rail(&mut s);
    let plane = json!({"origin": [500, 0, 0], "normal": [1, 0, 0]});
    let fid = run(&mut s, "frame.trim", json!({"members": [{"body": "Rail", "near": [590, 0, 0]}], "plane": plane, "gap": 2}))["feature"].clone();
    let (lo, hi) = bbox(&mut s, "Rail");
    assert!((lo[0] - 0.0).abs() < 1e-6 && (hi[0] - 498.0).abs() < 1e-6, "{lo:?} {hi:?}");
    let list = run(&mut s, "frame.cut_list", json!({}));
    assert!((f(&list["items"][0]["length"]) - 498.0).abs() < 1e-6, "the cut list takes the trim: {list}");
    // The other end, by name, is left alone; editing the trim to the start moves it there.
    let plane = json!({"origin": [100, 0, 0], "normal": [1, 0, 0]});
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": fid, "command": "frame.trim", "params": {"members": [{"body": "Rail", "end": "start"}], "plane": plane}}),
    );
    let (lo, hi) = bbox(&mut s, "Rail");
    assert!((lo[0] - 100.0).abs() < 1e-6 && (hi[0] - 600.0).abs() < 1e-6, "{lo:?} {hi:?}");
}

#[test]
fn extend_lengthens_an_end_to_a_plane_past_it_and_an_angled_plane_angles_the_end() {
    let mut s = Session::default();
    rail(&mut s);
    let plane = json!({"origin": [700, 0, 0], "normal": [1, 0, 0]});
    // Without extend the plane is past the end, and the member stays as it is.
    run(&mut s, "frame.trim", json!({"members": [{"body": "Rail", "end": "end"}], "plane": plane.clone()}));
    assert!((bbox(&mut s, "Rail").1[0] - 600.0).abs() < 1e-6);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "frame.trim", json!({"members": [{"body": "Rail", "end": "end"}], "plane": plane, "extend": true}));
    assert!((bbox(&mut s, "Rail").1[0] - 700.0).abs() < 1e-6, "{:?}", bbox(&mut s, "Rail"));
    run(&mut s, "edit.undo", json!({}));
    // A plane at 45° about Y through (600, 0, 0): the end leans 45°, from 580 to 620 along X.
    let plane = json!({"origin": [600, 0, 0], "normal": [1, 0, 1]});
    run(&mut s, "frame.trim", json!({"members": [{"body": "Rail", "end": "end"}], "plane": plane, "extend": true}));
    let (_, hi) = bbox(&mut s, "Rail");
    assert!((hi[0] - 620.0).abs() < 1e-6, "{hi:?}");
    let list = run(&mut s, "frame.cut_list", json!({}));
    let item = &list["items"][0];
    assert!((f(&item["length"]) - 620.0).abs() < 1e-6 && (f(&item["short_length"]) - 580.0).abs() < 1e-6, "{list}");
    assert!((f(&item["ends"][0]["angle"]) - 45.0).abs() < 1e-6, "{list}");
}

#[test]
fn a_member_extended_into_another_body_is_coped_by_it() {
    let mut s = Session::default();
    rail(&mut s);
    // A round tube post of another feature, upright at x = 650.
    run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "Post"}));
    run(&mut s, "sketch.line", json!({"points": [[650, -100], [650, 100]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Post", "profile": {"kind": "round_tube", "diameter": 60, "wall": 4}, "body_name": "Post"}));
    run(&mut s, "frame.trim", json!({"members": [{"body": "Rail", "end": "end"}], "tool": "Post", "extend": true}));
    // The rail runs on to the post and is cut by it: its corners reach x = 650 − √(30² − 20²).
    let (_, hi) = bbox(&mut s, "Rail");
    let want = 650.0 - (900.0f64 - 400.0).sqrt();
    assert!((hi[0] - want).abs() < 1e-3, "{hi:?} vs {want}");
    let list = run(&mut s, "frame.cut_list", json!({"feature": "Member1"}));
    let item = &list["items"][0];
    assert!(item["ends"][0]["cope"] == true && (f(&item["length"]) - want).abs() < 1e-3, "{list}");
    // With a 2 mm gap the rail is coped by the post grown to 32 mm radius.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "frame.trim", json!({"members": [{"body": "Rail", "end": "end"}], "tool": "Post", "extend": true, "gap": 2}));
    let (_, hi) = bbox(&mut s, "Rail");
    let want = 650.0 - (1024.0f64 - 400.0).sqrt();
    assert!((hi[0] - want).abs() < 1e-3, "with a gap: {hi:?} vs {want}");
}

#[test]
fn bad_trims_are_refused_with_the_reason() {
    let mut s = Session::default();
    rail(&mut s);
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Other"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 100], "p1": [50, 150]}));
    run(&mut s, "sketch.finish", json!({}));
    let made = run(&mut s, "solid.extrude", json!({"sketch": "Other", "distance": 10}));
    let block = made["created_bodies"][0].as_str().unwrap().to_string();
    let plane = json!({"origin": [500, 0, 0], "normal": [1, 0, 0]});
    let err = |s: &mut Session, p: Value| s.execute("frame.trim", &p).unwrap_err().to_string();
    assert!(err(&mut s, json!({"members": [{"body": "Rail", "end": "end"}]})).contains("one boundary"));
    assert!(
        err(&mut s, json!({"members": [{"body": "Rail", "end": "end"}], "plane": plane.clone(), "tool": block.clone()})).contains("one boundary")
    );
    assert!(err(&mut s, json!({"members": [], "plane": plane.clone()})).contains("members"));
    let e = err(&mut s, json!({"members": [{"body": block, "end": "end"}], "plane": plane.clone()}));
    assert!(e.contains("not a frame member"), "{e}");
    assert!(err(&mut s, json!({"members": [{"body": "Rail"}], "plane": plane.clone()})).contains("`end` or `near`"));
    assert!(
        err(&mut s, json!({"members": [{"body": "Rail", "end": "end"}, {"body": "Rail", "end": "end"}], "plane": plane.clone()})).contains("twice")
    );
    assert!(err(&mut s, json!({"members": [{"body": "Rail", "end": "end"}], "tool": "Rail"})).contains("by itself"));
    let along = json!({"origin": [0, 0, 0], "normal": [0, 1, 0]});
    assert!(err(&mut s, json!({"members": [{"body": "Rail", "end": "end"}], "plane": along})).contains("runs along"));
    let past = json!({"origin": [-50, 0, 0], "normal": [1, 0, 0]});
    assert!(err(&mut s, json!({"members": [{"body": "Rail", "end": "end"}], "plane": past})).contains("past the other end"));
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
fn a_gusset_fills_the_inside_corner_between_two_faces() {
    let mut s = Session::default();
    rectangle(&mut s);
    // The bottom rail's inside face (y = 20) and the right post's (x = 580) meet along x = 580,
    // y = 20: the gusset lies flat (square to Z), its legs running −X and +Y from there.
    let (rail, post) = (body_at(&mut s, [300.0, 20.0, 0.0]), body_at(&mut s, [580.0, 200.0, 0.0]));
    let faces = json!({"first": {"body": rail, "point": [300, 20, 0]}, "second": {"body": post, "point": [580, 200, 0]}});
    let mut p = faces.clone();
    p["d1"] = json!(100);
    p["thickness"] = json!(5);
    let made = run(&mut s, "frame.gusset", p.clone());
    assert_eq!(made["created_bodies"], json!(["Gusset"]), "{made}");
    let (lo, hi) = bbox(&mut s, "Gusset");
    let want = ([480.0, 20.0, -2.5], [580.0, 120.0, 2.5]);
    assert!((0..3).all(|k| (lo[k] - want.0[k]).abs() < 1e-6 && (hi[k] - want.1[k]).abs() < 1e-6), "{lo:?} {hi:?}");
    assert!((volume(&mut s, "Gusset") - 0.5 * 100.0 * 100.0 * 5.0).abs() < 1e-3);
    // A 10 mm corner clip, and a chamfered one with 20 mm off each tip.
    let mut clipped = p.clone();
    clipped["clip"] = json!(10);
    run(&mut s, "frame.gusset", clipped);
    assert!((volume(&mut s, "Gusset (2)") - (5000.0 - 50.0) * 5.0).abs() < 1e-3);
    let mut chamfered = p.clone();
    chamfered["shape"] = json!("chamfered");
    chamfered["chamfer"] = json!(20);
    run(&mut s, "frame.gusset", chamfered);
    assert!((volume(&mut s, "Gusset (3)") - (5000.0 - 400.0) * 5.0).abs() < 1e-3);
    // Moved 10 mm up the line where the faces meet.
    let mut moved = p;
    moved["offset"] = json!(10);
    run(&mut s, "frame.gusset", moved);
    let (lo, hi) = bbox(&mut s, "Gusset (4)");
    assert!((lo[2] - 7.5).abs() < 1e-6 && (hi[2] - 12.5).abs() < 1e-6, "{lo:?} {hi:?}");
}

#[test]
fn bad_gussets_are_refused_with_the_reason() {
    let mut s = Session::default();
    rectangle(&mut s);
    let rail = body_at(&mut s, [300.0, 20.0, 0.0]);
    let top = body_at(&mut s, [300.0, 380.0, 0.0]);
    let err = |s: &mut Session, p: Value| s.execute("frame.gusset", &p).unwrap_err().to_string();
    let parallel = json!({"first": {"body": rail, "point": [300, 20, 0]}, "second": {"body": top, "point": [300, 380, 0]}, "d1": 50, "thickness": 5});
    assert!(err(&mut s, parallel).contains("parallel"));
    let post = body_at(&mut s, [580.0, 200.0, 0.0]);
    let faces = json!({"first": {"body": rail, "point": [300, 20, 0]}, "second": {"body": post, "point": [580, 200, 0]}});
    let with = |k: &str, v: Value| {
        let mut p = faces.clone();
        p["d1"] = json!(50);
        p["thickness"] = json!(5);
        p[k] = v;
        p
    };
    assert!(err(&mut s, with("shape", json!("chamfered"))).contains("chamfer"));
    assert!(err(&mut s, with("clip", json!(60))).contains("clip"));
    assert!(err(&mut s, with("thickness", json!(0))).contains("thickness"));
    assert!(err(&mut s, with("d1", json!(-5))).contains("legs"));
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
