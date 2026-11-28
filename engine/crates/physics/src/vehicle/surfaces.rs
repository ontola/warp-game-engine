//! Grip per surface. The tyre itself is one measured Magic Formula tyre
//! (`tyre.rs`, a 205/60R15 passenger tyre fitted on dry asphalt); a surface
//! scales its friction (the MF scale factors LMUX and LMUY) by how much a
//! road tyre grips there compared with dry asphalt.
//!
//! Sources for `mu` (peak friction relative to dry asphalt, 0.85 in the
//! same table), `sliding` (sliding over peak friction; it sets how much grip
//! is left past the peak) and `rolling` (rolling resistance coefficient):
//! - J.Y. Wong, Theory of Ground Vehicles, 2nd ed. 1993, p. 26 (peak/sliding
//!   friction: dry asphalt 0.8-0.9/0.75, wet asphalt 0.5-0.7/0.45-0.6, gravel
//!   0.6/0.55, dry earth road 0.68/0.65, wet earth road 0.55/0.4-0.5) and
//!   p. 17 (rolling: medium hard soil 0.08).
//! - Bosch Automotive Handbook, 4th ed. 1996, p. 335 (wet road at 50 km/h, new
//!   tyre: dry 0.85, wet 0.65, puddles 0.50) and p. 330 (rolling: asphalt
//!   0.013, rolled gravel 0.02, unpaved road 0.05, field 0.10-0.35).
//! - P.D. Cenek, N.J. Jamieson, M.W. McLarin, "Frictional characteristics of
//!   roadside grass types" (Opus, NZ), Table 4: locked-wheel braking of a car
//!   (non-ABS Hyundai Sonata) from 35-45 km/h on rye-grass: 0.36-0.38 dry,
//!   0.21-0.24 wet; white clover 0.21 dry, 0.17 wet.
//! - Sand: Agarwal et al., "Modeling of the interaction of rigid wheels with
//!   dry granular media" (arXiv 1901.10667, 2019), Table 1, measured medium
//!   sand: internal friction angle 34 deg, cohesion 1.5 kPa. A tyre's thrust on
//!   sand is bounded by the sand's shear strength, c*A + W*tan(phi) (Bekker,
//!   Janosi-Hanamoto), so peak = tan 34 deg + 1.5 kPa * ~0.02 m^2 / 3000 N
//!   = 0.68. Loose dry sand has no peak in its shear curve, so sliding keeps
//!   the peak. Rolling: car tyre on solid sand 0.04-0.08, loose sand 0.2-0.4
//!   (Engineering ToolBox rolling friction table, no primary source given);
//!   the table takes firm sand, mid range.
//! Loose gravel has no sourced friction value yet; it keeps the ratio of the
//! earlier table (marked below). Grass's peak is not measured either: it
//! takes gravel's sliding/peak ratio over the measured sliding friction.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum Surface {
    Gravel = 0,
    Loose = 1,
    Grass = 2,
    Dirt = 3,
    Water = 4,
    Asphalt = 5,
    Mud = 6,
    Sand = 7,
}

impl Surface {
    pub const ALL: [Surface; 8] = [Surface::Gravel, Surface::Loose, Surface::Grass, Surface::Dirt, Surface::Water, Surface::Asphalt, Surface::Mud, Surface::Sand];

    pub fn from_u8(v: u8) -> Surface {
        *Surface::ALL.get(v as usize).unwrap_or(&Surface::Gravel)
    }

    pub fn name(self) -> &'static str {
        ["gravel", "loose", "grass", "dirt", "water", "asphalt", "mud", "sand"][self as usize]
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Grip {
    /// Friction relative to the reference tyre on dry asphalt (1.0).
    pub mu: f64,
    /// Sliding friction as a share of the peak.
    pub sliding: f64,
    /// Rolling resistance coefficient.
    pub rolling: f64,
    /// Soft ground dragging at the car body, N per (m/s) at 3000 N wheel load.
    pub drag: f64,
}

/// The authored table, then weather applied on a copy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Surfaces {
    pub table: [Grip; 8],
}

const fn g(mu: f64, sliding: f64, rolling: f64, drag: f64) -> Grip {
    Grip { mu, sliding, rolling, drag }
}

/// Dry asphalt's peak friction in the source tables, the reference for `mu`.
const DRY: f64 = 0.85;
/// Gravel's sliding over peak friction (Wong), used for loose surfaces.
const LOOSE_RATIO: f64 = 0.55 / 0.60;
/// Locked-wheel friction on rye-grass, dry and wet (Cenek, mean of long and short).
const GRASS_SLIDE: f64 = 0.37;
const WET_GRASS_SLIDE: f64 = 0.225;

/// Peak traction on dry sand: tan(34 deg) plus cohesion (Agarwal et al.).
const SAND: f64 = 0.68;

pub const TABLE: [Grip; 8] = [
    g(0.60 / DRY, 0.55 / 0.60, 0.020, 0.0),   // gravel (Wong), rolled gravel (Bosch)
    g(0.52 / DRY, 0.55 / 0.60, 0.050, 0.0),   // loose gravel: earlier ratio to gravel (0.86), unsourced; unpaved road
    g(GRASS_SLIDE / LOOSE_RATIO / DRY, LOOSE_RATIO, 0.100, 120.0), // dry rye-grass, sliding 0.37 (Cenek); field, low end (Bosch)
    g(0.68 / DRY, 0.65 / 0.68, 0.050, 200.0), // dry earth road (Wong); unpaved road (Bosch)
    g(0.50 / DRY, 0.525 / 0.60, 0.050, 0.0),  // standing water, 2 mm puddles (Bosch); wet asphalt's sliding level
    g(1.0, 0.75 / DRY, 0.013, 0.0),           // dry asphalt (Wong); asphalt (Bosch)
    g(0.55 / DRY, 0.45 / 0.55, 0.080, 200.0), // wet earth road (Wong); medium hard soil (Wong)
    g(SAND / DRY, 1.0, 0.060, 0.0),           // dry medium sand shear strength (Agarwal); firm sand rolling
];

impl Default for Surfaces {
    fn default() -> Self {
        Surfaces { table: TABLE }
    }
}

impl Surfaces {
    pub fn get(&self, s: Surface) -> &Grip {
        &self.table[s as usize]
    }

    /// Wet asphalt, grass and mud lose grip; the authored table stays untouched.
    /// Wet asphalt is Bosch's wet road (0.65 of 0.85) with 20% more rolling
    /// resistance and Wong's wet sliding level; wet mud keeps the earlier ratio
    /// (0.8), unsourced, with Wong's low end of wet earth sliding (0.40).
    pub fn with_wetness(base: &Surfaces, wet: f64) -> Surfaces {
        let a = wet.clamp(0.0, 1.0);
        let mut s = base.clone();
        let l = |x: f64, y: f64| x + (y - x) * a;
        let asph = Surface::Asphalt as usize;
        let mud = Surface::Mud as usize;
        let grass = Surface::Grass as usize;
        // Wet rye-grass: Cenek's measured locked-wheel friction.
        s.table[grass].mu = l(base.table[grass].mu, base.table[grass].mu * WET_GRASS_SLIDE / GRASS_SLIDE);
        s.table[asph].mu = l(base.table[asph].mu, 0.65 / DRY);
        s.table[asph].sliding = l(base.table[asph].sliding, 0.525 / 0.60);
        s.table[asph].rolling = l(base.table[asph].rolling, base.table[asph].rolling * 1.2);
        s.table[mud].mu = l(base.table[mud].mu, base.table[mud].mu * 0.8);
        s.table[mud].sliding = l(base.table[mud].sliding, 0.40 / 0.55);
        s.table[mud].rolling = l(base.table[mud].rolling, 0.10);
        s.table[mud].drag = l(base.table[mud].drag, 260.0);
        s
    }
}
