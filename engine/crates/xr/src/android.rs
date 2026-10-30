use anyhow::{Context, Result, ensure};
use android_activity::{AndroidApp, MainEvent, PollEvent, InputStatus};
use openxr as xr;
use crate::{Eye, Game, Pose, Tracking};
use std::time::{Duration,Instant};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

pub(crate) fn pose(p:xr::Posef)->Pose {
    Pose {position:glam::Vec3::new(p.position.x,p.position.y,p.position.z),
        orientation:glam::Quat::from_xyzw(p.orientation.x,p.orientation.y,p.orientation.z,p.orientation.w)}
}
const VIEW:xr::ViewConfigurationType=xr::ViewConfigurationType::PRIMARY_STEREO;
const HUD:[u32;2]=[1280,720];
fn swapchain(session:&xr::Session<xr::Vulkan>,size:[u32;2],layers:u32)->Result<xr::Swapchain<xr::Vulkan>> {
    Ok(session.create_swapchain(&xr::SwapchainCreateInfo {create_flags:xr::SwapchainCreateFlags::EMPTY,
        usage_flags:xr::SwapchainUsageFlags::COLOR_ATTACHMENT,
        format:ash::vk::Format::R8G8B8A8_SRGB.as_raw() as _,sample_count:1,width:size[0],height:size[1],face_count:1,array_size:layers,mip_count:1})?)
}

/// Keep lifecycle callbacks responsive even while the driver compiles pipelines.
/// The game is created on the render thread; it need not be Send.
pub fn run<G: Game + 'static>(app:AndroidApp, make:impl FnOnce()->Result<G,String> + Send + 'static)->Result<()> {
    let stop=Arc::new(AtomicBool::new(false));
    let worker_app=app.clone(); let worker_stop=stop.clone();
    let worker=std::thread::Builder::new().name("warp-openxr".into()).spawn(move || {
        let vm=unsafe {jni::JavaVM::from_raw(worker_app.vm_as_ptr().cast())}?;
        let _attached=vm.attach_current_thread()?;
        let game=make().map_err(anyhow::Error::msg)?;
        if worker_stop.load(Ordering::Acquire) {return Ok(());}
        run_inner(worker_app,game,worker_stop)
    })?;
    while !worker.is_finished() {
        app.poll_events(Some(Duration::from_millis(20)),|event| {
            match event {
                PollEvent::Main(MainEvent::Destroy)=>{stop.store(true,Ordering::Release);}
                PollEvent::Main(MainEvent::InputAvailable)=>{
                    // Touch gameplay is read through OpenXR, but Android key
                    // and motion events must still be acknowledged to prevent ANRs.
                    if let Ok(mut input)=app.input_events_iter() {while input.next(|_| InputStatus::Unhandled) {}}
                }
                _=>{}
            }
        });
        if stop.load(Ordering::Acquire) {std::thread::sleep(Duration::from_millis(20));}
    }
    worker.join().map_err(|_|anyhow::anyhow!("OpenXR worker panicked"))?
}

fn run_inner(app:AndroidApp, mut game:impl Game, stop:Arc<AtomicBool>)->Result<()> {
    // OpenXR 0.22 performs xrInitializeLoaderKHR using this VM/activity before
    // enumerating extensions. AndroidApp keeps both valid throughout the loop.
    let platform=unsafe {xr::AndroidPlatformInfo::new(app.vm_as_ptr().cast(),app.activity_as_ptr().cast())};
    let entry=unsafe {xr::Entry::load(&platform)}?;
    let available=entry.enumerate_extensions()?;
    ensure!(available.khr_vulkan_enable,"OpenXR runtime lacks XR_KHR_vulkan_enable");
    let mut extensions=xr::ExtensionSet::default();
    extensions.khr_vulkan_enable=true; extensions.khr_android_create_instance=true;
    extensions.fb_display_refresh_rate=available.fb_display_refresh_rate;
    extensions.ext_performance_settings=available.ext_performance_settings;
    let instance=entry.create_instance(&xr::ApplicationInfo {application_name:"Growing Guns",application_version:1,
        engine_name:"Warp",engine_version:1,api_version:xr::Version::new(1,0,0)},&extensions,&[],&platform)?;
    let system=instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
    let config=instance.enumerate_view_configuration_views(system,VIEW)?;
    ensure!(config.len()==2,"runtime did not return two stereo views");
    // Full resolution recommended by the runtime for its lens distortion.
    let size=[config.iter().map(|v|v.recommended_image_rect_width).max().unwrap(),
        config.iter().map(|v|v.recommended_image_rect_height).max().unwrap()];
    let (bridge,binding)=unsafe {crate::vulkan::Bridge::new(&instance,system)}?;
    log::info!("Warp OpenXR GPU {}, eye size {:?}",bridge.gpu.info.name,size);
    game.diagnose(&bridge.gpu).map_err(anyhow::Error::msg)?;
    let (session,mut waiter,mut stream)=unsafe {instance.create_session::<xr::Vulkan>(system,&binding)}?;
    let desired_refresh=if extensions.fb_display_refresh_rate {
        let rates=session.enumerate_display_refresh_rates()?;
        log::info!("Supported display rates: {rates:?}");
        game.refresh_rate().filter(|requested| {
            let supported=rates.iter().any(|r|(*r-requested).abs()<0.1);
            if !supported {log::warn!("Requested {requested} Hz is not exposed by this runtime");}
            supported
        })
    }else {None};
    ensure!(session.enumerate_swapchain_formats()?.contains(&(ash::vk::Format::R8G8B8A8_SRGB.as_raw() as u32)),"runtime lacks RGBA8 sRGB swapchain");
    // LOCAL remains valid with Quest's stationary boundary, where STAGE can
    // be created successfully yet reports invalid poses. The game calibrates
    // eye height once, so neither a floor nor a room setup is required.
    let stage=session.create_reference_space(xr::ReferenceSpaceType::LOCAL,xr::Posef::IDENTITY).context("LOCAL reference space")?;
    let head=session.create_reference_space(xr::ReferenceSpaceType::VIEW,xr::Posef::IDENTITY)?;
    let mut actions=crate::actions::Actions::new(&instance,&session)?;
    let mut color=swapchain(&session,size,1)?;
    let mut right=swapchain(&session,size,1)?;
    let mut ui=swapchain(&session,HUD,1)?;
    let mut vision=swapchain(&session,[1,1],1)?;
    let mut vision_bg=swapchain(&session,[1,1],1)?;
    let color_targets=unsafe{bridge.import_targets(&color.enumerate_images()?,size)}?;
    let right_targets=unsafe{bridge.import_targets(&right.enumerate_images()?,size)}?;
    let ui_targets=unsafe{bridge.import_targets(&ui.enumerate_images()?,HUD)}?;
    let vision_targets=unsafe{bridge.import_targets(&vision.enumerate_images()?,[1,1])}?;
    let vision_bg_targets=unsafe{bridge.import_targets(&vision_bg.enumerate_images()?,[1,1])}?;
    let mut hud=render::hud::HudRenderer::new(&bridge.gpu,wgpu::TextureFormat::Rgba8UnormSrgb).map_err(anyhow::Error::msg)?;
    let mut running=false; let mut focused=false; let mut destroyed=false;
    let mut event=xr::EventDataBuffer::new(); let mut last=Instant::now(); let mut recentered=false;
    let mut frames=0u64; let mut report=Instant::now(); let mut timings=[0.0f64;6];
    while !destroyed && !stop.load(Ordering::Acquire) {
        while let Some(event)=instance.poll_event(&mut event)? {
            match event {
                xr::Event::SessionStateChanged(e)=> {
                    log::info!("OpenXR state {:?}",e.state());
                    focused=e.state()==xr::SessionState::FOCUSED;
                    match e.state() {
                        xr::SessionState::READY=>{
                            session.begin(VIEW)?;running=true;last=Instant::now();
                            if let Some(ext)=instance.exts().ext_performance_settings.as_ref() {
                                for domain in [xr::PerfSettingsDomainEXT::CPU,xr::PerfSettingsDomainEXT::GPU] {
                                    let result=unsafe{(ext.perf_settings_set_performance_level)(session.as_raw(),domain,xr::PerfSettingsLevelEXT::SUSTAINED_HIGH)};
                                    log::info!("Sustained performance {domain:?}: {result:?}");
                                }
                            }
                            if let Some(hz)=desired_refresh {
                                if let Err(e)=session.request_display_refresh_rate(hz) {log::warn!("Display rate request failed: {e}");}
                            }
                            if extensions.fb_display_refresh_rate {log::info!("Current display rate: {} Hz",session.get_display_refresh_rate()?);}
                        },
                        xr::SessionState::STOPPING=>{session.end()?;running=false;},
                        xr::SessionState::EXITING|xr::SessionState::LOSS_PENDING=>destroyed=true,
                        _=>{}
                    }
                }
                xr::Event::DisplayRefreshRateChangedFB(e)=>log::info!("Display rate changed to {} Hz",e.to_display_refresh_rate()),
                xr::Event::InstanceLossPending(_)=>destroyed=true,
                xr::Event::ReferenceSpaceChangePending(_)=>recentered=true,
                _=>{}
            }
        }
        if destroyed {break;} if !running {std::thread::sleep(Duration::from_millis(20));continue;}
        let frame_start=Instant::now();
        let state=waiter.wait()?; stream.begin()?;
        let now=Instant::now(); let dt=(now-last).as_secs_f32().min(0.1);last=now;
        if !state.should_render {stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[])?;continue;}
        let (flags,views)=session.locate_views(VIEW,state.predicted_display_time,&stage)?;
        let h=head.locate(&stage,state.predicted_display_time)?;
        let valid=flags.contains(xr::ViewStateFlags::POSITION_VALID|xr::ViewStateFlags::ORIENTATION_VALID)
            && h.location_flags.contains(xr::SpaceLocationFlags::POSITION_VALID|xr::SpaceLocationFlags::ORIENTATION_VALID);
        if !valid||views.len()!=2 {stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[])?;continue;}
        let (controls,turn,aim)=actions.sample(&session,&stage,state.predicted_display_time,focused)?;
        let tracking=Tracking {eyes:std::array::from_fn(|i|Eye {pose:pose(views[i].pose),fov:[views[i].fov.angle_left,views[i].fov.angle_right,views[i].fov.angle_up,views[i].fov.angle_down]}),
            head:pose(h.pose),aim,controls,turn,focused,recentered:std::mem::take(&mut recentered)};
        if game.quit() { session.request_exit()?; stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[])?; continue; }
        if frames==0 {log::info!("Drawing first native Warp stereo frame");}
        let acquire_start=Instant::now();
        let color_index=color.acquire_image()?;color.wait_image(xr::Duration::INFINITE)?;
        let right_index=right.acquire_image()?;right.wait_image(xr::Duration::INFINITE)?;
        let ui_index=ui.acquire_image()?;ui.wait_image(xr::Duration::INFINITE)?;
        let draw_start=Instant::now();
        let output=game.frame(&bridge.gpu,&tracking,size,dt,Some(&[&color_targets.images[color_index as usize],&right_targets.images[right_index as usize]])).map_err(anyhow::Error::msg)?;
        let hud_start=Instant::now();
        crate::draw_hud(&bridge.gpu,&mut hud,&output.hud,&ui_targets.images[ui_index as usize]).map_err(anyhow::Error::msg)?;
        let mut tinted=[false;2];
        for (i,(chain,targets,tints)) in [(&mut vision_bg,&vision_bg_targets,&output.hud.vision_background_tints),(&mut vision,&vision_targets,&output.hud.vision_tints)].into_iter().enumerate() {
            if !tints.is_empty() {
                let index=chain.acquire_image()?;chain.wait_image(xr::Duration::INFINITE)?;
                crate::draw_vision(&bridge.gpu,tints,&targets.images[index as usize]);tinted[i]=true;
            }
        }
        let gpu_start=Instant::now();
        bridge.gpu.wait();
        let end_start=Instant::now();
        color.release_image()?;right.release_image()?;ui.release_image()?;
        if tinted[0]{vision_bg.release_image()?;}if tinted[1]{vision.release_image()?;}
        let rect=xr::Rect2Di {offset:xr::Offset2Di{x:0,y:0},extent:xr::Extent2Di{width:size[0] as i32,height:size[1] as i32}};
        let projection_views:Vec<_>=views.iter().enumerate().map(|(i,v)|xr::CompositionLayerProjectionView::new().pose(v.pose).fov(v.fov)
            .sub_image(xr::SwapchainSubImage::new().swapchain(if i==0{&color}else{&right}).image_array_index(0).image_rect(rect))).collect();
        let projection=xr::CompositionLayerProjection::new().space(&stage).views(&projection_views);
        // Keep original menus/cards legible at a comfortable fixed distance.
        // HUD rendering is premultiplied by normal alpha blending onto clear.
        let quad=xr::CompositionLayerQuad::new().space(&head).eye_visibility(xr::EyeVisibility::BOTH)
            .layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
            .pose(xr::Posef {orientation:xr::Quaternionf{x:0.0,y:0.0,z:0.0,w:1.0},position:xr::Vector3f{x:0.0,y:0.0,z:-2.0}})
            .size(xr::Extent2Df{width:2.4,height:1.35})
            .sub_image(xr::SwapchainSubImage::new().swapchain(&ui).image_array_index(0).image_rect(xr::Rect2Di{offset:xr::Offset2Di{x:0,y:0},extent:xr::Extent2Di{width:HUD[0] as i32,height:HUD[1] as i32}}));
        let tint_views:Vec<_>=views.iter().map(|v|xr::CompositionLayerProjectionView::new().pose(v.pose).fov(v.fov)
            .sub_image(xr::SwapchainSubImage::new().swapchain(&vision).image_array_index(0)
                .image_rect(xr::Rect2Di{offset:xr::Offset2Di{x:0,y:0},extent:xr::Extent2Di{width:1,height:1}}))).collect();
        let tint=xr::CompositionLayerProjection::new().space(&stage).layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA).views(&tint_views);
        let bg_views:Vec<_>=views.iter().map(|v|xr::CompositionLayerProjectionView::new().pose(v.pose).fov(v.fov)
            .sub_image(xr::SwapchainSubImage::new().swapchain(&vision_bg).image_array_index(0)
                .image_rect(xr::Rect2Di{offset:xr::Offset2Di{x:0,y:0},extent:xr::Extent2Di{width:1,height:1}}))).collect();
        let bg=xr::CompositionLayerProjection::new().space(&stage).layer_flags(xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA).views(&bg_views);
        match tinted {
            [false,false]=>stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[&projection,&quad])?,
            [true,false]=>stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[&projection,&bg,&quad])?,
            [false,true]=>stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[&projection,&quad,&tint])?,
            [true,true]=>stream.end(state.predicted_display_time,xr::EnvironmentBlendMode::OPAQUE,&[&projection,&bg,&quad,&tint])?,
        }
        for (total,span) in timings.iter_mut().zip([acquire_start-frame_start,draw_start-acquire_start,hud_start-draw_start,gpu_start-hud_start,end_start-gpu_start,end_start.elapsed()]) { *total+=span.as_secs_f64()*1000.0; }
        frames+=1;
        if frames==1 {log::info!("First native Warp stereo frame submitted");report=Instant::now();timings.fill(0.0);}
        if frames>1 && (frames-1)%30==0 {log::info!("30 frames submitted in {:.2}s ({:.1} fps), focused={}, controller={}, mean ms wait/acquire/game/hud/gpu/end={:?}",report.elapsed().as_secs_f32(),30.0/report.elapsed().as_secs_f32(),focused,tracking.aim.is_some(),timings.map(|t|(t/30.0*100.0).round()/100.0));report=Instant::now();timings.fill(0.0);}
    }
    bridge.gpu.wait();
    Ok(())
}
