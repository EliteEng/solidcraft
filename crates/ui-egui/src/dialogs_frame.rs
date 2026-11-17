//! Frame Member (`frame.member`): structural members along the lines of a frame sketch. The lines
//! are picked in the view; the section is a built-in profile with its dimensions, or the profiles
//! picked in another sketch, or a profile of the design's profile library. A placement point, angle
//! and mirror place the section on each line.

use std::collections::BTreeMap;

use egui::{Pos2, Rect, Sense, Stroke, vec2};
use serde_json::{Map, Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::Sel;
use solidcraft_engine::doc::expr::Kind as ValueKind;
use solidcraft_engine::doc::frame::{self, MemberProfile};
use solidcraft_engine::doc::frame_corner::{CornerDefaults, CornerOverride, JunctionKind, Treatment};
use solidcraft_engine::doc::frame_library;
use solidcraft_engine::doc::{FeatureKind, ProfileSel};

use crate::SolveApp;
use crate::dialogs::{Kind, check, curves_sketch, form_row, profile_indices, put, shown_expr};
use crate::selection::{PROFILES, SKETCH_LINES, SelInput, SelectionExt as _};
use crate::ui_kit::{choice_row, expr_row, hint_row, text_row};
use crate::viewport::Proj;

/// The anchors as `frame.member` names them, and their labels.
const ANCHORS: &[(&str, &str)] = &[
    ("origin", "Origin"),
    ("center", "Center"),
    ("centroid", "Centroid"),
    ("top_left", "Top Left"),
    ("top", "Top Middle"),
    ("top_right", "Top Right"),
    ("left", "Left Middle"),
    ("right", "Right Middle"),
    ("bottom_left", "Bottom Left"),
    ("bottom", "Bottom Middle"),
    ("bottom_right", "Bottom Right"),
];

/// The Frame Member dialog's values (the picks are its selection inputs).
#[derive(Clone, Debug)]
pub struct Member {
    /// Every line of the frame sketch, now and as lines are added, not only the lines picked.
    pub all: bool,
    /// The built-in profile (an index into `frame::profile_kinds`), used when no section is picked.
    pub kind: usize,
    /// Each built-in profile's dimensions, its required ones then its optional ones; kept per
    /// profile so switching back finds them again. An empty optional one takes its default.
    pub dims: Vec<Vec<String>>,
    /// A profile sketch whose whole drawn section is the member's (nested loops alternating), as
    /// a feature made that way keeps it while no section is picked.
    pub whole: Option<u64>,
    pub anchor: usize,
    /// A library profile by id, used when no section is picked.
    pub library: Option<String>,
    /// The library folder browsed (its category levels, outermost first); `None` is the built-in
    /// profiles, an empty folder the profiles without a category.
    pub folder: Option<Vec<String>>,
    /// The name Save to Library gives the section; empty names it after the profile.
    pub save_name: String,
    /// The category Save to Library files the section under, levels split by `/`; empty files it
    /// under the profile's kind.
    pub save_category: String,
    /// Save to Library also adds the profile to the personal library, offered in every design.
    pub save_personal: bool,
    /// A placement point by name beyond the anchors (a kind's own points, a library profile's), or
    /// with a library profile any point the member names instead of the profile's default.
    pub point: Option<String>,
    pub angle: String,
    pub mirror: bool,
    pub name: String,
    /// What is done where members meet: an index into [`CORNERS`].
    pub corner: usize,
    /// The weld gap at every junction; empty: none.
    pub gap: String,
    /// Butting members are cut on the face they meet, not square.
    pub flush: bool,
    /// Which pieces run through at a butt: an index into [`BUTT_THROUGH`].
    pub through: usize,
    /// Junctions' own settings (`corner_overrides`), by junction id.
    pub overrides: BTreeMap<String, CornerOverride>,
    /// The junction whose row in the corner list is under the pointer, marked in the view.
    pub list_hover: Option<String>,
    /// The junctions the corner list and the view show, with the path and settings they were
    /// found for (worked out again only when those change).
    pub junctions: Option<(String, Vec<frame::JunctionReport>)>,
    /// Each member's body name by its line id, as the feature being edited pinned them.
    pub pinned: BTreeMap<String, String>,
    /// What the corner list and the view call each line: its member's name where known.
    pub line_names: BTreeMap<String, String>,
    /// The group the rows and the Lines and Section picks edit: an index into `slots`.
    pub group: usize,
    /// Every group (the feature's own fields first), the edited one as it was when the window
    /// switched to it; empty: one group, the window's own.
    pub slots: Vec<Slot>,
    /// Where groups meet: an index into [`CORNERS`] (default Butt).
    pub between: usize,
    /// The weld gap where groups meet; empty: none.
    pub gap_between: String,
    /// The later group runs through where groups meet, not the earlier.
    pub group_later: bool,
}

/// One group of the Frame Member window: its id, its rows' values (in `m`) and its picks.
#[derive(Clone, Debug)]
pub struct Slot {
    pub id: String,
    pub m: Box<Member>,
    pub lines: Vec<Sel>,
    pub section: Vec<Sel>,
}

/// The Corners choices, as `frame.member`'s `corners.treatment` spells them, with their labels.
pub const CORNERS: [(&str, &str); 3] = [("mitre", "ui.frame_corner_mitre"), ("butt", "ui.frame_corner_butt"), ("none", "ui.frame_corner_none")];

/// The Runs through choices, as `corners.butt_through` spells them, with their labels.
pub const BUTT_THROUGH: [(&str, &str); 2] = [("longer", "ui.frame_through_longer"), ("shorter", "ui.frame_through_shorter")];

impl Member {
    fn new() -> Member {
        let dims = frame::profile_kinds()
            .iter()
            .map(|k| {
                let example = serde_json::to_value(&k.example).unwrap_or(Value::Null);
                let given = k.required.iter().map(|d| example.get(*d).and_then(Value::as_f64).map(|v| format!("{v} mm")).unwrap_or_default());
                given.chain(k.optional.iter().map(|_| String::new())).collect()
            })
            .collect();
        // A rectangular tube is the most common member.
        let kind = frame::profile_kinds().iter().position(|k| k.kind == "rect_tube").unwrap_or(0);
        Member {
            all: false,
            kind,
            dims,
            whole: None,
            anchor: 0,
            library: None,
            folder: None,
            save_name: String::new(),
            save_category: String::new(),
            save_personal: false,
            point: None,
            angle: String::new(),
            mirror: false,
            name: String::new(),
            corner: 0,
            gap: String::new(),
            flush: false,
            through: 0,
            overrides: BTreeMap::new(),
            list_hover: None,
            junctions: None,
            pinned: BTreeMap::new(),
            line_names: BTreeMap::new(),
            group: 0,
            slots: Vec::new(),
            between: 1,
            gap_between: String::new(),
            group_later: false,
        }
    }
}

/// The dialog for Frame Member. New members mitre their corners unless the preference keeps
/// square ends.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    let mut m = Member::new();
    if app.preferences.frame_square_ends {
        m.corner = CORNERS.iter().position(|c| c.0 == "none").unwrap_or(0);
    }
    (id == "frame.member")
        .then(|| (Kind::Frame(m), vec![SelInput::new("Lines", SKETCH_LINES, true), SelInput::new("Section (optional)", PROFILES, true)]))
}

fn tr(text: &str) -> &str {
    solidcraft_ui_text::tr_label(text)
}

/// The dialog's own rows (after the selection inputs); true when Enter was pressed in a value.
pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, m: &mut Member, inputs: &mut [SelInput]) -> bool {
    let mut enter = false;
    group_rows(ui, m, inputs);
    let inputs: &[SelInput] = inputs;
    if m.group == 0 {
        form_row(ui, "All Lines", |ui| check(ui, &mut m.all));
    }
    let picked = !crate::selection::items(inputs, 1).is_empty();
    if let Some(sketch) = m.whole.filter(|_| !picked) {
        let name = app.session.doc.feature(sketch).map(|f| f.name.clone()).unwrap_or_default();
        let mut on = true;
        form_row(ui, "Section", |ui| crate::ui_kit::checkbox(ui, &name, &mut on));
        if !on {
            m.whole = None;
        }
    } else if !picked {
        library_rows(app, ui, m);
        if m.library.is_none() {
            let kinds = frame::profile_kinds();
            let labels: Vec<&str> = kinds.iter().map(|k| k.label).collect();
            if choice_row(ui, "Profile", "frame_profile", &mut m.kind, &labels).changed() {
                m.point = None;
            }
            if let (Some(k), Some(values)) = (kinds.get(m.kind), m.dims.get_mut(m.kind)) {
                for (i, (name, v)) in k.required.iter().chain(k.optional).zip(values.iter_mut()).enumerate() {
                    let label = frame::dimension_label(name);
                    enter |= if i < k.required.len() { expr_row(ui, label, v) } else { hint_row(ui, label, v, tr("default")) };
                }
            }
        }
    }
    if m.library.is_none() {
        let (name, category) = save_defaults(app, m, inputs);
        enter |= text_row(ui, "ui.frame_save_name", &mut m.save_name, &name).0;
        enter |= text_row(ui, "ui.frame_category", &mut m.save_category, &category).0;
        form_row(ui, "ui.frame_save_personal", |ui| check(ui, &mut m.save_personal));
        if crate::ui_kit::button(ui, solidcraft_ui_text::tr("ui.frame_save_to_library"), crate::ui_kit::Button::Normal).clicked() {
            match save_to_library(app, m, inputs) {
                Ok(id) => {
                    if m.save_personal
                        && let Err(e) = app.run("frame.personal.add", json!({"id": id, "replace": true}))
                    {
                        app.set_status(e, true);
                    }
                    m.folder = app.session.doc.library_profile(&id).map(|e| e.category.clone());
                    m.library = Some(id);
                    m.whole = None;
                    m.save_name.clear();
                    m.save_category.clear();
                }
                Err(e) => app.set_status(e, true),
            }
        }
    }
    point_row(app, ui, m, picked || m.whole.is_some());
    enter |= hint_row(ui, "Angle", &mut m.angle, "0 deg");
    form_row(ui, "Mirror", |ui| check(ui, &mut m.mirror));
    let labels: Vec<&str> = CORNERS.iter().map(|c| tr(c.1)).collect();
    choice_row(ui, "ui.frame_corners", "frame_corners", &mut m.corner, &labels);
    if CORNERS.get(m.corner).is_some_and(|c| c.0 != "none") {
        enter |= hint_row(ui, "ui.frame_weld_gap", &mut m.gap, "0 mm");
        form_row(ui, "ui.frame_butt_flush", |ui| check(ui, &mut m.flush));
    }
    if CORNERS.get(m.corner).is_some_and(|c| c.0 == "butt") {
        let labels: Vec<&str> = BUTT_THROUGH.iter().map(|c| tr(c.1)).collect();
        choice_row(ui, "ui.frame_butt_through", "frame_butt_through", &mut m.through, &labels);
    }
    if m.slots.len() > 1 {
        let labels: Vec<&str> = CORNERS.iter().map(|c| tr(c.1)).collect();
        choice_row(ui, "ui.frame_between_groups", "frame_between_groups", &mut m.between, &labels);
        if CORNERS.get(m.between).is_some_and(|c| c.0 != "none") {
            enter |= hint_row(ui, "ui.frame_gap_between", &mut m.gap_between, "0 mm");
            let labels = [tr("ui.frame_group_earlier"), tr("ui.frame_group_later")];
            let mut later = usize::from(m.group_later);
            choice_row(ui, "ui.frame_group_through", "frame_group_through", &mut later, &labels);
            m.group_later = later == 1;
        }
    }
    refresh_junctions(app, m, inputs);
    corner_list(ui, m);
    enter |= text_row(ui, "Name", &mut m.name, tr("from the profile")).0;
    enter
}

/// The profiles the window offers: the design's library, then the personal library's profiles
/// whose names the design's doesn't have (the design's own wins).
fn offered(app: &SolveApp) -> Vec<&frame_library::LibraryProfile> {
    let design = &app.session.doc.frame_profiles;
    let personal = app.preferences.frame_profiles.0.iter().filter(|p| !design.iter().any(|e| e.name.eq_ignore_ascii_case(&p.name)));
    design.iter().chain(personal).collect()
}

/// The profile the window's `library` id names: the design's, or a personal one.
fn library_entry<'a>(app: &'a SolveApp, id: &str) -> Option<&'a frame_library::LibraryProfile> {
    app.session.doc.library_profile(id).or_else(|| app.preferences.frame_profiles.get(id))
}

/// The library's rows: Type (Built-in or a top-level category), a Group row for each level of
/// subcategories, and Size (the profiles in the chosen folder and below). Choosing a folder picks
/// its first profile, so a library profile is always chosen unless Type is Built-in. Personal
/// profiles are marked; OK copies the one chosen into the design's library.
fn library_rows(app: &SolveApp, ui: &mut egui::Ui, m: &mut Member) {
    let library = offered(app);
    if library.is_empty() {
        (m.library, m.folder) = (None, None);
        return;
    }
    // A member edited or a profile just saved names its profile: open the profile's folder.
    if m.folder.is_none()
        && let Some(e) = m.library.as_ref().and_then(|id| library.iter().find(|e| &e.id == id))
    {
        m.folder = Some(e.category.clone());
    }
    let tops = subfolders(&library, &[]);
    let uncategorised = library.iter().any(|e| e.category.is_empty());
    let mut labels: Vec<&str> = vec![tr("Built-in")];
    labels.extend(tops.iter().map(String::as_str));
    if uncategorised {
        labels.push(tr("ui.frame_uncategorised"));
    }
    let mut index = match &m.folder {
        None => 0,
        Some(f) => match f.first() {
            Some(top) => tops.iter().position(|t| t == top).map_or(0, |i| i + 1),
            None if uncategorised => tops.len() + 1,
            None => 0,
        },
    };
    let before = index;
    choice_row(ui, "Type", "frame_library", &mut index, &labels);
    if index != before {
        m.folder = index.checked_sub(1).map(|i| tops.get(i).map(|t| vec![t.clone()]).unwrap_or_default());
    }
    let Some(mut folder) = m.folder.clone() else {
        if m.library.take().is_some() {
            (m.point, m.anchor) = (None, 0);
        }
        return;
    };
    for level in 1..frame_library::MAX_CATEGORY_LEVELS {
        if folder.len() < level {
            break;
        }
        let subs = subfolders(&library, &folder[..level]);
        if subs.is_empty() {
            break;
        }
        let mut labels: Vec<&str> = vec![tr("All")];
        labels.extend(subs.iter().map(String::as_str));
        let mut index = folder.get(level).and_then(|g| subs.iter().position(|s| s == g)).map_or(0, |i| i + 1);
        let before = index;
        choice_row(ui, "ui.frame_group", &format!("frame_library_group_{level}"), &mut index, &labels);
        if index != before {
            folder.truncate(level);
            if let Some(g) = index.checked_sub(1).and_then(|i| subs.get(i)) {
                folder.push(g.clone());
            }
        }
    }
    let entries = profiles_in(&library, &folder);
    let Some(first) = entries.first() else {
        // The folder is gone (its profiles were removed or moved): back to the built-in profiles.
        (m.library, m.folder) = (None, None);
        return;
    };
    let shown: Vec<String> = entries
        .iter()
        .map(|e| {
            if e.id.starts_with(crate::frame_personal::ID_PREFIX) {
                solidcraft_ui_text::format("ui.frame_personal_profile", &[("name", &e.name)])
            } else {
                e.name.clone()
            }
        })
        .collect();
    let labels: Vec<&str> = shown.iter().map(String::as_str).collect();
    let mut index = m.library.as_ref().and_then(|id| entries.iter().position(|e| &e.id == id)).unwrap_or(0);
    choice_row(ui, "Size", "frame_library_size", &mut index, &labels);
    let chosen = entries.get(index).unwrap_or(first).id.clone();
    if m.library.as_ref() != Some(&chosen) {
        // Another profile has other points: start from its default again.
        m.point = None;
        m.anchor = 0;
        m.library = Some(chosen);
    }
    m.folder = Some(folder);
}

/// The folders one level below `folder`, in natural order (`50x25` before `100x50`).
fn subfolders(library: &[&frame_library::LibraryProfile], folder: &[String]) -> Vec<String> {
    let mut subs: Vec<String> =
        library.iter().filter(|e| e.category.starts_with(folder)).filter_map(|e| e.category.get(folder.len()).cloned()).collect();
    subs.sort_by(|a, b| natural(a, b));
    subs.dedup();
    subs
}

/// The profiles in `folder` and its subfolders, in natural order by name; an empty folder holds the
/// profiles without a category.
fn profiles_in<'a>(library: &[&'a frame_library::LibraryProfile], folder: &[String]) -> Vec<&'a frame_library::LibraryProfile> {
    let mut entries: Vec<_> =
        library.iter().copied().filter(|e| if folder.is_empty() { e.category.is_empty() } else { e.category.starts_with(folder) }).collect();
    entries.sort_by(|a, b| natural(&a.name, &b.name));
    entries
}

/// Natural order: numbers compare by value (`63.5x38` after `50x25`, before `100x50`), the rest
/// ignoring case.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    fn tokens(s: &str) -> Vec<Result<f64, char>> {
        let mut out = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while let Some(&c) = chars.get(i) {
            if c.is_ascii_digit() {
                let start = i;
                while chars.get(i).is_some_and(|c| c.is_ascii_digit())
                    || (chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) && !chars[start..i].contains(&'.'))
                {
                    i += 1;
                }
                let number: String = chars[start..i].iter().collect();
                out.push(number.parse::<f64>().map_err(|_| c));
            } else {
                out.extend(c.to_lowercase().map(Err));
                i += 1;
            }
        }
        out
    }
    let order = |x: &Result<f64, char>, y: &Result<f64, char>| match (x, y) {
        (Ok(x), Ok(y)) => x.total_cmp(y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(x), Err(y)) => x.cmp(y),
    };
    let (ta, tb) = (tokens(a), tokens(b));
    ta.iter().zip(&tb).map(|(x, y)| order(x, y)).find(|o| o.is_ne()).unwrap_or_else(|| ta.len().cmp(&tb.len())).then_with(|| a.cmp(b))
}

/// What Save to Library names and files the section as when its name and category are empty: the
/// built-in profile's description (`RHS 100 × 50 × 3`) under its kind, or the section sketch's name
/// under Custom. The name gets ` (2)`… when the library has it already.
fn save_defaults(app: &SolveApp, m: &Member, inputs: &[SelInput]) -> (String, String) {
    let doc = &app.session.doc;
    let section = crate::selection::items(inputs, 1).profiles();
    let (base, category) = match section.first().map(|x| x.0).or(m.whole) {
        Some(sketch) => (doc.feature(sketch).map(|f| f.name.clone()).unwrap_or_else(|| "Profile".into()), "Custom".to_string()),
        None => {
            let category = frame::profile_kinds().get(m.kind).map(|k| k.label.to_string()).unwrap_or_default();
            let name = builtin_profile(m)
                .ok()
                .and_then(|p| serde_json::from_value::<frame::ProfileSpec>(p).ok())
                .and_then(|spec| spec.resolve(|_, e, kind| doc.eval(&doc.with_design_unit(e, kind), kind)).ok())
                .map(|p| p.description())
                .unwrap_or_else(|| category.clone());
            (name, category)
        }
    };
    let taken = |n: &str| doc.frame_profiles.iter().any(|e| e.name.eq_ignore_ascii_case(n));
    let name = std::iter::once(base.clone()).chain((2..1000).map(|i| format!("{base} ({i})"))).find(|n| !taken(n)).unwrap_or(base);
    (name, category)
}

/// Adds the dialog's section to the design's profile library (`frame.library.add`): the built-in
/// profile with its dimensions, or the section sketch (its picked profiles), named and filed as
/// Save As and Category say (`Steel/RHS` is two levels), else as `save_defaults` does. Returns the
/// new profile's id.
fn save_to_library(app: &mut SolveApp, m: &Member, inputs: &[SelInput]) -> Result<String, String> {
    let section = crate::selection::items(inputs, 1).profiles();
    let mut p = match section.first().map(|x| x.0).or(m.whole) {
        Some(sketch) => {
            let mut p = json!({"profile_sketch": sketch});
            if !section.is_empty() {
                p["profiles"] = json!(section.iter().map(|x| x.1).collect::<Vec<_>>());
            }
            p
        }
        None => json!({"profile": builtin_profile(m)?}),
    };
    let (name, category) = save_defaults(app, m, inputs);
    let name = Some(m.save_name.trim()).filter(|n| !n.is_empty()).map_or(name, str::to_string);
    let category = Some(m.save_category.trim()).filter(|c| !c.is_empty()).map_or(category, str::to_string);
    p["name"] = json!(name);
    p["category"] = json!(category.split('/').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>());
    let out = app.run("frame.library.add", p)?;
    out["profile"]["id"].as_str().map(str::to_string).ok_or_else(|| "the library profile has no id".to_string())
}

/// The built-in profile the dialog's values describe, as `frame.member` takes it.
fn builtin_profile(m: &Member) -> Result<Value, String> {
    let kinds = frame::profile_kinds();
    let (k, values) = kinds.get(m.kind).zip(m.dims.get(m.kind)).ok_or("choose a profile")?;
    let mut profile = json!({"kind": k.kind});
    for (name, v) in k.required.iter().chain(k.optional).zip(values) {
        if !v.trim().is_empty() {
            profile[*name] = json!(v);
        }
    }
    Ok(profile)
}

/// The placement points the dialog offers, by name: the anchors, then the points the profile's kind
/// generates, then the points a library profile names. A sketch section has the anchors only.
fn point_names(app: &SolveApp, m: &Member, sketch_section: bool) -> Vec<String> {
    let mut names: Vec<String> = ANCHORS.iter().map(|a| a.0.to_string()).collect();
    if sketch_section {
        return names;
    }
    let kinds = frame::profile_kinds();
    let (kind, named) = match m.library.as_ref().and_then(|id| library_entry(app, id)) {
        Some(e) => {
            let kind = match &e.section {
                frame_library::LibrarySection::BuiltIn { profile } => Some(profile.kind.as_str()),
                frame_library::LibrarySection::Drawn { .. } => None,
            };
            (kind, e.points.iter().map(|p| p.name.clone()).collect())
        }
        None => (kinds.get(m.kind).map(|k| k.kind), Vec::new()),
    };
    if let Some(example) = kind.and_then(|kind| kinds.iter().find(|k| k.kind == kind)).map(|k| &k.example) {
        names.extend(frame_library::generated_points(example).into_iter().map(|(n, _)| n.to_string()));
    }
    names.extend(named);
    names
}

/// A placement point's label: an anchor's own, else its name with spaces (`heel_inside` → `Heel
/// inside`).
fn point_label(name: &str) -> String {
    if let Some((_, label)) = ANCHORS.iter().find(|a| a.0 == name) {
        return tr(label).to_string();
    }
    let spaced = name.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The Point row: which placement point sits on each line. With a library profile the first
/// choice is its default point; otherwise it is the origin.
fn point_row(app: &SolveApp, ui: &mut egui::Ui, m: &mut Member, sketch_section: bool) {
    let names = point_names(app, m, sketch_section);
    let library = m.library.as_ref().and_then(|id| library_entry(app, id));
    let mut labels: Vec<String> = Vec::new();
    if let Some(e) = library {
        let default = e.default_point.as_deref().unwrap_or("origin");
        labels.push(format!("{} ({})", tr("Default"), point_label(default)));
    }
    let offset = labels.len();
    labels.extend(names.iter().map(|n| point_label(n)));
    let mut index = match &m.point {
        Some(n) => names.iter().position(|x| x == n).map_or(0, |i| i + offset),
        None if offset > 0 => 0,
        None => m.anchor,
    };
    let shown: Vec<&str> = labels.iter().map(String::as_str).collect();
    choice_row(ui, "Point", "frame_point", &mut index, &shown);
    let picked = index.checked_sub(offset).and_then(|i| names.get(i));
    match picked {
        // With a library profile, any point it names wins over its default.
        Some(name) if library.is_some() => (m.anchor, m.point) = (0, Some(name.clone())),
        Some(name) => match ANCHORS.iter().position(|a| a.0 == name) {
            Some(anchor) => (m.anchor, m.point) = (anchor, None),
            None => (m.anchor, m.point) = (0, Some(name.clone())),
        },
        None => (m.anchor, m.point) = (0, None),
    }
}

/// The command the dialog's OK runs (`extra`: parameters kept from the feature being edited).
pub fn commands(app: &SolveApp, m: &Member, inputs: &[SelInput], extra: &Map<String, Value>) -> Result<Vec<(String, Value)>, String> {
    let slots = all_slots(m, inputs);
    let Some(first) = slots.first() else { return Err("no group".into()) };
    let (_, section_sketch) = section_of(&first.section, first.m.whole)?;
    let (sketch, lines) = path(app, &first.lines, section_sketch)?;
    let mut p = group_params(app, &first.m, &first.section, "body_name")?;
    p["sketch"] = json!(sketch);
    if !m.all {
        p["lines"] = json!(lines);
    }
    let mut groups = Vec::new();
    for (k, g) in slots.iter().enumerate().skip(1) {
        let label = |e: String| format!("{} {}: {e}", tr("ui.frame_member_group"), k + 1);
        let (_, section_sketch) = section_of(&g.section, g.m.whole).map_err(label)?;
        let (their, lines) = path(app, &g.lines, section_sketch).map_err(label)?;
        if their != sketch {
            return Err(label("its lines must be in the first group's sketch".into()));
        }
        let mut q = group_params(app, &g.m, &g.section, "body_name").map_err(label)?;
        q["id"] = json!(g.id);
        q["lines"] = json!(lines);
        groups.push(q);
    }
    // Always sent, so an edit that removes the last further group clears them.
    p["groups"] = json!(groups);
    p["corners"] = corners(m);
    p["corner_overrides"] = json!(m.overrides);
    if let Value::Object(o) = &mut p {
        for (key, v) in extra {
            o.entry(key.clone()).or_insert_with(|| v.clone());
        }
    }
    Ok(vec![("frame.member".to_string(), p)])
}

/// The frame sketch and the lines picked in it.
fn path(app: &SolveApp, picks: &[Sel], section_sketch: Option<u64>) -> Result<(u64, Vec<String>), String> {
    // A line picked in the view comes with its sketch (line ids are a sketch's own); a line
    // selected before the dialog opened is looked for in the newest sketch with it, not the
    // section's.
    let mut lines = Vec::new();
    let mut known = None;
    for pick in picks {
        match pick {
            Sel::ExternalSketchCurve { sketch, id } | Sel::SketchCurve { id, sketch: Some(sketch) } => {
                if known.is_some_and(|k| k != *sketch) {
                    return Err("the lines must be in one sketch".into());
                }
                known = Some(*sketch);
                lines.push(id.clone());
            }
            Sel::SketchCurve { id, sketch: None } => lines.push(id.clone()),
            _ => {}
        }
    }
    if lines.is_empty() {
        return Err("select the frame sketch's lines first".into());
    }
    let sketch = known.or_else(|| curves_sketch(app, &lines, section_sketch)).ok_or("the lines must be in one sketch, not the section's")?;
    Ok((sketch, lines))
}

/// `frame.member`'s `corners` for the Corners rows.
fn corners(m: &Member) -> Value {
    let treatment = CORNERS.get(m.corner).map_or("mitre", |c| c.0);
    let mut c = json!({ "treatment": treatment });
    if treatment != "none" {
        if !m.gap.trim().is_empty() {
            c["gap"] = json!(m.gap.trim());
        }
        if m.flush {
            c["butt_cut"] = json!("flush");
        }
        if let Some((through, _)) = BUTT_THROUGH.get(m.through).filter(|t| t.0 != "longer") {
            c["butt_through"] = json!(through);
        }
    }
    if m.slots.len() > 1 {
        let between = CORNERS.get(m.between).map_or("butt", |c| c.0);
        c["between_groups"] = json!(between);
        if between != "none" {
            if !m.gap_between.trim().is_empty() {
                c["gap_between"] = json!(m.gap_between.trim());
            }
            if m.group_later {
                c["group_through"] = json!("later");
            }
        }
    }
    c
}

/// The dialog editing a frame member feature, filled from it.
pub fn for_feature(app: &SolveApp, kind: &FeatureKind) -> Option<(Kind, Vec<SelInput>, Map<String, Value>)> {
    let FeatureKind::FrameMember {
        path_sketch,
        lines,
        profile,
        anchor,
        locate,
        point,
        angle,
        mirror,
        body_name,
        names,
        corners,
        corner_overrides,
        groups,
        ..
    } = kind
    else {
        return None;
    };
    let (_, mut inputs) = start(app, "frame.member")?;
    let st = app.session.model.state();
    let mut m = Member::new();
    m.all = lines.is_empty();
    // With All Lines, the first group's lines are the ones no other group has.
    let ids = if m.all {
        frame::line_groups_on(&st, path_sketch.0, &[], groups)
            .map(|(ids, of)| ids.into_iter().zip(of).filter(|(_, g)| *g == 0).map(|(id, _)| id).collect())
            .unwrap_or_default()
    } else {
        lines.clone()
    };
    let curves = |ids: &[String]| ids.iter().map(|id| Sel::ExternalSketchCurve { sketch: path_sketch.0, id: id.clone() }).collect::<Vec<_>>();
    let mut section = Vec::new();
    let own = GroupValues { profile, anchor: *anchor, point, angle, mirror: *mirror, name: body_name };
    load_group(&st, &mut m, &mut section, &own);
    if let Some(i) = inputs.get_mut(0) {
        i.items = curves(&ids);
    }
    if let Some(i) = inputs.get_mut(1) {
        i.items = section.clone();
    }
    if !groups.is_empty() {
        let mut first = Member::new();
        take_group(&mut first, &m);
        m.slots.push(Slot { id: "g1".into(), m: Box::new(first), lines: curves(&ids), section });
        for g in groups {
            let mut gm = Member::new();
            let mut section = Vec::new();
            let values = GroupValues { profile: &g.profile, anchor: g.anchor, point: &g.point, angle: &g.angle, mirror: g.mirror, name: &g.name };
            load_group(&st, &mut gm, &mut section, &values);
            m.slots.push(Slot { id: g.id.clone(), m: Box::new(gm), lines: curves(&g.lines), section });
        }
    }
    let treatment = serde_json::to_value(corners.within_group).ok();
    m.corner = CORNERS.iter().position(|c| treatment.as_ref().and_then(Value::as_str) == Some(c.0)).unwrap_or(0);
    m.gap = corners.gap_within.as_deref().map(|g| shown_expr(g, ValueKind::Length)).unwrap_or_default();
    m.flush = !corners.butt_cut.is_square();
    let through = serde_json::to_value(corners.butt_through).ok();
    m.through = BUTT_THROUGH.iter().position(|c| through.as_ref().and_then(Value::as_str) == Some(c.0)).unwrap_or(0);
    let between = serde_json::to_value(corners.between_groups).ok();
    m.between = CORNERS.iter().position(|c| between.as_ref().and_then(Value::as_str) == Some(c.0)).unwrap_or(1);
    m.gap_between = corners.gap_between.as_deref().map(|g| shown_expr(g, ValueKind::Length)).unwrap_or_default();
    m.group_later = !corners.group_through.is_earlier();
    m.overrides = corner_overrides.clone();
    m.pinned = names.clone();
    // The dialog has no field for a located point: an edit keeps it, so the member stays where it
    // is attached.
    let mut extra = Map::new();
    if let Some(at) = locate {
        extra.insert("locate".into(), json!(at));
    }
    Some((Kind::Frame(m), inputs, extra))
}

/// One group's saved settings: the feature's own fields or a `MemberGroup`'s.
struct GroupValues<'a> {
    profile: &'a MemberProfile,
    anchor: frame::Anchor,
    point: &'a Option<String>,
    angle: &'a Option<solidcraft_engine::doc::Expr>,
    mirror: bool,
    name: &'a Option<String>,
}

/// Fill a group's rows (and its section picks) from its saved settings.
fn load_group(st: &solidcraft_engine::doc::ModelState, m: &mut Member, section: &mut Vec<Sel>, g: &GroupValues) {
    match g.profile {
        MemberProfile::Library { profile } => {
            let kinds = frame::profile_kinds();
            if let Some((index, k)) = kinds.iter().enumerate().find(|(_, k)| k.kind == profile.kind)
                && let Some(values) = m.dims.get_mut(index)
            {
                m.kind = index;
                for (name, v) in k.required.iter().chain(k.optional).zip(values.iter_mut()) {
                    *v = profile.dims.get(*name).map(|e| shown_expr(e, frame::dimension_kind(name))).unwrap_or_default();
                }
            }
        }
        MemberProfile::LibraryProfile { id } => m.library = Some(id.clone()),
        MemberProfile::Sketch { sketch, profiles: ProfileSel::All } => m.whole = Some(sketch.0),
        MemberProfile::Sketch { sketch, profiles } => {
            let picks = st.sketch(sketch.0).map(|ss| profile_indices(ss, profiles)).unwrap_or_default();
            *section = picks.into_iter().map(|index| Sel::Profile { sketch: sketch.0, index }).collect();
        }
    }
    let anchor = serde_json::to_value(g.anchor).ok();
    m.anchor = ANCHORS.iter().position(|(a, _)| anchor.as_ref().and_then(Value::as_str) == Some(*a)).unwrap_or(0);
    m.point = g.point.clone();
    m.angle = g.angle.as_deref().map(|a| shown_expr(a, ValueKind::Angle)).unwrap_or_default();
    m.mirror = g.mirror;
    m.name = g.name.clone().unwrap_or_default();
}

/// The Group row: which group the rows edit, New group and Remove group.
fn group_rows(ui: &mut egui::Ui, m: &mut Member, inputs: &mut [SelInput]) {
    let count = m.slots.len().max(1);
    let mut labels: Vec<String> = (1..=count).map(|n| format!("{} {n}", tr("ui.frame_member_group"))).collect();
    if count < frame::MAX_GROUPS {
        labels.push(tr("ui.frame_new_group").to_string());
    }
    let shown: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut chosen = m.group;
    if choice_row(ui, "ui.frame_member_group", "frame_member_group", &mut chosen, &shown).changed() {
        if chosen >= count {
            new_group(m, inputs);
        } else {
            switch_group(m, inputs, chosen);
        }
    }
    if m.group > 0 && crate::ui_kit::row(ui, "", |ui| crate::ui_kit::button(ui, tr("ui.frame_remove_group"), crate::ui_kit::Button::Small).clicked())
    {
        remove_group(m, inputs);
    }
}

/// Copy the rows that belong to a group (the profile, its placement and name) from `src`.
fn take_group(dst: &mut Member, src: &Member) {
    dst.kind = src.kind;
    dst.dims = src.dims.clone();
    dst.whole = src.whole;
    dst.anchor = src.anchor;
    dst.library = src.library.clone();
    dst.folder = src.folder.clone();
    dst.point = src.point.clone();
    dst.angle = src.angle.clone();
    dst.mirror = src.mirror;
    dst.name = src.name.clone();
}

/// The group the window edits now, as a slot.
fn live(m: &Member, inputs: &[SelInput]) -> Slot {
    let mut copy = Member::new();
    take_group(&mut copy, m);
    let id = m.slots.get(m.group).map_or_else(|| "g1".to_string(), |s| s.id.clone());
    Slot { id, m: Box::new(copy), lines: crate::selection::items(inputs, 0).to_vec(), section: crate::selection::items(inputs, 1).to_vec() }
}

/// Every group, the edited one as the window shows it now.
fn all_slots(m: &Member, inputs: &[SelInput]) -> Vec<Slot> {
    let mut out = if m.slots.is_empty() { vec![live(m, inputs)] } else { m.slots.clone() };
    if let Some(slot) = out.get_mut(m.group) {
        *slot = live(m, inputs);
    }
    out
}

/// Put slot `next`'s rows and picks in the window.
fn show_slot(m: &mut Member, inputs: &mut [SelInput], next: Slot) {
    take_group(m, &next.m);
    if let Some(i) = inputs.get_mut(0) {
        i.items = next.lines;
    }
    if let Some(i) = inputs.get_mut(1) {
        i.items = next.section;
    }
}

/// Show group `to` in the rows and the picks, keeping the one shown.
fn switch_group(m: &mut Member, inputs: &mut [SelInput], to: usize) {
    let slots = all_slots(m, inputs);
    let Some(next) = slots.get(to).cloned() else { return };
    m.slots = slots;
    show_slot(m, inputs, next);
    m.group = to;
}

/// Add a group (with the shown group's profile, no lines) and show it.
fn new_group(m: &mut Member, inputs: &mut [SelInput]) {
    let mut slots = all_slots(m, inputs);
    if slots.len() >= frame::MAX_GROUPS {
        return;
    }
    let id = (2..).map(|n| format!("g{n}")).find(|id| !slots.iter().any(|s| s.id == *id)).unwrap_or_default();
    let mut copy = Member::new();
    take_group(&mut copy, m);
    copy.name.clear();
    slots.push(Slot { id, m: Box::new(copy), lines: Vec::new(), section: crate::selection::items(inputs, 1).to_vec() });
    m.slots = slots;
    let to = m.slots.len() - 1;
    switch_group(m, inputs, to);
}

/// Drop the shown group (not the first) and show the one before it.
fn remove_group(m: &mut Member, inputs: &mut [SelInput]) {
    if m.group == 0 || m.group >= m.slots.len() {
        return;
    }
    m.slots.remove(m.group);
    let to = m.group - 1;
    let Some(next) = m.slots.get(to).cloned() else { return };
    show_slot(m, inputs, next);
    m.group = to;
    if m.slots.len() == 1 {
        m.slots.clear();
    }
}

/// The picks' profiles, all in one sketch.
fn section_of(section: &[Sel], whole: Option<u64>) -> Result<(Vec<(u64, usize)>, Option<u64>), String> {
    let profiles = section.profiles();
    let sketch = profiles.first().map(|x| x.0).or(whole);
    if profiles.iter().any(|x| Some(x.0) != sketch) {
        return Err("the section's profiles must be in one sketch".into());
    }
    Ok((profiles, sketch))
}

/// A group's keys for `frame.member` (or a `groups` entry): its profile, placement and name.
fn group_params(app: &SolveApp, g: &Member, section: &[Sel], name_key: &str) -> Result<Value, String> {
    let (profiles, section_sketch) = section_of(section, g.whole)?;
    let mut p = json!({});
    if let Some(ps) = section_sketch {
        p["profile_sketch"] = json!(ps);
        if !profiles.is_empty() {
            p["profiles"] = json!(profiles.iter().map(|x| x.1).collect::<Vec<_>>());
        }
    } else if let Some(id) = &g.library {
        // A personal profile goes whole: the member copies it into the design's library.
        p["library"] = match app.preferences.frame_profiles.get(id) {
            Some(e) if app.session.doc.library_profile(id).is_none() => crate::frame_personal::member_library(e),
            _ => json!(id),
        };
    } else {
        p["profile"] = builtin_profile(g)?;
    }
    if g.anchor != 0
        && let Some((anchor, _)) = ANCHORS.get(g.anchor)
    {
        p["anchor"] = json!(anchor);
    }
    if let Some(point) = &g.point {
        p["point"] = json!(point);
    }
    put(&mut p, &[("angle", &g.angle), (name_key, g.name.trim())]);
    if g.mirror {
        p["mirror"] = json!(true);
    }
    Ok(p)
}

/// The feature's corner settings as the Corners rows set them.
fn corner_defaults(m: &Member) -> CornerDefaults {
    let c = corners(m);
    let doc = json!({
        "within_group": c.get("treatment").cloned().unwrap_or(Value::Null),
        "gap_within": c.get("gap").cloned().unwrap_or(Value::Null),
        "butt_cut": c.get("butt_cut").cloned().unwrap_or(json!("square")),
        "butt_through": c.get("butt_through").cloned().unwrap_or(json!("longer")),
        "between_groups": c.get("between_groups").cloned().unwrap_or(json!("butt")),
        "gap_between": c.get("gap_between").cloned().unwrap_or(Value::Null),
        "group_through": c.get("group_through").cloned().unwrap_or(json!("earlier")),
    });
    serde_json::from_value(doc).unwrap_or_default()
}

/// Work out the junctions of the lines picked again when the path or the corner settings changed.
pub fn refresh_junctions(app: &SolveApp, m: &mut Member, inputs: &[SelInput]) {
    let slots = all_slots(m, inputs);
    let Some(first) = slots.first() else { return };
    let section_sketch = first.section.profiles().first().map(|x| x.0).or(first.m.whole);
    let Ok((sketch, lines)) = path(app, &first.lines, section_sketch) else {
        m.junctions = None;
        return;
    };
    let lines = if m.all { Vec::new() } else { lines };
    // Only the further groups' lines matter here, not their sections.
    let groups: Vec<frame::MemberGroup> = slots
        .iter()
        .skip(1)
        .filter_map(|g| path(app, &g.lines, None).ok().filter(|(s, _)| *s == sketch))
        .enumerate()
        .map(|(k, (_, lines))| frame::MemberGroup {
            id: format!("g{}", k + 2),
            name: None,
            lines,
            profile: MemberProfile::LibraryProfile { id: String::new() },
            anchor: frame::Anchor::default(),
            point: None,
            angle: None,
            mirror: false,
        })
        .collect();
    let defaults = corner_defaults(m);
    let group_lines: Vec<&Vec<String>> = groups.iter().map(|g| &g.lines).collect();
    let key = json!([sketch, lines, group_lines, defaults, m.overrides, m.name.trim()]).to_string();
    if m.junctions.as_ref().is_some_and(|(k, _)| *k == key) {
        return;
    }
    let st = app.session.model.state();
    let component = app.session.doc.feature(sketch).map_or(0, |f| f.component);
    let found = frame::junctions_on(&app.session.doc, &st, sketch, (&lines, &groups), &defaults, &m.overrides, component).unwrap_or_default();
    m.junctions = Some((key, found));
    // Members are named as the feature names them: an edited feature's pinned names; a new one's
    // after Name (without one, after the profile, which the list doesn't know yet).
    m.line_names = if !m.pinned.is_empty() {
        m.pinned.clone()
    } else if !m.name.trim().is_empty() && groups.is_empty() {
        let ids = frame::line_ids_on(&st, sketch, &lines, &groups).unwrap_or_default();
        ids.iter().cloned().zip(frame::member_names(&st, &ids, &BTreeMap::new(), m.name.trim())).collect()
    } else {
        BTreeMap::new()
    };
}

/// What the corner list and the view call line `id`: its member's name, else the line.
fn line_label(m: &Member, id: &str) -> String {
    match m.line_names.get(id) {
        Some(name) => name.clone(),
        None => format!("{} {id}", tr("ui.frame_line")),
    }
}

/// How a junction's kind reads in the corner list.
fn kind_label(kind: JunctionKind) -> &'static str {
    match kind {
        JunctionKind::L => "L",
        JunctionKind::I => "I",
        JunctionKind::T => "T",
        JunctionKind::Multi => "Multi",
    }
}

/// Junction `j`'s own setting, made by `change` and dropped again when it sets nothing.
fn set_override(m: &mut Member, id: &str, change: impl FnOnce(&mut CornerOverride)) {
    let o = m.overrides.entry(id.to_string()).or_default();
    change(o);
    if *o == CornerOverride::default() {
        m.overrides.remove(id);
    }
}

/// Run the next member at junction `j` through: the other one at an L, the next in turn where
/// more meet.
fn swap_through(m: &mut Member, j: &frame::JunctionReport) {
    let mut lines: Vec<&str> = Vec::new();
    for (line, _) in &j.ends {
        if !lines.contains(&line.as_str()) {
            lines.push(line);
        }
    }
    let Some(now) = j.through.as_deref() else { return };
    let Some(k) = lines.iter().position(|l| *l == now) else { return };
    let Some(next) = lines.get((k + 1) % lines.len()).map(|l| l.to_string()) else { return };
    set_override(m, &j.id, |o| o.through = Some(next));
}

/// Frame Member's corner list: each junction of the lines picked, its treatment and the member
/// that runs through, each changeable for that junction alone (`corner_overrides`).
fn corner_list(ui: &mut egui::Ui, m: &mut Member) {
    m.list_hover = None;
    let list = m.junctions.as_ref().map(|(_, j)| j.clone()).unwrap_or_default();
    if list.is_empty() {
        return;
    }
    let mut hover = None;
    crate::ui_kit::section_count(ui, "frame_corner_list", "ui.frame_corner_list", false, Some(list.len()), |ui| {
        crate::ui_kit::status_line(ui, solidcraft_ui_text::tr("ui.frame_corner_list.hint"), crate::ui_kit::StatusTone::Dim);
        for j in &list {
            let r = ui.scope(|ui| corner_row(ui, m, j)).response;
            if r.contains_pointer() {
                hover = Some(j.id.clone());
            }
        }
    });
    m.list_hover = hover;
}

fn corner_row(ui: &mut egui::Ui, m: &mut Member, j: &frame::JunctionReport) {
    let mut options: Vec<(&str, Option<Treatment>)> = vec![("ui.frame_corner_default", None)];
    if j.kind == JunctionKind::L {
        options.push(("ui.frame_corner_mitre", Some(Treatment::Mitre)));
    }
    options.push(("ui.frame_corner_butt", Some(Treatment::Butt)));
    options.push(("ui.frame_corner_none", Some(Treatment::None)));
    let current = m.overrides.get(&j.id).and_then(|o| o.treatment);
    let mut chosen = options.iter().position(|o| o.1 == current).unwrap_or(0);
    let labels: Vec<&str> = options.iter().map(|o| tr(o.0)).collect();
    let title = format!("{} · {}", j.id, kind_label(j.kind));
    if choice_row(ui, &title, &format!("frame_corner_{}", j.id), &mut chosen, &labels).changed() {
        let treatment = options.get(chosen).and_then(|o| o.1);
        set_override(m, &j.id, |o| o.treatment = treatment);
    }
    let overridden = m.overrides.contains_key(&j.id);
    if j.through.is_some() || overridden {
        crate::ui_kit::row(ui, "", |ui| {
            ui.horizontal(|ui| {
                if let Some(line) = &j.through {
                    ui.label(format!("{} {}", tr("ui.frame_runs_through"), line_label(m, line)));
                    if crate::ui_kit::button(ui, tr("ui.frame_swap_through"), crate::ui_kit::Button::Small).clicked() {
                        swap_through(m, j);
                    }
                }
                if overridden && crate::ui_kit::button(ui, tr("ui.frame_corner_reset"), crate::ui_kit::Button::Small).clicked() {
                    m.overrides.remove(&j.id);
                }
            });
        });
    }
}

/// Size (points) of a junction marker's click target in the view.
const MARKER: f32 = 18.0;

/// The junctions marked in the view while Frame Member's corners are on, with where each is drawn.
fn markers<'a>(m: &'a Member, proj: &Proj) -> Vec<(&'a frame::JunctionReport, Pos2)> {
    if CORNERS.get(m.corner).is_some_and(|c| c.0 == "none") {
        return Vec::new();
    }
    let Some((_, list)) = &m.junctions else { return Vec::new() };
    list.iter().filter_map(|j| proj.to_screen(Vec3::new(j.at[0], j.at[1], j.at[2])).map(|c| (j, c))).collect()
}

/// Whether point `p` is on a junction marker of the open Frame Member window, so a click there
/// swaps the corner instead of picking in the model.
pub(crate) fn marker_at(app: &SolveApp, proj: &Proj, p: Pos2) -> bool {
    let Some(Kind::Frame(m)) = app.dialog.as_ref().map(|d| &d.kind) else { return false };
    markers(m, proj).iter().any(|(_, c)| Rect::from_center_size(*c, vec2(MARKER, MARKER)).contains(p))
}

/// The junction markers in the view: a dot at each junction (filled where it has its own
/// settings), a stroke along the member that runs through at a butt, and the junction's id when
/// it or its row is under the pointer. A click on a butt's marker runs the other member through.
pub fn overlay(app: &SolveApp, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj, d: &mut crate::dialogs::Dialog) {
    let inputs = d.inputs.clone();
    let Kind::Frame(m) = &mut d.kind else { return };
    refresh_junctions(app, m, &inputs);
    let t = solidcraft_theme::Tokens::get();
    let reach = app.cam.half_height() * 0.05;
    let mut clicked = None;
    for (j, c) in markers(m, proj) {
        let r = ui.interact(Rect::from_center_size(c, vec2(MARKER, MARKER)), ui.scope_id().with(("frame_corner_marker", &j.id)), Sense::click());
        let hot = r.hovered() || m.list_hover.as_deref() == Some(j.id.as_str());
        // The member that runs through: a short arrow along it from the corner, over the path.
        if let Some(line) = &j.through
            && let Some(k) = j.ends.iter().position(|(l, _)| l == line)
            && let Some(u) = j.inward.get(k)
            && let Some(tip) = proj.to_screen(Vec3::new(j.at[0], j.at[1], j.at[2]) + Vec3::new(u[0], u[1], u[2]) * reach)
        {
            let dir = (tip - c).normalized();
            let end = c + dir * if hot { 56.0 } else { 40.0 };
            let side = vec2(-dir.y, dir.x) * 6.0;
            let stroke = Stroke::new(if hot { 4.0 } else { 3.0 }, t.manipulator);
            painter.line_segment([c, end], stroke);
            painter.line_segment([end, end - dir * 10.0 + side], stroke);
            painter.line_segment([end, end - dir * 10.0 - side], stroke);
        }
        let fill = if m.overrides.contains_key(&j.id) { t.manipulator } else { t.panel };
        painter.circle(c, if hot { 9.0 } else { 7.0 }, fill, Stroke::new(2.5, t.manipulator));
        if hot {
            let text = match &j.through {
                Some(line) => format!("{} · {} {}", j.id, tr("ui.frame_runs_through"), line_label(m, line)),
                None => format!("{} · {}", j.id, kind_label(j.kind)),
            };
            // On a panel of its own, readable over the members.
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(t.font_label), t.text);
            let at = c + vec2(14.0, -14.0 - galley.size().y);
            painter.rect_filled(Rect::from_min_size(at, galley.size()).expand(4.0), 4.0, t.panel);
            painter.galley(at, galley, t.text);
        }
        // On the press: a marker is a switch, not something to drag, and a press that the
        // window is slow to see released still counts.
        if j.through.is_some() && r.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
            clicked = Some(j.clone());
        }
    }
    if let Some(j) = clicked {
        swap_through(m, &j);
    }
}

#[cfg(test)]
#[path = "dialogs_frame_tests.rs"]
mod tests;
