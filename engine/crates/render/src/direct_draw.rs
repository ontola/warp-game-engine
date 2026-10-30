//! Conservative direct-draw bounds for drivers with broken indirect drawing.
//! Counts include every LOD of every resident owner; the vertex shader rejects
//! excess instances against the current GPU-selected count. No frame readback.
use crate::scene::{Scene,GpuInstance};
use std::collections::HashMap;

pub(crate) struct Bounds {
    meshes:HashMap<(u32,u32),[u32;4]>,
    owners:Vec<[u32;4]>,
    totals:[u32;4],
    debris:Option<(usize,usize)>,
}
fn bucket(triangles:u32,small:u32)->usize {if triangles<=small {0}else if triangles<=32 {1}else if triangles<=64 {2}else{3}}
impl Bounds {
    pub fn new(scene:&Scene,small:u32)->Self {
        let meshes=scene.meshes.iter().map(|m|{
            let mut counts=[0;4];
            for c in &scene.clusters[m.first_cluster as usize..(m.first_cluster+m.cluster_count)as usize] {counts[bucket(c.tri_count,small)]+=1;}
            ((m.first_cluster,m.cluster_count),counts)
        }).collect();
        let mut result=Self {meshes,owners:vec![[0;4];scene.instances.len()],totals:[0;4],debris:scene.debris.as_ref().map(|d|(d.first as usize,d.capacity as usize))};
        result.update(0,&scene.instances).expect("scene mesh ranges");result
    }
    fn set(&mut self,owner:usize,first:u32,count:u32)->Result<(),String> {
        let counts=if count==0 {[0;4]}else{*self.meshes.get(&(first,count)).ok_or("direct drawing requires a registered mesh range")?};
        let previous=self.owners.get_mut(owner).ok_or("direct draw owner outside scene")?;
        for k in 0..4 {self.totals[k]=self.totals[k]-previous[k]+counts[k];}
        *previous=counts;Ok(())
    }
    pub fn update(&mut self,first:usize,instances:&[GpuInstance])->Result<(),String> {
        if first.checked_add(instances.len()).is_none_or(|end|end>self.owners.len()) {
            return Err("direct draw owner outside scene".into());
        }
        for inst in instances {
            if inst.cluster_count!=0 && !self.meshes.contains_key(&(inst.first_cluster,inst.cluster_count)) {
                return Err("direct drawing requires a registered mesh range".into());
            }
        }
        for (i,inst) in instances.iter().enumerate() {self.set(first+i,inst.first_cluster,inst.cluster_count)?;}
        Ok(())
    }
    pub fn shards(&mut self,spawns:&[crate::debris::Shard],reset:bool)->Result<(),String> {
        if let Some((first,count))=self.debris {
            for s in spawns {
                if s.mesh[3] as usize>=count {return Err("direct draw debris slot outside pool".into());}
                if s.mesh[1]!=0 && !self.meshes.contains_key(&(s.mesh[0],s.mesh[1])) {
                    return Err("direct draw debris mesh outside scene".into());
                }
            }
            if reset {for owner in first..first+count {self.set(owner,0,0)?;}}
            for s in spawns {self.set(first+s.mesh[3]as usize,s.mesh[0],s.mesh[1])?;}
        }
        Ok(())
    }
    pub fn counts(&self,buckets:bool)->[u32;4] {if buckets {self.totals}else{[self.totals.iter().sum(),0,0,0]}}
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn bounds_cover_repeated_meshes_updates_and_empty_slots() {
        let mut scene=Scene::default();
        let mesh=scene.add_mesh(&crate::primitives::cube(0).mesh());
        for _ in 0..4 {scene.add_instance(mesh,glam::Affine3A::IDENTITY);}
        let mut bounds=Bounds::new(&scene,8);
        assert_eq!(bounds.counts(true),[0,4,0,0]);
        scene.instances[2].cluster_count=0;
        bounds.update(2,&scene.instances[2..3]).unwrap();
        assert_eq!(bounds.counts(false),[3,0,0,0]);
        assert!(bounds.update(4,&scene.instances[..1]).is_err());
        scene.instances[2].cluster_count=1;
        bounds.update(2,&scene.instances[2..3]).unwrap();
        assert_eq!(bounds.counts(true),[0,4,0,0]);
    }
    #[test] fn every_triangle_count_fits_its_draw_bucket() {
        for small in [8,12] {for triangles in 1..=128 {let vertices=[small*3,96,192,384][bucket(triangles,small)];assert!(vertices>=triangles*3);}}
    }
}
