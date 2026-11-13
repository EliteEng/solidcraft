//! The personal profile library: kept in preferences, changed only through `frame.personal.*`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use solidcraft_engine::Session;

use crate::{Services, SolveApp};

fn app() -> SolveApp {
    let mut s = Session::default();
    s.execute("parameters.add", &json!({"name": "t", "expression": "3 mm"})).unwrap();
    s.execute(
        "frame.library.add",
        &json!({"name": "SHS 40", "category": ["Steel", "SHS"], "profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": "t"}}),
    )
    .unwrap();
    s.execute("frame.library.add", &json!({"name": "L50", "profile": {"kind": "angle", "width": 50, "height": 50, "thickness": 5}})).unwrap();
    SolveApp::new(s, Services::default())
}

fn names(app: &mut SolveApp) -> Vec<String> {
    let listed = app.run("frame.personal.list", json!({})).unwrap();
    listed["profiles"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

fn row(name: &str) -> Value {
    json!({"name": name, "section": {"source": "built_in", "profile": {"kind": "round_bar", "diameter": 10}}})
}

#[test]
fn profiles_are_added_from_the_design_and_kept_as_values() {
    let mut app = app();
    assert!(names(&mut app).is_empty());
    let added = app.run("frame.personal.add", json!({"id": "SHS 40"})).unwrap();
    assert_eq!(added["profiles"][0]["section"]["profile"]["wall"], "3 mm", "the parameter is kept as its value: {added}");
    assert!(added["profiles"][0].get("id").is_none(), "{added}");
    assert_eq!(app.preferences.frame_profiles.0[0].id, "personal:shs 40");
    // A name the personal library has is refused unless it is replaced; nothing is half added.
    let err = app
        .run("frame.personal.add", json!({"library": {"format": "solidcraft-frame-profiles/1", "profiles": [row("Bar 10"), row("shs 40")]}}))
        .unwrap_err();
    assert!(err.contains("has `SHS 40` already"), "{err}");
    assert_eq!(names(&mut app), ["SHS 40"]);
    app.run(
        "frame.personal.add",
        json!({"library": {"format": "solidcraft-frame-profiles/1", "profiles": [row("Bar 10"), row("shs 40")]}, "replace": true}),
    )
    .unwrap();
    assert_eq!(names(&mut app), ["shs 40", "Bar 10"]);
    assert_eq!(app.run("frame.personal.remove", json!({"name": "BAR 10"})).unwrap(), json!({"removed": "Bar 10"}));
    assert!(app.run("frame.personal.remove", json!({"name": "Bar 10"})).unwrap_err().contains("no profile `Bar 10`"));
    // Removing it changes no design: the design's library is its own.
    assert_eq!(app.session.doc.frame_profiles.len(), 2);
}

#[test]
fn the_personal_library_exports_a_file_another_design_imports() {
    let mut app = app();
    app.run("frame.personal.add", json!({"id": "SHS 40"})).unwrap();
    app.run("frame.personal.add", json!({"id": "L50"})).unwrap();
    let dir = std::env::temp_dir().join(format!("solidcraft-frame-personal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mine.json").to_string_lossy().to_string();
    let out = app.run("frame.personal.export", json!({"names": ["l50", "L50"], "path": path})).unwrap();
    assert_eq!(out["path"], json!(path));
    assert_eq!(out["library"]["profiles"].as_array().map(Vec::len), Some(1), "{out}");
    assert!(out["library"]["profiles"][0].get("id").is_none(), "{out}");
    assert!(app.run("frame.personal.export", json!({"names": ["nope"]})).unwrap_err().contains("no profile `nope`"));
    let mut other = SolveApp::new(Session::default(), Services::default());
    let imported = other.run("frame.library.import", json!({"path": path})).unwrap();
    assert_eq!(imported["profiles"][0]["name"], "L50", "{imported}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn bad_saved_rows_are_left_out_without_losing_the_others() {
    let mut bad_category = row("Deep");
    bad_category["category"] = json!(["a", "b", "c", "d"]);
    let saved =
        json!({"length_decimals": 4, "frame_profiles": [null, row("Bar 10"), {"name": "no section"}, bad_category, row("BAR 10"), row("Bar 12")]});
    let p = crate::prefs::lenient(&saved);
    let kept: Vec<&str> = p.frame_profiles.0.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(kept, ["Bar 10", "Bar 12"]);
    assert_eq!(serde_json::to_value(&p).unwrap()["length_decimals"], 4, "the other preferences are kept");
    let not_a_list = crate::prefs::lenient(&json!({"frame_profiles": {"unexpected": true}}));
    assert!(not_a_list.frame_profiles.0.is_empty());
}
