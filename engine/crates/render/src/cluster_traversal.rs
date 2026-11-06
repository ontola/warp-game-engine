//! Exact, opt-in cluster-selection acceleration. This is a BVH over the
//! existing cluster predicates, not a reconstructed meshoptimizer LOD DAG.
//! Original owner/cluster IDs and the final selection predicate are unchanged.
//! Nodes are shared per mesh and traversed in pre-order with escape indices;
//! no bounded GPU stack or per-frame CPU/GPU readback is required.
use crate::{cluster::GpuCluster, scene::Scene, Gpu};
use glam::{DVec3, Vec3};
use wgpu::util::DeviceExt;

pub const COUNTER_BYTES: u64 = 32;
pub const COUNTER_OFFSET: u64 = crate::occlusion::PHASE_COUNTER_BYTES;
pub const LEAF_CLUSTERS: usize = 32;
pub const MIN_CLUSTERS: usize = 64;
const NODE_WORDS: usize = 20;
const NONE: u32 = u32::MAX;

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub enabled: bool,
    pub shared_bytes: u64,
    pub metadata_nodes: u32,
    pub indexed_meshes: u32,
    pub counter_bytes_per_view: u64,
    pub accelerated_owners: u32,
    pub nodes_tested: u32,
    pub nodes_rejected: u32,
    /// Exact cluster-predicate evaluations, after node rejection.
    pub clusters_tested: u32,
    pub clusters_skipped: u32,
    /// Small/unindexed mesh ranges retained on the existing flat path.
    pub flat_owners: u32,
    /// Invalid/mismatched runtime ranges or transforms retained on the flat path.
    pub fallback_owners: u32,
    /// Sum of resident clusters for accelerated owners; tested + skipped.
    pub resident_clusters: u32,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    cull: [f32; 4],
    parent: [f32; 4],
    own: [f32; 4],
    max_parent_error: f32,
    min_own_error: f32,
    escape: u32,
    first: u32,
    count: u32,
    descendants: u32,
}

struct Packed { words: Vec<u32>, nodes: u32, meshes: u32 }

fn valid_cluster(c: &GpuCluster) -> bool {
    [c.cull,c.parent,c.own].iter().all(|b| b.iter().all(|v|v.is_finite() && v.abs()<1e18) && b[3]>=0.)
        && c.parent_error.is_finite() && c.parent_error>=0.
        && c.own_error.is_finite() && c.own_error>=0.
}

/// A sphere enclosing every original sphere, with a roundoff margin. The
/// GPU additionally expands for affine transformation roundoff and wind.
fn sphere(ids: &[u32], clusters: &[GpuCluster], field: fn(&GpuCluster)->[f32;4]) -> [f32;4] {
    let mut lo=DVec3::splat(f64::INFINITY); let mut hi=DVec3::splat(f64::NEG_INFINITY);
    for &id in ids { let b=field(&clusters[id as usize]); let p=DVec3::new(b[0]as f64,b[1]as f64,b[2]as f64);
        lo=lo.min(p-DVec3::splat(b[3]as f64)); hi=hi.max(p+DVec3::splat(b[3]as f64)); }
    let center=((lo+hi)*0.5).as_vec3(); let cd=center.as_dvec3();
    let radius=ids.iter().map(|&id| {let b=field(&clusters[id as usize]);
        (DVec3::new(b[0]as f64,b[1]as f64,b[2]as f64)-cd).length()+b[3]as f64
    }).fold(0.0f64,f64::max);
    center.extend((radius*(1.0+1e-5)+1e-4)as f32).to_array()
}

fn build_node(ids:&mut [u32], clusters:&[GpuCluster], nodes:&mut Vec<Node>, members:&mut Vec<u32>) -> u32 {
    let id=nodes.len()as u32;
    let cull=sphere(ids,clusters,|c|c.cull);
    let parent=sphere(ids,clusters,|c|c.parent);
    let own=sphere(ids,clusters,|c|c.own);
    let max_parent_error=ids.iter().map(|&i|clusters[i as usize].parent_error).fold(0.0f32,f32::max);
    let min_own_error=ids.iter().map(|&i|clusters[i as usize].own_error).fold(f32::INFINITY,f32::min);
    nodes.push(Node {cull,parent,own,max_parent_error,min_own_error,escape:0,first:0,count:0,descendants:ids.len()as u32});
    if ids.len()<=LEAF_CLUSTERS {
        nodes[id as usize].first=members.len()as u32;
        nodes[id as usize].count=ids.len()as u32;
        members.extend_from_slice(ids);
    } else {
        // Keep error ranges coherent first. Within identical error bands,
        // spatial separation makes frustum and distance bounds useful.
        let min_parent=ids.iter().map(|&i|clusters[i as usize].parent_error).fold(f32::INFINITY,f32::min);
        let max_own=ids.iter().map(|&i|clusters[i as usize].own_error).fold(0.0f32,f32::max);
        if min_own_error!=max_own {
            ids.sort_unstable_by(|a,b|clusters[*a as usize].own_error.total_cmp(&clusters[*b as usize].own_error).then(a.cmp(b)));
        } else if min_parent!=max_parent_error {
            ids.sort_unstable_by(|a,b|clusters[*a as usize].parent_error.total_cmp(&clusters[*b as usize].parent_error).then(a.cmp(b)));
        } else {
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for &i in ids.iter(){let p=Vec3::from_slice(&clusters[i as usize].cull[..3]);lo=lo.min(p);hi=hi.max(p);}
            let extent=hi-lo;let axis=if extent.x>=extent.y&&extent.x>=extent.z{0}else if extent.y>=extent.z{1}else{2};
            ids.sort_unstable_by(|a,b|clusters[*a as usize].cull[axis].total_cmp(&clusters[*b as usize].cull[axis]).then(a.cmp(b)));
        }
        let mid=ids.len()/2;
        let (left,right)=ids.split_at_mut(mid);
        build_node(left,clusters,nodes,members);build_node(right,clusters,nodes,members);
    }
    nodes[id as usize].escape=nodes.len()as u32;
    id
}

impl Packed {
    fn new(scene:&Scene, enabled:bool)->Result<Self,String> {
        if !enabled {return Ok(Self {words:vec![0,0,0,0],nodes:0,meshes:0});}
        let cluster_count=u32::try_from(scene.clusters.len()).map_err(|_|"cluster BVH exceeds u32 clusters")?;
        let mut roots=vec![[NONE,0];scene.clusters.len()];
        let mut nodes=Vec::new();let mut members=Vec::new();let mut meshes=0;
        for mesh in &scene.meshes {
            let first=mesh.first_cluster as usize;let count=mesh.cluster_count as usize;
            let end=first.checked_add(count).ok_or("cluster BVH mesh range overflow")?;
            if end>scene.clusters.len(){return Err("cluster BVH mesh range outside scene".into());}
            if count<MIN_CLUSTERS || !scene.clusters[first..end].iter().all(valid_cluster){continue;}
            if roots[first][0]!=NONE {
                if roots[first][1]!=mesh.cluster_count{return Err("cluster BVH conflicting mesh ranges".into());}
                continue;
            }
            let mut ids=(mesh.first_cluster..mesh.first_cluster+mesh.cluster_count).collect::<Vec<_>>();
            roots[first]=[build_node(&mut ids,&scene.clusters,&mut nodes,&mut members),mesh.cluster_count];meshes+=1;
        }
        let node_start=4usize.checked_add(roots.len().checked_mul(2).ok_or("cluster BVH table overflow")?).ok_or("cluster BVH table overflow")?;
        let member_start=node_start.checked_add(nodes.len().checked_mul(NODE_WORDS).ok_or("cluster BVH nodes overflow")?).ok_or("cluster BVH nodes overflow")?;
        let total=member_start.checked_add(members.len()).ok_or("cluster BVH members overflow")?;
        u32::try_from(total).map_err(|_|"cluster BVH word offsets exceed u32")?;
        let mut words=Vec::with_capacity(total);
        words.extend([1,cluster_count,node_start as u32,member_start as u32]);
        for [root,count] in roots {words.extend([if root==NONE{NONE}else{node_start as u32+root*NODE_WORDS as u32},count]);}
        for node in &nodes {
            words.extend(node.cull.map(f32::to_bits));words.extend(node.parent.map(f32::to_bits));words.extend(node.own.map(f32::to_bits));
            words.extend([node.max_parent_error.to_bits(),node.min_own_error.to_bits(),0,0]);
            words.extend([node_start as u32+node.escape*NODE_WORDS as u32,member_start as u32+node.first,node.count,node.descendants]);
        }
        words.extend(members);
        Ok(Self {words,nodes:nodes.len()as u32,meshes})
    }
}

pub(crate) struct Resources {
    pub enabled: bool,
    pub buffer: wgpu::Buffer,
    nodes:u32,
    meshes:u32,
}
impl Resources {
    pub fn new(gpu:&Gpu,scene:&Scene)->Result<Self,String> {
        let enabled=std::env::var("KERN_CLUSTER_TRAVERSAL").as_deref()==Ok("1");
        let packed=Packed::new(scene,enabled)?;
        let bytes=bytemuck::cast_slice(&packed.words);
        if bytes.len()as u64>u64::from(gpu.limits.max_storage_buffer_binding_size){return Err("cluster BVH exceeds storage binding limit".into());}
        let buffer=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("shared cluster predicate BVH"),contents:bytes,usage:wgpu::BufferUsages::STORAGE});
        if enabled {
            eprintln!("Cluster traversal: {} meshes, {} nodes, {} shared bytes; exact camera predicates, shadow paths unchanged",packed.meshes,packed.nodes,bytes.len());
            // Bounded startup evidence of actual mesh eligibility. This scan
            // is never part of a rendered frame or a diagnostic readback.
            let mut counts=std::collections::BTreeMap::<(u32,u32),usize>::new();
            for inst in &scene.instances {if inst.cluster_count>0 {*counts.entry((inst.first_cluster,inst.cluster_count)).or_default()+=1;}}
            let mut ranges=counts.into_iter().collect::<Vec<_>>();
            ranges.sort_unstable_by(|a,b|b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            for ((first,count),owners) in ranges.into_iter().take(16) {
                let table=4+first as usize*2;
                let indexed=first<packed.words[1] && packed.words[table]!=NONE && packed.words[table+1]==count;
                eprintln!("Cluster traversal range: first={first}, clusters={count}, owner_slots={owners}, indexed={indexed}");
            }
        }
        Ok(Self {enabled,buffer,nodes:packed.nodes,meshes:packed.meshes})
    }
    pub fn diagnostics(&self,words:&[u32])->Result<Diagnostics,String> {
        validate_counts(words)?;
        Ok(Diagnostics {enabled:self.enabled,shared_bytes:self.buffer.size(),metadata_nodes:self.nodes,indexed_meshes:self.meshes,counter_bytes_per_view:COUNTER_BYTES,
            accelerated_owners:words[0],nodes_tested:words[1],nodes_rejected:words[2],clusters_tested:words[3],clusters_skipped:words[4],flat_owners:words[5],fallback_owners:words[6],resident_clusters:words[7]})
    }
}
fn validate_counts(words:&[u32])->Result<(),String> {
    if words.len()!=8{return Err("invalid cluster traversal diagnostic size".into());}
    if words[2]>words[1] || words[3].checked_add(words[4])!=Some(words[7]) {
        return Err("incomplete cluster BVH traversal accounting".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{ClusterMesh,Part,SimplificationPolicy,read_kmesh};
    use glam::{Mat4,Vec4};
    fn synthetic(count:usize)->Scene {
        let clusters=(0..count).map(|i| {
            let x=(i%17)as f32*0.13;let y=(i%13)as f32*0.11;let z=(i%11)as f32*0.07;
            let band=i/96;
            GpuCluster {cull:[x,y,z,0.2],parent:[x+0.1,y,z,0.4],own:[x,y-0.1,z,0.3],
                parent_error:if band>=3{f32::MAX}else{[0.02,0.2,2.0][band]},
                own_error:[0.,0.02,0.2,2.0][band.min(3)],tri_count:8,..bytemuck::Zeroable::zeroed()}
        }).collect();
        let mut s=Scene::default();s.add_mesh(&ClusterMesh {clusters,..Default::default()});s
    }
    fn unpack(p:&Packed,node:usize)->Node {
        let v=|o:usize|std::array::from_fn(|i|f32::from_bits(p.words[node+o+i]));
        Node {cull:v(0),parent:v(4),own:v(8),max_parent_error:f32::from_bits(p.words[node+12]),min_own_error:f32::from_bits(p.words[node+13]),
            escape:p.words[node+16],first:p.words[node+17],count:p.words[node+18],descendants:p.words[node+19]}
    }
    fn member_ids(p:&Packed,word:usize)->Vec<u32> {
        let end=p.words[word+16]as usize;let mut cursor=word;let mut ids=Vec::new();
        while cursor<end {
            let n=unpack(p,cursor);
            if n.count>0 {ids.extend_from_slice(&p.words[n.first as usize..(n.first+n.count)as usize]);cursor=n.escape as usize;}
            else{cursor+=NODE_WORDS;}
        }ids
    }
    #[test]
    fn packed_stackless_topology_covers_each_cluster_once_and_preserves_source() {
        for count in [64,65,127,128,257,385] {
            let scene=synthetic(count);let before=bytemuck::cast_slice::<GpuCluster,u8>(&scene.clusters).to_vec();
            let p=Packed::new(&scene,true).unwrap();assert_eq!(p.meshes,1);
            let root=p.words[4]as usize;let mut ids=member_ids(&p,root);ids.sort_unstable();assert_eq!(ids,(0..count as u32).collect::<Vec<_>>());
            for word in (p.words[2]as usize..p.words[3]as usize).step_by(NODE_WORDS) {
                let n=unpack(&p,word);assert!(n.count<=LEAF_CLUSTERS as u32);
                assert!(n.escape as usize>word && n.escape<=p.words[3]);
                assert_eq!(member_ids(&p,word).len(),n.descendants as usize);
            }
            assert_eq!(before,bytemuck::cast_slice::<GpuCluster,u8>(&scene.clusters));
            assert_eq!(p.words,Packed::new(&scene,true).unwrap().words);
        }
    }
    #[test]
    fn small_invalid_and_disabled_meshes_stay_flat_without_unchecked_ranges() {
        let mut scene=synthetic(128);scene.clusters[42].own[0]=f32::NAN;
        let p=Packed::new(&scene,true).unwrap();assert_eq!(p.meshes,0);assert_eq!(p.words[4],NONE);
        assert_eq!(Packed::new(&synthetic(63),true).unwrap().meshes,0);
        assert_eq!(Packed::new(&scene,false).unwrap().words,[0,0,0,0]);
        scene.meshes[0].cluster_count=129;assert!(Packed::new(&scene,true).is_err());
    }
    // Independent final selector, expressed in f64. The accelerated node test
    // uses f32 like WGSL. Testing arbitrary valid predicate ranges also covers
    // custom CLOD meshes whose original DAG connections were never retained.
    struct View {xf:Mat4,cam:Vec3,scale:f32,wind:f32,shear:f32,threshold:f32,focal:f32,plane:f32,fog:f32}
    fn exact(c:&GpuCluster,v:&View)->bool {
        let world=|b:[f32;4]|(v.xf.transform_point3(Vec3::from_slice(&b[..3])).as_dvec3(),(b[3]*v.scale+v.wind)as f64);
        let px=|err:f32,b:[f32;4]| {let (p,r)=world(b);err as f64*(v.scale*(1.+v.shear))as f64/(p.distance(v.cam.as_dvec3())-r).max(0.1)*v.focal as f64};
        let (cull,r)=world(c.cull);
        (c.parent_error>1e30 || px(c.parent_error,c.parent)>v.threshold as f64)
            && px(c.own_error,c.own)<=v.threshold as f64
            && cull.x+v.plane as f64>=-r && cull.distance(v.cam.as_dvec3())<=v.fog as f64+r
    }
    fn linear_bound(v:&View)->f32 {
        let a=v.xf.x_axis.truncate();let b=v.xf.y_axis.truncate();let c=v.xf.z_axis.truncate();
        let ab=a.dot(b).abs();let ac=a.dot(c).abs();let bc=b.dot(c).abs();
        v.scale.max((a.length_squared()+ab+ac).max(b.length_squared()+ab+bc).max(c.length_squared()+ac+bc).sqrt())*(1.+1e-5)
    }
    fn world_bound(b:[f32;4],v:&View)->Vec4 {
        let p=v.xf.transform_point3(Vec3::from_slice(&b[..3]));let r=b[3]*linear_bound(v)+v.wind;
        p.extend(r+p.abs().max_element()*2e-6+r*1e-5+1e-4)
    }
    fn reject(n:&Node,v:&View)->bool {
        let c=world_bound(n.cull,v);
        if c.x+v.plane< -c.w || c.truncate().distance(v.cam)>v.fog+c.w{return true;}
        let e=v.scale*(1.+v.shear);
        if n.max_parent_error<=1e30 {
            let p=world_bound(n.parent,v);let upper=n.max_parent_error*e/(p.truncate().distance(v.cam)-p.w).max(0.1)*v.focal;
            if upper>=0. && upper+(upper.abs()*2e-5).max(1e-4)<v.threshold{return true;}
        }
        if n.min_own_error>0. {
            let o=world_bound(n.own,v);let lower=n.min_own_error*e/(o.truncate().distance(v.cam)+o.w).max(0.1)*v.focal;
            if lower<1e30 && lower-(lower.abs()*2e-5).max(1e-4)>v.threshold{return true;}
        }
        false
    }
    fn compare(scene:&Scene,p:&Packed,v:&View)->(usize,usize) {
        let mut flat=scene.clusters.iter().enumerate().filter(|(_,c)|exact(c,v)).map(|(i,_)|i as u32).collect::<Vec<_>>();
        let mut selected=Vec::new();let mut tested=0;let mut skipped=0;
        let mut cursor=p.words[4]as usize;let end=p.words[cursor+16]as usize;
        while cursor<end {
            let n=unpack(p,cursor);
            if reject(&n,v) {
                // Test the mathematical bound against every descendant,
                // not merely the final whole-mesh equality.
                assert!(member_ids(p,cursor).into_iter().all(|i|!exact(&scene.clusters[i as usize],v)),"unsafe node rejection");
                skipped+=n.descendants as usize;cursor=n.escape as usize;
            }else if n.count>0 {
                for &id in &p.words[n.first as usize..(n.first+n.count)as usize] {
                    tested+=1;if exact(&scene.clusters[id as usize],v){selected.push(id);}
                }cursor=n.escape as usize;
            }else{cursor+=NODE_WORDS;}
        }
        flat.sort_unstable();selected.sort_unstable();assert_eq!(flat,selected);assert_eq!(tested+skipped,scene.clusters.len());(tested,skipped)
    }
    fn views()->Vec<View> {
        let mut result=Vec::new();
        for xf in [Mat4::IDENTITY,Mat4::from_rotation_y(0.71)*Mat4::from_scale(Vec3::new(0.3,2.1,1.2)),
            Mat4::from_cols(Vec4::new(2.,0.2,0.,0.),Vec4::new(1.7,0.5,0.1,0.),Vec4::new(0.,0.6,1.2,0.),Vec4::new(1500.,20.,-700.,1.))] {
            let scale=xf.x_axis.truncate().length().max(xf.y_axis.truncate().length()).max(xf.z_axis.truncate().length());
            for distance in [0.05,2.,20.,120.,1200.] {for threshold in [0.,0.1,1.,4.,16.] {for wind in [0.,1.2] {for plane in [-2000.,0.,2000.] {
                result.push(View {xf,cam:xf.w_axis.truncate()+Vec3::new(0.7,1.1,distance),scale,wind,shear:wind*0.4,threshold,focal:960.,plane,fog:900.});
            }}}}
        }result
    }
    #[test]
    fn bounded_nodes_preserve_exact_selection_under_affine_wind_camera_and_lod_changes() {
        let scene=synthetic(385);let packed=Packed::new(&scene,true).unwrap();let mut skipped=0;
        for view in views(){skipped+=compare(&scene,&packed,&view).1;}
        assert!(skipped>10_000,"fixture must exercise substantial pruning, not an always-retain implementation");
    }
    #[test]
    fn authored_tree_clod_cuts_match_flat_selection_without_reconstructing_dag_edges() {
        for bytes in [include_bytes!("../../../assets/fir_a_near.kmesh").as_slice(),include_bytes!("../../../assets/fir_c_near.kmesh").as_slice()] {
            let source=read_kmesh(bytes).unwrap();
            let parts=source.iter().enumerate().map(|(i,(_,vertices,indices))|Part {vertices,indices,material:i as u32}).collect::<Vec<_>>();
            let mesh=ClusterMesh::build_with_policy(&parts,|p|if source[p.material as usize].0.is_empty(){SimplificationPolicy::AlphaCards}else{SimplificationPolicy::Geometry});
            let mut scene=Scene::default();scene.add_mesh(&mesh);let p=Packed::new(&scene,true).unwrap();
            if p.meshes==0 {assert!(scene.clusters.len()<MIN_CLUSTERS);continue;}
            let mut skipped=0;for view in views(){skipped+=compare(&scene,&p,&view).1;}
            assert!(skipped>0);
        }
    }
    #[test]
    fn diagnostics_detect_lost_or_double_visited_clusters() {
        let mut words=[4,20,8,256,768,2,0,1024];assert!(validate_counts(&words).is_ok());
        words[3]-=1;assert!(validate_counts(&words).is_err());words[3]+=2;assert!(validate_counts(&words).is_err());
        words[3]=256;words[2]=21;assert!(validate_counts(&words).is_err());
    }
    #[test]
    fn composed_shader_validates_with_two_phase_and_shadow_bindings_within_budget() {
        let source=crate::frame::cull_source();
        let m=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let n=m.global_variables.iter().filter(|(_,g)|matches!(g.space,naga::AddressSpace::Storage {..})).count();
        assert!(n<=16,"allocation-sized choices must fit portable storage limit: {n}");
        assert_eq!(COUNTER_OFFSET+COUNTER_BYTES,crate::occlusion::SPLIT_COUNTER_OFFSET);
        assert!(!m.global_variables.iter().any(|(_,g)|g.binding.as_ref().is_some_and(|b|b.group==1&&b.binding==13)));
        assert_eq!(LEAF_CLUSTERS,32);assert_eq!(COUNTER_BYTES,32);
        assert!(source.contains("cluster_traverse(id,inst,li,base_thr,wind_pad);"));
    }
}
