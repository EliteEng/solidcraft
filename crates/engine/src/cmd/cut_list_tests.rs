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

fn close(v: &Value, want: f64) -> bool {
    (f(v) - want).abs() < 1e-6
}

fn tube() -> Value {
    json!({"kind": "rect_tube", "width": 40, "height": 40, "wall": 3})
}

/// A 600 × 400 rectangle sketch `Frame` on XY.
fn rectangle(s: &mut Session) {
    run(s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]}));
    run(s, "sketch.finish", json!({}));
}

/// Mitred 40 mm tube: the outside runs 20 past each corner, the inside stops 20 short, so a rail
/// is 640 long outside and 560 inside, a post 440 and 360; both ends lean 45°. Butted with a 3 mm
/// gap, the through members keep their whole length and the others stop 23 short each end.
#[test]
fn the_cut_list_groups_identical_members_with_their_lengths_and_end_angles() {
    let mut s = Session::default();
    rectangle(&mut s);
    let fid = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre"}))["feature"].clone();
    let list = run(&mut s, "frame.cut_list", json!({"format": "csv"}));
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{list}");
    let (rails, posts) = (&items[0], &items[1]);
    assert!(close(&rails["length"], 640.0) && close(&rails["short_length"], 560.0) && rails["quantity"] == 2, "{rails}");
    assert!(close(&posts["length"], 440.0) && close(&posts["short_length"], 360.0) && posts["quantity"] == 2, "{posts}");
    assert!(items.iter().all(|i| close(&i["ends"][0]["angle"], 45.0) && close(&i["ends"][1]["angle"], 45.0)), "{list}");
    assert!(close(&list["stock"][0]["total_length"], 2.0 * 640.0 + 2.0 * 440.0) && list["stock"][0]["pieces"] == 4, "{list}");
    let each: f64 = items.iter().map(|i| f(&i["mass_each_g"]) * f(&i["quantity"])).sum();
    assert!(each > 0.0 && (f(&list["total_mass_g"]) - each).abs() < 1e-6, "{list}");
    let csv = list["csv"].as_str().unwrap();
    assert!(csv.starts_with("Item,Qty,Opposite hand,Description,Length (mm)") && csv.lines().count() == 3, "{csv}");
    assert!(csv.lines().nth(1).unwrap().starts_with("1,2,0,Rect Tube 40x40x3,640.00,560.00,45.00,45.00,"), "{csv}");
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": fid.clone(), "command": "frame.member", "params": {"sketch": "Frame", "profile": tube(), "corner": "butt", "gap": 3}}),
    );
    let list = run(&mut s, "frame.cut_list", json!({"feature": fid}));
    let mut lengths: Vec<(f64, f64, f64)> =
        list["items"].as_array().unwrap().iter().map(|i| (f(&i["length"]), f(&i["short_length"]), f(&i["ends"][0]["angle"]))).collect();
    lengths.sort_by(|a, b| b.0.total_cmp(&a.0));
    let rails_through = [(640.0, 640.0, 0.0), (354.0, 354.0, 0.0)];
    let posts_through = [(554.0, 554.0, 0.0), (440.0, 440.0, 0.0)];
    let near =
        |want: &[(f64, f64, f64); 2]| lengths.iter().zip(want).all(|(a, b)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && a.2.abs() < 1e-6);
    assert!(lengths.len() == 2 && (near(&rails_through) || near(&posts_through)), "{lengths:?}");
    assert!(list.get("csv").is_none());
    assert!(s.execute("frame.cut_list", &json!({"feature": "Frame"})).unwrap_err().to_string().contains("not a frame member"));
}

/// Item numbers are kept in the design once renumbered: a member added later takes the next free
/// number, though by profile it would come first, until the list is renumbered again; a note stays
/// with its item.
#[test]
fn cut_list_numbers_and_notes_are_kept_in_the_design() {
    let mut s = Session::default();
    rectangle(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre"}));
    let kept = run(&mut s, "frame.cut_list.renumber", json!({}));
    assert_eq!(kept["numbers"].as_object().unwrap().len(), 2, "{kept}");
    run(&mut s, "frame.cut_list.set", json!({"item": 2, "note": "P-102"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Brace"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 600], [500, 600]]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "frame.member", json!({"sketch": "Brace", "profile": {"kind": "bar", "width": 20, "height": 10}}));
    let list = run(&mut s, "frame.cut_list", json!({}));
    let by_item: Vec<(u64, String, Option<String>)> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["item"].as_u64().unwrap(), i["description"].as_str().unwrap().to_string(), i["note"].as_str().map(str::to_string)))
        .collect();
    assert_eq!(by_item[0].0, 1);
    assert_eq!((by_item[1].0, by_item[1].2.as_deref()), (2, Some("P-102")), "{list}");
    assert!(by_item[2].0 == 3 && by_item[2].1.starts_with("Bar"), "the new item takes the next number: {list}");
    // Renumbered by profile, the bar comes first; the note follows its item.
    run(&mut s, "frame.cut_list.renumber", json!({"order": "profile"}));
    let list = run(&mut s, "frame.cut_list", json!({}));
    assert!(list["items"][0]["description"].as_str().unwrap().starts_with("Bar"), "{list}");
    assert_eq!(list["items"][2]["note"], "P-102", "{list}");
    let saved = serde_json::to_value(&s.doc).unwrap();
    assert_eq!(saved["cut_list"]["descriptions"].as_object().unwrap().len(), 1, "{}", saved["cut_list"]);
    run(&mut s, "frame.cut_list.set", json!({"item": 3, "note": ""}));
    assert!(s.doc.cut_list.descriptions.is_empty());
    assert!(s.execute("frame.cut_list.set", &json!({"item": 9, "note": "x"})).unwrap_err().to_string().contains("no item 9"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.cut_list.descriptions.len(), 1, "undo brings the note back");
}

/// Mirror-image members (a group mirrored) are separate items unless asked to list them as
/// opposite hands.
#[test]
fn opposite_hands_are_listed_together_when_asked() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Frame"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [500, 0]]}));
    run(&mut s, "sketch.line", json!({"points": [[0, 300], [500, 300]]}));
    run(&mut s, "sketch.finish", json!({}));
    let made = run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": {"kind": "angle", "width": 40, "height": 40, "thickness": 4}}));
    let fid = made["feature"].clone();
    let f = s.doc.feature(fid.as_u64().unwrap()).unwrap();
    let lines = solidcraft_doc::frame::member_line_ids(&s.model.state(), &f.kind).unwrap();
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": fid, "command": "frame.member", "params": {"sketch": "Frame",
            "profile": {"kind": "angle", "width": 40, "height": 40, "thickness": 4}, "groups": [{"lines": [lines[0]]}, {"lines": [lines[1]], "mirror": true}]}}),
    );
    let apart = run(&mut s, "frame.cut_list", json!({}));
    assert_eq!(apart["items"].as_array().unwrap().len(), 2, "{apart}");
    let together = run(&mut s, "frame.cut_list", json!({"opposite_hand": true, "format": "csv"}));
    let items = together["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{together}");
    assert!(items[0]["quantity"] == 2 && items[0]["opposite"] == 1, "{together}");
    assert!(together["csv"].as_str().unwrap().lines().nth(1).unwrap().starts_with("1,2,1,"), "{together}");
}

/// With `plates`, gussets are listed with their thickness and size, and count in the mass.
#[test]
fn plates_are_listed_when_asked() {
    let mut s = Session::default();
    rectangle(&mut s);
    run(&mut s, "frame.member", json!({"sketch": "Frame", "profile": tube(), "corner": "mitre"}));
    let st = s.model.state();
    let at = |x: f64, y: f64| {
        st.bodies
            .iter()
            .find(|b| b.body.exact_bounds().is_ok_and(|bb| bb.min.x <= x && x <= bb.max.x && bb.min.y <= y && y <= bb.max.y))
            .unwrap()
            .name
            .clone()
    };
    let (rail, post) = (at(300.0, 0.0), at(600.0, 200.0));
    drop(st);
    run(
        &mut s,
        "frame.gusset",
        json!({"first": {"body": rail, "point": [300, 20, 0]}, "second": {"body": post, "point": [580, 200, 0]}, "d1": 100, "thickness": 5}),
    );
    let without = run(&mut s, "frame.cut_list", json!({}));
    assert!(without.get("plates").is_none(), "{without}");
    let with = run(&mut s, "frame.cut_list", json!({"plates": true}));
    let p = &with["plates"][0];
    assert_eq!((p["kind"].as_str(), p["quantity"].as_u64()), (Some("Gusset"), Some(1)), "{with}");
    assert!(close(&p["thickness"], 5.0) && close(&p["length"], 100.0) && close(&p["width"], 100.0), "{with}");
    assert!(f(&with["total_mass_g"]) > f(&without["total_mass_g"]), "{with}");
}
