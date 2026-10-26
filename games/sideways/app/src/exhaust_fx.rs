//! Bounded exhaust driven by fixed simulation ticks, independent of race RNG.
//! No physics forces, asset mutation, or render-rate random emission.
use glam::{Quat,Vec3};
use render::exhaust::Sprite;
use sideways::race::Race;

const DT:f32=1.0/120.0;
const CARS:usize=4;
const PUFFS_PER_CAR:usize=24;
const FLASH_TICKS:u8=2;
const COOLDOWN_TICKS:u32=48;

/// Tube centres from connected rear underbody components in bundled GLBs.
/// +Z is rear. MV6/Toyama use the full circular cross-section centre because
/// their tilted lip's rearmost vertices alone are not the bore centre.
fn mounts(model:&str)->&'static [[f32;3]] {
    match model.rsplit(['/', '\\']).next().unwrap_or("") {
        "asti_stradale.glb"=>&[[-0.602283,-0.016082,1.859950],[0.602331,-0.023824,1.859949]],
        "mv6.glb"=>&[[-0.064506,-0.004010,1.977772],[0.057851,-0.004010,1.977771]],
        "toyama_xr.glb"=>&[[-0.380331,-0.025942,2.177629]],
        _=>&[], // A new car needs an authored hardpoint; never a bbox guess.
    }
}
fn hash(mut x:u64)->u64 {x=x.wrapping_add(0x9e3779b97f4a7c15);x=(x^(x>>30)).wrapping_mul(0xbf58476d1ce4e5b9);x=(x^(x>>27)).wrapping_mul(0x94d049bb133111eb);x^(x>>31)}
fn unit(x:u64)->f32 {(hash(x)>>40)as f32/16777216.0}

#[derive(Clone)]
struct Input {pos:Vec3,rot:Quat,velocity:Vec3,angular:Vec3,ride:f32,model:String,rpm:f32,limit:f32,throttle:f32,cut:bool,downshift:bool}
impl Input {
    fn origins(&self)->Vec<(Vec3,Vec3)> {
        mounts(&self.model).iter().map(|p| {
            let arm=self.rot*(Vec3::from_array(*p)+Vec3::new(0.0,self.ride,0.015));
            (self.pos+arm,self.velocity+self.angular.cross(arm))
        }).collect()
    }
}
#[derive(Clone)]
struct Puff {pos:Vec3,velocity:Vec3,age:f32,life:f32,radius:f32,alpha:f32,seed:f32}
#[derive(Default)]
struct Emitter {input:Option<Input>,puffs:Vec<Puff>,credit:f32,serial:u64,flash:u8,cooldown:u32,events:u64}
impl Emitter {
    fn advance(&mut self,input:Input,seed:u64,dt:f32) {
        if dt==0.0 {self.input=Some(input);return;}
        self.flash=self.flash.saturating_sub(1);self.cooldown=self.cooldown.saturating_sub(1);
        let old=self.input.as_ref();
        let shift=old.is_some_and(|p|!p.downshift)&&input.downshift&&input.rpm>4300.0;
        let lift=old.is_some_and(|p|p.throttle>0.7)&&input.throttle<0.12&&input.rpm>5800.0;
        let cut=old.is_some_and(|p|!p.cut)&&input.cut&&input.rpm>5800.0;
        if (shift||lift||cut)&&self.cooldown==0&&!mounts(&input.model).is_empty() {
            self.serial=self.serial.wrapping_add(1);
            if unit(seed^self.serial)<if shift{0.65}else{0.35} {
                self.flash=FLASH_TICKS;self.cooldown=COOLDOWN_TICKS;self.events+=1;
            }
        }
        for p in &mut self.puffs {
            p.age+=dt;
            // Inherited vehicle momentum disperses into still air, buoyancy
            // rises slowly. This is a visual approximation, not fluid CFD.
            p.velocity=p.velocity*(-1.7*dt).exp()+Vec3::Y*(0.7*dt);
            p.pos+=p.velocity*dt;
        }
        self.puffs.retain(|p|p.age<p.life);
        let origins=input.origins();
        if input.rpm>400.0&&!origins.is_empty() {
            let load=input.throttle.clamp(0.0,1.0);
            self.credit+=dt*(5.0+load*8.0);
            while self.credit>=1.0 {
                self.credit-=1.0;self.serial=self.serial.wrapping_add(1);
                let key=seed^self.serial;let (pos,velocity)=origins[self.serial as usize%origins.len()];
                let jitter=Vec3::new(unit(key^11)-0.5,unit(key^29)-0.25,unit(key^53)-0.5)*0.14;
                if self.puffs.len()==PUFFS_PER_CAR {self.puffs.remove(0);}
                self.puffs.push(Puff {pos,velocity:velocity+input.rot*Vec3::Z*(0.65+2.0*load)+jitter,
                    age:0.0,life:0.55+unit(key^7)*0.3,radius:0.027+unit(key^3)*0.012,
                    alpha:0.055+load*0.05,seed:unit(key^97)*6.2831855});
            }
        } else {self.credit=0.0;}
        self.input=Some(input);
    }
    fn sprites(&self,time:f32,out:&mut Vec<Sprite>) {
        for p in &self.puffs {
            let age=p.age/p.life;
            out.push(Sprite {position_radius:p.pos.extend(p.radius+p.age*0.19).to_array(),axis_length:[0.0;4],
                colour_alpha:[0.48,0.5,0.53,p.alpha*(1.0-age).powi(2)*(age*12.0).min(1.0)],params:[0.0,p.seed,p.age,0.0]});
        }
        let Some(input)=&self.input else{return};if input.rpm<=400.0{return;}
        let load=input.throttle.clamp(0.0,1.0);let rpm=(input.rpm/input.limit.max(1000.0)).clamp(0.0,1.2);
        let axis=(input.rot*Vec3::Z+Vec3::Y*0.15).normalize();
        for (index,(pos,_)) in input.origins().into_iter().enumerate() {
            out.push(Sprite {position_radius:pos.extend(0.055+load*0.04).to_array(),axis_length:axis.extend(0.3+rpm*0.38+load*0.12).to_array(),colour_alpha:[0.0,0.0,0.0,0.32],params:[1.0,index as f32*2.7,time,0.5+rpm*0.9]});
            if self.flash>0 {
                out.push(Sprite {position_radius:pos.extend(0.045+load*0.025).to_array(),axis_length:(input.rot*Vec3::Z).extend(0.26+rpm*0.17).to_array(),colour_alpha:[8000.0,1800.0,150.0,0.9],params:[2.0,index as f32*2.7,time,0.0]});
            }
        }
    }
}
pub struct ExhaustFx {seed:u64,last:Option<(u32,u64)>,emitters:[Emitter;CARS]}
impl ExhaustFx {
    pub fn new(seed:u64)->Self {Self {seed,last:None,emitters:std::array::from_fn(|_|Emitter::default())}}
    /// Call once after each actual 120 Hz physics tick. Duplicate observations
    /// (pause, screenshots, multiple views) do not age gas or trigger events.
    pub fn step(&mut self,race:&Race) {
        let inputs=race.players.iter().take(CARS).map(|p| {let car=&p.car;let pose=car.pose(&race.phys);let r=pose.rotation;let t=pose.translation;let v=car.linvel(&race.phys);let w=car.angvel(&race.phys);
            Input {pos:Vec3::new(t.x,t.y,t.z),rot:Quat::from_xyzw(r.x,r.y,r.z,r.w),velocity:Vec3::new(v.x,v.y,v.z),angular:Vec3::new(w.x,w.y,w.z),ride:car.spec.ride as f32,model:car.spec.model.clone(),rpm:car.rpm()as f32,limit:car.spec.limit_rpm as f32,throttle:car.throttle as f32,cut:car.cut(),downshift:car.shift_timer>0.0&&car.shift_target>0&&car.shift_target<car.gear}
        }).collect::<Vec<_>>();
        self.observe(race.round,race.frame,&inputs);
    }
    fn observe(&mut self,round:u32,tick:u64,inputs:&[Input]) {
        if self.last==Some((round,tick)){return;}
        let continuous=self.last.is_some_and(|(r,t)|r==round&&t.checked_add(1)==Some(tick));
        if !continuous {self.emitters=std::array::from_fn(|_|Emitter::default());}
        let dt=if continuous{DT}else{0.0};
        for (i,e) in self.emitters.iter_mut().enumerate() {
            if let Some(input)=inputs.get(i) {e.advance(input.clone(),hash(self.seed^(round as u64).rotate_left(17)^i as u64),dt);}else{*e=Emitter::default();}
        }
        self.last=Some((round,tick));
    }
    pub fn sprites(&self)->Vec<Sprite> {let mut out=Vec::with_capacity(CARS*(PUFFS_PER_CAR+4));let time=self.last.map_or(0.0,|(_,t)|(t%432000)as f32*DT);for e in &self.emitters {e.sprites(time,&mut out);}out}
    pub fn metadata(&self)->serde_json::Value {serde_json::json!({"mode":"fixed_tick_measured_tailpipes","tick":self.last,"cars":CARS,"puff_capacity_per_car":PUFFS_PER_CAR,"sprite_capacity":CARS*(PUFFS_PER_CAR+4),"sprites":self.sprites().len(),"backfires":self.emitters.iter().map(|e|e.events).sum::<u64>(),"flash_ms":FLASH_TICKS as f32*DT*1000.0,"physics_unchanged":true,"unknown_model_policy":"disabled_without_authored_mount","limits":["four cars maximum","smoke has sky illumination without volumetric sun shadow","no projected flash light","opaque foreground depth protects heat refraction; transparent foreground is not depth-tested"]})}
}

#[cfg(test)] mod tests {
    use super::*;
    fn input()->Input {Input {pos:Vec3::ZERO,rot:Quat::IDENTITY,velocity:Vec3::ZERO,angular:Vec3::ZERO,ride:0.03,model:"asti_stradale.glb".into(),rpm:6500.0,limit:8000.0,throttle:0.8,cut:false,downshift:false}}
    #[test] fn measured_mounts_follow_body_and_unknown_models_are_disabled(){let mut p=input();let a=p.origins();assert_eq!(a.len(),2);assert!((a[0].0.z-1.87495).abs()<1e-5);p.rot=Quat::from_rotation_y(1.2);p.pos=Vec3::new(3.0,4.0,5.0);p.angular=Vec3::Y*2.0;let b=p.origins();assert!(b[0].0.abs_diff_eq(p.pos+p.rot*a[0].0,1e-5));assert!(b[0].1.abs_diff_eq(p.angular.cross(p.rot*a[0].0),1e-5));p.model="custom.glb".into();assert!(p.origins().is_empty());}
    #[test] fn hardpoints_remain_inside_actual_glb_pipe_cross_sections(){
        for (model,material) in [("asti_stradale.glb","UCB_BOTTOM.001"),("mv6.glb","UCB_BOTTOM_LT.001"),("toyama_xr.glb","Body_Chrome.001")] {
            let path=sideways::game_assets::asset("cars").join(model);
            let bytes=std::fs::read(path).unwrap();let glb=gltf::Gltf::from_slice_without_validation(&bytes).unwrap();let blob=glb.blob.as_deref().unwrap();
            let points:Vec<_>=glb.meshes().flat_map(|mesh|mesh.primitives()).filter(|p|p.material().name()==Some(material)).flat_map(|p|p.reader(|_|Some(blob)).read_positions().unwrap().map(Vec3::from_array)).collect();
            for centre in mounts(model).iter().copied().map(Vec3::from_array) {
                let rim:Vec<_>=points.iter().copied().filter(|p|(p.x-centre.x).abs()<0.065&&(p.y-centre.y).abs()<0.075&&p.z>centre.z-0.12&&p.z<centre.z+0.01).collect();
                assert!(rim.len()>=8,"{model} no measured tube rim");
                let low=rim.iter().copied().fold(Vec3::splat(f32::INFINITY),Vec3::min);let high=rim.iter().copied().fold(Vec3::splat(f32::NEG_INFINITY),Vec3::max);
                assert!(low.x<centre.x&&high.x>centre.x&&low.y<centre.y&&high.y>centre.y,"{model} mount outside bore cross-section");
                assert!((high.z-centre.z).abs()<0.005,"{model} mount not at tube outlet");
            }
        }
    }
    #[test] fn render_calls_and_pause_do_not_advance_emission(){let mut a=ExhaustFx::new(7);let mut b=ExhaustFx::new(7);for tick in 0..360{a.observe(1,tick,&[input()]);b.observe(1,tick,&[input()]);for _ in 0..8{b.observe(1,tick,&[input()]);let _=b.sprites();}assert_eq!(a.sprites(),b.sprites());}assert!(!a.emitters[0].puffs.is_empty());}
    #[test] fn four_car_budget_and_round_reset(){let mut fx=ExhaustFx::new(8);for tick in 0..1200{fx.observe(1,tick,&vec![input();4]);assert!(fx.sprites().len()<=112);assert!(fx.emitters.iter().all(|e|e.puffs.len()<=24));}fx.observe(2,1200,&[input()]);assert!(fx.emitters.iter().all(|e|e.puffs.is_empty()));assert_eq!(fx.sprites().len(),2);}
    #[test] fn flash_requires_engine_edge_and_expires_in_two_ticks(){let seed=(0..100).find(|s|unit(*s^1)<0.65).unwrap();let mut e=Emitter::default();e.advance(input(),seed,0.0);let mut down=input();down.downshift=true;e.advance(down.clone(),seed,DT);assert_eq!(e.flash,2);assert_eq!(e.events,1);e.advance(down.clone(),seed,DT);assert_eq!(e.flash,1);e.advance(down.clone(),seed,DT);assert_eq!(e.flash,0);for _ in 0..120{e.advance(down.clone(),seed,DT);}assert_eq!(e.events,1);}
}
