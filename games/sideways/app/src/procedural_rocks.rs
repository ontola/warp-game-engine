//! One continuous fractured rock field for every native uphill cut face.
//! The native 2 m terrain remains the collider. This is a bounded render-only
//! surface refinement. There are no separate overlaid exposure meshes.
//! Dense, welded source geometry enters the existing meshoptimizer cluster
//! LOD pipeline; there is no new distance switch, dither or runtime tessellation.

use glam::{Vec2, Vec3};

const MAX_RELIEF: f32 = 0.46;

fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e3779b9)
        ^ (y as u32).wrapping_mul(0x85ebca6b) ^ seed;
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846ca68b);
    n ^ (n >> 16)
}
fn unit(n: u32) -> f32 { (n >> 8) as f32 / 16_777_216.0 }
fn smooth(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

fn noise(p: Vec2, seed: u32) -> f32 {
    let cell = p.floor(); let f = p - cell;
    let x = cell.x as i32; let y = cell.y as i32;
    let fx = smooth(f.x); let fy = smooth(f.y);
    let a = unit(hash(x, y, seed)); let b = unit(hash(x + 1, y, seed));
    let c = unit(hash(x, y + 1, seed)); let d = unit(hash(x + 1, y + 1, seed));
    ((a + (b - a) * fx) * (1.0 - fy) + (c + (d - c) * fx) * fy) * 2.0 - 1.0
}

fn rotate(p: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos(); Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c)
}

// Beds have independently jittered levels and gentle lateral warping. They
// also break off along the wall; there is no periodic rem_euclid stripe mask.
fn bed_height(index: i32, along: f32, seed: u32) -> f32 {
    index as f32 * 1.55 + (unit(hash(index, 71, seed)) - 0.5) * 1.05
        + noise(Vec2::new(along / 11.0, index as f32 * 0.71), seed ^ 0xba519) * 0.18
}

/// A jittered fracture network in a local metre-space rock face. Each cell
/// has its own planar face; narrow recessed boundaries expose the joints.
/// No camera-dependent noise or material darkening is involved.
fn fracture(p: Vec2, size: f32, seed: u32) -> (f32, f32) {
    let q = p / size;
    let cell = q.floor();
    let mut best = f32::INFINITY;
    let mut second = f32::INFINITY;
    let mut nearest = Vec2::ZERO;
    let mut nearest_key = 0;
    let mut runner_up = Vec2::ZERO;
    for y in -1..=1 { for x in -1..=1 {
        let ix = cell.x as i32 + x;
        let iy = cell.y as i32 + y;
        let key = hash(ix, iy, seed);
        let site = Vec2::new(ix as f32 + 0.18 + unit(key) * 0.64,
            iy as f32 + 0.18 + unit(key.rotate_left(13)) * 0.64);
        let distance = q.distance_squared(site);
        if distance < best {
            second = best; runner_up = nearest;
            best = distance; nearest = site; nearest_key = key;
        } else if distance < second {
            second = distance; runner_up = site;
        }
    }}
    // Distance to the bisector, rather than F2-F1, keeps crack width roughly
    // constant on differently sized faces.
    let joint = (second - best) * size / (2.0 * nearest.distance(runner_up).max(0.01));
    let local = (q - nearest) * size;
    let tilt = Vec2::new(unit(nearest_key.rotate_left(7)) - 0.5,
        unit(nearest_key.rotate_left(21)) - 0.5);
    let plane = (0.15 + 0.17 * unit(nearest_key.rotate_left(3)) + local.dot(tilt) * 0.10)
        .clamp(0.06, 0.36);
    (plane, joint.max(0.0))
}

fn relief(p: Vec2, seed: u32) -> f32 {
    // One continuous metre-space geology field across a stage, not a reset
    // local pattern for every 14 m patch. Domain warping breaks the regular
    // fracture-cell lattice; differently oriented scales expose large slabs.
    let angle = (unit(hash(17, 31, seed)) - 0.5) * 0.9;
    let rotated = rotate(p, angle);
    let warp = Vec2::new(noise(rotated / 13.0, seed ^ 0x19351) * 1.25,
        noise(rotated / 19.0 + Vec2::new(7.1, 2.3), seed ^ 0x419af) * 0.85);
    let q = rotated + warp;
    let macro_size = 3.2 + noise(p / 43.0, seed ^ 0x99e31) * 0.95;
    let (face, joint) = fracture(q * Vec2::new(0.65, 1.08), macro_size, seed);
    let chip_angle = 0.55 + unit(hash(23, 9, seed)) * 0.75;
    let (chip, chip_joint) = fracture(rotate(q + Vec2::new(2.71, 4.23), chip_angle),
        1.15 + noise(p / 27.0, seed ^ 0x7153) * 0.35, seed ^ 0x374d13);
    let mut bedding = 1.0f32;
    let bed_index = (q.y / 1.55).floor() as i32;
    for index in bed_index - 2..=bed_index + 2 {
        let distance = (q.y - bed_height(index, q.x, seed)).abs();
        let continuity = smooth((noise(Vec2::new(q.x / 7.5, index as f32 * 1.91), seed ^ 0xabc713) + 0.1) / 0.85);
        bedding = bedding.min(1.0 - 0.28 * continuity * (1.0 - smooth(distance / 0.075)));
    }
    (0.025 + face * smooth(joint / 0.15) * bedding
        + chip * 0.13 * smooth(chip_joint / 0.055)).min(MAX_RELIEF)
}

/// Small, continuous rock relief for *all* native cut faces, including the
/// spaces between the larger authored exposures. World metre coordinates
/// make it independent of chunks, camera distance and the twelve patches.
/// The renderer bounds this field together with its rounded base surface;
/// this is visual surface detail, never a replacement for the terrain collider.
pub(crate) fn terrain_relief(p: Vec3, n: Vec3) -> f32 {
    let wx = n.x.abs();
    let wz = n.z.abs();
    let sum = (wx + wz).max(0.001);
    let x = relief(Vec2::new(p.z, p.y), 0x7a6e193);
    let z = relief(Vec2::new(p.x, p.y), 0x7a6e193);
    ((x * wx + z * wz) / sum - 0.18) * 1.4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_geology_has_repeatable_real_relief_everywhere_on_the_wall() {
        let mut lo=f32::INFINITY;let mut hi=f32::NEG_INFINITY;
        for origin in [-160.0,0.0,160.0] {
            let mut local_lo=f32::INFINITY;let mut local_hi=f32::NEG_INFINITY;
            for z in 0..64 {for y in 0..32 {
                let p=Vec3::new(17.0,y as f32*0.25,origin+z as f32*0.25);
                let value=terrain_relief(p,Vec3::new(0.97,0.24,0.0).normalize());
                assert_eq!(value.to_bits(),terrain_relief(p,Vec3::new(0.97,0.24,0.0).normalize()).to_bits());
                assert!(value.is_finite()&&value.abs()<0.45);
                local_lo=local_lo.min(value);local_hi=local_hi.max(value);
            }}
            assert!(local_hi-local_lo>0.15,"detail disappeared away from an old exposure location");
            lo=lo.min(local_lo);hi=hi.max(local_hi);
        }
        assert!(hi-lo>0.20);
    }

    #[test]
    fn world_geology_does_not_reset_at_chunk_or_old_patch_intervals() {
        let n=Vec3::new(0.85,0.3,0.35).normalize();
        for z in [-128.0,-64.0,-14.0,0.0,14.0,64.0,128.0] {
            let left=terrain_relief(Vec3::new(17.3,8.2,z-0.0001),n);
            let right=terrain_relief(Vec3::new(17.3,8.2,z+0.0001),n);
            assert!((left-right).abs()<0.002,"geology reset at {z}");
        }
    }
}
