//! The Cut List window: `frame.cut_list`'s items (identical frame members together, with their
//! lengths, end angles and mass), each profile's stock length and the total mass, with Export
//! CSV… and Export JSON…. Picking an item selects its members. The Weldments tab's Cut List
//! (`ui.start` of `frame.cut_list`) opens it.

use egui::RichText;
use serde_json::{Value, json};
use solidcraft_engine::Sel;

use crate::SolveApp;
use solidcraft_theme::Tokens;

/// The window's state; the items come from the engine.
#[derive(Default)]
pub struct CutListPanel {
    /// The list for the design revision it was made at.
    cache: Option<(u64, Value)>,
    /// The picked item's index.
    pub picked: Option<usize>,
    /// Why the list couldn't be made.
    pub error: Option<String>,
}

impl CutListPanel {
    /// The cut list for the design as it is now (made again only after a change).
    fn list(&mut self, app: &mut SolveApp) -> Value {
        let revision = app.session.revision;
        if let Some((r, v)) = &self.cache
            && *r == revision
        {
            return v.clone();
        }
        let v = match app.session.execute("frame.cut_list", &json!({})) {
            Ok(v) => {
                self.error = None;
                v
            }
            Err(e) => {
                self.error = Some(e.to_string());
                json!({"items": [], "stock": [], "total_mass_g": 0.0})
            }
        };
        self.cache = Some((revision, v.clone()));
        v
    }
}

/// Open the window.
pub fn open(app: &mut SolveApp) {
    app.cut_list = Some(CutListPanel::default());
}

/// The grid's headers.
const HEADERS: [&str; 8] = ["Item", "Qty", "Description", "Length", "Short length", "Angle 1", "Angle 2", "Mass each"];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// An item's cells after the item number, as the grid shows them.
fn cells(app: &SolveApp, item: &Value) -> [String; 7] {
    let length = |v: &Value| crate::canvas::format_len(&app.session.doc, number(v));
    let angle = |v: &Value| format!("{:.1}°", number(v));
    let mass = item["mass_each_g"].as_f64().map_or_else(String::new, |g| format!("{:.3} kg", g / 1000.0));
    [
        item["quantity"].to_string(),
        item["description"].as_str().unwrap_or_default().to_string(),
        length(&item["length"]),
        length(&item["short_length"]),
        angle(&item["ends"][0]["angle"]),
        angle(&item["ends"][1]["angle"]),
        mass,
    ]
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut d) = app.cut_list.take() else { return };
    let t = Tokens::get();
    let list = d.list(app);
    let items: Vec<Value> = list["items"].as_array().cloned().unwrap_or_default();
    let rows: Vec<[String; 7]> = items.iter().map(|i| cells(app, i)).collect();
    let stock: Vec<String> = list["stock"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            format!(
                "{}: {} {}, {}",
                s["description"].as_str().unwrap_or_default(),
                s["pieces"],
                solidcraft_ui_text::tr("ui.cut_list_pieces"),
                crate::canvas::format_len(&app.session.doc, number(&s["total_length"]))
            )
        })
        .collect();
    let (mut export, mut pick) = (None, None);
    let closed = crate::ui_kit::Window::new(egui::Id::unique("sc_cut_list"), "bom", solidcraft_ui_text::tr_label("Cut List")).cards(3.0).show_body(
        ctx,
        |ui| {
            if items.is_empty() {
                ui.label(RichText::new(solidcraft_ui_text::tr("ui.cut_list_empty")).color(t.text_dim));
            }
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                egui::Grid::new("sc_cut_list_grid").num_columns(HEADERS.len()).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                    for h in HEADERS {
                        ui.label(RichText::new(solidcraft_ui_text::tr_label(h)).strong().color(t.text_dim));
                    }
                    ui.end_row();
                    for (i, (item, row)) in items.iter().zip(&rows).enumerate() {
                        let picked = d.picked == Some(i);
                        if crate::ui_kit::list_picker(ui, &format!("cut_list_{i}"), &item["item"].to_string(), picked).clicked() {
                            pick = Some(if picked { None } else { Some(i) });
                        }
                        for c in row {
                            ui.label(c);
                        }
                        ui.end_row();
                    }
                });
            });
            ui.separator();
            for line in &stock {
                ui.label(RichText::new(line).small());
            }
            ui.label(
                RichText::new(format!("{}: {:.3} kg", solidcraft_ui_text::tr("ui.cut_list_total_mass"), number(&list["total_mass_g"]) / 1000.0))
                    .small()
                    .color(t.text_dim),
            );
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).color(t.error));
            }
            ui.separator();
            ui.horizontal(|ui| {
                for (label, ext) in [("Export CSV…", "csv"), ("Export JSON…", "json")] {
                    if crate::ui_kit::button(ui, label, crate::ui_kit::Button::Normal).clicked() {
                        export = Some(ext);
                    }
                }
            });
        },
    );
    if let Some(p) = pick {
        d.picked = p;
        // The item's members become the selection, so the view shows which they are.
        let members = p.and_then(|i| items.get(i)).and_then(|i| i["members"].as_array().cloned()).unwrap_or_default();
        app.session.selection = members.iter().filter_map(Value::as_str).map(|name| Sel::Body { name: name.to_string() }).collect();
    }
    if let Some(ext) = export {
        app.pick_save_command(&format!("cut-list.{ext}"), &[ext], "frame.cut_list", json!({"format": ext}));
    }
    if !closed {
        app.cut_list = Some(d);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::scenario::Harness;

    #[test]
    fn the_window_lists_items_and_picking_one_selects_its_members() {
        let mut h = Harness::new();
        h.app.run("sketch.create", json!({"plane": "XY", "name": "Frame"})).unwrap();
        h.app.run("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [600, 400]})).unwrap();
        h.app.run("sketch.finish", json!({})).unwrap();
        h.app.run("frame.member", json!({"sketch": "Frame", "profile": {"kind": "rect_tube", "width": 40, "height": 40, "wall": 3}})).unwrap();
        h.call("ui.start", json!({"command": "frame.cut_list"}));
        h.frames(3);
        assert!(h.app.cut_list.is_some(), "ui.start opens the window");
        assert!(h.widget("Export CSV…").is_some());
        let list = h.app.cut_list.as_ref().and_then(|d| d.cache.as_ref()).map(|(_, v)| v.clone()).unwrap_or_default();
        assert_eq!(list["items"].as_array().map(Vec::len), Some(2), "{list}");
        // Picking item 1 selects its two members.
        let Some([x, y]) = h.widget("1") else { panic!("no item row") };
        let item = egui::pos2(x as f32, y as f32);
        h.frame_with(vec![
            egui::Event::PointerMoved(item),
            egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
        ]);
        h.frame_with(vec![egui::Event::PointerButton {
            pos: item,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        h.frames(1);
        assert_eq!(h.app.session.selection.len(), 2, "{:?}", h.app.session.selection);
    }
}
