//! Native VR adapts the existing Gg app; no duplicate scene or card/game logic.
use app::Game as _;
use glam::{Affine3A,Quat,Vec3};
use growing_guns::{World,TrackedAim,DT};
use warp_xr::{Eye,Pose,Tracking};

pub struct NativeGame {
    game:crate::play::Gg,
    renderer:Option<render::Renderer>,
    output:Option<wgpu::Texture>,
    origin:Option<Vec3>, yaw:f32, snap_ready:bool, accumulator:f32, pending:u32,
    _speaker:Option<app::sound::Speaker>,
    audio_pending:bool,
    diagnostic:bool,
    profile:bool, profile_sequence:u64,
    diagnostic_requested:bool,
}
impl NativeGame {
    pub fn new(sound:bool)->Result<Self,String> {
        crate::graphics::configure(crate::graphics::Graphics::MINIMUM)?;
        let mut game=crate::play::Gg::new(World::new(3,&[false,true,true,true]),crate::session::Session::Local,vec![0]);
        if game.scripts.is_none() {return Err("Growing Guns card scripts did not load".into());}
        game.xr_stereo=true;game.no_kbm=true;game.ui.tilt=false;
        game.world.release_cage();
        // VR supplies an unjittered projection. Camera shake/zoom never changes it.
        game.light_mut().taa=0.0;
        game.light_mut().near_shadows=0.0;
        game.light_mut().lod_pixels=crate::graphics::get().lod_pixels;
        let yaw = game.world.players[0].yaw;
        Ok(Self {game,renderer:None,output:None,origin:None,yaw,snap_ready:true,accumulator:0.0,pending:0,_speaker:None,audio_pending:sound,diagnostic:false,profile:std::path::Path::new("xr-profile").exists(),profile_sequence:0,diagnostic_requested:std::path::Path::new("diagnose").exists()})
    }
    fn anchor(&self,eye:Vec3)->Vec3 {eye-Quat::from_rotation_y(self.yaw)*self.origin.unwrap()}
    fn hand(&self,t:&Tracking,eye:Vec3)->Option<Affine3A> {
        let pose=t.aim?.transformed(self.anchor(eye),self.yaw);
        Some(Affine3A::from_rotation_translation(pose.orientation,pose.position))
    }
}
impl warp_xr::Game for NativeGame {
    fn refresh_rate(&self)->Option<f32> {Some(120.0)}
    fn quit(&self) -> bool { self.game.quit() }
    fn diagnose(&mut self, gpu: &render::Gpu) -> Result<(), String> {
        if !self.diagnostic_requested { return Ok(()); }
        self.diagnostic=true;
        let head=Pose {position:Vec3::new(0.0,1.65,0.0),orientation:Quat::IDENTITY};
        let eyes=[-0.032,0.032].map(|x|Eye {pose:Pose {position:head.position+Vec3::X*x,..head},fov:[-0.8,0.8,0.8,-0.8]});
        let t=Tracking {eyes,head,aim:Some(Pose {position:Vec3::new(0.25,1.3,-0.35),..head}),controls:Default::default(),turn:0.0,focused:false,recentered:false};
        for (name, debug) in [("lit",0.0),("albedo",2.0),("normals",11.0)] {
            self.game.light_mut().debug=debug;
            let start=std::time::Instant::now();
            let out=self.frame(gpu,&t,[512,512],0.0,None)?;
            let rgba=gpu.read_rgba8(out.color.as_ref().unwrap());
            render::write_png(std::path::Path::new(&format!("diagnostic-{name}.png")),1024,512,&rgba)?;
            let light=self.game.light_mut();
            eprintln!("XR diagnostic {name}: {:.1} ms, mean RGB {:.1}, sun {}, sky {}, EV {}",start.elapsed().as_secs_f64()*1000.0,
                rgba.chunks_exact(4).map(|p|p[..3].iter().map(|v|*v as u64).sum::<u64>()).sum::<u64>() as f64/(1024.0*512.0*3.0),light.sun_lux,light.sky_lux,light.ev100);
        }
        self.game.light_mut().debug=0.0;self.origin=None;self.diagnostic=false;
        // Independent renderer probe distinguishes game data from Vulkan setup.
        for (name, device) in [("xr-device", gpu.clone()), ("wgpu-device", render::Gpu::new()?)] {
            let mut scene=render::Scene::default();
            let m=scene.add_material(render::scene::Material {tint:[0.8,0.2,0.1],layer:-1,roughness:0.8,flags:0,scale:1.0,metal:0.0});
            let mesh=scene.add_mesh(&render::primitives::cube(m).mesh());
            scene.add_instance(mesh,Affine3A::IDENTITY);
            let light=render::Lighting::default();
            let renderer=render::Renderer::new(&device,&scene,&light);
            let cam=render::Camera {pos:Vec3::new(3.0,2.0,4.0),target:Vec3::ZERO,fov_y:1.2,near:0.05};
            let (rgba,stats)=renderer.render(&device,&cam,&light,256,256);
            render::write_png(std::path::Path::new(&format!("diagnostic-{name}.png")),256,256,&rgba)?;
            eprintln!("XR probe {name}: clusters {}, triangles {}, mean {}",stats.clusters,stats.triangles,rgba.chunks_exact(4).map(|p|p[0] as u64+p[1] as u64+p[2] as u64).sum::<u64>() as f64/(256.0*256.0*3.0));
        }
        Ok(())
    }
    fn frame(&mut self,gpu:&render::Gpu,t:&Tracking,size:[u32;2],dt:f32,targets:Option<&[&wgpu::Texture]>)->Result<warp_xr::FrameOutput,String> {
        // A sleeping Quest can time out audio startup before OpenXR gets focus.
        // Start it only once the headset is active; audio failure must not abort VR.
        if self.audio_pending&&t.focused {
            self.audio_pending=false;
            match crate::sounds::speakers(None,None) {
                Ok((speaker,tx,_))=>{
                    self.game.set_sounds(crate::sounds::Sounds::new(tx,crate::sounds::find_patches(None),crate::sounds::find_dir(None).as_deref()));
                    self._speaker=Some(speaker);
                }
                Err(error)=>eprintln!("XR audio unavailable: {error}"),
            }
        }
        if self.origin.is_none() {
            let f = t.head.orientation * Vec3::NEG_Z;
            self.yaw -= (-f.x).atan2(-f.z);
            self.origin = Some(t.head.position);
        } else if t.recentered { self.origin=Some(t.head.position); }
        if t.focused && t.turn.abs()>0.7 && self.snap_ready {
            // Rotate about the current head, preserving its world position.
            let old=Quat::from_rotation_y(self.yaw)*(t.head.position-self.origin.unwrap());
            self.yaw-=t.turn.signum()*30f32.to_radians();
            self.origin=Some(t.head.position-Quat::from_rotation_y(-self.yaw)*old);
            self.snap_ready=false;
        }
        if t.turn.abs()<0.3 {self.snap_ready=true;}
        let forward=Quat::from_rotation_y(self.yaw)*t.head.orientation*Vec3::NEG_Z;
        self.pending|=t.controls.pressed.0;
        if t.focused {self.accumulator=(self.accumulator+dt).min(DT*4.0);}else{self.accumulator=0.0;self.pending=0;}
        while self.accumulator>=DT {
            let p=&mut self.game.world.players[0];
            p.yaw=(-forward.x).atan2(-forward.z);p.pitch=forward.y.clamp(-1.0,1.0).asin();
            let hand=self.hand(t,self.game.world.players[0].eye());
            self.game.xr_gun=hand;
            self.game.xr_aim=hand.map(|pose|TrackedAim {offset:(self.game.vis.tracked_muzzle(0,pose)-self.game.world.players[0].pos).clamp_length_max(2.45),direction:pose.transform_vector3(Vec3::NEG_Z)});
            let mut controls=t.controls;controls.pressed.0=std::mem::take(&mut self.pending);
            if hand.is_none() {controls.held.0 &= !(app::Buttons::RIGHT_TRIGGER|app::Buttons::LEFT_TRIGGER|app::Buttons::EAST);}
            self.game.tick(&[controls]);
            self.accumulator-=DT;
        }
        let alpha=self.accumulator/DT;
        let base=self.game.xr_eye(alpha);
        self.game.xr_gun=self.hand(t,base);
        let mut frame=app::Frame::new(gpu,&mut self.renderer,[1280,720],dt,alpha);
        let draw=self.game.draw(&mut frame)?;
        let eyes:[Eye;2]=t.eyes.map(|eye|Eye {pose:eye.pose.transformed(self.anchor(base),self.yaw),..eye});
        let renderer=self.renderer.as_ref().ok_or("Growing Guns did not create its renderer")?;
        renderer.set_camera_up_axes(&eyes.map(|e|e.pose.orientation*Vec3::Y));
        renderer.set_camera_projections(&eyes.map(Eye::projection));
        let views:Vec<_>=eyes.iter().enumerate().map(|(i,e)|render::ViewRect {cam:e.camera(),x:i as u32*size[0],y:0,w:size[0],h:size[1]}).collect();
        if let Some(targets)=targets {
            if self.profile {
                renderer.set_async_gpu_profiling(gpu,true)?;
                for sample in renderer.drain_gpu_profiles(gpu).samples {
                    #[cfg(target_os="android")] log::info!("XR GPU frame={} passes={:?}",sample.frame_id,sample.gpu_ms);
                    #[cfg(not(target_os="android"))] eprintln!("XR GPU frame={} passes={:?}",sample.frame_id,sample.gpu_ms);
                }
                renderer.set_gpu_profile_frame_id(self.profile_sequence);self.profile_sequence+=1;
            }
            renderer.try_render_eye_targets(gpu,&views,&draw.light,targets)?;
            return Ok(warp_xr::FrameOutput {color:None,hud:draw.hud});
        }
        if self.output.as_ref().is_none_or(|tex|tex.width()!=size[0]*2||tex.height()!=size[1]) {
            self.output=Some(render::Renderer::output_texture(gpu,size[0]*2,size[1]));
        }
        let output=self.output.as_ref().unwrap();
        if self.diagnostic {
            let (color, stats)=renderer.try_render_texture_profiled(gpu,&views,&draw.light,size[0]*2,size[1])?;
            eprintln!("XR GPU passes: {:?}; clusters {}, triangles {}, shadows {}; eye {:?}; projection {:?}",stats.gpu_ms,stats.clusters,stats.triangles,stats.shadow_clusters,views[0].cam,eyes[0].projection());
            return Ok(warp_xr::FrameOutput {color:Some(color),hud:draw.hud});
        }
        renderer.try_render_into(gpu,&views,&draw.light,output)?;
        Ok(warp_xr::FrameOutput {color:Some(output.clone()),hud:draw.hud})
    }
}

/// Headless native renderer fixture; it does not impersonate a Quest runtime.
pub fn shot(args:&crate::Args)->Result<(),String> {
    use warp_xr::Game;
    let out=args.value("--out").ok_or("xr-shot --out FILE.png")?;
    let size=args.size((640,640));
    let gpu=render::Gpu::new()?;
    let mut game=NativeGame::new(false)?;
    if args.flag("--diagnose") {game.diagnostic_requested=true;game.diagnose(&gpu)?;}
    let head=Pose {position:Vec3::new(0.0,1.65,0.0),orientation:Quat::IDENTITY};
    let eyes=[-0.032,0.032].map(|x|Eye {pose:Pose {position:head.position+Vec3::X*x,..head},fov:[-0.8,0.8,0.8,-0.8]});
    let aim=Pose {position:Vec3::new(0.25,1.3,-0.35),orientation:Quat::from_rotation_x(0.1)};
    let mut controls=app::Controls::default();
    if args.flag("--fire") {controls.held.0|=app::Buttons::RIGHT_TRIGGER;}
    let tracking=Tracking {eyes,head,aim:Some(aim),controls,turn:0.0,focused:true,recentered:false};
    // Real game logic and full card set, with the same rig used by Android.
    for _ in 0..90 {game.game.tick(&[Default::default()]);}
    let mut hud=render::hud::HudRenderer::new(&gpu,wgpu::TextureFormat::Rgba8UnormSrgb)?;
    let hud_texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("XR fixture HUD"),size:wgpu::Extent3d{width:1280,height:720,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
    let mut frame=game.frame(&gpu,&tracking,[size.0,size.1],DT,None)?;
    warp_xr::draw_hud(&gpu,&mut hud,&frame.hud,&hud_texture)?;
    if args.flag("--frames") {
        gpu.wait();
        let count=args.num("--frames",30u32).clamp(1,600);
        let start=std::time::Instant::now();
        for _ in 0..count {frame=game.frame(&gpu,&tracking,[size.0,size.1],DT,None)?;warp_xr::draw_hud(&gpu,&mut hud,&frame.hud,&hud_texture)?;if !args.flag("--queued") {gpu.wait();}}
        gpu.wait();
        println!("Headless native fixture: {count} frames, {:.2} ms/frame at {}x{} per eye including HUD (queued={}); no compositor or display pacing",start.elapsed().as_secs_f64()*1000.0/count as f64,size.0,size.1,args.flag("--queued"));
    }
    render::write_png(std::path::Path::new(out),size.0*2,size.1,&gpu.read_rgba8(frame.color.as_ref().unwrap()))?;
    println!("Native Warp stereo fixture: {out}; {} cards; GPU {}. This is not a headset test.",game.game.world.cards.enabled().count(),gpu.info.name);
    Ok(())
}

#[cfg(target_os="android")]
#[unsafe(no_mangle)]
fn android_main(app:android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_tag("WarpXR").with_max_level(log::LevelFilter::Info));
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<(),String>{
        install_assets(&app)?;
        warp_xr::run(app,|| NativeGame::new(true)).map_err(|e|format!("{e:#}"))
    }));
    match result {Ok(Ok(()))=>{},Ok(Err(e))=>log::error!("Warp XR stopped: {e}"),Err(_)=>log::error!("Warp XR panicked; inspect logcat")}
}

#[cfg(target_os="android")]
fn install_assets(app:&android_activity::AndroidApp)->Result<(),String> {
    use std::io::Read;
    let root=app.internal_data_path().ok_or("Android internal data path missing")?.join("growing-guns");
    std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
    let mut asset=app.asset_manager().open(c"growing-guns.zip").ok_or("APK missing Growing Guns assets")?;
    let mut bytes=Vec::new();asset.read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    let mut zip=zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e|e.to_string())?;
    zip.extract(&root).map_err(|e|e.to_string())?;
    // Done before game/audio/renderer worker threads start.
    unsafe {
        std::env::set_var("KERN_PIPELINE_CACHE_DIR", root.join("pipeline-cache"));
        std::env::set_var("GG_CHARACTERS",root.join("assets/characters"));
        std::env::set_var("GG_MODS",root.join("mods"));
        std::env::set_var("GG_SFX",root.join("sounds"));
        std::env::set_var("GG_SOUNDS",root.join("audio"));
    }
    std::env::set_current_dir(root).map_err(|e|e.to_string())
}
