//! Player sprite sheets match character.gd pixel for pixel. Regenerate the
//! fixtures in gamenight's lobbies/game-room with
//! `godot --headless --path . -s tests/warp_sprites.gd -- DIR` and copy
//! DIR/*.png to tests/sprites.

use game_room::character::{build, portrait, Face, Look, HATS};
use game_room::godot::rgba;
use image::{Rgba, RgbaImage};

/// The made-up face of warp_sprites.gd, soft edges included.
fn test_face() -> RgbaImage {
    let mut img = RgbaImage::new(22, 20);
    for y in 0..20 {
        for x in 0..22 {
            let length = glam::Vec2::new(x as f32 - 11.0, y as f32 - 10.0).length() as f64;
            let a = (1.2 - length / 9.0).clamp(0.0, 1.0) as f32;
            img.put_pixel(x, y, Rgba(rgba((x as f64 / 21.0) as f32, (y as f64 / 19.0) as f32, 0.4, a).rgba8()));
        }
    }
    img
}

fn same(name: &str, ours: &RgbaImage) {
    let godot = image::open(format!("{}/tests/sprites/{name}.png", env!("CARGO_MANIFEST_DIR"))).unwrap().to_rgba8();
    assert_eq!(godot.dimensions(), ours.dimensions(), "{name}: size");
    let diff: Vec<_> = godot.enumerate_pixels().filter(|(x, y, p)| ours.get_pixel(*x, *y) != *p).take(5)
        .map(|(x, y, p)| format!("({x},{y}) godot {:?} rust {:?}", p.0, ours.get_pixel(x, y).0)).collect();
    assert!(diff.is_empty(), "{name} differs: {}", diff.join(", "));
}

#[test]
fn sheets_match_godot() {
    let face = test_face();
    let center = [11.4, 9.7];
    same("plain", &build(&Look::default()));
    same("colours", &build(&Look { color: "#3ddc97", skin: "#8d5524", hat: "cap", phone: true, ..Default::default() }));
    same("face", &build(&Look { color: "#ff4f8b", face: Some(Face { image: &face, center }), hat: "crown", ..Default::default() }));
    for (i, hat) in HATS.iter().enumerate() {
        same(&format!("hat_{hat}"), &build(&Look { color: ["#ffb454", "#38d6ff", "#7c5cff"][i % 3], hat, phone: i % 2 == 0, ..Default::default() }));
    }
    same("portrait_plain", &portrait("", None));
    same("portrait_face", &portrait("#8d5524", Some(&Face { image: &face, center })));
}
