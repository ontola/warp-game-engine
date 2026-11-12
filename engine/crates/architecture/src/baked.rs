//! A house's room bake as a file, made once where there are cores (a
//! workstation, the render server) and shipped next to the model: a browser
//! tab bakes on one thread and would sit for minutes.
//!
//! Layout, little endian: `WBAK`, version u32, grid origin 3×f32, cell f32,
//! dims 3×u32, the sun direction 3×f32, then per probe its six axes
//! (18×f32), its separation plane (4×f32), its DDGI visibility map
//! (`VISIBILITY_SIDE`² u32), its offset out of geometry (u32) and its reach
//! (`REACH_WORDS` u32). The grid and sun are checked on load: a bake of
//! another model or another hour is refused.
use glam::Vec3;
use render::irradiance_volume::{Grid, Probe};
use render::room_bake::{REACH_WORDS, VISIBILITY_SIDE};

const MAGIC: &[u8; 4] = b"WBAK";
const VERSION: u32 = 2;
const MAP: usize = VISIBILITY_SIDE * VISIBILITY_SIDE;
/// Words per probe after the header.
const PER_PROBE: usize = 18 + 4 + MAP + 1 + REACH_WORDS;
const HEADER: usize = 12;

/// Everything `Renderer::set_sky_sun_separated` and
/// `Renderer::set_probe_visibility` take for a house.
pub struct Baked {
    pub probes: Vec<Probe>,
    pub planes: Vec<[f32; 4]>,
    pub visibility: Vec<[u32; MAP]>,
    pub offsets: Vec<u32>,
    pub reach: Vec<[u32; REACH_WORDS]>,
}

impl Baked {
    /// Bake it all (seconds on a many-core CPU).
    pub fn bake(grid: &Grid, occluders: &render::room_bake::Occluders, sun: Vec3, settings: &render::room_bake::Settings) -> Result<Baked, String> {
        use render::room_bake as rb;
        Ok(Baked {
            probes: rb::bake(grid, occluders, sun, settings)?,
            planes: rb::separation_planes(grid, occluders),
            visibility: rb::bake_visibility(grid, occluders),
            offsets: rb::pack_offsets(grid, &rb::probe_positions(grid, occluders)),
            reach: rb::bake_reach(grid, occluders),
        })
    }

    pub fn to_bytes(&self, grid: &Grid, sun: Vec3) -> Vec<u8> {
        let mut w: Vec<u32> = Vec::with_capacity(HEADER + self.probes.len() * PER_PROBE);
        w.push(u32::from_le_bytes(*MAGIC));
        w.push(VERSION);
        w.extend(grid.origin.to_array().map(f32::to_bits));
        w.push(grid.cell.to_bits());
        w.extend(grid.dims);
        w.extend(sun.to_array().map(f32::to_bits));
        for p in &self.probes { for a in p.axes { w.extend(a.to_array().map(f32::to_bits)); } }
        for p in &self.planes { w.extend(p.map(f32::to_bits)); }
        for m in &self.visibility { w.extend(m); }
        w.extend(&self.offsets);
        for r in &self.reach { w.extend(r); }
        w.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    pub fn from_bytes(bytes: &[u8], grid: &Grid, sun: Vec3) -> Result<Baked, String> {
        let count = grid.count();
        if bytes.len() < 8 || &bytes[..4] != MAGIC { return Err("not a room bake".into()); }
        let w: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if w[1] != VERSION { return Err(format!("room bake version {} (this build reads {VERSION})", w[1])); }
        if w.len() != HEADER + count * PER_PROBE { return Err("room bake is for another grid".into()); }
        let f = |i: usize| f32::from_bits(w[i]);
        let origin = Vec3::new(f(2), f(3), f(4));
        if origin.distance(grid.origin) > 1e-4 || (f(5) - grid.cell).abs() > 1e-6 || w[6..9] != grid.dims {
            return Err("room bake is for another model".into());
        }
        if Vec3::new(f(9), f(10), f(11)).distance(sun) > 1e-4 { return Err("room bake is for another sun".into()); }
        let mut at = HEADER;
        let probes = (0..count).map(|p| Probe { axes: std::array::from_fn(|a| { let i = at + p * 18 + a * 3; Vec3::new(f(i), f(i + 1), f(i + 2)) }) }).collect();
        at += count * 18;
        let planes = (0..count).map(|p| std::array::from_fn(|k| f(at + p * 4 + k))).collect();
        at += count * 4;
        let visibility = (0..count).map(|p| std::array::from_fn(|k| w[at + p * MAP + k])).collect();
        at += count * MAP;
        let offsets = w[at..at + count].to_vec();
        at += count;
        let reach = (0..count).map(|p| std::array::from_fn(|k| w[at + p * REACH_WORDS + k])).collect();
        Ok(Baked { probes, planes, visibility, offsets, reach })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_refusals() {
        let grid = Grid { origin: Vec3::new(-1.0, 0.25, 2.0), cell: 0.5, dims: [2, 3, 2] };
        let sun = Vec3::new(0.3, 0.8, -0.5).normalize();
        let n = grid.count();
        let b = Baked {
            probes: (0..n).map(|i| Probe { axes: std::array::from_fn(|a| Vec3::splat((i * 6 + a) as f32)) }).collect(),
            planes: (0..n).map(|i| [i as f32, 1.0, 2.0, 3.0]).collect(),
            visibility: (0..n).map(|i| std::array::from_fn(|k| (i * 100 + k) as u32)).collect(),
            offsets: (0..n as u32).map(|i| i * 7).collect(),
            reach: (0..n).map(|i| std::array::from_fn(|k| (i * 1000 + k) as u32)).collect(),
        };
        let bytes = b.to_bytes(&grid, sun);
        let back = Baked::from_bytes(&bytes, &grid, sun).unwrap();
        assert_eq!(back.probes, b.probes);
        assert_eq!(back.planes, b.planes);
        assert_eq!(back.visibility, b.visibility);
        assert_eq!(back.offsets, b.offsets);
        assert_eq!(back.reach, b.reach);
        assert!(Baked::from_bytes(&bytes, &Grid { cell: 0.75, ..grid }, sun).is_err());
        assert!(Baked::from_bytes(&bytes, &grid, Vec3::Y).is_err());
        assert!(Baked::from_bytes(&bytes[..40], &grid, sun).is_err());
    }
}
