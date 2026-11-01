//! A tiny, seamless material field baked once on the CPU. Minimum lava uses
//! one filtered colour lookup; palette and crack shaping are baked on the CPU.
use crate::Gpu;
const SIZE:u32=512;
const CELLS:i32=8;
fn hash(x:i32,y:i32,seed:u32)->f32 {
    let mut n=(x.rem_euclid(CELLS) as u32).wrapping_mul(0x8da6_b343)
        ^(y.rem_euclid(CELLS) as u32).wrapping_mul(0xd816_3841)^seed;
    n^=n>>16;n=n.wrapping_mul(0x7feb_352d);n^=n>>15;
    (n&0xffff) as f32/65535.0
}
fn field(x:f32,y:f32)->[u8;4] {
    let (cx,cy)=(x.floor() as i32,y.floor() as i32);
    let mut first=f32::INFINITY;let mut second=f32::INFINITY;let mut grain=0.0;
    for j in cy-1..=cy+1 {for i in cx-1..=cx+1 {
        let px=i as f32+0.2+0.6*hash(i,j,90210);let py=j as f32+0.2+0.6*hash(i,j,67123);
        let d=((x-px).powi(2)+(y-py).powi(2)).sqrt();
        if d<first {second=first;first=d;grain=hash(i,j,7639);}else{second=second.min(d);}
    }}
    let smooth=|t:f32|t*t*(3.0-2.0*t);
    let (u,v)=(smooth(x-x.floor()),smooth(y-y.floor()));
    let mix=|a:f32,b:f32,t:f32|a+(b-a)*t;
    let flow=mix(mix(hash(cx,cy,5147),hash(cx+1,cy,5147),u),mix(hash(cx,cy+1,5147),hash(cx+1,cy+1,5147),u),v);
    [(second-first).clamp(0.0,1.0),grain,flow,1.0].map(|v|(v*255.0).round() as u8)
}
fn colour(x:f32,y:f32)->[u8;4] {
    let b=field(y*2.0+1.3,x*2.0+0.7).map(|v|v as f32/255.);
    let a=field(x+(b[2]-0.5)*0.36,y+(b[0]-0.3)*0.36).map(|v|v as f32/255.);
    let smooth=|lo:f32,hi:f32,v:f32|{let t=((v-lo)/(hi-lo)).clamp(0.,1.);t*t*(3.-2.*t)};
    let edge=a[0]+(b[2]-0.5)*0.055;let rim=1.-smooth(0.06,0.28,edge);let core=1.-smooth(0.015,0.085,edge);
    let mut out=[0u8;4];
    for i in 0..3 {let crust=[0.006,0.002,0.001][i]*(0.45+a[1]*0.5+b[2]*1.1);let red=crust+([0.65,0.035,0.002][i]-crust)*rim;
        let c=red+([3.,0.75,0.065][i]*(0.8+b[2]*0.4)-red)*core;out[i]=(c/4.*255.).round().clamp(0.,255.) as u8;}
    out[3]=255;out
}
fn levels()->&'static Vec<Vec<u8>> {
    static LEVELS:std::sync::OnceLock<Vec<Vec<u8>>>=std::sync::OnceLock::new();
    LEVELS.get_or_init(||{
        let mut data=Vec::with_capacity((SIZE*SIZE*4) as usize);
        for y in 0..SIZE {for x in 0..SIZE {data.extend(colour((x as f32+0.5)*CELLS as f32/SIZE as f32,(y as f32+0.5)*CELLS as f32/SIZE as f32));}}
        let mut levels=vec![data];let mut size=SIZE as usize;
        while size>1 {
            let old=levels.last().unwrap();let next=size/2;let mut pixels=Vec::with_capacity(next*next*4);
            for y in 0..next {for x in 0..next {for channel in 0..4 {
                let sum:u32=[(0,0),(1,0),(0,1),(1,1)].iter().map(|(dx,dy)|old[((2*y+dy)*size+2*x+dx)*4+channel] as u32).sum();
                pixels.push(((sum+2)/4) as u8);
            }}}
            // Far cracks converge to a warm average rather than a yellow sheet.
            let fade=((levels.len() as f32-4.)/3.).clamp(0.,1.);
            for pixel in pixels.chunks_exact_mut(4) {for c in 0..3 {pixel[c]=(pixel[c] as f32*(1.-fade)+[20.,4.,1.][c]*fade).round() as u8;}}
            levels.push(pixels);size=next;
        }
        levels
    })
}
pub fn texture(gpu:&Gpu)->(wgpu::TextureView,wgpu::Sampler) {
    let (size,count)=(SIZE,SIZE.ilog2()+1);
    let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("minimum lava field"),size:wgpu::Extent3d{width:size,height:size,depth_or_array_layers:1},mip_level_count:count,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
    for level in 0..count {let side=size>>level;
        let data=levels()[level as usize].as_slice();
        gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&texture,mip_level:level,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},data,wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(side*4),rows_per_image:Some(side)},wgpu::Extent3d{width:side,height:side,depth_or_array_layers:1});
    }
    let sampler=gpu.device.create_sampler(&wgpu::SamplerDescriptor{label:Some("minimum lava repeat"),address_mode_u:wgpu::AddressMode::Repeat,address_mode_v:wgpu::AddressMode::Repeat,mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,mipmap_filter:wgpu::MipmapFilterMode::Linear,..Default::default()});
    (texture.create_view(&Default::default()),sampler)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn baked_field_tiles_and_stays_bounded() {
        for (x,y) in [(0.13,0.71),(3.9,2.1),(-1.2,5.7)] {
            let a=field(x,y);let b=field(x+CELLS as f32,y-CELLS as f32);
            assert!(a.iter().zip(b).all(|(a,b)|a.abs_diff(b)<=1));
        }
        assert_eq!(levels().len(),10);assert_eq!(levels().last().unwrap().len(),4);
        assert!(levels().iter().map(Vec::len).sum::<usize>()<1_400_000);
    }
}
