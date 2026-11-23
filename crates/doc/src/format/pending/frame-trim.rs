//! A new feature type, `frame_trim` (Trim/Extend): frame member ends, each a body name and `start`
//! or `end`, trimmed or extended to a boundary (`plane`, a flat `face` or a `tool` body), with an
//! optional `gap` and `extend`. Older designs have none and open unchanged. The last release opens
//! a design with one with a warning about the newer format and can't rebuild the trim.
