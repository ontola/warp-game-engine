//! Shared CPU residency / GPU distance-fade contract for source vegetation.
//! Profiles change representation range and capacity, never plant membership,
//! root, scale, colour or wind. Extended is an opt-in measurement experiment;
//! it is not a voxel renderer or a completed solution to distant-grass detail.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DistanceProfile {
    #[default]
    Default,
    Extended,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistanceBand {
    pub fade_start: f32,
    pub fade_end: f32,
    /// CPU cache radius around its world-tile anchor. It must exceed the
    /// zero-opacity endpoint by the maximum camera-to-anchor displacement.
    pub residency: f32,
    /// One shared pool for all views, not a capacity per camera.
    pub capacity: usize,
}

impl DistanceProfile {
    /// Environment parsing belongs at the application boundary. Invalid
    /// explicit choices fail instead of silently testing another profile.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("default") => Ok(Self::Default),
            Some("extended") => Ok(Self::Extended),
            Some(value) => Err(format!("unknown vegetation detail profile {value:?}; expected default or extended")),
        }
    }

    pub const fn label(self) -> &'static str {
        match self { Self::Default => "default", Self::Extended => "extended" }
    }

    pub const fn for_views(self, views: usize) -> DistanceBand {
        match (self, views <= 1) {
            (Self::Default, true) => DistanceBand { fade_start:40.0, fade_end:60.0, residency:64.0, capacity:70_000 },
            (Self::Default, false) => DistanceBand { fade_start:20.0, fade_end:38.0, residency:42.0, capacity:115_000 },
            (Self::Extended, true) => DistanceBand { fade_start:80.0, fade_end:120.0, residency:124.0, capacity:250_000 },
            (Self::Extended, false) => DistanceBand { fade_start:50.0, fade_end:80.0, residency:84.0, capacity:460_000 },
        }
    }

    /// Single-view start/end followed by split-view start/end, for one GPU
    /// uniform. Every raster/composite/motion consumer must use this tuple.
    pub const fn shader_ranges(self) -> [f32; 4] {
        let one=self.for_views(1);let split=self.for_views(4);
        [one.fade_start,one.fade_end,split.fade_start,split.fade_end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_unchanged_and_extended_requires_an_explicit_choice() {
        assert_eq!(DistanceProfile::default(),DistanceProfile::Default);
        assert_eq!(DistanceProfile::parse(None).unwrap().shader_ranges(),[40.0,60.0,20.0,38.0]);
        assert_eq!(DistanceProfile::parse(Some("extended")).unwrap().shader_ranges(),[80.0,120.0,50.0,80.0]);
        assert!(DistanceProfile::parse(Some("extendded")).is_err());
        for profile in [DistanceProfile::Default,DistanceProfile::Extended] {for views in 1..=4 {
            let band=profile.for_views(views);
            assert!(0.0<band.fade_start&&band.fade_start<band.fade_end&&band.fade_end<band.residency);
            assert_eq!(band.residency-band.fade_end,4.0);
        }}
    }
}
