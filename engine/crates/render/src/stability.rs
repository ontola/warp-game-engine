//! World-anchored grids for shadows and canopy lighting.
//!
//! Snapping the target *after* a look-at centered on that target is a no-op:
//! its light-space x/y are already zero. Use a sun-only basis, then quantize
//! the absolute target coordinates in that basis instead.

use glam::{Mat4, Vec3, Vec4};

const MIN_EXTENT: f32 = 96.0;
const EXTENT_STEP: f32 = 16.0;

fn sun_basis(sun_dir: Vec3) -> (Vec3, Vec3, Vec3) {
    let z = if sun_dir.is_finite() { sun_dir.normalize_or(Vec3::Y) } else { Vec3::Y };
    // Avoid the singular look-at basis at noon or with an invalid zero sun.
    let up = if z.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let x = up.cross(z).normalize();
    let y = z.cross(x);
    (x, y, z)
}

/// A stable directional-light projection, with the existing 1..900 m depth
/// interval and eye 400 m toward the sun. `extent` is the HALF width in metres.
/// For a fixed sun and extent, translation changes x/y only by whole texels.
/// Depth may follow the target continuously: it does not change the x/y grid.
pub fn stable_shadow(centre: Vec3, sun_dir: Vec3, extent: f32, resolution: u32) -> Mat4 {
    let centre = if centre.is_finite() { centre } else { Vec3::ZERO };
    let extent = if extent.is_finite() && extent > 0.0 { extent } else { MIN_EXTENT };
    let texel = 2.0 * extent / resolution.max(1) as f32;
    let (x, y, z) = sun_basis(sun_dir);
    let cx = (x.dot(centre) / texel).round() * texel;
    let cy = (y.dot(centre) / texel).round() * texel;
    let cz = z.dot(centre) + 400.0;
    let view = Mat4::from_cols(
        Vec4::new(x.x, y.x, z.x, 0.0),
        Vec4::new(x.y, y.y, z.y, 0.0),
        Vec4::new(x.z, y.z, z.z, 0.0),
        Vec4::new(-cx, -cy, -cz, 1.0),
    );
    Mat4::orthographic_rh(-extent, extent, -extent, extent, 1.0, 900.0) * view
}

/// Round outward, never cropping the requested half-width. Keeping the scale
/// constant within a bucket also keeps orthographic shadow mesh LOD constant.
pub fn quantized_shadow_extent(required: f32) -> f32 {
    let required = if required.is_finite() { required.max(MIN_EXTENT) } else { MIN_EXTENT };
    (required / EXTENT_STEP).ceil() * EXTENT_STEP
}

/// Retain the largest bucket for this scene so four players close to a bucket
/// boundary cannot make all shadows alternate size/LOD. Reset `previous` when
/// rebuilding the scene. Wide player separation trades shadow detail for
/// coverage; retaining this value separately from shadow-cache validity is
/// essential because moving cars invalidate shadow CONTENT every frame.
pub fn stable_shadow_extent(required: f32, previous: f32) -> f32 {
    quantized_shadow_extent(required).max(quantized_shadow_extent(previous))
}

/// Scroll a volume in complete world cells. The cell centres at overlapping
/// world positions then remain identical when the volume moves a whole cell.
/// `desired` is its lower corner, not its centre; floor also handles negatives.
pub fn world_grid_origin(desired: Vec3, voxel: f32) -> Vec3 {
    let voxel = if voxel.is_finite() && voxel > 0.0 { voxel } else { 1.0 };
    let desired = if desired.is_finite() { desired } else { Vec3::ZERO };
    (desired / voxel).floor() * voxel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_xy(m: Mat4, point: Vec3, resolution: u32) -> [f32; 2] {
        let p = m.project_point3(point);
        [p.x * resolution as f32 * 0.5, p.y * resolution as f32 * 0.5]
    }

    #[test]
    fn sub_texel_camera_motion_does_not_move_world_shadow_xy() {
        let sun = Vec3::new(0.4, 0.8, -0.3).normalize();
        let (x, y, z) = sun_basis(sun);
        let texel = 192.0 / 4096.0;
        let point = Vec3::new(7.0, 2.0, -9.0);
        let reference = pixel_xy(stable_shadow(Vec3::ZERO, sun, 96.0, 4096), point, 4096);
        // Includes suspension/depth motion: only x/y grid stability matters.
        for t in [-0.49, -0.2, 0.0, 0.2, 0.49] {
            let centre = x * (texel * t) + y * (texel * t) + z * (t * 0.03);
            let p = pixel_xy(stable_shadow(centre, sun, 96.0, 4096), point, 4096);
            assert!((p[0] - reference[0]).abs() < 0.001);
            assert!((p[1] - reference[1]).abs() < 0.001);
        }
    }

    #[test]
    fn moving_or_separated_cameras_change_projection_by_integer_texels() {
        let sun = Vec3::new(-0.6, 0.7, 0.2).normalize();
        let point = Vec3::new(2007.0, 11.0, -1900.0);
        let initial = Vec3::new(2000.0, 10.0, -1890.0);
        let extent = quantized_shadow_extent(90.0 + 80.0);
        let first = pixel_xy(stable_shadow(initial, sun, extent, 4096), point, 4096);
        for shift in [Vec3::new(0.01, 0.02, -0.03), Vec3::new(1.2, -0.1, 2.0), Vec3::new(15.0, 0.0, -9.0)] {
            let p = pixel_xy(stable_shadow(initial + shift, sun, extent, 4096), point, 4096);
            for a in 0..2 {
                let delta = p[a] - first[a];
                assert!((delta - delta.round()).abs() < 0.02, "fractional shadow shift: {delta}");
            }
        }
    }

    #[test]
    fn vertical_sun_and_invalid_direction_stay_finite_and_cover_target() {
        for sun in [Vec3::Y, Vec3::NEG_Y, Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0)] {
            let centre = Vec3::new(1300.0, 40.0, -2000.0);
            let m = stable_shadow(centre, sun, 96.0, 4096);
            assert!(m.is_finite());
            let p = m.project_point3(centre);
            assert!(p.x.abs() <= 1.0 / 4096.0 + 0.00001);
            assert!(p.y.abs() <= 1.0 / 4096.0 + 0.00001);
            assert!(p.z > 0.0 && p.z < 1.0);
        }
    }

    #[test]
    fn extent_does_not_oscillate_or_crop_at_a_split_screen_bucket_boundary() {
        assert_eq!(quantized_shadow_extent(90.0), 96.0);
        let mut extent = 96.0;
        for required in [111.999, 112.001, 111.999, 112.001, 95.0, 250.0, 90.0] {
            let next = stable_shadow_extent(required, extent);
            assert!(next >= required && next >= extent);
            assert_eq!(next % EXTENT_STEP, 0.0);
            extent = next;
        }
        assert_eq!(extent, 256.0);
    }

    #[test]
    fn scrolling_volume_keeps_absolute_cell_centres_including_negative_space() {
        let a = world_grid_origin(Vec3::new(-0.01, 0.74, 3.2), 1.5);
        let b = world_grid_origin(Vec3::new(0.02, 0.76, 3.7), 1.5);
        assert_eq!(a, Vec3::new(-1.5, 0.0, 3.0));
        assert_eq!(b, Vec3::new(0.0, 0.0, 3.0));
        // The same centre shifts one local index when the grid scrolls.
        let before = a + (Vec3::new(4.0, 3.0, 2.0) + Vec3::splat(0.5)) * 1.5;
        let after = b + (Vec3::new(3.0, 3.0, 2.0) + Vec3::splat(0.5)) * 1.5;
        assert_eq!(before, after);
        assert_eq!(world_grid_origin(Vec3::new(0.1, 0.1, 3.1), 1.5), b);
    }
}
