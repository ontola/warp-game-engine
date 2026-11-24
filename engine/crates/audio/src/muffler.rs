//! Plane pressure waves in an exhaust with one expansion chamber. Each pipe
//! carries waves in both directions; its diameter sets its characteristic
//! impedance and its length sets the transit time.

use crate::dsp::OnePole;
use serde::{Deserialize, Deserializer, Serialize};
use std::f32::consts::TAU;

const SOUND_SPEED: f32 = 343.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MufflerGeometry {
    #[serde(deserialize_with = "length")]
    pub inlet_length: f32,
    #[serde(deserialize_with = "pipe_diameter")]
    pub inlet_diameter: f32,
    #[serde(deserialize_with = "length")]
    pub chamber_length: f32,
    #[serde(deserialize_with = "chamber_diameter")]
    pub chamber_diameter: f32,
    #[serde(deserialize_with = "length")]
    pub outlet_length: f32,
    #[serde(deserialize_with = "pipe_diameter")]
    pub outlet_diameter: f32,
    /// Fraction of full absorptive lining on the chamber wall (0..1).
    #[serde(deserialize_with = "packing")]
    pub packing: f32,
}

const LENGTH: (f32, f32) = (0.05, 6.0);
const PIPE: (f32, f32) = (0.015, 0.30);
const CHAMBER: (f32, f32) = (0.015, 0.60);
const PACKING: (f32, f32) = (0.0, 1.0);

fn checked(value: f32, field: &str, (min, max): (f32, f32)) -> Result<f32, String> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!("muffler {field} must be between {min} and {max}"))
    }
}

fn dimension<'de, D: Deserializer<'de>>(
    d: D,
    field: &str,
    range: (f32, f32),
) -> Result<f32, D::Error> {
    checked(f32::deserialize(d)?, field, range).map_err(serde::de::Error::custom)
}
fn length<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    dimension(d, "length (m)", LENGTH)
}
fn pipe_diameter<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    dimension(d, "pipe diameter (m)", PIPE)
}
fn chamber_diameter<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    dimension(d, "chamber diameter (m)", CHAMBER)
}
fn packing<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    dimension(d, "packing", PACKING)
}

impl MufflerGeometry {
    /// Lengths and inside diameters are in metres.
    pub fn validate(&self) -> Result<(), String> {
        checked(self.inlet_length, "inlet length (m)", LENGTH)?;
        checked(self.chamber_length, "chamber length (m)", LENGTH)?;
        checked(self.outlet_length, "outlet length (m)", LENGTH)?;
        checked(self.inlet_diameter, "inlet diameter (m)", PIPE)?;
        checked(self.chamber_diameter, "chamber diameter (m)", CHAMBER)?;
        checked(self.outlet_diameter, "outlet diameter (m)", PIPE)?;
        checked(self.packing, "packing", PACKING)?;
        Ok(())
    }
}

/// A pair of one-way delay lines sharing one cursor. The opposed waves
/// travel the section in the same number of samples.
struct Pipe {
    waves: Box<[[f32; 2]]>,
    pos: usize,
    transmission: f32,
}

impl Pipe {
    fn new(length: f32, sr: f32) -> Self {
        let samples = (length * sr / SOUND_SPEED).round().max(1.0) as usize;
        Self {
            waves: vec![[0.0; 2]; samples].into_boxed_slice(),
            pos: 0,
            transmission: (-0.025 * length).exp(),
        }
    }

    #[inline]
    fn arriving(&self) -> [f32; 2] {
        self.waves[self.pos]
    }

    #[inline]
    fn send(&mut self, right: f32, left: f32) {
        self.waves[self.pos] = [right * self.transmission, left * self.transmission];
        self.pos += 1;
        if self.pos == self.waves.len() {
            self.pos = 0;
        }
    }
}

/// For incoming pressures a and b at a lossless area step, the common
/// junction pressure is a + b + r*(a-b). Subtract each incident pressure to
/// obtain its reflected wave. The ρc factor cancels between the pipes.
fn reflection(left_diameter: f32, right_diameter: f32) -> f32 {
    let left = left_diameter * left_diameter;
    let right = right_diameter * right_diameter;
    (left - right) / (left + right)
}

pub(crate) struct Muffler {
    inlet: Pipe,
    chamber: Pipe,
    outlet: Pipe,
    expansion: f32,
    contraction: f32,
    lining: [OnePole; 2],
    high_transmission: f32,
    open_end: OnePole,
}

impl Muffler {
    pub(crate) fn new(g: MufflerGeometry, sr: f32) -> Option<Self> {
        // Native callers can construct the public geometry directly. Bound
        // delay storage even when the patch didn't pass through serde.
        if g.validate().is_err() || !(8_000.0..=384_000.0).contains(&sr) {
            return None;
        }
        let radius = g.outlet_diameter * 0.5;
        // An unflanged mouth reflects low-frequency pressure inverted. Its
        // acoustic end correction is about 0.61 radii; reflection adds a
        // round trip over this length. Higher frequencies escape the mouth.
        let open_end = OnePole::new(SOUND_SPEED / (TAU * 2.0 * 0.61 * radius), sr);
        // Effective chamber-wall loss grows with its length and wetted
        // perimeter per unit area (4 / diameter). Fibrous lining absorbs
        // short waves more strongly than long waves on every traversal.
        let high_transmission =
            (-0.22 * g.packing * g.chamber_length * 4.0 / g.chamber_diameter).exp();
        let lining = std::array::from_fn(|_| OnePole::new(650.0, sr));
        Some(Self {
            inlet: Pipe::new(g.inlet_length, sr),
            chamber: Pipe::new(g.chamber_length, sr),
            outlet: Pipe::new(g.outlet_length, sr),
            expansion: reflection(g.inlet_diameter, g.chamber_diameter),
            contraction: reflection(g.chamber_diameter, g.outlet_diameter),
            lining,
            high_transmission,
            open_end,
        })
    }

    #[inline]
    pub(crate) fn run(&mut self, source: f32) -> f32 {
        let [in_right, in_left] = self.inlet.arriving();
        let [ch_right, ch_left] = self.chamber.arriving();
        let [out_right, out_left] = self.outlet.arriving();
        let expand = self.expansion * (in_right - ch_left);
        let contract = self.contraction * (ch_right - out_left);
        let mut chamber_waves = [in_right + expand, out_left + contract];
        for (wave, lining) in chamber_waves.iter_mut().zip(&mut self.lining) {
            let low = lining.run(*wave);
            *wave = low + self.high_transmission * (*wave - low);
        }
        // Effective source impedance of the collector: a partially closed
        // engine end returns part of the pressure travelling up the inlet.
        self.inlet.send(source + 0.42 * in_left, ch_left + expand);
        self.chamber.send(chamber_waves[0], chamber_waves[1]);
        let reflected = -self.open_end.run(out_right);
        self.outlet.send(ch_right + contract, reflected);
        // Lip pressure = incident + returning pressure. The derivative-like
        // radiation at long wavelengths arises from the open boundary.
        out_right + reflected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> MufflerGeometry {
        MufflerGeometry {
            inlet_length: 1.85,
            inlet_diameter: 0.050,
            chamber_length: 0.46,
            chamber_diameter: 0.165,
            outlet_length: 0.66,
            outlet_diameter: 0.045,
            packing: 0.78,
        }
    }

    fn impulse(g: MufflerGeometry, sr: f32, n: usize) -> Vec<f32> {
        let mut muffler = Muffler::new(g, sr).unwrap();
        (0..n)
            .map(|i| muffler.run(if i == 0 { 1.0 } else { 0.0 }))
            .collect()
    }

    #[test]
    fn radiation_arrives_after_all_three_duct_transits_and_rings_down() {
        let sr = 34_300.0; // exactly 100 samples per metre
        let samples = impulse(geometry(), sr, 34_300);
        let transit = 185 + 46 + 66;
        assert!(samples[..transit].iter().all(|&v| v == 0.0));
        assert!(samples[transit] > 0.001);
        assert!(
            samples[transit + 90..transit + 110]
                .iter()
                .any(|s| s.abs() > 1e-4),
            "chamber round trip"
        );
        assert!(
            samples[30_000..].iter().all(|s| s.abs() < 1e-6),
            "passive ring-down"
        );
    }

    fn magnitude(response: &[f32], hz: f32, sr: f32) -> f32 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, x) in response.iter().enumerate() {
            let (s, c) = (TAU * hz * (i as f32 / sr)).sin_cos();
            re += c * x;
            im += s * x;
        }
        re.hypot(im)
    }

    #[test]
    fn area_ratio_and_chamber_length_predict_the_transmission_minimum() {
        // Time-gate the initial transmission, before any inlet or outlet
        // echo can return from the long end pipes. The straight reference
        // cancels their losses and the response of the radiating mouth.
        let sr = 34_300.0;
        for length in [0.5, 1.0] {
            let straight = MufflerGeometry {
                inlet_length: 6.0,
                chamber_length: length,
                outlet_length: 6.0,
                inlet_diameter: 0.05,
                chamber_diameter: 0.05,
                outlet_diameter: 0.05,
                packing: 0.0,
            };
            let expanded = MufflerGeometry {
                chamber_diameter: 0.10,
                ..straight
            };
            let first = ((12.0 + length) * sr / SOUND_SPEED).round() as usize;
            let straight = impulse(straight, sr, first + 1200);
            let chamber = impulse(expanded, sr, first + 1200);
            let transmitted = |f| magnitude(&chamber, f, sr) / magnitude(&straight, f, sr);
            let quarter = SOUND_SPEED / (4.0 * length);
            // Expansion to 4× area gives |T|=2/(4+1/4) at each odd
            // quarter wavelength, unity at a half wavelength (slightly
            // less here due to propagation loss on internal round trips).
            let low = transmitted(quarter);
            let high = transmitted(quarter * 2.0);
            assert!(
                (low - 2.0 / 4.25).abs() < 0.025,
                "{length} m, {quarter} Hz: {low}"
            );
            assert!((high - 1.0).abs() < 0.04, "{length} m, half wave: {high}");
        }
    }

    #[test]
    fn packing_absorbs_short_waves_and_dimensions_bound_native_allocations() {
        let g = geometry();
        let bare = impulse(MufflerGeometry { packing: 0.0, ..g }, 48_000.0, 4096);
        let lined = impulse(g, 48_000.0, 4096);
        let power = |x: &[f32]| {
            (2_000..8_000)
                .step_by(80)
                .map(|f| magnitude(x, f as f32, 48_000.0).powi(2))
                .sum::<f32>()
        };
        assert!(power(&lined) < 0.22 * power(&bare));
        for bad in [
            MufflerGeometry {
                inlet_length: f32::NAN,
                ..g
            },
            MufflerGeometry {
                chamber_length: f32::INFINITY,
                ..g
            },
            MufflerGeometry {
                outlet_length: -1.0,
                ..g
            },
            MufflerGeometry { packing: 2.0, ..g },
            MufflerGeometry {
                outlet_diameter: 0.0,
                ..g
            },
        ] {
            assert!(bad.validate().is_err());
            assert!(Muffler::new(bad, 48_000.0).is_none());
        }
        assert!(Muffler::new(g, f32::INFINITY).is_none());
    }

    #[test]
    fn extreme_valid_area_ratios_stay_finite_at_device_rates() {
        let g = MufflerGeometry {
            inlet_diameter: 0.015,
            chamber_diameter: 0.6,
            outlet_diameter: 0.015,
            packing: 0.0,
            ..geometry()
        };
        for sr in [8_000.0, 44_100.0, 96_000.0, 192_000.0] {
            let mut m = Muffler::new(g, sr).unwrap();
            let mut noise = crate::dsp::Noise::new(17);
            for _ in 0..(sr as usize * 2) {
                let x = m.run(noise.white());
                assert!(x.is_finite() && x.abs() < 8.0);
            }
        }
    }
}
