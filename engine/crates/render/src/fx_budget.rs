//! Visual-only explosion admission. Simulation damage never enters this module.
use super::*;
#[derive(Clone,Copy,Debug)]
pub struct ExplosionBudget {pub cluster_metres:f32,pub pending:usize,pub full_per_tick:usize,pub full_per_window:usize,pub window_seconds:f32,pub live_puffs:usize,pub compact:bool}
impl ExplosionBudget {
    pub const FULL:Self=Self{cluster_metres:3.,pending:13,full_per_tick:3,full_per_window:25,window_seconds:0.5,live_puffs:512,compact:false};
    pub const LOW:Self=Self{pending:8,full_per_tick:2,full_per_window:12,live_puffs:96,..Self::FULL};
    pub const MINIMUM:Self=Self{pending:8,full_per_tick:1,full_per_window:4,live_puffs:12,compact:true,..Self::FULL};
}
#[derive(Clone,Copy,Debug,Default)]
pub struct ExplosionStats {pub requested:u64,pub clustered:u64,pub full:u64,pub cheap:u64,pub omitted:u64}
#[derive(Clone,Copy)]
struct Blast {time:f32,pos:Vec3,radius:f32,color:Vec3}
pub(super) struct BudgetState {settings:ExplosionBudget,pending:Vec<Blast>,window_start:f32,window_full:usize,tick:f32,tick_full:usize,pub stats:ExplosionStats}
impl BudgetState {
    pub(super) fn reset(old:Self)->Self{Self::new(old.settings)}
    pub(super) fn is_empty(&self)->bool{self.pending.is_empty()}
    fn new(settings:ExplosionBudget)->Self{Self{settings,pending:Vec::new(),window_start:f32::NEG_INFINITY,window_full:0,tick:f32::NEG_INFINITY,tick_full:0,stats:Default::default()}}
}
impl Effects {
    pub fn with_explosion_budget(budget:ExplosionBudget)->Self{Self{budget:Some(BudgetState::new(budget)),..Default::default()}}
    pub fn explosion_stats(&self)->ExplosionStats{self.budget.as_ref().map_or(Default::default(),|b|b.stats)}
    pub fn explosion(&mut self,time:f32,pos:Vec3,radius:f32,color:Vec3) {
        if !time.is_finite()||!pos.is_finite()||!radius.is_finite()||!color.is_finite(){return;}
        let Some(b)=self.budget.as_mut() else {self.full_explosion(time,pos,radius,color);return};
        b.stats.requested+=1;
        if let Some(old)=b.pending.iter_mut().find(|p|(p.time-time).abs()<0.02&&p.pos.distance_squared(pos)<=b.settings.cluster_metres*b.settings.cluster_metres) {
            // Keep the first centre but encompass a larger neighbouring blast.
            old.radius=old.radius.max(old.pos.distance(pos)+radius);old.color=old.color.max(color);b.stats.clustered+=1;return;
        }
        if b.pending.len()>=b.settings.pending {b.stats.omitted+=1;return;}
        b.pending.push(Blast{time,pos,radius:radius.max(0.5),color});
    }
    pub(super) fn emit_pending(&mut self,time:f32) {
        let Some(mut b)=self.budget.take() else{return};
        let before=self.puffs.len();self.puffs.retain(|p|p.end()>time);self.changed|=self.puffs.len()!=before;self.pops.retain(|p|time-p.birth<2.);
        if time<b.window_start||time-b.window_start>=b.settings.window_seconds {b.window_start=time;b.window_full=0;}
        for blast in b.pending.drain(..) {
            if blast.time!=b.tick {b.tick=blast.time;b.tick_full=0;}
            let full=b.tick_full<b.settings.full_per_tick&&b.window_full<b.settings.full_per_window;
            if full {b.tick_full+=1;b.window_full+=1;b.stats.full+=1;}else{b.stats.cheap+=1;}
            if full&&!b.settings.compact {self.full_explosion(blast.time,blast.pos,blast.radius,blast.color);}
            else {
                if full {self.billows(blast.time,blast.pos,blast.radius,1,false,0.65,0.6,1.0,blast.radius*0.3,1.,0.,Vec3::ONE,0.55);}
                self.billows(blast.time,blast.pos,blast.radius,if full{2}else{1},true,0.55,0.6,1.2,blast.radius*0.12,1.,0.,Vec3::ONE,0.9);
                self.pops.push(LightPop{pos:blast.pos,birth:blast.time,kind:0,radius:blast.radius,color:blast.color});self.changed=true;
            }
        }
        // Budget decorative smoke/fire/embers only. Never evict ion warning columns.
        // Over budget, drop the puffs nearest their end first: a new blast
        // must not wipe a smoke trail that still has seconds to fade (a
        // Growing Guns air strike trail is ~200 puffs, its blast ~140).
        let excess=self.puffs.iter().filter(|p|p.kind()<=3).count().saturating_sub(b.settings.live_puffs);
        if excess>0 {
            let mut ends:Vec<(f32,usize)>=self.puffs.iter().enumerate().filter(|(_,p)|p.kind()<=3).map(|(i,p)|(p.end(),i)).collect();
            ends.sort_by(|a,b|a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mut drop=vec![false;self.puffs.len()];
            for &(_,i) in &ends[..excess] {drop[i]=true;}
            let mut i=0;self.puffs.retain(|_|{i+=1;!drop[i-1]});self.changed=true;
        }
        if self.pops.len()>32 {self.pops.drain(..self.pops.len()-32);}
        self.budget=Some(b);
    }
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn overlapping_chain_is_clustered_and_sustained_spam_stays_bounded(){
        let mut fx=Effects::with_explosion_budget(ExplosionBudget::MINIMUM);
        fx.column(AdditiveColumn{life:10.,..Default::default()}).unwrap();
        for i in 0..1000 {fx.explosion(0.,Vec3::new((i%3) as f32*0.5,0.,0.),5.,Vec3::ONE);}
        fx.emit_pending(0.01);assert_eq!(fx.explosion_stats().clustered,999);assert_eq!(fx.explosion_stats().full,1);
        for tick in 1..100 {let t=tick as f32/60.;for i in 0..100 {fx.explosion(t,Vec3::new(i as f32*4.,0.,0.),5.,Vec3::ONE);}fx.emit_pending(t);
            assert!(fx.puffs.iter().filter(|p|p.kind()<=3).count()<=12);assert!(fx.pops.len()<=32);assert_eq!(fx.puffs.iter().filter(|p|p.kind()==4).count(),1);}
        assert!(fx.explosion_stats().omitted>0);assert!(fx.explosion_stats().cheap>0);assert!(fx.explosion_stats().full<=16);
    }
    #[test]fn a_rocket_trail_outlives_its_own_blast(){
        // Growing Guns: up to 3 exhaust puffs every 2 ticks for 2.3 s, then the radius-30 blast.
        let mut fx=Effects::with_explosion_budget(ExplosionBudget::FULL);
        let mut t=0.;
        for tick in 0..138 {t=tick as f32/60.;if tick%2==0{for k in 0..3{fx.exhaust_smoke(t,Vec3::new(0.,200.-tick as f32-k as f32*0.3,0.),5.);}}fx.emit_pending(t);}
        let trail=fx.puffs.len();assert!(trail>150,"trail {trail}");
        fx.explosion(t,Vec3::ZERO,30.,Vec3::ONE);fx.emit_pending(t+1./60.);
        assert_eq!(fx.puffs.iter().filter(|p|p.kind()==SMOKE as u32&&p.center_kind[1]>60.).count()>=trail-1,true,"the trail survives the blast");
    }
}
