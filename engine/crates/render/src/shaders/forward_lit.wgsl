// The low tier (KERN_FORWARD=1, forward.rs `Lit`): one forward pass for
// tile GPUs. Per pixel: the material's colour and albedo texture, the room's
// baked irradiance (one probe volume in a 3D texture: three filtered taps),
// the local lights unshadowed, emission, then exposure and AgX straight into
// the display target. Nothing per pixel goes through main memory but the
// final colour: depth and colour stay in the GPU's tile memory.
struct Spot {position:vec4<f32>,direction:vec4<f32>,intensity:vec4<f32>};
struct Uniform {vp:mat4x4<f32>,camera:vec4<f32>,sun:vec4<f32>,sun_color:vec4<f32>,ambient:vec4<f32>,fog:vec4<f32>,spots:array<Spot,4>};
struct Material {tint:vec3<f32>,layer:i32,roughness:f32,flags:u32,scale:f32,metal:f32};
// Same layout as headlights.rs `Uniform` (incident_lights.wgsl).
struct Headlight { view_proj:mat4x4<f32>, position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32> };
struct LocalLight { position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32>, source:vec4<f32> };
struct Headlights { lights:array<Headlight,8>, count:vec4<u32>, local:array<LocalLight,32>, local_min:vec4<f32>, local_max:vec4<f32>,
    spot_falloff:array<vec4<f32>,2> };
// x: exposure, y: emissive scale, z: contrast, w: saturation; sky: rgb irradiance, w: GI on.
// gi_origin: xyz, cell; gi_dims: xyz probes.
struct Lit {tone:vec4<f32>,sky:vec4<f32>,gi_origin:vec4<f32>,gi_dims:vec4<f32>};
@group(0) @binding(0) var<uniform> u:Uniform;
@group(0) @binding(1) var<storage,read> materials:array<Material>;
@group(0) @binding(5) var<storage,read> joints:array<vec4<f32>>;
@group(1) @binding(0) var<uniform> lit:Lit;
@group(1) @binding(1) var<uniform> headlights:Headlights;
@group(1) @binding(2) var albedo:texture_2d_array<f32>;
@group(1) @binding(3) var samp:sampler;
// The probes' six axes side by side along x: axis a at x in [a*dims.x, (a+1)*dims.x).
@group(1) @binding(4) var probes:texture_3d<f32>;
@group(1) @binding(5) var probe_samp:sampler;

const PI:f32=3.14159265;

struct Out {@builtin(position) pos:vec4<f32>,@location(0) world:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,
 @location(3) @interpolate(flat) material:u32};

@vertex fn vs(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,
 @location(3) r0:vec4<f32>,@location(4) r1:vec4<f32>,@location(5) r2:vec4<f32>,@location(6) material:u32,@location(7) flags:u32,@location(8) rigid:u32)->Out {
 var p=vec4<f32>(position,1.0);var bent=normal;var surface_material=material;var tex=uv;
 if rigid>0u {
  let row=rigid-1u+u32(uv.x+0.5)*3u;let a=joints[row];let b=joints[row+1u];let c=joints[row+2u];
  p=vec4<f32>(dot(a,p),dot(b,p),dot(c,p),1.0);bent=vec3<f32>(dot(a.xyz,normal),dot(b.xyz,normal),dot(c.xyz,normal));
  surface_material=u32(uv.y+0.5);tex=vec2<f32>(0.0);
 }
 let world=vec3<f32>(dot(r0,p),dot(r1,p),dot(r2,p));
 var out:Out;out.pos=u.vp*vec4<f32>(world,1.0);out.world=world;
 out.normal=vec3<f32>(dot(r0.xyz,bent),dot(r1.xyz,bent),dot(r2.xyz,bent));out.uv=tex;out.material=surface_material;
 return out;
}

// One axis of the baked room: probes filtered by the sampler, clamped
// inside the axis's own slab so neighbouring axes never blend.
fn probe_axis(f:vec3<f32>,axis:u32)->vec3<f32> {
 let dims=lit.gi_dims.xyz;
 let c=clamp(f+0.5,vec3<f32>(0.5),dims-0.5);
 let uvw=vec3<f32>((c.x+f32(axis)*dims.x)/(dims.x*6.0),c.y/dims.y,c.z/dims.z);
 return textureSampleLevel(probes,probe_samp,uvw,0.0).rgb;
}

fn ambient_irradiance(p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
 let sky=lit.sky.rgb;
 if lit.sky.w<0.5 {return sky;}
 let f=(p-lit.gi_origin.xyz)/lit.gi_origin.w;
 let last=lit.gi_dims.xyz-1.0;
 let outside=max(max(-f,f-last),vec3<f32>(0.0));
 let weight=clamp(1.0-max(outside.x,max(outside.y,outside.z))*2.0,0.0,1.0);
 if weight<=0.0 {return sky;}
 let n2=n*n;
 let room=probe_axis(f,select(1u,0u,n.x>=0.0))*n2.x+probe_axis(f,select(3u,2u,n.y>=0.0))*n2.y+probe_axis(f,select(5u,4u,n.z>=0.0))*n2.z;
 return mix(sky,room,weight);
}

// Shadowed spots lose their shadow here; their cone and falloff stay.
fn spot_irradiance(index:u32,p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
 let lamp=headlights.lights[index];
 let offset=p-lamp.position_range.xyz;let d2=dot(offset,offset);let dist=sqrt(d2);
 if dist<0.05 || dist>=lamp.position_range.w {return vec3<f32>(0.0);}
 let outward=offset/dist;
 let lambert=max(dot(n,-outward),0.0);
 let cone=smoothstep(lamp.direction_outer.w,lamp.intensity_inner.w,dot(outward,lamp.direction_outer.xyz));
 if cone*lambert<=0.0 {return vec3<f32>(0.0);}
 let kk=headlights.spot_falloff[index/4u][index%4u];
 let k=abs(kk);
 let nr=d2/(lamp.position_range.w*lamp.position_range.w);
 let window=max(0.0,1.0-nr*nr);
 let fade=select(1.0-smoothstep(lamp.position_range.w*0.8,lamp.position_range.w,dist),window*window,kk<0.0);
 let spread=select(pow(max(d2,0.04),k*0.5),max(d2,0.04),k==2.0);
 return lamp.intensity_inner.rgb*(cone*fade*lambert/spread);
}

fn local_irradiance(index:u32,p:vec3<f32>,n:vec3<f32>)->vec3<f32> {
 let lamp=headlights.local[index];
 let offset=p-lamp.position_range.xyz;
 let d2=dot(offset,offset);let range=lamp.position_range.w;
 if d2>=range*range || d2<1e-10 {return vec3<f32>(0.0);}
 let outward=offset*inverseSqrt(d2);
 let lambert=max(dot(n,-outward),0.0);
 let cone=smoothstep(lamp.direction_outer.w,lamp.intensity_inner.w,dot(outward,lamp.direction_outer.xyz));
 if cone*lambert<=0.0 {return vec3<f32>(0.0);}
 let normalized=d2/(range*range);let fade=max(0.0,1.0-normalized*normalized);
 let spread=select(pow(max(d2,lamp.source.x),lamp.source.y*0.5),max(d2,lamp.source.x),lamp.source.y==2.0||lamp.source.y==0.0);
 return lamp.intensity_inner.rgb*(cone*fade*fade*lambert/spread);
}

fn agx_contrast(x:vec3<f32>)->vec3<f32> {
 let x2=x*x;let x4=x2*x2;
 return 15.5*x4*x2-40.14*x4*x+31.96*x4-6.868*x2*x+0.4298*x2+0.1191*x-0.00232;
}
fn agx(c:vec3<f32>)->vec3<f32> {
 let m=mat3x3<f32>(vec3<f32>(0.842479062253094,0.0423282422610123,0.0423756549057051),
  vec3<f32>(0.0784335999999992,0.878468636469772,0.0784336),vec3<f32>(0.0792237451477643,0.0791661274605434,0.879142973793104));
 let min_ev=-12.47393;let max_ev=4.026069;
 var v=clamp(log2(max(m*c,vec3<f32>(1e-10))),vec3<f32>(min_ev),vec3<f32>(max_ev));
 return agx_contrast((v-min_ev)/(max_ev-min_ev));
}
fn agx_eotf(v:vec3<f32>)->vec3<f32> {
 let m=mat3x3<f32>(vec3<f32>(1.19687900512017,-0.0528968517574562,-0.0529716355144438),
  vec3<f32>(-0.0980208811401368,1.15190312990417,-0.0980434501171241),vec3<f32>(-0.0990297440797205,-0.0989611768448433,1.15107367264116));
 return pow(max(m*v,vec3<f32>(0.0)),vec3<f32>(2.2));
}
fn encode_srgb(c:vec3<f32>)->vec3<f32> {return select(c*12.92,1.055*pow(c,vec3<f32>(1.0/2.4))-0.055,c>vec3<f32>(0.0031308));}

// Radiance to the display (linear, 0..1): what present does for the main path.
fn display(rad:vec3<f32>)->vec3<f32> {
 var v=agx(max(rad,vec3<f32>(0.0))*lit.tone.x);
 let luma=dot(v,vec3<f32>(0.2126,0.7152,0.0722));
 v=mix(vec3<f32>(luma),v,lit.tone.w);
 v=clamp((v-0.5)*lit.tone.z+0.5,vec3<f32>(0.0),vec3<f32>(1.0));
 return clamp(agx_eotf(v),vec3<f32>(0.0),vec3<f32>(1.0));
}

fn shade(in:Out)->vec4<f32> {
 let m=materials[in.material];
 var base=m.tint;var alpha=1.0;
 if m.layer>=0 {let t=textureSample(albedo,samp,in.uv,m.layer);base*=t.rgb;alpha=t.a;}
 if (m.flags&1u)!=0u && alpha<0.5 {discard;}
 if (m.flags&4096u)!=0u {return vec4<f32>(display(base*m.scale*lit.tone.y),alpha);}
 let p=in.world;
 var n=normalize(in.normal);
 if (m.flags&8u)!=0u && dot(n,u.camera.xyz-p)<0.0 {n=-n;}
 var irradiance=ambient_irradiance(p,n);
 let spots=min(headlights.count.x,8u);
 for(var i=0u;i<spots;i++) {irradiance+=spot_irradiance(i,p,n);}
 let locals=select(min(headlights.count.y,32u),0u,any(p<headlights.local_min.xyz)||any(p>headlights.local_max.xyz));
 for(var i=0u;i<locals;i++) {irradiance+=local_irradiance(i,p,n);}
 var rad=base*irradiance/PI;
 if (m.flags&1024u)!=0u {rad+=base*abs(m.scale)*lit.tone.y;}
 return vec4<f32>(display(rad),alpha);
}

@fragment fn fs(in:Out)->@location(0) vec4<f32> {
 let c=shade(in);return vec4<f32>(encode_srgb(c.rgb),c.a);
}
@fragment fn fs_srgb(in:Out)->@location(0) vec4<f32> {return shade(in);}
