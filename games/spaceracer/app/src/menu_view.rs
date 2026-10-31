//! Compact source-style start/pause panel with the retained SpaceRacer logo.
use render::{
    hud::{HudDrawList, HudImage, ImageFilter, TextAlign},
    picture::Picture,
};
use std::sync::{Arc, OnceLock};
pub fn draw(hud: &mut HudDrawList, size: [f32; 2], row: usize, r: &super::Racer) {
    let [w, h] = size;
    let clip = [0., 0., w, h];
    let first = hud.vertices.len();
    hud.rect(clip, [0.015, 0.025, 0.045, 1.], clip);
    for vertex in &mut hud.vertices[first..] {
        vertex.color[3] = 0.93 - 0.85 * (vertex.position[0] / w);
    }
    let x = (w * 0.065).max(36.);
    let width = 480_f32.min(w - x * 2.);
    static LOGO: OnceLock<Arc<Picture>> = OnceLock::new();
    let logo = LOGO.get_or_init(|| {
        Arc::new(
            render::picture::decode(
                include_bytes!("../../assets/source/assets/branding/logo.png"),
                1.,
            )
            .expect("bundled logo"),
        )
    });
    let id = HudImage(0x53524c4f474f);
    hud.provide_image(id, logo, ImageFilter::Linear);
    hud.image(
        [
            x,
            55.,
            width,
            width * logo.height as f32 / logo.width as f32,
        ],
        id,
        [0., 0., 1., 1.],
        [1.; 4],
        clip,
    );
    let labels = [
        if r.race.clock > 0. {
            "Resume race"
        } else {
            "Race"
        },
        "New random race",
        "Players",
        "World",
        "Difficulty",
        "Track seed",
    ];
    let values = [
        String::new(),
        String::new(),
        r.players.to_string(),
        super::BIOMES[r.biome].to_uppercase(),
        super::DIFFICULTIES[r.difficulty].to_uppercase(),
        format!("{:05}", r.seed),
    ];
    for i in 0..6 {
        let y = 180. + i as f32 * 66.;
        let selected = i == row;
        let color = if selected {
            [0.49, 0.91, 0.84, 0.22]
        } else {
            [1., 1., 1., 0.045]
        };
        hud.rect([x, y, width, 54.], color, clip);
        let border = if selected {
            [0.49, 0.91, 0.84, 0.85]
        } else {
            [1., 1., 1., 0.12]
        };
        for (a, b) in [
            ([x, y], [x + width, y]),
            ([x + width, y], [x + width, y + 54.]),
            ([x + width, y + 54.], [x, y + 54.]),
            ([x, y + 54.], [x, y]),
        ] {
            hud.line(a, b, 1., border, clip);
        }
        if i < 2 {
            super::text(
                hud,
                10 + i as u64,
                labels[i],
                [x, y + 10.],
                24.,
                clip,
                [220, 246, 245, 245],
            );
            let t = hud.text.last_mut().unwrap();
            t.width = width;
            t.align = TextAlign::Center;
        } else {
            super::text(
                hud,
                10 + i as u64,
                labels[i],
                [x + 48., y + 12.],
                20.,
                clip,
                [145, 175, 190, 230],
            );
            super::text(
                hud,
                40 + i as u64,
                &values[i],
                [x + 180., y + 10.],
                24.,
                clip,
                [215, 237, 243, 245],
            );
            for (id, glyph, at) in [
                (60 + i as u64, "‹", x + 14.),
                (70 + i as u64, "›", x + width - 30.),
            ] {
                super::text(
                    hud,
                    id,
                    glyph,
                    [at, y + 8.],
                    26.,
                    clip,
                    [125, 233, 214, 230],
                );
            }
        }
    }
    super::text(
        hud,
        90,
        r.race.world.centreline.recipe.name,
        [x, 590.],
        18.,
        clip,
        [125, 233, 214, 235],
    );
    super::text(
        hud,
        91,
        "Up/down: select    Left/right: change    Start: race",
        [x, h - 58.],
        17.,
        clip,
        [145, 168, 183, 230],
    );
}
