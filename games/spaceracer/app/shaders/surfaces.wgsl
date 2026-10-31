// A game's own surface shader (material flag SURFACE, 1 << 15).
//
// A game replaces this file with `render::set_game_surface(source)` before
// the renderer is created. Its `game_surface` turns the shading point into
// albedo, normal, roughness, metalness and emission, so procedural Godot
// shaders can be ported line by line. The material's `layer` holds
// -2 - kind: `kind` tells the game's surfaces apart, and the negative layer
// keeps the texture array and alpha test away from these materials.
//
// The built-in copy keeps the material as it is.

struct SurfaceIn {
    // Which of the game's surfaces, from Material::layer.
    kind: u32,
    // The instance (index into the scene's instances).
    instance: u32,
    // World position, and object-space (rest pose) position.
    p: vec3<f32>,
    local: vec3<f32>,
    // Shading normal (world), and the direction from the eye to the point.
    n: vec3<f32>,
    dir: vec3<f32>,
    // Barycentrics and each corner's raw uv, so a game can pack per-vertex
    // data (such as vertex colours) into uvs and blend it itself.
    bary: vec3<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    // World-space footprint of one pixel (ray differentials).
    dx: vec3<f32>,
    dy: vec3<f32>,
    // Material tint, roughness, metalness and scale.
    tint: vec3<f32>,
    rough: f32,
    metal: f32,
    scale: f32,
    // Seconds (Lighting::time).
    time: f32,
};

struct SurfaceOut {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    rough: f32,
    metal: f32,
    // Radiance in nits for white, scaled like EMISSIVE.
    emission: vec3<f32>,
};


fn sr_hash(p:vec2<f32>)->f32 { return fract(sin(dot(p,vec2<f32>(127.1,311.7)))*43758.5453); }
fn sr_noise(p:vec2<f32>)->f32 {
    let i=floor(p);let f=fract(p);let u=f*f*(3.0-2.0*f);
    return mix(mix(sr_hash(i),sr_hash(i+vec2<f32>(1,0)),u.x),mix(sr_hash(i+vec2<f32>(0,1)),sr_hash(i+vec2<f32>(1,1)),u.x),u.y);
}
fn sr_line(x:f32,width:f32,aa:f32)->f32 {return 1.0-smoothstep(width,width+aa,abs(x));}
fn game_surface(s:SurfaceIn)->SurfaceOut {
    let uv=s.uv0*s.bary.x+s.uv1*s.bary.y+s.uv2*s.bary.z;
    let fp=max(length(s.dx),length(s.dy));
    var o=SurfaceOut(s.tint,s.n,s.rough,s.metal,vec3<f32>(0));
    if s.kind==0u { // Road: metres along ribbon, normalized lateral position.
        let grain=sr_noise(s.p.xz*14.0);let variation=sr_noise(s.p.xz*0.11);
        let micro=mix(0.5,grain,1.0-smoothstep(0.03,0.3,fp));
        let cross_seam=sr_line(fract(uv.y/12.0)-0.5,0.0007,max(0.0004,fp/12.0));
        let long_seam=sr_line(fract(uv.x*4.0)-0.5,0.003,0.002+fp*0.012);
        let lane=sr_line(abs(uv.x)-0.5,0.002,0.001+fp*0.01)*step(0.6,fract(uv.y/18.0));
        let edge=sr_line(abs(uv.x)-0.94,0.008,0.002+fp*0.012);
        o.albedo=mix(vec3<f32>(0.035,0.042,0.05)*(0.7+micro*0.35+variation*0.35),vec3<f32>(0.32,0.39,0.42),max(lane,edge)*0.5);
        o.albedo*=1.0-max(cross_seam,long_seam)*0.55;
        o.rough=mix(0.22,0.43,variation);o.metal=0.08;
        let ripple=vec3<f32>(sin(s.p.x*1.3+s.p.z*8.0),0.0,cos(s.p.z*1.6+s.p.x*3.0))*0.003*(1.0-smoothstep(0.2,2.0,fp));
        o.normal=normalize(s.n+ripple-s.n*dot(ripple,s.n));
    } else if s.kind==1u { // City glazing: fixed metre-sized floors/windows.
        let wall=select(s.p.xy,s.p.zy,abs(s.n.x)>abs(s.n.z));
        let q=wall/vec2<f32>(3.2,4.0);let cell=floor(q);let f=fract(q);
        let aa=clamp(fp*0.3,0.015,0.3);
        let pane=smoothstep(0.1,0.1+aa,f.x)*(1.0-smoothstep(0.83-aa,0.83,f.x))*smoothstep(0.16,0.16+aa,f.y)*(1.0-smoothstep(0.86-aa,0.86,f.y));
        let r=sr_hash(cell+vec2<f32>(s.scale*133.0,7.0));
        let occupancy=step(0.64,sr_hash(floor(cell/vec2<f32>(3.0,2.0))+s.scale));
        let lit=occupancy*step(0.24,r)*pane;
        let blind=0.65+0.35*step(0.24,fract(f.y*6.0));
        let warmth=mix(vec3<f32>(0.54,0.74,1.0),vec3<f32>(1.0,0.72,0.39),step(0.26,r));
        let stripe=sr_line(fract(q.y/12.0)-0.02,0.012,aa/12.0);
        let mullion=1.0-pane;
        o.albedo=mix(vec3<f32>(0.018,0.031,0.047),s.tint,mullion*0.7);
        o.rough=mix(0.24,0.58,mullion);o.metal=0.38;
        o.emission=warmth*lit*blind*(10.0+r*30.0)+vec3<f32>(0.13,0.35,0.5)*stripe*1.5;
        if abs(s.n.y)>0.7 {o.emission=vec3<f32>(0);o.albedo=s.tint*0.4;o.rough=0.7;}
    } else if s.kind>=10u && s.kind<=14u { // Authored hull livery, in the opaque paint pass.
        let p=s.local;let aa=clamp(fp*0.2,0.002,0.04);
        let seam=sr_line(fract((p.z+0.6)*0.4)-0.5,0.002,aa*0.3);
        var marking=0.0;
        if s.kind==11u {let t=clamp((p.z-2.15)/2.7,0.0,1.0);marking=sr_line(abs(p.x)-mix(0.54,0.16,t),mix(0.085,0.025,t),aa)*step(2.15,p.z)*(1.0-step(4.85,p.z))*step(0.0,p.y);}
        if s.kind==12u {let shoulder=select(0.2,0.5,s.scale>0.0);marking=sr_line(uv.x-shoulder,0.014,aa*0.1)*step(-1.55,p.z)*(1.0-step(2.2,p.z));}
        if s.kind==13u {let t=clamp((-p.z-0.45)/1.3,0.0,1.0);marking=sr_line(abs(p.x)-mix(3.22,4.09,t),mix(0.23,0.36,t),aa)*step(-1.75,p.z)*(1.0-step(-0.45,p.z));}
        if s.kind==14u {marking=sr_line(p.z-mix(-0.38,-0.22,clamp(p.y/1.32,0.0,1.0)),0.09,aa)*step(0.07,p.y)*(1.0-step(1.26,p.y));}
        let pigment=mix(s.tint,vec3<f32>(0.72,0.68,0.60),marking);
        let scratches=step(0.95,sr_noise(p.xz*22.0))*sr_line(fract(p.z*8.0)-0.5,0.025,aa)*(1.0-smoothstep(0.03,0.15,fp));
        o.albedo=pigment*mix(0.52,1.0,smoothstep(-0.3,0.3,p.y))*(1.0-seam*0.5);
        o.albedo=mix(o.albedo,vec3<f32>(0.12),scratches*0.15);o.rough=mix(0.31,0.43,seam);o.metal=0.32;
    } else if s.kind==3u { // Water: coherent swell and fine wind ripples.
        let a=s.p.x*0.055+s.p.z*0.032+s.time*0.65;let b=s.p.z*0.17-s.p.x*0.08-s.time*0.8;
        let dx=0.035*cos(a)+0.012*cos(b);let dz=0.025*sin(a)+0.018*sin(b);
        o.normal=normalize(vec3<f32>(dx,1.0,dz));o.albedo=s.tint;o.rough=0.16;o.metal=0.18;
    } else if s.kind==4u { // Stone: sediment rather than smooth plastic boulders.
        let strata=0.5+0.5*sin(s.p.y*0.32+sr_noise(s.p.xz*0.02)*4.0);
        let grain=sr_noise(s.p.xz*0.7+s.p.y*0.1);
        o.albedo=s.tint*(0.75+strata*0.07+grain*0.18);o.rough=0.86;o.metal=0.0;
    } else if s.kind==5u {
        let wave=sr_noise(s.p.xz*0.05+s.p.y*0.03);let rim=pow(1.0-abs(dot(s.n,-s.dir)),3.0);
        o.albedo=s.tint*(0.3+wave*0.5);o.metal=0.0;o.rough=0.35;
        o.emission=s.tint*(1.0+rim*12.0)*s.scale;
    } else if s.kind==6u {
        let variation=sr_noise(s.p.xz*0.027);let fine=sr_noise(s.p.xz*1.6);
        o.albedo=s.tint*(0.55+variation*0.7+fine*0.15);o.rough=0.93;o.metal=0.0;
    }
    return o;
}

