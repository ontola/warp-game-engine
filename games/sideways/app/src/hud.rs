//! Production race data to a screen-space HUD; no simulation or GPU state.
//! Layout follows src/hud.gd. Each viewport owns its clipping rectangle, while
//! text-cache identities follow the car when couch order changes.

use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use sideways::race::Race;
use sideways::stage::Note;

const WHITE: [u8; 4] = [248, 249, 246, 255];
const MUTED: [u8; 4] = [206, 214, 221, 230];
const AMBER: [u8; 4] = [255, 203, 114, 255];
const PLATE: [f32; 4] = [0.025, 0.032, 0.045, 0.70];

/// One compact, translucent standalone menu; leave the world and sky visible
/// during live lighting adjustments, including split-screen play.
pub fn live_pause_menu(size:[u32;2],selected:usize,time:&str,atmosphere:&str,biome:&str,status:&str)->HudDrawList {
    let mut draw=HudDrawList::new(size);let [w,h]=size.map(|v|v as f32);
    let s=(h/1080.0).clamp(0.55,2.0).min(w/700.0);let x=32.0*s;let y=(h-419.0*s)*0.5;
    let width=430.0*s;let clip=[0.0,0.0,w,h];
    draw.rect([x,y,width,419.0*s],[0.025,0.032,0.045,0.80],clip);
    let text=|draw:&mut HudDrawList,id:u64,value:String,top:f32,size:f32,color|draw.text.push(TextRun {
        id:u64::MAX-100-id,text:value,position:[x+20.0*s,y+top*s],bounds:clip,font_size:size*s,color,
        font:FontFace::Body,align:TextAlign::Left,width:390.0*s});
    text(&mut draw,0,"PAUSED".into(),18.0,32.0,WHITE);
    let rows=["Resume".to_owned(),format!("Time of day   < {time} >"),format!("Atmosphere   < {atmosphere} >"),
        format!("Next biome   < {biome} >"),"New stage".to_owned()];
    for (i,row) in rows.into_iter().enumerate() {
        let top=72.0+i as f32*49.0;
        if i==selected {draw.rect([x+12.0*s,y+(top-5.0)*s,width-24.0*s,42.0*s],[0.16,0.21,0.27,0.9],clip);}
        text(&mut draw,1+i as u64,row,top,25.0,if i==selected {AMBER}else{WHITE});
    }
    text(&mut draw,6,status.into(),329.0,19.0,MUTED);
    text(&mut draw,7,"D-pad / arrows: adjust   A / Enter: select".into(),365.0,17.0,MUTED);
    text(&mut draw,8,"Start / Esc: resume".into(),390.0,17.0,MUTED);
    draw
}

/// Global developer feedback, independent of the number of split viewports.
pub fn screenshot_toast(draw: &mut HudDrawList, message: &str) {
    let [w,h]=draw.size.map(|v|v as f32);
    let scale=(h/1080.0).clamp(0.65,2.0);
    let width=(1050.0*scale).min((w-24.0).max(1.0));
    let x=(w-width)*0.5;
    let y=(h-76.0*scale).max(0.0);
    let clip=[0.0,0.0,w,h];
    draw.rect([x,y,width,52.0*scale],[0.025,0.032,0.045,0.96],clip);
    draw.text.push(TextRun{id:u64::MAX-1,text:message.to_owned(),position:[x+12.0*scale,y+10.0*scale],bounds:clip,
        font_size:24.0*scale,color:WHITE,font:FontFace::Body,align:TextAlign::Center,width:(width-24.0*scale).max(1.0)});
}
const COLORS: [[f32; 4]; 8] = [
    [0.92, 0.27, 0.21, 1.0], [0.25, 0.56, 0.95, 1.0],
    [1.0, 0.76, 0.23, 1.0], [0.28, 0.84, 0.44, 1.0],
    [0.73, 0.46, 0.91, 1.0], [1.0, 0.53, 0.23, 1.0],
    [0.35, 0.84, 0.87, 1.0], [0.76, 0.79, 0.82, 1.0],
];

#[derive(Clone, Copy, Debug)]
struct Panel {
    car: usize,
    clip: [f32; 4],
    scale: f32,
}

fn panels(order: &[usize], rects: &[[u32; 4]], count: usize, size: [u32; 2]) -> Vec<Panel> {
    let mut out = Vec::with_capacity(rects.len().min(4));
    for (view, rect) in rects.iter().enumerate().take(4) {
        let Some(car) = (if order.is_empty() { Some(view) } else { order.get(view).copied() }) else { continue; };
        if car >= count || out.iter().any(|p: &Panel| p.car == car) { continue; }
        let x = rect[0].min(size[0]);
        let y = rect[1].min(size[1]);
        let w = rect[2].min(size[0] - x);
        let h = rect[3].min(size[1] - y);
        if w == 0 || h == 0 { continue; }
        // 4K single-player doubles the 1080p design. Four 4K quadrants use
        // the normal 1080p size; small preview windows retain readable type.
        let scale = (h as f32 / 1080.0).clamp(0.4, 2.0).min(w as f32 / 700.0);
        out.push(Panel { car, clip: [x as f32, y as f32, w as f32, h as f32], scale });
    }
    out
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    [x, y, (a[0] + a[2]).min(b[0] + b[2]).max(x) - x,
        (a[1] + a[3]).min(b[1] + b[3]).max(y) - y]
}

struct Painter<'a> {
    draw: &'a mut HudDrawList,
    panel: Panel,
    w: f32,
    h: f32,
}

impl<'a> Painter<'a> {
    fn new(draw: &'a mut HudDrawList, panel: Panel) -> Self {
        Self { w: panel.clip[2] / panel.scale, h: panel.clip[3] / panel.scale, draw, panel }
    }
    fn point(&self, p: [f32; 2]) -> [f32; 2] {
        [self.panel.clip[0] + p[0] * self.panel.scale, self.panel.clip[1] + p[1] * self.panel.scale]
    }
    fn rect(&mut self, r: [f32; 4], color: [f32; 4]) {
        let at = self.point([r[0], r[1]]);
        self.draw.rect([at[0], at[1], r[2] * self.panel.scale, r[3] * self.panel.scale], color, self.panel.clip);
    }
    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, color: [f32; 4]) {
        let (a, b) = (self.point(a), self.point(b));
        self.draw.line(a, b, width * self.panel.scale, color, self.panel.clip);
    }
    fn circle(&mut self, center: [f32; 2], radius: f32, color: [f32; 4]) {
        let center = self.point(center);
        self.draw.circle(center, radius * self.panel.scale, color, self.panel.clip);
    }
    fn triangle(&mut self, points: [[f32; 2]; 3], color: [f32; 4]) {
        let points = points.map(|p| self.point(p));
        self.draw.triangle(points, color, self.panel.clip);
    }
    fn text(&mut self, slot: u64, text: impl Into<String>, r: [f32; 4], font_size: f32, color: [u8; 4], font: FontFace, align: TextAlign) {
        let position = self.point([r[0], r[1]]);
        let bounds = intersect([position[0], position[1], r[2] * self.panel.scale, r[3] * self.panel.scale], self.panel.clip);
        if bounds[2] <= 0.0 || bounds[3] <= 0.0 { return; }
        self.draw.text.push(TextRun { id: self.panel.car as u64 * 100 + slot, text: text.into(), position, bounds,
            font_size: font_size * self.panel.scale, color, font, align, width: r[2] * self.panel.scale });
    }
}

/// Names are UTF-8 throughout. Bound the number of glyphs and remove line/control
/// characters; the renderer additionally clips wide glyphs to the name box.
fn short_name(name: &str) -> String {
    const MAX: usize = 28;
    let mut chars: Vec<_> = name.chars().filter_map(|c| {
        if c.is_whitespace() { Some(' ') } else if c.is_control() { None } else { Some(c) }
    }).take(MAX + 1).collect();
    if chars.len() > MAX { chars.truncate(MAX - 1); chars.push('…'); }
    chars.into_iter().collect::<String>().trim().to_owned()
}

fn format_time(seconds: f64) -> String {
    if seconds == f64::INFINITY { return "DNF".into(); }
    if !seconds.is_finite() { return "--:--.--".into(); }
    if seconds > 359_999.99 { return ">99h".into(); }
    // Round the complete duration before splitting fields. 59.999 seconds
    // becomes 01:00.00, never an impossible 00:60.00.
    let centiseconds = (seconds.max(0.0) * 100.0).round() as u64;
    format!("{:02}:{:02}.{:02}", centiseconds / 6000, (centiseconds / 100) % 60, centiseconds % 100)
}

fn finite(value: f64, fallback: f64) -> f64 { if value.is_finite() { value } else { fallback } }
fn color(index: usize) -> [f32; 4] { COLORS[index % COLORS.len()] }
fn text_color(index: usize) -> [u8; 4] { color(index).map(|v| (v * 255.0).round() as u8) }

fn standings(race: &Race) -> Vec<usize> {
    let mut order: Vec<_> = (0..race.players.len()).collect();
    order.sort_by(|&a, &b| {
        let pa = &race.players[a]; let pb = &race.players[b];
        pb.finished.cmp(&pa.finished).then_with(|| {
            if pa.finished { finite(pa.time, f64::INFINITY).total_cmp(&finite(pb.time, f64::INFINITY)) }
            else { finite(pb.along, 0.0).total_cmp(&finite(pa.along, 0.0)) }
        }).then(a.cmp(&b))
    });
    order
}

fn name_for(race: &Race, names: &[String], car: usize) -> String {
    let name = short_name(names.get(car).map(String::as_str).unwrap_or(&race.players[car].name));
    if name.is_empty() { format!("Player {}", car + 1) } else { name }
}

fn pace_note(p: &mut Painter<'_>, note: &Note, slot: usize, origin: [f32; 2], along: f64) {
    let [x, y] = origin;
    let alpha = if slot == 0 { 1.0 } else { 0.66 };
    p.rect([x, y, 132.0, 144.0], [PLATE[0], PLATE[1], PLATE[2], 0.68 * alpha]);
    let ink = [0.98, 0.98, 0.96, alpha];
    let c = [x + 66.0, y + 49.0];
    let label = if note.kind == "crest" {
        let mut previous = [c[0] - 46.0, c[1] + 18.0];
        for i in 1..=12 {
            let u = i as f32 / 6.0 - 1.0;
            let next = [c[0] + u * 46.0, c[1] + 18.0 - (1.0 - u * u) * 34.0];
            p.line(previous, next, 7.0, ink); previous = next;
        }
        "CREST".to_owned()
    } else {
        let grade = note.grade.clamp(0, 6);
        let bend = if grade == 0 { 180.0_f32 } else { 170.0 - 150.0 * grade as f32 / 6.0 }.to_radians();
        let direction = if note.kind == "left" { -1.0 } else { 1.0 };
        let mut at = [c[0] - direction * 18.0, c[1] + 28.0];
        let start = [at[0], at[1] - 28.0]; p.line(at, start, 7.0, ink); at = start;
        let mut heading = -std::f32::consts::FRAC_PI_2;
        let radius = if grade == 0 { 17.0 } else { 34.0 };
        for _ in 0..8 {
            heading += direction * bend / 8.0;
            let next = [at[0] + heading.cos() * radius * bend / 8.0, at[1] + heading.sin() * radius * bend / 8.0];
            p.line(at, next, 7.0, ink); at = next;
        }
        let back = [heading.cos(), heading.sin()]; let side = [-back[1], back[0]];
        p.triangle([[at[0] + back[0] * 14.0, at[1] + back[1] * 14.0],
            [at[0] - back[0] * 4.0 + side[0] * 13.0, at[1] - back[1] * 4.0 + side[1] * 13.0],
            [at[0] - back[0] * 4.0 - side[0] * 13.0, at[1] - back[1] * 4.0 - side[1] * 13.0]], ink);
        if grade == 0 { "H".into() } else { grade.to_string() }
    };
    let text_ink = [248, 249, 246, (255.0 * alpha) as u8];
    p.text(20 + slot as u64 * 2, label, [x + 6.0, y + 88.0, 120.0, 40.0], 32.0, text_ink, FontFace::Body, TextAlign::Center);
    let metres = finite(note.at - along, 0.0).max(0.0).round() as u32;
    p.text(21 + slot as u64 * 2, format!("{metres} m"), [x + 6.0, y + 122.0, 120.0, 22.0], 18.0, text_ink, FontFace::Body, TextAlign::Center);
}

/// `view_order` and `names` use canonical Race.players indices, not seat tokens.
/// Rectangles and explicit output size are physical pixels. No glyph shaping or
/// texture allocation happens here; the renderer caches stable TextRun IDs.
pub fn build(race: &Race, view_order: &[usize], rects: &[[u32; 4]], names: &[String], paused: bool, size: [u32; 2]) -> HudDrawList {
    let mut draw = HudDrawList::new(size);
    let order = standings(race);
    for panel in panels(view_order, rects, race.players.len(), size) {
        let index = panel.car;
        let player = &race.players[index]; let car = &player.car;
        let name = name_for(race, names, index);
        let rank = order.iter().position(|&i| i == index).unwrap_or(index) + 1;
        let mut p = Painter::new(&mut draw, panel);
        let (w, h) = (p.w, p.h);
        if paused {
            p.rect([0.0, 0.0, w, h], [0.02, 0.025, 0.035, 0.48]);
            let x = (w - 400.0) * 0.5; let y = (h - 150.0) * 0.5;
            p.rect([x, y, 400.0, 150.0], PLATE);
            p.text(90, "PAUSED", [x + 20.0, y + 20.0, 360.0, 72.0], 54.0, WHITE, FontFace::Title, TextAlign::Center);
            p.text(91, name, [x + 20.0, y + 100.0, 360.0, 36.0], 28.0, text_color(index), FontFace::Body, TextAlign::Center);
            continue;
        }
        if race.phase == "results" {
            let rows = order.len().min(8); let height = 154.0 + rows as f32 * 36.0;
            let x = (w - 560.0) * 0.5; let y = (h - height) * 0.5;
            p.rect([x, y, 560.0, height], [0.025, 0.032, 0.045, 0.85]);
            p.text(40, "STAGE COMPLETE", [x + 20.0, y + 16.0, 520.0, 52.0], 40.0, WHITE, FontFace::Title, TextAlign::Center);
            p.text(41, format!("{name}  ·  P{rank}"), [x + 20.0, y + 65.0, 520.0, 34.0], 26.0, text_color(index), FontFace::Body, TextAlign::Center);
            for (row, &driver) in order.iter().take(rows).enumerate() {
                let top = y + 104.0 + row as f32 * 36.0;
                if driver == index { p.rect([x + 14.0, top, 532.0, 34.0], [0.15, 0.19, 0.24, 0.8]); }
                let slot = 50 + row as u64 * 3;
                p.text(slot, format!("{}", row + 1), [x + 24.0, top + 2.0, 36.0, 32.0], 26.0, text_color(driver), FontFace::Body, TextAlign::Left);
                p.text(slot + 1, name_for(race, names, driver), [x + 70.0, top + 2.0, 298.0, 32.0], 26.0, WHITE, FontFace::Body, TextAlign::Left);
                let result = if race.players[driver].finished { format_time(race.players[driver].time) } else { "DNF".into() };
                p.text(slot + 2, result, [x + 378.0, top + 2.0, 158.0, 32.0], 26.0, WHITE, FontFace::Body, TextAlign::Right);
            }
            let remaining = finite(race.results, 0.0).max(0.0).ceil() as u32;
            p.text(42, format!("Next stage in {remaining}s"), [x + 20.0, y + height - 36.0, 520.0, 30.0], 23.0, MUTED, FontFace::Body, TextAlign::Center);
            continue;
        }

        let pad = 28.0;
        let time = if player.finished { player.time } else { finite(race.clock, 0.0).max(0.0) + finite(player.penalty, 0.0).max(0.0) };
        p.rect([pad, pad, 232.0, 60.0], PLATE);
        p.text(1, format_time(time), [pad + 14.0, pad + 4.0, 204.0, 54.0], 42.0, WHITE, FontFace::Body, TextAlign::Left);
        p.rect([pad, pad + 64.0, 232.0, 36.0], PLATE);
        p.rect([pad, pad + 64.0, 3.0, 36.0], color(index));
        p.text(0, name, [pad + 14.0, pad + 67.0, 204.0, 31.0], 25.0, WHITE, FontFace::Body, TextAlign::Left);
        if player.penalty > 0.0 {
            p.text(2, format!("+{:.0} s penalty", player.penalty), [pad + 14.0, pad + 105.0, 218.0, 32.0], 24.0, AMBER, FontFace::Body, TextAlign::Left);
        }
        let gx = w - pad - 250.0; let gy = h - pad - 128.0;
        p.rect([gx, gy, 250.0, 128.0], PLATE);
        let gear = if car.shift_timer > 0.0 { car.shift_target } else { car.gear };
        let gear = if gear < 0 { "R".into() } else if gear == 0 { "N".into() } else { gear.to_string() };
        p.text(3, gear, [gx + 16.0, gy + 2.0, 76.0, 111.0], 94.0, WHITE, FontFace::Title, TextAlign::Center);
        let speed = if !car.speed.is_finite() { "--".into() } else {
            let kmh = (car.speed.abs() * 3.6).round(); if kmh > 999.0 { "999+".into() } else { format!("{kmh:.0}") }
        };
        p.text(4, speed, [gx + 96.0, gy + 17.0, 136.0, 76.0], 64.0, WHITE, FontFace::Body, TextAlign::Right);
        p.text(5, "km/h", [gx + 96.0, gy + 91.0, 136.0, 27.0], 21.0, MUTED, FontFace::Body, TextAlign::Right);
        if car.manual { p.text(6, "M", [gx + 16.0, gy + 106.0, 76.0, 22.0], 18.0, MUTED, FontFace::Body, TextAlign::Center); }
        let rev = if car.spec.limit_rpm > 0.0 { finite(car.rpm() / car.spec.limit_rpm, 0.0).clamp(0.0, 1.0) as f32 } else { 0.0 };
        p.rect([gx, gy - 12.0, 250.0, 8.0], [0.0, 0.0, 0.0, 0.65]);
        p.rect([gx, gy - 12.0, 250.0 * rev, 8.0], if rev > 0.92 { [1.0, 0.22, 0.12, 1.0] } else { [0.95, 0.95, 0.89, 1.0] });
        if car.handbrake > 0.1 {
            p.rect([gx - 78.0, gy + 72.0, 66.0, 56.0], [0.85, 0.29, 0.055, 0.94]);
            p.text(7, "HB", [gx - 78.0, gy + 80.0, 66.0, 44.0], 34.0, WHITE, FontFace::Body, TextAlign::Center);
        }
        if player.off_track > 1.2 && !player.finished && !player.rival {
            let x = (w - 360.0) * 0.5; let y = h - pad - 68.0;
            p.rect([x, y, 360.0, 56.0], PLATE);
            p.text(9, "X", [x + 18.0, y + 7.0, 36.0, 44.0], 34.0, AMBER, FontFace::Body, TextAlign::Center);
            p.text(8, "Back on the road", [x + 67.0, y + 10.0, 280.0, 40.0], 30.0, WHITE, FontFace::Body, TextAlign::Left);
        }

        let along = finite(player.along, 0.0);
        let reach = 60.0 + finite(car.speed, 0.0).abs() * 3.2;
        let note_x = (w - 276.0) * 0.5;
        let note_y = if note_x < pad + 248.0 { pad + 146.0 } else { pad };
        for (slot, note) in race.stage.notes.iter().filter(|n| n.end >= along - 2.0 && n.at <= along + reach).take(2).enumerate() {
            pace_note(&mut p, note, slot, [note_x + slot as f32 * 144.0, note_y], along);
        }
        let lx = pad + 6.0; let top = pad + 158.0; let bottom = h - pad - 20.0;
        if bottom > top {
            p.line([lx, top], [lx, bottom], 7.0, [0.0, 0.0, 0.0, 0.4]);
            p.line([lx, top], [lx, bottom], 2.5, [1.0, 1.0, 1.0, 0.40]);
            p.line([lx - 9.0, top], [lx + 9.0, top], 3.0, [1.0, 1.0, 1.0, 0.8]);
            for i in (0..race.players.len()).filter(|&i| i != index).chain(std::iter::once(index)) {
                let progress = if race.stage.finish > 0.0 { finite(race.players[i].along / race.stage.finish, 0.0).clamp(0.0, 1.0) as f32 } else { 0.0 };
                let at = [lx, bottom + (top - bottom) * progress];
                if i == index { p.circle(at, 11.0, [1.0, 1.0, 1.0, 1.0]); }
                p.circle(at, if i == index { 8.0 } else { 5.5 }, color(i));
            }
        }
        if race.phase == "countdown" || (race.phase == "racing" && race.clock < 1.0) {
            let go = race.phase == "racing";
            let lit = finite(-race.clock, 3.0).ceil().clamp(1.0, 3.0) as usize;
            let cy = h * 0.30;
            for light in 0..3 {
                p.circle([w * 0.5 + (light as f32 - 1.0) * 68.0, cy], 26.0,
                    if go { [0.15, 0.85, 0.27, 1.0] } else if light < lit { [0.94, 0.12, 0.08, 1.0] } else { [0.10, 0.12, 0.14, 0.9] });
            }
            p.text(30, if go { "GO".into() } else { lit.to_string() }, [w * 0.5 - 100.0, cy + 38.0, 200.0, 80.0], 66.0, WHITE, FontFace::Title, TextAlign::Center);
        }
        if player.finished {
            let x = (w - 400.0) * 0.5; let y = h * 0.28;
            p.rect([x, y, 400.0, 130.0], PLATE);
            p.text(31, format!("FINISHED  ·  P{rank}"), [x + 18.0, y + 10.0, 364.0, 43.0], 32.0, text_color(index), FontFace::Title, TextAlign::Center);
            p.text(32, format_time(player.time), [x + 18.0, y + 54.0, 364.0, 70.0], 56.0, WHITE, FontFace::Body, TextAlign::Center);
        }
    }
    draw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_pause_controls_are_one_compact_panel_with_visible_world_behind() {
        for size in [[1280,720],[3840,2160],[768,432]] {
            let draw=live_pause_menu(size,3,"Sunset","Mist","rainforest","Updating sky...");
            assert_eq!(draw.text.iter().filter(|t|t.text=="PAUSED").count(),1);
            assert!(draw.text.iter().any(|t|t.text.contains("Time of day")));
            assert!(draw.text.iter().any(|t|t.text=="Next biome   < rainforest >" && t.color==AMBER));
            assert!(draw.text.iter().any(|t|t.text=="Updating sky..."));
            assert!(draw.vertices.iter().all(|v|v.position[0]<size[0] as f32*0.7&&v.color[3]<1.0),"menu must leave world visible");
            assert!(draw.text.iter().all(|t|t.position[0]>=0.0&&t.position[1]>=0.0&&t.position[1]+t.font_size<size[1] as f32));
        }
    }

    #[test]
    fn timer_rolls_minutes_and_handles_invalid_results() {
        assert_eq!(format_time(59.994), "00:59.99");
        assert_eq!(format_time(59.999), "01:00.00");
        assert_eq!(format_time(3600.0), "60:00.00");
        assert_eq!(format_time(-3.0), "00:00.00");
        assert_eq!(format_time(f64::INFINITY), "DNF");
        assert_eq!(format_time(f64::NAN), "--:--.--");
        assert_eq!(format_time(f64::MAX), ">99h");
    }

    #[test]
    fn reordered_views_keep_canonical_cache_ids_and_clip_to_output() {
        let mapped = panels(&[2, 0, 2, 99], &[[1920, 0, 1920, 1080], [0, 0, 1920, 1080], [0, 1080, 1920, 1080], [1920, 1080, 5000, 5000]], 3, [3840, 2160]);
        assert_eq!(mapped.iter().map(|p| p.car).collect::<Vec<_>>(), [2, 0]);
        assert_eq!(mapped[0].clip, [1920.0, 0.0, 1920.0, 1080.0]);
        assert_eq!(mapped[0].scale, 1.0);
        let mut draw = HudDrawList::new([3840, 2160]);
        for panel in mapped {
            let mut p = Painter::new(&mut draw, panel);
            p.text(0, "Zoë 日本語 🚗", [-20.0, -10.0, 300.0, 60.0], 25.0, WHITE, FontFace::Body, TextAlign::Left);
        }
        assert_eq!(draw.text.iter().map(|t| t.id).collect::<Vec<_>>(), [200, 0]);
        assert_eq!(draw.text[0].bounds, [1920.0, 0.0, 280.0, 50.0]);
        let edge = panels(&[0], &[[3800, 2100, u32::MAX, u32::MAX]], 1, [3840, 2160]);
        assert_eq!(edge[0].clip, [3800.0, 2100.0, 40.0, 60.0]);
        assert_eq!(panels(&[0], &[[0, 0, 3840, 2160]], 1, [3840, 2160])[0].scale, 2.0);
    }

    #[test]
    fn unicode_names_are_bounded_without_broken_encoding_or_newlines() {
        assert_eq!(short_name("Zoë 日本語 🚗"), "Zoë 日本語 🚗");
        assert_eq!(short_name("Ada\nBo\0"), "Ada Bo");
        let name = short_name(&"界🚗".repeat(80));
        assert_eq!(name.chars().count(), 28);
        assert!(name.ends_with('…'));
    }

    #[test]
    fn production_race_countdown_recovery_finish_and_pause_have_distinct_hud_states() {
        let settings = sideways::race::Settings { stage_km: 0.2, rivals: 0, bots: 2, ..Default::default() };
        let mut race = Race::new(1, settings, sideways::spec::Cars::load(None).unwrap());
        let names = vec!["Zoë".into(), "Αλέξης Борис".into()];
        let rects = [[0, 0, 1920, 1080], [1920, 0, 1920, 1080]];
        let hud = |race: &Race, paused| build(race, &[1, 0], &rects, &names, paused, [3840, 1080]);
        race.phase = "countdown"; race.clock = -2.2;
        let countdown = hud(&race, false);
        assert!(countdown.text.iter().any(|t| t.id == 130 && t.text == "3"));
        assert!(countdown.text.iter().any(|t| t.id == 100 && t.text == names[1]));
        race.phase = "racing"; race.clock = 0.5;
        assert!(hud(&race, false).text.iter().any(|t| t.id == 130 && t.text == "GO"));
        race.clock = 60.0; race.players[1].off_track = 2.0; race.players[1].car.handbrake = 1.0;
        let recovery = hud(&race, false);
        assert!(recovery.text.iter().any(|t| t.id == 108 && t.text == "Back on the road"));
        assert!(recovery.text.iter().any(|t| t.id == 107 && t.text == "HB"));
        race.players[1].finished = true; race.players[1].time = 59.999;
        let finished = hud(&race, false);
        assert!(finished.text.iter().any(|t| t.id == 131 && t.text.contains("P1")));
        assert!(finished.text.iter().any(|t| t.id == 132 && t.text == "01:00.00"));
        assert!(!finished.text.iter().any(|t| t.id == 108));
        let paused = hud(&race, true);
        assert_eq!(paused.text.len(), 4, "pause draws only its own two labels per view");
        assert!(paused.text.iter().all(|t| t.id % 100 >= 90));
        race.phase = "results";
        assert!(hud(&race, false).text.iter().any(|t| t.text == "DNF"));
    }
}
