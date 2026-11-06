//! Small owner-local GPU caches. Keys retain actual wgpu handles, whose Eq
//! compares resource identity, so a same-size replacement cannot reuse a bind.
//! Buffer contents deliberately do not participate: queue writes keep the bind.
use crate::Gpu;

/// Counters for the currently retained resource caches. Misses create a GPU
/// view or bind group; hits only clone its reference-counted handle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
}
impl CacheStats {
    pub(crate) fn add(&mut self, other: Self) {
        self.hits += other.hits;
        self.misses += other.misses;
        self.entries += other.entries;
    }
}

struct Bounded<T, const N: usize> {
    entries: Vec<T>,
    next: usize,
    stats: CacheStats,
}
impl<T, const N: usize> Default for Bounded<T, N> {
    fn default() -> Self { Self { entries: Vec::new(), next: 0, stats: CacheStats::default() } }
}
impl<T, const N: usize> Bounded<T, N> {
    fn get(&mut self, matches: impl Fn(&T) -> bool, make: impl FnOnce() -> T) -> &T {
        assert!(N > 0);
        let index = if let Some(index) = self.entries.iter().position(matches) {
            self.stats.hits += 1;
            index
        } else {
            self.stats.misses += 1;
            let entry = make();
            if self.entries.len() < N {
                self.entries.push(entry);
                self.entries.len() - 1
            } else {
                let index = self.next;
                self.entries[index] = entry;
                self.next = (index + 1) % N;
                index
            }
        };
        &self.entries[index]
    }
    fn stats(&self) -> CacheStats { CacheStats { entries: self.entries.len(), ..self.stats } }
}

pub(crate) struct TextureViews<const N: usize>(Bounded<(wgpu::Texture, wgpu::TextureView), N>);
impl<const N: usize> Default for TextureViews<N> { fn default() -> Self { Self(Bounded::default()) } }
impl<const N: usize> TextureViews<N> {
    /// Default whole-texture view only. Alternate formats/mips remain explicit.
    pub fn get(&mut self, texture: &wgpu::Texture) -> wgpu::TextureView {
        self.0.get(|(key, _)| key == texture,
            || (texture.clone(), texture.create_view(&Default::default()))).1.clone()
    }
    /// Whole-texture view as a 2D array (one layer is fine).
    pub fn get_array(&mut self, texture: &wgpu::Texture) -> wgpu::TextureView {
        self.0.get(|(key, _)| key == texture,
            || (texture.clone(), texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }))).1.clone()
    }
    pub fn stats(&self) -> CacheStats { self.0.stats() }
}

enum Resource {
    Buffer(wgpu::Buffer, u64, Option<wgpu::BufferSize>),
    Texture(wgpu::TextureView),
    Sampler(wgpu::Sampler),
    // Unsupported kinds are valid to bind but deliberately never cache-hit.
    Other,
}
impl Resource {
    fn new(resource: &wgpu::BindingResource<'_>) -> Self {
        match resource {
            wgpu::BindingResource::Buffer(b) => Self::Buffer(b.buffer.clone(), b.offset, b.size),
            wgpu::BindingResource::TextureView(v) => Self::Texture((*v).clone()),
            wgpu::BindingResource::Sampler(s) => Self::Sampler((*s).clone()),
            _ => Self::Other,
        }
    }
    fn matches(&self, resource: &wgpu::BindingResource<'_>) -> bool {
        match (self, resource) {
            (Self::Buffer(buffer, offset, size), wgpu::BindingResource::Buffer(b)) =>
                buffer == b.buffer && *offset == b.offset && *size == b.size,
            (Self::Texture(view), wgpu::BindingResource::TextureView(v)) => view == *v,
            (Self::Sampler(sampler), wgpu::BindingResource::Sampler(s)) => sampler == *s,
            _ => false,
        }
    }
}
struct CachedBind {
    layout: wgpu::BindGroupLayout,
    resources: Vec<(u32, Resource)>,
    bind: wgpu::BindGroup,
}
pub(crate) struct BindGroups<const N: usize>(Bounded<CachedBind, N>);
impl<const N: usize> Default for BindGroups<N> { fn default() -> Self { Self(Bounded::default()) } }
impl<const N: usize> BindGroups<N> {
    pub fn get(&mut self, gpu: &Gpu, layout: &wgpu::BindGroupLayout, label: &str,
        bindings: &[(u32, wgpu::BindingResource<'_>)]) -> wgpu::BindGroup {
        self.resources(gpu, layout, label, bindings.iter().map(|(b, r)| (*b, r.clone())))
    }
    pub fn entries(&mut self, gpu: &Gpu, layout: &wgpu::BindGroupLayout, label: &str,
        entries: &[wgpu::BindGroupEntry<'_>]) -> wgpu::BindGroup {
        self.resources(gpu, layout, label, entries.iter().map(|e| (e.binding, e.resource.clone())))
    }
    fn resources<'a>(&mut self, gpu: &Gpu, layout: &wgpu::BindGroupLayout, label: &str,
        bindings: impl ExactSizeIterator<Item = (u32, wgpu::BindingResource<'a>)> + Clone) -> wgpu::BindGroup {
        self.0.get(|cached| cached.layout == *layout && cached.resources.len() == bindings.len()
            && cached.resources.iter().zip(bindings.clone()).all(|((binding, resource), (b, r))|
                *binding == b && resource.matches(&r)), || {
            let entries: Vec<_> = bindings.clone().map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource }).collect();
            CachedBind {
                layout: layout.clone(),
                resources: bindings.clone().map(|(b, r)| (b, Resource::new(&r))).collect(),
                bind: gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries: &entries }),
            }
        }).bind.clone()
    }
    pub fn stats(&self) -> CacheStats { self.0.stats() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternating_history_reuses_both_generations_and_replacements_are_bounded() {
        let mut cache = Bounded::<(u64, u32), 2>::default();
        let mut creations = 0;
        for identity in [10, 11, 10, 11, 10, 11] {
            cache.get(|entry| entry.0 == identity, || { creations += 1; (identity, creations) });
        }
        assert_eq!(creations, 2);
        // Resize/reset creates new identities even if dimensions later match.
        for identity in 20..100 {
            assert_eq!(cache.get(|entry| entry.0 == identity, || (identity, 0)).0, identity);
            assert_eq!(cache.stats().entries, 2);
        }
        assert!(cache.entries.iter().all(|entry| entry.0 >= 98));
        assert_eq!(cache.stats().hits, 4);
        assert_eq!(cache.stats().misses, 82);
    }
    #[test]
    fn replacement_releases_the_previous_owned_resource() {
        use std::sync::Arc;
        let old = Arc::new(1);
        let weak = Arc::downgrade(&old);
        let mut cache = Bounded::<Arc<u32>, 1>::default();
        cache.get(|_| false, || old);
        assert!(weak.upgrade().is_some());
        cache.get(|_| false, || Arc::new(2));
        assert!(weak.upgrade().is_none());
    }
}
