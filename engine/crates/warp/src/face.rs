//! A player's GameNight face for the HUD: the profile's pixel art (the
//! `avatar` payload) over a round head in their skin colour, rimmed in their
//! lobby colour, as `hud.face` draws it. Without art (or outside GameNight)
//! a plain head with two eyes and a mouth.
use gamenight::artwork::{avatar, hex_color};

/// Square, `side` pixels a row, RGBA (alpha 0: nothing drawn).
#[derive(Clone, Debug, PartialEq)]
pub struct Face { pub side: u32, pub px: Vec<[u8; 4]> }

pub fn compose(art: &str, skin: &str, rim: [f32; 3]) -> Face {
    let art = avatar(art);
    let side = art.as_ref().map_or(48, |a| a.width.max(a.height).max(48));
    let center = [side as f32 / 2.0, side as f32 / 2.0 + 4.0];
    let skin = hex_color(skin).unwrap_or([236, 196, 160]);
    let rim = rim.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8);
    let mut px = vec![[0u8; 4]; (side * side) as usize];
    for y in 0..side {
        for x in 0..side {
            let at = [x as f32 + 0.5, y as f32 + 0.5];
            let q = [at[0] - center[0], at[1] - center[1]];
            let d = (q[0] * q[0] + q[1] * q[1]).sqrt();
            let mut p = [0u8; 4];
            if d < 13.0 {
                let c = if d < 11.8 { skin } else { rim };
                p = [c[0], c[1], c[2], 255];
            }
            if let Some(a) = &art {
                let ax = (q[0] + a.head[0]).floor() as i32;
                let ay = (q[1] + a.head[1]).floor() as i32;
                if ax >= 0 && ay >= 0 && ax < a.width as i32 && ay < a.height as i32 {
                    let k = ((ay as u32 * a.width + ax as u32) * 4) as usize;
                    if a.rgba[k + 3] > 0 { p.copy_from_slice(&a.rgba[k..k + 4]); }
                }
            } else if ((-5.0..-2.0).contains(&q[0]) || (3.0..6.0).contains(&q[0])) && (-3.0..0.0).contains(&q[1])
                || (-3.0..4.0).contains(&q[0]) && (5.0..6.0).contains(&q[1])
            {
                p = [26, 32, 51, 255];
            }
            px[(y * side + x) as usize] = p;
        }
    }
    Face { side, px }
}

impl Face {
    /// Rectangles (x, y, w, h in face pixels, colour 0..1) covering the face:
    /// runs along each row, so a HUD draws it with boxes.
    pub fn runs(&self) -> Vec<([f32; 4], [f32; 4])> {
        let mut out = Vec::new();
        for y in 0..self.side {
            let mut x = 0;
            while x < self.side {
                let p = self.px[(y * self.side + x) as usize];
                let mut n = 1;
                while x + n < self.side && self.px[(y * self.side + x + n) as usize] == p { n += 1; }
                if p[3] > 0 {
                    out.push(([x as f32, y as f32, n as f32, 1.0], [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]));
                }
                x += n;
            }
        }
        out
    }

    /// The square that holds anything drawn (the art can stand above the head).
    pub fn bounds(&self) -> [u32; 4] {
        let (mut x0, mut y0, mut x1, mut y1) = (self.side, self.side, 0, 0);
        for y in 0..self.side {
            for x in 0..self.side {
                if self.px[(y * self.side + x) as usize][3] > 0 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x + 1); y1 = y1.max(y + 1); }
            }
        }
        if x1 <= x0 { return [0, 0, self.side, self.side]; }
        let s = (x1 - x0).max(y1 - y0);
        let cx = (x0 + x1) / 2;
        let cy = (y0 + y1) / 2;
        [cx.saturating_sub(s / 2), cy.saturating_sub(s / 2), s, s]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_head_without_art_and_the_art_over_it() {
        let plain = compose("", "#aabbcc", [1.0, 0.0, 0.0]);
        assert_eq!(plain.side, 48);
        assert_eq!(plain.px[(28 * 48 + 24) as usize], [170, 187, 204, 255]);
        assert!(!plain.runs().is_empty());
        let mut px = vec![serde_json::Value::Null; 48 * 48];
        px[0] = "#ff0000".into();
        let art = serde_json::json!({ "v": 1, "w": 48, "h": 48, "px": px }).to_string();
        let face = compose(&art, "#aabbcc", [1.0, 0.0, 0.0]);
        assert_ne!(face, plain);
    }
}
