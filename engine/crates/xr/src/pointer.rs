//! The head-locked HUD quad and the controller as a pointer on it.
//!
//! `android.rs` shows the game's HUD on one quad, `HUD_DISTANCE` ahead of the
//! head and `HUD_METRES` large. While a menu is up a game turns the
//! controller's aim ray into HUD pixels with `hud_point` and draws `cursor`.
use glam::Vec3;
use crate::Pose;

/// HUD swapchain size in pixels; games lay their HUD out at this size.
pub const HUD_SIZE: [u32; 2] = [1280, 720];
/// The quad sits this far ahead of the head, metres.
pub const HUD_DISTANCE: f32 = 2.0;
/// The quad's width and height, metres.
pub const HUD_METRES: [f32; 2] = [2.4, 1.35];

/// Where a controller ray meets the head-locked HUD, in HUD pixels.
pub fn hud_point(head: Pose, aim: Pose) -> Option<[f32; 2]> {
    let inverse = head.orientation.inverse();
    let origin = inverse * (aim.position - head.position);
    let direction = inverse * (aim.orientation * Vec3::NEG_Z);
    if direction.z >= -1e-3 { return None; }
    let hit = origin + direction * ((-HUD_DISTANCE - origin.z) / direction.z);
    let (u, v) = (hit.x / HUD_METRES[0] + 0.5, 0.5 - hit.y / HUD_METRES[1]);
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then(|| [u * HUD_SIZE[0] as f32, v * HUD_SIZE[1] as f32])
}

/// The pointer dot, drawn over everything else on the HUD.
pub fn cursor(hud: &mut render::hud::HudDrawList, at: [f32; 2]) {
    let clip = [0.0, 0.0, HUD_SIZE[0] as f32, HUD_SIZE[1] as f32];
    hud.begin_overlay();
    hud.circle(at, 7.0, [0.0, 0.0, 0.0, 0.7], clip);
    hud.circle(at, 5.0, [1.0, 0.85, 0.35, 1.0], clip);
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    fn pose(position: Vec3, orientation: Quat) -> Pose { Pose { position, orientation } }

    #[test]
    fn the_controller_points_at_hud_pixels() {
        let head = pose(Vec3::new(0.0, 1.6, 0.0), Quat::IDENTITY);
        // Straight ahead from the head: the middle of the HUD.
        let [x, y] = hud_point(head, pose(head.position, Quat::IDENTITY)).unwrap();
        assert!((x - 640.0).abs() < 0.01 && (y - 360.0).abs() < 0.01);
        // A hand to the right and low, pointing ahead, lands right and low.
        let [x, y] = hud_point(head, pose(head.position + Vec3::new(0.6, -0.3, 0.0), Quat::IDENTITY)).unwrap();
        assert!((x - 960.0).abs() < 0.01 && (y - 520.0).abs() < 0.01);
        // The HUD turns with the head.
        let turned = pose(head.position, Quat::from_rotation_y(1.0));
        assert!(hud_point(turned, pose(head.position, Quat::from_rotation_y(1.0))).is_some());
        // Away from it, or past its edge: no pointer.
        assert_eq!(hud_point(head, pose(head.position, Quat::from_rotation_y(3.0))), None);
        assert_eq!(hud_point(head, pose(head.position, Quat::from_rotation_y(1.2))), None);
    }
}
