//! Weldment dialogs for features that work on frame members after they are made
//! (docs/design/weldments.md): Weld and the member exports, and the weld beads drawn in
//! the view; and Save as Weldment Profile, which saves a section drawn in a sketch to a profile
//! folder (`frame.personal.save`).

use serde_json::{Map, Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::doc::expr::Kind as ValueKind;

use crate::SolveApp;
use crate::dialogs::{Kind, check, form_row, shown_expr};
use crate::selection::{BODIES, EDGES, FACES, PLANAR_FACES, POINTS, PROFILES, SelInput, point_value as pt};
use crate::ui_kit::hint_row;

/// A weldment dialog's values (the picks are its selection inputs).
#[derive(Clone, Debug)]
pub enum Wd {
    /// Weld: edges.
    Weld { size: String, both_sides: bool, stitch_length: String, stitch_pitch: String },
    /// Export Member STEP: one member.
    ExportStep,
    /// Export Member DXF: the top and front faces of one member.
    ExportDxf { hidden: bool },
    /// Export Member NC1: one member.
    ExportNc1,
    /// Save as Weldment Profile: a sketch's section, and points on it to name.
    SaveProfile(SaveProfile),
}

/// Save as Weldment Profile's values.
#[derive(Clone, Debug, Default)]
pub struct SaveProfile {
    /// The sketch whose material is the section when no profiles are picked (the sketch the
    /// window was opened on).
    pub whole: Option<u64>,
    pub name: String,
    /// The profile folder, its place in the folder list.
    pub folder: usize,
    /// The categories, levels split by `/`: the subfolders the file goes in.
    pub category: String,
    /// A name for each placement point picked, in order.
    pub point_names: Vec<String>,
    /// The point members sit on: 0 the origin, else the named point before it.
    pub default_point: usize,
    pub material: String,
    pub description: String,
    pub stock_length: String,
    pub replace: bool,
}

impl Wd {
    pub fn title(&self) -> &'static str {
        match self {
            Wd::Weld { .. } => "WELD",
            Wd::ExportStep => "EXPORT MEMBER STEP",
            Wd::ExportDxf { .. } => "EXPORT MEMBER DXF",
            Wd::ExportNc1 => "EXPORT MEMBER NC1",
            Wd::SaveProfile(_) => "SAVE AS WELDMENT PROFILE",
        }
    }

    /// OK asks for a file to write, not a feature.
    pub fn saves(&self) -> bool {
        matches!(self, Wd::ExportStep | Wd::ExportDxf { .. } | Wd::ExportNc1)
    }

    pub fn command(&self) -> &'static str {
        match self {
            Wd::Weld { .. } => "frame.weld",
            Wd::ExportStep => "frame.export_member_step",
            Wd::ExportDxf { .. } => "frame.export_member_dxf",
            Wd::ExportNc1 => "frame.export_member_nc1",
            Wd::SaveProfile(_) => "frame.personal.save",
        }
    }

    /// Changes no body, so there is nothing to preview.
    pub fn previews(&self) -> bool {
        !matches!(self, Wd::SaveProfile(_))
    }
}

fn tr(text: &str) -> &str {
    solidcraft_ui_text::tr_label(text)
}

fn weld_inputs() -> Vec<SelInput> {
    vec![SelInput::new("Edges", EDGES, true)]
}

/// The dialog for a weldment command.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    match id {
        "frame.export_member_nc1" => Some((Kind::Weld(Wd::ExportNc1), vec![SelInput::new("Member", BODIES | FACES, false)])),
        "frame.export_member_step" => Some((Kind::Weld(Wd::ExportStep), vec![SelInput::new("Member", BODIES | FACES, false)])),
        "frame.export_member_dxf" => Some((
            Kind::Weld(Wd::ExportDxf { hidden: false }),
            vec![SelInput::new("Top face", PLANAR_FACES, false), SelInput::new("Front face", PLANAR_FACES, false)],
        )),
        "frame.personal.save" => {
            // Opened on a sketch (its menu, or while it is edited): the sketch's material, unless
            // profiles are picked.
            let is_sketch = |id: u64| app.session.doc.feature(id).is_some_and(|f| matches!(f.kind, FeatureKind::Sketch { .. }));
            let whole = app
                .session
                .selection
                .iter()
                .find_map(|sel| match sel {
                    Sel::Feature { id } if is_sketch(*id) => Some(*id),
                    _ => None,
                })
                .or(app.session.active_sketch);
            let inputs = vec![SelInput::new("Section (optional)", PROFILES, true), SelInput::new("Placement points", POINTS, true)];
            Some((Kind::Weld(Wd::SaveProfile(SaveProfile { whole, ..SaveProfile::default() })), inputs))
        }
        "frame.weld" => {
            let size = crate::dialogs::length_default(&app.session, "5");
            Some((Kind::Weld(Wd::Weld { size, both_sides: false, stitch_length: String::new(), stitch_pitch: String::new() }), weld_inputs()))
        }
        _ => None,
    }
}

pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut Wd, inputs: &[SelInput]) -> bool {
    let mut enter = false;
    match k {
        Wd::SaveProfile(sp) => enter |= save_profile_rows(app, ui, sp, inputs),
        Wd::ExportStep => {
            crate::ui_kit::status_line(ui, tr("ui.frame_export_step.hint"), crate::ui_kit::StatusTone::Dim);
            beside_row(app, ui);
        }
        Wd::ExportNc1 => {
            crate::ui_kit::status_line(ui, tr("ui.frame_export_nc1.hint"), crate::ui_kit::StatusTone::Dim);
            beside_row(app, ui);
        }
        Wd::ExportDxf { hidden } => {
            crate::ui_kit::status_line(ui, tr("ui.frame_export_dxf.hint"), crate::ui_kit::StatusTone::Dim);
            form_row(ui, "ui.frame_export_hidden", |ui| check(ui, hidden));
            beside_row(app, ui);
        }
        Wd::Weld { size, both_sides, stitch_length, stitch_pitch } => {
            crate::ui_kit::status_line(ui, tr("ui.frame_weld.hint"), crate::ui_kit::StatusTone::Dim);
            enter |= crate::ui_kit::expr_row(ui, "ui.frame_weld_size", size);
            form_row(ui, "ui.frame_weld_both", |ui| check(ui, both_sides));
            enter |= hint_row(ui, "ui.frame_weld_stitch_length", stitch_length, tr("ui.frame_weld_continuous"));
            if !stitch_length.trim().is_empty() {
                enter |= crate::ui_kit::expr_row(ui, "ui.frame_weld_stitch_pitch", stitch_pitch);
            }
        }
    }
    enter
}

/// The command OK runs.
pub fn commands(app: &SolveApp, k: &Wd, inputs: &[SelInput], _extra: &Map<String, Value>) -> Result<Vec<(String, Value)>, String> {
    match k {
        Wd::ExportStep | Wd::ExportDxf { .. } | Wd::ExportNc1 => Ok(vec![(k.command().into(), export_params(k, inputs)?)]),
        Wd::SaveProfile(sp) => Ok(vec![("frame.personal.save".into(), save_profile_params(app, sp, inputs)?)]),
        Wd::Weld { size, both_sides, stitch_length, stitch_pitch } => {
            let edges: Vec<Value> = inputs
                .first()
                .map(|i| {
                    i.items
                        .iter()
                        .filter_map(|sel| match sel {
                            Sel::Edge { body, point, .. } => Some(json!({"body": body, "point": pt(*point)})),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            if edges.is_empty() {
                return Err("pick the edges to weld".into());
            }
            let mut p = json!({"edges": edges, "size": size.trim()});
            if *both_sides {
                p["both_sides"] = json!(true);
            }
            if !stitch_length.trim().is_empty() {
                p["stitch_length"] = json!(stitch_length.trim());
                p["stitch_pitch"] = json!(stitch_pitch.trim());
            }
            Ok(vec![("frame.weld".into(), p)])
        }
    }
}

/// The dialog editing a weldment feature, filled from it.
pub fn for_feature(_app: &SolveApp, kind: &FeatureKind) -> Option<(Kind, Vec<SelInput>, Map<String, Value>)> {
    match kind {
        FeatureKind::FrameWeld { edges, size, both_sides, stitch_length, stitch_pitch } => {
            let mut inputs = weld_inputs();
            if let Some(i) = inputs.first_mut() {
                i.items = edges.iter().map(|e| Sel::Edge { body: e.body.0.clone(), index: 0, point: e.point.0 }).collect();
            }
            let o = |x: &Option<solidcraft_engine::doc::Expr>| x.as_ref().map(|x| shown_expr(x, ValueKind::Length)).unwrap_or_default();
            let k = Wd::Weld {
                size: shown_expr(size, ValueKind::Length),
                both_sides: *both_sides,
                stitch_length: o(stitch_length),
                stitch_pitch: o(stitch_pitch),
            };
            Some((Kind::Weld(k), inputs, Map::new()))
        }
        _ => None,
    }
}

/// An export's parameters from its picks (without `path`).
fn export_params(k: &Wd, inputs: &[SelInput]) -> Result<Value, String> {
    let pick = |i: usize| inputs.get(i).and_then(|x| x.items.first());
    match k {
        Wd::ExportStep | Wd::ExportNc1 => match pick(0) {
            Some(Sel::Body { name }) | Some(Sel::Face { body: name, .. }) => Ok(json!({"body": name})),
            _ => Err("pick the member to export".into()),
        },
        Wd::ExportDxf { hidden } => match (pick(0), pick(1)) {
            (Some(Sel::Face { body, point, .. }), Some(Sel::Face { body: other, point: front, .. })) if body == other => {
                Ok(json!({"body": body, "top": pt(*point), "front": pt(*front), "hidden": hidden}))
            }
            (Some(Sel::Face { .. }), Some(Sel::Face { .. })) => Err("pick the top and front faces on the same member".into()),
            _ => Err("pick the member's top face, then its front face".into()),
        },
        _ => Err("not an export".into()),
    }
}

/// OK of an export: ask where to write it, then write it.
pub fn run_export(app: &mut SolveApp, k: &Wd, inputs: &[SelInput]) -> Result<(), String> {
    let params = export_params(k, inputs)?;
    let body = params["body"].as_str().unwrap_or("member").to_string();
    let ext = match k {
        Wd::ExportStep => "step",
        Wd::ExportNc1 => "nc1",
        _ => "dxf",
    };
    save_export(app, &format!("{body}.{ext}"), ext, k.command(), params);
    Ok(())
}

/// "Save next to the design", remembered for every export and the cut list.
pub(crate) fn beside_row(app: &mut SolveApp, ui: &mut egui::Ui) {
    form_row(ui, "ui.frame_export_beside", |ui| check(ui, &mut app.preferences.frame_exports_beside_design));
}

/// Where an export goes without asking: `name` in the open design's folder, while "Save next to
/// the design" is on and the design has been saved to a file. `None`: ask with the save dialog.
pub(crate) fn beside_design(app: &SolveApp, name: &str) -> Option<String> {
    if cfg!(target_arch = "wasm32") || !app.preferences.frame_exports_beside_design {
        return None;
    }
    let design = std::path::Path::new(app.session.path.as_deref()?);
    let folder = design.parent().filter(|f| !f.as_os_str().is_empty())?;
    Some(folder.join(name).to_string_lossy().into_owned())
}

/// Write an export (`command` with `params`, and `path`): beside the design ([`beside_design`]),
/// saying where in the status line, or where the save dialog says.
pub(crate) fn save_export(app: &mut SolveApp, name: &str, ext: &str, command: &str, params: Value) {
    let Some(path) = beside_design(app, name) else {
        app.pick_save_command(name, &[ext], command, params);
        return;
    };
    let mut params = params;
    params["path"] = json!(path);
    match app.run(command, params) {
        Ok(_) => app.set_status(solidcraft_ui_text::format("ui.frame_export_saved", &[("path", &path)]), false),
        Err(e) => app.set_status(e, true),
    }
}

/// The section's sketch and the profiles picked in it (none: its material).
fn profile_section(sp: &SaveProfile, inputs: &[SelInput]) -> Result<(Option<u64>, Vec<usize>), String> {
    let picked: Vec<(u64, usize)> = inputs
        .first()
        .map(|i| {
            i.items
                .iter()
                .filter_map(|sel| match sel {
                    Sel::Profile { sketch, index } => Some((*sketch, *index)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let sketch = picked.first().map(|x| x.0).or(sp.whole);
    if picked.iter().any(|x| Some(x.0) != sketch) {
        return Err("the section's profiles must be in one sketch".into());
    }
    Ok((sketch, picked.into_iter().map(|x| x.1).collect()))
}

/// The placement points picked: each point's sketch and its id there.
fn placement_points(inputs: &[SelInput]) -> Vec<(Option<u64>, String)> {
    inputs
        .get(1)
        .map(|i| {
            i.items
                .iter()
                .filter_map(|sel| match sel {
                    Sel::SketchPoint { id } => Some(match id.split_once(':') {
                        Some((sketch, point)) => (sketch.parse().ok(), point.to_string()),
                        None => (None, id.clone()),
                    }),
                    Sel::ExternalSketchPoint { sketch, id } => Some((Some(*sketch), id.clone())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The name a placement point gets unless one is typed: its id in the sketch.
fn point_name(sp: &SaveProfile, points: &[(Option<u64>, String)], i: usize) -> String {
    let typed = sp.point_names.get(i).map(|n| n.trim()).unwrap_or_default();
    if typed.is_empty() { points.get(i).map(|p| format!("point_{}", p.1)).unwrap_or_default() } else { typed.to_string() }
}

/// The name a profile gets unless one is typed: its sketch's.
fn profile_name(app: &SolveApp, sketch: Option<u64>) -> String {
    sketch.and_then(|id| app.session.doc.feature(id)).map(|f| f.name.clone()).unwrap_or_default()
}

fn save_profile_rows(app: &mut SolveApp, ui: &mut egui::Ui, sp: &mut SaveProfile, inputs: &[SelInput]) -> bool {
    crate::ui_kit::status_line(ui, tr("ui.frame_save_profile.hint"), crate::ui_kit::StatusTone::Dim);
    let mut enter = false;
    let sketch = profile_section(sp, inputs).ok().and_then(|x| x.0);
    enter |= crate::ui_kit::text_row(ui, "ui.frame_save_name", &mut sp.name, &profile_name(app, sketch)).0;
    let folders: Vec<String> = app
        .frame_personal
        .folders()
        .iter()
        .map(|f| if f.writable { f.path.clone() } else { solidcraft_ui_text::format("ui.frame_folder_read_only_label", &[("folder", &f.path)]) })
        .collect();
    if folders.len() > 1 {
        let labels: Vec<&str> = folders.iter().map(String::as_str).collect();
        crate::ui_kit::choice_row(ui, "ui.frame_profile_folder", "frame_profile_folder", &mut sp.folder, &labels);
    }
    enter |= crate::ui_kit::text_row(ui, "ui.frame_category", &mut sp.category, tr("ui.frame_category_hint")).0;
    let points = placement_points(inputs);
    sp.point_names.resize(points.len(), String::new());
    for (i, (_, id)) in points.iter().enumerate() {
        let label = solidcraft_ui_text::format("ui.frame_point_name", &[("point", id)]);
        let hint = format!("point_{id}");
        if let Some(name) = sp.point_names.get_mut(i) {
            enter |= crate::ui_kit::text_row(ui, &label, name, &hint).0;
        }
    }
    let mut labels = vec![solidcraft_ui_text::tr("ui.frame_point_origin").to_string()];
    labels.extend((0..points.len()).map(|i| point_name(sp, &points, i)));
    sp.default_point = sp.default_point.min(labels.len() - 1);
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    crate::ui_kit::choice_row(ui, "ui.frame_default_point", "frame_save_default_point", &mut sp.default_point, &labels);
    enter |= crate::ui_kit::text_row(ui, "ui.frame_material", &mut sp.material, "").0;
    enter |= crate::ui_kit::text_row(ui, "ui.frame_description", &mut sp.description, "").0;
    enter |= hint_row(ui, "ui.frame_stock_length", &mut sp.stock_length, "6 m");
    form_row(ui, "ui.frame_save_replace", |ui| check(ui, &mut sp.replace));
    if app.frame_personal.loading() {
        crate::ui_kit::status_line(ui, tr("ui.frame_folders_reading"), crate::ui_kit::StatusTone::Dim);
    }
    enter
}

/// `frame.personal.save`'s parameters from the window.
fn save_profile_params(app: &SolveApp, sp: &SaveProfile, inputs: &[SelInput]) -> Result<Value, String> {
    let (sketch, profiles) = profile_section(sp, inputs)?;
    let sketch = sketch.ok_or("pick the section's profiles in a sketch")?;
    let name = Some(sp.name.trim()).filter(|n| !n.is_empty()).map_or_else(|| profile_name(app, Some(sketch)), str::to_string);
    let mut p = json!({"name": name, "profile_sketch": sketch.to_string()});
    if !profiles.is_empty() {
        p["profiles"] = json!(profiles);
    }
    let levels: Vec<&str> = sp.category.split('/').map(str::trim).filter(|l| !l.is_empty()).collect();
    if !levels.is_empty() {
        p["category"] = json!(levels);
    }
    let points = placement_points(inputs);
    if points.iter().any(|(s, _)| s.is_some_and(|s| s != sketch)) {
        return Err("pick placement points in the section's sketch".into());
    }
    if !points.is_empty() {
        let named: Vec<Value> = (0..points.len()).map(|i| json!({"name": point_name(sp, &points, i), "sketch_point": points[i].1})).collect();
        p["points"] = json!(named);
    }
    if sp.default_point > 0 {
        p["default_point"] = json!(point_name(sp, &points, sp.default_point - 1));
    }
    for (key, v) in [("material", &sp.material), ("description", &sp.description), ("stock_length", &sp.stock_length)] {
        if !v.trim().is_empty() {
            p[key] = json!(v.trim());
        }
    }
    if let Some(f) = app.frame_personal.folders().get(sp.folder) {
        p["folder"] = json!(f.path);
    }
    if sp.replace {
        p["replace"] = json!(true);
    }
    Ok(p)
}

/// OK of Save as Weldment Profile: save it, and say where it went.
pub fn run_save_profile(app: &mut SolveApp, sp: &SaveProfile, inputs: &[SelInput]) -> Result<(), String> {
    let p = save_profile_params(app, sp, inputs)?;
    let out = app.run("frame.personal.save", p)?;
    let e = &out["profiles"][0];
    let name = e["name"].as_str().unwrap_or_default();
    let place = e["file"].as_str().or_else(|| e["folder"].as_str()).unwrap_or_default();
    app.set_status(solidcraft_ui_text::format("ui.frame_profile_saved", &[("name", name), ("place", place)]), false);
    Ok(())
}

thread_local! {
    /// The weld beads last worked out, and the model state they were worked out for.
    static BEADS: std::cell::RefCell<Option<(std::sync::Arc<solidcraft_engine::doc::ModelState>, Vec<solidcraft_engine::doc::frame::Weld>)>> = const { std::cell::RefCell::new(None) };
}

/// The weld beads in the view: each weld feature's edges drawn as a thick stroke (display only;
/// the welds change no body). Worked out again only when the model does.
pub fn weld_beads(app: &SolveApp, painter: &egui::Painter, proj: &crate::viewport::Proj) {
    let doc = &app.session.doc;
    if !doc.features.iter().any(|f| !f.suppressed && matches!(f.kind, FeatureKind::FrameWeld { .. })) {
        return;
    }
    let st = app.session.model.state();
    let t = solidcraft_theme::Tokens::get();
    BEADS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.as_ref().is_some_and(|(at, _)| std::sync::Arc::ptr_eq(at, &st)) {
            *cache = Some((st.clone(), solidcraft_engine::doc::frame::welds(doc, &st)));
        }
        let Some((_, welds)) = cache.as_ref() else { return };
        for w in welds {
            for path in &w.paths {
                let pts: Vec<egui::Pos2> = path.iter().filter_map(|p| proj.to_screen(*p)).collect();
                if pts.len() >= 2 {
                    painter.add(egui::Shape::line(pts, egui::Stroke::new(4.0, t.manipulator)));
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "dialogs_weldment_tests.rs"]
mod tests;
