//! A seamless stone detail field baked once on the CPU, for the minimum
//! forward renderer: red is mottling and grain around 0.5, green is cracks.
//! One filtered read in world space brightens or darkens the vertex colour.
use crate::Gpu;
const SIZE:u32=256;
/// Cells across the tile, for the fracture pattern.
const CELLS:i32=6;
fn hash(x:i32,y:i32,period:i32,seed:u32)->f32 {
    let mut n=(x.rem_euclid(period) as u32).wrapping_mul(0x8da6_b343)
        ^(y.rem_euclid(period) as u32).wrapping_mul(0xd816_3841)^seed;
    n^=n>>16;n=n.wrapping_mul(0x7feb_352d);n^=n>>15;
    (n&0xffff) as f32/65535.0
}
/// Smooth value noise, tiling at `period` cells.
fn noise(x:f32,y:f32,period:i32,seed:u32)->f32 {
    let (cx,cy)=(x.floor() as i32,y.floor() as i32);
    let smooth=|t:f32|t*t*(3.0-2.0*t);
    let (u,v)=(smooth(x-x.floor()),smooth(y-y.floor()));
    let mix=|a:f32,b:f32,t:f32|a+(b-a)*t;
    mix(mix(hash(cx,cy,period,seed),hash(cx+1,cy,period,seed),u),mix(hash(cx,cy+1,period,seed),hash(cx+1,cy+1,period,seed),u),v)
}
/// Distance to the nearest edge between cells, roughly 0 at a crack.
fn cell_edge(x:f32,y:f32)->f32 {
    let (cx,cy)=(x.floor() as i32,y.floor() as i32);
    let (mut first,mut second)=(f32::INFINITY,f32::INFINITY);
    for j in cy-1..=cy+1 {for i in cx-1..=cx+1 {
        let px=i as f32+0.15+0.7*hash(i,j,CELLS,90210);let py=j as f32+0.15+0.7*hash(i,j,CELLS,67123);
        let d=((x-px).powi(2)+(y-py).powi(2)).sqrt();
        if d<first {second=first;first=d;}else{second=second.min(d);}
    }}
    second-first
}
/// At `u`, `v` in 0..1 across the tile.
fn texel(u:f32,v:f32)->[u8;4] {
    let n=|cells:i32,seed:u32|noise(u*cells as f32,v*cells as f32,cells,seed);
    let mottle=0.5*n(4,11)+0.3*n(9,23)+0.2*n(23,37);
    let grain=n(64,51)*0.6+n(128,73)*0.4;
    let bright=(0.5+(mottle-0.5)*1.5+(grain-0.5)*0.5).clamp(0.0,1.0);
    // Wobble the fractures so they do not read as straight cell walls.
    let (wx,wy)=((n(12,91)-0.5)*0.22,(n(12,97)-0.5)*0.22);
    let edge=cell_edge(u*CELLS as f32+wx*CELLS as f32*0.3,v*CELLS as f32+wy*CELLS as f32*0.3);
    let crack=(1.0-edge/0.07).clamp(0.0,1.0);
    [(bright*255.0).round() as u8,(crack*255.0).round() as u8,0,255]
}
fn levels()->Vec<Vec<u8>> {
    let mut data=Vec::with_capacity((SIZE*SIZE*4) as usize);
    for y in 0..SIZE {for x in 0..SIZE {data.extend(texel((x as f32+0.5)/SIZE as f32,(y as f32+0.5)/SIZE as f32));}}
    let mut levels=vec![data];let mut size=SIZE as usize;
    while size>1 {
        let old=levels.last().unwrap();let next=size/2;let mut pixels=Vec::with_capacity(next*next*4);
        for y in 0..next {for x in 0..next {for channel in 0..4 {
            let sum:u32=[(0,0),(1,0),(0,1),(1,1)].iter().map(|(dx,dy)|old[((2*y+dy)*size+2*x+dx)*4+channel] as u32).sum();
            pixels.push(((sum+2)/4) as u8);
        }}}
        levels.push(pixels);size=next;
    }
    levels
}
pub fn texture(gpu:&Gpu)->wgpu::TextureView {
    let levels=levels();
    let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("minimum stone field"),size:wgpu::Extent3d{width:SIZE,height:SIZE,depth_or_array_layers:1},mip_level_count:levels.len() as u32,sample_count:1,
        dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
    for (level,data) in levels.iter().enumerate() {
        let side=SIZE>>level;
        gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&texture,mip_level:level as u32,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},data,
            wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(side*4),rows_per_image:Some(side)},wgpu::Extent3d{width:side,height:side,depth_or_array_layers:1});
    }
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn the_field_tiles_and_averages_near_neutral() {
        let edge=(0..SIZE).map(|i|{let t=(i as f32+0.5)/SIZE as f32;
            (texel(0.001,t)[0] as i32-texel(0.999,t)[0] as i32).abs()}).max().unwrap();
        assert!(edge<48,"seam {edge}");
        let top=levels().pop().unwrap();
        assert!((90..=165).contains(&top[0]),"a far surface keeps its colour: {}",top[0]);
        assert!(top[1]<96,"cracks are thin: {}",top[1]);
    }
}
