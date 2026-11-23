//! Weldment dialogs for features that work on frame members after they are made
//! (docs/design/weldments.md): Trim/Extend, Gusset and Weld, and the weld beads drawn in
//! the view; and Save as Weldment Profile, which saves a section drawn in a sketch to a profile
//! folder (`frame.personal.save`).

use serde_json::{Map, Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::doc::expr::Kind as ValueKind;
use solidcraft_engine::doc::frame::TrimEndSide as End;

use crate::SolveApp;
use crate::dialogs::{Kind, check, face_sel, form_row, planar_face, plane_sel, shown_expr};
use crate::selection::{BODIES, EDGES, FACES, PLANAR_FACES, PLANES, POINTS, PROFILES, SelInput, point_value as pt};
use crate::ui_kit::hint_row;

/// A weldment dialog's values (the picks are its selection inputs).
#[derive(Clone, Debug)]
pub enum Wd {
    /// Trim/Extend: member ends (face picks near each end), then the boundary.
    Trim { gap: String, extend: bool },
    /// Gusset: two flat faces.
    Gusset { chamfered: bool, d1: String, d2: String, thickness: String, chamfer: String, clip: String, offset: String },
    /// Weld: edges.
    Weld { size: String, both_sides: bool, stitch_length: String, stitch_pitch: String },
    /// Export Member STEP: one member.
    ExportStep,
    /// Export Member DXF: the top and front faces of one member.
    ExportDxf { hidden: bool },
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
            Wd::Trim { .. } => "TRIM/EXTEND",
            Wd::Gusset { .. } => "GUSSET",
            Wd::Weld { .. } => "WELD",
            Wd::ExportStep => "EXPORT MEMBER STEP",
            Wd::ExportDxf { .. } => "EXPORT MEMBER DXF",
            Wd::SaveProfile(_) => "SAVE AS WELDMENT PROFILE",
        }
    }

    /// OK asks for a file to write, not a feature.
    pub fn saves(&self) -> bool {
        matches!(self, Wd::ExportStep | Wd::ExportDxf { .. })
    }

    pub fn command(&self) -> &'static str {
        match self {
            Wd::Trim { .. } => "frame.trim",
            Wd::Gusset { .. } => "frame.gusset",
            Wd::Weld { .. } => "frame.weld",
            Wd::ExportStep => "frame.export_member_step",
            Wd::ExportDxf { .. } => "frame.export_member_dxf",
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

fn gusset_inputs() -> Vec<SelInput> {
    vec![SelInput::new("First face", PLANAR_FACES, false), SelInput::new("Second face", PLANAR_FACES, false)]
}

fn weld_inputs() -> Vec<SelInput> {
    vec![SelInput::new("Edges", EDGES, true)]
}

fn ends_input() -> Vec<SelInput> {
    vec![SelInput::new("Member ends", FACES | BODIES, true)]
}

fn trim_inputs() -> Vec<SelInput> {
    let mut inputs = ends_input();
    inputs.push(SelInput::new("Boundary", PLANES | FACES | BODIES, false));
    inputs
}

/// The member ends picked (`members` for `frame.trim`): a face pick near the
/// end, or a body pick with its end kept in `extra` by Edit Feature.
fn member_ends(inputs: &[SelInput], extra: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let ends = inputs.first().map(|i| i.items.clone()).unwrap_or_default();
    if ends.is_empty() {
        return Err("pick each member near the end to work on".into());
    }
    let mut kept: Vec<(String, String)> = extra
        .get("trim_ends")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| Some((x.get(0)?.as_str()?.to_string(), x.get(1)?.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let mut members: Vec<Value> = Vec::with_capacity(ends.len());
    for sel in &ends {
        match sel {
            Sel::Face { body, point, .. } => members.push(json!({"body": body, "near": pt(*point)})),
            Sel::Body { name } => {
                let k = kept.iter().position(|(b, _)| b == name).ok_or("pick each member on a face near the end to work on")?;
                let (_, end) = kept.remove(k);
                members.push(json!({"body": name, "end": end}));
            }
            _ => return Err("pick each member on a face near the end to work on".into()),
        }
    }
    Ok(members)
}

/// Edit Feature's picks for member ends: each member as a body pick, its end kept in `extra`.
fn end_picks(members: &[solidcraft_engine::doc::frame::TrimEnd], inputs: &mut [SelInput], extra: &mut Map<String, Value>) {
    if let Some(i) = inputs.first_mut() {
        i.items = members.iter().map(|m| Sel::Body { name: m.body.0.clone() }).collect();
    }
    let ends: Vec<Value> = members.iter().map(|m| json!([m.body.0, if m.end == End::Start { "start" } else { "end" }])).collect();
    extra.insert("trim_ends".into(), Value::Array(ends));
}

/// The dialog for a weldment command.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    match id {
        "frame.trim" => Some((Kind::Weld(Wd::Trim { gap: String::new(), extend: true }), trim_inputs())),
        "frame.gusset" => {
            let l = |n: &str| crate::dialogs::length_default(&app.session, n);
            let k = Wd::Gusset {
                chamfered: false,
                d1: l("100"),
                d2: String::new(),
                thickness: l("5"),
                chamfer: l("10"),
                clip: String::new(),
                offset: String::new(),
            };
            Some((Kind::Weld(k), gusset_inputs()))
        }
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
        Wd::Trim { gap, extend } => {
            crate::ui_kit::status_line(ui, tr("ui.frame_trim.hint"), crate::ui_kit::StatusTone::Dim);
            enter |= hint_row(ui, "ui.frame_trim_gap", gap, "0 mm");
            form_row(ui, "ui.frame_trim_extend", |ui| check(ui, extend));
        }
        Wd::Gusset { chamfered, d1, d2, thickness, chamfer, clip, offset } => {
            crate::ui_kit::status_line(ui, tr("ui.frame_gusset.hint"), crate::ui_kit::StatusTone::Dim);
            let mut shape = usize::from(*chamfered);
            let labels = [tr("ui.frame_gusset_triangle"), tr("ui.frame_gusset_chamfered")];
            crate::ui_kit::choice_row(ui, "ui.frame_gusset_shape", "frame_gusset_shape", &mut shape, &labels);
            *chamfered = shape == 1;
            enter |= crate::ui_kit::expr_row(ui, "ui.frame_gusset_d1", d1);
            enter |= hint_row(ui, "ui.frame_gusset_d2", d2, tr("ui.frame_gusset_same"));
            enter |= crate::ui_kit::expr_row(ui, "ui.frame_cap_thickness", thickness);
            if *chamfered {
                enter |= crate::ui_kit::expr_row(ui, "ui.frame_gusset_chamfer", chamfer);
            }
            enter |= hint_row(ui, "ui.frame_gusset_clip", clip, "0 mm");
            enter |= hint_row(ui, "ui.frame_gusset_offset", offset, "0 mm");
        }
        Wd::ExportStep => {
            crate::ui_kit::status_line(ui, tr("ui.frame_export_step.hint"), crate::ui_kit::StatusTone::Dim);
        }
        Wd::ExportDxf { hidden } => {
            crate::ui_kit::status_line(ui, tr("ui.frame_export_dxf.hint"), crate::ui_kit::StatusTone::Dim);
            form_row(ui, "ui.frame_export_hidden", |ui| check(ui, hidden));
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
pub fn commands(app: &SolveApp, k: &Wd, inputs: &[SelInput], extra: &Map<String, Value>) -> Result<Vec<(String, Value)>, String> {
    let s = &app.session;
    match k {
        Wd::Gusset { chamfered, d1, d2, thickness, chamfer, clip, offset } => {
            let face = |k: usize, what: &str| -> Result<Value, String> {
                match inputs.get(k).and_then(|i| i.items.first()) {
                    Some(Sel::Face { body, point, .. }) => Ok(json!({"body": body, "point": pt(*point)})),
                    _ => Err(format!("pick the {what} flat face")),
                }
            };
            let mut p = json!({"first": face(0, "first")?, "second": face(1, "second")?, "d1": d1.trim(), "thickness": thickness.trim()});
            for (key, v) in [("d2", d2), ("clip", clip), ("offset", offset)] {
                if !v.trim().is_empty() {
                    p[key] = json!(v.trim());
                }
            }
            if *chamfered {
                p["shape"] = json!("chamfered");
                p["chamfer"] = json!(chamfer.trim());
            }
            Ok(vec![("frame.gusset".into(), p)])
        }
        Wd::ExportStep | Wd::ExportDxf { .. } => Ok(vec![(k.command().into(), export_params(k, inputs)?)]),
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
        Wd::Trim { gap, extend } => {
            let members = member_ends(inputs, extra)?;
            let mut p = json!({"members": members});
            match inputs.get(1).and_then(|i| i.items.first()) {
                Some(Sel::Plane { name }) => p["plane"] = json!(name),
                Some(Sel::Face { body, index, point }) => match planar_face(s, body, *index) {
                    Some(_) => p["face"] = json!({"body": body, "point": pt(*point)}),
                    None => p["tool"] = json!(body),
                },
                Some(Sel::Body { name }) => p["tool"] = json!(name),
                _ => return Err("pick the boundary: a plane, a flat face or a body".into()),
            }
            if !gap.trim().is_empty() {
                p["gap"] = json!(gap.trim());
            }
            if *extend {
                p["extend"] = json!(true);
            }
            Ok(vec![("frame.trim".into(), p)])
        }
    }
}

/// The dialog editing a weldment feature, filled from it.
pub fn for_feature(app: &SolveApp, kind: &FeatureKind) -> Option<(Kind, Vec<SelInput>, Map<String, Value>)> {
    let s = &app.session;
    match kind {
        FeatureKind::FrameTrim { members, plane, face, tool, gap, extend } => {
            let mut inputs = trim_inputs();
            let mut extra = Map::new();
            end_picks(members, &mut inputs, &mut extra);
            if let Some(i) = inputs.get_mut(1) {
                i.items = match (plane, face, tool) {
                    (Some(p), _, _) => plane_sel(s, p).into_iter().collect(),
                    (_, Some(f), _) => face_sel(s, f.point.0).into_iter().collect(),
                    (_, _, Some(t)) => vec![Sel::Body { name: t.0.clone() }],
                    _ => Vec::new(),
                };
            }
            let gap = gap.as_ref().map(|g| shown_expr(g, ValueKind::Length)).unwrap_or_default();
            Some((Kind::Weld(Wd::Trim { gap, extend: *extend }), inputs, extra))
        }
        FeatureKind::FrameGusset { first, second, shape, d1, d2, thickness, chamfer, clip, offset } => {
            let mut inputs = gusset_inputs();
            for (i, fa) in inputs.iter_mut().zip([first, second]) {
                i.items = crate::dialogs::face_sel_on(s, Some(&fa.body.0), fa.point.0).into_iter().collect();
            }
            let e = |x: &solidcraft_engine::doc::Expr| shown_expr(x, ValueKind::Length);
            let o = |x: &Option<solidcraft_engine::doc::Expr>| x.as_ref().map(|x| shown_expr(x, ValueKind::Length)).unwrap_or_default();
            let k = Wd::Gusset {
                chamfered: *shape == solidcraft_engine::doc::frame::GussetShape::Chamfered,
                d1: e(d1),
                d2: if d2 == d1 { String::new() } else { e(d2) },
                thickness: e(thickness),
                chamfer: o(chamfer),
                clip: o(clip),
                offset: o(offset),
            };
            Some((Kind::Weld(k), inputs, Map::new()))
        }
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
        Wd::ExportStep => match pick(0) {
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
    let ext = if matches!(k, Wd::ExportStep) { "step" } else { "dxf" };
    app.pick_save_command(&format!("{body}.{ext}"), &[ext], k.command(), params);
    Ok(())
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
