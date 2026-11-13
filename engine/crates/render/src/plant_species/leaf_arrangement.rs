//! Foliage recipes in the growth tip's local frame. Distances are metres.
//! Define another shoot by supplying a preset of its botanical arrangement.
use super::{blade, radial, stalk, update_normals};
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, tube, unit},
};
use glam::Vec3;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Two ranks on the same axis. A stagger of zero puts both members at each node.
/// At 0.5 the second rank falls between the first rank's nodes. The interval
/// includes both outermost insertion points; a single opposite pair starts at `start`.
pub(crate) fn paired_sites(
    count: u32,
    start: f32,
    end: f32,
    stagger: f32,
) -> impl Iterator<Item = PairedSite> {
    let intervals = count.saturating_sub(1) as f32 + stagger;
    (0..count).flat_map(move |pair| {
        [(-1.0, 0), (1.0, 1)].into_iter().map(move |(side, rank)| {
            let station = pair as f32 + rank as f32 * stagger;
            let fraction = if intervals > 0.0 {
                station / intervals
            } else {
                0.0
            };
            PairedSite {
                pair,
                side,
                index: pair * 2 + rank,
                fraction,
                t: fraction * end + (1.0 - fraction) * start,
            }
        })
    })
}

pub(crate) struct PairedSite {
    pub pair: u32,
    pub side: f32,
    pub index: u32,
    pub fraction: f32,
    pub t: f32,
}

/// A base value plus a signed change at full vigor (or at the tip).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Variation(pub f32, pub f32);
impl Variation {
    fn at(self, fraction: f32) -> f32 {
        self.0 + self.1 * fraction
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Twig {
    pub tip_m: Vec3,
    pub bow_m: Vec3,
    pub radius_m: f32,
    pub segments: usize,
}
impl Twig {
    fn build(self, part: &mut PartGeometry) -> Shoot {
        stalk(
            part,
            Vec3::ZERO,
            self.tip_m,
            self.bow_m,
            self.radius_m,
            self.segments,
        )
    }
}

/// Insertion sites for complete compound leaves along a terminal twig.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpiralShoot {
    pub twig: Twig,
    pub leaves: u32,
    pub first_node: f32,
    pub node_spacing: f32,
    pub divergence_rad: f32,
    pub variant_turn_rad: f32,
}
impl SpiralShoot {
    fn sites(
        self,
        part: &mut PartGeometry,
        v: u32,
    ) -> impl Iterator<Item = (u32, Vec3, Vec3)> + use<> {
        let twig = self.twig.build(part);
        (0..self.leaves).map(move |i| {
            let origin = twig.point(self.first_node + self.node_spacing * i as f32);
            let out = radial(self.divergence_rad * i as f32 + self.variant_turn_rad * v as f32);
            (i, origin, out)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OppositeBlade {
    pub length_m: Variation,
    pub lift: f32,
    pub hang: Variation,
    /// Axial tip drop, in blade lengths, as the pair expands.
    pub tip_drop: Variation,
}
/// A simple blade on each of two opposing petioles, with a separate young tip pair.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OppositeShoot {
    /// Base twig; the vertical internode lengths are added to its tip.
    pub twig: Twig,
    pub pairs: [u32; 3],
    pub internode_m: f32,
    pub variant_turn_rad: f32,
    pub mature: OppositeBlade,
    pub apical: OppositeBlade,
    pub petiole_m: Variation,
    pub petiole_radius_m: f32,
    pub width_ratio: Variation,
    /// Transverse folds, in blade widths, from nearly flat to deeply waved.
    pub edge_relief: Variation,
}
pub(super) const COFFEE: OppositeShoot = OppositeShoot {
    twig: Twig {
        tip_m: Vec3::new(0.02, 0.03, -0.025),
        bow_m: Vec3::new(-0.012, 0.0, 0.018),
        radius_m: 0.0026,
        segments: 6,
    },
    pairs: [3, 4, 5],
    internode_m: 0.075,
    variant_turn_rad: -0.35,
    mature: OppositeBlade {
        length_m: Variation(0.17, 0.075),
        lift: 0.12,
        hang: Variation(0.17, 0.42),
        tip_drop: Variation(0.35, 0.26),
    },
    apical: OppositeBlade {
        length_m: Variation(0.080, 0.045),
        lift: 0.90,
        hang: Variation(-0.10, 0.0),
        tip_drop: Variation(0.06, 0.12),
    },
    petiole_m: Variation(0.010, 0.008),
    petiole_radius_m: 0.0012,
    width_ratio: Variation(0.36, 0.08),
    edge_relief: Variation(0.023, 0.033),
};

/// A closed elliptic blade with a tapered drip tip and a wavy, entire margin.
/// The folds cross the midrib obliquely and fade at both ends of the lamina.
fn undulate_blade(
    part: &mut PartGeometry, base: Vec3, dir: Vec3, facing: Vec3,
    length: f32, width: f32, drop: f32, edge_relief: f32, phase: f32,
) {
    const SECTIONS: usize = 16;
    let side = facing.cross(dir).normalize();
    let upper = dir.cross(side);
    let start = part.vertices.len();
    let indices = part.indices.len();
    let axis = Shoot { root: base, delta: dir * length, bend: Vec3::ZERO, radius: 0.00065 };
    tube(part, axis, SECTIONS, 4);
    for v in &mut part.vertices[start..] {
        let t = v.uv[1];
        let (depth, lateral) = (v.uv[0] * TAU).sin_cos();
        let elliptic = (PI * (t + 0.065 * (PI * t).sin())).sin().max(0.0);
        let shoulder = if t > 0.87 { (1.0 - t) / 0.13 } else { 1.0 };
        let breadth = 0.020 + 0.98 * elliptic.powf(0.79) * shoulder;
        let edge = lateral.abs();
        let wave = ((t * 4.25 - edge * 0.28) * TAU + phase + 0.30 * lateral).sin();
        let half_width = 0.5 * width * breadth * (1.0 + 0.047 * wave);
        let fold = breadth * width * edge * (0.075 + edge_relief * wave);
        let along = t + edge * 0.027 * elliptic;
        v.pos = (base + dir * (along * length) - Vec3::Y * (drop * t * t)
            + side * (lateral * half_width) + upper * (fold + depth * axis.radius))
            .to_array();
    }
    update_normals(part, start, indices);
}

impl OppositeShoot {
    pub(super) fn build(self, part: &mut PartGeometry, v: u32) {
        let pairs = self.pairs[v as usize];
        let mut twig = self.twig;
        twig.tip_m.y += self.internode_m * pairs as f32;
        let stem = twig.build(part);
        for site in paired_sites(pairs, 0.0, 1.0, 0.0) {
            let node = site.pair;
            let rank = site.index % 2;
            let origin = stem.point((node as f32 + 0.65) / pairs as f32);
            let azimuth = self.variant_turn_rad * v as f32 + 0.32 * (unit(v, node, 11) - 0.5);
            let out = radial(azimuth) * site.side;
            let face = radial(azimuth + FRAC_PI_2);
            let leaf = if node + 1 == pairs {
                self.apical
            } else {
                self.mature
            };
            let vigor = unit(v, node, 12 + rank);
            let direction = (out + Vec3::Y * (leaf.lift + 0.34 * vigor)).normalize();
            let petiole = stalk(
                part,
                origin,
                direction * self.petiole_m.at(vigor),
                Vec3::ZERO,
                self.petiole_radius_m,
                2,
            );
            let length = leaf.length_m.at(vigor);
            let axis = (direction - face * leaf.hang.at(vigor)).normalize();
            let roll = 0.88 * (unit(v, node, 16 + rank) - 0.5);
            let facing = face * roll.cos() + Vec3::Y * roll.sin();
            let width = length * self.width_ratio.at(unit(v, node, 14 + rank));
            undulate_blade(
                part,
                petiole.point(1.0),
                axis,
                facing,
                length,
                width,
                length * leaf.tip_drop.at(vigor),
                self.edge_relief.at(vigor),
                TAU * unit(v, node, 18 + rank),
            );
        }
    }
}

/// Leaflets radiate from a common insertion at the end of each petiole.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PalmateShoot {
    pub shoot: SpiralShoot,
    pub petiole_m: Variation,
    pub petiole_lift: f32,
    pub petiole_bow_m: f32,
    pub petiole_radius_m: f32,
    pub leaflet_counts: [u32; 3],
    pub fan_half_angle_rad: Variation,
    pub leaflet_m: Variation,
    pub outer_reduction: f32,
    pub width_ratio: f32,
    pub curl_m: f32,
}
pub(super) const KAPOK: PalmateShoot = PalmateShoot {
    shoot: SpiralShoot {
        twig: Twig {
            tip_m: Vec3::new(0.03, 0.39, 0.01),
            bow_m: Vec3::new(0.0, 0.0, 0.021),
            radius_m: 0.0045,
            segments: 6,
        },
        leaves: 6,
        first_node: 0.10,
        node_spacing: 0.17,
        divergence_rad: 2.40,
        variant_turn_rad: 0.54,
    },
    petiole_m: Variation(0.11, 0.036),
    petiole_lift: 0.48,
    petiole_bow_m: 0.012,
    petiole_radius_m: 0.0019,
    leaflet_counts: [7, 5, 5],
    fan_half_angle_rad: Variation(1.40, 0.07),
    leaflet_m: Variation(0.162, 0.020),
    outer_reduction: 0.41,
    width_ratio: 0.31,
    curl_m: -0.023,
};
impl PalmateShoot {
    pub(super) fn build(self, part: &mut PartGeometry, v: u32) {
        let [a, b, c] = self.leaflet_counts;
        let stride = a.max(b).max(c) + 1;
        for (i, origin, out) in self.shoot.sites(part, v) {
            let across = out.cross(Vec3::Y);
            let petiole = stalk(
                part,
                origin,
                (out + Vec3::Y * self.petiole_lift) * self.petiole_m.at(unit(v, 1, i)),
                Vec3::Y * self.petiole_bow_m,
                self.petiole_radius_m,
                3,
            );
            let count = self.leaflet_counts[((i + v) % 3) as usize];
            let half_angle = self.fan_half_angle_rad.at(unit(v, 2, i));
            for j in 0..count {
                let f = if count > 1 {
                    2.0 * j as f32 / (count - 1) as f32 - 1.0
                } else {
                    0.0
                };
                let angle = f * half_angle;
                let dir = (out * angle.cos() + across * angle.sin() - Vec3::Y * 0.12).normalize();
                let key = i * stride + j;
                let length =
                    self.leaflet_m.at(unit(v, 3, key)) * (1.0 - self.outer_reduction * f.powi(2));
                blade(
                    part,
                    petiole.point(1.0),
                    dir * length,
                    Vec3::Y,
                    length * self.width_ratio,
                    self.curl_m,
                );
            }
        }
    }
}

/// Even-pinnate leaves. A bare petiole leads into a rachis bearing paired leaflets.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PinnateShoot {
    pub shoot: SpiralShoot,
    /// Length of the full axis, including its unbladed petiole.
    pub axis_m: Variation,
    pub petiole_m: Variation,
    pub axis_radius_m: f32,
    pub axis_bow_m: f32,
    pub basal_lift: f32,
    pub lift_per_leaf: f32,
    pub pairs: [u32; 3],
    pub stagger: f32,
    pub petiolule_m: Variation,
    pub petiolule_radius_m: f32,
    pub leaflet_m: Variation,
    pub width_ratio: f32,
    pub curl_m: f32,
    /// Axial staggering of opposite margins near the leaflet's insertion.
    pub base_skew: f32,
    /// Carries the upper shoulder outward before its short terminal point.
    pub distal_shoulder: f32,
}
pub(super) const MAHOGANY: PinnateShoot = PinnateShoot {
    shoot: SpiralShoot {
        twig: Twig {
            tip_m: Vec3::new(0.014, 0.23, 0.008),
            bow_m: Vec3::new(0.0, 0.0, 0.006),
            radius_m: 0.0038,
            segments: 5,
        },
        leaves: 4,
        first_node: 0.32,
        node_spacing: 0.22,
        divergence_rad: 2.40,
        variant_turn_rad: 0.60,
    },
    axis_m: Variation(0.19, 0.052),
    petiole_m: Variation(0.048, 0.012),
    axis_radius_m: 0.0018,
    axis_bow_m: 0.031,
    basal_lift: -0.44,
    lift_per_leaf: 0.27,
    pairs: [3, 4, 5],
    stagger: 0.0,
    petiolule_m: Variation(0.007, 0.003),
    petiolule_radius_m: 0.0009,
    leaflet_m: Variation(0.10, 0.040),
    width_ratio: 0.41,
    curl_m: -0.018,
    base_skew: 0.085,
    distal_shoulder: 0.045,
};
impl PinnateShoot {
    pub(super) fn build(self, part: &mut PartGeometry, v: u32) {
        let [a, b, c] = self.pairs;
        let stride = 2 * a.max(b).max(c);
        for (i, origin, out) in self.shoot.sites(part, v) {
            let across = out.cross(Vec3::Y);
            let length = self.axis_m.at(unit(v, 1, i));
            let axis =
                (out + Vec3::Y * (self.basal_lift + self.lift_per_leaf * i as f32)).normalize();
            let midrib = stalk(
                part,
                origin,
                axis * length,
                Vec3::Y * self.axis_bow_m,
                self.axis_radius_m,
                5,
            );
            let pairs = self.pairs[((v + i) % 3) as usize];
            let bare_petiole_m = self.petiole_m.at(unit(v, 2, i));
            for site in paired_sites(pairs, 0.0, 1.0, self.stagger) {
                let t = site.t + (1.0 - site.t) * bare_petiole_m / length;
                let key = i * stride + site.index;
                let dir = (across * site.side + out * (0.38 + 0.43 * site.fraction)
                    - Vec3::Y * (0.24 + 0.27 * unit(v, 3, key)))
                .normalize();
                let stalklet = stalk(
                    part,
                    midrib.point(t),
                    dir * self.petiolule_m.at(unit(v, 4, key)),
                    Vec3::ZERO,
                    self.petiolule_radius_m,
                    1,
                );
                let length = self.leaflet_m.at(site.fraction) * (0.95 + 0.10 * unit(v, 5, key));
                let facing = Vec3::Y + out * (0.20 + 0.50 * unit(v, 6, key));
                let start = part.vertices.len();
                let first_index = part.indices.len();
                blade(
                    part,
                    stalklet.point(1.0),
                    dir * length,
                    facing,
                    length * self.width_ratio,
                    self.curl_m,
                );
                for vertex in &mut part.vertices[start..] {
                    let [u, t] = vertex.uv;
                    let lateral = (u - 0.5) * 2.0;
                    let basal = self.base_skew * site.side * lateral * (1.0 - t).powi(2);
                    let shoulder = self.distal_shoulder * t * lateral.abs();
                    vertex.pos = (Vec3::from(vertex.pos) + dir * length * (basal + shoulder)).to_array();
                }
                update_normals(part, start, first_index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ShootRecipe;
    use super::*;

    #[test]
    fn opposite_and_staggered_ranks_span_the_authored_interval() {
        assert_eq!(paired_sites(0, 0.0, 1.0, 0.5).count(), 0);
        let single: Vec<_> = paired_sites(1, 0.2, 0.8, 0.0).collect();
        assert_eq!((single[0].t, single[1].t), (0.2, 0.2));
        let opposite: Vec<_> = paired_sites(4, 0.15, 0.92, 0.0).collect();
        for (i, pair) in opposite.chunks_exact(2).enumerate() {
            assert_eq!((pair[0].side, pair[1].side), (-1.0, 1.0));
            assert_eq!(pair[0].pair, i as u32);
            assert_eq!(pair[0].t, pair[1].t);
        }
        let staggered: Vec<_> = paired_sites(4, 0.15, 0.92, 0.5).collect();
        assert_eq!(staggered[0].t, 0.15);
        assert_eq!(staggered[7].t, 0.92);
        assert!(staggered.windows(2).all(|w| w[0].t < w[1].t));
        assert!((staggered[1].t - (staggered[0].t + staggered[2].t) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_new_palmate_preset_attaches_its_leaflets_to_one_point() {
        // A single three-fingered leaf on the same geometry path as kapok.
        const TRIFOLIATE: PalmateShoot = PalmateShoot {
            shoot: SpiralShoot {
                leaves: 1,
                ..KAPOK.shoot
            },
            leaflet_counts: [3; 3],
            ..KAPOK
        };
        let part = ShootRecipe::Palmate(&TRIFOLIATE).build(0);
        // The blade primitive's midrib begins at UV (0.5, 0); no stalk uses that UV.
        let bases: Vec<_> = part
            .vertices
            .iter()
            .filter(|v| v.uv == [0.5, 0.0])
            .collect();
        assert_eq!(bases.len(), 3);
        assert!(bases.iter().all(|v| v.pos == bases[0].pos));
    }

    #[test]
    fn paired_recipes_respect_leaf_counts_and_axis_length_from_the_preset() {
        const SIMPLE: OppositeShoot = OppositeShoot {
            pairs: [1, 2, 3],
            ..COFFEE
        };
        const COMPOUND: PinnateShoot = PinnateShoot {
            shoot: SpiralShoot {
                leaves: 1,
                ..MAHOGANY.shoot
            },
            pairs: [1, 2, 3],
            axis_m: Variation(0.30, 0.0),
            ..MAHOGANY
        };
        for v in 0..3 {
            for recipe in [
                ShootRecipe::Opposite(&SIMPLE),
                ShootRecipe::Pinnate(&COMPOUND),
            ] {
                let part = recipe.build(v);
                let bases: Vec<_> = part
                    .vertices
                    .iter()
                    .filter(|p| p.uv == [0.5, 0.0])
                    .collect();
                assert_eq!(bases.len(), (v as usize + 1) * 2);
                assert!(bases.chunks_exact(2).all(|pair| pair[0].pos != pair[1].pos));
            }
        }
        let short = PinnateShoot {
            axis_m: Variation(0.20, 0.0),
            ..COMPOUND
        };
        let empty = || PartGeometry {
            id: 0,
            vertices: vec![],
            indices: vec![],
        };
        let (mut a, mut b) = (empty(), empty());
        COMPOUND.build(&mut a, 1);
        short.build(&mut b, 1);
        let last_base = |part: &PartGeometry| {
            Vec3::from(
                part.vertices
                    .iter()
                    .rev()
                    .find(|p| p.uv == [0.5, 0.0])
                    .unwrap()
                    .pos,
            )
        };
        assert!((last_base(&a).distance(last_base(&b)) - 0.10).abs() < 1e-5);
    }
}
