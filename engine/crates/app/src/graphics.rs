//! The player's graphics level: one setting for every Warp game, separate
//! from device facts such as screen size or GPU (graphics-tiers.md).
//!
//! - [`Level::High`] (3, "Ultra"): the full renderer.
//! - [`Level::Medium`] (2, "Standard"): the phone preset of the same renderer.
//! - [`Level::Low`] (1, "Mobile"): the forward renderer, and games drop
//!   their costliest content.
//!
//! Chosen with `WARP_GRAPHICS=high|medium|low` (or `1|2|3`) natively, and
//! `?graphics=` in the browser (the older `?quality=full|mobile|low` reads
//! too). [`chosen`] is `None` when nothing chose: the host then picks from the
//! device (the browser's phone check, hardware fit) and a game from its own
//! defaults. A game maps the level onto its own content switches; it should
//! not grow its own parameter for the same thing.

/// See the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Low = 1, Medium = 2, High = 3 }

impl Level {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "3" | "high" | "ultra" | "full" => Some(Self::High),
            "2" | "medium" | "standard" => Some(Self::Medium),
            "1" | "low" | "mobile" | "minimum" => Some(Self::Low),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self { Self::High => "high", Self::Medium => "medium", Self::Low => "low" }
    }
}

/// The level someone chose for this run, if any (see the module docs).
pub fn chosen() -> Option<Level> {
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    {
        let q = |k| crate::web::query_param(k).and_then(|v| Level::parse(&v));
        if let Some(l) = q("graphics").or_else(|| q("quality")) { return Some(l); }
    }
    std::env::var("WARP_GRAPHICS").ok().and_then(|v| Level::parse(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_names_and_numbers() {
        assert_eq!(Level::parse("Ultra"), Some(Level::High));
        assert_eq!(Level::parse("2"), Some(Level::Medium));
        assert_eq!(Level::parse("mobile"), Some(Level::Low));
        assert_eq!(Level::parse("fast"), None);
    }
}
