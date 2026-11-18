//! A person's hair, grown from their seed out of their own scalp: where the
//! hairline runs, how long the hair is, where it parts, how it is combed and
//! how it falls. No groom mesh: a strand starts on the head's skin, leaves
//! it at an angle, bends under its weight and rests on the skull. The
//! renderer draws the strands as they are (render::humans::PlayerStrands).
use super::warp_skin::{hash, noise, smooth, unit, FACE, METRES};
use glam::Vec3;
use render::humans::BodyVertex;

/// Points along one strand, root to tip (the strand renderer's nine
/// segments).
pub(super) const POINTS: usize = 10;
/// Strands on a full head of hair (the strand renderer's budget per head).
const STRANDS: usize = 16_384;

/// The height of the hairline at a point of the head, metres above the
/// eyes: across the forehead, down at the temples to a sideburn before the
/// ear, close over the ear, and down behind it to the nape.
fn hairline(p: Vec3, recede: f32) -> f32 {
    // A hairline that recedes goes back furthest above the temples.
    let corners = smooth(0.015, 0.040, p.x.abs()) * smooth(0.075, 0.050, p.x.abs());
    let front = 0.062 + recede * (0.6 + 0.9 * corners) - 0.034 * smooth(0.030, 0.072, p.x.abs());
    let side = 0.012;
    let nape = -0.072;
    let line = front + (side - front) * smooth(-0.020, -0.050, p.z);
    let sideburn = 0.030 * smooth(-0.036, -0.046, p.z) * smooth(-0.068, -0.060, p.z);
    line - sideburn + (nape - side) * smooth(-0.100, -0.120, p.z)
}

/// How thickly hair grows at a point of the head, 0 to 1: `p` in metres
/// from the point between the eyes (x to the person's left, y up, z out of
/// the face).
pub(super) fn scalp(p: Vec3, recede: f32) -> f32 {
    let line = hairline(p, recede);
    smooth(line, line + 0.007, p.y)
}

/// What the seed decides about a head of hair: a cut as a hairdresser
/// makes it. Short cuts are tapered (short at the ears and the nape,
/// growing to the length on top); long cuts are cut to a hem, the height
/// at which all the hair ends.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cut {
    pub seed: u32,
    /// Metres on top of the head.
    pub top: f32,
    /// Metres at the hairline over the ears and at the nape; `band` is how
    /// far up from that hairline to the crown the hair is as long as on
    /// top (1 at the crown), and `fade` how late it grows on the way up
    /// (1 evenly, more for a fade).
    pub taper: f32,
    pub band: f32,
    pub fade: f32,
    /// Height in metres from the eyes below which hair is cut off.
    pub hem: f32,
    /// Where the hair parts across the head, metres from the middle; hair
    /// lies away from it on both sides.
    pub part: f32,
    /// How far the front is swept back (1) or worn forward (-1).
    pub sweep: f32,
    /// Whether the front hair comes down over the forehead while the rest
    /// follows the parting (1), or not (0).
    pub bangs: f32,
    /// Height above the eyes at which hair over the face is cut, metres.
    pub fringe: f32,
    /// How steeply hair leaves the scalp, 0 flat to 1 upright.
    pub lift: f32,
    /// How far locks lie out of the comb's line, 0 combed to 1 tousled.
    pub mess: f32,
    /// Waves: sideways swing in metres, and waves per metre of hair.
    pub wave: f32,
    pub pitch: f32,
    /// Tightly coiled hair (1) or not (0): it stands off the head in
    /// small spirals and does not hang.
    pub coil: f32,
    /// Metres the hairline has gone back at the forehead, and how bare the
    /// crown is (0 full to 1 bald).
    pub recede: f32,
    pub thin: f32,
    /// The share of hairs that are grey.
    pub grey: f32,
}

/// The cuts a seed chooses from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Style {
    Buzz,
    Crop,
    SidePart,
    SweptBack,
    Fringe,
    Afro,
    Curtains,
    Bob,
    Shoulder,
    Long,
}

/// No hem: the hair ends where its length does.
const UNCUT: f32 = -10.;

impl Cut {
    /// `skin`: how light the person's skin is, 0 to 1. `years`: their age.
    pub fn new(seed: u32, skin: f32, years: f32) -> Self {
        let roll = |k: u32| unit(hash(seed ^ k.wrapping_mul(0x9e37_79b9)));
        // Most people with dark skin have tightly coiled hair.
        let coily = roll(20) < 0.92 * smooth(0.42, 0.26, skin);
        // With the years the hairline goes back in some and the crown
        // thins in fewer; almost everyone greys.
        let older = smooth(28., 70., years);
        let recede = if roll(21) < 0.45 { 0.045 * older * (0.4 + 0.6 * roll(22)) } else { 0.008 * older };
        let thin = if roll(23) < 0.3 { smooth(40., 75., years) * (0.4 + 0.6 * roll(24)) } else { 0. };
        let grey = (smooth(35., 80., years).powf(1.5) * (0.3 + 1.2 * roll(25))).min(1.);
        let r = roll(1);
        let pick = |odds: &[(Style, f32)]| {
            let mut left = r * odds.iter().map(|o| o.1).sum::<f32>();
            for &(style, share) in odds {
                if left < share {
                    return style;
                }
                left -= share;
            }
            odds[0].0
        };
        let mut style = if coily {
            pick(&[(Style::Buzz, 0.28), (Style::Crop, 0.40), (Style::Afro, 0.32)])
        } else {
            // Short cuts by far the most; long hair is rare.
            pick(&[
                (Style::Buzz, 0.08),
                (Style::Crop, 0.27),
                (Style::SidePart, 0.22),
                (Style::SweptBack, 0.15),
                (Style::Fringe, 0.12),
                (Style::Curtains, 0.05),
                (Style::Bob, 0.05),
                (Style::Shoulder, 0.04),
                (Style::Long, 0.02),
            ])
        };
        // Thin hair is mostly worn short, and the young cuts are for the
        // young.
        if (recede > 0.02 || thin > 0.4) && roll(26) < 0.5 {
            style = Style::Buzz;
        } else if years > 50. && !coily && matches!(style, Style::Crop | Style::Fringe) && roll(27) < 0.6 {
            style = if roll(28) < 0.6 { Style::SidePart } else { Style::SweptBack };
        }
        Self { coil: f32::from(coily), recede, thin, grey, ..Self::styled(seed, style) }
    }

    pub fn styled(seed: u32, style: Style) -> Self {
        let roll = |k: u32| unit(hash(seed ^ k.wrapping_mul(0x9e37_79b9)));
        let side = if roll(7) < 0.5 { -1. } else { 1. };
        let wavy = roll(6).powi(2);
        let short = Self {
            seed,
            top: 0.05,
            taper: 0.005 + 0.009 * roll(3),
            band: 0.75 + 0.25 * roll(12),
            fade: 1.3 + 1.7 * roll(13),
            hem: UNCUT,
            part: 0.,
            sweep: 0.,
            bangs: 0.,
            fringe: 0.022 + 0.022 * roll(11),
            lift: 0.2,
            mess: 0.2,
            wave: 0.006 * wavy,
            pitch: 12. + 25. * roll(10),
            coil: 0.,
            recede: 0.,
            thin: 0.,
            grey: 0.,
        };
        // Cut to a hem: no taper, and long enough to reach it from the top.
        // The front is drawn back a little, so it falls beside the face and
        // not before it.
        let long = Self { top: 0.6, taper: 0.6, sweep: 0.35 + 0.25 * roll(8), wave: 0.014 * wavy, lift: 0.08 + 0.10 * roll(9), mess: 0.15, ..short };
        let parted = if roll(4) < 0.45 { 0. } else { (0.028 + 0.015 * roll(5)) * side };
        match style {
            Style::Buzz => Self { top: 0.004 + 0.007 * roll(2), taper: 0.002 + 0.002 * roll(3), lift: 0.8, mess: 0.3, wave: 0., ..short },
            // Short and tousled, pushed up or forward.
            Style::Crop => Self {
                top: 0.028 + 0.027 * roll(2),
                part: 0.02 * side * roll(5),
                sweep: if roll(8) < 0.5 { -0.5 - 0.3 * roll(14) } else { 0.2 + 0.5 * roll(14) },
                lift: 0.40 + 0.30 * roll(9),
                mess: 0.6 + 0.4 * roll(15),
                ..short
            },
            Style::SidePart => Self {
                top: 0.05 + 0.035 * roll(2),
                part: (0.030 + 0.014 * roll(5)) * side,
                sweep: 0.25 + 0.35 * roll(8),
                lift: 0.14 + 0.16 * roll(9),
                ..short
            },
            Style::SweptBack => Self {
                top: 0.06 + 0.035 * roll(2),
                part: if roll(4) < 0.5 { 0. } else { 0.035 * side },
                sweep: 0.8 + 0.2 * roll(8),
                lift: 0.25 + 0.25 * roll(9),
                mess: 0.3,
                ..short
            },
            // Worn forward from the crown and cut across the forehead.
            Style::Fringe => Self {
                top: 0.05 + 0.03 * roll(2),
                part: 0.02 * side * roll(5),
                sweep: -1.,
                lift: 0.10 + 0.12 * roll(9),
                mess: 0.35 + 0.3 * roll(15),
                ..short
            },
            // Grown out evenly all round, a little shorter low down.
            Style::Afro => {
                let top = 0.045 + 0.06 * roll(2).powi(2);
                Self { top, taper: 0.6 * top, band: 0.5, fade: 1., lift: 0.9, mess: 1., wave: 0., ..short }
            }
            // Parted in the middle and cut round at the ears.
            Style::Curtains => Self { hem: -0.025 - 0.035 * roll(2), part: 0.012 * side * roll(5), ..long },
            Style::Bob => Self { hem: -0.075 - 0.03 * roll(2), part: parted, bangs: f32::from(roll(16) < 0.25), fringe: 0.010 + 0.012 * roll(11), ..long },
            Style::Shoulder => Self { hem: -0.17 - 0.06 * roll(2), part: parted, bangs: f32::from(roll(16) < 0.25), fringe: 0.010 + 0.012 * roll(11), ..long },
            Style::Long => Self { hem: -0.26 - 0.04 * roll(2), part: parted, bangs: f32::from(roll(16) < 0.2), fringe: 0.010 + 0.012 * roll(11), ..long },
        }
    }

    /// How thickly this person's hair grows at `p`, 0 to 1.
    pub fn grows(&self, p: Vec3) -> f32 {
        let crown = Vec3::new(0., 0.118, -0.125);
        scalp(p, self.recede) * (1. - self.thin * smooth(0.075, 0.025, p.distance(crown)))
    }

    /// The length in metres to which hair is cut where it grows at, or lies
    /// over, `p`.
    fn length(&self, p: Vec3, crown: f32) -> f32 {
        // The front hairline is not tapered: hair there is as on top.
        let tapered = smooth(-0.015, -0.050, p.z);
        let line = hairline(p, 0.);
        let up = ((p.y - line) / (crown - line).max(0.01) / self.band).clamp(0., 1.).powf(self.fade);
        // Under the hairline nothing is left long: the neck and the ears
        // are cut clean.
        let grown = (self.taper + (self.top - self.taper) * up) * smooth(-0.012, 0., p.y - line);
        if self.taper >= self.top { self.top } else { self.top + (grown - self.top) * tapered }
    }
}

/// Directions round the head's middle in which its skin is measured.
const AROUND: usize = 64;
const UPWARD: usize = 32;

/// The head as the hair rests on it, in metres from the eye midpoint: an
/// ellipsoid through the skin above the ears for the way hair is combed,
/// and how far the skin itself (ears included) reaches from the middle in
/// every direction, for what hair may not pass.
struct Skull {
    centre: Vec3,
    radii: Vec3,
    reach: Vec<f32>,
}

impl Skull {
    /// `triangles`: the head's skin, three corners each.
    fn of(triangles: &[[Vec3; 3]]) -> Self {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for &p in triangles.iter().flatten().filter(|p| p.y > 0.03) {
            (lo, hi) = (lo.min(p), hi.max(p));
        }
        // The crown is the top of the ellipsoid; its middle sits at brow
        // height, where the head is widest.
        let centre = Vec3::new(0., 0.02, (lo.z + hi.z) * 0.5);
        let radii = Vec3::new(hi.x.max(-lo.x), hi.y - centre.y, (hi.z - lo.z) * 0.5);
        let mut reach = vec![0f32; AROUND * UPWARD];
        for t in triangles {
            // Points every few millimetres over the triangle.
            let n = (((t[1] - t[0]).length().max((t[2] - t[0]).length()) / 0.003).ceil() as usize).clamp(1, 12);
            for a in 0..=n {
                for b in 0..=n - a {
                    let (u, v) = (a as f32 / n as f32, b as f32 / n as f32);
                    let d = t[0] * (1. - u - v) + t[1] * u + t[2] * v - centre;
                    let (x, y) = Self::cell(d);
                    let at = &mut reach[x as usize % AROUND + (y as usize).min(UPWARD - 1) * AROUND];
                    *at = at.max(d.length());
                }
            }
        }
        // Where there is no skin (the open neck), the ellipsoid stands in.
        for y in 0..UPWARD {
            for x in 0..AROUND {
                if reach[x + y * AROUND] == 0. {
                    let lon = (x as f32 + 0.5) / AROUND as f32 * std::f32::consts::TAU - std::f32::consts::PI;
                    let lat = (y as f32 + 0.5) / UPWARD as f32 * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
                    let d = Vec3::new(lon.sin() * lat.cos(), lat.sin(), lon.cos() * lat.cos());
                    reach[x + y * AROUND] = 1. / (d / radii).length();
                }
            }
        }
        // Hair lies over an ear as over a soft rise, not into its folds:
        // every direction takes the furthest skin near it, smoothed.
        for _ in 0..2 {
            let near = |reach: &[f32], x: usize, y: usize, fold: fn(f32, f32) -> f32, start: f32| {
                let mut out = start;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let yy = (y as i32 + dy).clamp(0, UPWARD as i32 - 1) as usize;
                        let xx = (x as i32 + dx).rem_euclid(AROUND as i32) as usize;
                        out = fold(out, reach[xx + yy * AROUND]);
                    }
                }
                out
            };
            let furthest: Vec<f32> = (0..AROUND * UPWARD).map(|i| near(&reach, i % AROUND, i / AROUND, f32::max, 0.)).collect();
            reach = (0..AROUND * UPWARD).map(|i| near(&furthest, i % AROUND, i / AROUND, |a, b| a + b, 0.) / 9.).collect();
        }
        Self { centre, radii, reach }
    }

    /// Where direction `d` falls in the map, in cells.
    fn cell(d: Vec3) -> (f32, f32) {
        let lon = d.x.atan2(d.z);
        let lat = (d.y / d.length().max(1e-9)).clamp(-1., 1.).asin();
        (
            (lon + std::f32::consts::PI) / std::f32::consts::TAU * AROUND as f32,
            (lat + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI * UPWARD as f32,
        )
    }

    /// How far the skin reaches from the middle in direction `d`.
    fn skin(&self, d: Vec3) -> f32 {
        let (x, y) = Self::cell(d);
        let (x, y) = (x - 0.5, (y - 0.5).clamp(0., UPWARD as f32 - 1.));
        let (x0, y0) = (x.floor(), y.floor());
        let at = |i: f32, j: f32| {
            self.reach[(i.rem_euclid(AROUND as f32)) as usize % AROUND + (j as usize).min(UPWARD - 1) * AROUND]
        };
        let (fx, fy) = (x - x0, y - y0);
        (at(x0, y0) * (1. - fx) + at(x0 + 1., y0) * fx) * (1. - fy) + (at(x0, y0 + 1.) * (1. - fx) + at(x0 + 1., y0 + 1.) * fx) * fy
    }

    /// Keep `p` at least `clear` metres outside the skin.
    fn rest(&self, p: Vec3, clear: f32) -> Vec3 {
        let d = p - self.centre;
        let r = d.length();
        if r < 1e-5 {
            return p;
        }
        let skin = self.skin(d);
        if r >= skin + clear { p } else { self.centre + d / r * (skin + clear) }
    }

    /// Metres between `p` and the skin under it.
    fn above(&self, p: Vec3) -> f32 {
        let d = p - self.centre;
        d.length() - self.skin(d)
    }

    fn outward(&self, p: Vec3) -> Vec3 {
        ((p - self.centre) / (self.radii * self.radii)).normalize_or(Vec3::Y)
    }
}

/// Hair lying over shorter hair is cut to blend with it: no longer past a
/// point than this many times the length of the hair that grows there.
const BLEND: f32 = 1.1;

/// One strand from a root: POINTS positions in metres from the eye midpoint.
fn grow(root: Vec3, normal: Vec3, cut: Cut, skull: &Skull, id: u32) -> [Vec3; POINTS] {
    let roll = |k: u32| unit(hash(id.wrapping_mul(0x85eb_ca6b) ^ k));
    // Hair lies as its lock does: neighbours share length and bend.
    let lock = |k: u32| 0.5 + 0.5 * noise(root, 0.012, cut.seed ^ k);
    let uneven = 0.78 + 0.2 * lock(11) + 0.12 * roll(1);
    let length = cut.length(root, skull.centre.y + skull.radii.y) * uneven;
    let top = smooth(0.05, 0.85, normal.y);
    // The comb: away from the parting on top, back or forward at the front,
    // down everywhere else.
    // Next to the parting hair runs back along it before it turns aside,
    // so the parting is a line and not a bare strip.
    let away = smooth(-0.006, 0.006, root.x - cut.part) * 2. - 1.;
    let fore = smooth(-0.07, -0.01, root.z);
    let back = cut.sweep.max(0.);
    let mut comb = Vec3::new(away * (0.35 + 0.65 * (1. - fore * back)), -0.15, -back * fore - 0.25 * (1. - fore));
    // Hair worn forward grows out of a whorl at the back of the crown:
    // away from it on every side, and round it close by, so that the hair
    // next to the whorl lies over it.
    let forward = (-cut.sweep / 0.6).clamp(0., 1.);
    if forward > 0. {
        // Each lock takes the whorl a little elsewhere, so the locks cross
        // over its middle and none of it is bare.
        let whorl = Vec3::new(cut.part * 0.6 + 0.024 * (lock(23) - 0.5), 0., skull.centre.z - 0.55 * skull.radii.z + 0.024 * (lock(29) - 0.5));
        let from = Vec3::new(root.x - whorl.x, 0., root.z - whorl.z);
        let far = from.length();
        let outward = from / far.max(1e-4);
        let round = Vec3::new(-outward.z, 0., outward.x) * if cut.seed & 1 == 0 { 1. } else { -1. };
        let turn = 0.9 * smooth(0.05, 0.0, far) + 0.15;
        // At the front it comes down over the forehead as a fringe.
        comb = comb.lerp((outward * (1. - turn) + round * turn).normalize_or(Vec3::Z) + Vec3::new(0., -0.15 - 1.2 * fore, 0.), forward);
    }
    // Bangs: the hair above the forehead comes forward and down, whatever
    // the rest does.
    // Lock by lock, so that where bangs end the two lie through each other
    // and no parting opens between them.
    let front = smooth(-0.065, -0.030, root.z) * smooth(0.078, 0.045, root.x.abs());
    let bangs = cut.bangs * smooth(lock(43) - 0.15, lock(43) + 0.15, front * 1.3);
    comb = comb.lerp(Vec3::new(0.2 * away, -0.8, 1.), bangs);
    let flat = |v: Vec3, n: Vec3| (v - n * v.dot(n)).normalize_or(Vec3::NEG_Y);
    // Off the top hair lies down the head, except at a front that is
    // combed back: down from the hairline is over the face.
    let mut lie = flat(comb.lerp(Vec3::NEG_Y, (1. - top) * (1. - fore * back.min(1.)) * (1. - bangs)), normal);
    // Tousled hair: every lock lies a little its own way.
    let stray = (lock(31) - 0.5) * 2.4 * cut.mess * (0.3 + 0.7 * top);
    lie = (lie * stray.cos() + normal.cross(lie) * stray.sin()).normalize_or(lie);
    // Hair stands up most on top; at the sides it lies down the head.
    // Coiled hair stands off the head all round.
    let lift = (cut.lift * (0.7 + 0.6 * lock(13)) * (0.5 + 0.5 * top.max(cut.coil))).clamp(0.03, 0.9);
    let mut dir = (normal * lift + lie * (1. - lift)).normalize();
    let across = dir.cross(normal).normalize_or(Vec3::X);
    let phase = std::f32::consts::TAU * lock(17);
    let swing = |s: f32| cut.wave * (cut.pitch * s + phase).sin() * (s / 0.04).clamp(0., 1.);
    // Where this strand is cut: the hem, a little ragged, and thinned so
    // the ends are not one edge. Hair that grows below the hem (the nape
    // under a bob) is kept short instead.
    let hem = (cut.hem + 0.012 * (lock(37) - 0.5) + 0.03 * roll(7).powi(3)).min(root.y - 0.015 - 0.01 * roll(9));
    let fringe = cut.fringe + 0.006 * (lock(19) - 0.5) + 0.004 * roll(11);
    let face = (-0.018 - 0.014 * roll(5), 0.058 + 0.008 * lock(41));
    // The strand is laid in steps much shorter than its segments.
    let pace = (length / 18.).clamp(0.0008, 0.005 - 0.004 * cut.coil);
    // A coil: the strand winds round its own line.
    let spiral = (0.0022 + 0.0016 * roll(13), std::f32::consts::TAU / (0.010 + 0.008 * roll(15)), std::f32::consts::TAU * roll(17));
    let wind = |along: f32, dir: Vec3| {
        let u = dir.cross(normal).normalize_or(Vec3::X);
        let v = dir.cross(u);
        let turn = spiral.1 * along + spiral.2;
        (u * turn.cos() + v * turn.sin()) * (spiral.0 * cut.coil * (along / 0.004).min(1.))
    };
    let mut path = Vec::with_capacity(64);
    path.push(root);
    let mut p = root;
    let mut along = 0.;
    let mut limit = length;
    while along < limit && path.len() < 400 {
        let step = pace.min(limit - along);
        // Weight bends a strand more the longer it reaches past its root;
        // stray hairs stand off a little.
        // Coils carry their own weight.
        let sag = ((along / 0.06).min(1.) * 0.55 + 0.05) * (1. - 0.9 * cut.coil);
        // On top of the head hair lies where the comb left it: its weight
        // presses it down but does not drag it downhill (that parts it in
        // a star round the highest point). Past a few centimetres it
        // settles onto the head; where the head falls away, it hangs.
        let out = skull.outward(p);
        let carried = smooth(0.05, 0.45, out.y);
        let settle = carried * smooth(0.02, 0.08, along) * (step / 0.012).min(1.) * (1. - cut.coil);
        // Long hair that has left the head there is pressed back onto it.
        let press = carried * sag * smooth(0.03, 0.10, along) * smooth(0.002, 0.012, skull.above(p)) * 0.5;
        dir = (dir - out * (dir.dot(out).max(0.) * settle + press * step / 0.012) + Vec3::NEG_Y * sag * (1. - carried) * step / 0.012 * 0.25).normalize();
        let next = p + dir * step + across * (swing(along + step) - swing(along));
        along += step;
        // Rest on the skull, lower layers under upper ones.
        let clear = 0.002 + 0.006 * roll(3) * (along / length.max(1e-4)).min(1.) + 0.004 * lift;
        // Hair leaves the scalp where it grows and rises over what stands
        // out next to it (an ear) on a slope, not at once.
        let rise = (root - skull.centre).length() + 0.003 + 0.7 * along;
        let mut rested = skull.rest(next, clear);
        let (from, to) = ((next - skull.centre).length(), (rested - skull.centre).length());
        if to > rise.max(from) {
            rested = skull.centre + (rested - skull.centre) * (rise.max(from) / to);
        }
        if rested != next {
            // Slide along the head instead of into it.
            let out = skull.outward(rested);
            dir = flat(dir, out).lerp(dir, 0.2).normalize();
        }
        // Cut at the hem, and where it would come down over the face (a
        // fringe across the forehead; beside the face it hangs free).
        let over_face = rested.z > face.0 && rested.x.abs() < face.1;
        // Bangs are cut straight across, also where they reach past it.
        let floor = if over_face || bangs > 0.5 { fringe.max(hem) } else { hem };
        if rested.y < floor {
            if p.y > floor {
                path.push(p + (rested - p) * ((p.y - floor) / (p.y - rested.y)));
            }
            break;
        }
        p = rested;
        path.push(skull.rest(p + wind(along, dir), 0.001));
        // Lying over shorter hair, it is cut to blend with it.
        limit = limit.min(along + BLEND * cut.length(p, skull.centre.y + skull.radii.y) * uneven);
    }
    // Ten points along what was laid, closer together where it bends
    // (over the head) than where it hangs straight: a radian of bend
    // counts as BEND metres of hair.
    const BEND: f32 = 0.06;
    let mut measure = vec![0f32; path.len()];
    for k in 1..path.len() {
        let bend = if k >= 2 { (path[k] - path[k - 1]).angle_between(path[k - 1] - path[k - 2]) } else { 0. };
        measure[k] = measure[k - 1] + path[k].distance(path[k - 1]) + BEND * if bend.is_finite() { bend } else { 0. };
    }
    let whole = measure[path.len() - 1];
    let mut points = [root; POINTS];
    let mut at = 1;
    for (k, point) in points.iter_mut().enumerate().skip(1) {
        if path.len() < 2 {
            break;
        }
        let want = whole * k as f32 / (POINTS - 1) as f32;
        while at < path.len() - 1 && measure[at] < want {
            at += 1;
        }
        let span = measure[at] - measure[at - 1];
        *point = path[at - 1].lerp(path[at], if span > 0. { ((want - measure[at - 1]) / span).clamp(0., 1.) } else { 1. });
    }
    points
}

/// Added to a strand's shade when it is a grey hair (player_strands.wgsl).
pub(super) const GREY: f32 = 2.;

/// The strands of one person: POINTS points each, as [x, y, z, shade] in
/// the fitted body's normalized units, for a groom whose fit scales x and z
/// by `skull`. `eyes` is the eye midpoint in the same units.
pub(super) fn strands(body: &[BodyVertex], indices: &[u32], eyes: Vec3, skull: f32, cut: Cut) -> Vec<[f32; 4]> {
    let metres = |p: Vec3| (p - eyes) * METRES;
    let triangles: Vec<[&BodyVertex; 3]> = indices
        .chunks_exact(3)
        .map(|t| [0, 1, 2].map(|k| &body[t[k] as usize]))
        .filter(|t| t[0].material == FACE)
        .collect();
    let head: Vec<[Vec3; 3]> = triangles.iter().map(|t| t.map(|v| metres(v.position))).collect();
    let skull_shape = Skull::of(&head);
    // Hair-bearing area, to share the strands out evenly.
    let cover = |t: &[&BodyVertex; 3]| {
        let p = t.map(|v| metres(v.position));
        0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length() * cut.grows((p[0] + p[1] + p[2]) / 3.)
    };
    let total: f32 = triangles.iter().map(cover).sum();
    if total <= 0. {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(STRANDS * POINTS);
    let mut owed = 0.;
    let mut id = cut.seed;
    for t in &triangles {
        owed += cover(t) / total * STRANDS as f32;
        while owed >= 1. && out.len() < STRANDS * POINTS {
            owed -= 1.;
            id = hash(id.wrapping_add(0x9e37_79b9));
            let (mut u, mut v) = (unit(id), unit(hash(id ^ 0x51)));
            if u + v > 1. {
                (u, v) = (1. - u, 1. - v);
            }
            let w = 1. - u - v;
            let root = metres(t[0].position * w + t[1].position * u + t[2].position * v);
            // Thin out along the hairline rather than end at a triangle edge.
            if cut.grows(root) < unit(hash(id ^ 0x77)) {
                continue;
            }
            let normal = (t[0].normal * w + t[1].normal * u + t[2].normal * v).normalize_or(Vec3::Y);
            let mut shade = 0.62 + 0.55 * unit(hash(id ^ 0x99)) * (0.6 + 0.4 * noise(root, 0.01, cut.seed ^ 5).abs());
            // Grey hairs come first at the temples. The strand renderer
            // draws a shade of GREY and up without the hair's colour.
            let temples = smooth(0.03, 0.07, root.x.abs()) * smooth(-0.10, -0.03, root.z);
            if unit(hash(id ^ 0x6e)) < cut.grey * (0.7 + 0.8 * temples) {
                shade += GREY;
            }
            let grown = grow(root, normal, cut, &skull_shape, id);
            // A strand cut at its root is no hair.
            if grown[1] == grown[0] {
                continue;
            }
            for p in grown {
                let q = p / METRES + eyes;
                out.push([q.x / skull, q.y, q.z / skull, shade]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(seed: u64) -> (Vec<BodyVertex>, Vec<u32>, Vec3) {
        let person = render::humans::Instance::person(seed, Vec3::ZERO, 0., &Default::default());
        let (body, indices) = person.fitted_mesh(false);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for v in body.iter().filter(|v| v.material == 6) {
            (lo, hi) = (lo.min(v.position), hi.max(v.position));
        }
        (body, indices, (lo + hi) * 0.5)
    }

    fn metres(strand: &[[f32; 4]], eyes: Vec3, k: usize) -> Vec3 {
        (Vec3::new(strand[k][0], strand[k][1], strand[k][2]) - eyes) * METRES
    }

    #[test]
    fn hair_grows_from_the_scalp_and_stays_outside_the_head() {
        let (body, indices, eyes) = person(7);
        let cut = Cut::styled(3, Style::SidePart);
        let hair = strands(&body, &indices, eyes, 1., cut);
        assert!(hair.len() > STRANDS * POINTS * 8 / 10, "a full head: {} points", hair.len());
        assert_eq!(hair.len() % POINTS, 0);
        let mut face = 0;
        for strand in hair.chunks_exact(POINTS) {
            let at = |k: usize| metres(strand, eyes, k);
            assert!(scalp(at(0), 0.) > 0., "root on the scalp: {}", at(0));
            let length: f32 = (1..POINTS).map(|k| at(k).distance(at(k - 1))).sum();
            assert!(length > 0.001 && length < 0.12, "length {length}");
            // Nothing hangs in front of the eyes.
            face += usize::from((0..POINTS).any(|k| at(k).z > 0.0 && at(k).y.abs() < 0.012 && at(k).x.abs() < 0.05));
        }
        assert!(face < STRANDS / 100, "{face} strands across the eyes");
    }

    /// A short cut is short at the ears and the nape, and the longer hair
    /// on top does not hang over them.
    #[test]
    fn short_cuts_taper_to_the_hairline() {
        let (body, indices, eyes) = person(7);
        for style in [Style::Crop, Style::SidePart, Style::SweptBack, Style::Fringe] {
            let hair = strands(&body, &indices, eyes, 1., Cut::styled(5, style));
            for strand in hair.chunks_exact(POINTS) {
                let tip = metres(strand, eyes, POINTS - 1);
                if tip.z < -0.05 {
                    assert!(tip.y > hairline(tip, 0.) - 0.03, "{style:?}: hair ends {} m under the hairline", hairline(tip, 0.) - tip.y);
                }
            }
        }
    }

    /// Long cuts end at their hem, all round.
    #[test]
    fn long_cuts_end_at_the_hem() {
        let (body, indices, eyes) = person(7);
        let cut = Cut::styled(5, Style::Bob);
        let hair = strands(&body, &indices, eyes, 1., cut);
        let mut at_hem = 0;
        for strand in hair.chunks_exact(POINTS) {
            let tip = metres(strand, eyes, POINTS - 1);
            assert!(tip.y > cut.hem - 0.03, "hair ends {} m under the hem", cut.hem - tip.y);
            at_hem += usize::from(tip.y < cut.hem + 0.04);
        }
        assert!(at_hem > hair.len() / POINTS / 2, "{at_hem} strands reach the hem");
    }

    #[test]
    fn cuts_differ_by_seed() {
        let cuts: Vec<Cut> = (0..200).map(|s| Cut::new(s, 0.65, 30.)).collect();
        assert!(cuts.iter().any(|c| c.top < 0.03) && cuts.iter().any(|c| c.hem > UNCUT), "{cuts:?}");
        // Long hair is rare.
        let long = cuts.iter().filter(|c| c.hem > UNCUT && c.hem < -0.15).count();
        assert!(long < 30, "{long} of 200 with long hair");
    }

    #[test]
    fn dark_skin_mostly_has_coiled_hair_and_age_greys() {
        let coiled = |skin: f32| (0..200).filter(|&s| Cut::new(s, skin, 30.).coil > 0.5).count();
        assert!(coiled(0.2) > 150 && coiled(0.7) == 0, "{} dark, {} fair", coiled(0.2), coiled(0.7));
        let grey = |years: f32| (0..200).map(|s| Cut::new(s, 0.6, years).grey).sum::<f32>() / 200.;
        assert!(grey(20.) == 0. && grey(50.) < 0.25 && grey(80.) > 0.6, "{} at 20, {} at 50, {} at 80", grey(20.), grey(50.), grey(80.));
        let (body, indices, eyes) = person(7);
        let cut = Cut { coil: 1., ..Cut::styled(9, Style::Afro) };
        let hair = strands(&body, &indices, eyes, 1., cut);
        // An afro stands off the head: its tips are well outside the roots.
        let middle = Vec3::new(0., 0.02, -0.09);
        let out: f32 = hair.chunks_exact(POINTS).map(|s| metres(s, eyes, POINTS - 1).distance(middle) - metres(s, eyes, 0).distance(middle)).sum::<f32>() / (hair.len() / POINTS) as f32;
        assert!(out > 0.012, "tips {out} m further out than roots");
    }
}
