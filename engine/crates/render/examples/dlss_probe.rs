//! Actual NGX lifecycle and native-interop oracle, without a window or assets.
//! Run only with exclusive GPU access: --out NEW_DIRECTORY.
//! --missing-runtime-native requires KERN_DLSS_RUNTIME_DIR to a missing folder.
//! Constant radiance tests state/lifetime/color preservation, not scene quality,
//! motion accuracy, jitter convergence or DLSS performance.

use render::{dlss::{DlssContext, Inputs}, upscale::UpscaleMode, Gpu};
use std::path::{Path, PathBuf};

fn texture(gpu:&Gpu, format:wgpu::TextureFormat, size:(u32,u32), usage:wgpu::TextureUsages)->wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("DLSS lifecycle oracle"),
        size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
        dimension:wgpu::TextureDimension::D2,format,usage,view_formats:&[]})
}
fn upload(gpu:&Gpu,texture:&wgpu::Texture,pixel:&[u8]) {
    let data=pixel.repeat((texture.width()*texture.height())as usize);
    gpu.queue.write_texture(texture.as_image_copy(),&data,wgpu::TexelCopyBufferLayout {offset:0,
        bytes_per_row:Some(texture.width()*pixel.len()as u32),rows_per_image:Some(texture.height())},texture.size());
}
fn half(v:f32)->u16 {
    if v==0. {return 0;}
    assert!(v.is_finite() && v>0.);
    let bits=v.to_bits();let exponent=((bits>>23)&255)as i32-127+15;
    assert!((1..31).contains(&exponent));((exponent as u16)<<10)|((bits>>13)as u16&1023)
}
fn unhalf(v:u16)->f32 {
    let sign=if v&0x8000==0 {1.}else{-1.};let exponent=(v>>10)&31;let mantissa=(v&1023)as f32;
    match exponent {0=>sign*mantissa*2f32.powi(-24),31=>if mantissa==0. {sign*f32::INFINITY}else{f32::NAN},
        _=>sign*(1.+mantissa/1024.)*2f32.powi(exponent as i32-15)}
}
struct Source {color:wgpu::Texture,depth:wgpu::Texture,motion:wgpu::Texture,bias:wgpu::Texture}
impl Source {
    fn new(gpu:&Gpu,size:(u32,u32),color:[f32;3])->Self {
        let usage=wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST;
        let source=Self {color:texture(gpu,wgpu::TextureFormat::Rgba16Float,size,usage),
            depth:texture(gpu,wgpu::TextureFormat::R32Float,size,usage),motion:texture(gpu,wgpu::TextureFormat::Rg16Float,size,usage),
            bias:texture(gpu,wgpu::TextureFormat::R32Float,size,usage)};
        upload(gpu,&source.color,bytemuck::cast_slice(&[half(color[0]),half(color[1]),half(color[2]),half(1.)]));
        upload(gpu,&source.depth,&0.01f32.to_le_bytes());upload(gpu,&source.motion,&[0u8;4]);upload(gpu,&source.bias,&0f32.to_le_bytes());source
    }
}
#[derive(Clone,Copy)]
struct View {mode:UpscaleMode,size:(u32,u32),color:[f32;3],reset:bool}
const COLORS:[[f32;3];4]=[[0.65,0.1,0.05],[0.05,0.6,0.1],[0.05,0.15,0.7],[0.5,0.4,0.05]];
fn frame(gpu:&Gpu,context:&mut DlssContext,views:&[View],sample:u32)->Result<Vec<wgpu::Texture>,String> {
    let mut encoder=gpu.device.create_command_encoder(&Default::default());let mut segments=Vec::new();let mut outputs=Vec::new();
    for (index,view) in views.iter().enumerate() {
        let input=context.optimal_size(view.mode,view.size)?;let source=Source::new(gpu,input,view.color);
        let jitter=[if sample%2==0 {-0.25}else{0.25},if sample%4<2 {-0.25}else{0.25}];
        outputs.push(context.evaluate(gpu,&mut encoder,&mut segments,index,view.mode,input,view.size,Inputs {
            color:&source.color,depth:&source.depth,motion:&source.motion,bias:Some(&source.bias),jitter,
            reset:view.reset,exposure:1.,frame_time_ms:1000./120.})?);
        // Source handles intentionally drop before submit: the native segment's
        // retention callback must keep their actual image/views alive.
    }
    segments.push(encoder.finish());gpu.queue.submit(segments);Ok(outputs)
}
fn read_raw(gpu:&Gpu,texture:&wgpu::Texture)->Result<Vec<u8>,String> {
    let row=(texture.width()*8).div_ceil(256)*256;
    let buffer=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("DLSS raw output oracle"),size:row as u64*texture.height()as u64,
        usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(texture.as_image_copy(),wgpu::TexelCopyBufferInfo {buffer:&buffer,
        layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(row),rows_per_image:Some(texture.height())}},texture.size());
    gpu.queue.submit([encoder.finish()]);
    let (send,receive)=std::sync::mpsc::channel();buffer.slice(..).map_async(wgpu::MapMode::Read,move |r|{let _=send.send(r);});
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e|format!("readback wait: {e}"))?;
    receive.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let mapped=buffer.slice(..).get_mapped_range().map_err(|e|e.to_string())?;let mut result=Vec::new();
    for y in 0..texture.height() {let start=(y*row)as usize;result.extend_from_slice(&mapped[start..start+(texture.width()*8)as usize]);}
    drop(mapped);buffer.unmap();Ok(result)
}
fn check(gpu:&Gpu,out:&Path,name:&str,texture:&wgpu::Texture,expected:[f32;3])->Result<(String,bool),String> {
    let raw=read_raw(gpu,texture)?;std::fs::write(out.join(format!("{name}.rgba16f")),&raw).map_err(|e|e.to_string())?;
    let mut mean=[0f64;3];let mut worst=0f32;let mut finite=true;let mut n=0u32;let mut image=Vec::new();
    for y in 0..texture.height() {for x in 0..texture.width() {
        let offset=((y*texture.width()+x)*8)as usize;let mut color=[0f32;3];
        for c in 0..3 {color[c]=unhalf(u16::from_le_bytes(raw[offset+c*2..offset+c*2+2].try_into().unwrap()));finite&=color[c].is_finite();}
        if x>=16 && y>=16 && x+16<texture.width() && y+16<texture.height() {
            for c in 0..3 {mean[c]+=color[c]as f64;worst=worst.max((color[c]-expected[c]).abs());}n+=1;
        }
        image.extend(color.map(|c|(c.max(0.).min(1.).powf(1./2.2)*255.).round()as u8));image.push(255);
    }}
    for c in &mut mean {*c/=n.max(1)as f64;if !c.is_finite(){*c=0.;}}
    let passed=finite && n>0 && worst<0.04 && mean.iter().sum::<f64>()>0.02;
    render::gpu::write_png(&out.join(format!("{name}.png")),texture.width(),texture.height(),&image)?;
    Ok((format!("{{\"name\":{name:?},\"output\":[{},{}],\"expected_rgb\":{expected:?},\"mean_rgb\":{mean:?},\"max_interior_error\":{worst},\"finite_rgb\":{finite},\"passed\":{passed}}}",texture.width(),texture.height()),passed))
}
fn native_clear(gpu:&Gpu)->bool {
    let target=texture(gpu,wgpu::TextureFormat::Rgba8Unorm,(64,64),wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC);
    let view=target.create_view(&Default::default());let mut encoder=gpu.device.create_command_encoder(&Default::default());
    {let _pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("ordinary wgpu after NGX"),
        color_attachments:&[Some(wgpu::RenderPassColorAttachment {view:&view,resolve_target:None,depth_slice:None,
            ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color {r:0.25,g:0.5,b:0.75,a:1.}),store:wgpu::StoreOp::Store}})],
        depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});}
    gpu.queue.submit([encoder.finish()]);gpu.read_rgba8(&target).chunks_exact(4).all(|p|p[0].abs_diff(64)<=1 && p[1].abs_diff(128)<=1 && p[2].abs_diff(191)<=1 && p[3]==255)
}
fn run()->Result<(),String> {
    let args=std::env::args().collect::<Vec<_>>();let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("use --out NEW_DIRECTORY")?[1]);
    if out.exists(){return Err("output directory must be new".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    if args.iter().any(|a|a=="--missing-runtime-native") {
        if !cfg!(all(feature="dlss",target_os="windows")){return Err("missing-runtime test requires a feature-enabled Windows build".into());}
        let runtime=std::env::var_os("KERN_DLSS_RUNTIME_DIR").ok_or("set KERN_DLSS_RUNTIME_DIR to a missing directory explicitly")?;
        if PathBuf::from(runtime).join("nvngx_dlss.dll").exists(){return Err("missing-runtime mode must not point at a real DLL".into());}
        let gpu=Gpu::new()?;let native=native_clear(&gpu);let error=DlssContext::new(&gpu).err();let passed=native && error.is_some();
        let message=error.unwrap_or_default();std::fs::write(out.join("report.json"),format!("{{\"native_passed\":{native},\"explicit_dlss_error\":{message:?},\"passed\":{passed}}}")).map_err(|e|e.to_string())?;
        return if passed {Ok(())}else{Err("missing-runtime native behavior failed".into())};
    }
    let gpu=Gpu::new()?;let mut context=DlssContext::new(&gpu)?;let mut sibling=DlssContext::new(&gpu)?;
    let mut cases=Vec::new();let mut passed=true;
    let mut capture=|name:&str,texture:&wgpu::Texture,color:[f32;3]|->Result<(),String>{let (record,ok)=check(&gpu,&out,name,texture,color)?;println!("{name}: {}",if ok{"PASS"}else{"FAIL"});cases.push(record);passed&=ok;Ok(())};
    let single=View {mode:UpscaleMode::DlssQuality,size:(256,144),color:COLORS[0],reset:false};
    let first=frame(&gpu,&mut context,&[single],0)?;capture("first_native_output_not_cleared",&first[0],single.color)?;
    for i in 1..8 {let output=frame(&gpu,&mut context,&[single],i)?;if i==7 {capture("stable_history",&output[0],single.color)?;}}
    let four:Vec<_>=COLORS.iter().map(|color|View {size:(192,128),color:*color,reset:true,..single}).collect();
    let output=frame(&gpu,&mut context,&four,0)?;
    for i in 0..4 {capture(&format!("four_view_{i}"),&output[i],four[i].color)?;}
    for (name,mode,size) in [("balanced",UpscaleMode::DlssBalanced,(256,144)),("performance_odd_resize",UpscaleMode::DlssPerformance,(257,145))] {
        let v=View {mode,size,reset:true,color:COLORS[2]};let output=frame(&gpu,&mut context,&[v],0)?;capture(name,&output[0],v.color)?;
    }
    let before=View {mode:UpscaleMode::DlssQuality,size:(288,160),color:COLORS[1],reset:false};
    let old_output=frame(&gpu,&mut context,&[before],0)?;
    let after=View {size:(289,161),color:COLORS[3],reset:true,..before};
    let new_output=frame(&gpu,&mut context,&[after],1)?;
    // No readback/poll separates these two submissions. Old resources must
    // remain live while feature dimensions and input image handles are replaced.
    capture("inflight_resize_old_output",&old_output[0],before.color)?;
    capture("inflight_resize_new_output",&new_output[0],after.color)?;
    let reset=View {color:COLORS[0],reset:true,..after};let output=frame(&gpu,&mut context,&[reset],2)?;capture("explicit_history_reset",&output[0],reset.color)?;
    let sibling_view=View {color:COLORS[2],..single};let sibling_output=frame(&gpu,&mut sibling,&[sibling_view],0)?;
    drop(context);capture("other_context_survives_drop",&sibling_output[0],sibling_view.color)?;
    let sibling_output=frame(&gpu,&mut sibling,&[sibling_view],1)?;capture("other_context_evaluates_after_drop",&sibling_output[0],sibling_view.color)?;
    drop(sibling);drop(capture);
    let native=native_clear(&gpu);passed&=native;
    cases.push(format!("{{\"name\":\"ordinary_wgpu_after_ngx_drop\",\"passed\":{native}}}"));
    std::fs::write(out.join("report.json"),format!("{{\"schema\":1,\"scope\":\"Actual NGX constant-radiance lifecycle and resource preservation; not scene quality, jitter convergence, motion accuracy or performance\",\"gpu\":{:?},\"cases\":[{}],\"passed\":{passed}}}",gpu.info.name,cases.join(",\n"))).map_err(|e|e.to_string())?;
    if passed {Ok(())}else{Err("DLSS lifecycle oracle failed; see report.json".into())}
}
fn main(){if let Err(error)=run(){eprintln!("{error}");std::process::exit(1);}}
