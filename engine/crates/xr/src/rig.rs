//! The play space: where the real room sits in the game world.
//!
//! OpenXR reports poses in the room (stage space). A game places that room in
//! its world with an origin (the head position the room is anchored to) and a
//! yaw. Turning rotates the room about the current head, so the player's view
//! turns without their head moving through the world. Game-independent; the
//! game decides where the anchor is (a player's eye, a cockpit seat).
use glam::{Affine3A, Quat, Vec3};
use crate::{Pose, Tracking};

/// Thumbstick past this snaps once; it must return below `SNAP_RELEASE` first.
pub const SNAP_PRESS: f32 = 0.7;
pub const SNAP_RELEASE: f32 = 0.3;
/// Default snap-turn step.
pub const SNAP_DEGREES: f32 = 30.0;

#[derive(Clone, Debug)]
pub struct PlaySpace {
    origin: Option<Vec3>,
    /// World yaw of the room, radians.
    pub yaw: f32,
    snap_ready: bool,
    /// Snap-turn step in degrees.
    pub snap_degrees: f32,
}

impl PlaySpace {
    /// `yaw`: the world heading the player should look along when the
    /// headset first reports a pose.
    pub fn new(yaw: f32) -> Self { Self { origin: None, yaw, snap_ready: true, snap_degrees: SNAP_DEGREES } }

    /// Whether the room has been placed (the first tracked frame happened).
    pub fn placed(&self) -> bool { self.origin.is_some() }

    /// Forget the placement: the next `update` places the room again.
    pub fn reset(&mut self) { self.origin = None; }

    /// Place the room on the first frame, honour a runtime recenter and snap
    /// turn on the thumbstick. Call once per rendered frame before `anchor`.
    pub fn update(&mut self, t: &Tracking) {
        if self.origin.is_none() {
            // Whatever way the head faces now becomes the requested heading.
            self.yaw -= room_heading(t.head);
            self.origin = Some(t.head.position);
        } else if t.recentered {
            self.origin = Some(t.head.position);
        }
        if t.focused && t.turn.abs() > SNAP_PRESS && self.snap_ready {
            self.turn(t.head.position, -t.turn.signum() * self.snap_degrees.to_radians());
            self.snap_ready = false;
        }
        if t.turn.abs() < SNAP_RELEASE { self.snap_ready = true; }
    }

    /// Turn the room about the head at `head` by `by` radians, keeping the
    /// head's world position.
    pub fn turn(&mut self, head: Vec3, by: f32) {
        let origin = self.origin.unwrap_or(head);
        let old = Quat::from_rotation_y(self.yaw) * (head - origin);
        self.yaw += by;
        self.origin = Some(head - Quat::from_rotation_y(-self.yaw) * old);
    }

    /// World yaw the head looks along.
    pub fn heading(&self, head: Pose) -> f32 {
        let f = Quat::from_rotation_y(self.yaw) * head.orientation * Vec3::NEG_Z;
        (-f.x).atan2(-f.z)
    }

    /// Turn so that the head looks along world yaw `target`.
    pub fn face(&mut self, head: Pose, target: f32) {
        let pi = std::f32::consts::PI;
        let by = (target - self.heading(head) + pi).rem_euclid(std::f32::consts::TAU) - pi;
        self.turn(head.position, by);
    }

    /// World position of the room's origin when the anchored head is at `eye`.
    pub fn anchor(&self, eye: Vec3) -> Vec3 {
        eye - Quat::from_rotation_y(self.yaw) * self.origin.unwrap_or(Vec3::ZERO)
    }

    /// A room pose (eye, controller) in the world, anchored at `eye`.
    pub fn place(&self, pose: Pose, eye: Vec3) -> Pose { pose.transformed(self.anchor(eye), self.yaw) }

    /// World forward of the head.
    pub fn forward(&self, head: Pose) -> Vec3 { Quat::from_rotation_y(self.yaw) * head.orientation * Vec3::NEG_Z }

    /// A room pose carried by a vehicle: the room's origin sits at `seat`, a
    /// world transform whose -Z looks ahead and +Y is up. The room's own yaw
    /// (snap turn) still applies inside the seat.
    pub fn place_on(&self, pose: Pose, seat: Affine3A) -> Pose {
        let local = Quat::from_rotation_y(self.yaw);
        let (_, rotation, _) = seat.to_scale_rotation_translation();
        let offset = local * (pose.position - self.origin.unwrap_or(pose.position));
        Pose { position: seat.transform_point3(offset), orientation: rotation * local * pose.orientation }
    }
}

/// A seat that keeps the horizon level: only the vehicle's heading turns the
/// player, not its pitch and roll. Comfort option for fast vehicles.
pub fn level_seat(seat: Affine3A) -> Affine3A {
    let ahead = seat.transform_vector3(Vec3::NEG_Z);
    let flat = Vec3::new(ahead.x, 0.0, ahead.z);
    let yaw = if flat.length_squared() > 1e-8 { (-flat.x).atan2(-flat.z) } else { 0.0 };
    Affine3A::from_rotation_translation(Quat::from_rotation_y(yaw), seat.translation.into())
}

/// Yaw of the head in room space.
fn room_heading(head: Pose) -> f32 {
    let f = head.orientation * Vec3::NEG_Z;
    (-f.x).atan2(-f.z)
}

/// Yaw and pitch of a world direction.
pub fn yaw_pitch(d: Vec3) -> [f32; 2] { [(-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin()] }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Eye;
    fn tracking(head: Pose, turn: f32) -> Tracking {
        let eye = Eye { pose: head, fov: [-0.8, 0.8, 0.8, -0.8] };
        Tracking { eyes: [eye; 2], head, aim: None, controls: Default::default(), turn, focused: true, recentered: false }
    }

    #[test]
    fn first_frame_faces_the_requested_heading() {
        let head = Pose { position: Vec3::new(0.3, 1.6, -0.2), orientation: Quat::from_rotation_y(0.8) };
        let mut space = PlaySpace::new(1.2);
        space.update(&tracking(head, 0.0));
        assert!((space.heading(head) - 1.2).abs() < 1e-5);
        // The head itself sits at the anchor.
        let at = space.place(head, Vec3::new(5.0, 2.0, 3.0));
        assert!(at.position.distance(Vec3::new(5.0, 2.0, 3.0)) < 1e-5);
    }

    #[test]
    fn snap_turns_once_per_push_and_keeps_the_head_in_place() {
        let head = Pose { position: Vec3::new(0.4, 1.6, 0.5), orientation: Quat::IDENTITY };
        let mut space = PlaySpace::new(0.0);
        space.update(&tracking(head, 0.0));
        let moved = Pose { position: head.position + Vec3::new(0.2, 0.0, -0.3), ..head };
        let before = space.place(moved, Vec3::ZERO).position;
        space.update(&tracking(moved, 0.9));
        space.update(&tracking(moved, 0.9));
        assert!((space.yaw + 30f32.to_radians()).abs() < 1e-5, "one step while held");
        assert!(space.place(moved, Vec3::ZERO).position.distance(before) < 1e-5);
        space.update(&tracking(moved, 0.0));
        space.update(&tracking(moved, -0.9));
        assert!(space.yaw.abs() < 1e-5);
    }

    #[test]
    fn a_seat_carries_the_head_and_a_level_seat_drops_roll() {
        let head = Pose { position: Vec3::new(0.2, 1.2, 0.1), orientation: Quat::IDENTITY };
        let mut space = PlaySpace::new(0.0);
        space.update(&tracking(head, 0.0));
        let roll = Quat::from_rotation_y(0.7) * Quat::from_rotation_z(0.5);
        let seat = Affine3A::from_rotation_translation(roll, Vec3::new(10.0, 3.0, -4.0));
        let at = space.place_on(head, seat);
        assert!(at.position.distance(Vec3::new(10.0, 3.0, -4.0)) < 1e-5, "the tracked head sits on the seat");
        assert!((at.orientation * Vec3::Y - roll * Vec3::Y).length() < 1e-5, "rolls with the vehicle");
        let level = space.place_on(head, level_seat(seat));
        assert!((level.orientation * Vec3::Y - Vec3::Y).length() < 1e-5, "level seat keeps the horizon");
        assert!(((level.orientation * Vec3::NEG_Z).x - (roll * Vec3::NEG_Z).x).abs() < 0.2);
        // Leaning 10 cm right moves the eye 10 cm along the seat's right.
        let lean = space.place_on(Pose { position: head.position + Vec3::X * 0.1, ..head }, seat);
        assert!((lean.position - at.position - roll * Vec3::X * 0.1).length() < 1e-5);
    }

    #[test]
    fn face_turns_the_shortest_way() {
        let head = Pose { position: Vec3::new(0.0, 1.6, 0.0), orientation: Quat::from_rotation_y(0.3) };
        let mut space = PlaySpace::new(3.0);
        space.update(&tracking(head, 0.0));
        space.face(head, -3.0);
        assert!((space.heading(head) - (-3.0)).abs() < 1e-5);
        assert!((yaw_pitch(space.forward(head))[0] - space.heading(head)).abs() < 1e-5);
    }
}
