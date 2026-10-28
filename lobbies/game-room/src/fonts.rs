//! The lobby's faces: Outfit for all UI text, Bungee for the wordmark.

use render::hud::FontFace;
use std::sync::OnceLock;

static OUTFIT_600: &[u8] = include_bytes!("../assets/fonts/outfit-600.ttf");
static OUTFIT_800: &[u8] = include_bytes!("../assets/fonts/outfit-800.ttf");
static BUNGEE: &[u8] = include_bytes!("../assets/fonts/bungee-regular.ttf");

pub struct Faces {
    pub regular: FontFace,
    pub bold: FontFace,
    pub wordmark: FontFace,
}

pub fn faces() -> &'static Faces {
    static FACES: OnceLock<Faces> = OnceLock::new();
    FACES.get_or_init(|| Faces {
        regular: render::hud::register_font(OUTFIT_600),
        bold: render::hud::register_font(OUTFIT_800),
        wordmark: render::hud::register_font(BUNGEE),
    })
}

pub fn measure_bold(text: &str, size: f32) -> f32 {
    render::hud::measure(text, faces().bold, size)[0]
}

pub fn measure(text: &str, size: f32) -> f32 {
    render::hud::measure(text, faces().regular, size)[0]
}
