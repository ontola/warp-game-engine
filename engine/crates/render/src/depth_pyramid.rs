//! Current-frame conservative minimum reversed-depth pyramid. An empty/sky
//! pixel is zero and prevents occlusion. Odd edge texels fold into the last
//! parent; consumers map base indices by shift and clamp, never proportional UV.
use crate::Gpu;
pub(crate) const SOURCE: &str = r#"
@group(0) @binding(0) var source_depth:texture_depth_2d;
@group(0) @binding(1) var previous_mip:texture_2d<f32>;
@group(0) @binding(2) var output_mip:texture_storage_2d<r32float,write>;
@compute @workgroup_size(8,8) fn copy_depth(@builtin(global_invocation_id)id:vec3<u32>){
 let size=textureDimensions(output_mip);if any(id.xy>=size){return;}
 textureStore(output_mip,vec2<i32>(id.xy),vec4<f32>(textureLoad(source_depth,vec2<i32>(id.xy),0)));
}
@compute @workgroup_size(8,8) fn reduce_depth(@builtin(global_invocation_id)id:vec3<u32>){
 let size=textureDimensions(output_mip);if any(id.xy>=size){return;}
 let source_size=textureDimensions(previous_mip);let first=id.xy*2u;
 let end=select(first+2u,source_size,id.xy==size-1u);var depth=1.0;
 for(var y=first.y;y<end.y;y++){for(var x=first.x;x<end.x;x++){
  depth=min(depth,textureLoad(previous_mip,vec2<i32>(i32(x),i32(y)),0).x);
 }}
 textureStore(output_mip,vec2<i32>(id.xy),vec4<f32>(depth));
}
"#;
pub(crate) struct DepthPyramid {
    copy: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
}
pub(crate) struct PyramidTarget {
    pub size: (u32, u32),
    pub view: wgpu::TextureView,
    levels: Vec<wgpu::TextureView>,
    binds: Vec<wgpu::BindGroup>,
}
impl DepthPyramid {
    pub fn new(gpu: &Gpu) -> Self {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("conservative depth pyramid"),
                source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
            });
        let make = |entry| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: None,
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: gpu.pipeline_cache(),
                })
        };
        Self {
            copy: make("copy_depth"),
            reduce: make("reduce_depth"),
        }
    }
    pub fn target(&self, gpu: &Gpu, size: (u32, u32)) -> PyramidTarget {
        let count = 32 - size.0.max(size.1).leading_zeros();
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("current reversed minimum depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let levels: Vec<_> = (0..count)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let binds = (1..count as usize)
            .map(|i| {
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("depth pyramid reduce"),
                    layout: &self.reduce.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&levels[i - 1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&levels[i]),
                        },
                    ],
                })
            })
            .collect();
        PyramidTarget {
            size,
            view,
            levels,
            binds,
        }
    }
    pub fn encode(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &PyramidTarget,
        begin: Option<(&wgpu::QuerySet, u32)>,
    ) {
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth pyramid source"),
            layout: &self.copy.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&target.levels[0]),
                },
            ],
        });
        for i in 0..target.levels.len() {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("depth pyramid"),
                timestamp_writes: if i == 0 {
                    begin.map(|(query_set, index)| wgpu::ComputePassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(index),
                        end_of_pass_write_index: None,
                    })
                } else {
                    None
                },
            });
            pass.set_pipeline(if i == 0 { &self.copy } else { &self.reduce });
            pass.set_bind_group(0, if i == 0 { &bind } else { &target.binds[i - 1] }, &[]);
            pass.dispatch_workgroups(
                (target.size.0 >> i).max(1).div_ceil(8),
                (target.size.1 >> i).max(1).div_ceil(8),
                1,
            );
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn folded_odd_pyramid_covers_every_source_pixel() {
        for width in 1usize..130 {
            let mut cells: Vec<Vec<usize>> = (0..width).map(|x| vec![x]).collect();
            let mut level = 0;
            while cells.len() > 1 {
                let n = (cells.len() / 2).max(1);
                let next: Vec<Vec<usize>> = (0..n)
                    .map(|x| {
                        cells[x * 2..if x + 1 == n { cells.len() } else { x * 2 + 2 }]
                            .iter()
                            .flatten()
                            .copied()
                            .collect()
                    })
                    .collect();
                level += 1;
                for source in 0..width {
                    assert!(next[(source >> level).min(n - 1)].contains(&source));
                }
                cells = next;
            }
        }
    }
}
