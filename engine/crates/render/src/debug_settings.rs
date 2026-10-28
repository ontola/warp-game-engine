//! Live renderer controls. These change visual work, never scene/simulation data.

/// Defaults preserve the renderer's configured features and distance budgets.
/// Enabling a feature cannot create resources absent from the supplied Scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugRenderOptions {
    pub meadow_enabled: bool,
    pub headlights_enabled: bool,
    pub transparency_enabled: bool,
    pub exhaust_enabled: bool,
    pub wheel_particles_enabled: bool,
    /// Post-reconstruction highlight bloom; never modifies temporal history.
    pub bloom_enabled: bool,
    pub bloom_strength: f32,
    /// Blur rounds, each twice as wide: 1 is a small halo, 4 a wide glow
    /// (Godot's glow levels 3-6).
    pub bloom_rounds: u32,
    /// Exposed brightness where bloom starts.
    pub bloom_threshold: f32,
    /// Godot-style glow: smoothstep extraction over 2 stops, the sum of the
    /// wide rounds (levels 3-6) and a screen blend after tonemapping.
    pub bloom_screen: bool,
    /// Scale both meadow representation transitions and its outer fade. This
    /// only shrinks the existing allocation; it does not change root density.
    pub meadow_distance_scale: f32,
    /// Maximum projected source-geometry error in input pixels. Independent
    /// of draw distance and the general cluster-mesh LOD setting.
    pub meadow_lod_pixels: f32,
    /// Blade size about fixed basal anchors; root positions never scale.
    pub meadow_size_scale: f32,
    /// Fraction of the allocated world-cell patches retained (not blade alpha).
    pub meadow_density: f32,
    /// Linear albedo multiplier shared by source, aggregate and substrate.
    pub meadow_color_multiplier: [f32; 3],
}

impl Default for DebugRenderOptions {
    fn default() -> Self {
        Self {
            meadow_enabled: true,
            headlights_enabled: true,
            transparency_enabled: true,
            exhaust_enabled: true,
            wheel_particles_enabled: true,
            bloom_enabled: true,
            bloom_strength: crate::bloom::DEFAULT_STRENGTH,
            bloom_rounds: 1,
            bloom_threshold: crate::bloom::THRESHOLD,
            bloom_screen: false,
            meadow_distance_scale: 1.0,
            meadow_lod_pixels: 1.0,
            meadow_size_scale: 1.0,
            meadow_density: 1.0,
            meadow_color_multiplier: [1.0; 3],
        }
    }
}

impl DebugRenderOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !self.bloom_strength.is_finite() || !(0.0..=2.0).contains(&self.bloom_strength) {
            return Err("bloom strength must be finite and in 0.0..=2.0".into());
        }
        if !(1..=crate::bloom::MAX_ROUNDS).contains(&self.bloom_rounds) || !self.bloom_threshold.is_finite() || !(0.0..=16.0).contains(&self.bloom_threshold) {
            return Err(format!("bloom rounds must be 1..={} and threshold 0..=16", crate::bloom::MAX_ROUNDS));
        }
        if !self.meadow_distance_scale.is_finite()
            || !(0.25..=1.0).contains(&self.meadow_distance_scale)
        {
            return Err("meadow distance scale must be finite and in 0.25..=1.0".into());
        }
        if !self.meadow_lod_pixels.is_finite() || !(0.5..=4.0).contains(&self.meadow_lod_pixels) {
            return Err("meadow LOD error must be finite and in 0.5..=4.0 input pixels".into());
        }
        if !self.meadow_size_scale.is_finite() || !(0.5..=2.0).contains(&self.meadow_size_scale) {
            return Err("meadow size scale must be finite and in 0.5..=2.0".into());
        }
        if !self.meadow_density.is_finite() || !(0.0..=1.0).contains(&self.meadow_density) {
            return Err("meadow patch count fraction must be finite and in 0.0..=1.0".into());
        }
        if self.meadow_color_multiplier.iter().any(|v| !v.is_finite() || !(0.0..=2.0).contains(v)) {
            return Err("meadow RGB multipliers must be finite and in 0.0..=2.0".into());
        }
        Ok(())
    }

    pub(crate) fn resets_history(self, previous: Self) -> bool {
        self.meadow_enabled != previous.meadow_enabled
            || self.headlights_enabled != previous.headlights_enabled
            || self.transparency_enabled != previous.transparency_enabled
            || self.meadow_distance_scale != previous.meadow_distance_scale
            || self.meadow_lod_pixels != previous.meadow_lod_pixels
            || self.meadow_size_scale != previous.meadow_size_scale
            || self.meadow_density != previous.meadow_density
            || self.meadow_color_multiplier != previous.meadow_color_multiplier
        // Exhaust, wheel particles and bloom compose after reconstruction.
    }

    pub(crate) fn draws_meadow(self) -> bool {
        self.meadow_enabled && self.meadow_density > 0.0
    }
}

/// Shared source/aggregate endpoints, applied once per frame to all views.
/// Pool dimensions and root IDs remain unchanged when these ranges shrink.
pub(crate) fn meadow_ranges(source: [f32; 2], scale: f32) -> [f32; 4] {
    [source[0] * scale, source[1] * scale, 160.0 * scale, 180.0 * scale]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_options_validate_before_mutating_renderer_state() {
        let defaults = DebugRenderOptions::default();
        assert!(defaults.validate().is_ok());
        assert!(defaults.bloom_enabled);assert_eq!(defaults.bloom_strength,0.04);
        for bad in [f32::NAN,f32::INFINITY,-0.01,2.001] {
            assert!(DebugRenderOptions{bloom_strength:bad,..defaults}.validate().is_err());
        }
        assert_eq!(defaults.meadow_size_scale,1.0);
        assert_eq!(defaults.meadow_lod_pixels,1.0);
        assert_eq!(defaults.meadow_density,1.0);
        assert_eq!(defaults.meadow_color_multiplier,[1.0;3]);
        assert!(defaults.meadow_enabled && defaults.headlights_enabled
            && defaults.transparency_enabled && defaults.exhaust_enabled
            && defaults.wheel_particles_enabled);
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 0.249, 1.001] {
            assert!(DebugRenderOptions { meadow_distance_scale: scale, ..defaults }.validate().is_err());
        }
        for scale in [0.25, 0.5, 1.0] {
            assert!(DebugRenderOptions { meadow_distance_scale: scale, ..defaults }.validate().is_ok());
        }
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.499, 4.001] {
            assert!(DebugRenderOptions { meadow_lod_pixels: bad, ..defaults }.validate().is_err());
        }
        for pixels in [0.5, 1.0, 2.0, 4.0] {
            assert!(DebugRenderOptions { meadow_lod_pixels: pixels, ..defaults }.validate().is_ok());
        }
        for bad in [f32::NAN,f32::INFINITY,-0.1,2.001] {
            assert!(DebugRenderOptions{meadow_size_scale:bad,..defaults}.validate().is_err());
            assert!(DebugRenderOptions{meadow_color_multiplier:[1.0,bad,1.0],..defaults}.validate().is_err());
        }
        for bad in [f32::NAN,f32::INFINITY,-0.1,1.001] {
            assert!(DebugRenderOptions{meadow_density:bad,..defaults}.validate().is_err());
        }
        assert!(DebugRenderOptions{meadow_size_scale:0.499,..defaults}.validate().is_err());
        for size in [0.5,1.0,2.0] {
            for count in [0.0,0.5,1.0] {
                assert!(DebugRenderOptions{meadow_size_scale:size,meadow_density:count,meadow_color_multiplier:[0.0,1.0,2.0],..defaults}.validate().is_ok());
            }
        }
        assert!(!DebugRenderOptions{meadow_density:0.0,..defaults}.draws_meadow());
    }

    #[test]
    fn debug_temporal_changes_reset_both_disable_and_reenable() {
        let defaults = DebugRenderOptions::default();
        let variants = [
            DebugRenderOptions { meadow_enabled: false, ..defaults },
            DebugRenderOptions { headlights_enabled: false, ..defaults },
            DebugRenderOptions { transparency_enabled: false, ..defaults },
            DebugRenderOptions { meadow_distance_scale: 0.25, ..defaults },
            DebugRenderOptions { meadow_lod_pixels: 2.0, ..defaults },
            DebugRenderOptions { meadow_size_scale: 2.0, ..defaults },
            DebugRenderOptions { meadow_density: 0.0, ..defaults },
            DebugRenderOptions { meadow_color_multiplier: [0.0,0.5,2.0], ..defaults },
        ];
        assert!(!defaults.resets_history(defaults));
        for changed in variants {
            assert!(changed.resets_history(defaults));
            assert!(defaults.resets_history(changed));
        }
        let post_effects = DebugRenderOptions {
            exhaust_enabled: false, wheel_particles_enabled: false, bloom_enabled:false,bloom_strength:0.2, ..defaults
        };
        assert!(!post_effects.resets_history(defaults));
        assert!(!defaults.resets_history(post_effects));
    }

    #[test]
    fn debug_distance_ranges_keep_complementary_transitions_inside_allocations() {
        for source in [[60.0, 80.0], [80.0, 100.0]] {
            let allocated = meadow_ranges(source, 1.0);
            for scale in [0.25, 0.5, 0.75, 1.0] {
                let ranges = meadow_ranges(source, scale);
                assert!(ranges.windows(2).all(|p| p[0] < p[1]));
                for (a, b) in ranges.into_iter().zip(allocated) {
                    assert!(a <= b);
                    assert_eq!(a / scale, b);
                }
                // A matching normalized location has the same blend weight.
                let smoothstep = |a: f32, b: f32, x: f32| {
                    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let original = source[0] + t * (source[1] - source[0]);
                    assert_eq!(smoothstep(source[0], source[1], original),
                        smoothstep(ranges[0], ranges[1], original * scale));
                }
            }
        }
    }
}
