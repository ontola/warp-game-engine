//! Vegetation budgets per machine. The GPU forest draws every tree on every
//! GPU; what differs is how much detail the camera cut may spend. A tier
//! scales the pixel error target and caps the needle (source cluster),
//! voxel splat and branch slot budgets the refine pass reserves against
//! (foliage_refine.wgsl), so a weaker GPU gets coarser voxels and needles
//! nearer by instead of a slow frame. Nothing is switched off.
//!
//! The tier is process-wide: it starts from `KERN_VEGETATION_QUALITY`
//! (low|medium|high) or the adapter kind, survives scene rebuilds, and
//! `app` steps it down when the adaptive quality ladder runs out
//! (`Game::content_too_heavy`).
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier { Low = 0, Medium = 1, High = 2 }

/// What a tier allows on top of the scene's own budgets (never more than
/// the scene allocated).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budgets {
    /// Multiplier on the scene's pixel error target.
    pub error_scale: f32,
    /// Most needle (source) clusters the camera cut reserves.
    pub source_clusters: u32,
    /// Most voxel splats per consumer class (camera, shadows).
    pub splats: u32,
    /// Share of the scene's branch slots the cut may use.
    pub slot_share: f32,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Low, Tier::Medium, Tier::High];
    /// High leaves the scene's own budgets alone (terrain_fly: 2M source
    /// clusters, every slot, tuned on a desktop GPU at 1440p). The lower
    /// tiers save on pixel error, so voxels get coarser everywhere, and
    /// keep the needle budget: cutting that one turns the trees next to the
    /// camera into whole-plant proxies (RTX desktop, 1440p, 18 November:
    /// half the needle budget saved 2.5 ms and hung balls in near crowns).
    pub fn budgets(self) -> Budgets {
        match self {
            Tier::High => Budgets { error_scale: 1.0, source_clusters: 2 << 20, splats: 16 << 20, slot_share: 1.0 },
            Tier::Medium => Budgets { error_scale: 1.5, source_clusters: 2 << 20, splats: 8 << 20, slot_share: 1.0 },
            Tier::Low => Budgets { error_scale: 2.5, source_clusters: 1 << 20, splats: 4 << 20, slot_share: 0.5 },
        }
    }
    pub fn lower(self) -> Option<Tier> { (self != Tier::Low).then(|| Tier::ALL[self as usize - 1]) }
    pub fn higher(self) -> Option<Tier> { (self != Tier::High).then(|| Tier::ALL[self as usize + 1]) }
    pub fn name(self) -> &'static str { ["low", "medium", "high"][self as usize] }
    pub fn parse(name: &str) -> Option<Tier> { Tier::ALL.into_iter().find(|t| t.name() == name) }
    /// Where a machine starts: a discrete GPU high, an integrated one
    /// medium, unknown adapters (a browser does not say) low. A software
    /// rasteriser is a test rig, not a player's machine: it stays high so
    /// its images compare with a desktop GPU's. Only a starting point: the
    /// app lowers it when frames stay slow, and a game or player can set it.
    pub fn for_adapter(kind: wgpu::DeviceType) -> Tier {
        match kind {
            wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::Cpu => Tier::High,
            wgpu::DeviceType::IntegratedGpu => Tier::Medium,
            _ => Tier::Low,
        }
    }
}

const UNSET: u8 = u8::MAX;
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);

/// The tier in use; the first call decides it for this GPU.
pub fn tier(gpu: &crate::Gpu) -> Tier {
    let stored = CURRENT.load(Ordering::Relaxed);
    if stored != UNSET { return Tier::ALL[stored as usize]; }
    let tier = crate::switches::VEGETATION_QUALITY.get().and_then(|v| Tier::parse(&v)).unwrap_or_else(|| Tier::for_adapter(gpu.info.device_type));
    // Another thread may have set one meanwhile: keep that.
    match CURRENT.compare_exchange(UNSET, tier as u8, Ordering::Relaxed, Ordering::Relaxed) { Ok(_) => tier, Err(other) => Tier::ALL[other as usize] }
}
/// The tier if one was chosen or set already.
pub fn current() -> Option<Tier> { let t = CURRENT.load(Ordering::Relaxed); (t != UNSET).then(|| Tier::ALL[t as usize]) }
/// Takes effect on the next frame of every renderer.
pub fn set(tier: Tier) { CURRENT.store(tier as u8, Ordering::Relaxed); }
/// One tier down, if there is one: what `app` does when even the lowest
/// resolution rung stays slow. Returns the new tier.
pub fn step_down() -> Option<Tier> {
    let lower = current()?.lower()?;
    set(lower);
    Some(lower)
}

/// The budgets in force: the tier's, or `KERN_VEGETATION_BUDGETS` (error
/// scale, source clusters, splats, slot share) to try other steps without a
/// rebuild.
pub fn budgets(gpu: &crate::Gpu) -> Budgets {
    static OVERRIDE: std::sync::OnceLock<Option<Budgets>> = std::sync::OnceLock::new();
    OVERRIDE.get_or_init(|| crate::switches::VEGETATION_BUDGETS.get().and_then(|v| parse_budgets(&v))).unwrap_or_else(|| tier(gpu).budgets())
}
fn parse_budgets(text: &str) -> Option<Budgets> {
    let mut parts = text.split(',').map(str::trim);
    let b = Budgets { error_scale: parts.next()?.parse().ok()?, source_clusters: parts.next()?.parse().ok()?, splats: parts.next()?.parse().ok()?, slot_share: parts.next()?.parse().ok()? };
    (parts.next().is_none() && b.error_scale.is_finite() && b.error_scale > 0.0 && b.slot_share > 0.0 && b.slot_share <= 1.0).then_some(b)
}

/// The scene's budgets under a tier: (error pixels, branch slots, splats, source clusters).
pub fn apply(b: Budgets, error: f32, slots: u32, splats: u32, source_clusters: u32) -> (f32, u32, u32, u32) {
    let active = ((slots as f32 * b.slot_share) as u32).clamp(slots.min(1024), slots);
    (error * b.error_scale, active, splats.min(b.splats).max(1), source_clusters.min(b.source_clusters).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiers_only_tighten_the_scene_budgets_and_order_by_cost() {
        for pair in Tier::ALL.windows(2) {
            let (low, high) = (pair[0].budgets(), pair[1].budgets());
            assert!(low.error_scale >= high.error_scale && low.source_clusters <= high.source_clusters);
            assert!(low.splats <= high.splats && low.slot_share <= high.slot_share);
            assert_eq!(pair[1].lower(), Some(pair[0])); assert_eq!(pair[0].higher(), Some(pair[1]));
        }
        for tier in Tier::ALL {
            assert_eq!(Tier::parse(tier.name()), Some(tier));
            let (error, slots, splats, source) = apply(tier.budgets(), 1.0, 65_536, 8 << 20, 2 << 20);
            assert!(error >= 1.0 && (1024..=65_536).contains(&slots) && splats <= 8 << 20 && source <= 2 << 20);
        }
        // A small scene keeps its few slots at every tier.
        assert_eq!(apply(Tier::Low.budgets(), 1.0, 8, 100, 100).1, 8);
        assert_eq!(apply(Tier::High.budgets(), 1.0, 65_536, 8 << 20, 2 << 20), (1.0, 65_536, 8 << 20, 2 << 20));
        assert_eq!(parse_budgets("2, 1000, 500,0.5"), Some(Budgets { error_scale: 2.0, source_clusters: 1000, splats: 500, slot_share: 0.5 }));
        assert_eq!(parse_budgets("2,1000"), None); assert_eq!(parse_budgets("0,1,1,1"), None);
        assert_eq!(Tier::Low.lower(), None); assert_eq!(Tier::High.higher(), None);
    }
}
