//! Explicit output reconstruction policy. Native is the compatibility default.
//! A requested DLSS mode is never permission to silently choose another path.
//! The `temporal-*` modes are Warp's own upscaler (`temporal_upscale.rs`):
//! plain WGSL compute with no storage buffers, so it runs on every GPU and in
//! the browser, at the same render scales as the DLSS modes.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpscaleMode {
    #[default]
    Native,
    DlssQuality,
    DlssBalanced,
    DlssPerformance,
    TemporalQuality,
    TemporalBalanced,
    TemporalPerformance,
}

impl UpscaleMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::DlssQuality => "dlss-quality",
            Self::DlssBalanced => "dlss-balanced",
            Self::DlssPerformance => "dlss-performance",
            Self::TemporalQuality => "temporal-quality",
            Self::TemporalBalanced => "temporal-balanced",
            Self::TemporalPerformance => "temporal-performance",
        }
    }

    pub const ALL: [Self; 7] = [Self::Native, Self::DlssQuality, Self::DlssBalanced, Self::DlssPerformance,
        Self::TemporalQuality, Self::TemporalBalanced, Self::TemporalPerformance];

    pub const fn is_dlss(self) -> bool {
        matches!(self, Self::DlssQuality | Self::DlssBalanced | Self::DlssPerformance)
    }

    /// Render scale per axis of the portable upscaler (the same ratios as
    /// DLSS's modes), or None for native and DLSS.
    pub const fn portable_scale(self) -> Option<f32> {
        match self {
            Self::TemporalQuality => Some(2.0 / 3.0),
            Self::TemporalBalanced => Some(0.58),
            Self::TemporalPerformance => Some(0.5),
            _ => None,
        }
    }

    /// Render size of the portable upscaler for one output view.
    pub fn portable_size(self, output: (u32, u32)) -> Option<(u32, u32)> {
        let scale = self.portable_scale()?;
        let side = |v: u32| ((v as f32 * scale).round() as u32).clamp(1, v.max(1));
        Some((side(output.0), side(output.1)))
    }
}

impl std::str::FromStr for UpscaleMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "native" => Ok(Self::Native),
            "dlss-quality" => Ok(Self::DlssQuality),
            "dlss-balanced" => Ok(Self::DlssBalanced),
            "dlss-performance" => Ok(Self::DlssPerformance),
            "temporal-quality" => Ok(Self::TemporalQuality),
            "temporal-balanced" => Ok(Self::TemporalBalanced),
            "temporal-performance" => Ok(Self::TemporalPerformance),
            _ => Err(format!("unsupported upscale mode {value:?}; expected native, dlss-quality, dlss-balanced, dlss-performance, temporal-quality, temporal-balanced or temporal-performance")),
        }
    }
}

/// Actual successfully evaluated view dimensions, not guessed ratios.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpscaleViewStatus {
    pub input: [u32; 2],
    pub output: [u32; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpscaleStatus {
    pub mode: UpscaleMode,
    pub views: Vec<UpscaleViewStatus>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_modes_roundtrip_without_fallback_aliases() {
        for mode in UpscaleMode::ALL {
            assert_eq!(mode.as_str().parse::<UpscaleMode>().unwrap(), mode);
            assert_eq!(mode.is_dlss(), mode.as_str().starts_with("dlss"));
            assert_eq!(mode.portable_scale().is_some(), mode.as_str().starts_with("temporal"));
        }
        for invalid in ["", "auto", "dlss", "fsr", "quality", "temporal", "DLSS-QUALITY", " native"] {
            assert!(invalid.parse::<UpscaleMode>().is_err(), "{invalid}");
        }
        assert_eq!(UpscaleStatus::default().mode, UpscaleMode::Native);
        assert!(UpscaleStatus::default().views.is_empty());
    }
    #[test]
    fn portable_sizes_round_and_never_reach_zero() {
        assert_eq!(UpscaleMode::TemporalQuality.portable_size((1920, 1080)), Some((1280, 720)));
        assert_eq!(UpscaleMode::TemporalPerformance.portable_size((3840, 2160)), Some((1920, 1080)));
        assert_eq!(UpscaleMode::TemporalPerformance.portable_size((1, 1)), Some((1, 1)));
        assert_eq!(UpscaleMode::DlssQuality.portable_size((1920, 1080)), None);
        assert_eq!(UpscaleMode::Native.portable_size((1920, 1080)), None);
    }
}
