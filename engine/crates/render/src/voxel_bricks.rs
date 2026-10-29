//! Conservative, pose-local visibility bounds for occupied 4x4x4 bricks.
//! Geometry is shared per packed level. Instances contain only a tree/brick
//! reference: wind continues to use the existing owner transform.
use crate::foliage_volume_data::{GpuForest, cell_index};
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Brick {
    pub min: [f32; 4],
    pub max: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Reference {
    pub tree: u32,
    pub level: u32,
    pub brick: u32,
    pub padding: u32,
}
pub(crate) struct Catalog {
    pub bricks: Vec<Brick>,
    pub references: Vec<Reference>,
    pub spans: Vec<[u32;2]>,
}
impl Catalog {
    pub fn words(&self) -> Vec<u32> {
        let mut words=vec![self.references.len() as u32,4+self.bricks.len() as u32*8,
            4+self.bricks.len() as u32*8+self.references.len() as u32*4,0];
        words.extend_from_slice(bytemuck::cast_slice(&self.bricks));
        words.extend_from_slice(bytemuck::cast_slice(&self.references));
        words.extend_from_slice(bytemuck::cast_slice(&self.spans));
        words
    }
}

/// Hard admission limits apply before growing arrays. No omitted bricks on
/// exhaustion: the caller retains the complete original volume binning path.
pub(crate) fn build(forest: &GpuForest, max_bytes: usize) -> Result<Catalog, String> {
    let mut result = Catalog { bricks: Vec::new(), references: Vec::new(),spans:vec![[0;2];forest.levels.len()] };
    let max_bytes=max_bytes.checked_sub(result.spans.len()*8).ok_or("brick spans exceed budget")?;
    let mut spans = vec![None; forest.levels.len()];
    for (tree_id, tree) in forest.instances.iter().enumerate() {
        for relative in 0..tree.meta[2] {
            let level_id = (tree.meta[1] + relative) as usize;
            let level = forest.levels.get(level_id).ok_or("brick level out of range")?;
            let (first, count) = if let Some(span) = spans[level_id] { span } else {
                let first = result.bricks.len();
                let dims = level.dims_offset;
                let cell = level.origin_cell[3];
                if !cell.is_finite() || cell <= 0.0 { return Err("invalid brick cell size".into()); }
                for bz in 0..dims[2].div_ceil(4) {
                    for by in 0..dims[1].div_ceil(4) {
                        for bx in 0..dims[0].div_ceil(4) {
                            let base = [bx*4, by*4, bz*4];
                            let mut lo = [u32::MAX; 3];
                            let mut hi = [0; 3];
                            for z in base[2]..(base[2]+4).min(dims[2]) {
                                for y in base[1]..(base[1]+4).min(dims[1]) {
                                    for x in base[0]..(base[0]+4).min(dims[0]) {
                                        let address=cell_index(forest, level, [x,y,z]).ok_or("invalid brick indirection")?;
                                        if address==0 { continue; }
                                        if address as usize>forest.cells.len() {return Err("invalid brick cell reference".into());}
                                        let p = [x,y,z];
                                        for a in 0..3 { lo[a]=lo[a].min(p[a]); hi[a]=hi[a].max(p[a]+1); }
                                    }
                                }
                            }
                            if lo[0]==u32::MAX { continue; }
                            if (result.bricks.len()+1)*32+result.references.len()*16 > max_bytes {
                                return Err("brick visibility catalog exceeds admission budget".into());
                            }
                            let mut min = [0.0;4]; let mut max = [0.0;4];
                            // Trilinear reconstruction reaches half a cell beyond
                            // each occupied cell box. Omitting this halo drops
                            // silhouettes when a tent crosses a tile boundary.
                            for a in 0..3 {
                                min[a]=level.origin_cell[a]+(lo[a] as f32-0.5)*cell;
                                max[a]=level.origin_cell[a]+(hi[a] as f32+0.5)*cell;
                            }
                            result.bricks.push(Brick {min,max});
                        }
                    }
                }
                // Partition reconstruction halos on a common 4-cell lattice.
                // Tight bounds within each region may contain empty air, but
                // region interiors never overlap. The visibility stream can
                // integrate them without counting an optical medium twice.
                let mut regions=std::collections::BTreeMap::<[i32;3],Brick>::new();
                let width=cell*4.0;
                for b in &result.bricks[first..] {
                    let low=std::array::from_fn::<_,3,_>(|a|((b.min[a]-level.origin_cell[a])/width).floor() as i32);
                    let high=std::array::from_fn::<_,3,_>(|a|((b.max[a]-level.origin_cell[a])/width).ceil() as i32);
                    for z in low[2]..high[2] {for y in low[1]..high[1] {for x in low[0]..high[0] {
                        let key=[x,y,z];let mut part=*b;
                        for a in 0..3 {
                            part.min[a]=part.min[a].max(level.origin_cell[a]+key[a] as f32*width);
                            part.max[a]=part.max[a].min(level.origin_cell[a]+(key[a]+1) as f32*width);
                        }
                        if (0..3).any(|a|part.max[a]<=part.min[a]) {continue;}
                        if !regions.contains_key(&key) && (first+regions.len()+1)*32+result.references.len()*16>max_bytes {
                            return Err("brick support partition exceeds admission budget".into());
                        }
                        regions.entry(key).and_modify(|old|for a in 0..3 {
                            old.min[a]=old.min[a].min(part.min[a]);old.max[a]=old.max[a].max(part.max[a]);
                        }).or_insert(part);
                    }}}
                }
                result.bricks.truncate(first);result.bricks.extend(regions.into_values());
                let span=(first,result.bricks.len()-first); spans[level_id]=Some(span);
                result.spans[level_id]=[span.0 as u32,span.1 as u32];span
            };
            for brick in first..first+count {
                if result.bricks.len()*32+(result.references.len()+1)*16 > max_bytes {
                    return Err("brick visibility references exceed admission budget".into());
                }
                result.references.push(Reference {tree:tree_id as u32,level:relative,brick:brick as u32,padding:0});
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_volume_data::{GpuLevel,GpuCell,GpuTree};
    fn fixture() -> GpuForest {
        let mut indices=vec![0;9*5*2];
        indices[3]=1;indices[4+9*(4+5)]=1;indices[8]=1;
        GpuForest {levels:vec![GpuLevel {origin_cell:[-2.0,1.0,3.0,0.25],dims_offset:[9,5,2,0]}],
            indices,cells:vec![GpuCell::zeroed()],instances:vec![GpuTree {meta:[0,0,1,0],..GpuTree::zeroed()};3]}
    }
    #[test]
    fn shares_bricks_between_poses_and_includes_reconstruction_halo() {
        let f=fixture();let c=build(&f,4096).unwrap();
        assert!(!c.bricks.is_empty());assert_eq!(c.references.len(),c.bricks.len()*3);
        // Sample the full reconstruction support, including original brick
        // boundaries and negative halo coordinates. Exactly one partition
        // owns each interior point: no missed density or double extinction.
        for z in 0..2 {for y in 0..5 {for x in 0..9 {
            if cell_index(&f,&f.levels[0],[x,y,z])==Some(0) {continue;}
            for dz in [-0.49,0.01,0.51,1.01,1.49] {for dy in [-0.49,0.01,0.51,1.01,1.49] {for dx in [-0.49,0.01,0.51,1.01,1.49] {
                let offsets=[dx,dy,dz];
                let p=std::array::from_fn::<_,3,_>(|a|f.levels[0].origin_cell[a]+([x,y,z][a] as f32+offsets[a])*0.25);
                assert_eq!(c.bricks.iter().filter(|b|(0..3).all(|a|p[a]>=b.min[a]&&p[a]<b.max[a])).count(),1);
            }}}
        }}}
        for (i,r) in c.references.iter().enumerate() {assert_eq!(r.tree,(i/c.bricks.len()) as u32);assert_eq!(r.brick,(i%c.bricks.len()) as u32);}
        let words=c.words();assert_eq!(words[0] as usize,c.references.len());
        assert_eq!(words.len()*4,16+c.bricks.len()*32+c.references.len()*16+8);
        assert_eq!(c.spans[0],[0,c.bricks.len() as u32]);
    }
    #[test]
    fn admission_failure_never_returns_a_partial_catalog() {
        let f=fixture();let c=build(&f,4096).unwrap();let bytes=c.words().len()*4-16;
        assert!(build(&f,bytes-1).is_err());assert!(build(&f,bytes).is_ok());
        let mut broken=f.clone();broken.indices.clear();assert!(build(&broken,4096).is_err());
    }

    #[test]
    fn sparse_and_dense_indirection_produce_identical_bounds() {
        let mut f=fixture();let dense=build(&f,4096).unwrap();
        crate::foliage_volume_data::pack_bricks(&mut f,128).unwrap();
        let sparse=build(&f,4096).unwrap();
        assert_eq!(dense.words(),sparse.words());
    }

    #[test]
    #[ignore = "requires an exclusive GPU fixture slot"]
    fn gpu_visibility_sort_and_boundary_ownership() {
        use wgpu::util::DeviceExt;
        let gpu=crate::Gpu::new().unwrap();
        let actual=include_str!("shaders/voxel_forest.wgsl");
        let extract=|start:&str,end:&str|&actual[actual.find(start).unwrap()..actual.find(end).unwrap()];
        let shader=format!(r#"
struct VolumeInterval {{time:vec2<f32>,index:u32,padding:u32}};
@group(0) @binding(0) var<storage,read_write> volume_jobs:array<atomic<u32>>;
@group(0) @binding(1) var<storage,read> volume_bricks:array<u32>;
fn volume_stream_pool()->u32 {{return 0u;}}
struct OracleConfig {{dispatch:vec4<u32>}};
const volume_config=OracleConfig(vec4<u32>(0u,0u,2052u,0u));
const VOLUME_STREAM_FALLBACK:u32=0xffffffffu;
fn volume_stream_capacity()->u32 {{return 16u;}}
{}
{}
{}
@compute @workgroup_size(64)
fn reserve(@builtin(local_invocation_index) lane:u32) {{
    atomicStore(&volume_jobs[2053u+lane],volume_stream_reserve(1u));
}}
@compute @workgroup_size(1)
fn main() {{
    volume_stream_sort(0u,512u);
    let a=volume_brick_interval(0u,vec3<f32>(1.0,0.5,-1.0),vec3<f32>(0.0,0.0,1.0),0.0,10.0);
    let b=volume_brick_interval(1u,vec3<f32>(1.0,0.5,-1.0),vec3<f32>(0.0,0.0,1.0),0.0,10.0);
    atomicStore(&volume_jobs[2048u],bitcast<u32>(a.x));atomicStore(&volume_jobs[2049u],bitcast<u32>(a.y));
    atomicStore(&volume_jobs[2050u],bitcast<u32>(b.x));atomicStore(&volume_jobs[2051u],bitcast<u32>(b.y));
    atomicStore(&volume_jobs[2052u],0u);
}}
"#,extract("fn volume_stream_read(","// Compact support visibility."),
            extract("fn volume_brick_interval(","// Bounded reservation,"),
            extract("fn volume_stream_reserve(","fn volume_brick_rect("));
        let mut initial=vec![0u32;2117];
        let mut expected=Vec::new();
        for i in 0..512 {
            let enter=((i*37)%127) as f32;let leave=enter+1.0+(i%3) as f32;
            let row=[enter.to_bits(),leave.to_bits(),(511-i) as u32,0];
            initial[i*4..i*4+4].copy_from_slice(&row);expected.push(row);
        }
        expected.sort_by_key(|v|(v[0],v[2],v[1]));
        let output=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("brick visibility oracle"),contents:bytemuck::cast_slice(&initial),
            usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC});
        let bounds=[Brick {min:[0.0;4],max:[1.0,1.0,1.0,0.0]},Brick {min:[1.0,0.0,0.0,0.0],max:[2.0,1.0,1.0,0.0]}];
        let mut words=vec![0u32;4];words.extend_from_slice(bytemuck::cast_slice(&bounds));
        let input=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("brick oracle bounds"),contents:bytemuck::cast_slice(&words),usage:wgpu::BufferUsages::STORAGE});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:None,source:wgpu::ShaderSource::Wgsl(shader.into())});
        let data_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:None,entries:&[
            wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:false},has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry {binding:1,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None}]});
        let layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:None,bind_group_layouts:&[Some(&data_layout)],immediate_size:0});
        let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:None,layout:Some(&layout),module:&module,entry_point:Some("main"),compilation_options:Default::default(),cache:None});
        let reserve=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:None,layout:Some(&layout),module:&module,entry_point:Some("reserve"),compilation_options:Default::default(),cache:None});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:None,layout:&pipeline.get_bind_group_layout(0),entries:&[
            wgpu::BindGroupEntry {binding:0,resource:output.as_entire_binding()},wgpu::BindGroupEntry {binding:1,resource:input.as_entire_binding()}]});
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:output.size(),usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);}
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&reserve);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(1,1,1);}
        encoder.copy_buffer_to_buffer(&output,0,&read,0,output.size());let submission=gpu.queue.submit([encoder.finish()]);
        let (send,receive)=std::sync::mpsc::sync_channel(1);
        read.slice(..).map_async(wgpu::MapMode::Read,move |r|{let _=send.send(r);});
        gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).unwrap();
        receive.try_recv().unwrap().unwrap();let bytes=read.slice(..).get_mapped_range().unwrap();
        let actual:Vec<[u32;4]>=bytes[..8192].chunks_exact(16).map(bytemuck::pod_read_unaligned).collect();
        assert_eq!(actual,expected,"GPU interval sort must preserve all 512 records and stable depth ties");
        let boundary:[f32;4]=bytemuck::pod_read_unaligned(&bytes[8192..8208]);
        assert!(boundary[1]<=boundary[0]);assert_eq!(&boundary[2..],&[1.0,2.0]);
        let cursor:u32=bytemuck::pod_read_unaligned(&bytes[8208..8212]);assert!((16..=80).contains(&cursor));
        let mut allocated:Vec<u32>=bytes[8212..].chunks_exact(4).map(bytemuck::pod_read_unaligned::<u32>)
            .filter(|&v|v!=u32::MAX).collect();allocated.sort_unstable();
        assert_eq!(allocated,(0..16).collect::<Vec<_>>(),"concurrent overflow must neither duplicate nor expose an out-of-pool address");
    }
}
