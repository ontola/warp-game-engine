// Match fragments sample the stadium atmosphere at their own depth.
const PI: f32 = 3.14159265;
@group(2) @binding(0) var<uniform> view: View;
@group(2) @binding(2) var<uniform> light: Light;
@group(2) @binding(5) var shadow_tex: texture_depth_2d;
@group(2) @binding(6) var shadow_samp: sampler_comparison;
@group(2) @binding(7) var sky_tex: texture_2d<f32>;

struct Camera {
    vp: mat4x4<f32>, right: vec4<f32>, up: vec4<f32>,
    game: vec4<f32>, // enabled, first scene instance, sky lux, sun lux
    sun: vec4<f32>, sun_color: vec4<f32>,
    settings: vec4<f32>, // time, focal length in pixels, far mode, actor count
    shadow_vp:mat4x4<f32>,
}
// Four packed pairs of joint angles follow the light sample, shared by all
// vertices of this spectator. Angles stay continuous when LOD changes.
struct SeatLight { index:u32, rg:u32, blue:u32, direction_sun:u32, joints:vec4<u32> }

fn spectator_joints(a:Actor, time:f32, excitement:f32, fear:f32)->vec4<u32> {
    let seed=a.size_phase.z;
    let beat=time*(2.0+1.4*a.size_phase.w)+6.2832*seed;
    let breath=sin(beat*0.54);
    let shift=sin(beat*0.23+5.0*seed);
    // Bouts of cheering separated by quiet watching; either hand can lead.
    let bout=smoothstep(-0.18,0.65,sin(beat*0.29+4.0*seed));
    let cheer=bout*(0.18+0.82*excitement)*(1.0-fear);
    let lead=step(0.5,seed);
    let sides=mix(vec2<f32>(1.,0.30),vec2<f32>(0.30,1.),lead);
    let wave=sin(vec2<f32>(beat,beat+1.2+seed));
    let shoulders=vec2<f32>(-0.10)-sides*cheer*(1.80+0.43*wave)
        +0.065*wave-vec2<f32>(0.57*fear);
    let elbows=vec2<f32>(-0.25)-0.13*(wave+1.)-cheer*sides*(0.56-0.30*wave)-0.72*fear;
    let spine=vec2<f32>(0.042*shift+0.02*cheer*breath,0.10*shift+0.13*cheer*(2.*lead-1.));
    let glance=sin(beat*0.31+seed*12.0);
    let neck=vec2<f32>(0.30*glance+0.08*sin(beat*0.64),0.035*breath+0.10*cheer*sin(beat-0.45)+0.16*fear);
    return vec4<u32>(pack2x16float(shoulders),pack2x16float(elbows),pack2x16float(spine),pack2x16float(neck));
}
struct Actor { pos_yaw:vec4<f32>, size_phase:vec4<f32>, color:vec4<f32>, identity:vec4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage,read> actors:array<Actor>;
@group(0) @binding(2) var<storage,read_write> visible:array<SeatLight>;
@group(0) @binding(3) var<storage,read_write> draws:array<atomic<u32>>;

// Same 80-byte scene instance as the stadium animation compute pass.
struct Pose { x:vec4<f32>, y:vec4<f32>, z:vec4<f32>, sphere:vec4<f32>, mesh_info:vec4<u32> }
@group(0) @binding(5) var<storage,read> poses:array<Pose>;
fn actor(i:u32)->Actor {
    var a=actors[i];
    if camera.game.x>0. {
        let pose=poses[u32(camera.game.y)+2u*i];
        a.pos_yaw=vec4<f32>(pose.x.w,pose.y.w,pose.z.w,a.pos_yaw.w);
        a.size_phase.y=pose.y.y;
        if pose.sphere.w<=0. {a.color.w=-1.;}
    }
    return a;
}

fn reserve_slot(a:Actor)->u32 {
    if a.color.w<0. {return 0xffffffffu;} let h=a.size_phase.y;
    let p=camera.vp*vec4<f32>(a.pos_yaw.xyz+vec3<f32>(0,h*0.5,0),1);
    var bin=0u;
    if a.color.w>0.5 {
        if p.w < -h || abs(p.x)>max(p.w,0.)+h*4. || abs(p.y)>max(p.w,0.)+h*4. { return 0xffffffffu; }
        let pixels=h*camera.settings.y/max(p.w,0.01);
        bin=1u;
        if pixels>32. {bin=2u;}
        if pixels>170. {bin=4u;}
    }
    let slot=atomicAdd(&draws[bin*5u+1u],1u);
    return bin*u32(camera.settings.w)+slot;
}

@compute @workgroup_size(64)
fn select_lod(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=u32(camera.settings.w) {return;}
    let a=actor(gid.x);
    let slot=reserve_slot(a);
    if slot!=0xffffffffu {
        visible[slot]=SeatLight(gid.x,0u,0u,0u,spectator_joints(a,camera.settings.x,0.45,0.));
    }
}

// A single receiver at upper torso height shares the scene's maps across all LODs.
@group(1) @binding(5) var sun_depth:texture_depth_2d;
@group(1) @binding(6) var seat_shadow_sampler:sampler_comparison;
@group(1) @binding(17) var<uniform> headlights:Headlights;
// First vec4 of the existing crowd pose uniform: time, excitement, fear, pivot.
@group(1) @binding(19) var<uniform> crowd_mood:vec4<f32>;
@group(1) @binding(18) var headlight_depth:texture_depth_2d_array;

fn spot_visibility(index:u32,uv:vec2<f32>,z:f32,lamp:Headlight,slope:vec2<f32>,world_p:vec3<f32>)->f32 {
    return textureSampleCompareLevel(headlight_depth,seat_shadow_sampler,uv,i32(index),z);
}
@compute @workgroup_size(64)
fn select_match(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=u32(camera.settings.w) {return;}
    let a=actor(gid.x);
    let slot=reserve_slot(a);
    if slot==0xffffffffu {return;}
    let p=a.pos_yaw.xyz+vec3<f32>(0.,0.75*a.size_phase.y,0.);
    var sun=1.;
    if camera.game.w>0. && camera.sun.w>0. {
        let clip=camera.shadow_vp*vec4<f32>(p+camera.sun.xyz*0.025,1.);
        let ndc=clip.xyz/clip.w;
        if all(abs(ndc.xy)<=vec2<f32>(1.)) && ndc.z>=0. && ndc.z<=1. {
            let uv=ndc.xy*vec2<f32>(0.5,-0.5)+0.5;
            sun=mix(1.,textureSampleCompareLevel(sun_depth,seat_shadow_sampler,uv,ndc.z),camera.sun.w);
        }
    }
    var energy=vec3<f32>(0.);
    var moment=vec3<f32>(0.);
    for(var i=0u;i<incident_light_count(p);i++){
        let sample=incident_light_sample(i,p,vec3<f32>(0.,1.,0.));
        energy+=sample.irradiance;
        moment+=sample.direction*dot(sample.irradiance,vec3<f32>(0.2126,0.7152,0.0722));
    }
    let direction=moment/max(dot(energy,vec3<f32>(0.2126,0.7152,0.0722)),1e-5);
    energy=min(energy,vec3<f32>(65504.));
    visible[slot]=SeatLight(gid.x,pack2x16float(energy.rg),pack2x16float(vec2<f32>(energy.b,0.)),
        pack4x8snorm(vec4<f32>(direction,sun)),spectator_joints(a,crowd_mood.x,crowd_mood.y,crowd_mood.z));
}
@group(0) @binding(4) var<storage,read> draw_ids:array<SeatLight>;
@group(1) @binding(0) var atlas:texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler:sampler;
struct V { @builtin(position) pos:vec4<f32>, @location(0) n:vec3<f32>, @location(1) local:vec3<f32>,
    @location(2) color:vec3<f32>, @location(3) @interpolate(flat) mat:u32,
    @location(4) identity:vec4<f32>, @location(5) disc:vec2<f32>, @location(6) uv:vec2<f32>, @location(7) @interpolate(flat) lamps:vec3<f32>,
    @location(8) @interpolate(flat) direction_sun:vec4<f32>, @location(9) world:vec3<f32> }
fn rotate(angle:f32) -> mat2x2<f32> {
    return mat2x2<f32>(cos(angle),sin(angle),-sin(angle),cos(angle));
}
fn turn_xz(v:vec3<f32>,rotation:mat2x2<f32>)->vec3<f32> {
    let xz=rotation*v.xz;
    return vec3<f32>(xz.x,v.y,xz.y);
}
fn artifact_sample(tile:u32, uv:vec2<f32>) -> vec4<f32> {
    let origin=vec2<f32>(f32(tile%4u),f32(tile/4u));
    let local=clamp(uv,vec2<f32>(0.001),vec2<f32>(0.999));
    return textureSampleLevel(atlas,atlas_sampler,(origin+local)/vec2<f32>(4.,2.),0.);
}
@vertex fn vs(@location(0) rest:vec3<f32>,@location(1) mat:u32,@location(2) normal:vec3<f32>,
    @location(3) arm:f32,@location(4) center:vec3<f32>,@location(5) bin:u32,
    @location(6) bones:vec4<f32>, @location(7) nose:vec4<f32>, @location(8) mouth:vec4<f32>,
    @location(9) brow:vec4<f32>, @location(10) adult:vec4<f32>, @builtin(instance_index) idx:u32)->V {
    let seat=draw_ids[bin*u32(camera.settings.w)+idx];
    let a=actor(seat.index);
    var p=rest; var n=normal; var anchor=center;
    let phase=camera.settings.x*(2.+a.size_phase.w)+a.size_phase.z*6.283;
    let anatomical=bin==4u && center.z< -0.9;
    var o:V;
    if mat>=20u && mat!=20u+u32(a.identity.z*3.) {
        o.pos=vec4<f32>(2.,2.,2.,1.);
        return o;
    }
    if bin!=0u {
        let motion=sin(phase);
        let shoulders=unpack2x16float(seat.joints.x);
        let elbows=unpack2x16float(seat.joints.y);
        let right_arm=select(arm>0.,rest.x>0.,anatomical);
        let angle=select(shoulders.x,shoulders.y,right_arm);
        let elbow_angle=select(elbows.x,elbows.y,right_arm);
        let activity=smoothstep(0.20,1.6,-min(shoulders.x,shoulders.y));
        if anatomical {
            let weights=a.identity*2.-1.;
            p+=0.055*(nose.xyz*weights.x+mouth.xyz*weights.z+brow.xyz*weights.y+adult.xyz*weights.w);
            let side=sign(rest.x);
            let shoulder=vec3<f32>(side*0.1008,0.805,-0.012);
            let elbow=vec3<f32>(side*0.1878,0.6994,-0.0132);
            let lower=rotate(elbow_angle);
            p=vec3<f32>(p.x,mix(p.yz,elbow.yz+lower*(p.yz-elbow.yz),bones.y));
            n=vec3<f32>(n.x,mix(n.yz,lower*n.yz,bones.y));
            let relax=rotate(-0.61*side);
            let rest_arm=vec3<f32>(shoulder.xy+relax*(p.xy-shoulder.xy),p.z);
            let swing=rotate(angle);
            p=mix(p,vec3<f32>(rest_arm.x,shoulder.yz+swing*(rest_arm.yz-shoulder.yz)),bones.x);
            let arm_normal=vec3<f32>(relax*n.xy,n.z);
            n=mix(n,vec3<f32>(arm_normal.x,swing*arm_normal.yz),bones.x);
        } else if arm!=0. {
            let pivot=vec3<f32>(arm*0.118,0.770,0.);
            let rot=mat2x2<f32>(cos(angle),sin(angle),-sin(angle),cos(angle));
            if center.y<0.65 {
                let elbow=vec3<f32>(arm*0.13,0.646,0.008);
                let forearm=rotate(elbow_angle);
                var e=p-elbow; p=elbow+vec3<f32>(e.x,forearm*e.yz);
                e=anchor-elbow; anchor=elbow+vec3<f32>(e.x,forearm*e.yz);
                n=vec3<f32>(n.x,forearm*n.yz);
            }
            var d=p-pivot; p=pivot+vec3<f32>(d.x,rot*d.yz);
            d=anchor-pivot; anchor=pivot+vec3<f32>(d.x,rot*d.yz);
            n=vec3<f32>(n.x,rot*n.yz);
        }
        // Lift the collarbone with the raising arm about its inner end.
        let arm_weight=select(f32(arm!=0.),bones.x,anatomical);
        if arm_weight>0. {
            let side=select(-1.,1.,right_arm);
            let collar=vec2<f32>(0.025*side,0.79);
            let lift=rotate(side*0.14*smoothstep(0.4,2.0,-angle)*arm_weight);
            p=vec3<f32>(collar+lift*(p.xy-collar),p.z);
            n=vec3<f32>(lift*n.xy,n.z);
            anchor=vec3<f32>(collar+lift*(anchor.xy-collar),anchor.z);
        }
        // Hierarchical hip, knee and ankle. Counter-rotate the ankle to
        // plant the shoe and lower the pelvis as the two leg segments flex.
        let bend=0.12+0.24*activity*(0.5+0.5*motion);
        let knee=vec2<f32>(0.269,0.0);
        let hip=vec2<f32>(0.527,-0.014);
        let ankle=vec2<f32>(0.046,0.0);
        let leg=select(select(0.,1.,(mat==2u||mat==3u)&&center.y<0.44),bones.z,anatomical);
        let shin=select(select(0.,1.,(mat==2u||mat==3u)&&center.y<0.22),bones.w,anatomical);
        let foot=shin*(1.-smoothstep(0.042,0.084,rest.y));
        let at_ankle=ankle+rotate(-bend)*(p.yz-ankle);
        let at_knee=knee+rotate(2.*bend)*(mix(p.yz,at_ankle,foot)-knee);
        let at_hip=hip+rotate(-bend)*(mix(p.yz,at_knee,shin)-hip);
        let drop=0.48*(1.-cos(bend));
        p=vec3<f32>(p.x,mix(p.yz,at_hip,leg))-vec3<f32>(0.,drop,0.);
        n=vec3<f32>(n.x,rotate(bend*(2.*shin-foot-leg))*n.yz);
        anchor.y-=drop;
        if (anatomical && rest.y>0.83) || mat==7u || (mat==4u && rest.y>0.84) {
            let jaw=1.+(a.identity.w-0.5)*0.28*(1.-smoothstep(0.87,0.92,p.y));
            let cheek=(0.87+0.22*a.identity.x)*jaw;
            p.x*=cheek; n.x/=cheek;
            let depth=0.92+0.18*a.identity.z;
            p.z*=depth; n.z/=depth;
        }
        // Neck first, then the spine parent. Skin, eyes and hair share the
        // same head motion; the upper body carries both shoulder joints.
        let spine=unpack2x16float(seat.joints.z);
        let neck=unpack2x16float(seat.joints.w);
        let rest_height=select(rest.y,center.y,bin==1u);
        let head_weight=smoothstep(0.81,0.87,rest_height);
        let chest_weight=max(smoothstep(0.46,0.70,rest_height),arm_weight);
        if head_weight>0. {
            let nod=rotate(neck.y*head_weight);
            let turn=rotate(neck.x*head_weight);
            let pivot=vec3<f32>(0.,0.835-drop,0.);
            p=vec3<f32>(p.x,pivot.yz+nod*(p.yz-pivot.yz));
            p=turn_xz(p,turn);
            n=vec3<f32>(n.x,nod*n.yz);
            n=turn_xz(n,turn);
            anchor=vec3<f32>(anchor.x,pivot.yz+nod*(anchor.yz-pivot.yz));
            anchor=turn_xz(anchor,turn);
        }
        if chest_weight>0. {
            let twist=rotate(spine.y*chest_weight);
            let lean=rotate(spine.x*chest_weight);
            let waist=vec2<f32>(0.,0.49-drop);
            p=turn_xz(p,twist);
            p=vec3<f32>(waist+lean*(p.xy-waist),p.z);
            n=turn_xz(n,twist);
            n=vec3<f32>(lean*n.xy,n.z);
            anchor=turn_xz(anchor,twist);
            anchor=vec3<f32>(waist+lean*(anchor.xy-waist),anchor.z);
        }
    }
    let scale=select(a.size_phase.xyz,vec3<f32>(a.size_phase.x,a.size_phase.y,a.size_phase.x),bin!=0u);
    let yaw=a.pos_yaw.w; let rot=mat3x3<f32>(vec3<f32>(cos(yaw),0,-sin(yaw)),vec3<f32>(0,1,0),vec3<f32>(sin(yaw),0,cos(yaw)));
    var world=a.pos_yaw.xyz+rot*(scale*p); var disc=vec2<f32>(0.);
    if bin==1u && camera.settings.z==2. {
        // A camera-facing elliptical splat for each articulated part.
        let d=(rest-center)*scale;
        world=a.pos_yaw.xyz+rot*(scale*anchor)+camera.right.xyz*d.x+camera.up.xyz*d.y;
        disc=sign(rest.xy-center.xy);
    }
    o.lamps=vec3<f32>(unpack2x16float(seat.rg),unpack2x16float(seat.blue).x);
    o.direction_sun=unpack4x8snorm(seat.direction_sun);
    o.world=world; o.uv=center.xy; o.pos=camera.vp*vec4<f32>(world,1); o.n=rot*(n/scale); o.local=rest;
    o.color=a.color.rgb; o.mat=mat; o.identity=a.identity; o.disc=disc; return o;
}
fn human_color(v:V)->vec4<f32>{
    if dot(v.disc,v.disc)>1. {discard;}
    let id=v.identity; let p=v.local; let n=normalize(v.n);
    let pigment=mix(vec3<f32>(0.28,0.125,0.076),vec3<f32>(0.85,0.55,0.409),id.y);
    let tile=u32(id.w*3.);
    // Median cheek colour sampled from the three source albedos. Use the
    // same pigmentation at the collar, wrists and across the detailed face.
    let cheeks=array<vec3<f32>,3>(vec3<f32>(0.855,0.659,0.541),vec3<f32>(0.894,0.741,0.639),vec3<f32>(0.353,0.224,0.149));
    let skin=mix(cheeks[tile],pigment,0.46);
    let hair=mix(vec3<f32>(0.046,0.030,0.025),vec3<f32>(0.39,0.28,0.16),id.w*id.w);
    var color=v.color;
    switch v.mat {
    case 1u: {let stripe=step(0.56,id.z)*step(0.54,p.y)*step(p.y,0.68)*step(0.,sin(p.y*276.)); color*=1.-0.24*stripe;}
    case 2u: {color=vec3<f32>(0.16,0.185,0.21)*(0.65+0.5*id.z);}
    case 3u: {color=vec3<f32>(0.105,0.083,0.073);}
    case 4u: {color=skin;}
    case 11u: {

        let tex=artifact_sample(tile,v.uv).rgb;

        color=mix(skin,clamp(skin*tex/cheeks[tile],vec3<f32>(0.),vec3<f32>(1.)),smoothstep(0.862,0.895,p.y));
    }
    case 12u: {
        let cloth=artifact_sample(6u,v.uv).rgb;
        let detail=dot(cloth,vec3<f32>(0.25,0.5,0.25));
        let upper=smoothstep(0.51,0.526,p.y);
        color=mix(cloth,v.color*(0.25+detail*2.5),upper);
    }
    case 13u,20u,21u,22u: {
        let tile=select(7u,v.mat-20u+3u,v.mat>=20u);
        let strand=artifact_sample(tile,v.uv);
        if strand.a<0.43 {discard;}
        color=hair*(0.36+2.4*dot(strand.rgb,vec3<f32>(0.3,0.5,0.2)));
    }
    case 6u: {
        let eye=length(vec2<f32>(abs(p.x)-0.01846,p.y-0.92704));
        color=mix(vec3<f32>(0.78,0.74,0.68),vec3<f32>(0.17,0.12+0.11*id.z,0.09),1.-smoothstep(0.0030,0.0036,eye));
        color=mix(vec3<f32>(0.025),color,smoothstep(0.0011,0.0016,eye));
        let glint=length(vec2<f32>(abs(p.x)-0.0176,p.y-0.92815));
        color=mix(color,vec3<f32>(0.92),1.-smoothstep(0.00045,0.00095,glint));
    }
    case 7u: {color=hair;}
    default: {}
    }
    if camera.game.x>0. {
        let direct=camera.sun_color.rgb*camera.game.w*v.direction_sun.w*max(dot(n,camera.sun.xyz),0.);
        // Wrapped diffuse retains opposing lamps when their first moments cancel.
        let local=v.lamps*max(0.,0.25+0.75*dot(n,v.direction_sun.xyz));
        let irradiance=camera.game.z*(0.5+0.25*n.y)+direct+local;
        return vec4<f32>(pow(max(color,vec3<f32>(0.)),vec3<f32>(2.2))*irradiance/3.14159,1.);
    }
    let sun=normalize(vec3<f32>(-0.65,0.85,0.5)); let light=0.68+0.33*max(dot(n,sun),0.)+0.07*n.y;
    return vec4<f32>(clamp(color*light,vec3<f32>(0.),vec3<f32>(1.)),1.);
}

@fragment fn fs_review(v:V)->@location(0) vec4<f32> {
    return human_color(v);
}

@fragment fn fs_match(v:V)->@location(0) vec4<f32> {
    let radiance = human_color(v).rgb;
    let ray = v.world - view.cam.xyz;
    let d = length(ray);
    return vec4<f32>(apply_distance_fog(radiance, d, ray / max(d, 1e-6)), 1.0);
}
