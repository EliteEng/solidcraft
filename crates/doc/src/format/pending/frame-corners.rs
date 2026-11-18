//! A frame member feature saves `corners` (what is done where its members meet: `within_group`
//! mitre, butt, cope or none, `gap_within`, `butt_cut`) and `corner_overrides` (junctions' own settings,
//! by junction id). An older design has neither and reads as no treatment, so its members keep
//! their square ends and rebuild as before. The last release opens a design with this change with a
//! warning about the newer format and keeps what it can't read.
