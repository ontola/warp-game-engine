//! Profile changes repaint one cached atlas; the hull shader projects it onto paint.
use splitscreen_club::artwork::{avatar, hex_color};
use render::{
    Gpu, Renderer, Scene,
    hud::{HudDrawList, HudImage, ImageFilter},
    picture::Picture,
    screen::{SIZE, Screen},
};
use serde_json::{Value, json};
pub struct Faces {
    pub layer: i32,
    profiles: [Value; 6],
    portraits: [std::sync::Arc<Picture>; 6],
    pub revisions: [u32; 6],
    dirty: bool,
    screen: Option<Screen>,
    pub paints: Vec<Vec<usize>>,
}
impl Faces {
    pub fn new(scene: &mut Scene) -> Self {
        Self {
            layer: scene.add_screen_texture("pilot faces"),
            profiles: std::array::from_fn(|_| json!({})),
            portraits: std::array::from_fn(|_| std::sync::Arc::new(compose(&json!({})))),
            revisions: [0; 6],
            dirty: true,
            screen: None,
            paints: vec![],
        }
    }
    pub fn update(&mut self, scene: &mut Scene, i: usize, p: &Value) {
        if i >= 6 {
            return;
        }
        let next = json!({"avatar":p["avatar"],"skin_color":p["skin_color"],"color":p["color"]});
        if self.profiles[i] == next {
            return;
        }
        self.portraits[i] = std::sync::Arc::new(compose(&next));
        self.profiles[i] = next;
        self.revisions[i] += 1;
        self.dirty = true;
        let rgb = hex_color(p["color"].as_str().unwrap_or(""));
        if let Some(rgb) = rgb {
            let linear = rgb.map(|v| {
                let x = v as f32 / 255.;
                if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            });
            for &m in &self.paints[i] {
                scene.materials[m].tint = linear;
            }
        }
    }
    pub fn draw_hud(&self, list: &mut HudDrawList, i: usize, rect: [f32; 4]) {
        let s = (rect[2] / 800.).min(rect[3] / 450.);
        let id = HudImage(0x535246414345 + i as u64);
        list.provide_image(id, &self.portraits[i], ImageFilter::Nearest);
        list.image(
            [rect[0] + 3. * s, rect[1] + 2. * s, 38. * s, 38. * s],
            id,
            [0., 0., 1., 1.],
            [1., 1., 1., 0.93],
            rect,
        );
    }
    pub fn flush(&mut self, gpu: &Gpu, renderer: &Renderer, scene: &Scene) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        if self.screen.is_none() {
            self.screen = Some(Screen::new(gpu)?);
        }
        let screen = self.screen.as_mut().unwrap();
        let mut list = HudDrawList::new([SIZE, SIZE]);
        let cell = SIZE as f32 / 4.;
        for i in 0..6 {
            let id = HudImage(700 + i as u64);
            screen.set_image(gpu, id, &compose(&self.profiles[i]), ImageFilter::Nearest);
            list.image(
                [(i % 4) as f32 * cell, (i / 4) as f32 * cell, cell, cell],
                id,
                [0., 0., 1., 1.],
                [1.; 4],
                [0., 0., SIZE as f32, SIZE as f32],
            );
            for &m in &self.paints[i] {
                renderer.update_materials(gpu, m, &scene.materials[m..m + 1])?;
            }
        }
        screen.draw(gpu, renderer, self.layer, [0.; 4], &list)?;
        self.dirty = false;
        Ok(())
    }
}
pub fn compose(profile: &Value) -> Picture {
    let art = profile["avatar"].as_str().and_then(avatar);
    let side = art.as_ref().map_or(48, |a| a.width.max(a.height).max(48));
    let center = [side as f32 / 2., side as f32 / 2. + 4.];
    let skin = hex_color(profile["skin_color"].as_str().unwrap_or("")).unwrap_or([245, 233, 190]);
    let rim = hex_color(profile["color"].as_str().unwrap_or("")).unwrap_or([111, 229, 222]);
    let n = side * 4;
    let mut rgba = vec![0; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let at = [(x as f32 + 0.5) / 4., (y as f32 + 0.5) / 4.];
            let d = ((at[0] - center[0]).powi(2) + (at[1] - center[1]).powi(2)).sqrt();
            let mut px = [0; 4];
            if d < 13. {
                let c = if d < 12. { skin } else { rim };
                px = [c[0], c[1], c[2], ((13. - d).clamp(0., 1.) * 255.) as u8];
            }
            if let Some(a) = &art {
                let ax = (at[0] - center[0] + a.head[0]).floor() as i32;
                let ay = (at[1] - center[1] + a.head[1]).floor() as i32;
                if ax >= 0 && ay >= 0 && ax < a.width as i32 && ay < a.height as i32 {
                    let k = ((ay as u32 * a.width + ax as u32) * 4) as usize;
                    if a.rgba[k + 3] > 0 {
                        px.copy_from_slice(&a.rgba[k..k + 4]);
                    }
                }
            } else {
                let q = [at[0] - center[0], at[1] - center[1]];
                if ((-5. ..-2.).contains(&q[0]) || (3. ..6.).contains(&q[0]))
                    && (-3. ..0.).contains(&q[1])
                    || (-3. ..4.).contains(&q[0]) && (5. ..6.).contains(&q[1])
                {
                    px = [26, 32, 51, 255];
                }
            }
            let k = ((y * n + x) * 4) as usize;
            rgba[k..k + 4].copy_from_slice(&px);
        }
    }
    Picture::new(n, n, rgba).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skin_and_hat_keep_head_origin_and_live_changes() {
        let mut px = vec![Value::Null; 48 * 48];
        px[24] = json!("#ff0000");
        let p = json!({"avatar":json!({"v":1,"w":48,"h":48,"px":px}).to_string(),"skin_color":"#123456"});
        let a = compose(&p);
        let pixel = |x: usize, y: usize| {
            &a.rgba[(y * a.width as usize + x) * 4..(y * a.width as usize + x) * 4 + 4]
        };
        assert_eq!(pixel(24 * 4, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(24 * 4, 28 * 4), [18, 52, 86, 255]);
        assert_eq!(pixel(0, 0), [0; 4]);
        assert_ne!(a, compose(&json!({"skin_color":"#ffffff"})));
        assert_eq!(compose(&json!({"avatar":"invalid"})).width, 192);
    }
}
