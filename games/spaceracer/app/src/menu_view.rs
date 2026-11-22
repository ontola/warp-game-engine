//! Compact source-style start/pause panel with the retained SpaceRacer logo.
use render::{
    hud::{HudDrawList, HudImage, ImageFilter, TextAlign},
    picture::Picture,
};
use std::sync::{Arc, OnceLock};
/// The menu row under `at` (menu units, as `draw` lays out) and which side of
/// it: -1 the left third, +1 the right third, 0 the middle.
pub fn hit(size: [f32; 2], race_started: bool, lobby: bool, at: [f32; 2]) -> Option<(usize, i32)> {
    let x = (size[0] * 0.065).max(36.);
    let width = 480_f32.min(size[0] - x * 2.);
    order(race_started, lobby).into_iter().enumerate().find_map(|(k, i)| {
        let y = 180. + k as f32 * 66.;
        let inside = at[0] >= x && at[0] <= x + width && at[1] >= y && at[1] <= y + 54.;
        let side = if at[0] < x + width / 3. { -1 } else if at[0] > x + width * 2. / 3. { 1 } else { 0 };
        inside.then_some((i, side))
    })
}
/// Row 6, Back to lobby: under a Splitscreen Club host only.
pub const LOBBY: usize = 6;
/// The rows top to bottom (rules/game.luau `menu_order`): Resume once a race
/// runs, Start new race, Back to lobby under a host, then the settings.
pub fn order(race_started: bool, lobby: bool) -> Vec<usize> {
    let mut rows = vec![0, 1];
    if lobby { rows.push(LOBBY); }
    rows.extend(2..6);
    if !race_started { rows.remove(0); }
    rows
}
/// The loading screen while a new race builds: the logo panel and a bar.
pub fn loading(hud: &mut HudDrawList, size: [f32; 2], done: f32) {
    let [w, h] = size;
    let clip = [0., 0., w, h];
    hud.rect(clip, [0.015, 0.025, 0.045, 0.96], clip);
    let width = 480_f32.min(w * 0.8);
    let x = (w - width) * 0.5;
    let y = h * 0.5;
    super::text(hud, 95, "Loading race", [x, y - 52.], 26., clip, [220, 246, 245, 245]);
    let t = hud.text.last_mut().unwrap();
    t.width = width;
    t.align = TextAlign::Center;
    hud.rect([x, y, width, 14.], [1., 1., 1., 0.08], clip);
    hud.rect([x, y, width * done.clamp(0., 1.), 14.], [0.49, 0.91, 0.84, 0.9], clip);
    super::text(hud, 96, &format!("{}%", (done.clamp(0., 1.) * 100.).round()), [x, y + 26.], 18., clip, [125, 233, 214, 235]);
    let t = hud.text.last_mut().unwrap();
    t.width = width;
    t.align = TextAlign::Center;
}
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
        if r.race_started {
            "Resume race"
        } else {
            "Race"
        },
        "Start new race",
        "Players",
        "World",
        "Difficulty",
        "Track seed",
        "Back to lobby",
    ];
    let values = [
        String::new(),
        String::new(),
        r.players.to_string(),
        if r.random_biome {"RANDOM".into()} else {super::BIOMES[r.biome].to_uppercase()},
        if r.random_difficulty {"RANDOM".into()} else {super::DIFFICULTIES[r.difficulty].to_uppercase()},
        if r.random_seed {"RANDOM".into()} else {format!("{:05}", r.seed)},
        String::new(),
    ];
    let rows = order(r.race_started, r.lobby());
    for (k, &i) in rows.iter().enumerate() {
        let y = 180. + k as f32 * 66.;
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
        if i < 2 || i == LOBBY {
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
        if r.random_seed {"Random track type"} else {spaceracer::generation::Recipe::name_for_seed(r.seed)},
        [x, 180.+rows.len() as f32*66.+14.],
        18.,
        clip,
        [125, 233, 214, 235],
    );
}
