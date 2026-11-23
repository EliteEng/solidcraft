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

/// An empty folder of the test's own under the temporary folder.
fn temp_folder(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("solidcraft-frame-folders-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn library_file(profiles: &[Value]) -> String {
    serde_json::to_string(&json!({"format": "solidcraft-frame-profiles/1", "profiles": profiles})).unwrap()
}

#[test]
fn profiles_are_kept_as_files_whose_folders_are_their_categories() {
    let mut app = app();
    let dir = temp_folder("categories");
    let root = dir.to_string_lossy().to_string();
    let set = app.run("frame.personal.folders", json!({"folders": [root]})).unwrap();
    assert_eq!((set["folders"][0]["writable"].as_bool(), set["folders"][0]["profiles"].as_u64()), (Some(true), Some(0)), "{set}");
    // A profile saved goes in the subfolders its categories name, a file of its own.
    let added = app.run("frame.personal.add", json!({"id": "SHS 40"})).unwrap();
    let file = dir.join("Steel").join("SHS").join("SHS 40.json");
    assert!(file.is_file(), "{added}");
    assert_eq!(added["profiles"][0]["file"].as_str(), Some(file.to_string_lossy().as_ref()), "{added}");
    assert!(app.preferences.frame_profiles.0.is_empty(), "nothing is kept with the preferences while a folder can be written");
    // A file someone put in a folder is filed under that folder, whatever the file says.
    let extrusions = dir.join("Aluminium").join("Extrusions");
    std::fs::create_dir_all(&extrusions).unwrap();
    let mut bar = row("4040");
    bar["category"] = json!(["Somewhere", "Else"]);
    std::fs::write(extrusions.join("4040.json"), library_file(&[bar])).unwrap();
    // Too deep, not a library file, and hidden: reported or passed over, never a crash.
    let deep = dir.join("a").join("b").join("c").join("d");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("deep.json"), library_file(&[row("Deep")])).unwrap();
    std::fs::write(dir.join("broken.json"), "{not json").unwrap();
    std::fs::write(dir.join(".hidden.json"), library_file(&[row("Hidden")])).unwrap();
    let listed = app.run("frame.personal.list", json!({"refresh": true})).unwrap();
    let find = |name: &str| listed["profiles"].as_array().unwrap().iter().find(|e| e["name"] == name).cloned();
    assert_eq!(find("4040").map(|e| e["category"].clone()), Some(json!(["Aluminium", "Extrusions"])), "{listed}");
    assert_eq!(find("SHS 40").map(|e| e["category"].clone()), Some(json!(["Steel", "SHS"])), "{listed}");
    assert!(find("Deep").is_none() && find("Hidden").is_none(), "{listed}");
    let skipped: Vec<&str> = listed["skipped"].as_array().unwrap().iter().filter_map(|s| s["path"].as_str()).collect();
    assert!(skipped.iter().any(|p| p.ends_with("broken.json")) && skipped.iter().any(|p| p.ends_with('d')), "{listed}");
    // Frame Member offers them, under the folder's categories.
    let offered = crate::frame_personal::get(&app, "personal:4040").map(|e| e.category.clone());
    assert_eq!(offered, Some(vec!["Aluminium".to_string(), "Extrusions".to_string()]));
    // Removing a profile deletes its file.
    app.run("frame.personal.remove", json!({"name": "4040"})).unwrap();
    assert!(!extrusions.join("4040.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_earlier_folder_wins_a_name_and_a_save_can_choose_its_folder() {
    let mut app = app();
    let (shop, mine) = (temp_folder("shop"), temp_folder("mine"));
    std::fs::write(shop.join("bar.json"), library_file(&[row("Bar 10")])).unwrap();
    std::fs::write(mine.join("bar.json"), library_file(&[row("bar 10"), row("Bar 12")])).unwrap();
    let roots = [shop.to_string_lossy().to_string(), mine.to_string_lossy().to_string()];
    app.run("frame.personal.folders", json!({"folders": roots})).unwrap();
    let listed = app.run("frame.personal.list", json!({})).unwrap();
    let names: Vec<&str> = listed["profiles"].as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).collect();
    assert_eq!(names, ["Bar 10", "Bar 12"], "{listed}");
    assert_eq!(listed["profiles"][0]["folder"].as_str(), Some(roots[0].as_str()), "{listed}");
    assert!(listed["skipped"][0]["reason"].as_str().unwrap().contains("bar 10"), "{listed}");
    // A save names its folder by number (or path); a profile saved replaces its file.
    app.run("frame.personal.add", json!({"id": "L50", "folder": "2"})).unwrap();
    assert!(mine.join("L50.json").is_file());
    let err = app.run("frame.personal.add", json!({"id": "L50", "folder": "3"})).unwrap_err();
    assert!(err.contains("not one of your profile folders"), "{err}");
    app.run("frame.personal.add", json!({"id": "L50", "folder": roots[1], "replace": true})).unwrap();
    assert!(mine.join("L50.json").is_file() && !mine.join("L50 (2).json").exists(), "the file is replaced, not copied");
    // Removing one profile of a file of two keeps the other.
    app.run("frame.personal.remove", json!({"name": "Bar 12"})).unwrap();
    let left = std::fs::read_to_string(mine.join("bar.json")).unwrap();
    assert!(left.contains("bar 10") && !left.contains("Bar 12"), "{left}");
    for d in [shop, mine] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn profiles_kept_with_the_preferences_move_into_the_first_folder() {
    let mut app = app();
    app.run("frame.personal.add", json!({"id": "SHS 40"})).unwrap();
    app.run("frame.personal.add", json!({"id": "L50"})).unwrap();
    assert_eq!(app.preferences.frame_profiles.0.len(), 2, "no folder: the preferences keep them");
    let dir = temp_folder("move");
    app.run("frame.personal.folders", json!({"folders": [dir.to_string_lossy()]})).unwrap();
    assert!(app.preferences.frame_profiles.0.is_empty(), "both were written");
    assert!(dir.join("Steel").join("SHS").join("SHS 40.json").is_file() && dir.join("L50.json").is_file());
    // Listed in name order through the folder: `L50.json` before the `Steel` folder.
    assert_eq!(names(&mut app), ["L50", "SHS 40"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_that_is_missing_is_reported_and_saves_go_to_one_that_can_be_written() {
    let mut app = app();
    let dir = temp_folder("missing");
    let gone = dir.join("not-here").to_string_lossy().to_string();
    let set = app.run("frame.personal.folders", json!({"folders": [gone, dir.to_string_lossy()]})).unwrap();
    assert!(set["folders"][0]["error"].is_string() && set["folders"][0]["writable"] == false, "{set}");
    let added = app.run("frame.personal.add", json!({"id": "L50"})).unwrap();
    assert!(dir.join("L50.json").is_file(), "{added}");
    let err = app.run("frame.personal.add", json!({"id": "SHS 40", "folder": "1"})).unwrap_err();
    assert!(err.contains("can't be written"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
