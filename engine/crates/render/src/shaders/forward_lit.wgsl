// The low tier (KERN_FORWARD=1, forward.rs `Lit`): one forward pass for
// tile GPUs. Per pixel: the material's colour and albedo texture, the room's
// baked irradiance (one probe volume in a 3D texture: three filtered taps),
// the local lights unshadowed, emission, then exposure and AgX straight into
// the display target. Nothing per pixel goes through main memory but the
// final colour: depth and colour stay in the GPU's tile memory.
struct Spot {position:vec4<f32>,direction:vec4<f32>,intensity:vec4<f32>};
struct Uniform {vp:mat4x4<f32>,camera:vec4<f32>,sun:vec4<f32>,sun_color:vec4<f32>,ambient:vec4<f32>,fog:vec4<f32>,spots:array<Spot,4>,inv_vp:mat4x4<f32>};
struct Material {tint:vec3<f32>,layer:i32,roughness:f32,flags:u32,scale:f32,metal:f32};
// Same layout as headlights.rs `Uniform` (incident_lights.wgsl).
struct Headlight { view_proj:mat4x4<f32>, position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32> };
struct LocalLight { position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32>, source:vec4<f32> };
struct Headlights { lights:array<Headlight,8>, count:vec4<u32>, local:array<LocalLight,32>, local_min:vec4<f32>, local_max:vec4<f32>,
    spot_falloff:array<vec4<f32>,2> };
// x: exposure, y: emissive scale, z: contrast, w: saturation; sky: rgb room fill (lux), w: GI on; sh: the sky as SH.
// gi_origin: xyz, cell; gi_dims: xyz probes.
// medium: density, anisotropy, intensity, beam count; tint: rgb, extinction;
// ambient: rgb in-scatter, medium length (0 unbounded). Beams as volumetric.rs GpuLight.
struct ScatterLight { position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32>, source:vec4<f32> };
struct Lit {tone:vec4<f32>,sky:vec4<f32>,gi_origin:vec4<f32>,gi_dims:vec4<f32>,medium:vec4<f32>,tint:vec4<f32>,ambient:vec4<f32>,sh:array<vec4<f32>,9>,beams:array<ScatterLight,16>};
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
// Per material: blended alpha, additive (Scene::set_material_blend).
@group(1) @binding(6) var<storage,read> blends:array<vec4<f32>>;

const PI:f32=3.14159265;

struct Out {@builtin(position) pos:vec4<f32>,@location(0) world:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,
 @location(3) @interpolate(flat) material:u32,@location(4) local:vec3<f32>,@location(5) @interpolate(flat) owner:u32};

@vertex fn vs(@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,
 @location(3) r0:vec4<f32>,@location(4) r1:vec4<f32>,@location(5) r2:vec4<f32>,@location(6) material:u32,@location(7) flags:u32,@location(8) rigid:u32,@location(9) owner:u32)->Out {
 var p=vec4<f32>(position,1.0);var bent=normal;var surface_material=material;var tex=uv;
 if rigid>0u {
  let row=rigid-1u+u32(uv.x+0.5)*3u;let a=joints[row];let b=joints[row+1u];let c=joints[row+2u];
  p=vec4<f32>(dot(a,p),dot(b,p),dot(c,p),1.0);bent=vec3<f32>(dot(a.xyz,normal),dot(b.xyz,normal),dot(c.xyz,normal));
  surface_material=u32(uv.y+0.5);tex=vec2<f32>(0.0);
 }
 let world=vec3<f32>(dot(r0,p),dot(r1,p),dot(r2,p));
 var out:Out;out.pos=u.vp*vec4<f32>(world,1.0);out.world=world;
 out.normal=vec3<f32>(dot(r0.xyz,bent),dot(r1.xyz,bent),dot(r2.xyz,bent));out.uv=tex;out.material=surface_material;out.local=position;out.owner=owner;
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
 // The sky's irradiance (its nine SH coefficients, as shade.wgsl) plus the
 // authored room fill (Lighting::indirect_fill_lux), without occlusion.
 let sky=sky_irradiance(n)+lit.sky.rgb;
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


fn hg_phase(cos_theta:f32,g:f32)->f32 {
 let g2=g*g;let denom=max(1.0+g2-2.0*g*cos_theta,1e-4);
 return (1.0-g2)/(4.0*PI*denom*sqrt(denom));
}

// volumetric.wgsl `clip_cone`: the ray interval inside the outer cone, so
// the three samples land where the beam is.
fn clip_cone(o:vec3<f32>,v:vec3<f32>,light:ScatterLight,t0:f32,t1:f32)->vec2<f32> {
 let axis=light.direction_outer.xyz;let c2=light.direction_outer.w*light.direction_outer.w;
 let d0=o-light.position_range.xyz;let va=dot(v,axis);let d0a=dot(d0,axis);
 let qa=va*va-c2;let qb=va*d0a-c2*dot(d0,v);let qc=d0a*d0a-c2*dot(d0,d0);
 if abs(qa)<1e-4 {return vec2<f32>(t0,t1);}
 let disc=qb*qb-qa*qc;
 if disc<0.0 {if qa<0.0 {return vec2<f32>(1.0,0.0);} return vec2<f32>(t0,t1);}
 let s=sqrt(disc);let r1=(-qb-s)/qa;let r2=(-qb+s)/qa;let lo=min(r1,r2);let hi=max(r1,r2);
 var span=vec2<f32>(-1e30,1e30);
 if qa<0.0 {if d0a+va*0.5*(lo+hi)<0.0 {return vec2<f32>(1.0,0.0);} span=vec2<f32>(lo,hi);}
 else if va>0.0 {span=vec2<f32>(hi,1e30);} else {span=vec2<f32>(-1e30,lo);}
 return vec2<f32>(max(span.x,t0),min(span.y,t1));
}

// volumetric.wgsl `light_scatter` with three Gauss-Legendre points instead
// of six: the light one beam scatters towards the
// camera along the view ray up to the surface.
fn beam_scatter(o:vec3<f32>,v:vec3<f32>,tmax:f32,light:ScatterLight,sigma:f32,g:f32)->vec3<f32> {
 let p=light.position_range.xyz;let range=light.position_range.w;
 let rel=p-o;let tc=dot(rel,v);
 let h2=max(dot(rel,rel)-tc*tc,0.0);let r2=range*range;
 if h2>=r2 {return vec3<f32>(0.0);}
 let half_chord=sqrt(r2-h2);
 var span=vec2<f32>(max(tc-half_chord,0.0),min(tc+half_chord,tmax));
 if light.source.z>0.5 {span=clip_cone(o,v,light,span.x,span.y);}
 if span.y<=span.x {return vec3<f32>(0.0);}
 let s2=light.source.x;let falloff=light.source.y;
 let hp=sqrt(h2+s2);let tangent=falloff>=1.75;
 let x=(span-tc)/hp;
 let a=select(vec2<f32>(asinh(x.x),asinh(x.y)),vec2<f32>(atan(x.x),atan(x.y)),tangent);
 let exponent=select(0.5-0.5*falloff,1.0-0.5*falloff,tangent);
 let jacobian=select(1.0,1.0/hp,tangent);
 let centre=0.5*(a.x+a.y);let half_width=0.5*(a.y-a.x);
 var acc=0.0;
 for(var k=0u;k<3u;k++) {
  let node=select(select(0.7745967,-0.7745967,k==0u),0.0,k==1u);
  let weight=select(0.5555556,0.8888889,k==1u);
  let ak=centre+half_width*node;
  let t=tc+hp*select(sinh(ak),tan(ak),tangent);
  let off=o+v*t-p;let d2=dot(off,off);let dist=sqrt(d2);
  let outward=off/max(dist,1e-6);
  let cone=smoothstep(light.direction_outer.w,light.intensity_inner.w,dot(outward,light.direction_outer.xyz));
  let n=d2/r2;let fade=max(0.0,1.0-n*n);
  var spread=1.0;if exponent!=0.0 {spread=pow(d2+s2,exponent);}
  acc+=weight*cone*fade*fade*hg_phase(dot(outward,-v),g)*exp(-sigma*(t+dist))*spread;
 }
 return light.intensity_inner.rgb*(acc*half_width*jacobian);
}

// The medium between the camera and p: the beams' in-scatter plus the
// ambient, and how much the medium dims p (extinction).
// A blended surface's light through the medium: absorbed only, since the
// image behind already holds the in-scatter (else the card shows as a box).
fn absorbed(rad:vec3<f32>,p:vec3<f32>)->vec3<f32> {
 if lit.medium.x<=0.0 {return rad;}
 var tmax=distance(p,u.camera.xyz);
 if lit.ambient.w>0.0 {tmax=min(tmax,lit.ambient.w);}
 return rad*exp(-lit.tint.w*lit.medium.x*tmax);
}
fn medium(rad:vec3<f32>,p:vec3<f32>)->vec3<f32> {
 let count=u32(lit.medium.w);
 if lit.medium.x<=0.0 {return rad;}
 let o=u.camera.xyz;var tmax=distance(p,o);let v=(p-o)/max(tmax,1e-4);
 if lit.ambient.w>0.0 {tmax=min(tmax,lit.ambient.w);}
 var sum=vec3<f32>(0.0);
 for(var i=0u;i<min(count,16u);i++) {sum+=beam_scatter(o,v,tmax,lit.beams[i],lit.medium.x,lit.medium.y);}
 let reach=1.0-exp(-lit.medium.x*tmax);
 return rad*exp(-lit.tint.w*lit.medium.x*tmax)+sum*lit.tint.rgb*(lit.medium.x*lit.medium.z)+lit.ambient.rgb*reach;
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
 // Blended (BLEND): the material's alpha, premultiplied; additive keeps
 // what is behind whole. Emissive blends glow unlit (transparency.wgsl).
 let blended=(m.flags&256u)!=0u;
 var cover=alpha;
 if blended {
  let b=blends[in.material];alpha*=b.x;
  if alpha<=0.0 {discard;}
  cover=select(alpha,0.0,b.y>0.5);
  if (m.flags&1024u)!=0u {return vec4<f32>(display(absorbed(base*abs(m.scale)*lit.tone.y,in.world))*alpha,cover);}
 }
 if (m.flags&4096u)!=0u {if blended {return vec4<f32>(display(absorbed(base*m.scale*lit.tone.y,in.world))*alpha,cover);} return vec4<f32>(display(medium(base*m.scale*lit.tone.y,in.world)),alpha);}
 let p=in.world;
 var n=normalize(in.normal);
 if (m.flags&8u)!=0u && dot(n,u.camera.xyz-p)<0.0 {n=-n;}
 var emit=vec3<f32>(0.0);
 var rough=m.roughness;var metal=m.metal;var coat=0.0;
 // A game's own surface (SURFACE): its game_surface sees the pixel as the
 // shade pass shows it; the interpolated uv stands in for the three corners.
 if (m.flags&32768u)!=0u {
  var si:SurfaceIn;
  si.kind=u32(-2-m.layer);si.instance=in.owner;si.p=p;si.local=in.local;si.n=n;si.dir=normalize(p-u.camera.xyz);
  si.bary=vec3<f32>(1.0,0.0,0.0);si.uv0=in.uv;si.uv1=in.uv;si.uv2=in.uv;si.dx=dpdx(p);si.dy=dpdy(p);
  si.tint=m.tint;si.rough=m.roughness;si.metal=m.metal;si.scale=m.scale;si.time=light.anim.x;
  let so=game_surface(si);
  base=so.albedo;n=so.normal;emit=so.emission;rough=so.rough;metal=so.metal;
 }
 // Car parts (car_material.wgsl): the part's colour, metalness and clear
 // coat, without the procedural wear.
 let car=(m.flags>>16u)&0x3fffu;
 if car>0u && (m.flags&0xc0000000u)==0u {
  let at=scene_tables[0].y+9u*(car-1u);
  let colour=bitcast<vec4<f32>>(scene_tables[at]);let surface=bitcast<vec4<f32>>(scene_tables[at+2u]);
  if u32(colour.w)!=6u {base=colour.rgb;}
  rough=surface.x;metal=surface.y;coat=surface.z;
 }
 var irradiance=ambient_irradiance(p,n);
 let spots=min(headlights.count.x,8u);
 for(var i=0u;i<spots;i++) {irradiance+=spot_irradiance(i,p,n);}
 let locals=select(min(headlights.count.y,32u),0u,any(p<headlights.local_min.xyz)||any(p>headlights.local_max.xyz));
 for(var i=0u;i<locals;i++) {irradiance+=local_irradiance(i,p,n);}
 irradiance+=light.sun_color.rgb*light.sun_dir.w*max(dot(n,light.sun_dir.xyz),0.0);
 // The sky in what is glossy: its SH at the mirror direction (a blurred
 // reflection), by Schlick's Fresnel, fading out with roughness.
 let v=normalize(u.camera.xyz-p);let r=reflect(-v,n);
 let f0=mix(vec3<f32>(0.04),base,metal);let k=pow(1.0-clamp(dot(n,v),0.0,1.0),5.0);
 let gloss=(1.0-rough)*(1.0-rough);
 let env=sky_irradiance(r)/PI;
 let specular=(f0+(1.0-f0)*k)*gloss*env+coat*(0.04+0.96*k)*env;
 // Metal keeps a quarter of its diffuse: without the local lights' highlights
 // (none here) a metal would only show the sky.
 var rad=base*(1.0-0.75*metal)*irradiance/PI+specular+emit*lit.tone.y;
 if (m.flags&1024u)!=0u {rad+=base*abs(m.scale)*lit.tone.y;}
 let glow=game_glow(p,in.owner,in.material);rad=rad*(1.0-glow.a)+glow.rgb;
 if blended {return vec4<f32>(display(absorbed(rad,p))*alpha,cover);}
 return vec4<f32>(display(medium(rad,p)),alpha);
}

// The sky where no mesh is: its SH at each view ray (a soft gradient).
struct SkyOut {@builtin(position) pos:vec4<f32>,@location(0) ndc:vec2<f32>};
@vertex fn vs_sky(@builtin(vertex_index) i:u32)->SkyOut {
 let q=vec2<f32>(f32((i<<1u)&2u),f32(i&2u))*2.0-1.0;
 var out:SkyOut;out.pos=vec4<f32>(q,0.0,1.0);out.ndc=q;return out;
}
fn sky_colour(ndc:vec2<f32>)->vec3<f32> {
 let far=u.inv_vp*vec4<f32>(ndc,1e-6,1.0);
 let d=normalize(far.xyz/far.w-u.camera.xyz);
 return display(sky_irradiance(d)/PI);
}
@fragment fn fs_sky(in:SkyOut)->@location(0) vec4<f32> {return vec4<f32>(encode_srgb(sky_colour(in.ndc)),1.0);}
@fragment fn fs_sky_srgb(in:SkyOut)->@location(0) vec4<f32> {return vec4<f32>(sky_colour(in.ndc),1.0);}

@fragment fn fs(in:Out)->@location(0) vec4<f32> {
 let c=shade(in);return vec4<f32>(encode_srgb(c.rgb),c.a);
}
@fragment fn fs_srgb(in:Out)->@location(0) vec4<f32> {return shade(in);}
