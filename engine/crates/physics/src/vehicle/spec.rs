use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CarSpec {
    pub name: String,
    #[serde(default)]
    pub model: String,
    pub mass: f64,
    pub inertia: [f64; 3],
    pub cg: [f64; 3],
    pub wheelbase: f64,
    pub track: f64,
    #[serde(default)]
    pub track_rear: Option<f64>,
    pub wheel_radius: f64,
    pub wheel_inertia: f64,
    pub droop_y: f64,
    pub travel: f64,
    pub spring: [f64; 2],
    pub bump: [f64; 2],
    pub rebound: [f64; 2],
    pub arb: [f64; 2],
    pub max_steer_deg: f64,
    /// Multiplies each surface's tyre friction, retaining weather differences.
    /// Old car files retain their original handling with an implicit 1.0.
    #[serde(default = "unit_scale", deserialize_with = "tyre_scale")]
    pub tyre_grip_scale: f64,
    /// Multiplies the lateral slip at peak force; lower values bite earlier.
    #[serde(default = "unit_scale", deserialize_with = "tyre_scale")]
    pub lateral_peak_scale: f64,
    pub brake: [f64; 2],
    pub handbrake: f64,
    pub drive: [f64; 2],
    pub centre_lock: f64,
    pub diff_preload: [f64; 2],
    pub diff_ramp: [f64; 2],
    pub idle_rpm: f64,
    pub limit_rpm: f64,
    pub engine_inertia: f64,
    pub torque: Vec<[f64; 2]>,
    pub friction: [f64; 2],
    pub gears: Vec<f64>,
    #[serde(rename = "final")]
    pub final_drive: f64,
    pub shift_time: f64,
    pub clutch: f64,
    pub upshift_rpm: f64,
    pub downshift_rpm: f64,
    pub drag: f64,
    pub downforce: f64,
    #[serde(default)]
    pub ride: f64,
    /// [size x, y, z, centre x, y, z]
    pub boxes: Vec<[f64; 6]>,
}

fn unit_scale() -> f64 { 1.0 }

fn tyre_scale<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(deserializer)?;
    if !value.is_finite() || !(0.5..=1.5).contains(&value) {
        return Err(serde::de::Error::custom("tyre handling scale must be finite and within 0.5..1.5"));
    }
    Ok(value)
}

impl CarSpec {
    pub fn max_steer(&self) -> f64 {
        self.max_steer_deg * (std::f64::consts::PI / 180.0)
    }
    pub fn track_rear(&self) -> f64 {
        self.track_rear.unwrap_or(self.track)
    }
}

/// A compact four-wheel-drive road car for games using the stock fleet.
impl CarSpec {
    pub fn compact() -> Self {
        Self {
            name: "Compact".into(), model: String::new(), mass: 1230.,
            inertia: [1900., 2100., 520.], cg: [0., 0.16, -0.12], wheelbase: 2.50,
            track: 1.56, track_rear: None, wheel_radius: 0.33, wheel_inertia: 1.1,
            droop_y: -0.10, travel: 0.24, spring: [34000., 30000.], bump: [3200., 2800.],
            rebound: [4800., 4200.], arb: [9000., 6000.], max_steer_deg: 35.,
            tyre_grip_scale: 1.12, lateral_peak_scale: 0.8, brake: [1650., 1050.], handbrake: 2600.,
            drive: [0.5, 0.5], centre_lock: 900., diff_preload: [80., 120.], diff_ramp: [0.45, 0.65],
            idle_rpm: 950., limit_rpm: 7100., engine_inertia: 0.16,
            torque: vec![[1000., 130.], [2500., 240.], [4500., 300.], [6000., 260.], [7100., 180.], [7600., 0.]],
            friction: [18., 0.0085], gears: vec![3.20, 3.00, 2.15, 1.67, 1.36, 1.15, 1.00],
            final_drive: 4.40, shift_time: 0.12, clutch: 650., upshift_rpm: 6650., downshift_rpm: 2850.,
            drag: 0.46, downforce: 0.09, ride: 0.,
            boxes: vec![[1.78, 0.54, 4.05, 0., 0.34, 0.], [1.46, 0.54, 1.96, 0., 0.87, 0.1]],
        }
    }
}
