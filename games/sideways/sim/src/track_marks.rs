//! Visual-only tyre contact trails. No simulation state or race RNG is changed.
use crate::{surfaces::Surface, wheel_fx::Contact};

pub const MAX_WHEELS: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: [f32; 3], pub end: [f32; 3],
    pub start_normal: [f32; 3], pub end_normal: [f32; 3],
    pub width: f32, pub compaction: f32, pub rubber: f32, pub surface: Surface,
}
#[derive(Clone, Copy)]
struct Previous { contact: Contact, tick: u64 }
pub struct Emitter {
    previous: [Option<Previous>; MAX_WHEELS],
    widths: [f32; MAX_WHEELS], last_tick: Option<u64>, segments: Vec<Segment>,
}
impl Default for Emitter { fn default()->Self {Self::new()} }
impl Emitter {
    pub fn new()->Self {Self {previous:[None;MAX_WHEELS],widths:[0.24;MAX_WHEELS],last_tick:None,segments:Vec::with_capacity(MAX_WHEELS)}}
    pub fn reset(&mut self) {self.previous.fill(None);self.last_tick=None;self.segments.clear();}
    pub fn reset_wheel(&mut self,id:u8) {if let Some(p)=self.previous.get_mut(id as usize) {*p=None;}}
    pub fn set_wheel_width(&mut self,id:u8,width:f32) {if width.is_finite() {if let Some(w)=self.widths.get_mut(id as usize) {*w=width.clamp(0.12,0.6);}}}
    /// Sample once per physics tick. Missing contacts, air, water, recovery and
    /// skipped ticks break continuity. A paused/repeated tick emits nothing.
    pub fn tick(&mut self,tick:u64,contacts:&[Contact],wetness:f32)->&[Segment] {
        self.segments.clear();
        if self.last_tick==Some(tick) {return &self.segments;}
        if self.last_tick.is_some_and(|last|last.checked_add(1)!=Some(tick)) {self.previous.fill(None);}
        self.last_tick=Some(tick);
        let wet=if wetness.is_finite(){wetness.clamp(0.,1.)}else{0.};
        let mut samples=[None;MAX_WHEELS];
        for &c in contacts {let id=c.id as usize;if id<MAX_WHEELS&&samples[id].is_none(){samples[id]=Some(c);}}
        for (id,sample) in samples.into_iter().enumerate() {
            let Some(c)=sample.filter(|c|valid(*c)) else {self.previous[id]=None;continue};
            let Some(previous)=self.previous[id] else {self.previous[id]=Some(Previous{contact:c,tick});continue};
            let old=previous.contact;
            let d2=(0..3).map(|k|(c.point[k]-old.point[k]).powi(2)).sum::<f32>();
            // A two-metre step in 1/120 s is a recovery/discontinuity, not a
            // tyre stroke. Also avoid joining incompatible bank normals.
            if d2>4. || c.surface!=old.surface || dot(c.normal,old.normal)<0.5 {
                self.previous[id]=Some(Previous{contact:c,tick});continue;
            }
            let slip=(c.tread_speed-c.forward_speed).hypot(c.lateral_speed).max(c.slide.max(0.)*2.);
            if d2<0.0009 && !(slip>0.8&&tick.saturating_sub(previous.tick)>=12) {continue;}
            let load=(c.load/2500.).clamp(0.,1.);
            let rolling=(c.forward_speed.hypot(c.lateral_speed)/0.5).clamp(0.,1.);
            let skid=((slip-0.5)/5.).clamp(0.,1.);
            let (compaction,rubber)=match c.surface {
                Surface::Asphalt=>(0.,skid*load*(1.-wet*0.45)),
                Surface::Mud=>(load*(0.55*rolling+0.4*skid),0.),
                Surface::Grass=>(load*(0.45*rolling+0.4*skid),0.),
                Surface::Loose|Surface::Dirt=>(load*(0.5*rolling+0.35*skid),0.),
                _=>(load*(0.4*rolling+0.4*skid)*(1.+wet*0.15),0.),
            };
            if compaction>0.001||rubber>0.001 {self.segments.push(Segment {start:old.point,end:c.point,
                start_normal:old.normal,end_normal:c.normal,width:self.widths[id],compaction:compaction.min(1.),rubber:rubber.min(1.),surface:c.surface});}
            self.previous[id]=Some(Previous{contact:c,tick});
        }
        &self.segments
    }
}
fn dot(a:[f32;3],b:[f32;3])->f32 {(0..3).map(|k|a[k]*b[k]).sum()}
fn valid(c:Contact)->bool {
    c.grounded&&c.load.is_finite()&&c.load>1.&&c.surface!=Surface::Water
        &&c.point.into_iter().chain(c.normal).all(f32::is_finite)
        &&[c.forward_speed,c.lateral_speed,c.tread_speed,c.slide].into_iter().all(f32::is_finite)
        &&dot(c.normal,c.normal)>0.5&&c.normal[1]>0.35
}
#[cfg(test)]
mod tests {
    use super::*;
    fn contact(id:u8,x:f32)->Contact {Contact{id,grounded:true,surface:Surface::Gravel,point:[x,0.,0.],normal:[0.,1.,0.],inherited_velocity:[12.,0.,0.],forward:[1.,0.,0.],right:[0.,0.,1.],forward_speed:12.,lateral_speed:0.,tread_speed:12.,load:3000.,slide:0.}}
    #[test] fn continuity_breaks_on_air_water_reset_and_time_cuts() {
        let mut e=Emitter::new();assert!(e.tick(0,&[contact(0,0.)],0.).is_empty());
        let s=e.tick(1,&[contact(0,0.1)],0.);assert_eq!(s.len(),1);assert_eq!(s[0].start,[0.,0.,0.]);
        let mut air=contact(0,0.2);air.grounded=false;assert!(e.tick(2,&[air],0.).is_empty());
        assert!(e.tick(3,&[contact(0,0.3)],0.).is_empty());assert_eq!(e.tick(4,&[contact(0,0.4)],0.).len(),1);
        assert!(e.tick(4,&[contact(0,0.4)],0.).is_empty());e.reset_wheel(0);
        assert!(e.tick(5,&[contact(0,0.5)],0.).is_empty());assert!(e.tick(8,&[contact(0,0.8)],0.).is_empty());
        let mut water=contact(0,0.9);water.surface=Surface::Water;assert!(e.tick(9,&[water],0.).is_empty());
        assert!(e.tick(10,&[contact(0,1.)],0.).is_empty());assert!(e.tick(11,&[contact(0,50.)],0.).is_empty());
        e.reset();assert!(e.tick(0,&[contact(0,0.)],0.).is_empty());
    }
    #[test] fn four_players_bound_and_surface_slip_policy() {
        let mut e=Emitter::new();let a:Vec<_>=(0..16).map(|i|contact(i,i as f32)).collect();e.tick(0,&a,0.);
        let b:Vec<_>=a.iter().map(|c|Contact{point:[c.point[0]+0.1,0.,0.],..*c}).collect();assert_eq!(e.tick(1,&b,0.).len(),16);
        e.reset();let mut c=contact(0,0.);c.surface=Surface::Asphalt;e.tick(0,&[c],0.);c.point[0]=0.1;assert!(e.tick(1,&[c],0.).is_empty());
        c.lateral_speed=4.;c.point[0]=0.2;let s=e.tick(2,&[c],0.);assert_eq!(s.len(),1);assert!(s[0].rubber>0.);assert_eq!(s[0].compaction,0.);
    }
    #[test] fn stationary_burnout_is_bounded_and_input_order_independent() {
        let mut e=Emitter::new();let mut c=contact(0,0.);c.forward_speed=0.;c.tread_speed=20.;
        let mut count=0;for tick in 0..120 {count+=e.tick(tick,&[c],0.).len();}assert_eq!(count,9);
        let mut a=Emitter::new();let mut b=Emitter::new();for t in 0..20 {let cs:Vec<_>=(0..16).map(|i|contact(i,t as f32*0.1)).collect();let mut reverse=cs.clone();reverse.reverse();assert_eq!(a.tick(t,&cs,0.),b.tick(t,&reverse,0.));}
    }
}
