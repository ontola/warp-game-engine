//! Bullet dents in ROCK surfaces: small craters the shade pass carves into
//! the stone's normal and colour where a shot hit, without touching the
//! mesh. A scene reserves room with `Scene::reserve_dents`; the game hands
//! the live dents to `Renderer::set_dents` whenever they change.
//!
//! Layout in `aux` at the reserved offset (all u32 words):
//! `[buckets, capacity, index capacity, cell size (f32 bits)]`, then one
//! word per bucket (`start << 8 | count`) into the index list, the index
//! list itself, then `capacity` dents of `[x, y, z, radius]` as f32 bits.
//! A dent is listed in every grid cell its bounding box touches, so the
//! shader reads only the bucket of the pixel's own cell.

/// Hash buckets (a power of two).
pub const BUCKETS: usize = 4096;
/// Grid cell edge in metres; dents are at most half of this in radius.
pub const CELL: f32 = 1.0;
/// Most dents the shader looks at per pixel.
pub const MAX_PER_CELL: usize = 15;
const HEADER: usize = 4;

/// A dent: centre on the surface it hit, and its radius in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dent { pub pos: [f32; 3], pub radius: f32 }

/// Words of the aux region for `capacity` dents.
pub fn words(capacity: usize) -> usize { HEADER + BUCKETS + capacity * 8 + capacity * 4 }

fn bucket(c: [i32; 3]) -> usize {
    let h = (c[0] as u32).wrapping_mul(73856093) ^ (c[1] as u32).wrapping_mul(19349663) ^ (c[2] as u32).wrapping_mul(83492791);
    (h as usize) & (BUCKETS - 1)
}

fn cell(v: f32) -> i32 { (v / CELL).floor() as i32 }

/// The region for `dents` (the first `capacity` of them).
pub fn pack(capacity: usize, dents: &[Dent]) -> Vec<u32> {
    let dents = &dents[..dents.len().min(capacity)];
    let index_cap = capacity * 8;
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); BUCKETS];
    for (i, d) in dents.iter().enumerate() {
        let r = d.radius.clamp(0.0, CELL * 0.5);
        let lo = [cell(d.pos[0] - r), cell(d.pos[1] - r), cell(d.pos[2] - r)];
        let hi = [cell(d.pos[0] + r), cell(d.pos[1] + r), cell(d.pos[2] + r)];
        for x in lo[0]..=hi[0] { for y in lo[1]..=hi[1] { for z in lo[2]..=hi[2] {
            let b = &mut lists[bucket([x, y, z])];
            if !b.contains(&(i as u32)) { b.push(i as u32); }
        }}}
    }
    let mut out = vec![0u32; words(capacity)];
    out[0] = BUCKETS as u32;
    out[1] = capacity as u32;
    out[2] = index_cap as u32;
    out[3] = CELL.to_bits();
    let mut next = 0usize;
    for (b, list) in lists.iter().enumerate() {
        // Newest dents last: keep those when a bucket overflows.
        let keep = &list[list.len().saturating_sub(MAX_PER_CELL)..];
        if keep.is_empty() || next + keep.len() > index_cap { continue; }
        out[HEADER + b] = ((next as u32) << 8) | keep.len() as u32;
        out[HEADER + BUCKETS + next..HEADER + BUCKETS + next + keep.len()].copy_from_slice(keep);
        next += keep.len();
    }
    let base = HEADER + BUCKETS + index_cap;
    for (i, d) in dents.iter().enumerate() {
        let w = &mut out[base + i * 4..base + i * 4 + 4];
        w.copy_from_slice(&[d.pos[0].to_bits(), d.pos[1].to_bits(), d.pos[2].to_bits(), d.radius.min(CELL * 0.5).to_bits()]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dent_is_in_the_bucket_of_its_cells() {
        let dents = [Dent { pos: [0.95, 2.0, -0.05], radius: 0.2 }, Dent { pos: [10.0, 1.0, 3.0], radius: 0.1 }];
        let w = pack(16, &dents);
        assert_eq!(w.len(), words(16));
        let listed = |p: [f32; 3]| {
            let e = w[HEADER + bucket([cell(p[0]), cell(p[1]), cell(p[2])])];
            let (start, count) = ((e >> 8) as usize, (e & 255) as usize);
            w[HEADER + BUCKETS + start..HEADER + BUCKETS + start + count].to_vec()
        };
        // The first dent straddles x = 1 and z = 0: both sides list it.
        assert!(listed([0.9, 2.0, -0.1]).contains(&0) && listed([1.1, 2.0, 0.1]).contains(&0));
        assert!(listed([10.0, 1.0, 3.0]).contains(&1));
        let base = HEADER + BUCKETS + 16 * 8;
        assert_eq!(f32::from_bits(w[base + 4 + 3]), 0.1);
    }
}
