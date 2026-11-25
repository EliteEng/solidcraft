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

/// Weld takes edges and its size and stitch; Edit Feature brings it back.
#[test]
fn a_weld_takes_its_picks_and_comes_back_when_editing() {
    let mut app = rail_and_post();
    // The edge where the rail's start face meets its side at y = 20.
    app.start("frame.weld");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![Sel::Edge { body: "Rail".into(), index: 0, point: solidcore_record::Vec3::new(0.0, 20.0, 0.0) }];
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
    assert!(matches!(e.inputs[0].items.first(), Some(Sel::Edge { body, .. }) if body == "Rail"));
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

/// Save as Weldment Profile, opened from a sketch's menu, saves the sketch's section with a named
/// point to the profile folder chosen, under the category typed, and leaves the design alone.
#[test]
fn a_sketch_is_saved_as_a_weldment_profile_in_its_category_folder() {
    let mut s = Session::default();
    s.execute("sketch.create", &json!({"plane": "XY", "name": "Purlin"})).unwrap();
    s.execute("sketch.circle.center", &json!({"center": [0, 0], "radius": 20})).unwrap();
    s.execute("sketch.circle.center", &json!({"center": [0, 0], "radius": 17})).unwrap();
    let pt = s.execute("sketch.point", &json!({"point": [0, 17]})).unwrap();
    let pid = pt["created"]["points"][0].as_str().unwrap().to_string();
    s.execute("sketch.finish", &json!({})).unwrap();
    let sketch = s.doc.features.iter().find(|f| f.name == "Purlin").unwrap().id;
    let mut app = SolveApp::new(s, Services::default());
    let dir = std::env::temp_dir().join(format!("solidcraft-save-profile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    app.run("frame.personal.folders", json!({"folders": [dir.to_string_lossy()]})).unwrap();
    let features = app.session.doc.features.len();
    // The sketch's menu selects it and opens the window on its material.
    app.run("select.set", json!({"items": [{"type": "feature", "id": sketch}]})).unwrap();
    app.start("frame.personal.save");
    let mut d = app.dialog.clone().unwrap();
    assert!(!d.previews(), "saving a profile changes no body: nothing to preview");
    d.inputs[1].items = vec![Sel::SketchPoint { id: format!("{sketch}:{pid}") }];
    if let Kind::Weld(Wd::SaveProfile(sp)) = &mut d.kind {
        assert_eq!(sp.whole, Some(sketch));
        sp.name = "Tube 40".into();
        sp.category = "Steel / Round".into();
        sp.point_names = vec!["inside_top".into()];
        sp.default_point = 1;
        sp.stock_length = "6 m".into();
    }
    let c = apply_commands(&app, &d).unwrap();
    assert_eq!(c[0].0, "frame.personal.save");
    assert_eq!(c[0].1["category"], json!(["Steel", "Round"]), "{c:?}");
    assert_eq!(c[0].1["points"], json!([{"name": "inside_top", "sketch_point": pid}]), "{c:?}");
    assert_eq!(c[0].1["default_point"], "inside_top", "{c:?}");
    crate::dialogs::run_dialog(&mut app, &d).unwrap();
    let file = dir.join("Steel").join("Round").join("Tube 40.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"drawn\"") && text.contains("inside_top"), "{text}");
    assert_eq!(app.session.doc.features.len(), features, "the design doesn't change");
    assert!(app.session.doc.frame_profiles.is_empty());
    // Frame Member offers it in this design and any other, filed under its folders.
    let e = crate::frame_personal::get(&app, "personal:tube 40").unwrap();
    assert_eq!(e.category, ["Steel", "Round"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A member export of a saved design is written next to it without a dialog, named with the
/// design's revision once it has one; with the tick box off, or for a design never saved, the save
/// dialog asks.
#[test]
fn a_member_export_is_written_next_to_a_saved_design() {
    let dir = std::env::temp_dir().join(format!("solidcraft-member-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = rail_and_post();
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = asked.clone();
    app.services.pick_save = Some(Box::new(move |_, _| {
        count.set(count.get() + 1);
        Box::pin(std::future::ready(None))
    }));
    // Never saved: asked.
    app.start("frame.export_member_step");
    let mut d = app.dialog.clone().unwrap();
    d.inputs[0].items = vec![Sel::Body { name: "Rail".into() }];
    run_export(&mut app, &Wd::ExportStep, &d.inputs).unwrap();
    app.file_picker = None;
    assert_eq!(asked.get(), 1);

    let design = dir.join("Gate.sc");
    app.session.path = Some(design.to_string_lossy().into_owned());
    run_export(&mut app, &Wd::ExportStep, &d.inputs).unwrap();
    // Named after the design's file and the member's cut list item.
    let item = cut_list_item(&mut app, "Rail").unwrap();
    assert!(!item.is_empty() && item.chars().all(|c| c.is_ascii_digit()), "a number: {item}");
    assert!(dir.join(format!("Gate_{item}.step")).is_file(), "written beside the design");
    assert_eq!(asked.get(), 1, "without asking");

    // The design's revision goes in the name, so a new revision doesn't write over the last one's.
    for (rev, label) in [("A", "RevA"), ("Rev B", "RevB"), ("rev-C", "RevC")] {
        app.run("document.metadata.set", json!({"key": "revision", "value": rev})).unwrap();
        run_export(&mut app, &Wd::ExportStep, &d.inputs).unwrap();
        let file = format!("Gate_{item}_{label}.step");
        assert!(dir.join(&file).is_file(), "{rev}: {file}");
    }
    assert!(dir.join(format!("Gate_{item}_RevA.step")).is_file(), "the earlier revision's file is kept");

    app.preferences.frame_exports_beside_design = false;
    run_export(&mut app, &Wd::ExportStep, &d.inputs).unwrap();
    app.file_picker = None;
    assert_eq!(asked.get(), 2, "the tick box off asks again");
    let _ = std::fs::remove_dir_all(&dir);
}
