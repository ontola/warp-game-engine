//! Independent GPU motion-input oracle. No NGX evaluation and no window.
//! Run only after stopping other GPU workloads: motion_probe --out NEW_DIRECTORY.
//! Synthetic depth/owner layers isolate reprojection math from visibility and
//! shading. Native layer-production correctness needs the separate scene run.
use bytemuck::Zeroable;
use glam::{Affine3A, Mat4, Quat, Vec2, Vec3, Vec4};
use render::{Gpu, motion::{MotionInputs, MotionOutput, MotionRenderer, MotionView}, scene::{GpuInstance, LIVE_GRASS}};
use render::vegetation_detail::DistanceProfile;
use std::path::PathBuf;
use wgpu::util::DeviceExt;

const SIZE:(u32,u32)=(97,65);

#[derive(Clone,Copy)]
struct Pose { transform:Affine3A, shear:Vec2, flags:u32 }
impl Default for Pose { fn default()->Self {Self {transform:Affine3A::IDENTITY,shear:Vec2::ZERO,flags:0}} }
impl Pose {
    fn gpu(self)->GpuInstance {
        let matrix=Mat4::from(self.transform);
        GpuInstance { rows:[matrix.row(0).to_array(),matrix.row(1).to_array(),matrix.row(2).to_array()],
            sphere:[0.,0.,0.,100.],first_cluster:0,cluster_count:1,scale:1.,flags:self.flags }
    }
    fn world_to_rest(self,world:Vec3)->Vec3 {
        let d=world.y-self.transform.translation.y;
        self.transform.inverse().transform_point3(world-Vec3::new(self.shear.x*d,0.,self.shear.y*d))
    }
    fn rest_to_world(self,local:Vec3)->Vec3 {
        let p=self.transform.transform_point3(local);let d=p.y-self.transform.translation.y;
        p+Vec3::new(self.shear.x*d,0.,self.shear.y*d)
    }
}
#[derive(Clone,Copy)]
struct Camera { position:Vec3, rotation:Quat, jitter:Vec2 }
impl Default for Camera { fn default()->Self {Self {position:Vec3::ZERO,rotation:Quat::IDENTITY,jitter:Vec2::ZERO}} }
impl Camera {
    fn vp(self,size:(u32,u32))->Mat4 {
        Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2,size.0 as f32/size.1 as f32,0.1)
            *Mat4::look_to_rh(self.position,self.rotation*Vec3::NEG_Z,self.rotation*Vec3::Y)
    }
    fn inverse_jittered(self,size:(u32,u32))->Mat4 {
        (Mat4::from_translation(Vec3::new(2.*self.jitter.x/size.0 as f32,-2.*self.jitter.y/size.1 as f32,0.))*self.vp(size)).inverse()
    }
    fn view(self,index:usize,split:bool)->MotionView {MotionView {key:100+index as u64,size:SIZE,
        inverse_jittered_view_proj:self.inverse_jittered(SIZE),view_proj:self.vp(SIZE),camera:self.position,split,
        grass_detail:Default::default(),reset:false}}
}
#[derive(Clone,Copy)]
struct Layers { depth:f32, grass:Option<(usize,u8,f32)>, glass:Option<(usize,u8,f32)>, far:Option<(f32,f32)> }
impl Default for Layers {fn default()->Self {Self {depth:0.01,grass:None,glass:None,far:None}}}

fn texture(gpu:&Gpu,format:wgpu::TextureFormat,size:(u32,u32))->wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("motion oracle input"),size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,
        usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[]})
}
fn upload(gpu:&Gpu,texture:&wgpu::Texture,pixel:&[u8]) {
    let data=pixel.repeat((texture.width()*texture.height())as usize);
    gpu.queue.write_texture(texture.as_image_copy(),&data,wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(texture.width()*pixel.len()as u32),rows_per_image:Some(texture.height())},texture.size());
}
fn half(value:f32)->u16 { // Fixture only: values are finite, normal and nonnegative.
    if value==0. {return 0;}let bits=value.to_bits();let exp=((bits>>23)&255)as i32-127+15;
    assert!((1..31).contains(&exp));((exp as u16)<<10)|((bits>>13)as u16&1023)
}
fn unhalf(bits:u16)->f32 {
    let sign=if bits&0x8000==0 {1.}else{-1.};let exponent=(bits>>10)&31;let mantissa=(bits&1023)as f32;
    match exponent {0=>sign*mantissa*2f32.powi(-24),31=>if mantissa==0. {sign*f32::INFINITY}else{f32::NAN},_=>sign*(1.+mantissa/1024.)*2f32.powi(exponent as i32-15)}
}
struct Inputs {depth:wgpu::TextureView,visibility:wgpu::TextureView,grass:Option<wgpu::TextureView>,glass:Option<wgpu::TextureView>,far:Option<(wgpu::TextureView,wgpu::TextureView)>}
impl Inputs {
    fn new(gpu:&Gpu,layers:Layers)->Self {
        let depth=texture(gpu,wgpu::TextureFormat::Depth32Float,SIZE).create_view(&Default::default());
        let vis=texture(gpu,wgpu::TextureFormat::R32Uint,SIZE);upload(gpu,&vis,&0u32.to_le_bytes());
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        {let _pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("oracle constant depth"),color_attachments:&[],
            depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {view:&depth,depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Clear(layers.depth),store:wgpu::StoreOp::Store}),stencil_ops:None}),
            timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});}
        gpu.queue.submit([encoder.finish()]);
        let metadata=|value:Option<(usize,u8,f32)>|value.map(|(owner,alpha,depth)|{
            let t=texture(gpu,wgpu::TextureFormat::Rg32Uint,SIZE);
            upload(gpu,&t,bytemuck::cast_slice(&[(((owner+1)as u32)<<8)|alpha as u32,depth.to_bits()]));t.create_view(&Default::default())});
        let far=layers.far.map(|(alpha,distance)|{let size=(SIZE.0.div_ceil(2),SIZE.1.div_ceil(2));
            let c=texture(gpu,wgpu::TextureFormat::Rgba16Float,size);let d=texture(gpu,wgpu::TextureFormat::R32Float,size);
            upload(gpu,&c,bytemuck::cast_slice(&[half(0.1),half(0.2),half(0.1),half(alpha)]));upload(gpu,&d,&distance.to_le_bytes());
            (c.create_view(&Default::default()),d.create_view(&Default::default()))});
        Self {depth,visibility:vis.create_view(&Default::default()),grass:metadata(layers.grass),glass:metadata(layers.glass),far}
    }
    fn bindings<'a>(&'a self,visible:&'a wgpu::Buffer)->MotionInputs<'a> {MotionInputs {opaque_depth:&self.depth,visibility:&self.visibility,visible,
        grass:self.grass.as_ref(),transparency:self.glass.as_ref(),far_grass:self.far.as_ref().map(|(c,d)|(c,d))}}
}
struct Fixture {instances:wgpu::Buffer,wind:wgpu::Buffer,visible:wgpu::Buffer,motion:MotionRenderer}
impl Fixture {
    fn new(gpu:&Gpu)->Result<Self,String> {
        let instances=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("motion oracle instances"),contents:bytemuck::cast_slice(&[GpuInstance::zeroed();3]),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC});
        let wind=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("motion oracle wind"),contents:bytemuck::cast_slice(&[[0f32;4];3]),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC});
        let visible=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("motion oracle visible owner"),contents:bytemuck::cast_slice(&[0u32,0]),usage:wgpu::BufferUsages::STORAGE});
        let motion=MotionRenderer::new(gpu,&instances,&wind,3)?;Ok(Self {instances,wind,visible,motion})
    }
    fn poses(&self,gpu:&Gpu,poses:[Pose;3]) {
        gpu.queue.write_buffer(&self.instances,0,bytemuck::cast_slice(&poses.map(Pose::gpu)));
        gpu.queue.write_buffer(&self.wind,0,bytemuck::cast_slice(&poses.map(|p|[p.shear.x,p.shear.y,1.,0.])));
    }
    fn frame(&self,gpu:&Gpu,inputs:&Inputs,cameras:&[Camera])->Result<Vec<MotionOutput>,String> {
        self.frame_with_profile(gpu,inputs,cameras,DistanceProfile::Default)
    }
    fn frame_with_profile(&self,gpu:&Gpu,inputs:&Inputs,cameras:&[Camera],profile:DistanceProfile)->Result<Vec<MotionOutput>,String> {
        let mut encoder=gpu.device.create_command_encoder(&Default::default());let mut outputs=Vec::new();
        for (i,camera) in cameras.iter().enumerate() {
            let mut view=camera.view(i,cameras.len()>1);view.grass_detail=profile;
            outputs.push(self.motion.render(gpu,&mut encoder,i,view,inputs.bindings(&self.visible),None)?);
        }
        self.motion.finish_frame(&mut encoder);gpu.queue.submit([encoder.finish()]);Ok(outputs)
    }
}

// Oracle deliberately uses glam affine inversion and independent projection,
// not the shader's cofactor inversion or the library's pixel_motion helper.
fn expected(camera:Camera,old_camera:Camera,now:Pose,old:Pose,x:u32,y:u32,depth:f32)->Vec2 {
    let clip=Vec4::new((x as f32+0.5)/SIZE.0 as f32*2.-1.,1.-(y as f32+0.5)/SIZE.1 as f32*2.,depth,1.);
    let homogeneous=camera.inverse_jittered(SIZE)*clip;
    let (current,previous)=if depth==0. {(homogeneous.truncate().extend(0.),homogeneous.truncate().extend(0.))}
        else {let world=homogeneous.truncate()/homogeneous.w;(world.extend(1.),old.rest_to_world(now.world_to_rest(world)).extend(1.))};
    projected(camera.vp(SIZE),old_camera.vp(SIZE),current,previous)
}
fn projected(current:Mat4,previous:Mat4,world:Vec4,old:Vec4)->Vec2 {
    let a=current*world;let b=previous*old;
    Vec2::new((b.x/b.w-a.x/a.w)*SIZE.0 as f32*0.5,(a.y/a.w-b.y/b.w)*SIZE.1 as f32*0.5)
}
fn far_expected(camera:Camera,old:Camera,x:u32,y:u32,horizontal:f32)->(Vec2,f32) {
    let cx=((x/2*2)as f32+1.).min(SIZE.0 as f32-0.5);let cy=((y/2*2)as f32+1.).min(SIZE.1 as f32-0.5);
    let h=camera.inverse_jittered(SIZE)*Vec4::new(cx/SIZE.0 as f32*2.-1.,1.-cy/SIZE.1 as f32*2.,0.,1.);
    let direction=h.truncate().normalize();let ray_distance=horizontal/Vec2::new(direction.x,direction.z).length();
    let point=(camera.position+direction*ray_distance).extend(1.);let clip=camera.vp(SIZE)*point;
    (projected(camera.vp(SIZE),old.vp(SIZE),point,point),clip.z/clip.w)
}
struct Record {name:String,max_error:f32,max_depth_error:f32,samples:usize,passed:bool}
fn check(gpu:&Gpu,name:&str,output:&MotionOutput,oracle:impl Fn(u32,u32)->(Vec2,f32),bias:Option<f32>,reset:bool)->Record {
    // All three output formats are exactly four bytes per pixel. The existing
    // raw-copy helper does not reinterpret them; decode their actual types here.
    let vectors=gpu.read_rgba8(&output.vectors);let depths=gpu.read_rgba8(&output.depth);let biases=gpu.read_rgba8(&output.bias);
    let mut result=Record {name:name.into(),max_error:0.,max_depth_error:0.,samples:0,passed:output.reset==reset};
    for y in (12..SIZE.1-12).step_by(3) {for x in (18..SIZE.0-18).step_by(3) {
        let i=((y*SIZE.0+x)*4)as usize;let actual=Vec2::new(unhalf(u16::from_le_bytes(vectors[i..i+2].try_into().unwrap())),unhalf(u16::from_le_bytes(vectors[i+2..i+4].try_into().unwrap())));
        let depth=f32::from_le_bytes(depths[i..i+4].try_into().unwrap());let actual_bias=f32::from_le_bytes(biases[i..i+4].try_into().unwrap());
        let (expected,expected_depth)=oracle(x,y);let error=(actual-expected).abs().max_element();let depth_error=(depth-expected_depth).abs();
        result.max_error=result.max_error.max(error);result.max_depth_error=result.max_depth_error.max(depth_error);result.samples+=1;
        result.passed&=actual.is_finite() && depth.is_finite() && actual_bias.is_finite() && error<0.025+expected.abs().max_element()*0.001 && depth_error<0.000002;
        if let Some(bias)=bias {result.passed&=(actual_bias-bias).abs()<0.002;}
    }}result
}
fn run()->Result<(),String> {
    let args=std::env::args().collect::<Vec<_>>();let out=PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("use --out NEW_DIRECTORY")?[1]);
    if out.exists(){return Err("output directory must be new".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;let mut records=Vec::new();let base=[Pose::default();3];let camera=Camera::default();
    let mut scenarios=Vec::new();
    scenarios.push(("static_jitter",base,base,camera,Camera {jitter:Vec2::new(0.375,-0.25),..camera},Layers::default(),false));
    let mut translated=base;translated[0].transform=Affine3A::from_translation(Vec3::new(0.7,-0.2,0.3));
    scenarios.push(("car_translation",base,translated,camera,camera,Layers::default(),false));
    let mut old_wheel=base;old_wheel[0].transform=Affine3A::from_scale_rotation_translation(Vec3::new(1.2,0.8,1.1),Quat::from_rotation_y(0.12),Vec3::new(0.,-0.6,-10.));
    let mut wheel=old_wheel;wheel[0].transform=Affine3A::from_scale_rotation_translation(Vec3::new(1.2,0.8,1.1),Quat::from_rotation_x(0.18)*Quat::from_rotation_y(0.24),Vec3::new(0.1,-0.6,-10.));
    scenarios.push(("wheel_rotation_nonuniform",old_wheel,wheel,camera,camera,Layers::default(),false));
    let mut old_wind=base;old_wind[0].shear=Vec2::new(-0.02,0.03);let mut new_wind=old_wind;new_wind[0].shear=Vec2::new(0.12,-0.08);
    scenarios.push(("previous_current_wind",old_wind,new_wind,camera,camera,Layers::default(),false));
    let moved_camera=Camera {position:Vec3::new(0.4,0.2,0.),rotation:Quat::from_rotation_y(0.02),jitter:Vec2::new(-0.2,0.3)};
    scenarios.push(("camera_with_jitter",base,base,camera,moved_camera,Layers::default(),false));
    scenarios.push(("sky_translation_free",base,base,camera,Camera {position:Vec3::new(2.,1.,3.),..camera},Layers {depth:0.,..Layers::default()},false));
    scenarios.push(("sky_rotation",base,base,camera,Camera {position:Vec3::new(1.,0.,0.),rotation:Quat::from_rotation_y(0.12),..camera},Layers {depth:0.,..Layers::default()},false));
    let mut old_grass=base;old_grass[0].flags=LIVE_GRASS;let mut recycled=old_grass;recycled[0].transform=Affine3A::from_translation(Vec3::new(0.45,0.,0.));
    scenarios.push(("recycled_grass_reject",old_grass,recycled,camera,camera,Layers::default(),true));
    let mut singular=base;singular[0].transform=Affine3A::from_scale(Vec3::new(0.,1.,1.));
    scenarios.push(("singular_owner_reject",base,singular,camera,camera,Layers::default(),true));
    for (name,old,now,previous,current,layers,reject) in scenarios {
        let f=Fixture::new(&gpu)?;let inputs=Inputs::new(&gpu,layers);f.poses(&gpu,old);let first=f.frame(&gpu,&inputs,&[previous])?;
        if records.is_empty(){records.push(check(&gpu,"first_frame_reset",&first[0],|_,_|(Vec2::ZERO,layers.depth),Some(1.),true));}
        f.poses(&gpu,now);let frame=f.frame(&gpu,&inputs,&[current])?;
        records.push(check(&gpu,name,&frame[0],|x,y| (if reject {Vec2::ZERO}else{expected(current,previous,now[0],old[0],x,y,layers.depth)},layers.depth),Some(if reject {1.}else{0.}),false));
    }
    // All cameras prime their previous VP before the one shared pose snapshot.
    let f=Fixture::new(&gpu)?;let inputs=Inputs::new(&gpu,Layers::default());let old_cameras:Vec<_>=(0..4).map(|i|Camera {position:Vec3::new(i as f32*0.25,0.,0.),rotation:Quat::from_rotation_y(i as f32*0.03),..camera}).collect();
    let cameras:Vec<_>=old_cameras.iter().enumerate().map(|(i,c)|Camera {position:c.position+Vec3::new(0.1*i as f32,0.05,0.),jitter:Vec2::new(i as f32*0.1,-0.2),..*c}).collect();
    f.poses(&gpu,base);f.frame(&gpu,&inputs,&old_cameras)?;f.poses(&gpu,translated);let frames=f.frame(&gpu,&inputs,&cameras)?;
    for i in 0..4 {records.push(check(&gpu,&format!("independent_view_{i}"),&frames[i],|x,y|(expected(cameras[i],old_cameras[i],translated[0],base[0],x,y,0.01),0.01),Some(0.),false));}
    // Ambiguous near layers choose the strongest contribution and reject some
    // history. Synthetic metadata verifies owner decoding and merged depth.
    for (name,grass,glass,owner,depth) in [
        ("grass_dominant",Some((1,204,0.02)),None,1,0.02),
        ("faint_glass_over_grass",Some((1,204,0.02)),Some((2,26,0.03)),1,0.02),
        ("glass_dominant",Some((1,204,0.02)),Some((2,204,0.03)),2,0.03),
        ("faint_grass_ground_dominant",Some((1,51,0.02)),None,0,0.01)] {
        let f=Fixture::new(&gpu)?;let inputs=Inputs::new(&gpu,Layers {grass,glass,..Layers::default()});
        let mut poses=base;poses[1].transform=Affine3A::from_translation(Vec3::new(0.4,0.,0.));poses[2].transform=Affine3A::from_translation(Vec3::new(-0.6,0.,0.));
        f.poses(&gpu,base);f.frame(&gpu,&inputs,&[camera])?;f.poses(&gpu,poses);let frame=f.frame(&gpu,&inputs,&[camera])?;
        let bias=grass.into_iter().chain(glass).map(|(_,a,_)|{let a=a as f32/255.;4.*a*(1.-a)}).fold(0.,f32::max);
        records.push(check(&gpu,name,&frame[0],|x,y|(expected(camera,camera,poses[owner],base[owner],x,y,depth),depth),Some(bias),false));
    }
    // Identical layer inputs, different source/volume distance policy. With no
    // near metadata, Extended still weights the source representation here,
    // while Default has already reached pure far grass. The resulting motion
    // owner must differ, not just the reported profile label. Literal physical
    // endpoints and the cubic polynomial are independent of renderer helpers.
    for (name,views,distance,start,end,default_end) in [
        ("single",1,90.0f32,80.0f32,120.0f32,60.0f32),
        ("split",4,60.0f32,50.0f32,80.0f32,38.0f32)] {
        let layers=Layers {depth:0.0005,far:Some((0.8,distance)),..Layers::default()};
        let inputs=Inputs::new(&gpu,layers);let cameras=vec![camera;views];
        let mut moved=base;moved[0].transform=Affine3A::from_translation(Vec3::new(2.0,0.,0.));
        let fraction=(distance-start)/(end-start);
        let volume_weight=fraction*fraction*(3.0-2.0*fraction);
        let far_alpha=unhalf(half(0.8));
        let extended_contribution=far_alpha*volume_weight;
        assert!(fraction>0.0 && fraction<1.0 && extended_contribution<0.5);
        assert!(distance>default_end && far_alpha>=0.5);
        for (profile,contribution,far_dominates) in [
            (DistanceProfile::Default,far_alpha,true),
            (DistanceProfile::Extended,extended_contribution,false)] {
            let f=Fixture::new(&gpu)?;
            f.poses(&gpu,base);f.frame_with_profile(&gpu,&inputs,&cameras,profile)?;
            f.poses(&gpu,moved);let frame=f.frame_with_profile(&gpu,&inputs,&cameras,profile)?;
            records.push(check(&gpu,&format!("{}_{}_distance_dominance",profile.label(),name),&frame[0],|x,y| {
                if far_dominates {far_expected(camera,camera,x,y,distance)}
                else {(expected(camera,camera,moved[0],base[0],x,y,layers.depth),layers.depth)}
            },Some(contribution),false));
        }
    }
    let f=Fixture::new(&gpu)?;let layers=Layers {depth:0.0001,far:Some((0.8,100.)),..Layers::default()};let inputs=Inputs::new(&gpu,layers);
    let old=Camera {position:Vec3::new(0.,20.,0.),rotation:Quat::from_rotation_x(-0.45),..camera};let current=Camera {position:old.position+Vec3::new(1.,0.4,0.),jitter:Vec2::new(0.2,-0.3),..old};
    f.poses(&gpu,base);f.frame(&gpu,&inputs,&[old])?;let frame=f.frame(&gpu,&inputs,&[current])?;
    records.push(check(&gpu,"far_grass_horizontal_centroid",&frame[0],|x,y|far_expected(current,old,x,y,100.),Some(unhalf(half(0.8))),false));
    // A large camera jump must invalidate the whole view; a pose alone must not.
    let cut=Camera {position:current.position+Vec3::new(21.,0.,0.),..current};let frame=f.frame(&gpu,&inputs,&[cut])?;
    records.push(check(&gpu,"camera_cut_reset",&frame[0],|x,y|(Vec2::ZERO,far_expected(cut,cut,x,y,100.).1),Some(1.),true));
    f.motion.reset();let frame=f.frame(&gpu,&inputs,&[current])?;
    records.push(check(&gpu,"explicit_reset",&frame[0],|x,y|(Vec2::ZERO,far_expected(current,current,x,y,100.).1),Some(1.),true));
    let mut changed_identity=current.view(0,false);changed_identity.key=200;
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    let identity=f.motion.render(&gpu,&mut encoder,0,changed_identity,inputs.bindings(&f.visible),None)?;
    f.motion.finish_frame(&mut encoder);gpu.queue.submit([encoder.finish()]);
    records.push(check(&gpu,"camera_identity_reset",&identity,|x,y|(Vec2::ZERO,far_expected(current,current,x,y,100.).1),Some(1.),true));
    let passed=records.iter().all(|r|r.passed);
    let cases=records.iter().map(|r|format!("{{\"name\":\"{}\",\"samples\":{},\"max_vector_error_px\":{},\"max_depth_error\":{},\"passed\":{}}}",r.name,r.samples,r.max_error,r.max_depth_error,r.passed)).collect::<Vec<_>>().join(",\n");
    std::fs::write(out.join("report.json"),format!("{{\"schema\":1,\"scope\":\"Synthetic actual-GPU motion pass vs independent affine CPU oracle; not native layer production, NGX output quality or performance\",\"size\":[{},{}],\"float32_filterable_enabled\":{},\"cases\":[{}],\"passed\":{}}}",SIZE.0,SIZE.1,gpu.features.contains(wgpu::Features::FLOAT32_FILTERABLE),cases,passed)).map_err(|e|e.to_string())?;
    for r in &records {println!("{}: {} ({} samples, vector error {:.6} px)",r.name,if r.passed {"PASS"}else{"FAIL"},r.samples,r.max_error);}
    if passed {Ok(())}else{Err("motion oracle failed; see report.json".into())}
}
fn main(){if let Err(error)=run(){eprintln!("{error}");std::process::exit(1);}}
