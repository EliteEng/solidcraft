use crate::test_support::run_all;
use serde_json::json;
use solidcraft_engine::Session;

use super::*;
use crate::Services;
use crate::dialogs::{apply_commands, for_feature as edit_dialog};

/// A 40 mm tube `Rail` along X from 0 to 600 and a round tube `Post` upright at x = 650.
fn rail_and_post() -> SolveApp {
    let mut s = Session::default();
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Frame"})),
        ("sketch.line", json!({"points": [[0, 0], [600, 0]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Frame", "profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}, "body_name": "Rail"})),
        ("sketch.create", json!({"plane": "XZ", "name": "Upright"})),
        ("sketch.line", json!({"points": [[650, -100], [650, 100]]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Upright", "profile": {"kind": "round_tube", "diameter": 60, "wall": 4}, "body_name": "Post"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    SolveApp::new(s, Services::default())
}

/// A pick on the rail near its far end and on the post makes `frame.trim` extend the rail into
/// the post and cope it; Edit Feature brings the picks and values back.
#[test]
fn trim_extend_takes_member_ends_and_a_boundary_and_comes_back_when_editing() {
    let mut app = rail_and_post();
    app.start("frame.trim");
    let mut d = app.dialog.clone().unwrap();
    // The rail's top face near its end, and the post.
    let near = solidcore_record::Vec3::new(590.0, 0.0, 20.0);
    d.inputs[0].items = crate::dialogs::face_sel_on(&app.session, Some("Rail"), near).into_iter().collect();
    d.inputs[1].items = vec![Sel::Body { name: "Post".into() }];
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].0, "frame.trim");
    assert_eq!(c[0].1["tool"], "Post", "{c:?}");
    assert_eq!(c[0].1["members"][0]["body"], "Rail", "{c:?}");
    assert_eq!(c[0].1["extend"], true, "extend is on by default: {c:?}");
    run_all(&mut app, c);
    let f = app.session.doc.features.last().unwrap();
    assert!(matches!(&f.kind, FeatureKind::FrameTrim { members, .. } if members[0].end == End::End), "{:?}", f.kind);
    let fid = f.id;
    let mut e = edit_dialog(&app, fid, None).unwrap();
    assert!(matches!(e.kind, Kind::Weld(Wd::Trim { extend: true, .. })));
    assert_eq!(e.inputs[0].items, vec![Sel::Body { name: "Rail".into() }]);
    assert_eq!(e.inputs[1].items, vec![Sel::Body { name: "Post".into() }]);
    if let Kind::Weld(Wd::Trim { gap, .. }) = &mut e.kind {
        *gap = "1 mm".into();
    }
    let c = apply_commands(&app, &e).unwrap();
    assert_eq!(c[0].0, "timeline.redefine", "{c:?}");
    assert_eq!(c[0].1["params"]["members"][0], json!({"body": "Rail", "end": "end"}), "{c:?}");
    // A gap on a cope by a body is refused, with the reason.
    let err = app.session.execute(&c[0].0, &c[0].1).unwrap_err().to_string();
    assert!(err.contains("gap"), "{err}");
}

#[test]
fn trim_extend_without_picks_says_what_to_pick() {
    let mut app = rail_and_post();
    app.start("frame.trim");
    let d = app.dialog.clone().unwrap();
    assert!(apply_commands(&app, &d).unwrap_err().contains("member"));
}

/// Gusset takes two flat faces and its sizes; Weld takes edges and its size and stitch; Edit
/// Feature brings both back.
#[test]
fn gusset_and_weld_take_their_picks_and_come_back_when_editing() {
    let mut s = Session::default();
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Frame"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]})),
        ("sketch.finish", json!({})),
        ("frame.member", json!({"sketch": "Frame", "profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}, "body_name": "Side"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let mut app = SolveApp::new(s, Services::default());
    let face = |app: &SolveApp, p: [f64; 3]| crate::dialogs::face_sel(&app.session, solidcore_record::Vec3::new(p[0], p[1], p[2])).unwrap();
    app.start("frame.gusset");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![face(&app, [300.0, 20.0, 0.0])];
    d.inputs[1].items = vec![face(&app, [580.0, 200.0, 0.0])];
    if let Kind::Weld(Wd::Gusset { chamfered, d1, clip, .. }) = &mut d.kind {
        *chamfered = true;
        *d1 = "80 mm".into();
        *clip = "5 mm".into();
    }
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].0, "frame.gusset");
    assert_eq!((c[0].1["shape"].as_str(), c[0].1["d1"].as_str(), c[0].1["clip"].as_str()), (Some("chamfered"), Some("80 mm"), Some("5 mm")), "{c:?}");
    run_all(&mut app, c);
    let fid = app.session.doc.features.last().unwrap().id;
    let e = edit_dialog(&app, fid, None).unwrap();
    assert!(matches!(&e.kind, Kind::Weld(Wd::Gusset { chamfered: true, d1, d2, .. }) if d1 == "80 mm" && d2.is_empty()), "{:?}", e.kind);
    assert!(e.inputs.iter().all(|i| matches!(i.items.first(), Some(Sel::Face { .. }))), "{:?}", e.inputs);
    // A weld along the gusset's edge on the rail face.
    app.start("frame.weld");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![Sel::Edge { body: "Gusset".into(), index: 0, point: solidcore_record::Vec3::new(540.0, 20.0, 2.5) }];
    if let Kind::Weld(Wd::Weld { both_sides, .. }) = &mut d.kind {
        *both_sides = true;
    }
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!((c[0].0.as_str(), c[0].1["both_sides"].as_bool()), ("frame.weld", Some(true)), "{c:?}");
    run_all(&mut app, c);
    let fid = app.session.doc.features.last().unwrap().id;
    assert!(matches!(app.session.doc.feature(fid).map(|f| &f.kind), Some(FeatureKind::FrameWeld { .. })), "the weld was made");
    let e = edit_dialog(&app, fid, None).unwrap();
    assert!(matches!(&e.kind, Kind::Weld(Wd::Weld { both_sides: true, .. })), "{:?}", e.kind);
    assert!(matches!(e.inputs[0].items.first(), Some(Sel::Edge { body, .. }) if body == "Gusset"));
}

/// Export Member DXF takes the top and front faces of one member; picks on two members, or one
/// face, are refused with the reason. Export Member STEP takes the member.
#[test]
fn member_exports_take_their_picks() {
    let mut app = rail_and_post();
    let face = |app: &SolveApp, p: [f64; 3]| crate::dialogs::face_sel(&app.session, solidcore_record::Vec3::new(p[0], p[1], p[2])).unwrap();
    app.start("frame.export_member_dxf");
    let mut d = app.dialog.clone().unwrap();
    assert!(matches!(&d.kind, Kind::Weld(k) if k.saves()));
    d.inputs[0].items = vec![face(&app, [300.0, 0.0, 20.0])];
    let Kind::Weld(k) = &d.kind else { panic!() };
    assert!(export_params(k, &d.inputs).unwrap_err().contains("front"));
    d.inputs[1].items = vec![face(&app, [300.0, -20.0, 0.0])];
    let p = export_params(k, &d.inputs).unwrap();
    assert_eq!((p["body"].as_str(), p["hidden"].as_bool()), (Some("Rail"), Some(false)), "{p}");
    let r = app.session.execute("frame.export_member_dxf", &p).unwrap();
    assert!(r["text"].as_str().unwrap().contains("ENTITIES"));
    d.inputs[1].items = vec![face(&app, [650.0, 30.0, 0.0])];
    assert!(export_params(k, &d.inputs).unwrap_err().contains("same member"));
    app.start("frame.export_member_step");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![Sel::Body { name: "Post".into() }];
    let Kind::Weld(k) = &d.kind else { panic!() };
    assert_eq!(export_params(k, &d.inputs).unwrap(), json!({"body": "Post"}));
}
