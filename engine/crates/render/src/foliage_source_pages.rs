//! Source geometry shares the forest's logical publication and demand stream.
//! Cluster IDs remain stable; vertex/index payloads live in bounded arenas.
use super::{Arena,Allocation};
use crate::{Gpu,Scene,cluster::{Vertex,GpuCluster},scene::GpuInstance};
use std::collections::{BTreeMap,BTreeSet};

#[derive(Clone,Copy,Debug)]
pub struct Budget {pub vertices:u32,pub cluster_vertices:u32,pub triangles:u32}
impl Default for Budget {fn default()->Self{Self{vertices:2*1024*1024,cluster_vertices:4*1024*1024,triangles:4*1024*1024}}}
struct Page {vertices:Vec<Vertex>,indices:Vec<u32>,triangles:Vec<u32>,clusters:Vec<(u32,GpuCluster)>}
impl Page {
    fn cost(&self)->[u64;3]{[self.vertices.len()as u64,self.indices.len()as u64,self.triangles.len()as u64]}
    fn cook(scene:&Scene,ids:&[u32])->Result<Self,String>{
        Self::cook_reserved(scene,ids,0)
    }
    fn cook_reserved(scene:&Scene,ids:&[u32],reserved:usize)->Result<Self,String>{
        let mut p=Self{vertices:scene.vertices.get(..reserved).ok_or("skinned source vertex range outside catalog")?.to_vec(),indices:Vec::new(),triangles:Vec::new(),clusters:Vec::new()};let mut remap=BTreeMap::new();let mut topology=BTreeMap::new();
        for &id in ids {
            let mut c=*scene.clusters.get(id as usize).ok_or("source cluster outside catalog")?;
            let triangles=scene.tris.get(c.first_tri as usize..c.first_tri as usize+c.tri_count as usize).ok_or("source triangles outside catalog")?;
            let count=triangles.iter().flat_map(|t|[t&255,(t>>8)&255,(t>>16)&255]).max().map_or(0,|n|n+1);
            let indices=scene.cluster_verts.get(c.first_vert as usize..c.first_vert as usize+count as usize).ok_or("source cluster vertices outside catalog")?;
            let key=(c.first_tri,c.tri_count);
            c.first_tri=if let Some(&at)=topology.get(&key){at}else{let at=p.triangles.len()as u32;p.triangles.extend_from_slice(triangles);topology.insert(key,at);at};c.first_vert=p.indices.len()as u32;
            for &v in indices {let at=if (v as usize)<reserved{v}else if let Some(&at)=remap.get(&v){at}else{
                let vertex=*scene.vertices.get(v as usize).ok_or("source vertex outside catalog")?;
                let at=p.vertices.len()as u32;p.vertices.push(vertex);remap.insert(v,at);at};p.indices.push(at);}
            p.clusters.push((id,c));
        }Ok(p)
    }
    // Skinning and procedural terrain write original vertex addresses. Preserve their prefix;
    // compact other pinned assets so late non-forest meshes do not pin the
    // intervening forest catalog's entire original vertex/index allocation.
    fn pinned(scene:&Scene,ids:&[u32])->Result<Self,String>{
        let reserved=scene.skinning.actors.iter().map(|a|a.first_vertex as usize+a.vertex_count as usize).max().unwrap_or(0)
            .max(scene.procedural_terrain.as_ref().map_or(0,|t|t.first_vertex as usize+t.capacity as usize*81));
        Self::cook_reserved(scene,ids,reserved)
    }
}
#[derive(Clone)]
struct Resident {ranges:[Allocation;3],used:u64}
struct Write {buffer:usize,offset:u64,bytes:Vec<u8>}
struct Pending {id:usize,resident:Resident,writes:Vec<Write>,write:usize,offset:usize}
#[derive(Clone,Copy,Debug,Default,serde::Serialize)]
pub struct Stats {pub allocated_bytes:u64,pub resident_pages:u32,pub total_pages:u32,pub uploads:u64,pub evictions:u64,pub uploaded_bytes:u64,pub pinned_bytes:u64}
pub struct Cache {
    pages:Vec<Page>,pub deps:Vec<Vec<usize>>,pub capacities:[u64;3],arenas:[Arena;3],resident:Vec<Option<Resident>>,pending:Option<Pending>,
    /// Vertices, cluster vertices and triangles share one buffer (topology.rs).
    pub(crate) topology:crate::topology::Topology,pub clusters:wgpu::Buffer,pub dirty:BTreeSet<usize>,stats:Stats,
}
/// Conservative draw/traversal work, NOT unique resident asset bytes.
/// Repeated templates sharing one mesh each consume work. Includes every CLOD
/// cluster and triangle as an upper bound; actual selected cuts can be cheaper.
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub(crate) struct SourceWork {pub clusters:u32,pub triangles:u32}
pub(crate) fn source_work(scene:&Scene,source:GpuInstance)->Result<SourceWork,String>{
    let members=if source.cluster_count==0 && source.first_cluster!=0 && source.flags&crate::scene::PART_WIND!=0 {
        let start=(source.first_cluster>>12)as usize;let count=(source.first_cluster&4095)as usize;
        scene.instances.get(start..start+count).ok_or("source work templates outside catalog")?
    }else{std::slice::from_ref(&source)};
    let mut result=SourceWork::default();
    for member in members {
        let end=member.first_cluster.checked_add(member.cluster_count).ok_or("source work cluster range overflow")?;
        let clusters=scene.clusters.get(member.first_cluster as usize..end as usize).ok_or("source work clusters outside catalog")?;
        result.clusters=result.clusters.checked_add(member.cluster_count).ok_or("source cluster work overflow")?;
        for cluster in clusters {result.triangles=result.triangles.checked_add(cluster.tri_count).ok_or("source triangle work overflow")?;}
    }
    Ok(result)
}

/// Bins of the source work table: mesh error thresholds in the tree's local
/// metres, SOURCE_TABLE_T0 * 2^(k/2) for k in 0..24 (0.25 mm .. 0.7 m). Half
/// octaves: a cut books the bin at or below its threshold, and whole octaves
/// lost up to one LOD level there (thinned needle brushes step ~1.6x).
pub const SOURCE_TABLE_BINS:usize=24;
pub const SOURCE_TABLE_T0:f32=0.00025;
/// A packed bin at this value means "every cluster" (the node's full count).
pub const SOURCE_TABLE_FULL:u32=0xffff;
/// A node's table as stored on the GPU: two 16-bit bins per word, bin 2i
/// low. Counts that do not fit 16 bits store SOURCE_TABLE_FULL.
pub type PackedSourceTable=[u32;SOURCE_TABLE_BINS/2];
pub const FULL_SOURCE_TABLE:PackedSourceTable=[SOURCE_TABLE_FULL|SOURCE_TABLE_FULL<<16;SOURCE_TABLE_BINS/2];
/// Clusters the cull visits for this source at each bin's threshold: per
/// member, those whose parent_error (in the member's own units: the bin over
/// its scale) exceeds it when its mesh is sorted coarse first
/// (ClusterMesh::sort_coarse_first), all of them otherwise. Bin k bounds every
/// cut at a threshold of at least t_k; foliage_refine books by it.
pub(crate) fn source_work_table(scene:&Scene,source:GpuInstance)->Result<PackedSourceTable,String>{
    let members=if source.cluster_count==0 && source.first_cluster!=0 && source.flags&crate::scene::PART_WIND!=0 {
        let start=(source.first_cluster>>12)as usize;let count=(source.first_cluster&4095)as usize;
        scene.instances.get(start..start+count).ok_or("source work templates outside catalog")?
    }else{std::slice::from_ref(&source)};
    let mut table=[0u32;SOURCE_TABLE_BINS];
    for member in members {
        let end=member.first_cluster.checked_add(member.cluster_count).ok_or("source work cluster range overflow")?;
        let clusters=scene.clusters.get(member.first_cluster as usize..end as usize).ok_or("source work clusters outside catalog")?;
        let sorted=!clusters.is_empty() && clusters.iter().all(|c|c.order&crate::cluster::CLUSTERS_COARSE_FIRST!=0);
        for (k,slot) in table.iter_mut().enumerate() {
            let t=source_table_threshold(k)/member.scale.max(1e-12);
            let count=if sorted {clusters.partition_point(|c|c.parent_error>t)as u32}else{member.cluster_count};
            *slot=slot.checked_add(count).ok_or("source cluster work overflow")?;
        }
    }
    let bin=|k:usize|table[k].min(SOURCE_TABLE_FULL);
    Ok(std::array::from_fn(|i|bin(2*i)|bin(2*i+1)<<16))
}
/// Bin k's threshold in the tree's local metres.
pub fn source_table_threshold(k:usize)->f32 {SOURCE_TABLE_T0*(k as f32*0.5).exp2()}
/// The bins of a packed table (SOURCE_TABLE_FULL where every cluster counts).
pub fn unpack_source_table(t:&PackedSourceTable)->[u32;SOURCE_TABLE_BINS] {std::array::from_fn(|k|(t[k/2]>>(16*(k%2)))&0xffff)}

fn clusters(scene:&Scene,source:GpuInstance)->Result<BTreeSet<u32>,String>{
    let members=if source.cluster_count==0 && source.first_cluster!=0 && source.flags&32!=0 {
        let start=(source.first_cluster>>12)as usize;let count=(source.first_cluster&4095)as usize;
        scene.instances.get(start..start+count).ok_or("source template range outside catalog")?.to_vec()
    }else{vec![source]};
    let mut result=BTreeSet::new();for m in members {let end=m.first_cluster.checked_add(m.cluster_count).ok_or("source cluster overflow")?;
        if end as usize>scene.clusters.len(){return Err("source template clusters outside catalog".into());}result.extend(m.first_cluster..end);}
    Ok(result)
}
impl Cache {
    pub fn new(gpu:&Gpu,scene:&Scene,representations:&[Option<GpuInstance>],budget:Budget)->Result<Self,String>{
        let sets=representations.iter().map(|r|r.map(|s|clusters(scene,s)).unwrap_or_else(||Ok(BTreeSet::new()))).collect::<Result<Vec<_>,_>>()?;
        let mut foliage=sets.iter().flat_map(|s|s.iter().copied()).collect::<BTreeSet<_>>();
        // A mesh can be shared by foliage and a normal, independently visible
        // object. Such clusters must remain pinned, not follow forest eviction.
        let templates=representations.iter().flatten().filter(|s|s.cluster_count==0&&s.first_cluster!=0&&s.flags&32!=0)
            .flat_map(|s|(s.first_cluster>>12)..(s.first_cluster>>12)+(s.first_cluster&4095)).collect::<BTreeSet<_>>();
        for (i,&inst) in scene.instances.iter().enumerate(){
            let forest_slot=scene.gpu_forest.as_ref().is_some_and(|s|i>=s.first_owner as usize && i<(s.first_owner+s.slots)as usize);
            if !forest_slot && (!templates.contains(&(i as u32)) || scene.skinning.actors.iter().any(|a|a.owner as usize==i)) && inst.cluster_count>0 {for c in clusters(scene,inst)?{foliage.remove(&c);}}
        }
        let pinned=(0..scene.clusters.len()as u32).filter(|c|!foliage.contains(c)).collect::<Vec<_>>();
        let mut pages=vec![Page::pinned(scene,&pinned)?];let mut owners=BTreeMap::new();
        let ids=foliage.into_iter().collect::<Vec<_>>();for group in ids.chunks(64){let id=pages.len();for &c in group{owners.insert(c,id);}pages.push(Page::cook(scene,group)?);}
        let deps=sets.iter().map(|s|s.iter().filter_map(|c|owners.get(c).copied()).collect::<BTreeSet<_>>().into_iter().collect()).collect();
        let totals=pages.iter().fold([0u64;3],|mut a,p|{for(i,c)in p.cost().iter().enumerate(){a[i]+=c;}a});
        let limits=[budget.vertices as u64,budget.cluster_vertices as u64,budget.triangles as u64];let capacities=std::array::from_fn(|i|totals[i].min(limits[i]));
        if (0..3).any(|i|pages[0].cost()[i]>capacities[i]){return Err("pinned source geometry exceeds cache budget".into());}
        let sizes=[capacities[0]*32,capacities[1]*4,capacities[2]*4,scene.clusters.len()as u64*80];
        if sizes.iter().any(|&s|s>gpu.device.limits().max_storage_buffer_binding_size as u64){return Err("source cache exceeds device binding limit".into());}
        let topology=crate::topology::Topology::new(gpu,"bounded forest source topology",[sizes[0],sizes[1],sizes[2]],wgpu::BufferUsages::COPY_SRC);
        let clusters=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("bounded forest source clusters"),size:sizes[3].max(80),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let mut c=Self{deps,capacities,arenas:std::array::from_fn(|i|Arena::new(capacities[i]as u32)),resident:vec![None;pages.len()],pending:None,topology,clusters,dirty:(0..pages.len()).collect(),stats:Stats{allocated_bytes:sizes.iter().map(|s|s.max(&80)).sum(),total_pages:pages.len()as u32,..Default::default()},pages};
        c.begin(0,0)?;if c.pending.is_none(){return Err("cannot allocate pinned source geometry".into());}c.advance(gpu,u64::MAX)?;c.stats.pinned_bytes=c.stats.uploaded_bytes;Ok(c)
    }
    pub fn cost(&self,id:usize)->[u64;3]{self.pages[id].cost()}
    pub fn ready(&self,rep:usize)->bool{self.deps[rep].iter().all(|&p|self.resident[p].is_some())}
    pub fn changed(&self,rep:usize)->bool{self.deps[rep].iter().any(|p|self.dirty.contains(p))}
    pub fn complete(&self)->bool{self.pending.is_none()&&self.resident.iter().all(Option::is_some)}
    fn begin(&mut self,id:usize,frame:u64)->Result<(),String>{
        if self.pending.is_some()||self.resident[id].is_some(){return Ok(());}let cost=self.cost(id);let mut ranges=Vec::new();
        for i in 0..3 {if let Some(a)=self.arenas[i].allocate(cost[i]as u32){ranges.push(a);}else{for(j,a)in ranges.into_iter().enumerate(){self.arenas[j].release(a)?;}return Ok(());}}
        let ranges:[Allocation;3]=ranges.try_into().unwrap();let p=&self.pages[id];let indices=p.indices.iter().map(|v|v+ranges[0].range.start).collect::<Vec<_>>();
        let mut writes=vec![Write{buffer:0,offset:ranges[0].range.start as u64*32,bytes:bytemuck::cast_slice(&p.vertices).to_vec()},Write{buffer:1,offset:ranges[1].range.start as u64*4,bytes:bytemuck::cast_slice(&indices).to_vec()},Write{buffer:2,offset:ranges[2].range.start as u64*4,bytes:bytemuck::cast_slice(&p.triangles).to_vec()}];
        for &(id,mut c) in &p.clusters {
            c.first_vert+=ranges[1].range.start;c.first_tri+=ranges[2].range.start;let offset=id as u64*80;
            // Thousands of pinned terrain headers are contiguous: one upload,
            // not one queue allocation/write per logical cluster.
            if let Some(last)=writes.last_mut().filter(|w|w.buffer==3 && w.offset+w.bytes.len()as u64==offset){last.bytes.extend_from_slice(bytemuck::bytes_of(&c));}
            else{writes.push(Write{buffer:3,offset,bytes:bytemuck::bytes_of(&c).to_vec()});}
        }
        self.pending=Some(Pending{id,resident:Resident{ranges,used:frame},writes,write:0,offset:0});Ok(())
    }
    fn advance(&mut self,gpu:&Gpu,budget:u64)->Result<u64,String>{
        let Some(p)=self.pending.as_mut()else{return Ok(0);};let mut remaining=budget&!3;let initial=remaining;
        while p.write<p.writes.len() {let w=&p.writes[p.write];let n=(w.bytes.len()-p.offset).min(remaining as usize);
            if n>0 {let bytes=&w.bytes[p.offset..p.offset+n];if w.buffer==3 {gpu.queue.write_buffer(&self.clusters,w.offset+p.offset as u64,bytes);}else{self.topology.write(gpu,w.buffer,w.offset+p.offset as u64,bytes);}p.offset+=n;remaining-=n as u64;}
            if p.offset==w.bytes.len(){p.write+=1;p.offset=0;}else{break;}
        }
        if p.write==p.writes.len(){let p=self.pending.take().unwrap();self.resident[p.id]=Some(p.resident);self.dirty.insert(p.id);self.stats.uploads+=1;}
        self.stats.uploaded_bytes+=initial-remaining;Ok(initial-remaining)
    }
    pub fn service(&mut self,gpu:&Gpu,wanted:&BTreeSet<usize>,frame:u64,remaining:&mut u64,completed:&mut u32,max_pages:u32)->Result<(),String>{
        for &id in wanted{if let Some(r)=&mut self.resident[id]{r.used=frame;}}
        if self.pending.is_some(){*remaining-=self.advance(gpu,*remaining)?;if self.pending.is_none(){*completed+=1;}}
        for &id in wanted {
            if self.resident[id].is_some(){continue;}if self.pending.is_some()||*remaining<4||*completed>=max_pages{break;}
            self.begin(id,frame)?;
            while self.pending.is_none(){let victim=self.resident.iter().enumerate().filter(|(i,r)|*i!=0&&r.is_some()&&!wanted.contains(i)).min_by_key(|(_,r)|r.as_ref().unwrap().used).map(|(i,_)|i);
                let Some(victim)=victim else{break;};let r=self.resident[victim].take().unwrap();for(i,a)in r.ranges.into_iter().enumerate(){self.arenas[i].release(a)?;}self.dirty.insert(victim);self.stats.evictions+=1;self.begin(id,frame)?;}
            if self.pending.is_some(){*remaining-=self.advance(gpu,*remaining)?;if self.pending.is_none(){*completed+=1;}}
        }Ok(())
    }
    #[cfg(test)]
    pub fn warm(&mut self,gpu:&Gpu)->Result<(),String>{for id in 1..self.pages.len(){self.begin(id,0)?;if self.pending.is_none(){return Err("source fixture budget too small".into());}self.advance(gpu,u64::MAX)?;}Ok(())}
    pub fn stats(&self)->Stats{Stats{resident_pages:self.resident.iter().filter(|r|r.is_some()).count()as u32,..self.stats}}
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bytemuck::Zeroable;
    pub(crate) fn scene()->(Scene,Vec<Option<GpuInstance>>){
        let mut scene=Scene::default();
        for i in 0..129u32 {
            for x in 0..3{scene.vertices.push(Vertex{pos:[i as f32,x as f32,0.],normal:[0.,0.,1.],uv:[0.;2]});scene.cluster_verts.push(i*3+x);}
            scene.tris.push(0x020100);let mut c=GpuCluster::zeroed();c.first_vert=i*3;c.first_tri=i;c.tri_count=1;c.material=i;scene.clusters.push(c);
        }
        let mut pin=GpuInstance::zeroed();pin.cluster_count=1;pin.sphere[3]=1.;scene.instances.push(pin);
        let mut a=pin;a.first_cluster=1;a.cluster_count=64;let mut b=a;b.first_cluster=65;
        (scene,vec![None,Some(a),Some(b)])
    }
    #[test]
    fn foliage_source_page_cook_preserves_every_triangle_and_attribute(){
        let(scene,_)=scene();let p=Page::cook(&scene,&[2,65,128]).unwrap();
        for &(id,c) in &p.clusters{let original=scene.clusters[id as usize];assert_eq!(c.material,original.material);
            for t in 0..c.tri_count {let tri=p.triangles[(c.first_tri+t)as usize];let orig=scene.tris[(original.first_tri+t)as usize];
                for shift in [0,8,16]{let a=p.vertices[p.indices[(c.first_vert+((tri>>shift)&255))as usize]as usize];let b=scene.vertices[scene.cluster_verts[(original.first_vert+((orig>>shift)&255))as usize]as usize];assert_eq!(bytemuck::bytes_of(&a),bytemuck::bytes_of(&b));}
            }
        }
    }
    #[test]
    fn foliage_source_pins_compact_late_static_objects(){
        let(scene,_)=scene();let p=Page::pinned(&scene,&[128]).unwrap();
        assert_eq!(p.vertices.len(),3);assert_eq!(p.indices,[0,1,2]);assert_eq!(p.clusters[0].1.first_vert,0);
        assert_eq!(bytemuck::cast_slice::<Vertex,u8>(&p.vertices),bytemuck::cast_slice::<Vertex,u8>(&scene.vertices[384..]));
    }
    #[test]
    fn foliage_source_pins_preserve_dynamic_terrain_addresses_and_shared_topology(){
        let(mut scene,_)=scene();scene.materials.push(crate::scene::Material{tint:[1.;3],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
        let spec=crate::procedural_terrain::reserve(&mut scene,2,0,1,Default::default());
        let p=Page::pinned(&scene,&[spec.first_cluster,spec.first_cluster+1]).unwrap();
        assert_eq!(p.vertices.len(),spec.first_vertex as usize+162);
        assert_eq!(p.indices[0],spec.first_vertex);assert_eq!(p.indices[81],spec.first_vertex+81);
        assert_eq!(p.triangles.len(),128,"shared terrain topology must not expand per patch");
        assert_eq!(p.clusters[0].1.first_tri,p.clusters[1].1.first_tri);
    }
    #[test]
    #[ignore="exclusive bounded source page fixture"]
    fn foliage_source_pages_gpu_partial_upload_evict_reload_preserves_pinned_geometry(){
        let gpu=Gpu::new().unwrap();let(scene,reps)=scene();let budget=Budget{vertices:195,cluster_vertices:195,triangles:65};
        let mut cache=Cache::new(&gpu,&scene,&reps,budget).unwrap();assert_eq!(cache.stats().resident_pages,1);let allocated=cache.stats().allocated_bytes;
        let read=|buffer:&wgpu::Buffer|{let dst=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("source page fixture read"),size:buffer.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(buffer,0,&dst,0,buffer.size());gpu.queue.submit(Some(enc.finish()));dst.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();let bytes=dst.slice(..).get_mapped_range().unwrap().to_vec();dst.unmap();bytes};
        for target in [1,2,1] {
            let wanted=BTreeSet::from([0,target]);
            for frame in 0..100 {let mut remaining=256;let mut complete=0;cache.service(&gpu,&wanted,frame,&mut remaining,&mut complete,1).unwrap();assert!(remaining<=256);assert!(complete<=1);assert_eq!(cache.stats().allocated_bytes,allocated);assert!(cache.resident[0].is_some());if frame==0{assert!(!cache.ready(target));}if cache.ready(target){break;}}
            assert!(cache.ready(target));assert!(cache.stats().resident_pages<=2);
            let cluster_bytes=read(&cache.clusters);let all=read(&cache.topology.buffer);let region=|i:usize|{let (o,n)=cache.topology.regions[i];all[o as usize..(o+n)as usize].to_vec()};let vertices=region(0);let indices=region(1);let triangles=region(2);
            let id=if target==1{1}else{65};let c:GpuCluster=bytemuck::pod_read_unaligned(&cluster_bytes[id*80..][..80]);let tri:u32=bytemuck::pod_read_unaligned(&triangles[c.first_tri as usize*4..][..4]);assert_eq!(tri,scene.tris[id]);
            for x in 0..3{let v:u32=bytemuck::pod_read_unaligned(&indices[(c.first_vert as usize+x)*4..][..4]);let got:Vertex=bytemuck::pod_read_unaligned(&vertices[v as usize*32..][..32]);assert_eq!(bytemuck::bytes_of(&got),bytemuck::bytes_of(&scene.vertices[id*3+x]));}
            assert_eq!(&vertices[..96],bytemuck::cast_slice::<Vertex,u8>(&scene.vertices[..3]));assert_eq!(&cluster_bytes[..80],bytemuck::bytes_of(&scene.clusters[0]));
        }
        assert_eq!(cache.stats().evictions,2);
        assert!(Cache::new(&gpu,&scene,&reps,Budget{vertices:2,..budget}).is_err());
    }
}

#[cfg(test)] mod work_tests {
    use super::*;
    use bytemuck::Zeroable;
    #[test] fn foliage_source_work_counts_instances_not_unique_meshes() {
        let mut scene=Scene::default();
        let mut cluster=GpuCluster::zeroed();cluster.tri_count=64;scene.clusters.push(cluster);
        let mesh=GpuInstance {cluster_count:1,..GpuInstance::zeroed()};scene.instances=vec![mesh;3];
        let carrier=GpuInstance {first_cluster:3,flags:crate::scene::PART_WIND,..GpuInstance::zeroed()};
        assert_eq!(clusters(&scene,carrier).unwrap().len(),1,"one shared resident mesh");
        assert_eq!(source_work(&scene,carrier).unwrap(),SourceWork {clusters:3,triangles:192},"three instances still execute three times");
        assert_eq!(source_work(&scene,mesh).unwrap(),SourceWork {clusters:1,triangles:64});
        assert!(source_work(&scene,GpuInstance {first_cluster:4,..carrier}).is_err());
        assert!(source_work(&scene,GpuInstance {first_cluster:u32::MAX,..mesh}).is_err());
        scene.clusters[0].tri_count=u32::MAX;assert!(source_work(&scene,carrier).is_err());
    }
}
