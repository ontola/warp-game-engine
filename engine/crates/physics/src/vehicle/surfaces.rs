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
//! Loose gravel and grass have no sourced friction value yet; they keep the
//! ratios of the earlier table (marked below) and gravel's sliding level.

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
}

impl Surface {
    pub const ALL: [Surface; 7] = [Surface::Gravel, Surface::Loose, Surface::Grass, Surface::Dirt, Surface::Water, Surface::Asphalt, Surface::Mud];

    pub fn from_u8(v: u8) -> Surface {
        *Surface::ALL.get(v as usize).unwrap_or(&Surface::Gravel)
    }

    pub fn name(self) -> &'static str {
        ["gravel", "loose", "grass", "dirt", "water", "asphalt", "mud"][self as usize]
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
    pub table: [Grip; 7],
}

const fn g(mu: f64, sliding: f64, rolling: f64, drag: f64) -> Grip {
    Grip { mu, sliding, rolling, drag }
}

/// Dry asphalt's peak friction in the source tables, the reference for `mu`.
const DRY: f64 = 0.85;

pub const TABLE: [Grip; 7] = [
    g(0.60 / DRY, 0.55 / 0.60, 0.020, 0.0),   // gravel (Wong), rolled gravel (Bosch)
    g(0.52 / DRY, 0.55 / 0.60, 0.050, 0.0),   // loose gravel: earlier ratio to gravel (0.86), unsourced; unpaved road
    g(0.42 / DRY, 0.55 / 0.60, 0.100, 120.0), // grass: earlier ratio to gravel (0.70), unsourced; field, low end (Bosch)
    g(0.68 / DRY, 0.65 / 0.68, 0.050, 200.0), // dry earth road (Wong); unpaved road (Bosch)
    g(0.50 / DRY, 0.525 / 0.60, 0.050, 0.0),  // standing water, 2 mm puddles (Bosch); wet asphalt's sliding level
    g(1.0, 0.75 / DRY, 0.013, 0.0),           // dry asphalt (Wong); asphalt (Bosch)
    g(0.55 / DRY, 0.45 / 0.55, 0.080, 200.0), // wet earth road (Wong); medium hard soil (Wong)
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

    /// Wet asphalt and mud lose grip; the authored table stays untouched.
    /// Wet asphalt is Bosch's wet road (0.65 of 0.85) with 20% more rolling
    /// resistance and Wong's wet sliding level; wet mud keeps the earlier ratio
    /// (0.8), unsourced, with Wong's low end of wet earth sliding (0.40).
    pub fn with_wetness(base: &Surfaces, wet: f64) -> Surfaces {
        let a = wet.clamp(0.0, 1.0);
        let mut s = base.clone();
        let l = |x: f64, y: f64| x + (y - x) * a;
        let asph = Surface::Asphalt as usize;
        let mud = Surface::Mud as usize;
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
