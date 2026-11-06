//! The shade pass's one read-write buffer per view (the engine rule is 8
//! storage buffers per stage, browser.md). Layout, in bytes:
//! 0..16 hidden meadow header (skip count, width, height, ID row pairs);
//! 16..32 sky tile header (indirect x, y, z, where the tile list starts in
//! `words`); 256.. `words`: the certified meadow ID rows when the meadow
//! skips hidden shading (copied there by meadow_visibility.rs), then the
//! list of 8x8 tiles to shade. `ShadeScratch` in shade.wgsl.
pub(crate) const HEADER: u64 = 256;
/// Byte offset of the tile header, copied to the indirect dispatch.
pub(crate) const TILES: u64 = 16;

fn tile_count(size: (u32, u32)) -> u64 { u64::from(size.0.div_ceil(8)) * u64::from(size.1.div_ceil(8)) }

/// Bytes of one meadow ID row, as the texture copy lays it out.
pub(crate) fn id_row_bytes(width: u32) -> u32 { (width * 8).div_ceil(256) * 256 }

fn id_bytes(size: (u32, u32), ids: bool) -> u64 { if ids { u64::from(id_row_bytes(size.0)) * u64::from(size.1) } else { 0 } }

/// Total size for a view, with or without the meadow ID rows.
pub(crate) fn bytes(size: (u32, u32), ids: bool) -> u64 { HEADER + id_bytes(size, ids) + 4 * tile_count(size) }

/// The tile header that starts each frame: an empty list.
pub(crate) fn tile_header(size: (u32, u32), ids: bool) -> [u32; 4] { [0, 1, 1, (id_bytes(size, ids) / 4) as u32] }

/// The meadow header, written once when the buffer is made.
pub(crate) fn meadow_header(size: (u32, u32), ids: bool) -> [u32; 4] {
    if ids { [0, size.0, size.1, id_row_bytes(size.0) / 8] } else { [0; 4] }
}
