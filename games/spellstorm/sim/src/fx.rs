//! What the sim shows: one-shot visual events for the app (bursts, rings,
//! light pops, floating numbers, banners). The sim never draws; it says what
//! happened where, and the app makes it look like the Godot game.

use crate::El;
use glam::Vec3;

pub type C = [f32; 4];

pub const fn rgb(r: f32, g: f32, b: f32) -> C {
    [r, g, b, 1.0]
}

pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> C {
    [r, g, b, a]
}

pub fn el_c(e: El) -> C {
    let c = e.color();
    [c[0], c[1], c[2], 1.0]
}

pub const WHITE: C = rgb(1.0, 1.0, 1.0);

/// Godot's `Color.lerp`.
pub fn mix(a: C, b: C, t: f32) -> C {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Godot's `Color.lightened`.
pub fn lighten(c: C, t: f32) -> C {
    [
        c[0] + (1.0 - c[0]) * t,
        c[1] + (1.0 - c[1]) * t,
        c[2] + (1.0 - c[2]) * t,
        c[3],
    ]
}

/// Godot's `Color.darkened`.
pub fn darken(c: C, t: f32) -> C {
    [c[0] * (1.0 - t), c[1] * (1.0 - t), c[2] * (1.0 - t), c[3]]
}

/// How a monster or a wizard leaves the world (Remains.style_for and the
/// per-kind deaths in Enemy._death).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Death {
    Freeze,
    Gore,
    Burn,
    Shock,
    Void,
    Blown,
    Fall,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    /// Fx.burst: `n` motes flung out at `speed`, `size` big, styled by `el`.
    Burst {
        at: Vec3,
        c: C,
        n: u32,
        speed: f32,
        size: f32,
        el: Option<El>,
    },
    /// Fx.puff: a soft cloud.
    Puff {
        at: Vec3,
        c: C,
        size: f32,
    },
    /// Fx.ring: a ring running out over the floor.
    Ring {
        at: Vec3,
        c: C,
        r: f32,
        time: f32,
    },
    /// Fx.ripple: a ring on water.
    Ripple {
        at: Vec3,
        r: f32,
        time: f32,
    },
    /// Fx.light_pop: a light that flares and fades.
    LightPop {
        at: Vec3,
        c: C,
        energy: f32,
        range: f32,
        time: f32,
    },
    /// Fx.flash: a flat bright flash.
    Flash {
        at: Vec3,
        c: C,
        size: f32,
        time: f32,
    },
    /// Fx.text: a floating number.
    Text {
        at: Vec3,
        text: String,
        c: C,
        size: f32,
        time: f32,
    },
    /// A one-shot fountain of an element's particles (BoltFx._spray).
    Fountain {
        at: Vec3,
        el: El,
        n: u32,
        speed: f32,
        spread: f32,
        size: f32,
    },
    /// A jagged lightning line from `a` to `b` (BoltFx._zap, _spark_line).
    Zap {
        a: Vec3,
        b: Vec3,
        c: C,
        width: f32,
        time: f32,
    },
    /// A chain of lightning through `pts` (ArcFx._draw).
    Chain {
        pts: Vec<Vec3>,
        c: C,
    },
    /// A thunderbolt out of the sky (StrikeFx).
    Thunder {
        at: Vec3,
        top: Vec3,
        c: C,
        width: f32,
        radius: f32,
    },
    /// A nova's shockwave and its rings (NovaFx._ready).
    Nova {
        at: Vec3,
        r: f32,
        cs: Vec<C>,
        el: Option<El>,
        /// Its other elements, most first: each shows its own way too.
        payload: Vec<El>,
    },
    /// A thrown spell (a boulder, a lava bomb, a mud ball) landing, its
    /// splash `r` across: the view's crash of its elements.
    Land {
        at: Vec3,
        r: f32,
        el: Option<El>,
        payload: Vec<El>,
    },
    /// BlastFx.boom: fire, smoke and a scorch mark.
    Boom {
        at: Vec3,
        r: f32,
        mark: bool,
    },
    /// Camera shake.
    Shake(f32),
    /// A summoning circle where a monster is about to stand.
    Summon {
        at: Vec3,
    },
    /// A prop shattered; its chunks fly from `from` with `power`.
    PropBroke {
        prop: usize,
        from: Vec3,
        power: f32,
    },
    /// A prop took a hit but stands.
    PropChip {
        prop: usize,
    },
    /// A wizard went down, this way.
    WizardDown {
        seat: usize,
        how: Death,
        dir: Vec3,
    },
    /// A wizard stood up again.
    WizardUp {
        seat: usize,
    },
    /// A monster died (the app plays its kind's death).
    MonsterDied {
        id: u64,
        kind: &'static str,
        how: Death,
    },
    /// A monster's lob, arcing from `from` to `to` over `time`.
    Lob {
        from: Vec3,
        to: Vec3,
        el: El,
        time: f32,
    },
    /// Stone or ice spikes out of a trap.
    Spikes {
        at: Vec3,
        el: El,
        r: f32,
        time: f32,
    },
    /// Big text over the screen (Run.banner).
    Banner {
        title: String,
        sub: String,
        c: C,
    },
    /// A line at the bottom (Game.toast).
    Toast {
        text: String,
        c: C,
    },
    /// The screen fades to black (1) or back (0).
    Fade {
        to: f32,
        time: f32,
    },
    YouDied,
    YouDiedEnd,
    /// A new level was entered: the app rebuilds the world.
    Level {
        biome: usize,
        depth: usize,
    },
    /// The run ended, won or lost (GameNight notify_finished).
    RunOver {
        won: bool,
    },
    /// A spell form was cast for the first time this session.
    Grimoire {
        key: String,
        name: String,
    },
}
