//! The physics world: Rapier, with the stage's colliders and the cars'
//! bodies. A collider's user_data is the surface the tyres find on it, or
//! ROAD for the road ribbon (whose surface comes from the stage layout).

use crate::builder::{self, Block, Chunk, Road};
use crate::spec::CarSpec;
use crate::surfaces::Surface;
use gdcompat::math::V3;
use rapier3d::glamx::{Pose3, Quat, Vec3};
use rapier3d::parry::shape::TriMeshFlags;
use rapier3d::prelude::*;

pub const ROAD: u128 = 255;
pub const GROUP_WORLD: Group = Group::GROUP_1;
pub const GROUP_CAR: Group = Group::GROUP_2;
pub const GROUP_GHOST: Group = Group::GROUP_3;
pub const HZ: f64 = 120.0;

pub struct Physics {
    pub world: PhysicsWorld,
}

/// Render-authoring identity for an existing trunk; it does not generate roots.
#[derive(Clone, Copy, Debug)]
pub struct TreeColliderSource {
    pub root: [f32; 3],
    /// Original horizontal and vertical instance scale.
    pub scale: [f32; 2],
}

struct TreeColliderEntry {
    source: usize,
    handle: ColliderHandle,
    root: Vec3,
    original: Collider,
}

/// Opaque stable handles to verified native trunks, captured before art edits.
/// Background visual trees have no native collider and remain absent here.
pub struct TreeColliderBaseline {
    sources: usize,
    entries: Vec<TreeColliderEntry>,
}

impl TreeColliderBaseline {
    /// Match root X/Z exactly, then verify static Dirt cylinder dimensions and
    /// centre against the original source scale. Never selects all cylinders.
    pub fn capture(physics: &Physics, sources: &[TreeColliderSource]) -> Result<Self, String> {
        let mut roots = std::collections::HashMap::new();
        for (index, source) in sources.iter().enumerate() {
            if source.root.iter().chain(source.scale.iter()).any(|v| !v.is_finite())
                || source.scale.iter().any(|v| *v <= 0.0)
            { return Err("tree collider source must have finite roots and positive scales".into()); }
            let key = (source.root[0].to_bits(), source.root[2].to_bits());
            if roots.insert(key, index).is_some() {
                return Err("duplicate tree roots cannot identify native trunk colliders safely".into());
            }
        }
        let mut entries = Vec::new();
        let mut matched = std::collections::HashSet::new();
        for (handle, collider) in physics.world.colliders.iter() {
            if collider.parent().is_some() || collider.user_data != Surface::Dirt as u128
                || collider.collision_groups() != Physics::world_groups()
            { continue; }
            let Some(cylinder) = collider.shape().as_cylinder() else { continue; };
            let p = collider.translation();
            let Some(&index) = roots.get(&(p.x.to_bits(), p.z.to_bits())) else { continue; };
            let source = sources[index];
            let expected_y = source.root[1] + 2.5 * source.scale[1];
            let close = |a: f32, b: f32| (a-b).abs() <= 2e-5 * b.abs().max(1.0);
            if !close(cylinder.radius, 0.22 * source.scale[0])
                || !close(cylinder.half_height, 3.0 * source.scale[1])
                || !close(p.y, expected_y)
                || !collider.rotation().abs_diff_eq(Quat::IDENTITY, 1e-6)
            { return Err(format!("collider at tree root {index} does not match its original trunk dimensions")); }
            if !matched.insert(index) { return Err("multiple native trunk colliders share one tree root".into()); }
            entries.push(TreeColliderEntry { source: index, handle,
                root: Vec3::from_array(source.root), original: collider.clone() });
        }
        Ok(Self { sources: sources.len(), entries })
    }

    pub fn len(&self) -> usize { self.entries.len() }

    /// Validate every handle before any mutation. Rendering must already have
    /// rebuilt its canopy/shadow resources from the same size and selection.
    /// Call while the simulation is paused; normal fixed steps propagate the
    /// changed collider bounds through Rapier's broad phase on resume.
    pub fn apply(&self, physics: &mut Physics, size: f32, enabled: &[bool]) -> Result<(), String> {
        if !size.is_finite() || !(0.5..=1.5).contains(&size) || enabled.len() != self.sources {
            return Err("tree collider edit has invalid size or selection length".into());
        }
        for entry in &self.entries {
            let collider = physics.world.colliders.get(entry.handle)
                .ok_or("tree collider baseline belongs to an older physics world")?;
            if collider.parent().is_some() || collider.user_data != entry.original.user_data
                || collider.collision_groups() != entry.original.collision_groups()
                || collider.shape().as_cylinder().is_none()
                || collider.translation().x.to_bits() != entry.root.x.to_bits()
                || collider.translation().z.to_bits() != entry.root.z.to_bits()
            { return Err("tree collider changed identity after its baseline was captured".into()); }
        }
        for entry in &self.entries {
            let collider = physics.world.colliders.get_mut(entry.handle).unwrap();
            if size == 1.0 {
                collider.set_shape(entry.original.shared_shape().clone());
                collider.set_position(*entry.original.position());
            } else {
                let cylinder = entry.original.shape().as_cylinder().unwrap();
                collider.set_shape(SharedShape::cylinder(cylinder.half_height * size, cylinder.radius * size));
                let mut pose = *entry.original.position();
                pose.translation = entry.root + (entry.original.translation() - entry.root) * size;
                collider.set_position(pose);
            }
            collider.set_enabled(enabled[entry.source] && entry.original.is_enabled());
        }
        Ok(())
    }
}

fn g(v: V3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

impl Physics {
    pub fn new() -> Physics {
        let mut world = PhysicsWorld::default();
        world.gravity = Vec3::new(0.0, -9.8, 0.0); // Godot's default
        world.integration_parameters.dt = (1.0 / HZ) as f32;
        Physics { world }
    }

    fn world_groups() -> InteractionGroups {
        InteractionGroups::new(GROUP_WORLD, Group::ALL, InteractionTestMode::And)
    }

    /// Godot winds its ground triangles facing down (its concave shapes
    /// are double-sided); Rapier's are one-sided with internal-edge fixing,
    /// so the winding is flipped here to face up.
    pub fn add_trimesh(&mut self, verts: Vec<Vec3>, tris: Vec<[u32; 3]>, tag: u128) {
        let tris: Vec<[u32; 3]> = tris.into_iter().map(|[a, b, c]| [a, c, b]).collect();
        let c = ColliderBuilder::trimesh_with_flags(verts, tris, TriMeshFlags::FIX_INTERNAL_EDGES)
            .expect("trimesh")
            .user_data(tag)
            .collision_groups(Self::world_groups())
            .build();
        self.world.colliders.insert(c);
    }

    pub fn add_chunk(&mut self, c: &Chunk) {
        let (verts, tris) = c.triangles();
        self.add_trimesh(verts.into_iter().map(g).collect(), tris, Surface::Grass as u128);
    }

    pub fn add_road(&mut self, r: &Road) {
        self.add_trimesh(r.verts.iter().map(|v| g(*v)).collect(), r.tris.clone(), ROAD);
    }

    /// A tree trunk: the 12-sided prism (radius 0.22, -0.5..5.5 m) as a cylinder.
    pub fn add_trunk(&mut self, t: &builder::Spot) {
        let half = 3.0 * t.scale_y;
        let c = ColliderBuilder::cylinder(half, 0.22 * t.scale)
            .translation(Vec3::new(t.pos.0, t.pos.1 + 2.5 * t.scale_y, t.pos.2))
            .user_data(Surface::Dirt as u128)
            .collision_groups(Self::world_groups())
            .build();
        self.world.colliders.insert(c);
    }

    pub fn add_grown_trunk(&mut self, spot: &builder::Spot, profile: crate::stage::TrunkShape) {
        let root = Vec3::new(spot.pos.0, spot.pos.1, spot.pos.2);
        let bole = Quat::from_rotation_y(spot.yaw) * Vec3::from_array(profile.tip) * spot.scale;
        let pose = Pose3::from_parts(root + 0.5 * bole, Quat::from_rotation_arc(Vec3::Y, bole.normalize()));
        let collider = ColliderBuilder::cylinder(0.5 * bole.length(), profile.radius * spot.scale)
            .position(pose).user_data(Surface::Dirt as u128).collision_groups(Self::world_groups()).build();
        self.world.colliders.insert(collider);
    }

    pub fn add_block(&mut self, b: &Block) {
        // Godot's looking_at: local -Z along the direction.
        let rot = Quat::from_rotation_arc(Vec3::NEG_Z, g(b.along).normalize_or(Vec3::NEG_Z));
        let c = ColliderBuilder::cuboid(b.half.x, b.half.y, b.half.z)
            .position(Pose3::from_parts(g(b.centre), rot))
            .user_data(Surface::Dirt as u128)
            .collision_groups(Self::world_groups())
            .build();
        self.world.colliders.insert(c);
    }

    /// Flat bench ground at y = 0. A half-space, not Godot's 6 km box:
    /// GJK loses f32 precision against support points that far out.
    pub fn add_flat_ground(&mut self, surface: Surface) {
        let c = ColliderBuilder::new(SharedShape::halfspace(Vec3::Y))
            .user_data(surface as u128)
            .collision_groups(Self::world_groups())
            .build();
        self.world.colliders.insert(c);
    }

    /// A car body: mass, inertia and centre of mass from the spec, boxes
    /// for collision. `contact` lets it hit other cars.
    pub fn add_car(&mut self, spec: &CarSpec, pose: Pose3, contact: bool) -> RigidBodyHandle {
        let mp = MassProperties::new(
            Vec3::new(spec.cg[0] as f32, spec.cg[1] as f32, spec.cg[2] as f32),
            spec.mass as f32,
            Vec3::new(spec.inertia[0] as f32, spec.inertia[1] as f32, spec.inertia[2] as f32),
        );
        let rb = RigidBodyBuilder::dynamic()
            .pose(pose)
            .additional_mass_properties(mp)
            .angular_damping(0.05)
            .linear_damping(0.0)
            .can_sleep(false)
            .ccd_enabled(true)
            .build();
        let h = self.world.bodies.insert(rb);
        for b in &spec.boxes {
            let c = ColliderBuilder::cuboid(b[0] as f32 * 0.5, b[1] as f32 * 0.5, b[2] as f32 * 0.5)
                .translation(Vec3::new(b[3] as f32, b[4] as f32, b[5] as f32))
                .density(0.0)
                .friction(0.45)
                .friction_combine_rule(CoefficientCombineRule::Min)
                .restitution(0.05)
                .collision_groups(car_groups(contact))
                .build();
            self.world.colliders.insert_with_parent(c, h, &mut self.world.bodies);
        }
        h
    }

    pub fn set_car_contact(&mut self, body: RigidBodyHandle, contact: bool) {
        let colliders: Vec<_> = self.world.bodies[body].colliders().to_vec();
        for c in colliders {
            self.world.colliders[c].set_collision_groups(car_groups(contact));
        }
    }

    pub fn step(&mut self) {
        self.world.step();
    }
}

impl Default for Physics {
    fn default() -> Self {
        Physics::new()
    }
}

/// Cars collide with the world, and with each other unless ghosted.
fn car_groups(contact: bool) -> InteractionGroups {
    if contact {
        InteractionGroups::new(GROUP_CAR, GROUP_WORLD | GROUP_CAR, InteractionTestMode::And)
    } else {
        InteractionGroups::new(GROUP_GHOST, GROUP_WORLD, InteractionTestMode::And)
    }
}

#[cfg(test)]
mod tree_tuning_tests {
    use super::*;

    fn fixture() -> (Physics, Vec<TreeColliderSource>) {
        let mut physics=Physics::new();
        let sources=vec![TreeColliderSource{root:[4.,2.,8.],scale:[0.8,1.2]},
            TreeColliderSource{root:[14.,3.,18.],scale:[1.1,0.9]},
            TreeColliderSource{root:[40.,8.,80.],scale:[1.,1.]}];
        for source in &sources[..2] {
            physics.add_trunk(&builder::Spot{pos:builder::V3Ser(source.root[0],source.root[1],source.root[2]),
                yaw:0.,scale:source.scale[0],scale_y:source.scale[1]});
        }
        // An unrelated same-material cylinder must survive tree edits.
        physics.world.colliders.insert(ColliderBuilder::cylinder(0.5,0.3)
            .translation(Vec3::new(100.,1.,100.)).user_data(Surface::Dirt as u128)
            .collision_groups(Physics::world_groups()).build());
        (physics,sources)
    }

    #[test]
    fn tree_colliders_match_roots_scale_disable_and_restore_exactly() {
        let (mut physics,sources)=fixture();
        let baseline=TreeColliderBaseline::capture(&physics,&sources).unwrap();
        assert_eq!(baseline.len(),2,"far source has no invented physics");
        baseline.apply(&mut physics,1.5,&[true,false,true]).unwrap();
        for entry in &baseline.entries {
            let collider=&physics.world.colliders[entry.handle];
            let actual=collider.shape().as_cylinder().unwrap();
            let old=entry.original.shape().as_cylinder().unwrap();
            assert_eq!(actual.radius,old.radius*1.5);
            assert_eq!(actual.half_height,old.half_height*1.5);
            assert_eq!(collider.is_enabled(),entry.source==0);
            assert!(collider.translation().abs_diff_eq(entry.root+(entry.original.translation()-entry.root)*1.5,1e-6));
        }
        let unrelated=physics.world.colliders.iter().find(|(_,c)|c.translation().x==100.).unwrap().1;
        assert!(unrelated.is_enabled());assert_eq!(unrelated.shape().as_cylinder().unwrap().radius,0.3);
        baseline.apply(&mut physics,1.0,&[true;3]).unwrap();
        for entry in &baseline.entries {
            let actual=&physics.world.colliders[entry.handle];
            assert_eq!(actual.translation(),entry.original.translation());
            assert_eq!(actual.rotation(),entry.original.rotation());
            assert_eq!(actual.shape().as_cylinder().unwrap().radius,entry.original.shape().as_cylinder().unwrap().radius);
            assert_eq!(actual.shape().as_cylinder().unwrap().half_height,entry.original.shape().as_cylinder().unwrap().half_height);
            assert!(actual.is_enabled());
        }
    }

    #[test]
    fn invalid_tree_edit_is_atomic_and_ambiguous_matching_is_rejected() {
        let (mut physics,sources)=fixture();let baseline=TreeColliderBaseline::capture(&physics,&sources).unwrap();
        let first=baseline.entries[0].handle;
        let radius=physics.world.colliders[first].shape().as_cylinder().unwrap().radius;
        physics.world.colliders.get_mut(baseline.entries[1].handle).unwrap().user_data=ROAD;
        assert!(baseline.apply(&mut physics,1.5,&[false;3]).is_err());
        assert!(physics.world.colliders[first].is_enabled());
        assert_eq!(physics.world.colliders[first].shape().as_cylinder().unwrap().radius,radius);
        assert!(baseline.apply(&mut physics,f32::NAN,&[true;3]).is_err());
        let (physics,mut sources)=fixture();sources.push(sources[0]);
        assert!(TreeColliderBaseline::capture(&physics,&sources).is_err());
        let (physics,mut sources)=fixture();sources[0].scale[0]*=2.0;
        assert!(TreeColliderBaseline::capture(&physics,&sources).is_err());
    }
}
