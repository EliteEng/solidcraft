use crate::test_support::run_all;
use serde_json::json;
use solidcraft_engine::Session;
use solidcraft_engine::geom::prelude::*;

use super::*;
use crate::Services;
use crate::dialogs::{Dialog, apply_commands, for_feature as edit_dialog};

fn app_with(cmds: &[(&str, Value)]) -> SolveApp {
    let mut s = Session::default();
    for (id, p) in cmds {
        s.execute(id, p).unwrap();
    }
    SolveApp::new(s, Services::default())
}

/// A 600 × 400 rectangle on XY as the frame sketch `Frame`, and, drawn after it on YZ, a ring of
/// two circles as the section sketch `Ring` (its circles' ids are not the frame's line ids).
fn frame_app() -> SolveApp {
    app_with(&[
        ("sketch.create", json!({"plane": "XY", "name": "Frame"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]})),
        ("sketch.finish", json!({})),
        ("sketch.create", json!({"plane": "YZ", "name": "Ring"})),
        ("sketch.circle.center", json!({"center": [0, 0], "radius": 20})),
        ("sketch.circle.center", json!({"center": [0, 0], "radius": 17})),
        ("sketch.finish", json!({})),
    ])
}

fn sketch_id(app: &SolveApp, name: &str) -> u64 {
    app.session.doc.find_feature(name).unwrap().id
}

fn frame_lines(app: &SolveApp) -> Vec<String> {
    app.session.doc.sketch(sketch_id(app, "Frame")).unwrap().curves.iter().map(|c| c.id.clone()).collect()
}

fn body_names(app: &SolveApp) -> Vec<String> {
    crate::open_meshing::session_world(&app.session).bodies.iter().map(|b| b.name.clone()).collect()
}

/// The dialog started with the given lines picked.
fn dialog_on(app: &mut SolveApp, lines: &[String]) -> Dialog {
    app.session.selection = lines.iter().map(|id| Sel::SketchCurve { id: id.clone(), sketch: None }).collect();
    app.start("frame.member");
    app.dialog.clone().unwrap()
}

fn member(d: &mut Dialog) -> &mut Member {
    match &mut d.kind {
        Kind::Frame(m) => m,
        k => panic!("{k:?}"),
    }
}

/// Picked lines and the default built-in profile make one member a line; Edit Feature refills
/// the dialog and runs `timeline.redefine`, and the members keep their names.
#[test]
fn members_from_picked_lines_and_a_built_in_profile() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines[..2]);
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].0, "frame.member");
    let p = &c[0].1;
    assert_eq!(p["sketch"], json!(sketch_id(&app, "Frame")), "the frame sketch, not the newer section sketch: {p}");
    assert_eq!(p["lines"], json!(lines[..2]));
    assert_eq!(p["profile"], json!({"kind": "rect_tube", "width": "40 mm", "height": "40 mm", "wall": "3 mm"}));
    assert!(p.get("anchor").is_none() && p.get("angle").is_none() && p.get("mirror").is_none() && p.get("body_name").is_none(), "{p}");
    // Another built-in profile and its dimensions; an optional one left empty takes its default.
    let m = member(&mut d);
    m.kind = frame::profile_kinds().iter().position(|k| k.kind == "angle").unwrap();
    m.dims[m.kind][2] = "4 mm".into();
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["profile"], json!({"kind": "angle", "width": "50 mm", "height": "50 mm", "thickness": "4 mm"}));
    run_all(&mut app, c);
    assert_eq!(body_names(&app), ["Angle 50x50x4", "Angle 50x50x4 (2)"]);
    app.run("body.rename", json!({"body": "Angle 50x50x4 (2)", "name": "Rail"})).unwrap();
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    assert_eq!(e.inputs[0].items.len(), 2);
    let m = member(&mut e);
    assert_eq!((frame::profile_kinds()[m.kind].kind, m.dims[m.kind][2].as_str()), ("angle", "4 mm"));
    m.dims[m.kind][2] = "5 mm".into();
    let c = apply_commands(&app, &e).unwrap();
    assert_eq!(c[0].0, "timeline.redefine");
    run_all(&mut app, c);
    assert_eq!(body_names(&app), ["Angle 50x50x4", "Rail"], "the members keep their names through the edit");
}

/// A section picked in another sketch is the profile, the picks naming its regions; All Lines,
/// anchor, angle, mirror and name reach the command and come back when editing.
#[test]
fn a_picked_section_and_the_placement() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let ring = sketch_id(&app, "Ring");
    let mut d = dialog_on(&mut app, &lines[..1]);
    // The ring between the circles: the region whose interior point lies between radii 17 and 20.
    let ss = crate::open_meshing::session_world(&app.session).sketch(ring).unwrap().clone();
    let index = ss.profiles.iter().position(|p| (p.region.interior_point().len() - 18.5).abs() < 1.5).unwrap();
    d.inputs[1].items = vec![Sel::Profile { sketch: ring, index }];
    let m = member(&mut d);
    m.all = true;
    m.anchor = ANCHORS.iter().position(|a| a.0 == "bottom").unwrap();
    m.angle = "15 deg".into();
    m.mirror = true;
    m.name = " Rail ".into();
    let c = apply_commands(&app, &d).unwrap();
    let p = &c[0].1;
    assert_eq!(p["sketch"], json!(sketch_id(&app, "Frame")));
    assert!(p.get("lines").is_none() && p.get("profile").is_none(), "{p}");
    assert_eq!((&p["profile_sketch"], &p["profiles"]), (&json!(ring), &json!([index])));
    assert_eq!((&p["anchor"], &p["angle"], &p["mirror"], &p["body_name"]), (&json!("bottom"), &json!("15 deg"), &json!(true), &json!("Rail")));
    run_all(&mut app, c);
    assert_eq!(body_names(&app), ["Rail", "Rail (2)", "Rail (3)", "Rail (4)"]);
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    assert_eq!(e.inputs[0].items.len(), 4, "every line of the sketch shows as picked");
    assert_eq!(e.inputs[1].items, [Sel::Profile { sketch: ring, index }]);
    let m = member(&mut e);
    assert!(m.all && m.mirror && m.name == "Rail" && m.angle == "15 deg" && ANCHORS[m.anchor].0 == "bottom", "{m:?}");
}

/// A member whose section is a whole profile sketch keeps it when edited: the dialog shows the
/// sketch rather than picking every region of it, which would fill the tube's hole.
#[test]
fn a_whole_sketch_section_stays_whole_through_an_edit() {
    let mut app = frame_app();
    app.run("frame.member", json!({"sketch": "Frame", "profile_sketch": "Ring"})).unwrap();
    let volume = |app: &mut SolveApp| app.session.execute("inspect.measure", &json!({})).unwrap()["bodies"][0]["volume_mm3"].as_f64().unwrap();
    let before = volume(&mut app);
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    assert!(e.inputs[1].items.is_empty());
    assert_eq!(member(&mut e).whole, Some(sketch_id(&app, "Ring")));
    member(&mut e).angle = "30 deg".into();
    let c = apply_commands(&app, &e).unwrap();
    assert!(c[0].1["params"].get("profiles").is_none(), "{}", c[0].1);
    run_all(&mut app, c);
    // To the accuracy SMLib measures at (1e-4 relative, the finest it takes: kernel
    // `measure::ACCURACY`); the ring's exact π (20² − 17²) 600 = 209230.07 mm³ came out
    // 209229.86 before the turn and 209230.43 after it. The filled circle would be 3.6 times it.
    let after = volume(&mut app);
    assert!((after - before).abs() < 1e-4 * before, "still the ring, not the filled circle: {after} mm³, {before} before");
    // Unticked, the section is a built-in profile again.
    let mut e = edit_dialog(&app, f, None).unwrap();
    member(&mut e).whole = None;
    assert_eq!(apply_commands(&app, &e).unwrap()[0].1["params"]["profile"]["kind"], "rect_tube");
}

/// A member placed by a profile point (`locate`, which the dialog has no field for) keeps it
/// through an edit, so it stays attached where it was.
#[test]
fn an_edit_keeps_a_located_point() {
    let mut app = frame_app();
    let profile = json!({"kind": "angle", "width": 60, "height": 40, "thickness": 5});
    app.run("frame.member", json!({"sketch": "Frame", "profile": profile, "locate": [10, 5]})).unwrap();
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    member(&mut e).angle = "90 deg".into();
    let c = apply_commands(&app, &e).unwrap();
    assert_eq!(c[0].1["params"]["locate"], json!([10.0, 5.0]), "{}", c[0].1);
    run_all(&mut app, c);
    let FeatureKind::FrameMember { locate, .. } = &app.session.doc.feature(f).unwrap().kind else { panic!() };
    assert_eq!(*locate, Some([10.0, 5.0]));
}

/// Without lines there is nothing to place; lines and the section in one sketch are refused.
#[test]
fn missing_or_mixed_picks_are_refused() {
    let mut app = frame_app();
    let d = dialog_on(&mut app, &[]);
    assert!(apply_commands(&app, &d).unwrap_err().contains("lines"));
    let ring = sketch_id(&app, "Ring");
    let circles: Vec<String> = app.session.doc.sketch(ring).unwrap().curves.iter().map(|c| c.id.clone()).collect();
    let mut d = dialog_on(&mut app, &circles[..1]);
    d.inputs[1].items = vec![Sel::Profile { sketch: ring, index: 0 }];
    assert!(apply_commands(&app, &d).is_err());
}

/// Lines picked in the view keep their sketch: two 3D sketches both number their lines `w1`…, and
/// the members go on the lines that were clicked, not on the newer sketch's lines of those ids.
#[test]
fn picked_lines_stay_in_the_sketch_they_were_picked_in() {
    let mut app = app_with(&[
        ("sketch.create", json!({"plane": "XY", "name": "Uprights"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 500]]})),
        ("sketch.line3d", json!({"points": [[300, 0, 0], [300, 0, 500]]})),
        ("sketch.finish", json!({})),
        ("sketch.create", json!({"plane": "XY", "name": "Rails"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[0, 0, 250], [300, 0, 250]]})),
        ("sketch.line3d", json!({"points": [[0, 0, 480], [300, 0, 480]]})),
        ("sketch.finish", json!({})),
    ]);
    let (uprights, rails) = (sketch_id(&app, "Uprights"), sketch_id(&app, "Rails"));
    let wires = |id: u64| -> Vec<String> { app.session.doc.sketch(id).unwrap().wires.iter().map(|w| w.id.clone()).collect() };
    let ids = wires(uprights);
    assert_eq!(ids, wires(rails), "the two sketches share their line ids");
    app.start("frame.member");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = ids.iter().map(|id| Sel::ExternalSketchCurve { sketch: uprights, id: id.clone() }).collect();
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["sketch"], json!(uprights));
    run_all(&mut app, c.clone());
    // Two posts the height of the uprights, not two rails.
    let m = app.session.execute("inspect.measure", &json!({})).unwrap();
    for b in m["bodies"].as_array().unwrap() {
        let (lo, hi) = (b["bbox"]["min"][2].as_f64().unwrap(), b["bbox"]["max"][2].as_f64().unwrap());
        assert!(lo.abs() < 1e-6 && (hi - 500.0).abs() < 1e-6, "{b}");
    }
    // Lines picked in two sketches are refused.
    d.inputs[0].items.push(Sel::ExternalSketchCurve { sketch: rails, id: ids[0].clone() });
    assert!(apply_commands(&app, &d).unwrap_err().contains("one sketch"));
}

/// Editing an end cap shows a face for each saved end. When one cannot be found, the faces start
/// empty and OK names the missing end instead of dropping it unasked; what is picked again is taken,
/// even fewer ends.
#[test]
fn an_end_cap_edit_refuses_to_drop_an_end_it_cannot_find() {
    let tube = json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3, "outer_radius": 0});
    let mut app = app_with(&[
        ("sketch.create", json!({"plane": "XY", "name": "Post path"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 300]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Post path", "body_name": "Post", "profile": tube})),
    ]);
    let end = |app: &SolveApp, z: f64| {
        let st = app.session.model.state();
        let b = st.body("Post").unwrap();
        let i = (0..b.body.face_count())
            .find(|i| b.body.planar_face(*i).is_some_and(|pl| pl.normal().z.abs() > 0.999 && (pl.origin.z - z).abs() < 1e-6))
            .unwrap();
        Sel::Face { body: "Post".into(), index: i, point: solidcore_record::Vec3::new(18.5, 0.0, z) }
    };
    let (top, bottom) = (end(&app, 300.0), end(&app, 0.0));
    let faces: Vec<Value> = [&top, &bottom]
        .iter()
        .map(|s| match s {
            Sel::Face { body, index, point } => json!({"body": body, "index": index, "face": point}),
            _ => Value::Null,
        })
        .collect();
    app.session.execute("frame.end_cap", &json!({"faces": faces, "thickness": 6})).unwrap();
    let kind = app.session.doc.find_feature("EndCap1").unwrap().kind.clone();
    let (crate::dialogs::Kind::EndCap(c), inputs, extra) = crate::dialogs_end_cap::for_feature(&app, &kind).unwrap() else { panic!() };
    assert!(c.unresolved.is_empty() && inputs[0].items.len() == 2, "{c:?}");
    assert!(crate::dialogs_end_cap::commands(&app, &c, &inputs, &extra).is_ok());
    // The second end could not be found: nothing is picked, and OK says which end is missing.
    let c = crate::dialogs_end_cap::EndCap { unresolved: vec![2], ..c };
    let mut inputs = inputs;
    inputs[0].items.clear();
    let e = crate::dialogs_end_cap::commands(&app, &c, &inputs, &extra).unwrap_err();
    assert!(e.contains('2'), "{e}");
    // Picked again, the remaining end alone is a deliberate choice, as are both.
    inputs[0].items = vec![top.clone()];
    assert!(crate::dialogs_end_cap::commands(&app, &c, &inputs, &extra).is_ok());
    inputs[0].items = vec![top, bottom];
    assert!(crate::dialogs_end_cap::commands(&app, &c, &inputs, &extra).is_ok());
}

/// A 300 mm post up Z, a 40 × 40 square tube named `Post`, and a `Ref` sketch line on XY at 30°.
fn post_commands() -> Vec<(&'static str, Value)> {
    let tube = json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3, "outer_radius": 0});
    vec![
        ("sketch.create", json!({"plane": "XY", "name": "Post path"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 300]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Post path", "body_name": "Post", "profile": tube})),
        ("sketch.create", json!({"plane": "XY", "name": "Ref"})),
        ("sketch.line", json!({"points": [[100, 0], [100.0 + 100.0 * 3f64.sqrt() / 2.0, 50]], "ids": ["ref"]})),
        ("sketch.finish", json!({})),
    ]
}

fn post_app() -> SolveApp {
    app_with(&post_commands())
}

fn post_harness() -> crate::scenario::Harness {
    let mut h = crate::scenario::Harness::new();
    for (id, p) in post_commands() {
        h.app.run(id, p).unwrap();
    }
    h
}

/// The post's outer side facing +X, as the view picks it, and as the command takes it.
fn post_side(app: &SolveApp) -> (Sel, Value) {
    let st = app.session.model.state();
    let b = st.body("Post").unwrap();
    let i = (0..b.body.face_count())
        .find(|i| b.body.planar_face(*i).is_some_and(|pl| pl.normal().x > 0.999 && (pl.origin.x - 20.0).abs() < 1e-6))
        .unwrap();
    let point = solidcore_record::Vec3::new(20.0, 0.0, 150.0);
    (Sel::Face { body: "Post".into(), index: i, point }, json!({"face": point, "body": "Post", "index": i}))
}

fn rotate(d: &mut Dialog) -> &mut crate::dialogs_rotate_member::RotateMember {
    match &mut d.kind {
        Kind::RotateMember(r) => r,
        k => panic!("{k:?}"),
    }
}

/// The dialog's picks: as each frame's rows see them.
fn sync(app: &SolveApp, d: &mut Dialog) {
    let mut inputs = d.inputs.clone();
    crate::dialogs_rotate_member::sync(app, rotate(d), &mut inputs);
    d.inputs = inputs;
}

/// The dial of an angle-section post: on the post's profile's box centre (off its line: the line
/// is on a corner), at mid-length, along the line.
#[test]
fn rotate_member_dial_sits_on_the_turn_pivot() {
    let angle = json!({"kind": "angle", "width": 60, "height": 40, "thickness": 5});
    let app = app_with(&[
        ("sketch.create", json!({"plane": "XY", "name": "Post path"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 300]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Post path", "body_name": "Post", "profile": angle, "anchor": "bottom_left"})),
    ]);
    // The post runs up Z, so its box centre is the profile's box centre at mid-length.
    let centre = app.session.model.state().body("Post").unwrap().mesh().bounds().center();
    assert!((centre.z - 150.0).abs() < 1e-6 && centre.x.hypot(centre.y) > 1.0, "an angle's box centre is off its line: {centre:?}");
    let st = app.session.model.state();
    let b = st.body("Post").unwrap();
    let index = (0..b.body.face_count()).find(|i| b.body.planar_face(*i).is_some()).unwrap();
    let mut inputs = crate::dialogs_rotate_member::start(&app, "frame.rotate_member").unwrap().1;
    inputs[0].items = vec![Sel::Face { body: "Post".into(), index, point: centre }];
    let (at, along) = crate::dialogs_rotate_member::dial(&app, &inputs).expect("a dial on the picked post");
    assert!(at.dist(centre) < 1e-6, "dial centre {at:?}, the turn's pivot {centre:?}");
    assert!(along.cross(solidcore_record::Vec3::Z).len() < 1e-9 && along.z > 0.0, "dial axis {along:?}, the post's line +Z");
}

/// No dial on a member inside a component, moved off the origin: the command refuses those
/// members (not supported yet), so the dial would set an angle OK can't use.
#[test]
fn rotate_member_no_dial_on_a_member_in_a_component() {
    let angle = json!({"kind": "angle", "width": 60, "height": 40, "thickness": 5});
    let mut app = app_with(&[
        ("component.create", json!({"name": "Frame", "grounded": false})),
        ("sketch.create", json!({"plane": "XY", "name": "Post path"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[0, 0, 0], [0, 0, 300]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Post path", "body_name": "Post", "profile": angle, "anchor": "bottom_left"})),
    ]);
    let frame = app.session.doc.find_component("Frame").unwrap();
    let occurrence = app.session.doc.occurrence_of(frame).unwrap().id;
    app.run("occurrence.move", json!({"occurrence": occurrence, "axis": [1, 2, 3], "angle": "37 deg", "translate": [50, 20, 10]})).unwrap();
    let world = crate::open_meshing::session_world(&app.session);
    let body = world.bodies.iter().find(|b| b.name.contains("Post")).unwrap_or_else(|| panic!("{:?}", body_names(&app)));
    let index = (0..body.body.face_count()).find(|i| body.body.planar_face(*i).is_some()).unwrap();
    let mut inputs = crate::dialogs_rotate_member::start(&app, "frame.rotate_member").unwrap().1;
    inputs[0].items = vec![Sel::Face { body: body.name.clone(), index, point: body.mesh().bounds().center() }];
    assert!(crate::dialogs_rotate_member::dial(&app, &inputs).is_none());
}

fn turns(app: &SolveApp) -> Vec<String> {
    app.session
        .doc
        .features
        .iter()
        .filter_map(|f| match &f.kind {
            FeatureKind::FrameMemberTurn { angle, .. } => Some(angle.to_string()),
            _ => None,
        })
        .collect()
}

/// The picked member and the typed angle make one `frame.rotate_member`.
#[test]
fn rotate_member_dialog_emits_its_command() {
    let mut app = post_app();
    let (side, face) = post_side(&app);
    app.start("frame.rotate_member");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![side];
    rotate(&mut d).angle = "30 deg".into();
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c, [("frame.rotate_member".to_string(), json!({"face": face, "angle": "30 deg"}))]);
    run_all(&mut app, c);
    assert_eq!(turns(&app), ["30 deg"]);
}

/// A Parallel To pick that names no one line (a curve id in two sketches) says so on OK, rather
/// than OK leaving it out.
#[test]
fn rotate_member_dialog_reports_an_unresolved_parallel_to() {
    let mut app = post_app();
    app.run("sketch.create", json!({"plane": "XY", "name": "Ref 2"})).unwrap();
    app.run("sketch.line", json!({"points": [[0, 100], [50, 150]], "ids": ["ref"]})).unwrap();
    app.run("sketch.finish", json!({})).unwrap();
    let (side, _) = post_side(&app);
    app.start("frame.rotate_member");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![side];
    d.inputs[1].items = vec![Sel::SketchCurve { id: "ref".into(), sketch: None }];
    let e = apply_commands(&app, &d).unwrap_err();
    assert!(e.contains("only one sketch"), "{e}");
}

/// Edit Feature opens on the step's member and angle, and runs `timeline.redefine` even with the
/// angle unchanged.
#[test]
fn rotate_member_edit_opens_with_its_member_and_angle() {
    let mut app = post_app();
    let (_, face) = post_side(&app);
    app.run("frame.rotate_member", json!({"face": face, "angle": "30 deg"})).unwrap();
    let f = app.session.doc.features.iter().find(|f| matches!(f.kind, FeatureKind::FrameMemberTurn { .. })).unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    assert!(matches!(e.inputs[0].items.as_slice(), [Sel::Face { body, .. }] if body == "Post"), "{:?}", e.inputs[0].items);
    assert_eq!(rotate(&mut e).angle, "30 deg");
    sync(&app, &mut e);
    assert_eq!(rotate(&mut e).angle, "30 deg", "the step's own angle stays");
    assert_eq!(apply_commands(&app, &e).unwrap()[0].0, "timeline.redefine", "an edit back to the current turn still runs");
    rotate(&mut e).angle = "45 deg".into();
    let c = apply_commands(&app, &e).unwrap();
    run_all(&mut app, c);
    assert_eq!(turns(&app), ["45 deg"]);
}

/// OK on a new step at the member's current turn closes without adding one.
#[test]
fn rotate_member_dialog_same_angle_runs_nothing() {
    let mut h = post_harness();
    let (side, face) = post_side(&h.app);
    h.app.start("frame.rotate_member");
    let mut d = h.app.dialog.clone().unwrap();
    d.inputs[0].items = vec![side.clone()];
    sync(&h.app, &mut d);
    assert_eq!(rotate(&mut d).angle, "0 deg", "a member never turned shows 0");
    assert_eq!(apply_commands(&h.app, &d).unwrap(), []);
    h.app.run("frame.rotate_member", json!({"face": face, "angle": "30 deg"})).unwrap();
    h.app.start("frame.rotate_member");
    let mut d = h.app.dialog.clone().unwrap();
    d.inputs[0].items = vec![side];
    sync(&h.app, &mut d);
    assert_eq!(rotate(&mut d).angle, "30 deg", "the member's current turn");
    assert_eq!(apply_commands(&h.app, &d).unwrap(), []);
    rotate(&mut d).angle = "45 deg".into();
    assert_eq!(apply_commands(&h.app, &d).unwrap().len(), 1);
    // OK at the current turn closes the dialog and adds nothing.
    rotate(&mut d).angle = "30 deg".into();
    h.app.dialog = Some(d);
    h.frames(2);
    h.step(&json!({"key": "Enter"})).unwrap();
    h.settle();
    assert!(h.app.dialog.is_none());
    assert_eq!(turns(&h.app), ["30 deg"]);
}

/// A parameter at the member's current turn still adds a step: OK on `tilt` (0 deg) on a member
/// never turned adds one, and changing `tilt` turns the member.
#[test]
fn rotate_member_dialog_adds_a_parameter_angle_at_the_current_turn() {
    let mut app = post_app();
    app.run("parameters.add", json!({"name": "tilt", "expression": "0 deg"})).unwrap();
    let (side, _) = post_side(&app);
    app.start("frame.rotate_member");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![side];
    sync(&app, &mut d);
    rotate(&mut d).angle = "tilt".into();
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c.len(), 1, "a step is added: {c:?}");
    run_all(&mut app, c);
    assert_eq!(turns(&app), ["tilt"]);
    let x = |app: &SolveApp| app.session.model.state().body("Post").unwrap().mesh().bounds().max.x;
    assert!((x(&app) - 20.0).abs() < 1e-6, "{}", x(&app));
    app.run("parameters.change", json!({"name": "tilt", "expression": "30 deg"})).unwrap();
    // Turned 30 deg, the 40 x 40 section spans 20 (cos 30 deg + sin 30 deg) each side.
    let half = 20.0 * (30f64.to_radians().cos() + 30f64.to_radians().sin());
    assert!((x(&app) - half).abs() < 1e-6, "{} vs {half}", x(&app));
}

/// The post's design with a second post, `Other`, 100 mm along -X.
fn two_posts_app() -> SolveApp {
    let tube = json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3, "outer_radius": 0});
    let mut cmds = post_commands();
    cmds.extend([
        ("sketch.create", json!({"plane": "XY", "name": "Other path"})),
        ("sketch.options", json!({"sketch_3d": true})),
        ("sketch.line3d", json!({"points": [[-100, 0, 0], [-100, 0, 300]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Other path", "body_name": "Other", "profile": tube})),
    ]);
    app_with(&cmds)
}

/// Edit Feature opens with the member fixed: a click on another member leaves the member, the
/// Angle and the pick as they were.
#[test]
fn rotate_member_edit_keeps_its_member() {
    let mut app = two_posts_app();
    let (_, face) = post_side(&app);
    app.run("frame.rotate_member", json!({"face": face, "angle": "30 deg"})).unwrap();
    let f = app.session.doc.features.iter().find(|f| matches!(f.kind, FeatureKind::FrameMemberTurn { .. })).unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    let member = rotate(&mut e).member.clone();
    let picked = e.inputs[0].items.clone();
    let st = app.session.model.state();
    let other = st.body("Other").unwrap();
    let i = (0..other.body.face_count())
        .find(|i| other.body.planar_face(*i).is_some_and(|pl| pl.normal().x > 0.999 && (pl.origin.x + 80.0).abs() < 1e-6))
        .unwrap();
    e.inputs[0].items = vec![Sel::Face { body: "Other".into(), index: i, point: solidcore_record::Vec3::new(-80.0, 0.0, 150.0) }];
    sync(&app, &mut e);
    assert_eq!(rotate(&mut e).member, member, "the step's member");
    assert_eq!(rotate(&mut e).angle, "30 deg");
    assert_eq!(e.inputs[0].items, picked, "the pick goes back to the step's member");
    // Clearing the pick keeps the step's member and the Angle as typed.
    rotate(&mut e).angle = "45 deg".into();
    e.inputs[0].items.clear();
    sync(&app, &mut e);
    assert_eq!(rotate(&mut e).member, member, "a cleared pick keeps the step's member");
    assert_eq!(e.inputs[0].items, picked, "and puts its pick back");
    assert_eq!(rotate(&mut e).angle, "45 deg");
}

/// Editing a top-level step while another component is active: Parallel To still works the angle
/// out, in the step's own component as `timeline.redefine` runs it.
#[test]
fn rotate_member_edit_works_parallel_to_out_with_another_component_active() {
    let mut app = post_app();
    let (_, face) = post_side(&app);
    app.run("frame.rotate_member", json!({"face": face, "angle": "30 deg"})).unwrap();
    let f = app.session.doc.features.iter().find(|f| matches!(f.kind, FeatureKind::FrameMemberTurn { .. })).unwrap().id;
    app.run("component.create", json!({"name": "Other"})).unwrap();
    assert_ne!(app.session.active_component, 0, "another component is active");
    let sketch = sketch_id(&app, "Ref");
    let mut e = edit_dialog(&app, f, None).unwrap();
    e.inputs[1].items = vec![Sel::SketchCurve { id: "ref".into(), sketch: Some(sketch) }];
    sync(&app, &mut e);
    // The step is left out, so the side facing +X is square to the 30 deg line at -60 deg.
    assert_eq!(rotate(&mut e).angle, "-60 deg");
}

/// Picking Parallel To works the angle out (the side facing +X turned square to the 30° line:
/// −60°, the nearer of −60° and 120°); the Angle stays editable, and an edited one wins.
#[test]
fn rotate_member_parallel_to_fills_the_angle() {
    let mut app = post_app();
    let (side, face) = post_side(&app);
    let sketch = sketch_id(&app, "Ref");
    app.start("frame.rotate_member");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![side];
    d.inputs[1].items = vec![Sel::SketchCurve { id: "ref".into(), sketch: Some(sketch) }];
    sync(&app, &mut d);
    assert_eq!(rotate(&mut d).angle, "-60 deg");
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1, json!({"face": face, "angle": "-60 deg", "parallel_to": {"sketch": sketch, "curve": "ref"}}));
    rotate(&mut d).angle = "-50 deg".into();
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1, json!({"face": face, "angle": "-50 deg"}), "the typed angle wins");
    run_all(&mut app, c);
    assert_eq!(turns(&app), ["-50 deg"]);
}

/// The dial sits on the member once it is picked; dragging it a quarter turn sets the Angle, and
/// the camera stays put.
#[test]
fn rotate_member_dial_sets_the_angle() {
    let mut h = post_harness();
    h.app.preferences.nav = "openscad".into();
    h.step(&json!({"call": "ui.view", "params": {"view": "top"}})).unwrap();
    h.app.start("frame.rotate_member");
    h.frames(3);
    assert!(crate::scenario::handle_at("rotate_member").is_none(), "no dial before a member is picked");
    let (side, _) = post_side(&h.app);
    h.app.dialog.as_mut().unwrap().inputs[0].items = vec![side];
    h.frames(5);
    let camera = h.app.cam;
    let start = h.at(&json!({"handle": "rotate_member"})).unwrap();
    let centre = h.at(&json!({"world": [0, 0, 150]})).unwrap();
    // A quarter turn clockwise in the top view is -90 degrees about the post's line (+Z).
    let end = [centre[0] - (start[1] - centre[1]), centre[1] + (start[0] - centre[0])];
    h.step(&json!({"drag": [start, end], "steps": 12})).unwrap();
    assert_eq!(rotate(h.app.dialog.as_mut().unwrap()).angle, "-90 deg");
    assert_eq!(h.app.cam, camera, "the dial owns the drag");
    h.step(&json!({"key": "Enter"})).unwrap();
    h.settle();
    assert!(h.app.dialog.is_none());
    assert_eq!(turns(&h.app), ["-90 deg"]);
}

/// A library profile and a named placement point reach the command and come back when editing;
/// the point names the dialog offers are the profile kind's own.
#[test]
fn a_library_profile_and_a_named_point() {
    let mut app = frame_app();
    app.run(
        "frame.library.add",
        json!({"name": "L50", "profile": {"kind": "angle", "width": 50, "height": 50, "thickness": 5}, "default_point": "heel"}),
    )
    .unwrap();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines[..1]);
    let m = member(&mut d);
    m.library = Some("lp1".into());
    m.point = Some("heel_inside".into());
    let c = apply_commands(&app, &d).unwrap();
    let p = &c[0].1;
    assert_eq!((&p["library"], &p["point"]), (&json!("lp1"), &json!("heel_inside")), "{p}");
    assert!(p.get("profile").is_none() && p.get("anchor").is_none(), "{p}");
    let names = point_names(&app, member(&mut d), false);
    assert!(
        names.starts_with(&["origin".to_string()]) && names.contains(&"heel_inside".to_string()) && names.contains(&"toe_y".to_string()),
        "{names:?}"
    );
    run_all(&mut app, c);
    assert_eq!(body_names(&app), ["L50"]);
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    let m = member(&mut e);
    assert_eq!((m.library.as_deref(), m.point.as_deref()), (Some("lp1"), Some("heel_inside")), "{m:?}");
    assert_eq!(point_label("heel_inside"), "Heel inside");
}

/// Save to Library adds the dialog's built-in profile to the library, named after it and filed under
/// its kind unless the dialog gives a name and a category (`/` between levels).
#[test]
fn save_to_library_adds_the_built_in_profile() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let d = dialog_on(&mut app, &lines[..1]);
    let mut m = match &d.kind {
        Kind::Frame(m) => m.clone(),
        k => panic!("{k:?}"),
    };
    let id = save_to_library(&mut app, &m, &d.inputs).unwrap();
    assert_eq!(id, "lp1");
    assert_eq!(app.session.doc.frame_profiles[0].name, "Rect Tube 40x40x3");
    // Saving the same profile again takes the next free name.
    m.library = None;
    save_to_library(&mut app, &m, &d.inputs).unwrap();
    assert_eq!(app.session.doc.frame_profiles[1].name, "Rect Tube 40x40x3 (2)");
    assert_eq!(app.session.doc.frame_profiles[1].category, ["Rectangular Tube"]);
    m.save_name = " 40x40x3 SHS ".into();
    m.save_category = "Steel / SHS (C350)/".into();
    save_to_library(&mut app, &m, &d.inputs).unwrap();
    let e = &app.session.doc.frame_profiles[2];
    assert_eq!((e.name.as_str(), &e.category), ("40x40x3 SHS", &vec!["Steel".to_string(), "SHS (C350)".to_string()]));
    // A name the library has already is refused, not renamed.
    let err = save_to_library(&mut app, &m, &d.inputs).unwrap_err();
    assert!(err.contains("already has a profile named"), "{err}");
}

/// The library's Type, Group and Size choices: folders one level down, the profiles in a folder and
/// below, sizes in natural order, and the profiles without a category on their own.
#[test]
fn library_folders_and_sizes() {
    let entry = |name: &str, category: &[&str]| frame_library::LibraryProfile {
        id: name.into(),
        name: name.into(),
        category: category.iter().map(|c| c.to_string()).collect(),
        section: frame_library::LibrarySection::Drawn { regions: Vec::new() },
        points: Vec::new(),
        default_point: None,
        material: None,
        description: None,
        stock_length: None,
    };
    let owned = [
        entry("100x50x3.0 RHS", &["Steel", "RHS"]),
        entry("63.5x38x2.5 RHS", &["Steel", "RHS"]),
        entry("50x20x1.6 RHS", &["Steel", "RHS"]),
        entry("50x20x2.0 RHS", &["Steel", "RHS"]),
        entry("100 PFC", &["Steel", "Channel"]),
        entry("Flat 50x6", &["Aluminium"]),
        entry("Odd one", &[]),
    ];
    let library: Vec<&frame_library::LibraryProfile> = owned.iter().collect();
    assert_eq!(subfolders(&library, &[]), ["Aluminium", "Steel"]);
    assert_eq!(subfolders(&library, &["Steel".into()]), ["Channel", "RHS"]);
    assert!(subfolders(&library, &["Aluminium".into()]).is_empty());
    let names = |folder: &[&str]| -> Vec<String> {
        let folder: Vec<String> = folder.iter().map(|f| f.to_string()).collect();
        profiles_in(&library, &folder).iter().map(|e| e.name.clone()).collect()
    };
    assert_eq!(names(&["Steel", "RHS"]), ["50x20x1.6 RHS", "50x20x2.0 RHS", "63.5x38x2.5 RHS", "100x50x3.0 RHS"]);
    assert_eq!(names(&["Steel"]), ["50x20x1.6 RHS", "50x20x2.0 RHS", "63.5x38x2.5 RHS", "100 PFC", "100x50x3.0 RHS"]);
    assert_eq!(names(&[]), ["Odd one"]);
    assert!(names(&["Timber"]).is_empty());
}

/// A personal profile is offered in another design, marked as personal; OK copies it into that
/// design's library, and editing the member finds the design's copy.
#[test]
fn a_personal_profile_is_offered_and_copied_into_the_design() {
    let mut first = frame_app();
    first
        .run(
            "frame.library.add",
            json!({"name": "L50", "category": ["Angle"], "profile": {"kind": "angle", "width": 50, "height": 50, "thickness": 5}}),
        )
        .unwrap();
    first.run("frame.personal.add", json!({"id": "lp1"})).unwrap();
    let mut app = frame_app();
    app.preferences.frame_profiles = first.preferences.frame_profiles.clone();
    let ids: Vec<String> = offered(&app).iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["personal:l50"]);
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines[..1]);
    member(&mut d).library = Some("personal:l50".into());
    member(&mut d).point = Some("heel_inside".into());
    assert!(point_names(&app, member(&mut d), false).contains(&"toe_x".to_string()), "a personal profile's points are offered");
    let c = apply_commands(&app, &d).unwrap();
    let p = &c[0].1;
    assert_eq!(p["library"]["name"], "L50", "the personal profile goes whole: {p}");
    assert!(p["library"].get("id").is_none(), "{p}");
    run_all(&mut app, c);
    assert_eq!(body_names(&app), ["L50"]);
    assert_eq!(app.session.doc.frame_profiles.iter().map(|e| (e.id.as_str(), e.name.as_str())).collect::<Vec<_>>(), [("lp1", "L50")]);
    // The design's copy is offered now, not the personal one beside it.
    let ids: Vec<String> = offered(&app).iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["lp1"]);
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    assert_eq!(member(&mut e).library.as_deref(), Some("lp1"));
}

/// Corners: mitre by default, or none with the square-ends preference; the treatment, gap and
/// flush cut reach the command and come back when editing, and an edit keeps a junction's own
/// settings.
#[test]
fn corners_reach_the_command_and_come_back_when_editing() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines);
    assert_eq!(apply_commands(&app, &d).unwrap()[0].1["corners"], json!({"treatment": "mitre"}));
    let m = member(&mut d);
    m.corner = CORNERS.iter().position(|c| c.0 == "butt").unwrap();
    m.gap = "2 mm".into();
    m.flush = true;
    m.through = BUTT_THROUGH.iter().position(|t| t.0 == "shorter").unwrap();
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["corners"], json!({"treatment": "butt", "gap": "2 mm", "butt_cut": "flush", "butt_through": "shorter"}));
    run_all(&mut app, c);
    let f = app.session.doc.features.last().unwrap().id;
    // A junction of its own, set by command, survives the dialog's edit.
    let j = app.run("frame.junctions", json!({"feature": f})).unwrap();
    let corner = j["junctions"][0]["id"].as_str().unwrap().to_string();
    app.run(
        "timeline.redefine",
        json!({"feature": f, "command": "frame.member", "params": {"sketch": "Frame", "lines": lines, "profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}, "corners": {"treatment": "butt", "gap": "2 mm", "butt_cut": "flush", "butt_through": "shorter"}, "corner_overrides": {corner.clone(): {"treatment": "mitre"}}}}),
    )
    .unwrap();
    let mut e = edit_dialog(&app, f, None).unwrap();
    let m = member(&mut e);
    assert_eq!((CORNERS[m.corner].0, m.gap.as_str(), m.flush, BUTT_THROUGH[m.through].0), ("butt", "2 mm", true, "shorter"));
    m.gap.clear();
    let c = apply_commands(&app, &e).unwrap();
    run_all(&mut app, c);
    let j = app.run("frame.junctions", json!({"feature": f})).unwrap();
    let it = j["junctions"].as_array().unwrap().iter().find(|x| x["id"] == corner.as_str()).unwrap().clone();
    assert_eq!((it["treatment"].as_str(), it["overridden"].as_bool()), (Some("mitre"), Some(true)), "{j}");
    assert!(
        j["junctions"].as_array().unwrap().iter().filter(|x| x["id"] != corner.as_str()).all(|x| x["gap"] == 0.0 && x["treatment"] == "butt"),
        "{j}"
    );
    // The preference keeps the old square ends for new members.
    app.preferences.frame_square_ends = true;
    let mut d = dialog_on(&mut app, &lines);
    assert_eq!(CORNERS[member(&mut d).corner].0, "none");
    assert_eq!(apply_commands(&app, &d).unwrap()[0].1["corners"], json!({"treatment": "none"}));
}

/// The corner list shows the picked lines' junctions; Swap (or a click on a corner's dot in the
/// view) runs the other member through at that corner alone, which reaches the command, the
/// built feature and Edit Feature, and Reset drops it again.
#[test]
fn the_corner_list_swaps_one_corner_through_to_the_feature() {
    use solidcraft_engine::doc::frame_corner::JunctionKind;
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines);
    let inputs = d.inputs.clone();
    let m = member(&mut d);
    m.corner = CORNERS.iter().position(|c| c.0 == "butt").unwrap();
    refresh_junctions(&app, m, &inputs);
    let list = m.junctions.as_ref().unwrap().1.clone();
    assert_eq!(list.len(), 4, "{list:?}");
    assert!(list.iter().all(|j| j.kind == JunctionKind::L && j.through.is_some()));
    let j = list[0].clone();
    let before = j.through.clone().unwrap();
    swap_through(m, &j);
    refresh_junctions(&app, m, &inputs);
    let through = |m: &Member, id: &str| m.junctions.as_ref().unwrap().1.iter().find(|x| x.id == id).and_then(|x| x.through.clone()).unwrap();
    let after = through(m, &j.id);
    assert_ne!(after, before, "the other member runs through");
    assert_eq!(
        list[1..].iter().map(|x| through(m, &x.id)).collect::<Vec<_>>(),
        list[1..].iter().map(|x| x.through.clone().unwrap()).collect::<Vec<_>>(),
        "the other corners stay"
    );
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].1["corner_overrides"], json!({j.id.clone(): {"through": after.clone()}}));
    run_all(&mut app, c);
    let f = app.session.doc.features.last().unwrap().id;
    let built = app.run("frame.junctions", json!({"feature": f})).unwrap();
    let it = built["junctions"].as_array().unwrap().iter().find(|x| x["id"] == j.id.as_str()).unwrap().clone();
    assert_eq!(it["through"], after.as_str(), "{built}");
    // Edit Feature brings the corner's setting back; Reset drops it, and OK clears it.
    let mut e = edit_dialog(&app, f, None).unwrap();
    let m = member(&mut e);
    assert_eq!(m.overrides.get(&j.id).and_then(|o| o.through.clone()), Some(after));
    m.overrides.remove(&j.id);
    let c = apply_commands(&app, &e).unwrap();
    assert_eq!((c[0].0.as_str(), &c[0].1["params"]["corner_overrides"]), ("timeline.redefine", &json!({})), "{c:?}");
    run_all(&mut app, c);
    let built = app.run("frame.junctions", json!({"feature": f})).unwrap();
    assert!(built["junctions"].as_array().unwrap().iter().all(|x| x["overridden"] == false), "{built}");
}

/// With Corners on, each junction is marked in the view, and a click on a butt corner's marker
/// runs the other member through there.
#[test]
fn a_click_on_a_corner_marker_swaps_the_member_that_runs_through() {
    let mut h = crate::scenario::Harness::new();
    for (c, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Frame"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]})),
        ("sketch.finish", json!({})),
        (
            "frame.member",
            json!({"sketch": "Frame", "profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}, "corners": {"treatment": "butt"}}),
        ),
    ] {
        h.app.run(c, p).unwrap();
    }
    let f = h.app.session.doc.features.last().unwrap().id;
    h.app.edit_feature(f);
    h.frames(3);
    let rect = h.app.viewport.rect.unwrap();
    let proj = crate::viewport::projection(&h.app, rect);
    let Some(Kind::Frame(m)) = h.app.dialog.as_ref().map(|d| &d.kind) else { panic!("no dialog") };
    let (j, at) = markers(m, &proj)
        .into_iter()
        .find(|(_, c)| rect.shrink(20.0).contains(*c))
        .map(|(j, c)| (j.clone(), c))
        .expect("a corner marker in the view");
    let before = j.through.clone().unwrap();
    assert!(marker_at(&h.app, &proj, at), "the marker owns the pointer, so the click doesn't pick in the model");
    assert!(
        h.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.center.distance(at) < 0.5 && c.radius >= 7.0)),
        "a marker is drawn at the corner"
    );
    h.call("ui.click", json!({"x": at.x, "y": at.y}));
    h.frames(2);
    let Some(Kind::Frame(m)) = h.app.dialog.as_ref().map(|d| &d.kind) else { panic!("no dialog") };
    let now = m.overrides.get(&j.id).and_then(|o| o.through.clone());
    assert!(now.is_some() && now.as_deref() != Some(before.as_str()), "{:?}", m.overrides);
}

/// Shift-clicking corner markers picks them together; the Selected corners rows then give them
/// all one treatment and one weld gap, which reach the feature as their own settings.
#[test]
fn corners_picked_together_take_one_treatment_and_gap() {
    let mut h = crate::scenario::Harness::new();
    for (c, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Frame"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 40]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Frame", "profile": {"kind": "rect_tube", "width": 4, "height": 4, "wall": 0.5}})),
    ] {
        h.app.run(c, p).unwrap();
    }
    let f = h.app.session.doc.features.last().unwrap().id;
    h.app.edit_feature(f);
    h.frames(3);
    let rect = h.app.viewport.rect.unwrap();
    let proj = crate::viewport::projection(&h.app, rect);
    let Some(Kind::Frame(m)) = h.app.dialog.as_ref().map(|d| &d.kind) else { panic!("no dialog") };
    let shown: Vec<(String, Pos2)> =
        markers(m, &proj).into_iter().filter(|(_, c)| rect.shrink(20.0).contains(*c)).map(|(j, c)| (j.id.clone(), c)).collect();
    assert!(shown.len() >= 2, "{shown:?} of {:?} in {rect:?}", markers(m, &proj).iter().map(|(j, c)| (j.id.clone(), *c)).collect::<Vec<_>>());
    for (_, at) in &shown[..2] {
        h.call("ui.click", json!({"x": at.x, "y": at.y, "shift": true}));
        h.frames(2);
    }
    let picked: BTreeSet<String> = shown[..2].iter().map(|(id, _)| id.clone()).collect();
    let d = h.app.dialog.as_mut().unwrap();
    let m = member(d);
    assert_eq!(m.selected, picked, "a shift-click picks, and swaps nothing: {:?}", m.overrides);
    assert!(m.overrides.is_empty());
    set_selected(m, Some(Some(Treatment::Butt)), Some("0.2 mm"));
    let d = h.app.dialog.clone().unwrap();
    let c = apply_commands(&h.app, &d).unwrap();
    let want: Map<String, Value> = picked.iter().map(|id| (id.clone(), json!({"treatment": "butt", "gap": "0.2 mm"}))).collect();
    assert_eq!(c[0].1["params"]["corner_overrides"], Value::Object(want), "{c:?}");
    run_all(&mut h.app, c);
    let built = h.app.run("frame.junctions", json!({"feature": f})).unwrap();
    for j in built["junctions"].as_array().unwrap() {
        let id = j["id"].as_str().unwrap();
        let (treatment, gap) = if picked.contains(id) { ("butt", 0.2) } else { ("mitre", 0.0) };
        assert_eq!((j["treatment"].as_str().unwrap(), j["gap"].as_f64().unwrap()), (treatment, gap), "{j}");
    }
}

/// The section preview under the Point row has the profile's outline and a dot per placement
/// point, the anchors and the points its kind generates.
#[test]
fn the_section_preview_has_the_outline_and_every_placement_point() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines);
    let m = member(&mut d).clone();
    let p = preview_of(&app, &m, &[]).expect("a built-in profile's preview");
    assert_eq!(p.outlines.len(), 2, "a rectangular tube's outside and inside");
    let names: Vec<&str> = p.points.iter().map(|x| x.0.as_str()).collect();
    for n in point_names(&app, &m, false) {
        assert!(names.contains(&n.as_str()), "{n} in {names:?}");
    }
    let (lo, hi) = p.outlines[0].iter().fold((f64::MAX, f64::MIN), |(lo, hi), q| (lo.min(q.x), hi.max(q.x)));
    assert!(hi - lo > 1.0, "the outline has a size: {lo}…{hi}");
    // A dot chosen puts that point on the lines, as the Point row does.
    let mut m = m;
    choose_point(&mut m, Some("top_left"), false);
    assert_eq!((m.anchor, m.point.clone()), (ANCHORS.iter().position(|a| a.0 == "top_left").unwrap(), None));
    let generated = p.points.iter().map(|x| x.0.clone()).find(|n| !ANCHORS.iter().any(|a| a.0 == n)).expect("a generated point");
    choose_point(&mut m, Some(&generated), false);
    assert_eq!((m.anchor, m.point.clone()), (0, Some(generated)));
}

/// The corner list names lines after their members: an edited feature's names, a new feature's
/// after Name, else the line.
#[test]
fn the_corner_list_names_lines_after_their_members() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines);
    let inputs = d.inputs.clone();
    let m = member(&mut d);
    refresh_junctions(&app, m, &inputs);
    assert_eq!(line_label(m, &lines[0]), format!("line {}", lines[0]));
    m.name = "Rail".into();
    refresh_junctions(&app, m, &inputs);
    let named: Vec<String> = lines.iter().map(|l| line_label(m, l)).collect();
    assert_eq!(named, ["Rail", "Rail (2)", "Rail (3)", "Rail (4)"]);
    let c = apply_commands(&app, &d).unwrap();
    run_all(&mut app, c);
    let f = app.session.doc.features.last().unwrap().id;
    app.run("body.rename", json!({"body": "Rail (3)", "name": "Top"})).unwrap();
    let mut e = edit_dialog(&app, f, None).unwrap();
    let inputs = e.inputs.clone();
    let m = member(&mut e);
    refresh_junctions(&app, m, &inputs);
    assert_eq!(line_label(m, &lines[2]), "Top", "{:?}", m.line_names);
}

/// New group starts a group with no lines; its lines and profile are its own, the first group's
/// stay as they were, both reach `frame.member` (`groups`), and Edit Feature brings them back.
#[test]
fn groups_have_their_own_lines_and_profile_and_come_back_when_editing() {
    let mut app = frame_app();
    let lines = frame_lines(&app);
    let mut d = dialog_on(&mut app, &lines[..2]);
    let inputs = &mut d.inputs;
    let Kind::Frame(m) = &mut d.kind else { panic!() };
    new_group(m, inputs);
    assert_eq!((m.group, m.slots.len()), (1, 2));
    assert!(inputs[0].items.is_empty(), "a new group has no lines yet");
    inputs[0].items = lines[2..].iter().map(|id| Sel::SketchCurve { id: id.clone(), sketch: None }).collect();
    m.dims[m.kind][0] = "60 mm".into();
    m.name = "Post".into();
    switch_group(m, inputs, 0);
    assert_eq!(inputs[0].items.len(), 2, "the first group's lines are back");
    assert_eq!(m.dims[m.kind][0], "40 mm");
    let c = apply_commands(&app, &d).unwrap();
    let p = &c[0].1;
    assert_eq!(p["lines"], json!(lines[..2]));
    assert_eq!(p["groups"][0]["lines"], json!(lines[2..]), "{p}");
    assert_eq!(
        (p["groups"][0]["id"].as_str(), p["groups"][0]["body_name"].as_str(), p["groups"][0]["profile"]["width"].as_str()),
        (Some("g2"), Some("Post"), Some("60 mm"))
    );
    assert_eq!(p["corners"]["between_groups"], "butt");
    run_all(&mut app, c);
    assert!(body_names(&app).iter().any(|n| n == "Post (2)"), "{:?}", body_names(&app));
    let f = app.session.doc.features.last().unwrap().id;
    let mut e = edit_dialog(&app, f, None).unwrap();
    let inputs = &mut e.inputs;
    let Kind::Frame(m) = &mut e.kind else { panic!() };
    assert_eq!(m.slots.len(), 2);
    switch_group(m, inputs, 1);
    assert_eq!((m.name.as_str(), m.dims[m.kind][0].as_str(), inputs[0].items.len()), ("Post", "60 mm", 2));
    // Remove group puts its lines back with the first group (All Lines off: they go).
    remove_group(m, inputs);
    assert!(m.slots.is_empty() && m.group == 0);
    let c = apply_commands(&app, &e).unwrap();
    assert_eq!(c[0].1["params"]["groups"], json!([]), "{c:?}");
}
