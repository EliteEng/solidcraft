//! The cut list of frame members (`frame.cut_list`), with item numbers and notes the design keeps
//! (`frame.cut_list.renumber`, `frame.cut_list.set`). Each member is measured on its body as
//! built, so corners, trims and later cuts are in it.

use std::collections::BTreeMap;

use serde_json::Value;
use solidcraft_doc::FeatureKind;
use solidcraft_doc::frame;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::cut_list::{
    CutEnd, CutItem, CutListChangeResult, CutListFormat, CutListParams, CutListRenumberParams, CutListResult, CutListSetParams, CutPlate, CutStock,
    CutWeld, RenumberOrder,
};
use crate::schema::weldment::typed;
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

#[cfg(test)]
#[path = "cut_list_tests.rs"]
mod tests;

const CUT_LIST_CONTRACT: TypedContract<CutListParams> = TypedContract::new();
const RENUMBER_CONTRACT: TypedContract<CutListRenumberParams> = TypedContract::new();
const CUT_SET_CONTRACT: TypedContract<CutListSetParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("frame.cut_list", "Cut List", crate::checked!("frame.cut_list", cut_list))
        .at("WELDMENTS", "INSPECT")
        .icon("bom")
        .noundo()
        .doc_only()
        .contract(&CUT_LIST_CONTRACT)
        .output(output_schema_for::<CutListResult>)
        .params("feature?: a frame member feature (id or name; default every one); format?: json|csv (csv adds the list as text); path?: also write it to a .csv or .json file; opposite_hand?: bool (mirror-image pieces as one item, `opposite` of them the other hand); plates?: bool (also the end caps and gussets: plates [{quantity, kind, thickness, length, width, material, mass_each_g?, members}]). Returns items: [{item (kept in the design: frame.cut_list.renumber), key, quantity, opposite?, note? (frame.cut_list.set), description, length (longest, mm), short_length (mm), ends: [{angle (deg from square), towards (deg in the profile's x/y)}, …2], material, mass_each_g?, members: [body names]}] (identical members together, either way round), stock: [{description, pieces, total_length}], total_mass_g, csv?, path?"),
    CommandSpec::new("frame.cut_list.renumber", "Renumber Cut List", crate::checked!("frame.cut_list.renumber", cut_list_renumber))
        .icon("bom")
        .contract(&RENUMBER_CONTRACT)
        .output(output_schema_for::<CutListChangeResult>)
        .params("order?: profile|length|current (default profile: by profile, then longest first; length: longest first; current: keep the numbers shown now); opposite_hand?: bool (as frame.cut_list). Keeps every item's number in the design, so later members take new numbers and these keep theirs. Returns numbers: {grouping key: number}"),
    CommandSpec::new("frame.cut_list.set", "Cut List Note", crate::checked!("frame.cut_list.set", cut_list_set))
        .icon("bom")
        .contract(&CUT_SET_CONTRACT)
        .output(output_schema_for::<CutListChangeResult>)
        .params("item: its number in the cut list; note?: text (a part number, a finish…; empty or null clears it); opposite_hand?: bool (as frame.cut_list). Keeps the note, and the item's number, in the design by the item's grouping key. Returns numbers"),
];

fn frame_reply(cmd: &str, r: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(r).map_err(|e| bad(cmd, e.to_string()))
}

/// Most members a cut list takes.
const MAX_CUT_MEMBERS: usize = 20_000;

/// The cut list of frame member feature `feature` (none: every one), numbered as the design keeps
/// it; `opposite` lists mirror-image pieces together, `plates` adds the end caps and gussets.
fn build_cut_list(s: &Session, cmd: &str, feature: Option<&crate::schema::edit::FeatureRef>, opposite: bool, plates: bool) -> Result<CutListResult> {
    let features: Vec<u64> = match feature {
        Some(f) => {
            let id = super::edit::feature_id(s, Some(&f.0), cmd)?;
            match s.doc.feature(id) {
                Some(f) if matches!(f.kind, FeatureKind::FrameMember { .. }) => vec![id],
                Some(f) => return Err(bad(cmd, format!("{} is not a frame member feature", f.name))),
                None => return Err(bad(cmd, format!("no feature {id}"))),
            }
        }
        None => s.doc.features.iter().filter(|f| !f.suppressed && matches!(f.kind, FeatureKind::FrameMember { .. })).map(|f| f.id).collect(),
    };
    let st = s.model.state();
    // Each member: its cut, body name, material and mass.
    let mut members: Vec<(frame::MemberCut, String, String, Option<f64>)> = Vec::new();
    for &id in &features {
        let Some(f) = s.doc.feature(id) else { continue };
        let cuts = frame::member_cuts_report(&s.doc, &st, f).map_err(|e| bad(cmd, format!("{}: {e}", f.name)))?;
        for c in cuts {
            let Some(body) = st.body(&c.body) else { continue };
            let material = s.doc.body_material(&body.name);
            let mass = body
                .measure()
                .ok()
                .filter(|_| super::inspect::encloses_volume(&body.body))
                .map(|m| m.volume * s.doc.density(&body.name) / 1000.0)
                .filter(|m| m.is_finite());
            members.push((c, body.name.clone(), material, mass));
            if members.len() > MAX_CUT_MEMBERS {
                return Err(bad(cmd, format!("at most {MAX_CUT_MEMBERS} members in a cut list")));
            }
        }
    }
    // Identical members share a key: the section, both lengths and the two ends either way round.
    // Rounded to what the list shows past its last digit, with −0 and 360° read as 0.
    let rounded = |v: f64, places: i32| {
        let k = 10f64.powi(places);
        (v * k).round() / k + 0.0
    };
    let end_key = |e: &[frame::EndCut], cope: bool| {
        let cut =
            e.first().map_or_else(|| "square".to_string(), |c| format!("{}@{}", rounded(c.angle, 4), rounded(c.towards, 3).rem_euclid(360.0) + 0.0));
        if cope { format!("cope {cut}") } else { cut }
    };
    let mut items: Vec<CutItem> = Vec::new();
    // With `opposite`, mirror-image pieces share this key: the section without its mirror, and
    // each end's lean folded onto one side of the profile's y axis.
    let mut unhanded: Vec<String> = Vec::new();
    for (c, name, material, mass) in members {
        let mut ends = [end_key(&c.ends[0], c.coped[0]), end_key(&c.ends[1], c.coped[1])];
        ends.sort();
        let key = format!("{}|{}|{}|{}|{}|{material}", c.profile_key, rounded(c.length, 4), rounded(c.short_length, 4), ends[0], ends[1]);
        let fold = |e: &[frame::EndCut], cope: bool| {
            let t = e.first().map_or(0.0, |c| c.towards);
            let folded: Vec<frame::EndCut> =
                e.iter().take(1).map(|c| frame::EndCut { angle: c.angle, towards: t.min((180.0 - t).rem_euclid(360.0)) }).collect();
            end_key(&folded, cope)
        };
        let mut plain = [fold(&c.ends[0], c.coped[0]), fold(&c.ends[1], c.coped[1])];
        plain.sort();
        let base = c.profile_key.trim_end_matches("|mirrored");
        let hand_key = format!("{base}|{}|{}|{}|{}|{material}", rounded(c.length, 4), rounded(c.short_length, 4), plain[0], plain[1]);
        if let Some(item) = items.iter_mut().find(|x| x.key == key) {
            item.quantity += 1;
            item.members.push(name);
            continue;
        }
        if opposite
            && let Some(k) = unhanded.iter().position(|x| *x == hand_key)
            && let Some(item) = items.get_mut(k)
        {
            item.quantity += 1;
            item.opposite += 1;
            item.members.push(name);
            continue;
        }
        let end = |e: &[frame::EndCut], cope: bool| {
            e.first().map_or(CutEnd { angle: 0.0, towards: 0.0, cope }, |c| CutEnd { angle: c.angle, towards: c.towards, cope })
        };
        let mut pair = [end(&c.ends[0], c.coped[0]), end(&c.ends[1], c.coped[1])];
        if (pair[1].cope, pair[1].angle) > (pair[0].cope, pair[0].angle) {
            pair.swap(0, 1);
        }
        unhanded.push(hand_key);
        let note = s.doc.cut_list.descriptions.get(&key).cloned();
        items.push(CutItem {
            item: 0,
            key,
            quantity: 1,
            opposite: 0,
            note,
            description: c.description,
            length: c.length,
            short_length: c.short_length,
            ends: pair,
            material,
            mass_each_g: mass,
            members: vec![name],
        });
    }
    items.sort_by(|x, y| x.description.cmp(&y.description).then(y.length.total_cmp(&x.length)));
    // Kept numbers first; a new item takes the next free number after them, in list order.
    let kept = &s.doc.cut_list.numbers;
    let mut used: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for item in &mut items {
        if let Some(&n) = kept.get(&item.key).filter(|n| **n > 0 && !used.contains(*n)) {
            item.item = n;
            used.insert(n);
        }
    }
    let mut next = kept.values().copied().max().unwrap_or(0);
    for item in items.iter_mut().filter(|i| i.item == 0) {
        next += 1;
        while used.contains(&next) {
            next += 1;
        }
        item.item = next;
        used.insert(next);
    }
    items.sort_by_key(|i| i.item);
    let mut stock: Vec<CutStock> = Vec::new();
    for item in &items {
        match stock.iter_mut().find(|s| s.description == item.description) {
            Some(s) => {
                s.pieces += item.quantity;
                s.total_length += item.length * item.quantity as f64;
            }
            None => stock.push(CutStock {
                description: item.description.clone(),
                pieces: item.quantity,
                total_length: item.length * item.quantity as f64,
            }),
        }
    }
    // The welds on these features' members, by size.
    let mut welds: Vec<CutWeld> = Vec::new();
    for w in frame::welds(&s.doc, &st) {
        let on_listed = w.bodies.iter().any(|b| st.body(b).is_some_and(|mb| features.contains(&mb.feature)));
        if !(on_listed || feature.is_none()) {
            continue;
        }
        let density = w.bodies.first().map_or(0.0, |b| s.doc.density(b));
        let mass = w.size * w.size / 2.0 * w.welded * density / 1000.0;
        match welds.iter_mut().find(|x| rounded(x.size, 4) == rounded(w.size, 4)) {
            Some(x) => {
                x.length += w.welded;
                x.mass_g += mass;
            }
            None => welds.push(CutWeld { size: w.size, length: w.welded, mass_g: mass }),
        }
    }
    welds.sort_by(|x, y| x.size.total_cmp(&y.size));
    let plates = if plates { cut_plates(s, &st, &features, feature.is_none()) } else { Vec::new() };
    let total_mass_g = items.iter().filter_map(|i| i.mass_each_g.map(|m| m * i.quantity as f64)).sum::<f64>()
        + welds.iter().map(|w| w.mass_g).sum::<f64>()
        + plates.iter().filter_map(|p| p.mass_each_g.map(|m| m * p.quantity as f64)).sum::<f64>();
    Ok(CutListResult { items, stock, welds, plates, total_mass_g, csv: None, path: None })
}

/// The end caps and gussets on the members of `features` (every one when `all`), alike ones
/// together: their thickness and bounding box (exact), material and mass.
fn cut_plates(s: &Session, st: &solidcraft_doc::ModelState, features: &[u64], all: bool) -> Vec<CutPlate> {
    let rounded = |v: f64| (v * 100.0).round() / 100.0 + 0.0;
    let mut out: Vec<CutPlate> = Vec::new();
    for b in &st.bodies {
        let Some(f) = s.doc.feature(b.feature).filter(|f| !f.suppressed) else { continue };
        let (kind, thickness, listed) = match &f.kind {
            // A cap records its members by feature and line.
            FeatureKind::FrameEndCap { thickness, ends, .. } => {
                ("End cap", thickness, all || ends.iter().any(|e| features.contains(&e.member.feature.0)))
            }
            // A gusset and a plate record the members they rest on by feature.
            FeatureKind::FrameGusset { thickness, supports, .. } => {
                ("Gusset", thickness, all || supports.iter().any(|sp| sp.member.as_ref().is_some_and(|m| features.contains(&m.feature.0))))
            }
            FeatureKind::FramePlate { thickness, member, .. } => {
                ("Plate", thickness, all || member.as_ref().is_some_and(|m| features.contains(&m.feature.0)))
            }
            _ => continue,
        };
        let (Ok(t), Ok(bb)) = (
            s.doc.eval(&s.doc.with_design_unit(thickness, solidcraft_doc::expr::Kind::Length), solidcraft_doc::expr::Kind::Length),
            b.body.exact_bounds(),
        ) else {
            continue;
        };
        if !listed {
            continue;
        }
        let mut sides = [bb.max.x - bb.min.x, bb.max.y - bb.min.y, bb.max.z - bb.min.z];
        sides.sort_by(|x, y| y.total_cmp(x));
        let material = s.doc.body_material(&b.name);
        let mass = b.measure().ok().map(|m| m.volume * s.doc.density(&b.name) / 1000.0).filter(|m| m.is_finite());
        let same = |p: &CutPlate| {
            p.kind == kind
                && rounded(p.thickness) == rounded(t)
                && rounded(p.length) == rounded(sides[0])
                && rounded(p.width) == rounded(sides[1])
                && p.material == material
        };
        match out.iter_mut().find(|p| same(p)) {
            Some(p) => {
                p.quantity += 1;
                p.members.push(b.name.clone());
            }
            None => out.push(CutPlate {
                quantity: 1,
                kind: kind.into(),
                thickness: t,
                length: sides[0],
                width: sides[1],
                material,
                mass_each_g: mass,
                members: vec![b.name.clone()],
            }),
        }
    }
    out
}

/// Keep the cut list's item numbers in the design, in `order`.
fn cut_list_renumber(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.cut_list.renumber";
    let a: CutListRenumberParams = typed(cmd, p)?;
    let list = build_cut_list(s, cmd, None, a.opposite_hand.unwrap_or(false), false)?;
    let mut items = list.items;
    match a.order.unwrap_or(RenumberOrder::Profile) {
        RenumberOrder::Current => {}
        RenumberOrder::Profile => items.sort_by(|x, y| x.description.cmp(&y.description).then(y.length.total_cmp(&x.length))),
        RenumberOrder::Length => items.sort_by(|x, y| y.length.total_cmp(&x.length).then(x.description.cmp(&y.description))),
    }
    let numbers: BTreeMap<String, usize> = match a.order.unwrap_or(RenumberOrder::Profile) {
        RenumberOrder::Current => items.iter().map(|i| (i.key.clone(), i.item)).collect(),
        _ => items.iter().enumerate().map(|(k, i)| (i.key.clone(), k + 1)).collect(),
    };
    s.doc_mut().cut_list.numbers = numbers.clone();
    frame_reply(cmd, CutListChangeResult { numbers })
}

/// Keep a note for one cut-list item in the design.
fn cut_list_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.cut_list.set";
    let a: CutListSetParams = typed(cmd, p)?;
    let list = build_cut_list(s, cmd, None, a.opposite_hand.unwrap_or(false), false)?;
    let item = list.items.iter().find(|i| i.item == a.item).ok_or_else(|| bad(cmd, format!("the cut list has no item {}", a.item)))?;
    let note = a.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note.as_ref().is_some_and(|n| n.chars().count() > 1000) {
        return Err(bad(cmd, "`note` holds at most 1000 characters"));
    }
    let (key, numbers) = (item.key.clone(), list.items.iter().map(|i| (i.key.clone(), i.item)).collect::<BTreeMap<_, _>>());
    let d = s.doc_mut();
    match note {
        Some(n) => d.cut_list.descriptions.insert(key, n),
        None => d.cut_list.descriptions.remove(&key),
    };
    // Its number is kept too, so the note and the number stay together.
    d.cut_list.numbers.extend(numbers.clone());
    frame_reply(cmd, CutListChangeResult { numbers })
}

fn cut_list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "frame.cut_list";
    let a: CutListParams = typed(cmd, p)?;
    let mut out = build_cut_list(s, cmd, a.feature.as_ref(), a.opposite_hand.unwrap_or(false), a.plates.unwrap_or(false))?;
    if a.format == Some(CutListFormat::Csv) && a.path.is_none() {
        out.csv = Some(cut_list_csv(&out.items, &out.welds, &out.plates));
    }
    if let Some(path) = a.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        let lower = path.to_ascii_lowercase();
        let as_csv = match a.format {
            Some(f) => f == CutListFormat::Csv,
            None if lower.ends_with(".csv") => true,
            None if lower.ends_with(".json") => false,
            None => return Err(bad(cmd, "`path` must end in .csv or .json, or give `format`")),
        };
        let text = if as_csv {
            cut_list_csv(&out.items, &out.welds, &out.plates)
        } else {
            serde_json::to_string_pretty(&out).map_err(|e| bad(cmd, e.to_string()))?
        };
        solidcraft_io::vfs::write(path, text.as_bytes()).map_err(|e| crate::EngineError::Other(format!("{path}: {e}")))?;
        out.path = Some(path.to_string());
    }
    frame_reply(cmd, out)
}

/// How a cut-list end reads in the CSV: `cope`, `angled` or `square`.
fn end_kind(e: &CutEnd) -> &'static str {
    if e.cope {
        "cope"
    } else if e.angle > 0.0 {
        "angled"
    } else {
        "square"
    }
}

/// The cut list as comma-separated text, a header line first; lengths in mm, angles in degrees,
/// masses in kg. The welds follow under a header of their own.
fn cut_list_csv(items: &[CutItem], welds: &[CutWeld], plates: &[CutPlate]) -> String {
    let field = |t: &str| if t.contains([',', '"', '\n']) { format!("\"{}\"", t.replace('"', "\"\"")) } else { t.to_string() };
    let mass = |m: Option<f64>| m.map_or_else(String::new, |m| format!("{:.3}", m / 1000.0));
    let mut out = String::from(
        "Item,Qty,Opposite hand,Description,Length (mm),Short length (mm),Angle 1 (deg),Angle 2 (deg),End 1,End 2,Material,Mass each (kg),Total mass (kg),Note,Members\n",
    );
    for i in items {
        out.push_str(&format!(
            "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{},{},{},{},{},{},{}\n",
            i.item,
            i.quantity,
            i.opposite,
            field(&i.description),
            i.length,
            i.short_length,
            i.ends[0].angle,
            i.ends[1].angle,
            end_kind(&i.ends[0]),
            end_kind(&i.ends[1]),
            field(&i.material),
            mass(i.mass_each_g),
            mass(i.mass_each_g.map(|m| m * i.quantity as f64)),
            field(i.note.as_deref().unwrap_or_default()),
            field(&i.members.join(" ")),
        ));
    }
    if !plates.is_empty() {
        out.push_str("\nPlate,Qty,Thickness (mm),Length (mm),Width (mm),Material,Mass each (kg),Members\n");
        for p in plates {
            out.push_str(&format!(
                "{},{},{:.2},{:.2},{:.2},{},{},{}\n",
                field(&p.kind),
                p.quantity,
                p.thickness,
                p.length,
                p.width,
                field(&p.material),
                mass(p.mass_each_g),
                field(&p.members.join(" "))
            ));
        }
    }
    if !welds.is_empty() {
        out.push_str("\nWeld size (mm),Length welded (mm),Mass (kg)\n");
        for w in welds {
            out.push_str(&format!("{:.2},{:.2},{:.3}\n", w.size, w.length, w.mass_g / 1000.0));
        }
    }
    out
}
