//! One regular grid for tyre contact, character support and gameplay rays.
use crate::vehicle::surfaces::Surface;
use glam::{Quat, Vec2, Vec3};
use rapier3d::{
    glamx::Pose3,
    parry::{shape::HeightFieldFlags, utils::Array2},
    prelude::*,
};
use std::sync::Arc;

#[derive(Debug)]
pub struct RoadStrip {
    pub points: Vec<Vec2>,
    pub width: f32,
    pub surface: Surface,
}

#[derive(Debug)]
struct RoadSegment {
    start: Vec2,
    delta: Vec2,
    inv_length_sq: f32,
    radius_sq: f32,
    surface: Surface,
}

#[derive(Debug)]
pub struct HeightGrid {
    shape: SharedShape,
    columns: usize,
    rows: usize,
    half: Vec2,
    spacing: f32,
    base: Surface,
    segments: Vec<RoadSegment>,
    // A wheel only tests strips touching its cell, in the caller's priority order.
    cells: Vec<Vec<u16>>,
}

impl HeightGrid {
    pub const MAX_SAMPLES: usize = 65_536;
    pub const MAX_ROAD_POINTS: usize = 4096;

    pub fn new(
        columns: usize,
        spacing: f32,
        heights: &[f32],
        base: Surface,
        roads: &[RoadStrip],
    ) -> Result<Self, String> {
        if columns < 2
            || heights.len() > Self::MAX_SAMPLES
            || heights.len() / columns < 2
            || heights.len() % columns != 0
            || !spacing.is_finite()
            || spacing <= 0.
            || heights.iter().any(|y| !y.is_finite())
        {
            return Err("heightfield needs a rectangular grid of 2..65536 finite samples and positive spacing".into());
        }
        let rows = heights.len() / columns;
        let span = Vec2::new((columns - 1) as f32, (rows - 1) as f32) * spacing;
        if !span.is_finite() {
            return Err("heightfield extent too large".into());
        }
        let samples = Array2::from_fn(rows, columns, |z, x| heights[z * columns + x]);
        let shape = SharedShape::heightfield_with_flags(
            samples,
            rapier3d::glamx::Vec3::new(span.x, 1., span.y),
            HeightFieldFlags::FIX_INTERNAL_EDGES,
        );
        let mut grid = Self {
            shape,
            columns,
            rows,
            half: span * 0.5,
            spacing,
            base,
            segments: Vec::new(),
            cells: vec![Vec::new(); (columns - 1) * (rows - 1)],
        };
        if roads.iter().map(|r| r.points.len()).sum::<usize>() > Self::MAX_ROAD_POINTS {
            return Err("heightfield road point budget exceeded".into());
        }
        let mut references = 0usize;
        for road in roads {
            if road.points.len() < 2
                || !road.width.is_finite()
                || road.width <= 0.
                || road.points.iter().any(|p| !p.is_finite())
            {
                return Err(
                    "heightfield road needs a positive width and at least two finite points".into(),
                );
            }
            let radius = road.width * 0.5;
            for pair in road.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let delta = b - a;
                if delta.length_squared() <= f32::EPSILON {
                    continue;
                }
                let lo = a.min(b) - Vec2::splat(radius);
                let hi = a.max(b) + Vec2::splat(radius);
                if hi.x < 0. || hi.y < 0. || lo.x > span.x || lo.y > span.y {
                    continue;
                }
                let (ix, iz) = grid.cell(lo);
                let (jx, jz) = grid.cell(hi);
                references += (jx - ix + 1) * (jz - iz + 1);
                if references > 1_048_576 {
                    return Err("heightfield road coverage budget exceeded".into());
                }
                let id = grid.segments.len() as u16;
                grid.segments.push(RoadSegment {
                    start: a,
                    delta,
                    inv_length_sq: 1. / delta.length_squared(),
                    radius_sq: radius * radius,
                    surface: road.surface,
                });
                for z in iz..=jz {
                    for x in ix..=jx {
                        grid.cells[z * (columns - 1) + x].push(id);
                    }
                }
            }
        }
        Ok(grid)
    }

    pub fn center_offset(&self) -> Vec3 {
        Vec3::new(self.half.x, 0., self.half.y)
    }

    fn cell(&self, offset: Vec2) -> (usize, usize) {
        let p = offset / self.spacing;
        (
            (p.x.floor() as usize).min(self.columns - 2),
            (p.y.floor() as usize).min(self.rows - 2),
        )
    }

    fn surface(&self, offset: Vec2) -> Option<Surface> {
        if offset.min_element() < 0. || offset.x > 2. * self.half.x || offset.y > 2. * self.half.y {
            return None;
        }
        let (x, z) = self.cell(offset);
        for &id in &self.cells[z * (self.columns - 1) + x] {
            let segment = &self.segments[id as usize];
            let rel = offset - segment.start;
            let along = (rel.dot(segment.delta) * segment.inv_length_sq).clamp(0., 1.);
            if (rel - segment.delta * along).length_squared() <= segment.radius_sq {
                return Some(segment.surface);
            }
        }
        Some(self.base)
    }
}

#[derive(Clone, Debug)]
pub struct PlacedGrid {
    grid: Arc<HeightGrid>,
    pose: Pose3,
}

impl PlacedGrid {
    pub fn new(grid: Arc<HeightGrid>, rotation: Quat, position: Vec3) -> Self {
        Self {
            grid,
            pose: Pose3::from_parts(crate::to(position), crate::to_q(rotation)),
        }
    }

    pub(crate) fn collider(&self) -> ColliderBuilder {
        ColliderBuilder::new(self.grid.shape.clone()).position(self.pose)
    }

    pub fn ray(&self, origin: Vec3, direction: Vec3, limit: f32) -> Option<(f32, Vec3)> {
        let hit = self.grid.shape.cast_ray_and_get_normal(
            &self.pose,
            &Ray::new(crate::to(origin), crate::to(direction)),
            limit,
            false,
        )?;
        Some((hit.time_of_impact, crate::from(hit.normal)))
    }

    pub fn floor(&self, x: f32, z: f32) -> Option<f32> {
        let bounds = self.grid.shape.compute_aabb(&self.pose);
        if x < bounds.mins.x || x > bounds.maxs.x || z < bounds.mins.z || z > bounds.maxs.z {
            return None;
        }
        let top = bounds.maxs.y + 1.;
        self.ray(Vec3::new(x, top, z), Vec3::NEG_Y, top - bounds.mins.y + 1.)
            .map(|(d, _)| top - d)
    }

    pub fn touches(&self, pos: Vec3, radius: f32) -> bool {
        self.grid
            .shape
            .distance_to_point(&self.pose, crate::to(pos), false)
            <= radius
    }

    pub(crate) fn surface_at(&self, position: rapier3d::glamx::Vec3) -> Option<Surface> {
        let p = self.pose.inverse_transform_point(position);
        self.grid.surface(Vec2::new(p.x, p.z) + self.grid.half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vehicle::{
        CarSpec,
        fleet::{Fleet, Pedals},
    };

    #[test]
    fn diagonal_rays_and_ordered_grip_follow_the_same_grid() {
        let road = |width, surface| RoadStrip {
            points: vec![Vec2::new(0., 1.), Vec2::new(4., 1.)],
            width,
            surface,
        };
        let grid = HeightGrid::new(
            3,
            2.,
            &[0., 2., 4., 1., 5., 6.],
            Surface::Grass,
            &[road(0.5, Surface::Asphalt), road(1.5, Surface::Gravel)],
        )
        .unwrap();
        let center = grid.center_offset() + Vec3::new(4., 7., -3.);
        let placed = PlacedGrid::new(Arc::new(grid), Quat::IDENTITY, center);
        for (x, z, h) in [(0.5, 0.5, 0.75), (1.5, 1.5, 3.25), (3., 0.5, 3.75)] {
            assert!((placed.floor(4. + x, -3. + z).unwrap() - (7. + h)).abs() < 0.001);
        }
        assert!(placed.floor(3.99, -2.).is_none());
        for (z, expected) in [
            (1., Surface::Asphalt),
            (1.5, Surface::Gravel),
            (0., Surface::Grass),
        ] {
            assert_eq!(
                placed.surface_at(crate::to(Vec3::new(5., 7., -3. + z))),
                Some(expected)
            );
        }
    }

    #[test]
    fn driven_car_climbs_the_contact_surface() {
        let samples: Vec<f32> = (0..11)
            .flat_map(|z| std::iter::repeat_n((10 - z) as f32, 3))
            .collect();
        let grid = HeightGrid::new(3, 10., &samples, Surface::Gravel, &[]).unwrap();
        let offset = grid.center_offset();
        let mut fleet = Fleet::new(std::iter::empty());
        fleet.add_heightfield(PlacedGrid::new(Arc::new(grid), Quat::IDENTITY, offset));
        let id = fleet.spawn(CarSpec::compact(), Vec3::new(10., 1., 90.), 0.);
        fleet.drive(
            id,
            Some(Pedals {
                throttle: 1.,
                ..Default::default()
            }),
        );
        for _ in 0..240 {
            fleet.step(1. / 60.);
        }
        let car = fleet.pose(id).unwrap();
        let floor = (100. - car.pos.z) * 0.1;
        // A front-wheel-drive road car on 10% gravel: about 0.15 g net, ~9 m in 4 s.
        assert!(
            car.pos.z < 82. && car.pos.y > 2. && (car.pos.y - floor).abs() < 1.,
            "pos {:?} floor {}",
            car.pos,
            floor
        );
    }
}
