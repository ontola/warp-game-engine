//! Stable world population bookkeeping, never camera visibility or detail selection.
use crate::foliage_runtime::RootPlacement;
use std::collections::{BTreeSet,HashMap};

#[derive(Clone,Copy,Debug,PartialEq)]
pub struct WorldRoot {pub id:u64,pub placement:RootPlacement}

#[derive(Clone,Debug)]
pub(crate) struct Population {
    pub slots:Vec<Option<WorldRoot>>,pub generations:Vec<u32>,ids:HashMap<u64,usize>,
}
pub(crate) struct Update {pub next:Population,pub changed:Vec<usize>,pub retained:usize}
impl Population {
    pub fn new(roots:&[WorldRoot],capacity:usize)->Result<Self,String> {
        if capacity==0 || capacity>=1<<19 || roots.len()>capacity {return Err("forest root capacity exceeded".into());}
        let empty=Self {slots:vec![None;capacity],generations:vec![0;capacity],ids:HashMap::new()};
        Ok(empty.plan(roots)?.next)
    }
    /// A snapshot of loaded world cells, not the camera's visible trees.
    /// Validation/planning is transactional; rejected input changes no live IDs.
    pub fn plan(&self,roots:&[WorldRoot])->Result<Update,String> {
        if roots.len()>self.slots.len(){return Err("forest root capacity exceeded".into());}
        let mut wanted=HashMap::with_capacity(roots.len());
        for root in roots {
            let p=root.placement;
            if !p.pos.is_finite() || !p.yaw.is_finite() || !p.scale.is_finite() || p.scale<0.0001
                || wanted.insert(root.id,root).is_some() {return Err("invalid or duplicate forest world root".into());}
        }
        let mut next=self.clone();let mut changed=BTreeSet::new();let mut retained=0;
        for (&id,&slot) in &self.ids {
            if !wanted.contains_key(&id) {next.slots[slot]=None;next.ids.remove(&id);changed.insert(slot);}
        }
        let mut free=(0..next.slots.len()).rev().filter(|&i|next.slots[i].is_none()).collect::<Vec<_>>();
        for &root in roots {
            let slot=if let Some(&slot)=next.ids.get(&root.id) {slot}else {
                let slot=free.pop().ok_or("forest has no free root slot")?;next.ids.insert(root.id,slot);slot
            };
            if self.slots[slot]==Some(root) {retained+=1;}else {changed.insert(slot);}
            next.slots[slot]=Some(root);
        }
        for &slot in &changed {
            next.generations[slot]=self.generations[slot].checked_add(1).ok_or("forest root generation exhausted")?;
        }
        Ok(Update {next,changed:changed.into_iter().collect(),retained})
    }
    pub fn active(&self)->usize {self.ids.len()}
}

#[cfg(test)] mod tests {
    use super::*;use glam::Vec3;
    fn root(id:u64)->WorldRoot {WorldRoot {id,placement:RootPlacement {pos:Vec3::new(id as f32,0.,0.),yaw:0.,scale:1.,variant:0}}}
    #[test] fn reorder_replace_delete_and_reuse_keep_identity() {
        let population=Population::new(&[root(10),root(20)],3).unwrap();
        let reorder=population.plan(&[root(20),root(10)]).unwrap();assert!(reorder.changed.is_empty());assert_eq!(reorder.retained,2);
        let update=population.plan(&[root(20),root(30)]).unwrap();assert_eq!(update.changed,vec![0]);
        assert_eq!(update.next.slots[1],population.slots[1]);assert_eq!(update.next.generations[1],population.generations[1]);
        assert_eq!(update.next.generations[0],population.generations[0]+1);
        let empty=update.next.plan(&[]).unwrap();assert_eq!(empty.changed,vec![0,1]);assert_eq!(empty.next.active(),0);
        let back=empty.next.plan(&[root(10)]).unwrap();assert!(back.next.generations[0]>population.generations[0]);
    }
    #[test] fn rejected_batch_is_transactional_and_moves_invalidate_generation() {
        let population=Population::new(&[root(10)],1).unwrap();
        assert!(population.plan(&[root(10),root(20)]).is_err());
        let spare=Population::new(&[root(10)],3).unwrap();
        assert!(spare.plan(&[root(10),root(10)]).is_err());
        let mut invalid=root(20);invalid.placement.pos.x=f32::NAN;assert!(spare.plan(&[invalid]).is_err());
        let mut moved=root(10);moved.placement.pos.x+=1.;let update=population.plan(&[moved]).unwrap();
        assert_eq!(update.changed,vec![0]);assert_eq!(population.slots[0],Some(root(10)));
        assert_eq!(update.next.generations[0],population.generations[0]+1);
    }
}
