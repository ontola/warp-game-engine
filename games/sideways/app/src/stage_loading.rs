//! Next-stage construction off the window thread. The displayed world and
//! simulation stay unchanged until a complete replacement is ready.
use super::{cameras, lighting, poses};
use crate::{car_look::Looks, gamenight::Seat, procedural_sky::PreparedSky, visuals::{self, Built}};
use gdcompat::Pcg;
use kern::Timings;
use render::{Gpu, Lighting, Renderer};
use sideways::{physics::Physics, race::{Race, Settings, Stats}, spec::Cars, stage::Stage, surfaces::Surfaces};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Instant;

pub struct Seed {
    settings:Settings, rng:Pcg, cars:Cars, round:u32, frame:u64, stats:Stats, events:Vec<String>,
}
impl Seed {
    pub fn next(race:&Race,settings:Settings)->Self {
        Self {settings,rng:race.rng,cars:race.cars.clone(),round:race.round,frame:race.frame,
            stats:race.stats.clone(),events:race.events.clone()}
    }
    /// The next stage drives these cars (data/cars.luau read again).
    pub fn with_cars(mut self,cars:Cars)->Self {self.cars=cars;self}
    fn shell(self)->Race {
        // A tiny placeholder uses its own RNG. The real stream is preserved
        // bit-for-bit and new_stage consumes it once, just as the old path did.
        Race {stage_origin:None,settings:self.settings,rng:self.rng,cars:self.cars,
            stage:Stage::generate(&mut Pcg::new(0),10.0),phys:Physics::new(),surfaces:Surfaces::default(),
            weather:"clear",players:vec![],phase:"results",clock:0.0,results:0.0,first_finish:-1.0,
            round:self.round,frame:self.frame,stats:self.stats,events:self.events}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { WaitingForDisplay, Race, Visuals, Graphics, Upload, Ready }
impl Phase {
    fn label(self)->&'static str {match self {
        Self::WaitingForDisplay=>"Preparing next stage",Self::Race=>"Building route and collisions",
        Self::Visuals=>"Building landscape and vegetation",Self::Graphics=>"Preparing graphics",
        Self::Upload=>"Uploading graphics",Self::Ready=>"Starting stage",
    }}
}

pub struct Progress {pub phase:Phase,pub since:Instant,pub report:serde_json::Value}
pub struct Loaded {pub race:Race,pub built:Built,pub renderer:Renderer,pub light:Lighting,pub sky:serde_json::Value,
    pub tree_baseline:Option<crate::tree_tuning::Baseline>,pub tree_settings:crate::tree_tuning::Settings}
pub struct Request {
    pub seed:Seed,pub seats:Vec<Seat>,pub views:usize,pub settings:serde_json::Value,
    pub looks:Looks,pub sky:Option<PreparedSky>,pub upscale:render::upscale::UpscaleMode,
    pub tree_settings:crate::tree_tuning::Settings,
}

/// Heavy first-renderer construction uses the same device as the presenter,
/// but owns the CPU scene until it returns. The race never advances here.
pub struct InitialGraphics {pub scene:render::Scene,pub renderer:Renderer}
struct InitialRequest {scene:render::Scene,light:Lighting,upscale:render::upscale::UpscaleMode}
// Keep ownership outside the fallible/panicking graphics closure. The same
// slot also survives a failure to spawn the worker, without cloning a scene.
fn prepare_retaining_scene<T>(slot:&Mutex<Option<render::Scene>>,prepare:impl FnOnce(&render::Scene)->Result<T,String>)
    ->Result<(render::Scene,T),String>
{
    let mut scene=slot.lock().unwrap_or_else(|e|e.into_inner());
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepare(scene.as_ref().ok_or("graphics request scene missing")?)
    })).unwrap_or_else(|panic| {
        let reason=panic.downcast_ref::<String>().map(String::as_str)
            .or_else(||panic.downcast_ref::<&str>().copied()).unwrap_or("unknown worker panic");
        Err(format!("graphics worker failed: {reason}"))
    });
    result.map(|prepared|(scene.take().expect("successful graphics preparation retains its scene"),prepared))
}
struct PresentationGate<T> {request:Option<T>,frames:u64}
impl<T> PresentationGate<T> {
    fn new(request:T)->Self {Self {request:Some(request),frames:0}}
    fn presented(&mut self,displayed:bool)->Option<T> {
        if !displayed{return None;}
        self.frames+=1;self.request.take()
    }
}
pub struct Startup {
    gate:PresentationGate<InitialRequest>,worker:Option<std::thread::JoinHandle<Result<InitialGraphics,String>>>,
    progress:Arc<Mutex<Progress>>,cancel:Arc<AtomicBool>,failure:Option<String>,pub started:Instant,
    recovery_scene:Arc<Mutex<Option<render::Scene>>>,
}
impl Startup {
    pub fn new(scene:render::Scene,light:Lighting,upscale:render::upscale::UpscaleMode)->Self {
        let now=Instant::now();Self {gate:PresentationGate::new(InitialRequest {scene,light,upscale}),
            worker:None,cancel:Arc::new(AtomicBool::new(false)),failure:None,started:now,
            recovery_scene:Arc::new(Mutex::new(None)),
            progress:Arc::new(Mutex::new(Progress {phase:Phase::WaitingForDisplay,since:now,
                report:serde_json::json!({"scope":"initial graphics wall time; existing race and generated CPU scene retained"})}))}
    }
    pub fn display_frames(&self)->u64 {self.gate.frames}
    pub fn displayed(&mut self,gpu:&Gpu,displayed:bool) {
        let Some(request)=self.gate.presented(displayed) else{return;};
        let InitialRequest {scene,light,upscale}=request;
        *self.recovery_scene.lock().unwrap_or_else(|e|e.into_inner())=Some(scene);
        let recovery_scene=Arc::clone(&self.recovery_scene);
        let gpu=gpu.clone();
        let progress=Arc::clone(&self.progress);let cancel=Arc::clone(&self.cancel);
        self.progress.lock().unwrap_or_else(|e|e.into_inner()).report["first_display_before_worker_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
        match std::thread::Builder::new().name("startup-graphics".into()).spawn(move|| {
          prepare_retaining_scene(&recovery_scene,|scene| {
            if cancel.load(Ordering::Relaxed){return Err("startup cancelled".into());}
            let start=Instant::now();{
                let mut p=progress.lock().unwrap_or_else(|e|e.into_inner());p.phase=Phase::Graphics;p.since=start;
            }
            let renderer=Renderer::new(&gpu,scene,&light);
            let renderer_prepare_ms=start.elapsed().as_secs_f64()*1000.0;
            let upscale_started=Instant::now();renderer.set_upscale(&gpu,upscale)?;
            let upscale_prepare_ms=upscale_started.elapsed().as_secs_f64()*1000.0;
            if cancel.load(Ordering::Relaxed){return Err("startup cancelled".into());}
            {let mut p=progress.lock().unwrap_or_else(|e|e.into_inner());
                p.report["renderer_prepare_ms"]=renderer_prepare_ms.into();p.report["upscale_prepare_ms"]=upscale_prepare_ms.into();
                p.report["graphics_prepare_ms"]=(start.elapsed().as_secs_f64()*1000.0).into();p.phase=Phase::Upload;p.since=Instant::now();}
            let at=Instant::now();let submission=gpu.queue.submit(std::iter::empty());
            gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(60))})
                .map_err(|e|format!("startup upload completion: {e:?}"))?;
            if cancel.load(Ordering::Relaxed){return Err("startup cancelled".into());}
            {let mut p=progress.lock().unwrap_or_else(|e|e.into_inner());
                p.report["upload_wait_ms"]=(at.elapsed().as_secs_f64()*1000.0).into();p.phase=Phase::Ready;p.since=Instant::now();}
            Ok(renderer)
          }).map(|(scene,renderer)|InitialGraphics {scene,renderer})
        }) {Ok(worker)=>self.worker=Some(worker),Err(e)=>self.failure=Some(format!("could not start graphics worker: {e}"))}
    }
    pub fn cancel(&self){self.cancel.store(true,Ordering::Relaxed);}
    pub fn poll(&mut self)->Option<Result<InitialGraphics,String>> {
        if let Some(error)=self.failure.take(){return Some(Err(error));}
        if !self.worker.as_ref().is_some_and(|w|w.is_finished()){return None;}
        Some(self.worker.take().unwrap().join().unwrap_or_else(|panic| {
            let reason=panic.downcast_ref::<String>().map(String::as_str).or_else(||panic.downcast_ref::<&str>().copied()).unwrap_or("unknown worker panic");
            Err(format!("graphics worker failed: {reason}"))
        }))
    }
    /// Recover the unchanged CPU scene after poll() reports an error. Never
    /// waits for a running worker and returns None after successful transfer.
    /// The caller can restore authored values and keep the previous renderer.
    pub fn take_failed_scene(&mut self)->Option<render::Scene> {
        if self.worker.is_some(){return None;}
        self.recovery_scene.lock().unwrap_or_else(|e|e.into_inner()).take()
    }
    pub fn metadata(&self)->serde_json::Value {
        let p=self.progress.lock().unwrap_or_else(|e|e.into_inner());
        serde_json::json!({"phase":format!("{:?}",p.phase),"elapsed_ms":self.started.elapsed().as_secs_f64()*1000.0,
            "display_frames":self.gate.frames,"worker_started":self.gate.frames>0&&self.gate.request.is_none(),"timings":p.report})
    }
    pub fn hud(&self,size:[u32;2])->render::hud::HudDrawList {
        self.hud_titled(size,"STARTING SIDEWAYS")
    }
    pub fn hud_titled(&self,size:[u32;2],title:&str)->render::hud::HudDrawList {
        let phase=self.progress.lock().unwrap_or_else(|e|e.into_inner()).phase;
        let label=if phase==Phase::WaitingForDisplay {"Preparing graphics"}else{phase.label()};
        status_hud(size,title,label,self.started.elapsed().as_secs_f64())
    }
}
impl Drop for Startup {fn drop(&mut self){self.cancel();}}

fn status_hud(size:[u32;2],title:&str,phase:&str,elapsed:f64)->render::hud::HudDrawList {
    use render::hud::{FontFace,HudDrawList,TextAlign,TextRun};
    let mut list=HudDrawList::new(size);let [w,h]=size.map(|v|v as f32);let clip=[0.0,0.0,w,h];
    let s=(h/1080.0).clamp(0.55,2.0);let panel_w=(680.0*s).min(w-24.0);let x=(w-panel_w)*0.5;let y=h*0.42;
    list.rect(clip,[0.01,0.015,0.025,1.0],clip);
    list.rect([x,y,panel_w,155.0*s],[0.025,0.032,0.045,1.0],clip);
    for (i,(text,top,font_size)) in [(title.to_owned(),15.0,31.0),(phase.to_owned(),59.0,24.0),
        (format!("{elapsed:.1} seconds"),96.0,20.0)].into_iter().enumerate() {
        list.text.push(TextRun {id:u64::MAX-310-i as u64,text,position:[x+18.0*s,y+top*s],bounds:clip,
            font_size:font_size*s,color:[230,235,239,255],font:FontFace::Body,align:TextAlign::Center,width:panel_w-36.0*s});
    }
    let travel=(panel_w-48.0*s).max(1.0);let phase=(elapsed as f32*0.7).fract();
    list.rect([x+16.0*s,y+138.0*s,panel_w-32.0*s,3.0*s],[0.12,0.16,0.2,1.0],clip);
    list.rect([x+16.0*s+travel*phase,y+136.0*s,16.0*s,7.0*s],[0.95,0.65,0.22,1.0],clip);list
}
pub struct Loading {
    request:Option<Request>,worker:Option<std::thread::JoinHandle<Result<Loaded,String>>>,
    pub progress:Arc<Mutex<Progress>>,pub started:Instant,pub display_frames:u64,
    pub seats:Vec<Seat>,pub views:usize,pub settings:serde_json::Value,
    cancel:Arc<AtomicBool>,failure:Option<String>,
}

impl Loading {
    pub fn new(request:Request)->Self {
        let now=Instant::now();
        Self {seats:request.seats.clone(),views:request.views,settings:request.settings.clone(),
            request:Some(request),worker:None,started:now,display_frames:0,cancel:Arc::new(AtomicBool::new(false)),failure:None,
            progress:Arc::new(Mutex::new(Progress {phase:Phase::WaitingForDisplay,since:now,
                report:serde_json::json!({"scope":"wall time; graphics preparation includes CPU mips, pipeline creation and upload enqueue; upload wait is not a rendering FPS measurement"})}))}
    }
    /// Called only after a successful display/present (hidden validation uses
    /// its offscreen present). A failed/occluded acquire cannot start the job.
    pub fn displayed(&mut self,gpu:&Gpu)->Result<(),String> {
        self.display_frames+=1;
        if self.display_frames==1 {
            self.progress.lock().unwrap_or_else(|e|e.into_inner()).report["loading_display_before_worker_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
        }
        let Some(request)=self.request.take() else{return Ok(());};
        let gpu=gpu.clone();
        let progress=Arc::clone(&self.progress);let cancel=Arc::clone(&self.cancel);
        match std::thread::Builder::new().name("next-stage".into()).spawn(move||build(request,gpu,progress,cancel)) {
            Ok(worker)=>self.worker=Some(worker),
            Err(error)=>self.failure=Some(format!("could not start stage worker: {error}")),
        }
        Ok(())
    }
    pub fn cancel(&self){self.cancel.store(true,Ordering::Relaxed);}
    pub fn cancelled(&self)->bool{self.cancel.load(Ordering::Relaxed)}
    pub fn poll(&mut self)->Option<Result<Loaded,String>> {
        if let Some(error)=self.failure.take(){return Some(Err(error));}
        if self.cancelled() && self.worker.is_none(){self.request=None;return Some(Err("stage build cancelled before start".into()));}
        if !self.worker.as_ref().is_some_and(|w|w.is_finished()){return None;}
        Some(self.worker.take().unwrap().join().unwrap_or_else(|panic| {
            let reason=panic.downcast_ref::<String>().map(String::as_str).or_else(||panic.downcast_ref::<&str>().copied()).unwrap_or("unknown worker panic");
            Err(format!("stage worker failed: {reason}"))
        }))
    }
    pub fn metadata(&self)->serde_json::Value {
        let p=self.progress.lock().unwrap_or_else(|e|e.into_inner());
        serde_json::json!({"phase":p.phase.label(),"elapsed_ms":self.started.elapsed().as_secs_f64()*1000.0,
            "phase_elapsed_ms":p.since.elapsed().as_secs_f64()*1000.0,"display_frames":self.display_frames,"timings":p.report})
    }
    pub fn hud(&self,size:[u32;2])->render::hud::HudDrawList {
        use render::hud::{FontFace,HudDrawList,TextAlign,TextRun};
        let mut list=HudDrawList::new(size);let [w,h]=size.map(|v|v as f32);let clip=[0.0,0.0,w,h];
        let s=(h/1080.0).clamp(0.55,2.0);let panel_w=(680.0*s).min(w-24.0);let x=(w-panel_w)*0.5;let y=h*0.42;
        list.rect(clip,[0.01,0.015,0.025,0.7],clip);
        list.rect([x,y,panel_w,155.0*s],[0.025,0.032,0.045,0.97],clip);
        let p=self.progress.lock().unwrap_or_else(|e|e.into_inner());
        for (i,(text,top,font_size,color)) in [
            ("BUILDING THE NEXT STAGE".to_owned(),15.0,31.0,[248,249,246,255]),
            (p.phase.label().to_owned(),59.0,24.0,[206,214,221,255]),
            (format!("{:.1} seconds",self.started.elapsed().as_secs_f64()),96.0,20.0,[206,214,221,255]),
        ].into_iter().enumerate() {
            list.text.push(TextRun {id:u64::MAX-300-i as u64,text,position:[x+18.0*s,y+top*s],bounds:clip,
                font_size:font_size*s,color,font:FontFace::Body,align:TextAlign::Center,width:panel_w-36.0*s});
        }
        // An activity bar, deliberately not a fabricated completion percent.
        let travel=(panel_w-48.0*s).max(1.0);let phase=(self.started.elapsed().as_secs_f32()*0.7).fract();
        list.rect([x+16.0*s,y+138.0*s,panel_w-32.0*s,3.0*s],[0.12,0.16,0.2,1.0],clip);
        list.rect([x+16.0*s+travel*phase,y+136.0*s,16.0*s,7.0*s],[0.95,0.65,0.22,1.0],clip);
        list
    }
}
impl Drop for Loading {fn drop(&mut self){self.cancel.store(true,Ordering::Relaxed);}}

fn build(request:Request,gpu:Gpu,progress:Arc<Mutex<Progress>>,cancel:Arc<AtomicBool>)->Result<Loaded,String> {
    let start=Instant::now();
    let phase=|phase:Phase|->Result<(),String>{
        if cancel.load(Ordering::Relaxed){return Err("stage build cancelled".into());}
        let mut p=progress.lock().unwrap_or_else(|e|e.into_inner());p.phase=phase;p.since=Instant::now();Ok(())
    };
    let record=|key:&str,value:serde_json::Value|{progress.lock().unwrap_or_else(|e|e.into_inner()).report[key]=value;};
    phase(Phase::Race)?;let at=Instant::now();let mut race=request.seed.shell();let mut timings=Timings::default();race.new_stage(&mut timings);
    record("race_generation_ms",(at.elapsed().as_secs_f64()*1000.0).into());record("race_breakdown",timings.table().into());
    for (i,seat) in request.seats.iter().enumerate(){if let Some(player)=race.players.get_mut(i){player.name=seat.name.clone();}}
    phase(Phase::Visuals)?;let at=Instant::now();let poses=poses(&race);let mut rest=Default::default();
    let cams=cameras(&race,&poses,&[0.0;4],&[false;4],&mut rest);
    let mut order:Vec<_>=(0..request.seats.len()).collect();order.sort_by_key(|i|(request.seats[*i].couch.unwrap_or(1000),request.seats[*i].index));
    let positions:Vec<_>=order.iter().filter_map(|i|cams.get(*i).map(|c|c.pos)).take(request.views).collect();
    let mut built=visuals::build_live(&race.stage,&poses,&positions,&request.looks)?;
    // The next stage is played in a window: stream its textures.
    built.scene.texture_streaming=true;
    let tree_baseline=crate::tree_tuning::Baseline::prepare_optional_stage(&mut built.scene,&mut race.phys,request.tree_settings)?;
    record("tree_settings",crate::tree_tuning::Baseline::stage_metadata(tree_baseline.as_ref(),request.tree_settings));
    record("visuals_ms",(at.elapsed().as_secs_f64()*1000.0).into());record("visuals_breakdown",serde_json::json!(built.log));
    let at=Instant::now();let mut light=lighting(race.weather);
    let sky=if let Some(sky)=request.sky {sky.apply_scene(&mut built.scene,&mut light);sky.metadata}else{crate::procedural_sky::apply_environment(&mut built.scene,&mut light)?};
    record("sky_ms",(at.elapsed().as_secs_f64()*1000.0).into());
    phase(Phase::Graphics)?;let at=Instant::now();let renderer=Renderer::new(&gpu,&built.scene,&light);
    record("renderer_prepare_ms",(at.elapsed().as_secs_f64()*1000.0).into());let upscale_started=Instant::now();renderer.set_upscale(&gpu,request.upscale)?;
    record("upscale_prepare_ms",(upscale_started.elapsed().as_secs_f64()*1000.0).into());
    record("graphics_prepare_ms",(at.elapsed().as_secs_f64()*1000.0).into());
    phase(Phase::Upload)?;let at=Instant::now();let submission=gpu.queue.submit(std::iter::empty());
    gpu.device.poll(wgpu::PollType::Wait{submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(60))}).map_err(|e|format!("stage upload completion: {e:?}"))?;
    record("upload_wait_ms",(at.elapsed().as_secs_f64()*1000.0).into());record("total_worker_ms",(start.elapsed().as_secs_f64()*1000.0).into());
    phase(Phase::Ready)?;
    Ok(Loaded {race,built,renderer,light,sky,tree_baseline,tree_settings:request.tree_settings})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_types_can_cross_threads_without_unsafe_app_lifetimes() {
        fn send<T:Send>(){} send::<Seed>();send::<Loaded>();send::<InitialGraphics>();send::<InitialRequest>();
    }
    #[test]
    fn initial_graphics_cannot_start_on_failed_or_occluded_presentation() {
        let scene_bytes=vec![12u8,34,56];let allocation=scene_bytes.as_ptr();
        let mut gate=PresentationGate::new(scene_bytes);
        for _ in 0..20 {assert!(gate.presented(false).is_none());}
        assert_eq!(gate.frames,0);assert_eq!(gate.request.as_ref().unwrap().as_ptr(),allocation);
        let request=gate.presented(true).unwrap();assert_eq!(request.as_ptr(),allocation);
        assert_eq!(request,[12,34,56]);assert_eq!(gate.frames,1);
        assert!(gate.presented(true).is_none());assert_eq!(gate.frames,2);
        assert!(gate.presented(false).is_none());assert_eq!(gate.frames,2);
    }
    #[test]
    fn initial_loading_hud_is_explicit_and_animates_without_gameplay_hud_or_race() {
        for size in [[1280,720],[3840,2160]] {
            let first=status_hud(size,"STARTING SIDEWAYS","Preparing graphics",0.0);
            let later=status_hud(size,"STARTING SIDEWAYS","Preparing graphics",0.5);
            assert_eq!(first.size,size);assert!(first.text.iter().any(|t|t.text=="STARTING SIDEWAYS"));
            assert!(first.text.iter().any(|t|t.text=="Preparing graphics"));
            assert_eq!(first.vertices[0].position,[0.0,0.0]);assert_eq!(first.vertices[0].color[3],1.0);
            assert_ne!(first.vertices.last().unwrap().position,later.vertices.last().unwrap().position);
            assert!(first.vertices.iter().all(|v|v.position.iter().all(|x|x.is_finite())));
        }
    }
    #[test]
    fn initial_graphics_retains_existing_scene_and_light_until_first_presentation() {
        let mut scene=render::Scene::default();scene.aux=vec![71,93];
        let light=Lighting {ev100:7.25,..Default::default()};
        let mut startup=Startup::new(scene,light,render::upscale::UpscaleMode::Native);
        assert!(startup.poll().is_none());assert_eq!(startup.display_frames(),0);
        let request=startup.gate.request.as_ref().unwrap();
        assert_eq!(request.scene.aux,[71,93]);assert_eq!(request.light.ev100,7.25);
        assert_eq!(startup.metadata()["worker_started"],false);
    }
    #[test]
    fn graphics_errors_and_unwind_keep_the_exact_scene_allocation() {
        let mut scene=render::Scene::default();scene.aux=vec![11,29,47];
        let allocation=scene.aux.as_ptr();let slot=Mutex::new(Some(scene));
        for stage in ["renderer","upscale","upload"] {
            let result=prepare_retaining_scene::<()>(&slot,|scene| {
                assert_eq!(scene.aux,[11,29,47]);Err(format!("{stage} failed"))
            });
            assert_eq!(result.err().unwrap(),format!("{stage} failed"));
            assert_eq!(slot.lock().unwrap().as_ref().unwrap().aux.as_ptr(),allocation);
        }
        let result=prepare_retaining_scene::<()>(&slot,|_|panic!("constructor failed"));
        assert_eq!(result.err().unwrap(),"graphics worker failed: constructor failed");
        assert!(!slot.is_poisoned());
        let (scene,value)=prepare_retaining_scene(&slot,|_|Ok(73)).unwrap();
        assert_eq!(value,73);assert_eq!(scene.aux.as_ptr(),allocation);
        assert!(slot.lock().unwrap().is_none());
    }
    #[test]
    fn startup_recovery_survives_an_unstarted_worker_and_is_taken_once() {
        let mut scene=render::Scene::default();scene.aux=vec![83,91];let allocation=scene.aux.as_ptr();
        let mut startup=Startup::new(scene,Lighting::default(),render::upscale::UpscaleMode::Native);
        assert!(startup.take_failed_scene().is_none());
        let request=startup.gate.presented(true).unwrap();
        *startup.recovery_scene.lock().unwrap()=Some(request.scene);
        // A rejected spawn drops its closure and captured Arc. The app's Arc
        // must remain the owner; no OS resource-exhaustion injection is needed.
        let worker_slot=Arc::clone(&startup.recovery_scene);
        let unstarted=move||drop(worker_slot);drop(unstarted);
        startup.failure=Some("could not start graphics worker: test".into());
        assert!(matches!(startup.poll(),Some(Err(_))));
        let recovered=startup.take_failed_scene().unwrap();
        assert_eq!(recovered.aux,[83,91]);assert_eq!(recovered.aux.as_ptr(),allocation);
        assert!(startup.take_failed_scene().is_none());
    }
    #[test]
    fn race_shell_preserves_continuation_rng_and_counts() {
        // Tiny initial stage keeps this CPU lifecycle check bounded.
        let mut original=Race::new(71,Settings {stage_km:0.05,rivals:0,..Default::default()},Cars::load(None).unwrap());
        original.frame=1234;original.stats.finishes=2;
        let mut copy=Seed::next(&original,original.settings.clone()).shell();
        original.new_stage(&mut Timings::default());copy.new_stage(&mut Timings::default());
        assert_eq!(copy.rng,original.rng);assert_eq!(copy.round,original.round);assert_eq!(copy.frame,1234);
        assert_eq!(copy.stage.noise_seed,original.stage.noise_seed);assert_eq!(copy.stage.detail_seed,original.stage.detail_seed);
        assert_eq!(copy.players.len(),original.players.len());assert_eq!(copy.stats.finishes,2);
        for (a,b) in super::super::poses(&copy).iter().zip(super::super::poses(&original)){assert_eq!(a.pos,b.pos);assert_eq!(a.rot,b.rot);}
    }
}
