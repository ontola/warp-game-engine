//! Bounded exhaust driven by fixed simulation ticks, independent of race RNG.
//! No physics forces, asset mutation, or render-rate random emission.
use glam::{Quat,Vec3};
use render::exhaust::Sprite;
use render::headlights::LocalLight;
use sideways::race::Race;

const DT:f32=1.0/120.0;
const CARS:usize=4;
const PUFFS_PER_CAR:usize=24;
const FLASH_TICKS:u8=2;
const COOLDOWN_TICKS:u32=48;
const MAX_OUTLET_MOTION_M:f32=0.003;
const FLASH_INTENSITY_CD:Vec3=Vec3::new(160.0,36.0,3.0);
const FLASH_RANGE_M:f32=2.0;

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
        self.moving_origins(Vec3::ZERO)
    }
    fn moving_origins(&self,offset:Vec3)->Vec<(Vec3,Vec3)> {
        mounts(&self.model).iter().map(|p| {
            let arm=self.rot*(Vec3::from_array(*p)+Vec3::new(0.0,self.ride,0.015)+offset);
            (self.pos+arm,self.velocity+self.angular.cross(arm))
        }).collect()
    }
}
#[derive(Clone)]
struct Puff {pos:Vec3,velocity:Vec3,age:f32,life:f32,radius:f32,alpha:f32,seed:f32}
#[derive(Default)]
struct Emitter {input:Option<Input>,puffs:Vec<Puff>,credit:f32,serial:u64,flash:u8,cooldown:u32,events:u64,phase:f32}
impl Emitter {
    // A bounded visual engine pulse, not physical body/pipe animation. It is
    // integrated at simulation cadence so RPM changes cannot teleport phase.
    fn outlet_motion(&self,input:&Input)->Vec3 {
        if input.rpm<=400.0 {return Vec3::ZERO;}
        let load=input.throttle.clamp(0.0,1.0);
        let rpm=(input.rpm/input.limit.max(1000.0)).clamp(0.0,1.0);
        let amplitude=0.001+0.0015*load+0.0005*rpm;
        Vec3::new(0.55*self.phase.sin(),0.75*(2.0*self.phase+0.4).sin(),0.30*(self.phase+0.8).sin())*amplitude
    }
    fn local_axis(&self)->Vec3 {
        Vec3::new(0.012*self.phase.sin(),0.008*(2.0*self.phase+0.3).sin(),1.0).normalize()
    }
    fn flash_energy(&self)->f32 {match self.flash {0=>0.0,1=>0.6,_=>1.0}}
    fn advance(&mut self,input:Input,seed:u64,dt:f32) {
        if self.input.is_none() {self.phase=unit(seed^0xe11a57)*std::f32::consts::TAU;}
        if dt==0.0 {self.input=Some(input);return;}
        if input.rpm>400.0 {
            let hz=6.0+input.rpm.clamp(0.0,12000.0)/600.0;
            self.phase=(self.phase+std::f32::consts::TAU*hz*dt).rem_euclid(std::f32::consts::TAU);
        }
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
        let origins=input.moving_origins(self.outlet_motion(&input));
        if input.rpm>400.0&&!origins.is_empty() {
            let load=input.throttle.clamp(0.0,1.0);
            self.credit+=dt*(5.0+load*8.0);
            while self.credit>=1.0 {
                self.credit-=1.0;self.serial=self.serial.wrapping_add(1);
                let key=seed^self.serial;let (pos,velocity)=origins[self.serial as usize%origins.len()];
                let jitter=Vec3::new(unit(key^11)-0.5,unit(key^29)-0.25,unit(key^53)-0.5)*0.14;
                if self.puffs.len()==PUFFS_PER_CAR {self.puffs.remove(0);}
                self.puffs.push(Puff {pos,velocity:velocity+input.rot*self.local_axis()*(0.65+2.0*load)+jitter,
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
        let rear=input.rot*self.local_axis();
        let axis=(rear+Vec3::Y*0.15).normalize();let pulse=1.0+0.07*self.phase.sin();
        for (index,(pos,_)) in input.moving_origins(self.outlet_motion(input)).into_iter().enumerate() {
            out.push(Sprite {position_radius:pos.extend(0.055+load*0.04).to_array(),axis_length:axis.extend((0.3+rpm*0.38+load*0.12)*pulse).to_array(),colour_alpha:[0.0,0.0,0.0,0.32],params:[1.0,index as f32*2.7,time,(0.5+rpm*0.9)*pulse]});
            if self.flash>0 {
                out.push(Sprite {position_radius:pos.extend(0.045+load*0.025).to_array(),axis_length:rear.extend(0.26+rpm*0.17).to_array(),colour_alpha:[8000.0,1800.0,150.0,0.9*self.flash_energy()],params:[2.0,index as f32*2.7,time,0.0]});
            }
        }
    }
    fn light(&self)->Option<LocalLight> {
        let input=self.input.as_ref()?;
        if self.flash==0 || input.rpm<=400.0 {return None;}
        let origins=input.moving_origins(self.outlet_motion(input));
        if origins.is_empty() {return None;}
        // One bounded light approximates both pipes. Its source lies inside
        // the flame, just behind the measured outlets, never in the engine.
        let rear=input.rot*self.local_axis();
        let position=origins.iter().map(|(p,_)|*p).sum::<Vec3>()/origins.len()as f32+rear*0.08;
        Some(LocalLight {position,direction:(rear-input.rot*Vec3::Y*0.12).normalize(),
            intensity_cd:FLASH_INTENSITY_CD*self.flash_energy(),range_m:FLASH_RANGE_M,
            inner_cos:45.0f32.to_radians().cos(),outer_cos:70.0f32.to_radians().cos(),source_radius_m:0.05})
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
    /// Event-local rear spots: the host applies the exhaust visibility gate and
    /// uploads this list with other local lights. No shadowed headlamp slots.
    pub fn lights(&self)->Vec<LocalLight> {self.emitters.iter().filter_map(Emitter::light).collect()}
    pub fn metadata(&self)->serde_json::Value {serde_json::json!({"mode":"fixed_tick_measured_tailpipes","tick":self.last,"cars":CARS,"puff_capacity_per_car":PUFFS_PER_CAR,"sprite_capacity":CARS*(PUFFS_PER_CAR+4),"sprites":self.sprites().len(),"backfires":self.emitters.iter().map(|e|e.events).sum::<u64>(),"flash_ms":FLASH_TICKS as f32*DT*1000.0,"physics_unchanged":true,"unknown_model_policy":"disabled_without_authored_mount","engine_motion":{"scope":"body-local emitted gas/flame and heat only; no body mesh, camera or physics motion","maximum_outlet_motion_m":MAX_OUTLET_MOTION_M,"clock":"fixed120Hz,RPM-integrated visual pulse"},"flash_lights":{"scope":"emitter requests before host exhaust gate; one approximate rear spot per car, not one per pipe","range_m":FLASH_RANGE_M,"peak_intensity_cd":FLASH_INTENSITY_CD.to_array(),"lights":self.lights().iter().map(|l|serde_json::json!({"position":l.position.to_array(),"direction":l.direction.to_array(),"intensity_cd":l.intensity_cd.to_array(),"range_m":l.range_m,"inner_cos":l.inner_cos,"outer_cos":l.outer_cos,"source_radius_m":l.source_radius_m})).collect::<Vec<_>>()},"limits":["four cars maximum","smoke has sky illumination without volumetric sun shadow","flash light is a short-range rear-cone approximation, no volumetric beam","opaque foreground depth protects heat refraction; transparent foreground is not depth-tested"]})}
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
    #[test] fn outlet_motion_is_bounded_and_follows_full_body_pose() {
        let mut p=input();p.throttle=1.0;p.rpm=p.limit;
        let mut e=Emitter::default();let q=Quat::from_euler(glam::EulerRot::YXZ,0.8,0.4,-0.3);
        for i in 0..2048 {
            e.phase=i as f32*std::f32::consts::TAU/2048.0;
            let offset=e.outlet_motion(&p);assert!(offset.length()<=MAX_OUTLET_MOTION_M);
            assert!(e.local_axis().dot(Vec3::Z)>1.0f32.to_radians().cos());
            let base=p.moving_origins(offset);let mut rotated=p.clone();rotated.rot=q;rotated.pos=Vec3::new(3.0,4.0,-7.0);
            for (a,b) in base.iter().zip(rotated.moving_origins(offset)) {
                assert!(b.0.abs_diff_eq(rotated.pos+q*a.0,1e-5));
            }
        }
        p.rpm=0.0;assert_eq!(e.outlet_motion(&p),Vec3::ZERO);
    }
    #[test] fn pulse_phase_tracks_rpm_only_on_actual_ticks() {
        let mut low=input();low.rpm=1000.0;let mut high=low.clone();high.rpm=7000.0;
        let mut a=Emitter::default();let mut b=Emitter::default();
        a.advance(low.clone(),7,0.0);b.advance(high.clone(),7,0.0);
        let start=a.phase;assert_eq!(start,b.phase);
        a.advance(low.clone(),7,DT);b.advance(high.clone(),7,DT);
        let da=(a.phase-start).rem_euclid(std::f32::consts::TAU);
        let db=(b.phase-start).rem_euclid(std::f32::consts::TAU);
        assert!(db>da);assert!((da-(6.0+1000.0/600.0)*std::f32::consts::TAU*DT).abs()<1e-6);
        let held=a.phase;a.advance(low,7,0.0);assert_eq!(held,a.phase);
        let mut stopped=input();stopped.rpm=0.0;a.advance(stopped,7,DT);assert_eq!(held,a.phase);
    }
    #[test] fn flash_light_and_entire_flame_share_event_and_decay() {
        let seed=(0..100).find(|s|unit(*s^1)<0.65).unwrap();let mut e=Emitter::default();
        e.advance(input(),seed,0.0);assert!(e.light().is_none());let mut down=input();down.downshift=true;
        for energy in [1.0,0.6,0.0] {
            e.advance(down.clone(),seed,DT);let mut sprites=Vec::new();e.sprites(0.0,&mut sprites);
            let flames:Vec<_>=sprites.iter().filter(|s|s.params[0]==2.0).collect();
            if energy==0.0 {assert!(e.light().is_none());assert!(flames.is_empty());continue;}
            let light=e.light().unwrap();assert_eq!(flames.len(),2);
            assert!(light.intensity_cd.abs_diff_eq(FLASH_INTENSITY_CD*energy,1e-6));
            assert!(flames.iter().all(|s|(s.colour_alpha[3]-0.9*energy).abs()<1e-6));
            let midpoint=flames.iter().map(|s|Vec3::from_slice(&s.position_radius[..3])).sum::<Vec3>()/2.0;
            assert!(light.position.abs_diff_eq(midpoint+Vec3::from_slice(&flames[0].axis_length[..3])*0.08,1e-6));
            assert!((light.direction.length()-1.0).abs()<1e-6);assert_eq!(light.range_m,2.0);
            assert!(light.inner_cos>light.outer_cos&&light.outer_cos>0.0);
        }
        assert_eq!(e.events,1);
    }
    #[test] fn flash_lights_are_four_car_bounded_and_cannot_survive_reset_or_unknown_model() {
        let mut fx=ExhaustFx::new(4);fx.observe(1,0,&vec![input();CARS]);
        for e in &mut fx.emitters {e.flash=2;}
        assert_eq!(fx.lights().len(),CARS);let positions:Vec<_>=fx.lights().iter().map(|l|l.position).collect();
        fx.observe(1,0,&vec![input();CARS]);assert_eq!(positions,fx.lights().iter().map(|l|l.position).collect::<Vec<_>>());
        fx.emitters[0].input.as_mut().unwrap().model="custom.glb".into();assert_eq!(fx.lights().len(),CARS-1);
        fx.observe(2,0,&vec![input();CARS]);assert!(fx.lights().is_empty());
    }
}
