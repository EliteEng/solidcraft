//! A frame member's vertical members in a flat sketch now face out of the sketch, as its
//! horizontal members do, so a frame drawn on an upright plane has its posts and rails facing the
//! same way; before, a vertical member's profile x was always world X, which turned the posts of
//! a frame drawn on the Front plane a quarter turn against its rails. A feature saves
//! `posts_world_x: true` to keep the old rule. Older designs are rewritten as they open: every
//! frame member gets `posts_world_x: true`, so it rebuilds as it was saved. The last release
//! opens a design with this change with a warning about the newer format, and its frame members
//! keep world-X posts.
use serde_json::Value;

pub fn step(v: &mut Value) -> crate::Result<()> {
    let Some(Value::Array(features)) = v.get_mut("features") else {
        return Ok(());
    };
    for feature in features {
        let Some(object) = feature.as_object_mut() else { continue };
        if object.get("type").and_then(Value::as_str) == Some("frame_member") {
            object.insert("posts_world_x".into(), Value::Bool(true));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn older_frame_members_keep_world_x_posts() {
        let mut v = json!({"features": [
            {"type": "frame_member", "path_sketch": 1, "profile": {"source": "library"}},
            {"type": "extrude", "posts_world_x": false}
        ]});
        super::step(&mut v).unwrap();
        assert_eq!(v["features"][0]["posts_world_x"], json!(true));
        assert_eq!(v["features"][1]["posts_world_x"], json!(false), "only frame members");
        let mut empty = json!({"name": "Untitled"});
        super::step(&mut empty).unwrap();
        assert_eq!(empty, json!({"name": "Untitled"}));
    }
}
