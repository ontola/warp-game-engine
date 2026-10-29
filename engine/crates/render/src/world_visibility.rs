//! Runtime bridge for shared, camera-independent terrain visibility.
//! Kept opt-in until scene captures and steady GPU cost pass acceptance.
use crate::{Gpu, scene::Scene, terrain_trace::{Coverage, GpuHierarchy}};
use wgpu::util::DeviceExt;

pub(crate) struct Resources {
    pub buffer: wgpu::Buffer,
    pub uniform: wgpu::Buffer,
    pub enabled: bool,
    pub bytes: u64,
    pub build_cpu_ms: f64,
}

impl Resources {
    pub fn new(gpu: &Gpu, scene: &Scene, height_view: &wgpu::TextureView) -> Result<Self,String> {
        let enabled=match std::env::var("KERN_WORLD_TERRAIN_VISIBILITY").as_deref() {Ok("1")=>true,Ok("0")=>false,_=>scene.world_terrain_visibility};
        let start=std::time::Instant::now();
        let budget=std::env::var("KERN_TERRAIN_NODE_BUDGET").ok().and_then(|v|v.parse::<u32>().ok())
            .unwrap_or(crate::terrain_trace::DEFAULT_NODE_BUDGET).min(crate::terrain_trace::MAX_NODE_BUDGET);
        let buffer=if enabled {
            let coverage=if scene.terrain_visibility_complete {Coverage::CompleteLoadedTerrain}else{Coverage::UnknownOutside};
            let hierarchy=GpuHierarchy::new(gpu,&scene.heightfield,coverage,Some(height_view))?;
            let mut encoder=gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor{label:Some("world terrain hierarchy once")});
            hierarchy.encode_build(&mut encoder);
            gpu.queue.submit([encoder.finish()]);
            hierarchy.buffer.clone()
        } else {
            // Header304 + one vec2 node, rounded to the struct's16-byte
            // alignment. Disabled shading never reads it.
            gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("disabled world visibility"),size:320,
                usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false})
        };
        let params=[u32::from(enabled) as f32,budget as f32,scene.terrain_visual_offset_m.max(0.0),0.0];
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("world visibility policy"),
            contents:bytemuck::cast_slice(&params),usage:wgpu::BufferUsages::UNIFORM});
        let bytes=buffer.size()+uniform.size();
        Ok(Self{buffer,uniform,enabled,bytes,build_cpu_ms:start.elapsed().as_secs_f64()*1000.0})
    }
}

#[cfg(test)]
mod tests {
    use crate::{scene::Heightfield, terrain_trace::{Coverage, Hierarchy, Status}};
    use glam::Vec3;

    /// Independent half-space oracle for the shader's representation contract.
    /// Actual shader execution is separately checked by GPU captures; these
    /// cases establish which origins are physically allowed before tracing.
    fn origin_for_flat_ground(receiver_y:f32, permitted_error:f32)->Vec3 {
        let numeric_offset=0.025;
        let offset_y=receiver_y+numeric_offset;
        let inside_declared_shell=offset_y>=-permitted_error-numeric_offset
            && offset_y<numeric_offset;
        Vec3::new(1.0,if inside_declared_shell {numeric_offset}else{offset_y},1.0)
    }
    fn flat()->Hierarchy {
        Hierarchy::with_coverage(&Heightfield {
            origin:[0.0,0.0],cell:1.0,w:5,h:5,heights:vec![0.0;25],
        },Coverage::CompleteLoadedTerrain).unwrap()
    }
    #[test]
    fn metre_buried_object_stays_blocked_while_declared_road_shell_can_escape() {
        let terrain=flat();
        for sun in [Vec3::Y,Vec3::new(1.0,2f32.to_radians().tan(),0.0).normalize()] {
            // Exact solid half-space y<=0: a receiver still below zero cannot
            // see the sun, regardless of upward or grazing ray direction.
            let object=origin_for_flat_ground(-1.0,0.0);
            assert!(object.y<0.0);
            assert_eq!(terrain.trace_upward(object,sun,0.01,256).status,Status::Hit);
            // Only the terrain/road representation is allowed to substitute
            // its declared 1.10 m shell for the base heightfield surface.
            let road=origin_for_flat_ground(-1.0,1.10);
            assert!(road.y>0.0);
            assert_eq!(terrain.trace_upward(road,sun,0.01,256).status,Status::AboveTerrainBounds);
        }
    }
    #[test]
    fn terrain_escape_is_bounded_and_numeric_surface_offset_is_not_a_metre_lift() {
        let terrain=flat();
        let too_deep=origin_for_flat_ground(-1.20,1.10);
        assert!(too_deep.y<0.0);
        assert_eq!(terrain.trace_upward(too_deep,Vec3::Y,0.01,256).status,Status::Hit);
        let surface=origin_for_flat_ground(0.0,0.0);
        assert!((surface.y-0.025).abs()<1e-6);
        assert_eq!(terrain.trace_upward(surface,Vec3::Y,0.01,256).status,Status::AboveTerrainBounds);
        let underground=origin_for_flat_ground(-0.10,0.0);
        assert!(underground.y<0.0);
        assert_eq!(terrain.trace_upward(underground,Vec3::Y,0.01,256).status,Status::Hit);
    }
}
