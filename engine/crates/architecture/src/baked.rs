//! A house's room bake as a file, made once where there are cores (a
//! workstation, the render server) and shipped next to the model: a browser
//! tab bakes on one thread and would sit for minutes.
//!
//! Layout, little endian: `WBAK`, version u32, grid origin 3×f32, cell f32,
//! dims 3×u32, the sun direction 3×f32, then per probe its six axes
//! (18×f32), then per probe its separation plane (4×f32). The grid and sun
//! are checked on load: a bake of another model or another hour is refused.
use glam::Vec3;
use render::irradiance_volume::{Grid, Probe};

const MAGIC: &[u8; 4] = b"WBAK";
const VERSION: u32 = 1;

pub struct Baked {
    pub probes: Vec<Probe>,
    pub planes: Vec<[f32; 4]>,
}

pub fn to_bytes(grid: &Grid, sun: Vec3, probes: &[Probe], planes: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48 + probes.len() * 88);
    out.extend_from_slice(MAGIC);
    let mut u = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    u(VERSION);
    let head: Vec<f32> = [grid.origin.to_array().as_slice(), &[grid.cell]].concat();
    let mut f = |out: &mut Vec<u8>, v: f32| out.extend_from_slice(&v.to_le_bytes());
    for v in head { f(&mut out, v); }
    for d in grid.dims { out.extend_from_slice(&d.to_le_bytes()); }
    for v in sun.to_array() { f(&mut out, v); }
    for p in probes { for a in p.axes { for v in a.to_array() { f(&mut out, v); } } }
    for p in planes { for v in p { f(&mut out, *v); } }
    out
}

pub fn from_bytes(bytes: &[u8], grid: &Grid, sun: Vec3) -> Result<Baked, String> {
    let count = grid.count();
    if bytes.len() != 48 + count * 88 || &bytes[..4] != MAGIC {
        return Err("not a room bake for this grid".into());
    }
    let word = |i: usize| <[u8; 4]>::try_from(&bytes[4 + i * 4..8 + i * 4]).unwrap();
    let f = |i: usize| f32::from_le_bytes(word(i));
    if u32::from_le_bytes(word(0)) != VERSION { return Err("room bake version differs".into()); }
    let origin = Vec3::new(f(1), f(2), f(3));
    let dims = [1, 2, 3].map(|k| u32::from_le_bytes(word(4 + k)));
    if origin.distance(grid.origin) > 1e-4 || (f(4) - grid.cell).abs() > 1e-6 || dims != grid.dims {
        return Err("room bake is for another model".into());
    }
    if Vec3::new(f(8), f(9), f(10)).distance(sun) > 1e-4 { return Err("room bake is for another sun".into()); }
    let at = 11;
    let probes = (0..count).map(|p| Probe { axes: std::array::from_fn(|a| {
        let i = at + p * 18 + a * 3;
        Vec3::new(f(i), f(i + 1), f(i + 2))
    }) }).collect();
    let at = at + count * 18;
    let planes = (0..count).map(|p| std::array::from_fn(|k| f(at + p * 4 + k))).collect();
    Ok(Baked { probes, planes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_refusals() {
        let grid = Grid { origin: Vec3::new(-1.0, 0.25, 2.0), cell: 0.5, dims: [2, 3, 2] };
        let sun = Vec3::new(0.3, 0.8, -0.5).normalize();
        let probes: Vec<Probe> = (0..12).map(|i| Probe { axes: std::array::from_fn(|a| Vec3::splat((i * 6 + a) as f32)) }).collect();
        let planes: Vec<[f32; 4]> = (0..12).map(|i| [i as f32, 1.0, 2.0, 3.0]).collect();
        let bytes = to_bytes(&grid, sun, &probes, &planes);
        let back = from_bytes(&bytes, &grid, sun).unwrap();
        assert_eq!(back.probes, probes);
        assert_eq!(back.planes, planes);
        assert!(from_bytes(&bytes, &Grid { cell: 0.75, ..grid }, sun).is_err());
        assert!(from_bytes(&bytes, &grid, Vec3::Y).is_err());
        assert!(from_bytes(&bytes[..40], &grid, sun).is_err());
    }
}
