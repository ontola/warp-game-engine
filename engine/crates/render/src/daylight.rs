//! One daylight environment per world. Bake small, sun-disc-free radiance maps
//! once, then interpolate their radiance and diffuse light as the sun moves.
use crate::atmosphere::{AtmosphereCache, BakeQuality, BakedSky, SolarState};
use crate::sky::{WeatherLook, exposure_for_elevation};
use glam::DVec3;
use std::sync::Arc;

// Extra samples at the horizon capture the rapid change in atmospheric extinction.
const HOURS: &[f32] = &[
    0., 4., 5., 5.5, 5.75, 6., 6.25, 6.5, 7., 8., 10., 12., 14., 16., 17., 17.5, 17.75, 18., 18.25,
    18.5, 19., 20., 24.,
];

/// Equinox at 30° north. World east is +X and north is -Z.
pub fn sun_at(hour: f32) -> SolarState {
    let angle = (f64::from(hour.rem_euclid(24.)) - 12.) * std::f64::consts::TAU / 24.;
    let latitude = 30_f64.to_radians();
    let dir = DVec3::new(
        -angle.sin(),
        latitude.cos() * angle.cos(),
        latitude.sin() * angle.cos(),
    );
    SolarState {
        elevation_degrees: dir.y.asin().to_degrees(),
        azimuth_degrees: dir.x.atan2(-dir.z).to_degrees().rem_euclid(360.),
    }
}

pub struct DaylightCycle {
    pub weather: WeatherLook,
    maps: Vec<Arc<BakedSky>>,
    peaks: Vec<f32>,
    sky: BakedSky,
    limit: f32,
    hour: Option<f32>,
}

impl DaylightCycle {
    pub fn new(weather: WeatherLook) -> Result<Self, String> {
        let quality = BakeQuality {
            sky_width: 64,
            sky_height: 32,
            sky_steps: 24,
            ..BakeQuality::default()
        };
        let mut cache = AtmosphereCache::default();
        let model = weather.model();
        let mut maps = Vec::with_capacity(HOURS.len());
        for &hour in &HOURS[..HOURS.len() - 1] {
            maps.push(cache.get_or_bake(&model, sun_at(hour), quality)?);
        }
        maps.push(maps[0].clone());
        let peaks = maps
            .iter()
            .map(|s| {
                s.pixels
                    .iter()
                    .flat_map(|p| p[..3].iter().copied())
                    .fold(0., f32::max)
            })
            .collect();
        Ok(Self {
            weather,
            sky: (*maps[0]).clone(),
            maps,
            peaks,
            limit: 0.,
            hour: None,
        })
    }

    pub fn sky(&self) -> &BakedSky {
        &self.sky
    }
    pub fn radiance_limit(&self) -> f32 {
        self.limit
    }

    pub fn sample(&mut self, hour: f32) -> bool {
        let hour = hour.rem_euclid(24.);
        if self.hour == Some(hour) {
            return false;
        }
        self.hour = Some(hour);
        let i = HOURS
            .partition_point(|h| *h <= hour)
            .saturating_sub(1)
            .min(HOURS.len() - 2);
        let f = (hour - HOURS[i]) / (HOURS[i + 1] - HOURS[i]);
        let (a, b) = (&self.maps[i], &self.maps[i + 1]);
        let dst = &mut self.sky;
        dst.solar = sun_at(hour);
        dst.sun_dir = dst.solar.direction().as_vec3();
        dst.sun_rgb_lux = a.sun_rgb_lux.lerp(b.sun_rgb_lux, f);
        dst.sky_horizontal_rgb_lux = a.sky_horizontal_rgb_lux.lerp(b.sky_horizontal_rgb_lux, f);
        dst.horizon = a.horizon.lerp(b.horizon, f);
        dst.night_ambient_rgb_lux = a.night_ambient_rgb_lux.lerp(b.night_ambient_rgb_lux, f);
        dst.night_stars[0] = a.night_stars[0] + f * (b.night_stars[0] - a.night_stars[0]);
        self.limit = self.peaks[i] + f * (self.peaks[i + 1] - self.peaks[i]);
        for ((dst, a), b) in dst
            .sh
            .iter_mut()
            .zip(&a.sh)
            .zip(&b.sh)
            .chain(dst.pixels.iter_mut().zip(&a.pixels).zip(&b.pixels))
        {
            for k in 0..3 {
                dst[k] = a[k] + f * (b[k] - a[k]);
            }
        }
        true
    }

    pub fn apply_lighting(&self, light: &mut crate::Lighting) {
        self.sky.apply_lighting(light);
        light.ev100 = exposure_for_elevation(self.sky.solar.elevation_degrees);
        (light.fog_start, light.fog_end, light.fog_density) = self.weather.fog();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daylight_crosses_the_clock_seam_and_former_preset_boundaries_continuously() {
        let mut cycle = DaylightCycle::new(WeatherLook::Clear).unwrap();
        for hour in [0., 5.5, 6., 7., 11., 14., 18., 19., 20.] {
            cycle.sample(hour - 0.0005);
            let before = cycle.sky().clone();
            cycle.sample(hour + 0.0005);
            let after = cycle.sky();
            assert!(before.sun_dir.distance(after.sun_dir) < 0.0004);
            assert!(
                (before.horizon - after.horizon).length() < before.horizon.length() * 0.02 + 0.05
            );
            assert!(
                before
                    .pixels
                    .iter()
                    .flatten()
                    .zip(after.pixels.iter().flatten())
                    .all(|(a, b)| (a - b).abs() < a.abs() * 0.08 + 0.5)
            );
        }
        cycle.sample(12.);
        assert!(cycle.sky().sun_dir.y > 0.85 && cycle.sky().sun_rgb_lux.max_element() > 1000.);
        assert!(!cycle.sample(12.));
        cycle.sample(0.);
        assert_eq!(cycle.sky().sun_rgb_lux, glam::Vec3::ZERO);
    }
}
