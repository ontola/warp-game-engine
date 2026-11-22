//! Bake a game's horizontal lawns and paths into the native GPU meadow terrain field.
//! Geometry, root selection, lighting, wind and distance LOD live in the renderer.
use glam::{Vec2, Vec3};
use render::meadow_data::{MAX_FIELD_SAMPLES, MeadowField, MeadowGeometry};

const CELL: f32 = 0.5;
// The meadow renderer measures road clearance from the road's outer shoulder.
// An authored cutout already describes its finished edge.
const SHOULDER: f32 = 0.9;

pub(crate) struct Rect {
    center: Vec2,
    height: f32,
    half: Vec2,
    axis: Vec2,
}

impl Rect {
    pub(crate) fn new(pos: Vec3, width: f32, depth: f32, yaw: f32) -> Result<Self, String> {
        if !pos.is_finite() || !yaw.is_finite() || !width.is_finite() || !depth.is_finite()
            || width <= 0. || depth <= 0. {
            return Err("grass rectangle needs finite coordinates and positive width and depth".into());
        }
        let (sin, cos) = yaw.sin_cos();
        Ok(Self { center: Vec2::new(pos.x, pos.z), height: pos.y, half: Vec2::new(width, depth) * 0.5,
            axis: Vec2::new(cos, -sin) })
    }

    // Positive inside, negative outside, in world metres even for diagonal paths.
    fn clearance(&self, point: Vec2) -> f32 {
        let p = point - self.center;
        let q = Vec2::new(self.axis.dot(p), self.axis.perp_dot(p)).abs() - self.half;
        -(q.max(Vec2::ZERO).length() + q.max_element().min(0.))
    }

    fn bounds(&self) -> (Vec2, Vec2) {
        let a = self.axis.abs();
        let span = a * self.half.x + Vec2::new(a.y, a.x) * self.half.y;
        (self.center - span, self.center + span)
    }
}

pub(crate) struct Lawns {
    pub patches: Vec<Rect>,
    pub cutouts: Vec<Rect>,
    pub seed: u32,
    pub growth: f32,
    pub distance: f32,
}

impl Lawns {
    pub(crate) fn build(&self) -> Result<(MeadowField, Vec<[f32; 4]>), String> {
        if self.patches.is_empty() {
            return Err("grass.set needs at least one patch (use nil to clear)".into());
        }
        if !(0.1..=1.5).contains(&self.growth) || !(10.0..=256.0).contains(&self.distance) {
            return Err("grass.set growth must be 0.1 to 1.5 and distance 10 to 256 metres".into());
        }
        let mut low = Vec2::splat(f32::INFINITY);
        let mut high = Vec2::splat(f32::NEG_INFINITY);
        let mut footprints = Vec::with_capacity(self.patches.len());
        for patch in &self.patches {
            let (lo, hi) = patch.bounds();
            low = low.min(lo);
            high = high.max(hi);
            footprints.push([lo.x, lo.y, hi.x, hi.y]);
        }
        // A halo carries the correct elevation and signed distance up to the patch edge.
        let origin = (low / CELL).floor() * CELL - 2. * CELL;
        let dims = ((high - origin) / CELL).ceil() + 3.;
        if !dims.is_finite() || dims.x * dims.y > MAX_FIELD_SAMPLES as f32
            || (origin + Vec2::splat(CELL) - origin).min_element() < CELL {
            return Err("grass patches span too large an area for the meadow grid".into());
        }
        let (width, height) = (dims.x as usize, dims.y as usize);
        let mut samples = vec![[0.; 4]; width * height];
        for patch in &self.patches {
            let (lo, hi) = patch.bounds();
            let start = (((lo - origin) / CELL).floor() - 2.).max(Vec2::ZERO).as_uvec2();
            let end = (((hi - origin) / CELL).ceil() + 2.).as_uvec2();
            for j in start.y as usize..=usize::min(end.y as usize, height - 1) {
                for i in start.x as usize..=usize::min(end.x as usize, width - 1) {
                    let at = origin + Vec2::new(i as f32, j as f32) * CELL;
                    let d = self.cutouts.iter().fold(patch.clearance(at), |d, cut| d.min(-cut.clearance(at)));
                    let entry = &mut samples[j * width + i];
                    if entry[2] == 0. || SHOULDER + d > entry[1] {
                        *entry = [patch.height, SHOULDER + d, 1., 0.];
                    }
                }
            }
        }
        // One reusable set of three clump shapes. The game seed drives the GPU's
        // placement, variant choice, yaw, height variation and patch colour.
        let geometry = MeadowGeometry::cook_mown_turf(73)?;
        certify_edge_planes(&mut samples, width, height);
        let field = MeadowField { origin: origin.to_array(), cell: CELL, width: width as u32, height: height as u32,
            samples, seed: self.seed, growth: self.growth, dry_season: 0., radius: self.distance,
            fade_start: self.distance * 0.72, geometry };
        field.validate()?;
        Ok((field, footprints))
    }
}

/// The two native ground triangles are affine. If one plane underestimates
/// every vertex in the adjacent cells, it underestimates every interpolated
/// point there too. The GPU can trim individual blade anchors along this edge
/// even when their footprint crosses triangles. At corners it uses its normal
/// conservative whole-clump test.
fn certify_edge_planes(samples: &mut [[f32; 4]], width: usize, height: usize) {
    for j in 1..height - 2 {
        for i in 1..width - 2 {
            let index = j * width + i;
            let a = samples[index];
            if a[2] == 0. || (a[1] - SHOULDER).abs() > 2.0 { continue; }
            let dx = samples[index + 1][1] - a[1];
            let dz = samples[index + width][1] - a[1];
            let diagonal = samples[index + width + 1][1];
            if (a[1] + dx + dz - diagonal).abs() > 0.00001 { continue; }
            let certified = (j - 1..=j + 2).all(|row| (i - 1..=i + 2).all(|col| {
                let sample = samples[row * width + col];
                let plane = a[1] + (col as f32 - i as f32) * dx + (row as f32 - j as f32) * dz;
                sample[2] != 0. && sample[1] >= plane - 0.00001
            }));
            if certified { samples[index][3] = CELL; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn rotated_paths_and_disconnected_floors_survive_scene_refresh() {
        let field = Lawns {
            patches: vec![Rect::new(Vec3::ZERO, 16., 16., 0.).unwrap(),
                          Rect::new(Vec3::new(32., 2., 0.), 6., 6., 0.).unwrap()],
            cutouts: vec![Rect::new(Vec3::ZERO, 2., 24., std::f32::consts::FRAC_PI_4).unwrap()],
            seed: 19, growth: 0.3, distance: 24.,
        };
        let mut world = World::new(std::path::Path::new("."), 8);
        let (meadow, bounds) = field.build().unwrap();
        world.scene.meadow = Some(meadow);
        world.scene.meadow_footprints = bounds;
        world.fresh_scene().unwrap();
        let m = world.scene.meadow.as_ref().unwrap();
        let sample = |x: f32, z: f32| {
            let col = ((x - m.origin[0]) / m.cell) as usize;
            let row = ((z - m.origin[1]) / m.cell) as usize;
            m.samples[row * m.width as usize + col]
        };
        assert!(sample(3., 3.)[1] < SHOULDER);
        assert!(sample(3., -3.)[1] > SHOULDER);
        assert!(sample(8.5, 0.)[1] < SHOULDER);
        assert_eq!(sample(16., 0.)[2], 0.);
        assert_eq!(sample(7.5, -3.)[3], CELL);
        assert!(m.geometry.mown_turf);
        assert_eq!(m.geometry.lod_ranges[0][2][1], 60 * 3);
        assert!(m.geometry.vertices.iter().all(|v| v.color[1] > 0.2));
        assert_eq!(sample(32., 0.)[0], 2.);
        assert_eq!(world.scene.meadow_footprints, [[-8., -8., 8., 8.], [29., -3., 35., 3.]]);
        world.clear();
        assert!(world.scene.meadow.is_none() && world.scene.meadow_footprints.is_empty());
    }

    #[test]
    fn rejects_nonfinite_shapes_and_oversized_grids_before_allocating() {
        assert!(Rect::new(Vec3::NAN, 1., 1., 0.).is_err());
        let field = Lawns { patches: vec![Rect::new(Vec3::ZERO, 20000., 20000., 0.).unwrap()],
            cutouts: vec![], seed: 1, growth: 1., distance: 24. };
        assert!(field.build().is_err());
    }
}
