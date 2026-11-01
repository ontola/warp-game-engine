//! Reproduce continuously scaled text using Growing Guns' eight outline copies.
//! hud_font_churn OUTPUT.csv [--fixed|--quantized] [--large] [--cached]. No window or XR compositor.
#[cfg(target_os="android")]
#[link(name="c++_shared")]
unsafe extern "C" {}
use render::{Gpu,hud::{HudDrawList,HudRenderer,TextRun,FontFace,TextAlign},hud_profile::HudProfiler};
fn main()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();let out=args.get(1).ok_or("hud_font_churn OUTPUT.csv [--fixed|--quantized] [--large] [--cached]")?;
    let large=args.iter().any(|a|a=="--large");
    let fixed=args.iter().any(|a|a=="--fixed");let quantized=args.iter().any(|a|a=="--quantized");
    let gpu=Gpu::new()?;let format=wgpu::TextureFormat::Rgba8UnormSrgb;
    let target=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("HUD churn"),size:wgpu::Extent3d{width:1280,height:720,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
    let view=target.create_view(&Default::default());let mut hud=HudRenderer::new(&gpu,format)?;
    let mut profiler=HudProfiler::new(&gpu,120);let mut samples=Vec::new();
    for frame in 0..136 {
        let raw=if large{180.+(frame as f32*0.021).sin()*60.}else{60.+(frame as f32*0.021).sin()*20.};let size=if fixed{if large{180.}else{60.}}else if quantized{(raw/4.).round()*4.}else{raw};
        let mut list=HudDrawList::new([1280,720]);list.cached_text=args.iter().any(|a|a=="--cached");
        for (i,(dx,dy)) in [(-1.,-1.),(1.,-1.),(-1.,1.),(1.,1.),(0.,-1.4),(0.,1.4),(-1.4,0.),(1.4,0.),(0.,0.)].into_iter().enumerate() {
            list.text.push(TextRun{id:i as u64,text:"ROUND 12".into(),position:[300.+dx*size*0.1,200.+dy*size*0.1],bounds:[0.,0.,1280.,720.],font_size:size,color:if i==8{[255,230,180,255]}else{[0,0,0,255]},font:FontFace::Title,align:TextAlign::Left,width:700.});
        }
        let stats=hud.prepare(&gpu,&list)?;
        let mut enc=gpu.device.create_command_encoder(&Default::default());
        {let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("HUD churn draw"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})],timestamp_writes:if frame>=16{profiler.as_ref().map(|p|p.writes())}else{None},..Default::default()});hud.render(&mut pass)?;}
        if frame>=16 {if let Some(p)=profiler.as_mut(){p.resolve(&mut enc)?;}samples.push((size,stats));}
        gpu.queue.submit([enc.finish()]);gpu.wait();
    }
    render::write_png(&std::path::Path::new(out).with_extension("png"),1280,720,&gpu.read_rgba8(&target))?;
    let times=profiler.map(|p|p.read(&gpu)).transpose()?;
    let mut csv=String::from("size,prepare_ms,upload_ms,shape_ms,warm_ms,outline_ms,atlas_ms,reshaped,gpu_ms\n");
    for (i,(size,s)) in samples.iter().enumerate(){let p=s.phase_ms;csv.push_str(&format!("{size},{},{},{},{},{},{},{},{}\n",s.prepare_ms,p[0],p[1],p[2],p[3],p[4],s.reshaped,times.as_ref().map(|t|t[i].to_string()).unwrap_or_default()));}
    std::fs::write(out,csv).map_err(|e|e.to_string())?;
    println!("HUD churn: 120 measured frames after 16 warmups; fixed={fixed}, quantized={quantized}, large={large}, GPU {}",gpu.info.name);Ok(())
}
