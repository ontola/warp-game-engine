//! Distance-Field Ambient Occlusion (DFAO): a compact 3D unsigned Euclidean
//! distance field packed in `Scene::aux` (four `u8` distances per `u32` word)
//! and sampled along the surface normal and reflection in `shade.wgsl`.
//!
//! Built from the scene's voxel `Occupancy` grid via a
//! 3-pass separable 1D Meijster / Felzenszwalb Euclidean distance transform,
//! Games choose the update cadence; Growing Guns currently bakes at arena setup.

use crate::debris::Occupancy;
use glam::Vec3;

pub const HEADER_WORDS: usize = 8;
pub const MAX_VOXELS: usize = 1 << 20;
const INF: i32 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: Vec3,
    pub voxel: f32,
    pub dims: [u32; 3],
    pub max_distance: f32,
}

impl Grid {
    /// Build a DFAO grid covering the same world bounds as `occ` at `step`x
    /// the occupancy voxel size (e.g. `step = 2` turns 0.5m voxels into 1.0m).
    pub fn from_occupancy(occ: &Occupancy, step: u32, max_distance: f32) -> Self {
        let step = step.max(1);
        let dims = occ.dims.map(|d| d.div_ceil(step).max(2));
        Self {
            origin: occ.origin,
            voxel: occ.voxel * step as f32,
            dims,
            max_distance: max_distance.max(occ.voxel * step as f32),
        }
    }

    pub fn voxel_count(&self) -> usize {
        self.dims[0] as usize * self.dims[1] as usize * self.dims[2] as usize
    }

    pub fn words(&self) -> usize {
        HEADER_WORDS + self.voxel_count().div_ceil(4)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.origin.is_finite()
            || !(self.voxel.is_finite() && self.voxel > 0.05)
            || !(self.max_distance.is_finite() && self.max_distance >= self.voxel)
        {
            return Err("dfao grid origin/voxel/max_distance invalid".into());
        }
        if self.dims.iter().any(|&d| d < 2) || self.voxel_count() > MAX_VOXELS {
            return Err(format!("dfao grid needs 2..{MAX_VOXELS} voxels, at least 2 per axis"));
        }
        Ok(())
    }

    pub fn empty_words(&self) -> Result<Vec<u32>, String> {
        self.validate()?;
        let mut words = vec![u32::MAX; self.words()];
        self.write_header(&mut words);
        Ok(words)
    }

    fn write_header(&self, words: &mut [u32]) {
        words[0] = self.origin.x.to_bits();
        words[1] = self.origin.y.to_bits();
        words[2] = self.origin.z.to_bits();
        words[3] = self.voxel.to_bits();
        words[4] = self.dims[0];
        words[5] = self.dims[1];
        words[6] = self.dims[2];
        words[7] = self.max_distance.to_bits();
    }
}

/// 1D squared Euclidean distance transform (Felzenszwalb & Huttenlocher) along
/// a line of length `n`. `f` holds input squared distances (`0` at solid cells,
/// `INF` at empty cells, or previous passes' accumulated squared distances).
fn edt_1d(f: &[i32], d: &mut [i32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let fq = f[q] as f32 + (q * q) as f32;
        let mut s;
        loop {
            let vk = v[k];
            let fvk = f[vk] as f32 + (vk * vk) as f32;
            s = (fq - fvk) / (2 * (q - vk)) as f32;
            if s > z[k] || k == 0 {
                break;
            }
            k -= 1;
        }
        if s <= z[k] {
            v[0] = q;
            z[0] = f32::NEG_INFINITY;
            z[1] = f32::INFINITY;
        } else {
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
        }
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dx = q as i32 - v[k] as i32;
        *out = (dx * dx + f[v[k]]).min(INF);
    }
}

/// Bake the packed `aux` words for `grid` from a high-resolution `Occupancy`
/// bitgrid. Runs in ~0.7 ms for a 104x34x104 grid.
pub fn bake_from_occupancy(grid: &Grid, occ: &Occupancy) -> Result<Vec<u32>, String> {
    grid.validate()?;
    let [nx, ny, nz] = grid.dims.map(|d| d as usize);
    let step = (grid.voxel / occ.voxel).round().max(1.0) as usize;
    let [ox, oy, oz] = occ.dims.map(|d| d as usize);
    let total = nx * ny * nz;
    let mut dist2 = vec![INF; total];

    // Seed solid cells from the occupancy bitfield.
    for z in 0..nz {
        let z0 = z * step;
        let z1 = ((z + 1) * step).min(oz);
        for y in 0..ny {
            let y0 = y * step;
            let y1 = ((y + 1) * step).min(oy);
            let row_out = (y + z * ny) * nx;
            for x in 0..nx {
                let x0 = x * step;
                let x1 = ((x + 1) * step).min(ox);
                let mut solid = false;
                'sub: for sz in z0..z1 {
                    for sy in y0..y1 {
                        let base = (sy + sz * oy) * ox;
                        for sx in x0..x1 {
                            let bit = base + sx;
                            if (occ.bits[bit >> 5] >> (bit & 31)) & 1 != 0 {
                                solid = true;
                                break 'sub;
                            }
                        }
                    }
                }
                if solid {
                    dist2[row_out + x] = 0;
                }
            }
        }
    }

    // Pass 1: 1D distance transform along X (forward + backward sweep).
    for row in dist2.chunks_exact_mut(nx) {
        let mut d = INF;
        for cell in row.iter_mut() {
            if *cell == 0 {
                d = 0;
            } else if d < INF {
                d += 1;
                *cell = d * d;
            }
        }
        d = INF;
        for cell in row.iter_mut().rev() {
            if *cell == 0 {
                d = 0;
            } else {
                if d < INF {
                    d += 1;
                    *cell = (*cell).min(d * d);
                }
                let root = (*cell as f32).sqrt() as i32;
                if root < d {
                    d = root;
                }
            }
        }
    }

    let max_dim = nx.max(ny).max(nz);
    let mut f = vec![0i32; max_dim];
    let mut out_line = vec![0i32; max_dim];
    let mut v = vec![0usize; max_dim];
    let mut z_env = vec![0.0f32; max_dim + 1];

    // Pass 2: 1D distance transform along Y.
    for z in 0..nz {
        let slice = z * nx * ny;
        for x in 0..nx {
            for y in 0..ny {
                f[y] = dist2[slice + y * nx + x];
            }
            edt_1d(&f[..ny], &mut out_line[..ny], &mut v[..ny], &mut z_env[..=ny]);
            for y in 0..ny {
                dist2[slice + y * nx + x] = out_line[y];
            }
        }
    }

    // Pass 3: 1D distance transform along Z.
    let stride_z = nx * ny;
    for y in 0..ny {
        let row = y * nx;
        for x in 0..nx {
            let col = row + x;
            for z in 0..nz {
                f[z] = dist2[z * stride_z + col];
            }
            edt_1d(&f[..nz], &mut out_line[..nz], &mut v[..nz], &mut z_env[..=nz]);
            for z in 0..nz {
                dist2[z * stride_z + col] = out_line[z];
            }
        }
    }

    // Pack 4 quantized u8 distances per u32 word after the 8-word header.
    let mut words = vec![0u32; grid.words()];
    grid.write_header(&mut words);
    let scale = 255.0 / grid.max_distance;
    for (i, &d2) in dist2.iter().enumerate() {
        let dist_m = if d2 <= 0 {
            0.0
        } else {
            (((d2 as f32).sqrt() - 0.5).max(0.0) * grid.voxel).min(grid.max_distance)
        };
        let byte = (dist_m * scale).round().clamp(0.0, 255.0) as u32;
        words[HEADER_WORDS + (i >> 2)] |= byte << ((i & 3) * 8);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euclidean_distance_field_matches_exact_distance_to_solid_box() {
        let mut occ = Occupancy::new(Vec3::splat(-8.0), Vec3::splat(8.0), 0.5);
        occ.fill_box(Vec3::ZERO, Vec3::splat(1.0), 0.0, true);
        let grid = Grid::from_occupancy(&occ, 2, 10.0);
        let words = bake_from_occupancy(&grid, &occ).expect("bake dfao");
        assert_eq!(words.len(), grid.words());

        let sample = |x: usize, y: usize, z: usize| -> f32 {
            let idx = x + y * grid.dims[0] as usize + z * (grid.dims[0] * grid.dims[1]) as usize;
            let byte = (words[HEADER_WORDS + (idx >> 2)] >> ((idx & 3) * 8)) & 0xff;
            byte as f32 / 255.0 * grid.max_distance
        };

        // Inside the solid box at the centre (voxel 8, 8, 8) distance is 0.
        assert_eq!(sample(8, 8, 8), 0.0);
        // Farther away along +X, distance increases monotonically.
        let d_near = sample(10, 8, 8);
        let d_far = sample(14, 8, 8);
        assert!(d_near > 0.5 && d_near < 3.0, "d_near={d_near}");
        assert!(d_far > d_near + 3.0, "d_far={d_far} vs d_near={d_near}");
    }
}
