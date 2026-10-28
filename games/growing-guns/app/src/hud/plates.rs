//! Name tags over players, after the Godot player's `NameLabel` (a
//! billboarded Label3D 1.4 m above the body: 24 px text, 4 px outline in
//! the player's identity colour darkened by 45 %). Shown over bots and
//! remote players while they live and are not ghosts; never over the
//! players seated at this machine. Drawn as sprites, so walls hide them.
use super::style::*;
use super::Hud;
use growing_guns::World;
use render::hud::HudDrawList;
use render::sprites::{Canvas, Sprite, SpriteImage};

/// Godot `player.gd IDENTITY_COLORS`, by player index.
pub const IDENTITY_COLORS: [C; 8] = [
    [0.95, 0.26, 0.21, 1.0], [0.16, 0.55, 0.98, 1.0], [1.0, 0.8, 0.1, 1.0], [0.2, 0.82, 0.35, 1.0],
    [0.75, 0.3, 0.95, 1.0], [1.0, 0.55, 0.12, 1.0], [0.1, 0.85, 0.85, 1.0], [0.98, 0.4, 0.7, 1.0],
];

const IMAGE: SpriteImage = SpriteImage(0x6767_7461);
/// Canvas texels per Godot label pixel (crisp up close).
const TEXELS: f32 = 2.0;
/// One tag row: 32 label pixels tall, 320 wide.
const ROW: [u32; 2] = [640, 64];
/// Label3D's default pixel size: metres per label pixel.
const PIXEL_M: f32 = 0.005;
const MAX_TAGS: usize = 32;
/// Unshaded white, in the scene's nits.
const NITS: f32 = 9000.0;

#[derive(Default)]
pub struct Plates {
    canvas: Option<Canvas>,
    /// The (name, colour index) drawn into each row.
    rows: Vec<(String, usize)>,
}

impl Plates {
    /// The renderer was rebuilt: draw and bind the tags again.
    pub fn invalidate(&mut self) { self.rows.clear(); }

    /// The tags this frame: rows are redrawn when the names change.
    /// `pos`: each player's drawn (interpolated) position.
    pub fn update(&mut self, gpu: &render::Gpu, r: &render::Renderer, world: &World, hud: &Hud, locals: &[usize], pos: &[glam::Vec3]) -> Result<(), String> {
        let tagged: Vec<usize> = world.players.iter().enumerate().filter(|(i, p)| !locals.contains(i) && p.alive && !p.ghost && i < &pos.len()).map(|(i, _)| i).take(MAX_TAGS).collect();
        let rows: Vec<(String, usize)> = tagged.iter().map(|&i| (hud.name(i).to_string(), i)).collect();
        if rows != self.rows && !rows.is_empty() {
            if self.canvas.is_none() { self.canvas = Some(Canvas::new(gpu, [ROW[0], ROW[1] * MAX_TAGS as u32])?); }
            let canvas = self.canvas.as_mut().expect("made above");
            let size = [ROW[0], ROW[1] * MAX_TAGS as u32];
            let mut d = HudDrawList::new(size);
            for (k, (name, i)) in rows.iter().enumerate() {
                let edge = darkened(IDENTITY_COLORS[i % IDENTITY_COLORS.len()], 0.45);
                let cy = (k as f32 + 0.5) * ROW[1] as f32;
                let font = fit(name, BODY, 24.0 * TEXELS, ROW[0] as f32 - 12.0 * TEXELS);
                Label::new(BODY, font, BONE).outline(4.0 * TEXELS, edge).center().draw(&mut d, name, ROW[0] as f32 * 0.5, cy, [0.0, k as f32 * ROW[1] as f32, ROW[0] as f32, ROW[1] as f32]);
            }
            canvas.draw(gpu, &d)?;
            r.set_sprite_texture(gpu, IMAGE, canvas.texture(), render::hud::ImageFilter::Linear);
        }
        self.rows = rows;
        let (w, h) = (ROW[0] as f32 / TEXELS * PIXEL_M, ROW[1] as f32 / TEXELS * PIXEL_M);
        let total = (ROW[1] * MAX_TAGS as u32) as f32;
        let sprites: Vec<Sprite> = tagged.iter().enumerate().map(|(k, &i)| {
            let p = &world.players[i];
            let lift = 1.4 * p.weapon.body_scale.max(0.1) * p.weapon.body_scale_axes.y.max(0.1);
            let mut s = Sprite::billboard(pos[i] + glam::Vec3::Y * lift, [w, h], IMAGE, [0.0, k as f32 * ROW[1] as f32 / total, 1.0, ROW[1] as f32 / total], glam::Vec3::splat(NITS));
            s.alpha_cut = 0.0;
            s
        }).collect();
        r.set_sprites(gpu, &sprites)
    }
}
