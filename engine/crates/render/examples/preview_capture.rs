//! Exclusive-GPU preview readback acceptance. No game or render-quality change.
use render::{Gpu, hud::{HudDrawList,HudRenderer,TextRun,FontFace,TextAlign}, present::Presenter};
use wgpu::util::DeviceExt;
use std::path::PathBuf;

const W:u32=512;
const H:u32=288;
const SMALL_W:u32=137;

fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);}}

fn bilinear(bytes:&[u8],w:u32,h:u32,out_w:u32,out_h:u32)->Vec<u8> {
    let mut out=Vec::with_capacity((out_w*out_h*4)as usize);
    for y in 0..out_h {for x in 0..out_w {
        let sx=(x as f64+0.5)*w as f64/out_w as f64-0.5;
        let sy=(y as f64+0.5)*h as f64/out_h as f64-0.5;
        let ix=sx.floor()as i64;let iy=sy.floor()as i64;
        let fx=sx-ix as f64;let fy=sy-iy as f64;
        for c in 0..4 {
            let at=|dx:i64,dy:i64|bytes[((iy+dy).clamp(0,h as i64-1)as u32*w+(ix+dx).clamp(0,w as i64-1)as u32)as usize*4+c]as f64;
            out.push(((at(0,0)*(1.-fx)+at(1,0)*fx)*(1.-fy)+(at(0,1)*(1.-fx)+at(1,1)*fx)*fy).round().clamp(0.,255.)as u8);
        }
    }}out
}
fn max_error(a:&[u8],b:&[u8])->u8 {assert_eq!(a.len(),b.len());a.iter().zip(b).map(|(a,b)|a.abs_diff(*b)).max().unwrap_or(0)}
fn readback_bytes(w:u32,h:u32)->u64 {u64::from((w*4).div_ceil(256)*256)*u64::from(h)}

fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists(){return Err("fresh output directory required".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    // Independent red/green gradients and a blue midtone expose crop, swizzle,
    // missing sRGB decode and accidental double decode/encode separately.
    let rgba:Vec<_>=(0..H).flat_map(|y|(0..W).flat_map(move|x|[
        (20+x*210/(W-1))as u8,(30+y*190/(H-1))as u8,(80+(x+y)%31)as u8,255])).collect();
    let source=gpu.device.create_texture_with_data(&gpu.queue,&wgpu::TextureDescriptor {
        label:Some("preview encoded RGB gradient"),size:wgpu::Extent3d{width:W,height:H,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
        usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]},Default::default(),&rgba);
    let mut cases=Vec::new();
    for (name,format) in [("rgba",wgpu::TextureFormat::Rgba8Unorm),("rgba-srgb",wgpu::TextureFormat::Rgba8UnormSrgb),
        ("bgra",wgpu::TextureFormat::Bgra8Unorm),("bgra-srgb",wgpu::TextureFormat::Bgra8UnormSrgb)] {
        let presenter=Presenter::new(&gpu,format);
        let mut hud=HudRenderer::new(&gpu,format)?;
        let mut draw=HudDrawList::new([W,H]);let clip=[0.,0.,W as f32,H as f32];
        for (rect,color) in [([8.,8.,56.,40.],[1.,0.,0.,1.]),([W as f32-64.,8.,56.,40.],[0.,1.,0.,1.]),
            ([8.,H as f32-48.,56.,40.],[0.,0.,1.,1.]),([W as f32-64.,H as f32-48.,56.,40.],[1.,1.,0.,1.]),
            ([120.,100.,160.,52.],[0.7,0.2,0.8,0.5])] {draw.rect(rect,color,clip);}
        draw.text.push(TextRun{id:1,text:"FULL HUD 1 2 3 4".into(),position:[80.,H as f32-46.],bounds:clip,
            font_size:28.,color:[255,255,255,255],font:FontFace::Body,align:TextAlign::Left,width:350.});
        hud.prepare(&gpu,&draw)?;
        let (nw,nh,native)=presenter.capture_rgba_scaled(&gpu,&source,format,None,0)?;
        assert_eq!((nw,nh),(W,H));let native_error=max_error(&native,&rgba);
        if native_error>1{return Err(format!("{name}: native gradient changed by {native_error}"));}
        let (w,h,small)=presenter.capture_rgba_scaled(&gpu,&source,format,None,SMALL_W)?;
        assert_eq!((w,h),(SMALL_W,77));
        let no_hud_error=max_error(&small,&bilinear(&native,W,H,w,h));
        if no_hud_error>2{return Err(format!("{name}: scaled gradient mismatch {no_hud_error}"));}
        let (full_w,full_h,full_hud)=presenter.capture_rgba_scaled(&gpu,&source,format,Some(&hud),0)?;
        assert_eq!((full_w,full_h),(W,H));
        let (hud_w,hud_h,small_hud)=presenter.capture_rgba_scaled(&gpu,&source,format,Some(&hud),SMALL_W)?;
        assert_eq!((hud_w,hud_h),(w,h));
        let hud_error=max_error(&small_hud,&bilinear(&full_hud,W,H,w,h));
        // Hardware interpolation precision and sRGB round trips may each round
        // by one byte. This still strongly rejects a double gamma conversion.
        if hud_error>3{return Err(format!("{name}: scaled full-HUD mismatch {hud_error}"));}
        let expected=[[255,0,0],[0,255,0],[0,0,255],[255,255,0]];
        let centers=[(36.,28.),(W as f32-36.,28.),(36.,H as f32-28.),(W as f32-36.,H as f32-28.)];
        for (i,(x,y)) in centers.into_iter().enumerate() {
            let x=(x*w as f32/W as f32)as u32;let y=(y*h as f32/H as f32)as u32;let p=((y*w+x)*4)as usize;
            if max_error(&small_hud[p..p+3],&expected[i])>2{return Err(format!("{name}: HUD corner {i} missing/cropped"));}
        }
        let (large_w,large_h,large)=presenter.capture_rgba_scaled(&gpu,&source,format,None,W*2)?;
        assert_eq!((large_w,large_h),(W,H));assert_eq!(large,native);
        let native_bytes=readback_bytes(W,H);let scaled_bytes=readback_bytes(w,h);
        assert!(scaled_bytes<native_bytes/8);
        render::write_png(&out.join(format!("{name}-native-hud.png")),W,H,&full_hud)?;
        render::write_png(&out.join(format!("{name}-preview-hud.png")),w,h,&small_hud)?;
        render::write_png(&out.join(format!("{name}-preview.png")),w,h,&small)?;
        cases.push(format!("{{\"format\":\"{name}\",\"native_error\":{native_error},\"scaled_error\":{no_hud_error},\"hud_scaled_error\":{hud_error},\"native_size\":[{W},{H}],\"preview_size\":[{w},{h}],\"native_readback_bytes\":{native_bytes},\"preview_readback_bytes\":{scaled_bytes},\"four_hud_corners_visible\":true}}"));
    }
    std::fs::write(out.join("report.json"),format!("{{\"passed\":true,\"scope\":\"presentation and preview correctness; not gameplay FPS\",\"cases\":[{}]}}",cases.join(","))).map_err(|e|e.to_string())?;
    println!("preview capture PASS: four formats, gradient/gamma, complete HUD, bounded downsample");Ok(())
}
