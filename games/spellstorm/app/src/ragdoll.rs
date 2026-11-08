//! A fallen wizard as a ragdoll (Joep): six capsules (body, head, arms,
//! legs) from the kit's pieces as they stood when the wizard died, thrown
//! with the body's momentum and the killing blow. Only the view: the
//! simulation never sees it.
use crate::kits::Kit;
use crate::look::Piece;
use glam::{Mat4, Vec3};
use kern::collide::YawBox;
pub use physics::ragdoll::Segment;
use physics::ragdoll::Ragdoll;
use physics::Physics;
use render::scene::MeshRef;

/// Body, head, left arm, right arm, left leg, right leg.
const PARTS: [&str; 6] = ["body", "hat", "arm_l", "arm_r", "leg_l", "leg_r"];
const STEP: f32 = 1.0 / 60.0;

pub struct Doll {
    rig: Ragdoll,
    /// Each piece's world transform when it fell, and the part it rides.
    initial: Vec<Mat4>,
    parts: Vec<usize>,
}

/// The physics the fallen lie in: a floor and nothing else (walls hold
/// the living; the fallen do not slide far).
pub struct Dolls {
    phys: Physics,
    acc: f32,
    pub dolls: Vec<Option<Doll>>,
}

impl Default for Dolls {
    fn default() -> Self {
        let mut phys = Physics::new(Vec3::new(0.0, -20.0, 0.0), 1.0 / STEP);
        phys.add_static(&YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(4000.0, 1.0, 4000.0), 0.0));
        Dolls { phys, acc: 0.0, dolls: Vec::new() }
    }
}

/// Which part a node rides: its nearest named ancestor among PARTS; the
/// staff goes with the right hand, the sword with the left.
fn part_of(kit: &Kit, node: usize) -> usize {
    let named = |name: &str| kit.tree.node(name);
    for j in kit.tree.ancestors(node) {
        if named("staff") == Some(j) {
            return 3;
        }
        if named("sword") == Some(j) {
            return 2;
        }
        if let Some(p) = PARTS.iter().position(|&x| named(x) == Some(j)) {
            return p;
        }
    }
    0
}

impl Dolls {
    /// Advance by `dt` of drawing time; returns how far between the last two
    /// steps to draw.
    pub fn step(&mut self, dt: f32) -> f32 {
        if self.dolls.iter().all(Option::is_none) {
            self.acc = 0.0;
            return 1.0;
        }
        self.acc += dt;
        let mut n = 0;
        while self.acc >= STEP && n < 4 {
            self.phys.step();
            for d in self.dolls.iter_mut().flatten() {
                d.rig.update(&self.phys);
            }
            self.acc -= STEP;
            n += 1;
        }
        self.acc = self.acc.min(STEP);
        self.acc / STEP
    }

    /// Up again (or gone): its bodies leave the world.
    pub fn clear(&mut self, i: usize) {
        if let Some(Some(mut d)) = self.dolls.get_mut(i).map(Option::take) {
            d.rig.remove(&mut self.phys);
        }
    }

    /// Wizard `i` falls: nodes at `node`, pieces at `xfs` (world), moving at `vel`, struck
    /// along `blow` as hard as `force`.
    #[allow(clippy::too_many_arguments)]
    pub fn fall(&mut self, i: usize, kit: &Kit, node: &dyn Fn(usize) -> Mat4, pieces: &[Piece], xfs: &[Mat4], meshes: &[MeshRef], vel: Vec3, blow: Vec3, force: f32) {
        self.clear(i);
        if self.dolls.len() <= i {
            self.dolls.resize_with(i + 1, || None);
        }
        let parts: Vec<usize> = pieces.iter().map(|p| part_of(kit, p.node)).collect();
        // Each part's pivot (its node) and how far its pieces reach from it.
        let pivot = |k: usize| kit.tree.node(PARTS[k]).map(|n| node(n).w_axis.truncate()).unwrap_or(xfs[0].w_axis.truncate());
        let mut lo = [Vec3::splat(f32::INFINITY); 6];
        let mut hi = [Vec3::splat(f32::NEG_INFINITY); 6];
        for ((p, x), &k) in pieces.iter().zip(xfs).zip(&parts) {
            let Some(m) = meshes.get(p.mesh) else { continue };
            let c = x.transform_point3(Vec3::new(m.sphere[0], m.sphere[1], m.sphere[2]));
            if k == 3 && kit.tree.node("staff").is_some_and(|s| kit.tree.ancestors(p.node).any(|j| j == s)) {
                continue;
            }
            lo[k] = lo[k].min(c);
            hi[k] = hi[k].max(c);
        }
        let base = xfs[0].w_axis.truncate();
        let body_lo = if lo[0].is_finite() { lo[0] } else { base + Vec3::Y * 0.5 };
        let body_hi = if hi[0].is_finite() { hi[0] } else { base + Vec3::Y * 1.3 };
        let torso = Segment { start: Vec3::new(body_lo.x, body_lo.y.max(base.y + 0.45), body_lo.z), end: Vec3::new(body_hi.x, body_hi.y.max(base.y + 1.1), body_hi.z), radius: 0.24, parent: None, anchor: base };
        let mut segs = vec![torso];
        for (k, radius) in [(1, 0.16), (2, 0.08), (3, 0.08), (4, 0.1), (5, 0.1)] {
            let a = pivot(k);
            let far = if lo[k].is_finite() {
                // The end of the part farthest from its pivot.
                let (l, h) = (lo[k], hi[k]);
                let cands = [l, h, Vec3::new(l.x, h.y, l.z), Vec3::new(h.x, l.y, h.z)];
                cands.into_iter().max_by(|p, q| p.distance(a).total_cmp(&q.distance(a))).unwrap()
            } else {
                a + Vec3::NEG_Y * 0.4
            };
            let end = if far.distance(a) < 0.15 { a + (far - a).normalize_or(Vec3::NEG_Y) * 0.25 } else { far };
            segs.push(Segment { start: a, end, radius, parent: Some(0), anchor: a });
        }
        self.throw(i, &segs, parts, xfs, vel, blow, force);
    }

    /// The first slot under `cap` with nobody in it.
    pub fn free(&self, cap: usize) -> Option<usize> {
        (0..cap).find(|&i| self.dolls.get(i).is_none_or(Option::is_none))
    }

    /// Body `i` goes limp as the capsules `segs` (parent before child),
    /// piece `j` riding segment `parts[j]` from where it stands at `xfs[j]`:
    /// thrown with `vel`, struck along `blow` as hard as `force`.
    #[allow(clippy::too_many_arguments)]
    pub fn throw(&mut self, i: usize, segs: &[Segment], parts: Vec<usize>, xfs: &[Mat4], vel: Vec3, blow: Vec3, force: f32) -> bool {
        self.clear(i);
        if self.dolls.len() <= i {
            self.dolls.resize_with(i + 1, || None);
        }
        let blow = Vec3::new(blow.x, 0.0, blow.z).normalize_or_zero();
        let spin = Vec3::Y.cross(blow) * (2.5 + force * 0.5);
        let v = vel + blow * force + Vec3::Y * (1.0 + force * 0.25);
        let Ok(rig) = Ragdoll::spawn(&mut self.phys, segs, v, spin, 1u32 << (i % 32)) else { return false };
        self.dolls[i] = Some(Doll { rig, initial: xfs.to_vec(), parts });
        true
    }

    /// Where piece `j` of fallen wizard `i` is now.
    pub fn piece(&self, i: usize, j: usize, alpha: f32) -> Option<Mat4> {
        let d = self.dolls.get(i)?.as_ref()?;
        Some(d.rig.delta(*d.parts.get(j)?, alpha) * *d.initial.get(j)?)
    }
}
