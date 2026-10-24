//! Explicit output reconstruction policy. Native is the compatibility default.
//! A requested DLSS mode is never permission to silently choose another path.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpscaleMode {
    #[default]
    Native,
    DlssQuality,
    DlssBalanced,
    DlssPerformance,
}

impl UpscaleMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::DlssQuality => "dlss-quality",
            Self::DlssBalanced => "dlss-balanced",
            Self::DlssPerformance => "dlss-performance",
        }
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
            _ => Err(format!("unsupported upscale mode {value:?}; expected native, dlss-quality, dlss-balanced or dlss-performance")),
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
        for mode in [UpscaleMode::Native, UpscaleMode::DlssQuality, UpscaleMode::DlssBalanced, UpscaleMode::DlssPerformance] {
            assert_eq!(mode.as_str().parse::<UpscaleMode>().unwrap(), mode);
        }
        for invalid in ["", "auto", "dlss", "fsr", "quality", "DLSS-QUALITY", " native"] {
            assert!(invalid.parse::<UpscaleMode>().is_err(), "{invalid}");
        }
        assert_eq!(UpscaleStatus::default().mode, UpscaleMode::Native);
        assert!(UpscaleStatus::default().views.is_empty());
    }
}
