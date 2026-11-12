//! Where the sun is for a place and time (NOAA's solar position
//! equations, within about a degree), as a direction in a house's glTF
//! frame.
use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// A house's place on Earth.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub latitude: f32,
    pub longitude: f32,
    /// Compass bearing of the plan's +y axis (glTF -z) in degrees: 0 when
    /// the plan's up points north, 90 when it points east.
    pub north_deg: f32,
}

impl Site {
    /// Unit vector towards the sun on day `day_of_year` (1 = 1 January) at
    /// `utc_hours`. Below the horizon its y is negative.
    pub fn sun_dir(&self, day_of_year: u32, utc_hours: f32) -> Vec3 {
        let (elevation, azimuth) = solar_position(self.latitude, self.longitude, day_of_year, utc_hours);
        // Bearing relative to the plan's +y, clockwise towards plan +x;
        // plan (x, y) is glTF (x, -z).
        let b = azimuth - self.north_deg.to_radians();
        Vec3::new(b.sin() * elevation.cos(), elevation.sin(), -b.cos() * elevation.cos())
    }
}

/// Solar elevation and azimuth (from north, clockwise) in radians.
pub fn solar_position(latitude: f32, longitude: f32, day_of_year: u32, utc_hours: f32) -> (f32, f32) {
    let g = TAU / 365.0 * (day_of_year as f32 - 1.0 + (utc_hours - 12.0) / 24.0);
    let eqtime = 229.18 * (0.000075 + 0.001868 * g.cos() - 0.032077 * g.sin() - 0.014615 * (2.0 * g).cos() - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos() + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos() + 0.00148 * (3.0 * g).sin();
    let solar_minutes = utc_hours * 60.0 + eqtime + 4.0 * longitude;
    let ha = (solar_minutes / 4.0 - 180.0).to_radians();
    let lat = latitude.to_radians();
    let elevation = (lat.sin() * decl.sin() + lat.cos() * decl.cos() * ha.cos()).clamp(-1.0, 1.0).asin();
    let azimuth = ha.sin().atan2(ha.cos() * lat.sin() - decl.tan() * lat.cos()) + PI;
    (elevation, azimuth.rem_euclid(TAU))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_in_amsterdam() {
        // Amsterdam, 21 June, 11:40 UTC (about local solar noon): sun due
        // south at about 61 degrees.
        let (el, az) = solar_position(52.37, 4.9, 172, 11.67);
        assert!((el.to_degrees() - 61.0).abs() < 1.5, "{}", el.to_degrees());
        assert!((az.to_degrees() - 180.0).abs() < 5.0, "{}", az.to_degrees());
        // Morning: east of south.
        let (_, az) = solar_position(52.37, 4.9, 172, 7.0);
        assert!(az.to_degrees() > 60.0 && az.to_degrees() < 120.0, "{}", az.to_degrees());
    }

    #[test]
    fn plan_frame() {
        // Plan up = north: a sun due south is towards glTF +z.
        let site = Site { latitude: 52.37, longitude: 4.9, north_deg: 0.0 };
        let d = site.sun_dir(172, 11.67);
        assert!(d.z > 0.4 && d.x.abs() < 0.1, "{d}");
    }
}
