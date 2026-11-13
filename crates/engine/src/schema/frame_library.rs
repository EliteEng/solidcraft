//! Contracts and result types for the profile library commands in `cmd/frame_library.rs`
//! (`frame.library.list`, `.add`, `.edit`, `.remove`, `.replace`) and `frame.profile.points` (#4003).
//!
//! Unlike the older frame commands, these handlers read their parameters through the types here
//! ([`typed`]), so a call the contract accepts is exactly a call the handler reads. Whether a sketch
//! exists, an expression evaluates or a section can be built stays the handler's to check, as it
//! has the design.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;

use super::check::located;
use super::frame::{ProfileOut, ProfileParam};
use super::types::{ProfilesParam, SketchRef};
use crate::params::bad;

/// The call's parameters read as `T`, with the contract's message on the key that is wrong.
pub fn typed<T: DeserializeOwned>(cmd: &str, p: &Value) -> crate::Result<T> {
    let v = if p.is_null() { Value::Object(Default::default()) } else { p.clone() };
    serde_path_to_error::deserialize(&v).map_err(|e| bad(cmd, located(&e)))
}

/// A placement point an entry names: `{name, at: [x, y]}` in the profile's own mm, or
/// `{name, sketch_point}` for a point of the sketch the section is copied from.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct PointParam {
    /// The point's name: letters, digits, `_` and `-`, unique ignoring case, and not one of the
    /// names every profile has (`origin`, `center`, `top_left`...).
    pub name: String,
    /// Where it is, [x, y] mm in the profile's own plane (x across, y up).
    #[serde(default)]
    pub at: Option<[f64; 2]>,
    /// With `profile_sketch`: the id of the sketch point it is (`p3`).
    #[serde(default)]
    pub sketch_point: Option<String>,
}

/// Defines a parameter type with the keys `frame.library.add` and `frame.library.edit` share
/// (what an entry holds) after its own keys. Serde can't refuse unknown keys through `flatten`, so
/// each type spells them out.
macro_rules! entry_params {
    ($(#[$meta:meta])* $name:ident { $($own:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Deserialize, JsonSchema)]
        #[schemars(deny_unknown_fields)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $($own)*
            /// The levels the profile is filed under in pickers, outermost first, at most 3:
            /// `["Steel", "SHS"]`.
            #[serde(default)]
            pub category: Option<Vec<String>>,
            /// The section as a built-in profile, its dimensions numbers in the design's units or
            /// length expressions over its parameters. Give it or `profile_sketch`, not both.
            #[serde(default)]
            pub profile: Option<ProfileParam>,
            /// The section copied from a sketch: its material (or the `profiles` picked), in the
            /// sketch's own x and y around its origin. The entry keeps the copy; later edits to the
            /// sketch don't change it. Give it or `profile`, not both.
            #[serde(default)]
            pub profile_sketch: Option<SketchRef>,
            /// With `profile_sketch`: which of its profiles are the section, as `solid.extrude`
            /// takes them.
            #[serde(default)]
            pub profiles: Option<ProfilesParam>,
            /// Placement points the entry names, beyond the ones every profile has and the ones its
            /// built-in kind generates (`frame.profile.points` lists them). Replaces the entry's list.
            #[serde(default)]
            pub points: Option<Vec<PointParam>>,
            /// The placement point a member made from it sits on unless it names another. Empty
            /// text clears it (the origin).
            #[serde(default)]
            pub default_point: Option<String>,
            /// The stock's material, by name. Empty text clears it.
            #[serde(default)]
            pub material: Option<String>,
            /// What the cut list calls it. Empty text clears it (the entry's name).
            #[serde(default)]
            pub description: Option<String>,
            /// The length the stock comes in, a length expression (`"6 m"`). Empty text clears it.
            #[serde(default)]
            pub stock_length: Option<String>,
        }
    };
}

entry_params! {
    /// Add a profile to the design's profile library. Members made from it (`frame.member` with
    /// `library`) change when it is edited.
    LibraryAddParams {
        /// Its name, unique in the library ignoring case: `SHS 40x40x3`.
        pub name: String,
    }
}

entry_params! {
    /// Change a profile of the library. Keys left out keep their values; every member made from
    /// it is rebuilt.
    LibraryEditParams {
        /// The profile to change, by id (`lp1`) or name.
        pub id: String,
        /// A new name, unique in the library ignoring case.
        #[serde(default)]
        pub name: Option<String>,
    }
}

/// Remove a profile from the library. Refused while members are made from it (`frame.library.replace`
/// moves them to another profile first).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct LibraryRemoveParams {
    /// The profile, by id or name.
    pub id: String,
}

/// Move every member made from one library profile to another.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct LibraryReplaceParams {
    /// The profile the members are made from now, by id or name.
    pub from: String,
    /// The profile they are made from after, by id or name.
    pub to: String,
}

/// List the design's profile library.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct LibraryListParams {}

/// List a profile's placement points: a built-in `profile`, or a `library` profile. One of them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct ProfilePointsParams {
    /// A built-in profile.
    #[serde(default)]
    pub profile: Option<ProfileParam>,
    /// A profile of the design's library, by id or name.
    #[serde(default)]
    pub library: Option<String>,
}

/// A placement point: its name and where it is, mm in the profile's own plane (x across, y up).
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PointOut {
    pub name: String,
    pub at: [f64; 2],
}

/// An entry's section as it is saved: `{"source": "built_in", "profile": {...}}` or
/// `{"source": "drawn", "regions": [...]}`.
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct SectionOut(pub Value);

impl JsonSchema for SectionOut {
    fn schema_name() -> Cow<'static, str> {
        "LibrarySection".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The section: {\"source\": \"built_in\", \"profile\": {kind, …dimensions as given}} or {\"source\": \"drawn\", \"regions\": [the copied regions]}.",
            "type": "object",
            "required": ["source"]
        })
    }
}

/// One profile of the library, as the commands report it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LibraryEntryOut {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category: Vec<String>,
    pub section: SectionOut,
    /// With a built-in section: the profile in mm, every default filled in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileOut>,
    /// Every placement point: those every profile has, those its kind generates, then its own.
    pub points: Vec<PointOut>,
    /// The points the entry names itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub named_points: Vec<PointOut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The stock length as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_length: Option<String>,
    /// The stock length worked out, mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_length_mm: Option<f64>,
    /// The section's area, mm².
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    /// The features whose members are made from it, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_by: Vec<String>,
    /// Why it can't be worked out in this design now (a parameter it uses is gone...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A feature that failed to rebuild after a library change.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct LibraryFeatureError {
    pub feature: String,
    pub error: String,
}

/// What `frame.library.list` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryListResult {
    pub profiles: Vec<LibraryEntryOut>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.library.add` and `frame.library.edit` return: the entry, and the features that
/// failed to rebuild with it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryEntryResult {
    pub profile: LibraryEntryOut,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<LibraryFeatureError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.library.remove` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryRemoveResult {
    /// The removed profile's id.
    pub removed: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.library.replace` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryReplaceResult {
    /// The features moved to the other profile, by name.
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<LibraryFeatureError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.profile.points` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProfilePointsResult {
    pub points: Vec<PointOut>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// A profile library file's JSON as given (`{"format": "solidcraft-frame-profiles/1", "profiles":
/// [...]}`); the handler reads and checks it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct LibraryFileJson(pub Value);

impl JsonSchema for LibraryFileJson {
    fn schema_name() -> Cow<'static, str> {
        "FrameProfileLibraryFile".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A profile library: {\"format\": \"solidcraft-frame-profiles/1\", \"profiles\": [entries as frame.personal.list holds them]}.",
            "type": "object",
            "required": ["format", "profiles"]
        })
    }
}

/// One library profile as it is saved (`{id, name, category, section, points, default_point,
/// material, description, stock_length}`).
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct EntryJson(pub Value);

impl JsonSchema for EntryJson {
    fn schema_name() -> Cow<'static, str> {
        "FrameLibraryEntry".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A library profile as it is saved: {id, name, category?, section, points?, default_point?, material?, description?, stock_length?}.",
            "type": "object",
            "required": ["name", "section"]
        })
    }
}

/// A file path as the library commands read `path`: not blank, at most 4096 bytes.
fn path_ok(path: &Option<String>) -> bool {
    path.as_ref().is_none_or(|p| !p.trim().is_empty() && p.len() <= 4096)
}

/// Add the profiles of a library file to the design's library: from `path`, or `library` given
/// inline. One of them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct LibraryImportParams {
    /// The library file to read.
    #[serde(default)]
    pub path: Option<String>,
    /// The library itself.
    #[serde(default)]
    pub library: Option<LibraryFileJson>,
    /// Only these of its profiles, by name. Default: all of them.
    #[serde(default)]
    pub names: Option<Vec<String>>,
}

impl LibraryImportParams {
    pub fn path_ok(&self) -> bool {
        path_ok(&self.path)
    }
}

/// Write profiles of the design's library as a library file. Expressions that read the design's
/// parameters are written as their values, so the file means the same in any design.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct LibraryExportParams {
    /// The profiles to write, by id or name. Default: all of them.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// Where to write the file. Default: nowhere (the reply holds it).
    #[serde(default)]
    pub path: Option<String>,
}

impl LibraryExportParams {
    pub fn path_ok(&self) -> bool {
        path_ok(&self.path)
    }
}

/// List the personal profile library (kept by the app, offered in every design).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct PersonalListParams {}

/// Add profiles to the personal library: a profile of the design's library (`id`), or the profiles
/// of a library file (`path` or `library`). One of them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct PersonalAddParams {
    /// A profile of the design's library, by id or name. Expressions that read the design's
    /// parameters are kept as their values.
    #[serde(default)]
    pub id: Option<String>,
    /// A library file to read.
    #[serde(default)]
    pub path: Option<String>,
    /// A library given inline.
    #[serde(default)]
    pub library: Option<LibraryFileJson>,
    /// A personal profile of the same name is replaced. Default: false, and such a name is refused.
    #[serde(default)]
    pub replace: Option<bool>,
}

impl PersonalAddParams {
    pub fn path_ok(&self) -> bool {
        path_ok(&self.path)
    }
}

/// Remove a profile from the personal library. Designs that use it keep their own copy.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct PersonalRemoveParams {
    /// The personal profile, by name.
    pub name: String,
}

/// Write profiles of the personal library as a library file.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub struct PersonalExportParams {
    /// The profiles to write, by name. Default: all of them.
    #[serde(default)]
    pub names: Option<Vec<String>>,
    /// Where to write the file. Default: nowhere (the reply holds it).
    #[serde(default)]
    pub path: Option<String>,
}

impl PersonalExportParams {
    pub fn path_ok(&self) -> bool {
        path_ok(&self.path)
    }
}

/// A profile a library file brought into the design.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ImportedOut {
    /// Its name in the file.
    pub from: String,
    /// Its id in the design's library.
    pub id: String,
    /// Its name in the design's library (` (2)`… when the library had the name for another profile).
    pub name: String,
    /// The design's library had the same profile already, and it is used.
    pub reused: bool,
}

/// What `frame.library.import` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryImportResult {
    pub profiles: Vec<ImportedOut>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.library.export` and `frame.personal.export` return: the library file, and where it
/// was written.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryExportResult {
    pub library: LibraryFileJson,
    /// `frame.personal.export`: the personal profiles asked for, by name, for the app to write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.personal.list` and `frame.personal.add` return: personal profiles (the whole
/// library, or those added).
#[derive(Debug, Serialize, JsonSchema)]
pub struct PersonalProfilesResult {
    pub profiles: Vec<EntryJson>,
    /// With `frame.personal.add`: personal profiles of the same names are replaced.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `frame.personal.remove` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PersonalRemoveResult {
    /// The removed profile's name.
    pub removed: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "frame_library_tests.rs"]
mod tests;
