//! Contracts for the cut list commands in `cmd/cut_list.rs`: `frame.cut_list`,
//! `frame.cut_list.renumber` and `frame.cut_list.set`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// `frame.cut_list`'s parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CutListParams {
    /// One frame member feature, by id or name. Default: every frame member feature that isn't
    /// suppressed.
    #[serde(default)]
    pub feature: Option<super::edit::FeatureRef>,
    /// `csv` also gives the list as comma-separated text (`csv`); with `path`, the file's format.
    /// Default: `json` only, or as `path` ends.
    #[serde(default)]
    pub format: Option<CutListFormat>,
    /// Also write the list to this file: `.csv` (or `format: csv`) or `.json`.
    #[serde(default)]
    pub path: Option<String>,
    /// List mirror-image pieces (a left and a right) as one item, `opposite` of them the other
    /// hand. Default: false, apart.
    #[serde(default)]
    pub opposite_hand: Option<bool>,
    /// Also list the end caps and gussets as plates. Default: false.
    #[serde(default)]
    pub plates: Option<bool>,
}

/// How `frame.cut_list.renumber` numbers the items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenumberOrder {
    /// By profile, then longest first (the list's own order).
    Profile,
    /// Longest first, whatever the profile.
    Length,
    /// As they are numbered now: kept numbers stay and new items keep the numbers they show.
    Current,
}

/// `frame.cut_list.renumber`'s parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CutListRenumberParams {
    /// Default: profile.
    #[serde(default)]
    pub order: Option<RenumberOrder>,
    /// As `frame.cut_list` takes it: mirror-image pieces as one item.
    #[serde(default)]
    pub opposite_hand: Option<bool>,
}

/// `frame.cut_list.set`'s parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CutListSetParams {
    /// The item, by its number in the cut list.
    pub item: usize,
    /// The item's note (a part number, a finish, a drawing reference); empty or null clears it.
    #[serde(default)]
    pub note: Option<String>,
    /// As `frame.cut_list` takes it: mirror-image pieces as one item.
    #[serde(default)]
    pub opposite_hand: Option<bool>,
}

/// `frame.cut_list.renumber`'s and `frame.cut_list.set`'s result.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CutListChangeResult {
    /// The items' numbers now, by grouping key.
    pub numbers: BTreeMap<String, usize>,
}

/// End caps and gussets of one size in the cut list.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CutPlate {
    pub quantity: usize,
    /// `End cap` or `Gusset`.
    pub kind: String,
    /// The plate's thickness (mm).
    pub thickness: f64,
    /// Its bounding box's two larger sides (mm), the larger first.
    pub length: f64,
    pub width: f64,
    pub material: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mass_each_g: Option<f64>,
    pub members: Vec<String>,
}

/// How `frame.cut_list` gives the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CutListFormat {
    Json,
    Csv,
}

/// One end of a cut-list item.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CutEnd {
    /// How far the cut leans from square (degrees; 0: square).
    pub angle: f64,
    /// Which way, as a direction in the profile's own x and y (degrees from +x).
    pub towards: f64,
    /// The end is coped: fitted round another member, not cut flat.
    pub cope: bool,
}

/// The welds of one size in the cut list.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CutWeld {
    /// The fillet's leg (mm).
    pub size: f64,
    /// The length welded (mm): both sides counted, stitches only.
    pub length: f64,
    /// Their mass (g): size² / 2 times the length, times the welded members' density.
    pub mass_g: f64,
}

/// One cut-list item: identical members (same section, lengths and end cuts, either way round).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CutItem {
    /// The number the design keeps for it (`frame.cut_list.renumber`); a new item takes the next
    /// free one, by description, then longest first.
    pub item: usize,
    /// What identifies it: its section, lengths, end cuts and material.
    pub key: String,
    pub quantity: usize,
    /// With `opposite_hand`, how many of `quantity` are the other hand.
    #[serde(skip_serializing_if = "is_zero")]
    pub opposite: usize,
    /// The note the design keeps for it (`frame.cut_list.set`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The profile's description (`Rect Tube 40x40x3`).
    pub description: String,
    /// The longest extent along the member (mm).
    pub length: f64,
    /// The shortest extent along the member (mm), what a saw marks for a mitre.
    pub short_length: f64,
    /// Its two ends, the more leaning first.
    pub ends: [CutEnd; 2],
    /// The members' material.
    pub material: String,
    /// One member's mass (g), when its body is a closed solid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mass_each_g: Option<f64>,
    /// The members, by body name.
    pub members: Vec<String>,
}

/// The total length of one profile, what a shop orders.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CutStock {
    pub description: String,
    pub pieces: usize,
    /// mm, the longest lengths added up.
    pub total_length: f64,
}

/// `frame.cut_list`'s result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CutListResult {
    pub items: Vec<CutItem>,
    pub stock: Vec<CutStock>,
    /// The welds on these members, by size.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub welds: Vec<CutWeld>,
    /// With `plates`, the end caps and gussets.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plates: Vec<CutPlate>,
    /// Every member's mass and every weld's added up (g); members whose mass isn't known are left
    /// out.
    pub total_mass_g: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv: Option<String>,
    /// The file written, with `path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}
