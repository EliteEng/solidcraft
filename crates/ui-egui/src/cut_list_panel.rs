//! The Cut List window: `frame.cut_list`'s items (identical frame members together, with their
//! lengths, end angles and mass), each profile's stock length and the total mass, with Export
//! CSV… and Export JSON…. Picking an item selects its members and offers its note; Keep Numbers
//! and Renumber keep the item numbers in the design. The Weldments tab's Cut List (`ui.start` of
//! `frame.cut_list`) opens it.

use egui::RichText;
use serde_json::{Value, json};
use solidcraft_engine::Sel;

use crate::SolveApp;
use solidcraft_theme::Tokens;

/// The window's state; the items come from the engine.
#[derive(Default)]
pub struct CutListPanel {
    /// The list for the design revision (and the choices below) it was made at.
    cache: Option<((u64, bool, bool), Value)>,
    /// Mirror-image pieces listed as one item.
    pub opposite: bool,
    /// End caps and gussets listed as plates.
    pub plates: bool,
    /// The picked item's note as typed, and the item key it was loaded for.
    pub note: String,
    note_for: Option<String>,
    /// The picked item's index.
    pub picked: Option<usize>,
    /// Why the list couldn't be made.
    pub error: Option<String>,
}

impl CutListPanel {
    /// The cut list for the design as it is now (made again only after a change).
    fn list(&mut self, app: &mut SolveApp) -> Value {
        let revision = (app.session.revision, self.opposite, self.plates);
        if let Some((r, v)) = &self.cache
            && *r == revision
        {
            return v.clone();
        }
        let v = match app.session.execute("frame.cut_list", &json!({"opposite_hand": self.opposite, "plates": self.plates})) {
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
const HEADERS: [&str; 9] = ["Item", "Qty", "Description", "Length", "Short length", "Angle 1", "Angle 2", "Mass each", "Note"];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// An item's cells after the item number, as the grid shows them.
fn cells(app: &SolveApp, item: &Value) -> [String; 8] {
    // To a hundredth of a millimetre, in the design's units.
    let length = |v: &Value| crate::canvas::format_len_drag(&app.session.doc, number(v), 0.01, 2);
    // A coped end reads `cope` after its angle.
    let angle = |v: &Value| {
        let a = format!("{:.1}°", number(&v["angle"]));
        if v["cope"].as_bool() == Some(true) { format!("{a} {}", solidcraft_ui_text::tr("ui.frame_corner_cope")) } else { a }
    };
    let mass = item["mass_each_g"].as_f64().map_or_else(String::new, |g| format!("{:.3} kg", g / 1000.0));
    // Opposite hands listed together: how many of them are the other hand.
    let quantity = match item["opposite"].as_u64().filter(|n| *n > 0) {
        Some(n) => format!("{} ({n} {})", item["quantity"], solidcraft_ui_text::tr("ui.cut_list_opposite")),
        None => item["quantity"].to_string(),
    };
    [
        quantity,
        item["description"].as_str().unwrap_or_default().to_string(),
        length(&item["length"]),
        length(&item["short_length"]),
        angle(&item["ends"][0]),
        angle(&item["ends"][1]),
        mass,
        item["note"].as_str().unwrap_or_default().to_string(),
    ]
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut d) = app.cut_list.take() else { return };
    let t = Tokens::get();
    let list = d.list(app);
    let items: Vec<Value> = list["items"].as_array().cloned().unwrap_or_default();
    let rows: Vec<[String; 8]> = items.iter().map(|i| cells(app, i)).collect();
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
                crate::canvas::format_len_drag(&app.session.doc, number(&s["total_length"]), 0.01, 2)
            )
        })
        .collect();
    // The welds, one line per size after the stock.
    let len = |v: &Value| crate::canvas::format_len_drag(&app.session.doc, number(v), 0.01, 2);
    let stock: Vec<String> = stock
        .into_iter()
        .chain(list["welds"].as_array().into_iter().flatten().map(|w| {
            format!(
                "{} {}: {}, {:.3} kg",
                solidcraft_ui_text::tr("ui.cut_list_weld"),
                len(&w["size"]),
                len(&w["length"]),
                number(&w["mass_g"]) / 1000.0
            )
        }))
        .chain(list["plates"].as_array().into_iter().flatten().map(|p| {
            format!(
                "{} × {} {}, {} × {} × {}",
                p["quantity"],
                solidcraft_ui_text::tr_label(p["kind"].as_str().unwrap_or_default()),
                p["material"].as_str().unwrap_or_default(),
                len(&p["length"]),
                len(&p["width"]),
                len(&p["thickness"])
            )
        }))
        .collect();
    let picked_item = d.picked.and_then(|i| items.get(i)).cloned();
    if picked_item.as_ref().map(|i| i["key"].as_str().unwrap_or_default().to_string()) != d.note_for {
        d.note_for = picked_item.as_ref().map(|i| i["key"].as_str().unwrap_or_default().to_string());
        d.note = picked_item.as_ref().and_then(|i| i["note"].as_str()).unwrap_or_default().to_string();
    }
    let mut change: Option<(&str, Value)> = None;
    let (mut export, mut pick) = (None, None);
    let mut beside = app.preferences.frame_exports_beside_design;
    let closed = crate::ui_kit::Window::new(egui::Id::unique("sc_cut_list"), "bom", solidcraft_ui_text::tr_label("Cut List")).cards(3.0).show_body(
        ctx,
        |ui| {
            if items.is_empty() {
                ui.label(RichText::new(solidcraft_ui_text::tr("ui.cut_list_empty")).color(t.text_dim));
            }
            egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
                // Each column as wide as its widest text, worked out before drawing, in rows of
                // fixed-width cells: every cell shows its whole text.
                let font = egui::TextStyle::Body.resolve(ui.style());
                let width = |s: &str| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size().x.ceil();
                let mut widths: Vec<f32> = HEADERS.iter().map(|h| width(solidcraft_ui_text::tr_label(h))).collect();
                let padding = 2.0 * ui.spacing().button_padding.x;
                for (item, row) in items.iter().zip(&rows) {
                    if let Some(w) = widths.get_mut(0) {
                        *w = w.max(width(&item["item"].to_string()) + padding);
                    }
                    for (k, c) in row.iter().enumerate() {
                        if let Some(w) = widths.get_mut(k + 1) {
                            *w = w.max(width(c));
                        }
                    }
                }
                let cell = |ui: &mut egui::Ui, w: f32, add: &mut dyn FnMut(&mut egui::Ui) -> egui::Response| {
                    ui.allocate_ui_with_layout(egui::vec2(w, crate::ui_kit::row_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(w);
                        add(ui)
                    })
                    .inner
                };
                let spacing = 14.0;
                let total: f32 = widths.iter().sum::<f32>() + spacing * widths.len().saturating_sub(1) as f32;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    for (h, w) in HEADERS.iter().zip(&widths) {
                        cell(ui, *w, &mut |ui| {
                            ui.add(egui::Label::new(RichText::new(solidcraft_ui_text::tr_label(h)).strong().color(t.text_dim)).extend())
                        });
                    }
                });
                for (i, (item, row)) in items.iter().zip(&rows).enumerate() {
                    let picked = d.picked == Some(i);
                    if i % 2 == 0 {
                        let top = ui.cursor().min;
                        ui.painter().rect_filled(egui::Rect::from_min_size(top, egui::vec2(total, crate::ui_kit::row_height())), 2.0, t.hover);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = spacing;
                        let first = widths.first().copied().unwrap_or(0.0);
                        if cell(ui, first, &mut |ui| crate::ui_kit::list_picker(ui, &format!("cut_list_{i}"), &item["item"].to_string(), picked))
                            .clicked()
                        {
                            pick = Some(if picked { None } else { Some(i) });
                        }
                        for (k, c) in row.iter().enumerate() {
                            cell(ui, widths.get(k + 1).copied().unwrap_or(0.0), &mut |ui| ui.add(egui::Label::new(c.as_str()).extend()));
                        }
                    });
                }
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
            // Wrapped, so the window keeps its width however long the labels are.
            ui.horizontal_wrapped(|ui| {
                crate::ui_kit::checkbox(ui, solidcraft_ui_text::tr("ui.cut_list_opposite_together"), &mut d.opposite);
                crate::ui_kit::checkbox(ui, solidcraft_ui_text::tr("ui.cut_list_plates"), &mut d.plates);
                crate::ui_kit::checkbox(ui, solidcraft_ui_text::tr("ui.frame_export_beside"), &mut beside);
            });
            if let Some(item) = &picked_item {
                ui.horizontal(|ui| {
                    ui.label(format!("{} {}", solidcraft_ui_text::tr("ui.cut_list_note_for"), item["item"]));
                    // Enter in the note saves it, as the button does.
                    let (entered, _) = crate::ui_kit::text_field(ui, &mut d.note, "");
                    if crate::ui_kit::button(ui, solidcraft_ui_text::tr("ui.cut_list_save_note"), crate::ui_kit::Button::Small).clicked() || entered {
                        change = Some(("frame.cut_list.set", json!({"item": item["item"], "note": d.note, "opposite_hand": d.opposite})));
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                if crate::ui_kit::button(ui, solidcraft_ui_text::tr("ui.cut_list_keep_numbers"), crate::ui_kit::Button::Normal).clicked() {
                    change = Some(("frame.cut_list.renumber", json!({"order": "current", "opposite_hand": d.opposite})));
                }
                if crate::ui_kit::button(ui, solidcraft_ui_text::tr("ui.cut_list_renumber"), crate::ui_kit::Button::Normal).clicked() {
                    change = Some(("frame.cut_list.renumber", json!({"order": "profile", "opposite_hand": d.opposite})));
                }
                for (label, ext) in [("Export CSV…", "csv"), ("Export JSON…", "json")] {
                    if crate::ui_kit::button(ui, label, crate::ui_kit::Button::Normal).clicked() {
                        export = Some(ext);
                    }
                }
            });
        },
    );
    app.preferences.frame_exports_beside_design = beside;
    if let Some(p) = pick {
        d.picked = p;
        // The item's members become the selection, so the view shows which they are.
        let members = p.and_then(|i| items.get(i)).and_then(|i| i["members"].as_array().cloned()).unwrap_or_default();
        app.session.selection = members.iter().filter_map(Value::as_str).map(|name| Sel::Body { name: name.to_string() }).collect();
    }
    if let Some(ext) = export {
        let name = format!("{}.{ext}", crate::dialogs_weldment::export_stem(app, "CutList"));
        crate::dialogs_weldment::save_export(
            app,
            &name,
            ext,
            "frame.cut_list",
            json!({"format": ext, "opposite_hand": d.opposite, "plates": d.plates}),
        );
    }
    if let Some((cmd, p)) = change
        && let Err(e) = app.run(cmd, p)
    {
        d.error = Some(e);
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
