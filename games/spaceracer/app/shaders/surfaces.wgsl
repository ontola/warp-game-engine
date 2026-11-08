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
// Retained source city_surface: the tint encodes style/UV offsets, not paint.
fn sr_facade(p:vec3<f32>,n:vec3<f32>,tint:vec3<f32>,fp:f32)->SurfaceOut {
    let face=select(p.x,p.z,abs(n.x)>0.5);
    let uv=vec2<f32>(face/(56.0+tint.b*20.0),-p.y/(90.0+tint.g*28.0))+floor(tint.rg*12.0)/12.0;
    let mip=max(0.0,log2(max(fp,0.001)*f32(textureDimensions(albedo).x)/60.0));
    let tex=textureSampleLevel(albedo,samp,fract(uv),0,mip).rgb;
    let wall=1.0-smoothstep(0.4,0.8,abs(n.y));
    let brightness=dot(tex,vec3<f32>(0.2126,0.7152,0.0722));
    let style=floor(tint.r*4.0);var structure=1.0;
    if style<1.0 {structure=mix(0.14,1.0,smoothstep(0.08,0.13,fract(face/22.0)));}
    else if style<2.0 {structure=mix(0.10,1.0,smoothstep(0.12,0.18,fract(p.y/48.0)));}
    else if style<3.0 {structure=mix(0.22,1.0,smoothstep(0.12,0.2,fract(face/13.0)));}
    structure=mix(structure,0.85,smoothstep(0.5,2.0,fp/7.0));
    let room=smoothstep(0.11,0.48,brightness)*wall*structure;
    var occupancy=0.16+0.84*step(0.24,fract(sin(floor(p.y/8.0)*17.17+floor(face/7.0)*3.73+tint.r*33.0)*433.1));
    occupancy=mix(occupancy,0.8,smoothstep(0.35,1.5,fp/7.0));
    let tone=mix(vec3<f32>(1.0,0.64,0.30),vec3<f32>(0.76,0.88,1.0),tint.g*0.65);
    let paint=mix(vec3<f32>(0.075,0.09,0.115),tex*0.52*mix(vec3<f32>(0.65,0.78,0.88),vec3<f32>(1.0,0.87,0.74),tint.b)*structure,wall);
    return SurfaceOut(paint,n,0.5,0.3,tex*tone*room*occupancy*58.0);
}
fn sr_aux3(i:u32)->vec3<f32>{return vec3<f32>(bitcast<f32>(aux_word(i)),bitcast<f32>(aux_word(i+1u)),bitcast<f32>(aux_word(i+2u)));}
fn sr_ray_bounds(p:vec3<f32>,inv:vec3<f32>,lo:vec3<f32>,hi:vec3<f32>)->f32 {
    let a=(lo-p)*inv;let b=(hi-p)*inv;let near=min(a,b);let far=max(a,b);
    let enter=max(max(near.x,near.y),near.z);let leave=min(min(far.x,far.y),far.z);
    return select(1e20,max(enter,0.0),leave>=max(enter,0.0));
}
fn sr_reflections(p:vec3<f32>,n:vec3<f32>,dir:vec3<f32>,fp:f32)->vec3<f32>{
    if aux_len()<8u || aux_word(0)!=0x53524143u || aux_word(7)==0u {return vec3<f32>(0);}
    let ray=reflect(dir,n);let inv=1.0/select(vec3<f32>(0.00001),ray,abs(ray)>vec3<f32>(0.00001));
    var nearest=1600.0;var hit=0xffffffffu;
    var node=0u;
    // Threaded surface-area BVH. Each node stores its subtree's escape index:
    // no per-pixel stack, no repeated child tests, no arbitrary hit truncation.
    for(var step=0u;step<aux_word(7) && node<aux_word(7);step++) {
        let current=node;let b=aux_word(6)+current*8u;node=aux_word(b+3u);
        let enter=sr_ray_bounds(p,inv,sr_aux3(b),sr_aux3(b+4u));
        if enter>=nearest {continue;}
        let payload=aux_word(b+7u);
        if payload==0xffffffffu {
            node=current+1u;
        } else if (payload&0x80000000u)!=0u {
            let sign=aux_word(3)+(payload&0x7fffffffu)*16u;
            let normal=sr_aux3(sign+12u);let facing=dot(ray,normal);
            if facing>=-0.001 {continue;}
            let distance=dot(sr_aux3(sign)-p,normal)/facing;
            if distance<=0.0 || distance>=nearest {continue;}
            let at=p+ray*distance-sr_aux3(sign);
            let uv=vec2<f32>(dot(at,sr_aux3(sign+4u))/bitcast<f32>(aux_word(sign+3u))+0.5,0.5-dot(at,sr_aux3(sign+8u))/bitcast<f32>(aux_word(sign+7u)));
            if min(uv.x,uv.y)>=0.0 && max(uv.x,uv.y)<=1.0 {nearest=distance;hit=payload;}
        } else if enter>0.0 {nearest=enter;hit=payload;}
    }
    if hit==0xffffffffu {return vec3<f32>(0);}
    let fresnel=pow(1.0-max(dot(n,-dir),0.0),3.0);
    if (hit&0x80000000u)==0u {
        let b=aux_word(4)+hit*12u;let lo=sr_aux3(b);let hi=sr_aux3(b+4u);
        let at=p+ray*nearest;let rel=(at-(lo+hi)*0.5)/(hi-lo);
        var normal=select(vec3<f32>(0,0,sign(rel.z)),vec3<f32>(sign(rel.x),0,0),abs(rel.x)>abs(rel.z));
        if abs(rel.y)>max(abs(rel.x),abs(rel.z)){normal=vec3<f32>(0,sign(rel.y),0);}
        let facade=sr_facade(at,normal,sr_aux3(b+8u),fp+nearest*0.0005);
        return (facade.emission+facade.albedo*2.0)*mix(0.08,0.65,fresnel)*exp(-nearest*0.0015);
    }
    let b=aux_word(3)+(hit&0x7fffffffu)*16u;let at=p+ray*nearest-sr_aux3(b);
    let width=bitcast<f32>(aux_word(b+3u));let height=bitcast<f32>(aux_word(b+7u));
    var uv=vec2<f32>(dot(at,sr_aux3(b+4u))/width+0.5,0.5-dot(at,sr_aux3(b+8u))/height);
    uv.x=clamp(uv.x+(sr_noise(p.xz*1.5)-0.5)*0.012,0.003,0.997);
    let tile=u32(bitcast<f32>(aux_word(b+11u)));var layer=1;var atlas=(uv+vec2<f32>(f32(tile%3u),f32(tile/3u)))/vec2<f32>(3,2);
    if tile>=6u {layer=2;atlas=(uv+vec2<f32>(f32(tile-6u),0))/vec2<f32>(4,1);}
    let mip=max(0.0,log2(max(fp+nearest*0.0003,0.001)*f32(textureDimensions(albedo).x)/width/3.0));
    return textureSampleLevel(albedo,samp,atlas,layer,mip).rgb*145.0*mix(0.18,0.8,fresnel)*exp(-nearest*0.0015);
}
fn sr_hash1(x:f32)->f32{return fract(sin(x*127.1+0.31)*43758.5453);}
fn sr_noise1(x:f32)->f32{let f=fract(x);return mix(sr_hash1(floor(x)),sr_hash1(floor(x)+1.0),f*f*(3.0-2.0*f));}
fn sr_sandstone(p:vec3<f32>,n:vec3<f32>,detail:f32)->vec4<f32>{
    let steep=1.0-abs(n.y);let h=p.y+sr_noise(p.xz*0.0035)*26.0+sr_noise(p.xz*0.017)*6.0+dot(p.xz,vec2<f32>(0.011,0.006));
    let unit_coordinate=h/68.0+sr_noise1(h/91.0)*0.8;let unit=floor(unit_coordinate);let kind=sr_hash1(unit*3.7+1.3);
    var base=vec3<f32>(0.78,0.57,0.38);var bed_size=5.0;var crossbedded=0.0;var varnish_strength=0.35;
    if kind<0.38 {base=vec3<f32>(0.60,0.28,0.15);bed_size=2.6;varnish_strength=0.7;}
    else if kind<0.72 {base=vec3<f32>(0.74,0.43,0.24);bed_size=9.0;crossbedded=1.0;varnish_strength=0.5;}
    else if kind<0.9 {base=vec3<f32>(0.52,0.27,0.16);bed_size=1.6;varnish_strength=0.2;}
    base*=0.9+sr_hash1(unit*9.1)*0.2;
    let edge=fract(unit_coordinate);base*=mix(0.86,1.0,smoothstep(0.0,0.05,edge)*(1.0-smoothstep(0.97,1.0,edge)));
    let bed_coordinate=h/bed_size+sr_noise1(h/(bed_size*3.3)+unit*7.0)*1.4;let bed=floor(bed_coordinate);let within=fract(bed_coordinate);
    let random=sr_hash1(bed*1.73+unit*13.1);var color=base*mix(1.0,0.86+random*0.28,detail*0.85+0.15);
    let hard=step(0.58,random)*(1.0-crossbedded*0.6);let lip=1.0-smoothstep(0.0,0.14,within);
    color*=1.0-hard*lip*0.32*detail*steep;color=mix(color,color*1.12+vec3<f32>(0.03,0.02,0.01),hard*smoothstep(0.75,1.0,within)*0.5*detail);
    if crossbedded>0.5{let angle=sr_hash1(bed*5.3+unit)*6.2832;let dip=vec2<f32>(cos(angle),sin(angle))*(0.25+sr_hash1(bed*2.1)*0.35);let lamina=fract(h*0.9+dot(p.xz,dip)*0.9+sr_noise(p.xz*0.08)*0.8);color*=1.0-smoothstep(0.8,1.0,lamina)*0.12*detail*steep;}
    let space=vec2<f32>((p.x*0.71-p.z*0.71)*0.21+(p.x+p.z)*0.05,h*0.012);let streak=sr_noise(space)*0.65+sr_noise(space*vec2<f32>(2.7,1.3)+4.0)*0.35;
    let varnish=smoothstep(0.5,0.78,streak)*varnish_strength*(smoothstep(0.0,0.55,1.0-within)*0.4+0.6)*steep;
    color=mix(color,vec3<f32>(0.22,0.11,0.07),varnish*mix(0.55,1.0,detail));
    let face=vec2<f32>(p.x+p.z*0.6,p.y);let mottle=sr_noise(face*0.11)*0.6+sr_noise(face*0.43)*0.4;color*=0.86+mottle*0.26;
    color*=1.0-smoothstep(0.78,0.92,sr_noise(face*1.3+bed))*detail*steep*0.25;
    return vec4<f32>(color,(within-0.5)*mix(0.18,0.55,hard)*detail*steep);
}
fn sr_hash3(p:vec3<f32>)->vec3<f32>{return fract(sin(vec3<f32>(dot(p,vec3<f32>(127.1,311.7,74.7)),dot(p,vec3<f32>(269.5,183.3,246.1)),dot(p,vec3<f32>(113.5,271.9,124.6))))*43758.5453);}
fn sr_voronoi(p:vec3<f32>)->vec3<f32>{
    let cell=floor(p);let f=fract(p);var first=8.0;var second=8.0;var id=0.0;
    for(var x=-1;x<=1;x++){for(var y=-1;y<=1;y++){for(var z=-1;z<=1;z++){
        let o=vec3<f32>(f32(x),f32(y),f32(z));let r=o+sr_hash3(cell+o)-f;let d=dot(r,r);
        if d<first{second=first;first=d;id=sr_hash3(cell+o+17.0).x;}else if d<second{second=d;}
    }}}
    return vec3<f32>(sqrt(first),sqrt(second)-sqrt(first),id);
}
fn game_surface(s:SurfaceIn)->SurfaceOut {
    let uv=s.uv0*s.bary.x+s.uv1*s.bary.y+s.uv2*s.bary.z;
    let fp=max(length(s.dx),length(s.dy));
    var o=SurfaceOut(s.tint,s.n,s.rough,s.metal,vec3<f32>(0));
    if s.kind==0u { // Road: metres along ribbon, normalized lateral position.
        let grain=sr_noise(s.p.xz*14.0);let variation=sr_noise(s.p.xz*0.11);
        let micro=mix(0.5,grain,1.0-smoothstep(0.03,0.3,fp));
        let cross_seam=sr_line(fract(uv.y/18.0)-0.5,0.0007,max(0.0004,fp/12.0));
        let long_seam=sr_line(fract(uv.x*4.0)-0.5,0.003,0.002+fp*0.012);
        let lane=sr_line(abs(uv.x)-0.33,0.002,0.001+fp*0.01)*step(0.6,fract(uv.y/18.0));
        let edge=sr_line(abs(uv.x)-0.94,0.008,0.002+fp*0.012);
        o.albedo=mix(vec3<f32>(0.018,0.025,0.032)*(0.7+micro*0.35+variation*0.35),vec3<f32>(0.32,0.39,0.42),max(lane,edge)*0.5);
        o.albedo*=1.0-max(cross_seam,long_seam)*0.55;
        o.rough=mix(0.22,0.43,variation);o.metal=0.08;
        let ripple=vec3<f32>(sin(s.p.x*1.3+s.p.z*8.0),0.0,cos(s.p.z*1.6+s.p.x*3.0))*0.003*(1.0-smoothstep(0.2,2.0,fp));
        o.normal=normalize(s.n+ripple-s.n*dot(ripple,s.n));
        o.emission+=sr_reflections(s.p,o.normal,s.dir,fp)*mix(0.38,1.0,micro);
        // Fine aggregate breaks up the clearcoat; larger wet patches remain coherent.
        let wet=smoothstep(0.38,0.72,variation);
        o.rough=mix(0.32,0.24,wet);
        let shoulder=smoothstep(0.77,0.98,abs(uv.x));
        let drain=sr_line(fract(uv.y*1.8)-0.5,0.12,clamp(fp*0.7,0.01,0.3));
        o.albedo*=1.0-shoulder*drain*0.35;
        // A broad reflection under the luminous edge, never a solid coloured lane.
        o.emission+=vec3<f32>(0.04,0.45,0.65)*pow(smoothstep(0.78,1.0,abs(uv.x)),5.0)*3.0;

    } else if s.kind==1u {
        o=sr_facade(s.p,s.n,s.tint,fp);
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
        o.albedo=mix(o.albedo,vec3<f32>(0.12),scratches*0.15);
        if abs(s.scale)>=2.0 {
            let code=u32(round(abs(s.scale)))-2u;let side=sign(s.scale);
            let face_uv=vec2<f32>(.5+(p.z-.65)*side/.9,.5+(.24-p.y)/.9);
            if all(face_uv>=vec2<f32>(0)) && all(face_uv<=vec2<f32>(1)) && p.x*side>.05 {
                let tile=vec2<f32>(f32(code%8u%4u),f32(code%8u/4u));
                let f=textureSampleLevel(albedo,samp,(tile+clamp(face_uv,vec2<f32>(.003),vec2<f32>(.997)))/4.,i32(code/8u),clamp(log2(max(fp,.001)*256./.9),0.,6.));
                let obj=instances[s.instance];let axis=normalize(vec3<f32>(obj.r0.x,obj.r1.x,obj.r2.x));
                let ink=f.a*smoothstep(.28,.48,abs(dot(s.n,axis)));
                o.albedo=mix(o.albedo,f.rgb/max(f.a,.001),ink);
            }
        }
        let finish=sr_noise(p.xz*8.0);
        o.rough=mix(0.19,0.27,finish)+seam*0.15;o.metal=0.52;
        let vehicle=sr_vehicle(s.instance);
        if vehicle!=0u {
            let amount=bitcast<f32>(aux_word(vehicle+2u));
            let obj=instances[s.instance];
            let local_n=normalize(vec3<f32>(dot(s.n,vec3<f32>(obj.r0.x,obj.r1.x,obj.r2.x)),dot(s.n,vec3<f32>(obj.r0.y,obj.r1.y,obj.r2.y)),dot(s.n,vec3<f32>(obj.r0.z,obj.r1.z,obj.r2.z))));
            var weights=pow(abs(local_n),vec3<f32>(6.));weights/=max(.001,weights.x+weights.y+weights.z);
            let at=p*.38;let layer=i32(aux_word(aux_word(5)));
            let lod=max(0.,log2(max(fp,.001)*.38*f32(textureDimensions(albedo).x)));
            var mask=textureSampleLevel(albedo,samp,at.yz,layer,lod).rgb*weights.x
                +textureSampleLevel(albedo,samp,at.xz,layer,lod).rgb*weights.y
                +textureSampleLevel(albedo,samp,at.xy,layer,lod).rgb*weights.z;
            // Texture array is sRGB; the authored mask channels are linear data.
            mask=select(mask*12.92,1.055*pow(mask,vec3<f32>(1./2.4))-.055,mask>vec3<f32>(.0031308));
            let edge=max(.015,fp*.05);let threshold=.11+amount*.31;
            let chip=1.-smoothstep(threshold-edge,threshold+edge,mask.r);
            let rim=(1.-smoothstep(threshold+.015-edge,threshold+.045+edge,mask.r))-chip;
            let scratch=mask.g*(.13+amount*.87);let metal=max(chip,scratch*.85);
            let dirt=smoothstep(.40,.78,mask.b)*(.09+amount*.24);
            let rear=1.-smoothstep(-2.3,.8,p.z);
            let scorch=smoothstep(.55,1.,amount)*smoothstep(.35,.72,mask.b)*mix(.3,.95,rear);
            o.albedo*=1.-dirt;
            o.albedo=mix(o.albedo,vec3<f32>(.11,.12,.125),rim*.7);
            o.albedo=mix(o.albedo,vec3<f32>(.20,.22,.23),metal);
            o.albedo=mix(o.albedo,vec3<f32>(.028,.024,.021),scorch);
            o.metal=mix(o.metal,.82,metal)*(1.-scorch*.85);
            o.rough=clamp(o.rough+amount*.12+dirt*.8+scratch*.20+scorch*.42+chip*.22,.28,.91);
        }
        // The same nearby facade/screen rays used on the road: real moving
        // reflections as the hull turns, using the shared near-first hierarchy.
        let fresnel=0.04+0.96*pow(1.0-max(dot(s.n,-s.dir),0.0),5.0);
        o.emission+=sr_reflections(s.p,s.n,s.dir,fp)*mix(0.11,0.38,fresnel);

    } else if s.kind==17u { // Tinted canopy glass with a crisp dielectric coat.
        let rim=pow(1.0-max(dot(s.n,-s.dir),0.0),5.0);
        o.albedo=mix(vec3<f32>(0.006,0.018,0.028),vec3<f32>(0.018,0.065,0.09),rim);
        o.rough=0.11;o.metal=0.55;
        o.emission=sr_reflections(s.p,s.n,s.dir,fp)*mix(0.13,0.65,rim);
    } else if s.kind==18u { // Moss-covered boulders in the forest understory.
        let moss_noise=sr_noise(s.p.xz*0.12+s.p.y*0.08);
        let moss=smoothstep(0.25,0.8,s.n.y)*smoothstep(0.3,0.65,moss_noise);
        o.albedo=mix(vec3<f32>(0.13,0.15,0.14),vec3<f32>(0.07,0.16,0.025),moss)*(0.8+moss_noise*0.4);
        o.rough=0.94;o.metal=0.0;
    } else if s.kind==23u { // Retained paint under irregular soot and exposed metal.
        let soot=sr_noise(s.local.xz*1.9+s.local.y*.7)*.65+sr_noise(s.local.xy*7.1)*.35;
        let burn=smoothstep(.28,.68,soot);
        let scraped=pow(max(0.,sin(s.local.y*65.+s.local.x*5.)),24.)*smoothstep(.65,.82,soot);
        o.albedo=mix(s.tint*.7,vec3<f32>(.012,.009,.007),burn*.92);
        o.albedo=mix(o.albedo,vec3<f32>(.12,.13,.14),scraped*.6);
        o.rough=mix(.4,.94,burn);o.metal=mix(.45,.03,burn);
    } else if s.kind==19u || s.kind==20u { // Living chromatophores, no plastic flat orange.
        let mottles=sr_noise(s.local.xy*14.0+s.local.z*9.0);
        let wave=.5+.5*sin(s.local.y*6.0-s.time*1.3+s.local.x*3.0);
        o.albedo=mix(vec3<f32>(.36,.055,.025),vec3<f32>(.82,.28,.10),wave*.65+mottles*.35);
        o.albedo=mix(o.albedo,vec3<f32>(.95,.62,.40),smoothstep(.74,.95,mottles)*.45);
        if s.kind==20u {
            let row=min(abs(uv.x-.08),abs(uv.x-.92));
            let cell=vec2<f32>((fract(uv.y*38.0)-.5)*1.4,row*9.0);
            let d=length(cell);let cup=1.0-smoothstep(.23,.34,d);
            let rim=smoothstep(.12,.22,d)*cup;
            o.albedo=mix(o.albedo,vec3<f32>(.94,.65,.46)*(1.0-rim*.4),cup);
        }
        o.rough=.43;o.metal=0.0;
        o.emission=o.albedo*80.0;
    } else if s.kind==21u {
        let slit=1.0-smoothstep(.035,.07,abs(uv.y-.5));
        o.albedo=mix(vec3<f32>(.85,.48,.07),vec3<f32>(.007,.01,.008),slit);
        o.emission=o.albedo*50.0;o.rough=.13;o.metal=.1;
    } else if s.kind==22u {
        let rim=pow(1.0-abs(dot(s.n,-s.dir)),3.0);
        o.albedo=vec3<f32>(.015,.07,.08);o.rough=.08;o.metal=.25;
        o.emission=vec3<f32>(.25,.75,.85)*rim*220.0;
    } else if s.kind==3u { // Water: coherent swell and fine wind ripples.
        let a=s.p.x*0.055+s.p.z*0.032+s.time*0.65;let b=s.p.z*0.17-s.p.x*0.08-s.time*0.8;
        let fine=sin(s.p.x*0.8+s.p.z*1.2+s.time*1.1)*(1.0-smoothstep(0.25,1.0,fp));
        let dx=0.018*cos(a)+0.009*cos(b)+fine*0.005;let dz=0.015*sin(a)+0.011*sin(b)-fine*0.004;
        o.normal=normalize(vec3<f32>(dx,1.0,dz));
        let shore=textureSampleLevel(albedo,samp,clamp(s.p.xz/7680.0+0.5,vec2<f32>(0.001),vec2<f32>(0.999)),3,0.0).r;
        var foam=smoothstep(0.88,0.99,shore)*(1.0-smoothstep(0.99,1.0,shore));foam*=smoothstep(0.03,0.35,sin(a)+cos(b)+sin(s.time*0.65+shore*28.0)*0.4);
        o.albedo=mix(mix(vec3<f32>(0.004,0.022,0.027),vec3<f32>(0.025,0.095,0.068),shore*shore),vec3<f32>(0.36,0.45,0.39),foam*0.35);o.rough=0.16+foam*0.15;o.metal=0.0;
    } else if s.kind==4u {
        o.albedo=sr_sandstone(s.p,s.n,1.0-smoothstep(0.3,1.5,fp)).rgb;o.rough=0.86;o.metal=0.0;
    } else if s.kind==5u {
        let wave=sr_noise(s.p.xz*0.05+s.p.y*0.03);let rim=pow(1.0-abs(dot(s.n,-s.dir)),3.0);
        o.albedo=s.tint*(0.3+wave*0.5);o.metal=0.0;o.rough=0.35;
        o.emission=s.tint*(1.0+rim*12.0)*s.scale;
    } else if s.kind==6u {
        let patchiness=sr_noise(s.p.xz*0.012)*0.65+sr_noise(s.p.xz*0.043)*0.35;
        let grain=(sr_noise(s.p.xz*0.38)-0.5)*(1.0-smoothstep(0.3,1.8,fp));
        var grass=mix(vec3<f32>(0.045,0.095,0.024),vec3<f32>(0.13,0.21,0.045),patchiness);
        var stone=mix(vec3<f32>(0.095,0.105,0.087),vec3<f32>(0.19,0.18,0.13),patchiness);
        let rock=1.0-smoothstep(0.68,0.94,s.n.y);let earth=mix(vec3<f32>(0.095,0.074,0.037),vec3<f32>(0.23,0.20,0.115),patchiness);
        let shore=1.0-smoothstep(1.0,9.0,s.p.y-s.scale);
        let meadow=smoothstep(0.55,0.8,sr_noise(s.p.xz*0.004+3.1))*(1.0-rock)*(1.0-shore);
        grass=mix(grass,mix(vec3<f32>(0.14,0.24,0.05),vec3<f32>(0.25,0.33,0.07),patchiness),meadow*0.7);
        let cell=floor(s.p.xz*0.7);let bloom=sr_hash(cell);let petal=1.0-smoothstep(0.12,0.42,length(fract(s.p.xz*0.7)-0.5-(vec2<f32>(sr_hash(cell+5.3),sr_hash(cell+9.1))-0.5)*0.5));
        let flowers=step(0.86-meadow*0.18,bloom)*petal*(1.0-smoothstep(0.12,0.52,fp));
        var blossom=vec3<f32>(0.62,0.36,0.85);if bloom>0.96{blossom=vec3<f32>(0.85,0.78,0.95);}else if bloom>0.92{blossom=vec3<f32>(0.95,0.78,0.16);}
        stone=mix(stone,vec3<f32>(0.08,0.13,0.04),smoothstep(0.35,0.7,sr_noise(s.p.xz*0.09))*0.5);
        let ground=mix(mix(grass,stone,rock),earth,shore)*(0.94+grain*0.22);
        o.albedo=mix(ground,blossom,flowers*(1.0-rock)*(1.0-shore))*mix(0.63,1.0,smoothstep(-1.0,3.0,s.p.y-s.scale));o.rough=0.95;o.metal=0.0;
    }
    if s.kind==8u || s.kind==15u || s.kind==16u {
        let detail=1.0-smoothstep(0.3,1.5,fp);let patchiness=sr_noise(s.p.xz*0.006)*0.6+sr_noise(s.p.xz*0.031)*0.4;
        let phase=dot(s.p.xz,vec2<f32>(0.82,0.57))*0.55+sr_noise(s.p.xz*0.02)*7.0;
        var sand=mix(vec3<f32>(0.62,0.39,0.2),vec3<f32>(0.86,0.62,0.36),patchiness);
        sand*=0.92+sr_noise(s.p.xz*0.9)*0.16*detail;
        let rock=select(1.0-smoothstep(0.58,0.84,s.n.y),1.0,s.kind==16u);let stone=sr_sandstone(s.p,s.n,detail);
        o.albedo=mix(sand,stone.rgb,rock);o.rough=0.92;o.metal=0.0;
        let slope=cos(phase)*0.22*detail*(1.0-rock);
        o.normal=normalize(s.n+vec3<f32>(0.82,0,0.57)*slope+vec3<f32>(0,1,0)*stone.a*rock);
        if s.kind==15u || s.kind==16u {
            let face=vec2<f32>(s.p.x+s.p.z*0.7,s.p.y+s.p.z*0.3);let mottle=sr_noise(s.p.xz*0.04+s.p.y*0.03)*0.5+sr_noise(face*0.19)*0.3+sr_noise(face*0.8)*0.2*detail;
            let reef=mix(vec3<f32>(0.12,0.13,0.13),vec3<f32>(0.3,0.29,0.26),mottle);
            let hue=sr_noise(s.p.xz*0.006+s.p.y*0.004);let colony=mix(vec3<f32>(0.62,0.3,0.16),vec3<f32>(0.42,0.22,0.42),smoothstep(0.3,0.7,hue));
            let cover=smoothstep(0.56,0.72,sr_noise(face*0.12)*0.55+sr_noise(face*0.37+3.0)*0.3+sr_noise(face*1.4)*0.15*detail);
            o.albedo=mix(mix(vec3<f32>(0.36,0.36,0.3),vec3<f32>(0.56,0.54,0.44),patchiness),mix(reef,colony,cover*0.7),rock);
            let caustic=pow(0.5+0.5*sin(s.p.x*0.17+sin(s.p.z*0.12+s.time)*2.0)*sin(s.p.z*0.19+s.time*0.7),10.0);
            o.emission=o.albedo*vec3<f32>(0.55,0.9,1)*caustic*250.0*max(s.n.y,0.0);
        }
    }
    if s.kind==9u {
        let p=normalize(s.local);let t=s.time;
        let flow=sin(p.x*11.0+sin(p.z*8.0)*1.8)+sin(p.y*17.0+p.z*9.0+sin(p.x*12.0));
        let cloud=0.5+0.5*sin(p.z*6.0+sin(p.x*7.0)+p.y*4.0);
        let warm=pow(max(dot(p,normalize(vec3<f32>(0.55,0.35,-0.75))),0.0),2.0);let cool=pow(max(dot(p,normalize(vec3<f32>(-0.6,-0.2,0.7))),0.0),2.0);
        var deep=mix(vec3<f32>(0.008,0.03,0.042),vec3<f32>(0.03,0.085,0.078),cloud)+vec3<f32>(0.09,0.035,0.05)*warm+vec3<f32>(0.02,0.03,0.08)*cool;
        let tiles=sr_voronoi(p*7.0+vec3<f32>(0,t*0.02,0));let wall=1.0-smoothstep(0.0,0.06,tiles.y);let vein=pow(0.5+0.5*sin(flow*5.0+tiles.x*9.0),18.0);
        let color=mix(vec3<f32>(0.07,0.2,0.17),vec3<f32>(0.24,0.11,0.15),warm);
        let ridges=pow(0.5+0.5*sin(flow*3.0+p.y*9.0+t*0.15),12.0);let pulse=0.94+0.06*sin(t*0.35+flow);
        let bokeh=sr_voronoi(p*13.0+vec3<f32>(t*0.01,0,-t*0.015));let glow=(1.0-smoothstep(0.06,0.2,bokeh.x))*step(0.7,bokeh.z);
        deep+=color*wall*0.55+color*vein*0.18+vec3<f32>(0.028,0.065,0.05)*ridges+mix(vec3<f32>(0.12,0.3,0.26),vec3<f32>(0.36,0.2,0.24),warm)*glow*0.35;
        o.albedo=vec3<f32>(0);o.emission=deep*pulse*280.0;o.rough=1.0;o.metal=0.0;
    }
    if (s.kind>=10u && s.kind<=14u) || s.kind==17u {
        let b=sr_vehicle(s.instance);
        if b!=0u {
            let progress=bitcast<f32>(aux_word(b+3u));
            if progress<1.0 {
                let q=sr_hull(s.p,b);let front=mix(-1.,2.6,min(1.,progress*1.4));
                let band=1.-smoothstep(.035,.23,abs(q.y-front));
                let grid=abs(fract(q*vec3<f32>(3.,9.,3.))-.5);
                let line=1.-smoothstep(vec3<f32>(.015),vec3<f32>(.015+max(.012,fp*4.)),grid);
                let lattice=max(line.y,max(line.x,line.z)*.55);
                let fade=1.-smoothstep(.60,1.,progress);
                o.emission+=mix(vec3<f32>(.12,.65,1.),vec3<f32>(.8,1.,1.),band)*
                    (band*2.6+.08+.18*lattice)*fade*bitcast<f32>(aux_word(b+20u));
            }
        }
    }
    return o;
}

