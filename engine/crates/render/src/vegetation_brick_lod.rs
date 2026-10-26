//! Projected spatial-error bounds, not a guarantee of optical equivalence.
//! A homogeneous voxel cannot reconstruct source silhouettes lost by its cook.
use crate::foliage_volume_data::GpuLevel;
use glam::{Mat4, Vec3};

/// Blend only over the first quarter octave after the coarser level becomes
/// eligible (distance ratio 1..2^0.25). Else use one optical mip.
pub const MIP_TRANSITION_OCTAVES: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrickMipSelection {
    pub fine_mip: u32,
    pub coarse_mip: u32,
    pub coarse_weight: f32,
    pub finest_projected_pixels: f32,
    /// A positive requested budget cannot be met even at mip zero. The caller
    /// must retain source representation; choosing mip zero is not acceptance.
    pub finest_over_budget: bool,
}

fn select(numerator: f32, radius: f32, entry: f32, budget: f32, smooth: bool) -> BrickMipSelection {
    let finest = if entry > radius { numerator / (entry - radius) } else { f32::INFINITY };
    let ratio = if budget > 0.0 { budget * entry.max(1e-5) / (numerator + budget * radius) } else { 1.0 };
    let coordinate = ratio.max(1.0).log2();
    let coarse = coordinate.floor().clamp(0.0, 3.0) as u32;
    let mut result = BrickMipSelection { fine_mip: coarse, coarse_mip: coarse, coarse_weight: 0.0,
        finest_projected_pixels: finest, finest_over_budget: budget > 0.0 && finest > budget };
    if smooth && coarse > 0 {
        let weight = ((coordinate - coarse as f32) / MIP_TRANSITION_OCTAVES).clamp(0.0, 1.0);
        if weight < 1.0 {
            result.fine_mip = coarse - 1;
            result.coarse_weight = weight * weight * (3.0 - 2.0 * weight);
            if weight == 0.0 { result.coarse_mip = result.fine_mip; }
        }
    }
    result
}

/// CPU reference of the consumer's per-ray perspective footprint and selector.
/// `entry_distance` is ray distance to the clipped brick entrance, not its
/// centre. A zero budget explicitly forces finest; it does not certify quality.
pub fn select_brick_mips(view_projection: Mat4, input_size: [u32; 2], ray_direction: Vec3,
    entry_distance: f32, fine_cell: f32, lod_pixels: f32, smooth_mips: bool) -> Result<BrickMipSelection, String> {
    if !view_projection.is_finite() || input_size.contains(&0) || !ray_direction.is_finite()
        || ray_direction.length_squared() < 1e-12 || !entry_distance.is_finite() || entry_distance < 0.0
        || !fine_cell.is_finite() || fine_cell <= 0.0 || !lod_pixels.is_finite() || lod_pixels < 0.0 {
        return Err("invalid vegetation brick projected-error query".into());
    }
    let direction = ray_direction.normalize();
    let rows = view_projection.transpose();
    let row_w = rows.w_axis.truncate();
    let row_x = rows.x_axis.truncate();
    let row_y = rows.y_axis.truncate();
    let rate = row_w.dot(direction).max(1e-6);
    let x = row_x.dot(direction) / rate;
    let y = row_y.dot(direction) / rate;
    let jx = (row_x - row_w * x) * (0.5 * input_size[0] as f32);
    let jy = (row_y - row_w * y) * (0.5 * input_size[1] as f32);
    let diameter = fine_cell * 1.73205081;
    Ok(select(diameter * (jx.length_squared() + jy.length_squared()).sqrt() / rate,
        0.5 * diameter / rate, entry_distance, lod_pixels, smooth_mips))
}

/// Conservative upper bound for the projected diameter of ANY finest cell in
/// this brick. Suitable for a CPU whole-brick source-retention gate before
/// rendering. Crossing the camera plane returns infinity (retain source).
/// This bounds cell footprint only; provider quadrature error and unresolved
/// source coverage need separate validation and must not be silently ignored.
pub fn brick_projected_error_bound(view_projection: Mat4, input_size: [u32; 2], level: &GpuLevel) -> Result<f32, String> {
    if !view_projection.is_finite() || input_size.contains(&0)
        || !level.origin_cell.iter().all(|v| v.is_finite()) || level.origin_cell[3] <= 0.0
        || level.dims_offset[..3].contains(&0) {
        return Err("invalid vegetation brick projection bound".into());
    }
    let matrix = view_projection.as_dmat4();
    let rows = matrix.transpose();
    let (rx, ry, rw) = (rows.x_axis.truncate(), rows.y_axis.truncate(), rows.w_axis.truncate());
    let lower = glam::DVec3::new(level.origin_cell[0] as f64, level.origin_cell[1] as f64, level.origin_cell[2] as f64);
    let extent = glam::DVec3::new(level.dims_offset[0] as f64, level.dims_offset[1] as f64, level.dims_offset[2] as f64) * level.origin_cell[3] as f64;
    let (mut min_w, mut max_x, mut max_y) = (f64::INFINITY, 0.0f64, 0.0f64);
    for i in 0..8 {
        let p = lower + extent * glam::DVec3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64);
        let clip = matrix * p.extend(1.0);
        if clip.w <= 0.0 { return Ok(f32::INFINITY); }
        min_w = min_w.min(clip.w);
        // Linear-fractional projected coordinates attain extrema at corners
        // while W>0. Squared Jacobian row norms are convex in each coordinate.
        max_x = max_x.max((rx - rw * (clip.x / clip.w)).length_squared());
        max_y = max_y.max((ry - rw * (clip.y / clip.w)).length_squared());
    }
    let scale = (max_x * (0.5 * input_size[0] as f64).powi(2)
        + max_y * (0.5 * input_size[1] as f64).powi(2)).sqrt() / min_w;
    let bound = level.origin_cell[3] as f64 * 3.0f64.sqrt() * scale;
    // Round outward across f64->f32; infinity remains conservative.
    let rounded = bound as f32;
    Ok(if rounded.is_finite() { f32::from_bits(rounded.to_bits() + 1) } else { rounded })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smoothing_is_local_continuous_and_never_selects_ineligible_mips() {
        let (numerator, radius, budget) = (23.0, 0.08, 2.0);
        for boundary in 1..=3 {
            let threshold = 2.0f32.powi(boundary);
            for fraction in [-0.0001, 0.0, 0.0001, 0.1, 0.251, 0.5, 0.99] {
                let ratio = threshold * 2.0f32.powf(fraction);
                let entry = ratio * (numerator + budget * radius) / budget;
                let s = select(numerator, radius, entry, budget, true);
                for mip in [s.fine_mip, s.coarse_mip] {
                    let scale = (1u32 << mip) as f32;
                    assert!(numerator * scale / (entry - radius * scale) <= budget * 1.00001);
                }
                if (0.25..1.0).contains(&fraction) { assert_eq!(s.fine_mip, s.coarse_mip); }
                let value = (1.0 - s.coarse_weight) * (s.fine_mip + 1) as f32
                    + s.coarse_weight * (s.coarse_mip + 1) as f32;
                if fraction.abs() < 0.001 { assert!((value - boundary as f32).abs() < 0.001); }
            }
        }
        let too_close = select(23.0, 0.08, 1.0, 2.0, true);
        assert!(too_close.finest_over_budget); assert_eq!(too_close.fine_mip, 0);
        assert_eq!(select(23.0, 0.08, 1e6, 0.0, true).fine_mip, 0);
    }
    #[test]
    fn whole_brick_bound_contains_actual_cell_corner_projections() {
        let projection = Mat4::perspective_infinite_reverse_rh(55f32.to_radians(), 1.5, 0.05);
        let size = [960, 640];
        let level = GpuLevel { origin_cell: [-2.0, -1.0, -12.0, 0.25], dims_offset: [16, 8, 16, 0] };
        let bound = brick_projected_error_bound(projection, size, &level).unwrap();
        let project = |p:glam::DVec3| {
            let c = projection.as_dmat4() * p.extend(1.0);
            glam::DVec2::new(c.x / c.w * 480.0, c.y / c.w * 320.0)
        };
        for z in 0..16 { for y in 0..8 { for x in 0..16 {
            let lower = glam::DVec3::new(-2.0 + x as f64 * 0.25, -1.0 + y as f64 * 0.25, -12.0 + z as f64 * 0.25);
            let corners: Vec<_> = (0..8).map(|i| project(lower + glam::DVec3::new((i&1) as f64, ((i>>1)&1) as f64, ((i>>2)&1) as f64) * 0.25)).collect();
            for a in &corners { for b in &corners { assert!(a.distance(*b) <= bound as f64); }}
        }}}
        let mut crossing = level; crossing.origin_cell[2] = -0.25;
        assert!(brick_projected_error_bound(projection, size, &crossing).unwrap().is_infinite());
    }
    #[test]
    fn optical_moment_blending_conserves_integrated_mass_and_endpoints() {
        // Eight fine child cells, equal length. Empty cells contribute neither
        // black material nor artificial normals; mix coefficients, not alpha.
        let fine = [0.0f64, 2.0, 0.0, 0.0, 4.0, 0.0, 1.0, 0.0];
        let coarse = fine.iter().sum::<f64>() / 8.0;
        for weight in [0.0, 0.001, 0.2, 0.5, 0.999, 1.0] {
            let mass:f64 = fine.iter().map(|&v| v * (1.0 - weight) + coarse * weight).sum();
            assert!((mass - fine.iter().sum::<f64>()).abs() < 1e-12);
            assert!(((-mass).exp() - (-fine.iter().sum::<f64>()).exp()).abs() < 1e-12);
        }
        let sigma_a = 0.0; let sigma_b = 2.0; let weight = 0.25;
        let mass = sigma_a * (1.0 - weight) + sigma_b * weight;
        let red = (0.0 * sigma_a * (1.0 - weight) + 0.8 * sigma_b * weight) / mass;
        assert_eq!(red, 0.8);
    }
    #[test]
    fn blended_coarse_tail_remains_visible_when_fine_child_is_empty() {
        // An empty fine child is not an empty mixed medium. Only an exactly
        // empty ancestor at/above the coarse optical mip permits a jump.
        let fine = [0.0f64, 0.0, 4.0, 0.0];
        let coarse = [0.0f64, 2.0];
        let weight = 0.3;
        let clipped_lengths = [0.2f64, 1.0, 1.0, 0.15];
        let expected: f64 = (0..4).map(|i|
            (fine[i]*(1.0-weight)+coarse[i/2]*weight)*clipped_lengths[i]).sum();
        let mut integrated = 0.0;
        let mut visited = 0;
        let mut i = 0;
        while i < 4 {
            if coarse[i/2] == 0.0 {
                assert_eq!(fine[i],0.0);
                i = (i/2+1)*2; // exact-empty coarse jump
                continue;
            }
            integrated += (fine[i]*(1.0-weight)+coarse[i/2]*weight)*clipped_lengths[i];
            visited += 1;i += 1;
        }
        assert_eq!(visited,2);assert_eq!(integrated,expected);
        assert!(integrated > fine[2]*(1.0-weight)); // clipped nonzero tail retained
    }
}
