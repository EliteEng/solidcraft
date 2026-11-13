//! The personal profile library: frame profiles kept with the app's preferences and offered by
//! Frame Member in every design (docs/frames.md). The engine's `frame.personal.*` commands check
//! their parameters and read the entries they add (from a design's library or a library file);
//! this module keeps them. A design never depends on it: a member made from a personal profile
//! copies it into the design's own library.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcraft_engine::doc::frame_library::{LibraryFile, LibraryProfile, MAX_LIBRARY_PROFILES};
use solidcraft_engine::io::vfs;

use crate::SolveApp;

/// The id prefix that marks a personal profile in the Frame Member window, apart from the design's
/// library ids (`lp1`…).
pub const ID_PREFIX: &str = "personal:";

/// A personal profile's id: its name, which is unique ignoring case.
fn id_for(name: &str) -> String {
    format!("{ID_PREFIX}{}", name.to_lowercase())
}

/// The personal profiles, in the order they were added.
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

    /// The personal profile with Frame Member id `id`.
    pub fn get(&self, id: &str) -> Option<&LibraryProfile> {
        self.0.iter().find(|e| e.id == id)
    }

    /// Adds `entries`, or with `replace` puts them in place of personal profiles of the same names.
    /// Nothing changes unless all of them can be added.
    fn add(&mut self, entries: Vec<LibraryProfile>, replace: bool) -> Result<Vec<LibraryProfile>, String> {
        let mut next = self.clone();
        let mut added = Vec::new();
        for e in entries {
            let e = LibraryProfile { id: id_for(&e.name), ..e };
            match next.find(&e.name) {
                Some(i) if replace => {
                    if let Some(slot) = next.0.get_mut(i) {
                        *slot = e.clone();
                    }
                }
                Some(i) => {
                    let had = next.0.get(i).map(|x| x.name.clone()).unwrap_or_default();
                    return Err(format!("your library has `{had}` already (`replace: true` replaces it)"));
                }
                None if next.0.len() >= MAX_LIBRARY_PROFILES => {
                    return Err(format!("your library holds at most {MAX_LIBRARY_PROFILES} profiles"));
                }
                None => next.0.push(e.clone()),
            }
            added.push(e);
        }
        *self = next;
        Ok(added)
    }
}

/// An entry as the commands report it: as saved, without the window's id.
fn shown(e: &LibraryProfile) -> Value {
    let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("id");
    }
    v
}

/// A personal profile as `frame.member` takes it in `library`.
pub fn member_library(e: &LibraryProfile) -> Value {
    shown(e)
}

/// Acts on a `frame.personal.*` command the engine checked, from the engine's reply (which echoes
/// what the call asked for).
pub fn command(app: &mut SolveApp, id: &str, reply: &Value) -> Result<Value, String> {
    let library = &mut app.preferences.frame_profiles;
    match id {
        "frame.personal.list" => Ok(json!({ "profiles": library.0.iter().map(shown).collect::<Vec<_>>() })),
        "frame.personal.add" => {
            let entries = reply
                .get("profiles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|v| serde_json::from_value::<LibraryProfile>(v.clone()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let replace = reply.get("replace").and_then(Value::as_bool).unwrap_or(false);
            let added = library.add(entries, replace)?;
            let mut out = json!({ "profiles": added.iter().map(shown).collect::<Vec<_>>() });
            if replace {
                out["replace"] = json!(true);
            }
            Ok(out)
        }
        "frame.personal.remove" => {
            let name = reply.get("removed").and_then(Value::as_str).unwrap_or_default();
            let i = library.find(name).ok_or_else(|| format!("your library has no profile `{name}`"))?;
            let removed = library.0.remove(i);
            Ok(json!({ "removed": removed.name }))
        }
        "frame.personal.export" => {
            let entries: Vec<LibraryProfile> = match reply.get("names").and_then(Value::as_array) {
                Some(names) => {
                    let mut out: Vec<LibraryProfile> = Vec::new();
                    for name in names.iter().filter_map(Value::as_str) {
                        let e = library.find(name).and_then(|i| library.0.get(i)).ok_or_else(|| format!("your library has no profile `{name}`"))?;
                        if !out.iter().any(|x| x.id == e.id) {
                            out.push(e.clone());
                        }
                    }
                    out
                }
                None => library.0.clone(),
            };
            if entries.is_empty() {
                return Err("your library is empty: there is nothing to export".into());
            }
            let v = serde_json::to_value(LibraryFile::new(entries)).map_err(|e| e.to_string())?;
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
