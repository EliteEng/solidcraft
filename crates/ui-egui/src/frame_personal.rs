//! The personal profile library: frame profiles offered by Frame Member in every design
//! (docs/frames.md). They are library files in your profile folders, whose subfolders, up to three
//! levels, are the categories the profiles are filed under (`Steel/Purlins/C 150x50x2.4.json`), so
//! a shop can keep one set of profiles on a shared drive. Where no folder can be written (the
//! browser, which has no folders to keep, and tests) they are kept with the preferences instead.
//!
//! The engine's `frame.personal.*` commands check their parameters and build the entries they add
//! (from a design's library, a library file or a sketch's section); this module keeps them. A design
//! never depends on it: a member made from a personal profile copies it into the design's library.
//!
//! Reading the folders can be slow (a network drive), so the window reads them on a thread
//! ([`poll`]); the commands read them in place, as an agent waits for its reply anyway.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcraft_engine::doc::frame_library::{LibraryFile, LibraryProfile, MAX_CATEGORY_LEVELS, MAX_LIBRARY_PROFILES};
use solidcraft_engine::io::vfs;
use solidcraft_engine::schema::frame_library::{FolderOut, MAX_PROFILE_FOLDERS, SkippedOut};

use crate::SolveApp;

/// The id prefix that marks a personal profile in the Frame Member window, apart from the design's
/// library ids (`lp1`…).
pub const ID_PREFIX: &str = "personal:";

/// What the commands report as the folder of a profile kept with the preferences.
pub const PREFERENCES: &str = "preferences";

/// The most bytes one profile file may have.
const MAX_PROFILE_FILE: usize = 8 << 20;

/// The most files the folders may have between them.
const MAX_FILES: usize = 10_000;

/// A personal profile's id: its name, which is unique ignoring case.
fn id_for(name: &str) -> String {
    format!("{ID_PREFIX}{}", name.to_lowercase())
}

/// The personal profiles kept with the preferences, in the order they were added: the browser's
/// library, and in the desktop app what older versions kept there until it is moved into the
/// first profile folder ([`migrate`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Library(pub Vec<LibraryProfile>);

impl<'de> Deserialize<'de> for Library {
    /// Rows that can't be read, aren't valid profiles or repeat a name are left out, so one bad
    /// row never costs the others (nor the rest of the preferences).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Library::lenient(&Value::deserialize(d)?))
    }
}

impl Library {
    fn lenient(v: &Value) -> Library {
        let mut out = Library::default();
        for row in v.as_array().into_iter().flatten() {
            if out.0.len() >= MAX_LIBRARY_PROFILES {
                break;
            }
            if let Ok(e) = serde_json::from_value::<LibraryProfile>(row.clone())
                && e.check().is_ok()
                && out.find(&e.name).is_none()
            {
                out.0.push(LibraryProfile { id: id_for(&e.name), ..e });
            }
        }
        out
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.0.iter().position(|e| e.name.eq_ignore_ascii_case(name.trim()))
    }
}

/// Where a personal profile is kept.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A library file in profile folder `folder` (its place in the folder list).
    Folder { folder: usize, file: PathBuf },
    /// The preferences.
    Preferences,
}

/// A personal profile and where it is kept.
#[derive(Clone, Debug)]
pub struct Located {
    pub entry: LibraryProfile,
    pub source: Source,
}

/// What reading the profile folders found.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    /// The profiles, the earlier folder's first where two have the same name.
    entries: Vec<Located>,
    folders: Vec<FolderOut>,
    skipped: Vec<SkippedOut>,
}

/// The app's view of the profile folders.
#[derive(Default)]
pub struct State {
    scan: Scan,
    /// The folders `scan` read; `None` until they are read, or after Refresh.
    read: Option<Vec<String>>,
    /// A reading under way on a thread, for these folders.
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<(Vec<String>, std::sync::mpsc::Receiver<Scan>)>,
    /// Profiles kept with the preferences were moved into a folder this run (or there were none).
    migrated: bool,
}

impl State {
    /// Read the folders again (on a thread, from the next frame).
    pub fn refresh(&mut self) {
        self.read = None;
    }

    /// A reading is under way.
    pub fn loading(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// The folders as last read.
    pub fn folders(&self) -> &[FolderOut] {
        &self.scan.folders
    }
}

/// The profile folders, in order: those set in Preferences, else the host's default folder (none in
/// the browser, which has no folders to keep, nor in tests).
pub fn folders(app: &SolveApp) -> Vec<String> {
    if cfg!(target_arch = "wasm32") {
        return Vec::new();
    }
    let set: Vec<String> = app.preferences.frame_profile_folders.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
    if set.is_empty() { app.services.frame_profiles_dir.iter().cloned().collect() } else { set }
}

/// Every personal profile Frame Member offers: those in the folders, then those kept with the
/// preferences whose names the folders don't have.
pub fn entries(app: &SolveApp) -> Vec<&LibraryProfile> {
    located(app).into_iter().map(|(e, _)| e).collect()
}

fn located(app: &SolveApp) -> Vec<(&LibraryProfile, Source)> {
    let scan = &app.frame_personal.scan;
    let mut out: Vec<(&LibraryProfile, Source)> = scan.entries.iter().map(|l| (&l.entry, l.source.clone())).collect();
    for e in &app.preferences.frame_profiles.0 {
        if !out.iter().any(|(x, _)| x.name.eq_ignore_ascii_case(&e.name)) {
            out.push((e, Source::Preferences));
        }
    }
    out
}

/// The personal profile with Frame Member id `id`.
pub fn get<'a>(app: &'a SolveApp, id: &str) -> Option<&'a LibraryProfile> {
    entries(app).into_iter().find(|e| e.id == id)
}

/// Keeps the folders read: starts a reading on a thread when the folder list changed or Refresh
/// asked, takes a finished one, then moves profiles kept with the preferences into the first folder.
pub fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    let want = folders(app);
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some((asked, rx)) = &app.frame_personal.pending {
            match rx.try_recv() {
                Ok(scan) => {
                    let asked = asked.clone();
                    app.frame_personal.pending = None;
                    app.frame_personal.scan = scan;
                    app.frame_personal.read = Some(asked);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.frame_personal.pending = None;
                    app.frame_personal.read = Some(want.clone());
                }
            }
        }
        if app.frame_personal.read.as_ref() != Some(&want) {
            let (tx, rx) = std::sync::mpsc::channel();
            let (asked, default) = (want.clone(), app.services.frame_profiles_dir.clone());
            match std::thread::Builder::new().name("frame-profiles".into()).spawn(move || {
                let _ = tx.send(scan_folders(&asked, default.as_deref()));
            }) {
                Ok(_) => app.frame_personal.pending = Some((want, rx)),
                // No thread: read them here rather than never.
                Err(_) => read_now(app),
            }
            return;
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        if app.frame_personal.read.as_ref() != Some(&want) {
            read_now(app);
        }
    }
    if !app.frame_personal.migrated {
        app.frame_personal.migrated = true;
        if let Err(e) = migrate(app) {
            app.set_status(e, true);
        }
    }
}

/// Reads the folders in place (the commands, and where no thread can start).
fn read_now(app: &mut SolveApp) {
    let want = folders(app);
    app.frame_personal.scan = scan_folders(&want, app.services.frame_profiles_dir.as_deref());
    app.frame_personal.read = Some(want);
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.frame_personal.pending = None;
    }
}

/// Reads the folders in place unless they were read for the current list (the commands).
fn fresh(app: &mut SolveApp) {
    if app.frame_personal.read.as_ref() != Some(&folders(app)) || app.frame_personal.loading() {
        read_now(app);
    }
}

/// Reads every profile in `folders`. The default folder needn't exist yet: the first save makes it.
pub fn scan_folders(folders: &[String], default: Option<&str>) -> Scan {
    let mut scan = Scan::default();
    let mut files = 0usize;
    for (i, path) in folders.iter().take(MAX_PROFILE_FOLDERS).enumerate() {
        let root = Path::new(path);
        let before = scan.entries.len();
        let mut out = FolderOut { path: path.clone(), writable: false, profiles: 0, error: None };
        if root.is_dir() {
            walk(root, &mut Vec::new(), i, &mut scan, &mut files);
            out.writable = writable(root);
        } else if default == Some(path.as_str()) {
            out.writable = true;
        } else {
            out.error = Some("it doesn't exist or can't be reached".to_string());
        }
        out.profiles = scan.entries.len() - before;
        scan.folders.push(out);
    }
    scan
}

/// Reads the profile files in `dir`, filed under `levels`, and the folders in it one level down.
fn walk(dir: &Path, levels: &mut Vec<String>, folder: usize, scan: &mut Scan, files: &mut usize) {
    let shown = |p: &Path| p.to_string_lossy().into_owned();
    let mut items: Vec<_> = match std::fs::read_dir(dir) {
        Ok(read) => read.filter_map(Result::ok).collect(),
        Err(e) => {
            scan.skipped.push(SkippedOut { path: shown(dir), reason: e.to_string() });
            return;
        }
    };
    items.sort_by_key(|item| item.file_name());
    for item in items {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        // Hidden files and folders (`.git`, a probe left behind) are not profiles.
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = item.file_type() else { continue };
        if kind.is_dir() {
            if levels.len() >= MAX_CATEGORY_LEVELS {
                scan.skipped.push(SkippedOut {
                    path: shown(&path),
                    reason: format!("more than {MAX_CATEGORY_LEVELS} category levels down: move its profiles up"),
                });
                continue;
            }
            levels.push(name);
            walk(&path, levels, folder, scan, files);
            levels.pop();
        } else if kind.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) {
            *files += 1;
            if *files > MAX_FILES || scan.entries.len() >= MAX_LIBRARY_PROFILES {
                scan.skipped.push(SkippedOut {
                    path: shown(&path),
                    reason: format!("the folders hold more than {MAX_LIBRARY_PROFILES} profiles or {MAX_FILES} files"),
                });
                continue;
            }
            match read_file(&path) {
                Ok(file) => {
                    for e in file.profiles {
                        if let Some(first) = scan.entries.iter().find(|l| l.entry.name.eq_ignore_ascii_case(&e.name)) {
                            let at = match &first.source {
                                Source::Folder { file, .. } => shown(file),
                                Source::Preferences => PREFERENCES.to_string(),
                            };
                            let reason = format!("`{}` is in {at} already, which is offered instead", e.name);
                            scan.skipped.push(SkippedOut { path: shown(&path), reason });
                            continue;
                        }
                        let entry = LibraryProfile { id: id_for(&e.name), category: levels.clone(), ..e };
                        scan.entries.push(Located { entry, source: Source::Folder { folder, file: path.clone() } });
                    }
                }
                Err(reason) => scan.skipped.push(SkippedOut { path: shown(&path), reason }),
            }
        }
    }
}

/// A profile file read: at most [`MAX_PROFILE_FILE`] bytes, a valid library file.
fn read_file(path: &Path) -> Result<LibraryFile, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_PROFILE_FILE as u64 {
        return Err(format!("over {} MB, too big for a profile file", MAX_PROFILE_FILE >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    LibraryFile::from_json(v).map_err(|e| e.to_string())
}

/// Whether files can be made in `dir`: a probe file is made and removed again.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".solidcraft-write-check-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// `text` as one file or folder name: characters no system allows become `_`, it is trimmed of
/// spaces and dots, and kept to 120 bytes. Empty: `profile`.
fn file_name(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    out = out.trim().trim_matches('.').trim().to_string();
    while out.len() > 120 {
        out.pop();
    }
    if out.is_empty() { "profile".into() } else { out }
}

/// Where a profile is saved in folder `root`: its categories as subfolders, its name as the file
/// name, made unique with ` (2)`… against the files there.
fn file_for(root: &Path, e: &LibraryProfile) -> PathBuf {
    let mut dir = root.to_path_buf();
    for level in e.category.iter().take(MAX_CATEGORY_LEVELS) {
        dir.push(file_name(level));
    }
    let base = file_name(&e.name);
    let mut path = dir.join(format!("{base}.json"));
    let mut n = 2;
    while path.exists() && n < 1000 {
        path = dir.join(format!("{base} ({n}).json"));
        n += 1;
    }
    path
}

/// The profile folder a save goes to: `key` (a path in the list or its number from 1), else the
/// first one that can be written. `None`: there are no folders, so the preferences keep it.
fn target(app: &SolveApp, key: Option<&str>) -> Result<Option<usize>, String> {
    let list = &app.frame_personal.scan.folders;
    if list.is_empty() {
        return match key {
            Some(_) => Err("there are no profile folders here (the browser keeps your profiles with its settings)".to_string()),
            None => Ok(None),
        };
    }
    let i = match key {
        Some(k) => match k.parse::<usize>() {
            Ok(n) if (1..=list.len()).contains(&n) => n - 1,
            _ => list
                .iter()
                .position(|f| Path::new(&f.path) == Path::new(k))
                .ok_or_else(|| format!("`{k}` is not one of your profile folders (frame.personal.folders lists them)"))?,
        },
        None => list
            .iter()
            .position(|f| f.writable)
            .ok_or_else(|| "none of your profile folders can be written to: add one you can write to in Preferences".to_string())?,
    };
    match list.get(i) {
        Some(f) if f.writable => Ok(Some(i)),
        Some(f) => Err(format!("{} can't be written to", f.path)),
        None => Err("there are no profile folders here (the browser keeps your profiles with its settings)".to_string()),
    }
}

/// Writes `entries` as a library file at `path`, making its folders. Only the profile folders
/// are reached, whatever an agent's own folders are.
fn write_file(roots: &[String], path: &Path, entries: Vec<LibraryProfile>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&LibraryFile::new(entries)).map_err(|e| e.to_string())?;
    let shown = path.to_string_lossy().into_owned();
    let scope = vfs::PathScope::new(roots.iter().map(PathBuf::from));
    vfs::scoped(Some(scope), || {
        if let Some(dir) = path.parent() {
            vfs::create_new_dir_all(&dir.to_string_lossy())?;
        }
        vfs::write(&shown, text.as_bytes())
    })
    .map_err(|e| format!("{shown}: {e}"))
}

/// Removes profile `name` from the library file it is in: the file goes when it held only that one.
fn remove_from_file(roots: &[String], file: &Path, name: &str) -> Result<(), String> {
    let rest: Vec<LibraryProfile> = read_file(file)?.profiles.into_iter().filter(|e| !e.name.eq_ignore_ascii_case(name)).collect();
    if rest.is_empty() {
        let shown = file.to_string_lossy().into_owned();
        let scope = vfs::PathScope::new(roots.iter().map(PathBuf::from));
        vfs::scoped(Some(scope), || vfs::remove(&shown)).map_err(|e| format!("{shown}: {e}"))
    } else {
        write_file(roots, file, rest)
    }
}

/// Saves `entries` as personal profiles in folder `folder` (else the preferences), each in a file of
/// its own under its categories; with `replace` in place of personal profiles of the same names.
/// Every name is checked before anything is written.
fn save(app: &mut SolveApp, entries: Vec<LibraryProfile>, replace: bool, folder: Option<&str>) -> Result<Vec<Located>, String> {
    let target = target(app, folder)?;
    let roots = folders(app);
    let have = located(app).into_iter().map(|(e, s)| (e.name.clone(), s)).collect::<Vec<_>>();
    for e in &entries {
        if let Some((had, source)) = have.iter().find(|(n, _)| n.eq_ignore_ascii_case(&e.name)) {
            if !replace {
                return Err(format!("your library has `{had}` already (`replace: true` replaces it)"));
            }
            if let Source::Folder { folder, .. } = source
                && !app.frame_personal.scan.folders.get(*folder).is_some_and(|f| f.writable)
            {
                let path = app.frame_personal.scan.folders.get(*folder).map(|f| f.path.clone()).unwrap_or_default();
                return Err(format!("`{had}` is in {path}, which can't be written to"));
            }
        }
    }
    let count = entries.iter().filter(|e| !have.iter().any(|(n, _)| n.eq_ignore_ascii_case(&e.name))).count();
    if have.len() + count > MAX_LIBRARY_PROFILES {
        return Err(format!("your library holds at most {MAX_LIBRARY_PROFILES} profiles"));
    }
    let mut saved = Vec::new();
    for e in entries {
        let e = LibraryProfile { id: id_for(&e.name), ..e };
        // The one it replaces goes first, so its file name is free; one kept with the preferences
        // that stays there keeps its place.
        let mut slot = None;
        if let Some((_, source)) = have.iter().find(|(n, _)| n.eq_ignore_ascii_case(&e.name)) {
            match source {
                Source::Folder { file, .. } => remove_from_file(&roots, file, &e.name)?,
                Source::Preferences => {
                    let library = &mut app.preferences.frame_profiles;
                    if let Some(i) = library.find(&e.name) {
                        library.0.remove(i);
                        slot = Some(i);
                    }
                }
            }
            app.frame_personal.scan.entries.retain(|l| !l.entry.name.eq_ignore_ascii_case(&e.name));
        }
        let source = match target.and_then(|i| roots.get(i).map(|r| (i, r))) {
            Some((i, root)) => {
                // The default folder is made by the first save.
                std::fs::create_dir_all(root).map_err(|err| format!("{root}: {err}"))?;
                let file = file_for(Path::new(root), &e);
                write_file(&roots, &file, vec![e.clone()])?;
                Source::Folder { folder: i, file }
            }
            None => {
                let library = &mut app.preferences.frame_profiles.0;
                library.insert(slot.unwrap_or(library.len()).min(library.len()), e.clone());
                Source::Preferences
            }
        };
        let l = Located { entry: e, source };
        if let Source::Folder { folder, .. } = &l.source {
            if let Some(f) = app.frame_personal.scan.folders.get_mut(*folder) {
                f.profiles += 1;
            }
            app.frame_personal.scan.entries.push(l.clone());
        }
        saved.push(l);
    }
    Ok(saved)
}

/// Moves the profiles older versions kept with the preferences into the first folder that can be
/// written, each filed under its categories. One stays where it is when its file can't be written
/// or a folder has a different profile of its name; one a folder has already, the same, is dropped.
fn migrate(app: &mut SolveApp) -> Result<(), String> {
    if app.preferences.frame_profiles.0.is_empty() {
        return Ok(());
    }
    let Ok(Some(target)) = target(app, None) else { return Ok(()) };
    let roots = folders(app);
    let Some(root) = roots.get(target).cloned() else { return Ok(()) };
    let mut kept = Vec::new();
    let mut failed = None;
    for e in std::mem::take(&mut app.preferences.frame_profiles.0) {
        match app.frame_personal.scan.entries.iter().find(|l| l.entry.name.eq_ignore_ascii_case(&e.name)) {
            Some(l) if l.entry.same_as(&e) => {}
            Some(_) => kept.push(e),
            None => {
                let file = file_for(Path::new(&root), &e);
                let written =
                    std::fs::create_dir_all(&root).map_err(|err| format!("{root}: {err}")).and_then(|()| write_file(&roots, &file, vec![e.clone()]));
                match written {
                    Ok(()) => {
                        if let Some(f) = app.frame_personal.scan.folders.get_mut(target) {
                            f.profiles += 1;
                        }
                        app.frame_personal.scan.entries.push(Located { entry: e, source: Source::Folder { folder: target, file } });
                    }
                    Err(err) => {
                        failed.get_or_insert(err);
                        kept.push(e);
                    }
                }
            }
        }
    }
    app.preferences.frame_profiles.0 = kept;
    match failed {
        Some(err) => Err(format!("Some frame profiles kept in your settings could not be moved to your profile folder: {err}")),
        None => Ok(()),
    }
}

/// An entry as the commands report it: as saved, without the window's id, with where it is kept.
fn shown(e: &LibraryProfile, source: Option<(&Source, &[String])>) -> Value {
    let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("id");
        match source {
            Some((Source::Folder { folder, file }, roots)) => {
                o.insert("folder".into(), json!(roots.get(*folder)));
                o.insert("file".into(), json!(file.to_string_lossy()));
            }
            Some((Source::Preferences, _)) => {
                o.insert("folder".into(), json!(PREFERENCES));
            }
            None => {}
        }
    }
    v
}

/// A personal profile as `frame.member` takes it in `library`.
pub fn member_library(e: &LibraryProfile) -> Value {
    shown(e, None)
}

/// Sets the profile folders. While an agent runs the command, each folder must be one it may use.
fn set_folders(app: &mut SolveApp, paths: Vec<String>) -> Result<(), String> {
    if let Some(scope) = vfs::current_scope() {
        for p in &paths {
            scope.check(p).map_err(|e| format!("{p}: {e}"))?;
        }
    }
    let mut seen: Vec<String> = Vec::new();
    for p in paths {
        if !seen.iter().any(|s| Path::new(s) == Path::new(&p)) {
            seen.push(p);
        }
    }
    app.preferences.frame_profile_folders = seen;
    app.frame_personal.migrated = false;
    read_now(app);
    if let Err(e) = migrate(app) {
        app.set_status(e, true);
    }
    app.frame_personal.migrated = true;
    Ok(())
}

/// Preferences › Tools › Frames: the profile folders in order, each with how many profiles it has
/// (or why it can't be read), and Add Folder, Move Up, Remove, Open and Refresh. Changes go
/// through `frame.personal.folders`.
pub fn folders_ui(app: &mut SolveApp, ui: &mut egui::Ui) {
    use crate::ui_kit::{Button, StatusTone, button, status_line};
    let tr = |key: &str| solidcraft_ui_text::tr(key).to_string();
    ui.label(tr("prefs.frame_folders"));
    let set = folders(app);
    if set.is_empty() {
        status_line(ui, &tr("prefs.frame_folders_browser"), StatusTone::Dim);
        return;
    }
    status_line(ui, &tr("prefs.frame_folders_note"), StatusTone::Dim);
    let explicit = !app.preferences.frame_profile_folders.is_empty();
    let shown = app.frame_personal.scan.folders.clone();
    let mut change: Option<Vec<String>> = None;
    for (i, path) in set.iter().enumerate() {
        let info = shown.iter().find(|f| &f.path == path);
        let count = info.map(|f| f.profiles.to_string()).unwrap_or_default();
        let status = match info {
            Some(FolderOut { error: Some(e), .. }) => e.clone(),
            Some(f) if !f.writable => solidcraft_ui_text::format("prefs.frame_folder_count_read_only", &[("count", &count)]),
            Some(_) => solidcraft_ui_text::format("prefs.frame_folder_count", &[("count", &count)]),
            None => tr("ui.frame_folders_reading"),
        };
        let title = if explicit { path.clone() } else { solidcraft_ui_text::format("prefs.frame_folder_default", &[("folder", path)]) };
        ui.label(title);
        ui.horizontal_wrapped(|ui| {
            status_line(ui, &status, if info.is_some_and(|f| f.error.is_some()) { StatusTone::Warning } else { StatusTone::Dim });
            if i > 0 && button(ui, tr("prefs.frame_folder_up"), Button::Normal).clicked() {
                let mut list = set.clone();
                list.swap(i, i - 1);
                change = Some(list);
            }
            if explicit && button(ui, tr("prefs.frame_folder_remove"), Button::Normal).clicked() {
                let mut list = set.clone();
                list.remove(i);
                change = Some(list);
            }
            if app.services.show_in_folder.is_some()
                && Path::new(path).is_dir()
                && button(ui, tr("prefs.frame_folder_open"), Button::Normal).clicked()
                && let Some(show) = &app.services.show_in_folder
            {
                // The host opens the folder a file is in.
                show(&Path::new(path).join("profile.json").to_string_lossy());
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        if app.services.pick_folder.is_some() && button(ui, tr("prefs.frame_folder_add"), Button::Normal).clicked() {
            let base = set.clone();
            let current = set.last().cloned().unwrap_or_default();
            app.pick_folder(&current, move |app, picked| {
                let Some(picked) = picked else { return };
                let mut list = base;
                list.push(picked);
                if let Err(e) = app.run("frame.personal.folders", json!({ "folders": list })) {
                    app.set_status(e, true);
                }
            });
        }
        if button(ui, tr("prefs.frame_folders_refresh"), Button::Normal).clicked() {
            app.frame_personal.refresh();
        }
    });
    if let Some(list) = change
        && let Err(e) = app.run("frame.personal.folders", json!({ "folders": list }))
    {
        app.set_status(e, true);
    }
}

/// Acts on a `frame.personal.*` command the engine checked, from the engine's reply (which echoes
/// what the call asked for).
pub fn command(app: &mut SolveApp, id: &str, reply: &Value) -> Result<Value, String> {
    if id == "frame.personal.folders" {
        if reply.get("set").and_then(Value::as_bool).unwrap_or(false) {
            let paths = reply["folders"].as_array().into_iter().flatten().filter_map(|f| f["path"].as_str().map(str::to_string)).collect();
            set_folders(app, paths)?;
        } else {
            fresh(app);
        }
        return Ok(json!({ "folders": app.frame_personal.scan.folders }));
    }
    if id == "frame.personal.list" {
        read_now(app);
    } else {
        fresh(app);
    }
    let roots = folders(app);
    match id {
        "frame.personal.list" => {
            let profiles: Vec<Value> = located(app).iter().map(|(e, s)| shown(e, Some((s, &roots)))).collect();
            let mut out = json!({ "profiles": profiles });
            if !app.frame_personal.scan.folders.is_empty() {
                out["folders"] = json!(app.frame_personal.scan.folders);
            }
            if !app.frame_personal.scan.skipped.is_empty() {
                out["skipped"] = json!(app.frame_personal.scan.skipped);
            }
            Ok(out)
        }
        "frame.personal.add" | "frame.personal.save" => {
            let entries = reply
                .get("profiles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|v| serde_json::from_value::<LibraryProfile>(v.clone()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let replace = reply.get("replace").and_then(Value::as_bool).unwrap_or(false);
            let folder = reply.get("folder").and_then(Value::as_str);
            let saved = save(app, entries, replace, folder)?;
            let mut out = json!({ "profiles": saved.iter().map(|l| shown(&l.entry, Some((&l.source, &roots)))).collect::<Vec<_>>() });
            if replace {
                out["replace"] = json!(true);
            }
            Ok(out)
        }
        "frame.personal.remove" => {
            let name = reply.get("removed").and_then(Value::as_str).unwrap_or_default();
            let (e, source) = located(app)
                .into_iter()
                .find(|(e, _)| e.name.eq_ignore_ascii_case(name.trim()))
                .map(|(e, s)| (e.clone(), s))
                .ok_or_else(|| format!("your library has no profile `{name}`"))?;
            match source {
                Source::Folder { folder, file } => {
                    let f = app.frame_personal.scan.folders.get(folder).cloned();
                    if !f.as_ref().is_some_and(|f| f.writable) {
                        let path = f.map(|f| f.path).unwrap_or_default();
                        return Err(format!("`{}` is in {path}, which can't be written to", e.name));
                    }
                    remove_from_file(&roots, &file, &e.name)?;
                    app.frame_personal.scan.entries.retain(|l| !l.entry.name.eq_ignore_ascii_case(&e.name));
                    if let Some(f) = app.frame_personal.scan.folders.get_mut(folder) {
                        f.profiles = f.profiles.saturating_sub(1);
                    }
                }
                Source::Preferences => {
                    let library = &mut app.preferences.frame_profiles;
                    if let Some(i) = library.find(&e.name) {
                        library.0.remove(i);
                    }
                }
            }
            Ok(json!({ "removed": e.name }))
        }
        "frame.personal.export" => {
            let all: Vec<LibraryProfile> = entries(app).into_iter().cloned().collect();
            let chosen: Vec<LibraryProfile> = match reply.get("names").and_then(Value::as_array) {
                Some(names) => {
                    let mut out: Vec<LibraryProfile> = Vec::new();
                    for name in names.iter().filter_map(Value::as_str) {
                        let e = all
                            .iter()
                            .find(|e| e.name.eq_ignore_ascii_case(name.trim()))
                            .ok_or_else(|| format!("your library has no profile `{name}`"))?;
                        if !out.iter().any(|x| x.id == e.id) {
                            out.push(e.clone());
                        }
                    }
                    out
                }
                None => all,
            };
            if chosen.is_empty() {
                return Err("your library is empty: there is nothing to export".into());
            }
            let v = serde_json::to_value(LibraryFile::new(chosen)).map_err(|e| e.to_string())?;
            let mut out = json!({ "library": v });
            if let Some(path) = reply.get("path").and_then(Value::as_str) {
                let text = serde_json::to_string_pretty(&out["library"]).map_err(|e| e.to_string())?;
                vfs::write(path, text.as_bytes()).map_err(|e| format!("{path}: {e}"))?;
                out["path"] = json!(path);
            }
            Ok(out)
        }
        _ => Err(format!("`{id}` is not a personal library command")),
    }
}

#[cfg(test)]
#[path = "frame_personal_tests.rs"]
mod tests;
