//! Pixel-art player sprite sheets: a port of character.gd. A chibi body in
//! the player's clothing colour with their own drawn face on a big round
//! head (radius 12 at 1:1, so profile art keeps every pixel). Frames sit in
//! one row (see FRAMES); the pixels match Godot's.

use crate::godot::{rgba, Color};
use image::{Rgba, RgbaImage};

/// The frame leaves room around the 32x46 body for hats and profile art
/// (48x48 around the head). Drawing code uses body coordinates; OX/OY
/// shift them into the frame.
pub const W: u32 = 48;
pub const H: u32 = 62;
const OX: i32 = 8;
const OY: i32 = 16;
const HEAD: [f32; 2] = [16.0, 13.0];
const RADIUS: i32 = 12;
const INK: Color = crate::godot::rgb(0.043_137_256, 0.043_137_256, 0.070_588_24);
pub const FRAMES: [&str; 12] = ["idle0", "idle1", "walk0", "walk1", "walk2", "walk3", "jump", "fall", "sleep", "carry", "throw", "cheer"];
pub const HATS: [&str; 7] = ["none", "cap", "crown", "party", "tophat", "beanie", "headphones"];
pub const PORTRAIT: u32 = 32;

pub fn frame_index(name: &str) -> usize { FRAMES.iter().position(|&f| f == name).unwrap_or(0) }

/// Profile art and the point of it that sits on the head's centre.
pub struct Face<'a> {
    pub image: &'a RgbaImage,
    pub center: [f32; 2],
}

/// What a sheet is built from: profile colours, art, hat, and whether the
/// player holds a phone (joined on one).
#[derive(Default)]
pub struct Look<'a> {
    pub color: &'a str,
    pub skin: &'a str,
    pub face: Option<Face<'a>>,
    pub hat: &'a str,
    pub phone: bool,
}

/// The sprite sheet: FRAMES.len() frames of W x H in one row.
pub fn build(look: &Look) -> RgbaImage {
    let cloth = Color::from_string(look.color, Color::hex("7c5cff"));
    let skin = Color::from_string(look.skin, Color::hex("f5e9be"));
    let solid = look.face.as_ref().map(|f| solid(f.image));
    let face = look.face.as_ref().zip(solid.as_ref()).map(|(f, image)| Face { image, center: f.center });
    let mut sheet = clear(W * FRAMES.len() as u32, H);
    for (i, frame) in FRAMES.iter().enumerate() {
        let mut img = clear(W, H);
        draw_frame(&mut img, frame, cloth, skin, face.as_ref(), look.hat, look.phone);
        outline(&mut img);
        image::imageops::replace(&mut sheet, &img, i as i64 * W as i64, 0);
    }
    sheet
}

fn draw_frame(img: &mut RgbaImage, frame: &str, cloth: Color, skin: Color, face: Option<&Face>, hat: &str, phone: bool) {
    let mut bob = 0;
    let mut legs = [0, 0];
    let mut lift = [0, 0];
    let mut arms = ["down", "down"];
    let sleeping = frame == "sleep";
    match frame {
        "idle1" => bob = 1,
        "walk0" => legs = [2, -2],
        "walk1" => { lift = [0, 1]; bob = -1 }
        "walk2" => legs = [-2, 2],
        "walk3" => { lift = [1, 0]; bob = -1 }
        "jump" => { legs = [1, -1]; lift = [2, 0]; arms = ["up", "up"]; bob = -1 }
        "fall" => { legs = [-1, 1]; lift = [0, 1]; arms = ["out", "out"] }
        "sleep" => { bob = 1; arms = ["folded", "folded"] }
        "carry" => arms = ["forward", "forward"],
        "throw" => { arms = ["up", "forward"]; legs = [2, -1] }
        "cheer" => { arms = ["up", "up"]; bob = -1 }
        _ => {}
    }
    let dark = cloth.darkened(0.35);
    let light = cloth.lightened(0.25);
    let pants = Color::hex("2a2840");
    let shoe = Color::hex("15141f");
    // Tuck the knees while sleeping; the view lays this curled pose on its side.
    if sleeping {
        rect(img, 9, 35, 16, 5, pants);
        rect(img, 20, 38, 5, 5, pants);
        rect(img, 16, 41, 9, 3, shoe);
    } else {
        for side in 0..2 {
            let x = if side == 0 { 11 } else { 17 } + legs[side];
            let top = 36 + bob;
            let bottom = 44 - lift[side];
            rect(img, x, top, 4, bottom - top, pants);
            rect(img, x - if side == 0 { 1 } else { 0 }, bottom - 1, 5, 2, shoe);
        }
    }
    // Torso: a rounded jacket with a darker side and a highlight.
    let (tx, ty, tw, th) = (9, 25 + bob, 14, 12);
    rect(img, tx, ty, tw, th, cloth);
    rect(img, tx, ty, 2, th, dark);
    rect(img, tx + tw - 3, ty + 1, 2, th - 2, light);
    px(img, tx, ty + th - 1, Color::TRANSPARENT);
    px(img, tx + tw - 1, ty + th - 1, Color::TRANSPARENT);
    // Arms.
    for side in 0..2 {
        let x = if side == 0 { 6 } else { 23 };
        let y = ty + 1;
        let sleeve = if side == 0 { dark } else { cloth };
        match arms[side] {
            "down" => { rect(img, x, y, 3, 9, sleeve); rect(img, x, y + 9, 3, 2, skin); }
            "up" => { rect(img, x, y - 8, 3, 9, sleeve); rect(img, x, y - 10, 3, 2, skin); }
            "out" => {
                rect(img, x - if side == 0 { 3 } else { 0 }, y, 6, 3, sleeve);
                rect(img, if side == 0 { x - 5 } else { x + 6 }, y, 2, 3, skin);
            }
            "forward" => {
                rect(img, x, y, 3, 5, sleeve);
                rect(img, 23, y + 4, 7, 3, if side == 1 { cloth } else { dark });
                rect(img, 29, y + 4, 2, 3, skin);
            }
            _ => {}
        }
    }
    // Head: skin disc, the player's face art at 1:1, then the hat.
    let head = [HEAD[0], HEAD[1] + bob as f32];
    disc(img, head, skin, OX, OY);
    let (hx, hy) = (head[0] as i32, head[1] as i32);
    if let Some(face) = face {
        let origin = [(head[0] - face.center[0]) as i32 + OX, (head[1] - face.center[1]) as i32 + OY];
        blit_mask(img, face.image, face.image, origin);
    } else {
        // Same default face as the shared helpers.
        let eye_y = hy - 2;
        if sleeping {
            rect(img, hx - 1, eye_y, 3, 1, INK);
            rect(img, hx + 5, eye_y, 3, 1, INK);
        } else {
            rect(img, hx - 1, eye_y - 1, 2, 2, INK);
            rect(img, hx + 5, eye_y - 1, 2, 2, INK);
        }
        rect(img, hx + 2, hy + 5, 4, 1, INK);
    }
    if sleeping {
        // Bent forearms join the shoulders to the face, above custom artwork.
        rect(img, hx - 10, hy - 2, 5, 18, dark);
        rect(img, hx - 9, hy - 3, 9, 5, cloth);
        rect(img, hx + 7, hy + 1, 5, 14, dark);
        rect(img, hx + 3, hy + 1, 9, 5, cloth);
        rect(img, hx - 5, hy - 6, 9, 9, skin.darkened(0.06));
        rect(img, hx + 3, hy - 5, 9, 10, skin);
        rect(img, hx - 4, hy - 6, 7, 1, skin.lightened(0.14));
        rect(img, hx + 4, hy - 5, 7, 1, skin.lightened(0.14));
    }
    draw_hat(img, hat, head, cloth);
    if phone && !sleeping { draw_phone(img, arms[0], ty + 1, skin); }
}

/// One span per row: the pixels whose centres lie within RADIUS.
fn disc(img: &mut RgbaImage, head: [f32; 2], skin: Color, ox: i32, oy: i32) {
    let r = RADIUS as f32;
    for y in (head[1] as i32 - RADIUS - 1)..(head[1] as i32 + RADIUS + 2) {
        let dy = y as f32 + 0.5 - head[1];
        if dy * dy > r * r { continue; }
        let half = (r * r - dy * dy).sqrt();
        let from = (head[0] - half - 0.5).ceil() as i32;
        let to = (head[0] + half - 0.5).floor() as i32;
        fill(img, from + ox, y + oy, to - from + 1, 1, skin);
    }
}

/// A phone upright in the back hand, fingers wrapped over its lower edge.
fn draw_phone(img: &mut RgbaImage, arm: &str, y: i32, skin: Color) {
    let hand = match arm { "up" => [6, y - 10], "out" => [1, y], "forward" => [29, y + 4], _ => [6, y + 9] };
    let at = [hand[0] - 2, hand[1] - 5];
    rect(img, at[0], at[1], 5, 7, Color::hex("24273a"));
    rect(img, at[0] + 1, at[1] + 1, 3, 4, Color::hex("a8e6ff"));
    px(img, at[0] + 1, at[1] + 1, Color::hex("e4f6ff"));
    px(img, at[0] + 2, at[1] + 6, Color::hex("4a4f6a"));
    rect(img, hand[0] - 1, hand[1], 4, 2, skin);
}

fn draw_hat(img: &mut RgbaImage, hat: &str, head: [f32; 2], cloth: Color) {
    let x = head[0] as i32;
    let top = head[1] as i32 - RADIUS;
    match hat {
        "cap" => {
            rect(img, x - 9, top + 1, 18, 4, cloth.darkened(0.2));
            rect(img, x - 7, top - 1, 14, 2, cloth.darkened(0.2));
            rect(img, x + 6, top + 4, 9, 2, cloth.darkened(0.45));
        }
        "crown" => {
            let gold = Color::hex("ffcf5a");
            rect(img, x - 8, top - 1, 16, 4, gold);
            for i in 0..4 { rect(img, x - 8 + i * 5, top - 4, 2, 3, gold); }
            px(img, x - 1, top, Color::hex("ff4f8b"));
            px(img, x, top, Color::hex("ff4f8b"));
        }
        "party" => {
            for i in 0..10 { rect(img, x - 5 + i / 2, top + 2 - i, 10 - i, 1, if i % 3 != 0 { Color::hex("ff4f8b") } else { Color::hex("ffb454") }); }
            rect(img, x - 1, top - 10, 2, 2, Color::hex("3ddc97"));
        }
        "tophat" => {
            let black = Color::hex("1d1b2c");
            rect(img, x - 10, top + 1, 20, 2, black);
            rect(img, x - 6, top - 9, 12, 10, black);
            rect(img, x - 6, top - 2, 12, 2, Color::hex("7c5cff"));
        }
        "beanie" => {
            let wool = Color::hex("3ddc97");
            rect(img, x - 10, top + 1, 20, 5, wool);
            rect(img, x - 8, top - 2, 16, 3, wool);
            rect(img, x - 10, top + 4, 20, 2, wool.darkened(0.3));
            rect(img, x - 2, top - 5, 4, 3, Color::hex("f2f1f8"));
        }
        "headphones" => {
            let band = Color::hex("2b2b3d");
            rect(img, x - 10, top - 1, 20, 2, band);
            rect(img, x - 13, head[1] as i32 - 4, 4, 8, Color::hex("ffb454"));
            rect(img, x + 9, head[1] as i32 - 4, 4, 8, Color::hex("ffb454"));
        }
        _ => {}
    }
}

/// An image filled with Color.TRANSPARENT (white at zero alpha), as
/// Godot's `fill(Color.TRANSPARENT)` leaves it.
fn clear(w: u32, h: u32) -> RgbaImage { RgbaImage::from_pixel(w, h, Rgba(Color::TRANSPARENT.rgba8())) }

fn px(img: &mut RgbaImage, x: i32, y: i32, c: Color) {
    let (x, y) = (x + OX, y + OY);
    if x >= 0 && y >= 0 && x < img.width() as i32 && y < img.height() as i32 { img.put_pixel(x as u32, y as u32, Rgba(c.rgba8())); }
}

/// A rectangle in body coordinates, clipped to the frame.
fn rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Color) { fill(img, x + OX, y + OY, w, h, c); }

/// Image::fill_rect in image coordinates.
fn fill(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Color) {
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + w).min(img.width() as i32), (y + h).min(img.height() as i32));
    let p = Rgba(c.rgba8());
    for yy in y0..y1 { for xx in x0..x1 { img.put_pixel(xx as u32, yy as u32, p); } }
}

/// Image::blit_rect_mask: copy `src` where `mask` is not transparent.
fn blit_mask(dst: &mut RgbaImage, src: &RgbaImage, mask: &RgbaImage, at: [i32; 2]) {
    for (u, v, p) in src.enumerate_pixels() {
        let (x, y) = (at[0] + u as i32, at[1] + v as i32);
        if x < 0 || y < 0 || x >= dst.width() as i32 || y >= dst.height() as i32 { continue; }
        if mask.get_pixel(u, v)[3] != 0 { dst.put_pixel(x as u32, y as u32, *p); }
    }
}

/// Image::blend_rect at an offset: Color::blend per pixel, stored as RGBA8.
fn blend(dst: &mut RgbaImage, src: &RgbaImage, at: [i32; 2]) {
    for (u, v, p) in src.enumerate_pixels() {
        let (x, y) = (at[0] + u as i32, at[1] + v as i32);
        if x < 0 || y < 0 || x >= dst.width() as i32 || y >= dst.height() as i32 { continue; }
        let over = to_color(*p);
        if over.a <= 0.0 { continue; }
        let under = to_color(*dst.get_pixel(x as u32, y as u32));
        dst.put_pixel(x as u32, y as u32, Rgba(color_blend(under, over).rgba8()));
    }
}

fn to_color(p: Rgba<u8>) -> Color { rgba(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0) }

/// Godot's Color::blend (`over` on top).
fn color_blend(under: Color, over: Color) -> Color {
    let sa = 1.0 - over.a;
    let a = under.a * sa + over.a;
    if a == 0.0 { return rgba(0.0, 0.0, 0.0, 0.0); }
    let c = |u: f32, o: f32| (u * under.a * sa + o * over.a) / a;
    rgba(c(under.r, over.r), c(under.g, over.g), c(under.b, over.b), a)
}

/// One-pixel dark outline around every opaque shape: ink wherever a
/// neighbour is opaque, then the sprite back on top.
fn outline(img: &mut RgbaImage) {
    let source = img.clone();
    let (w, h) = img.dimensions();
    let mut grown = RgbaImage::new(w, h);
    for d in [[1, 0], [-1, 0], [0, 1], [0, -1]] { blend(&mut grown, &source, d); }
    let ink = RgbaImage::from_pixel(w, h, Rgba(INK.rgba8()));
    blit_mask(img, &ink, &grown, [0, 0]);
    blend(img, &source, [0, 0]);
}

/// Face art with its half-transparent pixels dropped.
fn solid(art: &RgbaImage) -> RgbaImage {
    let mut s = art.clone();
    for p in s.pixels_mut() { if (p[3] as f32 / 255.0) < 0.5 { *p = Rgba(Color::TRANSPARENT.rgba8()); } }
    s
}

/// Just the head, outlined, on a transparent square: waiting profiles hang
/// on the wall as these portraits.
pub fn portrait(skin: &str, face: Option<&Face>) -> RgbaImage {
    let mut img = clear(PORTRAIT, PORTRAIT);
    let skin = Color::from_string(skin, Color::hex("f5e9be"));
    let head = [PORTRAIT as f32 / 2.0; 2];
    disc(&mut img, head, skin, 0, 0);
    if let Some(face) = face {
        let art = solid(face.image);
        blit_mask(&mut img, &art, &art, [(head[0] - face.center[0]) as i32, (head[1] - face.center[1]) as i32]);
    } else {
        fill(&mut img, 15, 13, 2, 2, INK);
        fill(&mut img, 21, 13, 2, 2, INK);
        fill(&mut img, 18, 21, 4, 1, INK);
    }
    let mut outlined = clear(PORTRAIT, PORTRAIT);
    for d in [[1, 0], [-1, 0], [0, 1], [0, -1]] { blend(&mut outlined, &img, d); }
    let ink = RgbaImage::from_pixel(PORTRAIT, PORTRAIT, Rgba(INK.rgba8()));
    let mut result = RgbaImage::new(PORTRAIT, PORTRAIT);
    blit_mask(&mut result, &ink, &outlined, [0, 0]);
    blend(&mut result, &img, [0, 0]);
    result
}
