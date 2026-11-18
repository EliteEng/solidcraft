//! A new feature type, `frame_weld` (Weld): fillet welds along `edges` (each a body and a point
//! on the edge), with a `size`, `both_sides`, and an optional `stitch_length` and `stitch_pitch`.
//! It changes no body. Older designs have none and open unchanged. The last release opens a
//! design with one with a warning about the newer format and ignores the weld.
