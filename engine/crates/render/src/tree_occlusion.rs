//! Current-frame conservative tree occlusion. Only real, alpha-tested source
//! depth is an occluder; camera visibility never removes shadow/GI casters.
use crate::{depth_pyramid::{DepthPyramid, PyramidTarget}, Gpu};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

pub const SPLIT_METRES: f32 = 32.0;
pub(crate) const COUNTER_WORDS: usize = 32;
pub(crate) const COUNTER_BYTES: u64 = COUNTER_WORDS as u64 * 4;

#[derive(Clone, Debug)]
pub struct Diagnostics {
    pub enabled: bool,
    /// False for the phase-order diagnostic: both draws remain, but Hi-Z
    /// cannot reject geometry. Also false when two-phase rendering is off.
    pub rejection_enabled: bool,
    pub disabled_reason: Option<&'static str>,
    pub frame_id: u64,
    pub render_sequence: u64,
    pub view_index: usize,
    pub split_m: f32,
    pub phase1_clusters: u32,
    pub phase2_clusters: u32,
    pub instances_tested: u32,
    pub instances_rejected: u32,
    pub clusters_tested: u32,
    pub clusters_rejected: u32,
    /// Spatial broadphase work; populated by the renderer's matching resources.
    pub instance_groups: Option<crate::cull_groups::Diagnostics>,
    pub cluster_traversal: Option<crate::cluster_traversal::Diagnostics>,
}

pub(crate) struct Resources {
    pub enabled: bool,
    pub rejection_enabled: bool,
    pub disabled_reason: Option<&'static str>,
    pub reference: wgpu::Buffer,
    pub phases: [wgpu::Buffer; 2],
    pub empty: wgpu::TextureView,
    pub pyramid: Option<DepthPyramid>,
    pub targets: Mutex<Vec<Option<PyramidTarget>>>,
}
impl Resources {
    pub fn new(gpu: &Gpu, bucketed: bool) -> Self {
        let requested = std::env::var("KERN_TREE_OCCLUSION").as_deref() == Ok("1");
        let enabled = requested && bucketed;
        let rejection_enabled = enabled
            && std::env::var("KERN_TREE_OCCLUSION_REJECT").as_deref() != Ok("0");
        let disabled_reason = if requested && !bucketed { Some("cluster_buckets_required") }
            else if !requested { Some("not_requested") } else { None };
        let control = |phase: f32| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable tree occlusion phase"),
            contents: bytemuck::cast_slice(&[phase, SPLIT_METRES, if rejection_enabled {1.0} else {0.0}, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let empty = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("disabled tree occlusion depth"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        if requested { eprintln!("Tree occlusion: enabled={enabled}, rejection_enabled={rejection_enabled}, split={SPLIT_METRES}m, disabled_reason={disabled_reason:?}; current-frame real alpha depth, camera only"); }
        Self { enabled, rejection_enabled, disabled_reason, reference: control(0.0), phases: [control(1.0), control(2.0)],
            empty, pyramid: enabled.then(||DepthPyramid::new(gpu)), targets: Mutex::new(Vec::new()) }
    }
    pub fn decode(&self, words: &[u32], frame_id:u64, render_sequence:u64, view_index:usize) -> Result<Diagnostics,String> {
        if words.len() != COUNTER_WORDS { return Err("invalid tree occlusion counter size".into()); }
        if self.enabled && (words[13].checked_add(words[14]) != Some(words[0])
            || words[16] > words[15] || words[18] > words[17]) {
            return Err("inconsistent tree occlusion submitted/rejected counters".into());
        }
        if !self.rejection_enabled && (words[16] != 0 || words[18] != 0) {
            return Err("tree occlusion rejection counters are nonzero while rejection is disabled".into());
        }
        Ok(Diagnostics { enabled:self.enabled, rejection_enabled:self.rejection_enabled, disabled_reason:self.disabled_reason, frame_id, render_sequence, view_index,
            split_m:SPLIT_METRES, phase1_clusters:if self.enabled {words[13]} else {words[0]},
            phase2_clusters:words[14], instances_tested:words[15], instances_rejected:words[16],
            clusters_tested:words[17], clusters_rejected:words[18], instance_groups:None,cluster_traversal:None })
    }
}

#[cfg(test)]
mod tests {
    use glam::{Mat4,Vec3};
    // Independent clip projection oracle: near-plane or W
    // crossings must never become an occluded box. Mirrors only the geometric
    // contract, not shader sampling or its pixel-coordinate arithmetic.
    fn closest(vp:Mat4, center:Vec3, radius:f32)->Option<f64> {
        let mut nearest=0.0f64;
        for corner in 0..8 { let p=center+Vec3::new(if corner&1==0 {-radius}else{radius},if corner&2==0 {-radius}else{radius},if corner&4==0 {-radius}else{radius});
            let q=vp* p.extend(1.0);
            if !q.is_finite() || q.w<=1e-5 || q.z<0.0 || q.z>=q.w-1e-6 {return None;}
            nearest=nearest.max(q.z as f64/q.w as f64);
        } Some(nearest)
    }
    #[test]
    fn phase_partition_has_no_holes_even_at_wind_envelope_boundary() {
        for is_tree in [false,true] { for distance in [0.0f32,31.0,32.0,33.0,100.0] {for radius in [0.0,1.0,12.0] {for wind in [0.0,0.4,3.0] {
            let first=!is_tree || distance-(radius+wind)<=super::SPLIT_METRES;
            let second=is_tree && distance-(radius+wind)>super::SPLIT_METRES;
            assert_ne!(first,second);
        }}}}
    }
    #[test]
    fn near_plane_and_wind_bounds_fail_open_and_alpha_holes_prevent_rejection() {
        let vp=Mat4::perspective_infinite_reverse_rh(1.1,1.7,0.1);
        assert!(closest(vp,Vec3::new(0.,0.,-0.12),0.04).is_none());
        assert!(closest(vp,Vec3::ZERO,0.5).is_none());
        let center=Vec3::new(0.,0.,-10.);
        let rest=closest(vp,center,1.).unwrap();let wind=closest(vp,center,3.).unwrap();
        assert!(wind>rest, "nearer wind envelope must make rejection harder");
        let covered=[0.04f64;4];let gap=[0.04,0.04,0.0,0.04];
        assert!(covered.into_iter().fold(1.0,f64::min)>wind+2e-5);
        assert!(gap.into_iter().fold(1.0,f64::min)<=rest+2e-5);
        // An occluder behind any part of the bound is insufficient.
        assert!(!(rest+1e-5>rest+2e-5));
    }
    #[test]
    fn appended_phase_draws_preserve_pixel_ids_and_accumulate_submitted_work() {
        // Two previously rasterized pixels refer to slots0 and2. Bucket
        // scatter in phase2 may reorder draws, never these retained records.
        let mut visible=vec![(7u32,3u32,8u32),(9,4,64),(8,5,19)];
        let old_ids=[(0u32<<7)|5,(2u32<<7)|12];
        let before=old_ids.map(|id| (visible[(id>>7)as usize].0,visible[(id>>7)as usize].1,id&127));
        let prefix=visible.len();
        visible.extend([(11,8,128),(15,6,6),(16,10,31)]);
        let bucket=|n| if n<=8 {0usize}else if n<=32 {1}else if n<=64 {2}else {3};
        let mut phase_draws=[Vec::new(),Vec::new(),Vec::new(),Vec::new()];
        for slot in prefix..visible.len() {phase_draws[bucket(visible[slot].2)].push(slot);}
        let mut remaining=phase_draws.iter().flatten().copied().collect::<Vec<_>>();remaining.sort_unstable();
        assert_eq!(remaining,vec![3,4,5],"phase2 excludes every phase1 prefix record");
        assert_eq!(before,old_ids.map(|id| (visible[(id>>7)as usize].0,visible[(id>>7)as usize].1,id&127)));
        let submitted=|entries:&[(u32,u32,u32)]|entries.iter().map(|v|[24u32,96,192,384][bucket(v.2)]).sum::<u32>();
        assert_eq!(submitted(&visible[..prefix])+submitted(&visible[prefix..]),submitted(&visible));
        assert_eq!(visible[..prefix].len()+remaining.len(),visible.len());
    }
    #[test]
    fn cull_counter_abi_has_room_for_both_phases_without_extra_storage_binding() {
        let source=crate::frame::cull_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let (_,counter)=module.global_variables.iter().find(|(_,g)|g.name.as_deref()==Some("counter")).unwrap();
        let naga::TypeInner::Array {size:naga::ArraySize::Constant(size),stride,..}=module.types[counter.ty].inner else {panic!("fixed counter array required")};
        assert_eq!(size.get() as usize,super::COUNTER_WORDS);
        assert_eq!(size.get() as u64*stride as u64,super::COUNTER_BYTES);
        assert_eq!(counter.binding.as_ref().unwrap().binding,2);
    }
}
