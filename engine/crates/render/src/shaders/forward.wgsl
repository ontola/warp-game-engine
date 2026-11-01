struct Spot {position:vec4<f32>,direction:vec4<f32>,intensity:vec4<f32>};
struct Uniform {vp:mat4x4<f32>,camera:vec4<f32>,sun:vec4<f32>,sun_color:vec4<f32>,ambient:vec4<f32>,fog:vec4<f32>,spots:array<Spot,4>};
struct Material {tint:vec3<f32>,layer:i32,roughness:f32,flags:u32,scale:f32,metal:f32};
@group(0) @binding(0) var<uniform> u:Uniform;
@group(0) @binding(1) var<storage,read> materials:array<Material>;
@group(0) @binding(2) var lava_field:texture_2d<f32>;
@group(0) @binding(3) var lava_sampler:sampler;
struct Out {@builtin(position) pos:vec4<f32>,@location(0) color:vec3<f32>,@location(1) world:vec3<f32>,@location(2) @interpolate(flat) lava:vec4<f32>};
@vertex fn vs(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,
 @location(3) r0:vec4<f32>,@location(4) r1:vec4<f32>,@location(5) r2:vec4<f32>,@location(6) material:u32,@location(7) flags:u32)->Out {
 let p=vec4<f32>(position,1.0);let world=vec3<f32>(dot(r0,p),dot(r1,p),dot(r2,p));
 let n=normalize(vec3<f32>(dot(r0.xyz,normal),dot(r1.xyz,normal),dot(r2.xyz,normal)));
 let m=materials[material];
 var illumination=vec3<f32>(u.ambient.x*(0.2+0.3*max(n.y,0.0)))+u.sun_color.rgb*u.sun.w*max(dot(n,u.sun.xyz),0.0);
 for(var i=0u;i<u32(u.ambient.y);i++) {let lamp=u.spots[i];let offset=world-lamp.position.xyz;let d=length(offset);
  if d>0.05 && d<lamp.position.w {let outward=offset/d;let cone=smoothstep(lamp.direction.w,lamp.intensity.w,dot(outward,lamp.direction.xyz));
   illumination+=lamp.intensity.rgb*(cone*max(dot(n,-outward),0.0)*(1.0-d/lamp.position.w)/max(d,0.2));}}
 var value=m.tint*illumination/3.14159265;
 if (m.flags&5120u)!=0u {value+=m.tint*m.scale*u.camera.w*u.sun_color.w;}
 let flash=f32((flags>>16u)&255u)/255.0;value=mix(value,vec3<f32>(1.0),flash*0.9);
 if u.ambient.z==2.0 {value=m.tint;}
 if u.ambient.z==11.0 {value=n*0.5+0.5;}
 let mapped=sqrt(max(value,vec3<f32>(0.0))/(1.0+max(value,vec3<f32>(0.0))));
 var out:Out;out.pos=u.vp*vec4<f32>(world,1.0);out.color=mapped;out.world=world;
 out.lava=vec4<f32>(m.tint*select(0.0,m.scale*u.camera.w*u.sun_color.w,(m.flags&4096u)!=0u && u.ambient.z==0.0),flash);return out;
}
// Palette, cellular shaping and the mip chain are baked once. One filtered
// lookup in world space; both eyes and separate lava meshes share the flow.
fn decode_srgb(c:vec3<f32>)->vec3<f32> {return select(c/12.92,pow((c+0.055)/1.055,vec3<f32>(2.4)),c>vec3<f32>(0.04045));}
fn encode_srgb(c:vec3<f32>)->vec3<f32> {return select(c*12.92,1.055*pow(c,vec3<f32>(1.0/2.4))-0.055,c>vec3<f32>(0.0031308));}
fn surface(in:Out,srgb:bool)->vec3<f32> {
 let uv=in.world.xz*0.0625+u.ambient.w*vec2<f32>(0.0018,-0.0012);
 let dx=dpdx(uv);let dy=dpdy(uv);
 if max(in.lava.r,max(in.lava.g,in.lava.b))<=0.0 {if srgb {return decode_srgb(in.color);}return in.color;}
 let radiance=textureSampleGrad(lava_field,lava_sampler,uv,dx,dy).rgb*4.0;
 let value=mix(radiance*in.lava.rgb,vec3<f32>(1.0),in.lava.w*0.9);
 let mapped=value/(1.0+value);
 if srgb {return mapped;}return encode_srgb(mapped);
}
@fragment fn fs(in:Out)->@location(0) vec4<f32>{return vec4<f32>(surface(in,false),1.0);}
@fragment fn fs_srgb(in:Out)->@location(0) vec4<f32>{return vec4<f32>(surface(in,true),1.0);}
