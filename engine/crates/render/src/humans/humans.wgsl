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
struct SeatLight { index:u32, rg:u32, blue_turn:u32, direction_sun:u32, joints:vec4<u32>, legs:vec4<u32> }

// A smooth reach, a brief hold, then release. Each interval gets its own
// height and timing; both ends meet at zero even when the next pulse differs.
fn alarm_pulse(clock:f32, salt:u32)->f32 {
    var bits=u32(floor(clock))^salt;
    bits=(bits^(bits>>16u))*0x7feb352du;
    bits=(bits^(bits>>15u))*0x846ca68bu;
    bits=bits^(bits>>16u);
    let phase=fract(clock);
    let attack=0.22+0.17*f32(bits & 255u)/255.;
    let strength=0.18+0.82*f32((bits>>8u) & 255u)/255.;
    return strength*smoothstep(0.,attack,phase)*(1.-smoothstep(attack+0.10,0.90,phase));
}

fn gesture_fear(mood:f32, row_offset:f32)->f32 {
    return max(mood,0.78*smoothstep(0.015,0.40,abs(row_offset)));
}

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
    var shoulders=vec2<f32>(-0.10)-sides*cheer*(1.80+0.43*wave)
        +0.065*wave-vec2<f32>(0.57*fear);
    var elbows=vec2<f32>(-0.25)-0.13*(wave+1.)-cheer*sides*(0.56-0.30*wave)-0.72*fear;
    var spine=vec2<f32>(0.042*shift+0.02*cheer*breath,0.10*shift+0.13*cheer*(2.*lead-1.));
    let glance=sin(beat*0.31+seed*12.0);
    var neck=vec2<f32>(0.30*glance+0.08*sin(beat*0.64),0.035*breath+0.10*cheer*sin(beat-0.45)+0.16*fear);
    let alarm=smoothstep(0.08+0.12*seed,0.57+0.16*seed,fear);
    if alarm>0. {
        let salt=u32(seed*16777216.);
        let clock=time*(1.55+0.75*a.size_phase.w)+seed*19.;
        let reach=vec2<f32>(alarm_pulse(clock,salt),
            alarm_pulse(clock*(0.89+0.19*a.identity.z)+0.37,salt^0x9e3779b9u));
        let other_arm=0.34+0.66*smoothstep(0.18,0.72,a.size_phase.w);
        let lift=mix(vec2<f32>(other_arm,1.),vec2<f32>(1.,other_arm),lead);
        shoulders=mix(shoulders,-0.72-lift*(1.45+0.72*reach),alarm);
        elbows=mix(elbows,-1.28+lift*(0.28+0.65*reach),alarm);
        let twist=(reach.x-reach.y)*alarm;
        spine+=vec2<f32>(0.050,0.095)*twist;
        neck+=vec2<f32>(0.15*twist,-0.075*alarm*max(reach.x,reach.y));
    }
    return vec4<u32>(pack2x16float(shoulders),pack2x16float(elbows),pack2x16float(spine),pack2x16float(neck));
}
// Pack hip pitch, knee flexion, hip roll and pelvis drop once per visible leg.
fn pack_leg(hip:f32,knee:f32,roll:f32,drop:f32)->vec2<u32> {
    return vec2<u32>(pack2x16float(vec2<f32>(hip,knee)),pack2x16float(vec2<f32>(roll,drop)));
}
fn leg_to(x:f32,forward:f32,lift:f32,drop:f32)->vec2<u32> {
    let thigh=0.258;
    let shin=0.223;
    let down=thigh+shin-drop-lift;
    let plane=sqrt(down*down+x*x);
    let knee=acos(clamp((plane*plane+forward*forward-thigh*thigh-shin*shin)/(2.*thigh*shin),-1.,1.));
    let hip=-atan2(forward,plane)-atan2(shin*sin(knee),thigh+shin*cos(knee));
    return pack_leg(hip,knee,atan2(x,down),drop);
}
fn stepping_leg(cycle:f32, stride:f32, weight:f32, drop:f32, swing_start:f32, shift:f32, forward:f32)->vec2<u32> {
    let phase=fract(cycle);
    let swing=smoothstep(swing_start,swing_start+0.42,phase);
    // During stance, adding the moving root back to x gives a fixed footfall.
    let x=(swing-phase)*stride*weight;
    let lift=0.074*weight*4.*swing*(1.-swing);
    return leg_to(x-shift,forward,lift,drop);
}
fn spectator_legs(a:Actor,time:f32,joints:vec4<u32>,travel:f32,speed:f32,crouch:f32)->vec4<u32> {
    let arm=unpack2x16float(joints.x);
    let shift=0.42*unpack2x16float(joints.z).x;
    let stagger=(a.size_phase.z-0.5)*0.065;
    let activity=smoothstep(0.20,1.6,-min(arm.x,arm.y));
    let phase=time*(2.+a.size_phase.w)+6.2832*a.size_phase.z;
    let bend=0.12+0.24*activity*(0.5+0.5*sin(phase));
    let drop=0.004+0.481*(1.-cos(bend));
    if speed<=0.015 {
        return vec4<u32>(leg_to(-shift,stagger,0.,drop+crouch),leg_to(-shift,-stagger,0.,drop+crouch));
    }
    let weight=smoothstep(0.015,0.65,speed);
    let stride=0.32+0.05*a.size_phase.w;
    let cycle=travel/stride;
    // Lower the hips enough to reach the planted foot as the other one swings.
    let pelvis=mix(drop,0.051,weight)+crouch;
    let left=stepping_leg(cycle,stride,weight,pelvis,0.54,shift,stagger);
    let right=stepping_leg(cycle,stride,weight,pelvis,0.04,shift,-stagger);
    return vec4<u32>(left,right);
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
        let joints=spectator_joints(a,camera.settings.x,0.45,0.);
        visible[slot]=SeatLight(gid.x,0u,0u,0u,joints,spectator_legs(a,camera.settings.x,joints,0.,0.,0.));
    }
}

// A single receiver at upper torso height shares the scene's maps across all LODs.
@group(1) @binding(5) var sun_depth:texture_depth_2d;
@group(1) @binding(6) var seat_shadow_sampler:sampler_comparison;
@group(1) @binding(17) var<uniform> headlights:Headlights;
// First vec4 of the existing crowd pose uniform: time, excitement, fear, pivot.
@group(1) @binding(19) var<uniform> crowd_mood:vec4<f32>;
@group(1) @binding(20) var<storage,read> escapes:array<Reaction>;
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
    // Match the collision pose's height reduction by bending the legs.
    // Keep the size of the skull, torso and limb segments throughout.
    let crouch=0.22*crowd_mood.z;
    let p=a.pos_yaw.xyz+vec3<f32>(0.,(0.75-crouch)*a.size_phase.y,0.);
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
    let escape=escapes[gid.x];
    let offset=displacement(escape,crowd_mood.x);
    let speed=abs(displacement(escape,crowd_mood.x+1./120.)-offset)*120.;
    let right=vec2<f32>(cos(a.pos_yaw.w),-sin(a.pos_yaw.w));
    let travel=dot(a.pos_yaw.xz-actors[gid.x].pos_yaw.xz,right)/a.size_phase.x;
    let alarm=gesture_fear(crowd_mood.z,offset);
    var joints=spectator_joints(a,crowd_mood.x,crowd_mood.y,alarm);
    let step_weight=smoothstep(0.12,0.75,speed);
    if step_weight>0. {
        let counter=sin(travel*2.*PI/(0.32+0.05*a.size_phase.w))*step_weight;
        joints.x=pack2x16float(unpack2x16float(joints.x)+vec2<f32>(0.15,-0.15)*counter);
        joints.z=pack2x16float(unpack2x16float(joints.z)+vec2<f32>(0.025,0.09)*counter);
    }
    let legs=spectator_legs(a,crowd_mood.x,joints,travel,speed,crouch);
    visible[slot]=SeatLight(gid.x,pack2x16float(energy.rg),pack2x16float(vec2<f32>(energy.b,blast_turn_at(escape,crowd_mood.x))),
        pack4x8snorm(vec4<f32>(direction,sun)),joints,legs);
}
@group(0) @binding(4) var<storage,read> draw_ids:array<SeatLight>;
@group(1) @binding(0) var atlas:texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler:sampler;
struct V { @builtin(position) pos:vec4<f32>, @location(0) n:vec3<f32>, @location(1) local:vec3<f32>,
    @location(2) color:vec3<f32>, @location(3) @interpolate(flat) mat:u32,
    @location(4) identity:vec4<f32>, @location(5) disc:vec2<f32>, @location(6) uv:vec2<f32>, @location(7) @interpolate(flat) lamps:vec3<f32>,
    @location(8) @interpolate(flat) direction_sun:vec4<f32>, @location(9) world:vec3<f32>, @location(10) @interpolate(flat) appearance:u32 }
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
@vertex fn vs(@location(0) position:vec4<f32>,@location(1) direction:vec4<f32>,@location(2) pivot:vec4<f32>,
    @location(3) tags:vec4<u32>,@location(4) bones:vec4<f32>,@location(5) face0:vec4<f32>,
    @location(6) face1:vec4<f32>,@location(7) face2:vec4<f32>,@location(8) face3:vec4<f32>,
    @builtin(instance_index) idx:u32)->V {
    let rest=position.xyz*1.2; let normal=direction.xyz; let center=pivot.xyz;
    let tagged_mat=tags.x; let arm=f32(tags.y)-1.; let bin=tags.z;
    let seat=draw_ids[bin*u32(camera.settings.w)+idx];
    let a=actor(seat.index);
    var p=rest; var n=normal; var anchor=center;
    let anatomical=bin==4u && center.z< -0.9;
    var o:V;
    let mat=tagged_mat & 31u;
    let mask=tagged_mat >> 5u;
    // People store wardrobe + 1 in the low nine bits. Zero is a stand.
    let appearance=u32(max(0.,a.color.w-1.));
    let outfit=appearance & 3u;
    if (mask!=0u && (mask & (1u<<outfit))==0u) || (mat>=20u && mat!=20u+u32(a.identity.z*3.)) {
        o.pos=vec4<f32>(2.,2.,2.,1.);
        return o;
    }
    if bin!=0u {
        let shoulders=unpack2x16float(seat.joints.x);
        let elbows=unpack2x16float(seat.joints.y);
        let right_arm=select(arm>0.,rest.x>0.,anatomical);
        let angle=select(shoulders.x,shoulders.y,right_arm);
        let elbow_angle=select(elbows.x,elbows.y,right_arm);
        let zones=1.-clamp(abs(vec3<f32>(rest.y)-vec3<f32>(0.78,0.57,0.48))/vec3<f32>(0.12,0.13,0.11),vec3<f32>(0.),vec3<f32>(1.));
        let width=1.+dot(zones*zones*(3.-2.*zones),vec3<f32>(0.13,0.12,0.10)*(2.*vec3<f32>(a.size_phase.zw,a.identity.x)-1.));
        let head=select(smoothstep(0.825,0.860,select(rest.y,center.y,bin==1u)),face3.w,anatomical);
        let breadth=mix(width,1.,head);
        let skull=mix(1.,a.size_phase.y/a.size_phase.x,head);
        p*=vec3<f32>(breadth*skull,1.,skull);
        n/=vec3<f32>(breadth*skull,1.,skull);
        anchor*=vec3<f32>(breadth*skull,1.,skull);
        if anatomical {
            let squared=a.identity*a.identity;
            let raw=squared*squared+0.001;
            let weights=raw/dot(raw,vec4<f32>(1.));
            p+=vec3<f32>(breadth*skull,1.,skull)*0.055*(face0.xyz*weights.x+face1.xyz*weights.y+face2.xyz*weights.z+face3.xyz*weights.w);
            let closure=vec3<f32>(face0.w,face1.w,face2.w);
            if any(closure!=vec3<f32>(0.)) {
                let phase=fract(camera.settings.x/(3.1+1.8*a.identity.x)+a.identity.x);
                let blink=smoothstep(0.,0.020,phase)*(1.-smoothstep(0.025,0.064,phase));
                p+=0.012*closure*blink;
            }
            if mat==21u {
                let fringe=clamp((p.z-0.033)/0.027,0.,1.)*clamp(1.-abs(p.x-0.006)/0.052,0.,1.)*clamp((0.974-p.y)/0.032,0.,1.);
                p+=vec3<f32>(-0.012,0.030,0.)*fringe;
            }
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
        let right_leg=select(center.x>0.,rest.x>0.,anatomical);
        let joints=select(seat.legs.xy,seat.legs.zw,right_leg);
        let pitch=unpack2x16float(joints.x);
        let roll_drop=unpack2x16float(joints.y);
        let drop=roll_drop.y;
        let hip=vec3<f32>(select(-0.055,0.055,right_leg),0.527,0.);
        let ankle=vec3<f32>(hip.x,0.046,0.);
        let knee=vec2<f32>(0.269,0.);
        let leg=select(f32((mat==2u||mat==3u)&&center.y<0.44),bones.z,anatomical);
        // The far leg is a single shape. Place its centre on the line from
        // the planted hip to the solved ankle so it still meets the shoe.
        let single_leg=bin==1u && mat==2u && center.y<0.44;
        let shin=select(f32((mat==2u||mat==3u)&&center.y<0.22 || single_leg),bones.w,anatomical);
        let foot=shin*select(f32(mat==3u),1.-smoothstep(0.042,0.084,rest.y),anatomical);
        if leg>0. {
            let foot_roll=rotate(-roll_drop.x*foot);
            let foot_pitch=rotate(-(pitch.x+pitch.y)*foot);
            let knee_pitch=rotate(pitch.y*shin);
            let hip_pitch=rotate(pitch.x*leg);
            let hip_roll=rotate(roll_drop.x*leg);
            // For impostors the animated centre moves the entire splat.
            let impostor=bin==1u && camera.settings.z==2.;
            let source=select(select(p,anchor,impostor),ankle,single_leg);
            var point=vec3<f32>(ankle.xy+foot_roll*(source.xy-ankle.xy),source.z);
            point=vec3<f32>(point.x,ankle.yz+foot_pitch*(point.yz-ankle.yz));
            point=vec3<f32>(point.x,knee+knee_pitch*(point.yz-knee));
            point=vec3<f32>(point.x,hip.yz+hip_pitch*(point.yz-hip.yz));
            point=vec3<f32>(hip.xy+hip_roll*(point.xy-hip.xy),point.z);
            if single_leg {
                point=mix(hip,point,(hip.y-center.y)/(hip.y-ankle.y));
                p=point+(p-center);
            } else { p=point; }
            anchor=point;
            n=vec3<f32>(foot_roll*n.xy,n.z);
            n=vec3<f32>(n.x,hip_pitch*(knee_pitch*(foot_pitch*n.yz)));
            n=vec3<f32>(hip_roll*n.xy,n.z);
        }
        let pelvis_shift=0.42*unpack2x16float(seat.joints.z).x;
        p.x+=pelvis_shift; anchor.x+=pelvis_shift;
        p.y-=drop; anchor.y-=drop;
        // Neck first, then the spine parent. Skin, eyes and hair share the
        // same head motion; the upper body carries both shoulder joints.
        let spine=unpack2x16float(seat.joints.z);
        let neck=unpack2x16float(seat.joints.w);
        let blast_turn=unpack2x16float(seat.blue_turn).y;
        let rest_height=select(rest.y,center.y,bin==1u);
        let head_weight=select(smoothstep(0.81,0.87,rest_height),face3.w,anatomical);
        let chest_weight=max(smoothstep(0.46,0.70,rest_height),arm_weight);
        if head_weight>0. {
            let nod=rotate(neck.y*head_weight);
            let turn=rotate((neck.x+0.23*blast_turn)*head_weight);
            let pivot=vec3<f32>(0.,0.835-drop,0.);
            p=vec3<f32>(p.x,pivot.yz+nod*(p.yz-pivot.yz));
            p=turn_xz(p,turn);
            n=vec3<f32>(n.x,nod*n.yz);
            n=turn_xz(n,turn);
            anchor=vec3<f32>(anchor.x,pivot.yz+nod*(anchor.yz-pivot.yz));
            anchor=turn_xz(anchor,turn);
        }
        // Feet keep their row heading; the pelvis pivots over the legs and
        // the shoulders and neck carry the rest of the turn.
        let hip_weight=max(smoothstep(0.08,0.46,rest_height),arm_weight);
        if chest_weight>0. || (blast_turn!=0. && hip_weight>0.) {
            let twist=rotate(spine.y*chest_weight+blast_turn*(0.34*hip_weight+0.43*chest_weight));
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
    o.lamps=vec3<f32>(unpack2x16float(seat.rg),unpack2x16float(seat.blue_turn).x);
    o.direction_sun=unpack4x8snorm(seat.direction_sun);
    o.world=world; o.uv=center.xy; o.pos=camera.vp*vec4<f32>(world,1); o.n=rot*(n/scale); o.local=rest;
    if mat==6u { o.uv=normal.xy; }
    var dye=a.color.x;
    if mat==2u || mat==14u {dye=a.color.y;}
    if mat==3u {dye=a.color.z;}
    o.color=unpack4x8unorm(u32(dye)).rgb;
    o.appearance=appearance; o.mat=select(mat,1u,!anatomical && arm!=0. && mat==4u && outfit!=1u && rest.y>0.558); o.identity=a.identity; o.disc=disc; return o;
}
fn human_color(v:V, eye_dir:vec3<f32>)->vec4<f32>{
    if dot(v.disc,v.disc)>1. {discard;}
    let id=v.identity; let p=v.local; let n=normalize(v.n);
    let pigment=mix(vec3<f32>(0.28,0.125,0.076),vec3<f32>(0.85,0.55,0.409),id.y);
    let tile=u32(id.w*3.);
    // Median cheek colour sampled from the three source albedos. Use the
    // same pigmentation at the collar, wrists and across the detailed face.
    let cheeks=array<vec3<f32>,3>(vec3<f32>(0.855,0.659,0.541),vec3<f32>(0.894,0.741,0.639),vec3<f32>(0.353,0.224,0.149));
    let skin=mix(cheeks[tile],pigment,0.46);
    let hair_bits=v.appearance>>9u;
    let hair=vec3<f32>(vec3<u32>(hair_bits>>8u,hair_bits>>4u,hair_bits) & vec3<u32>(15u))/15.;
    let wardrobe=v.appearance & 511u;
    var color=v.color;
    var specular=0.;
    var gloss=32.;
    switch v.mat {
    case 3u: {color*=mix(0.71,1.,smoothstep(0.012,0.020,p.y));}
    case 4u: {color=skin;}
    case 11u: {

        let face=artifact_sample(tile,v.uv);
        let tex=face.rgb;
        let sheen=clamp(2.*face.a-1.,0.,1.);
        specular=0.045+0.15*sheen;
        gloss=18.+46.*sheen;

        color=mix(skin,clamp(skin*tex/cheeks[tile],vec3<f32>(0.),vec3<f32>(1.)),smoothstep(0.862,0.895,p.y));
    }
    case 12u,14u: {
        var outfit=wardrobe & 3u;
        if v.mat==14u && outfit>0u {outfit=1u;}
        color*=artifact_sample(6u,v.uv)[outfit];
        let print=wardrobe>>2u;
        let chest=vec2<f32>(p.x/0.19+0.5,(0.738-p.y)/0.14+0.5);
        if v.mat==12u && print!=0u && p.z>0.02 && all(chest>vec2<f32>(0.)) && all(chest<vec2<f32>(1.)) {
            var glyph=print-91u;
            var local=chest;
            if print<=100u {
                let number=print-1u;
                glyph=select(number/10u,number%10u,chest.x>=0.5);
                local.x=fract(2.*chest.x);
            }
            let uv=(vec2<f32>(f32(glyph%4u),f32(glyph/4u))+local)*0.25;
            let ink=select(0.88,0.14,dot(v.color,vec3<f32>(0.3,0.6,0.1))>0.58);
            color=mix(color,vec3<f32>(ink),0.89*artifact_sample(6u,uv).a);
        }
    }
    case 13u,20u,21u,22u: {
        let tile=select(7u,v.mat-20u+3u,v.mat>=20u);
        let strand=artifact_sample(tile,v.uv);
        if strand.a<0.43 {discard;}
        let brow=mix(mix(vec3<f32>(0.09,0.065,0.05),vec3<f32>(0.47,0.36,0.26),f32(hair_bits>>12u)/5.),skin,0.20);
        // Use the same source highlights as the fitted player grooms. In
        // particular the light bob texture must retain a dark selected tint.
        let highlights=array<f32,5>(45.,222.,27.,1.,24.);
        let luma=dot(strand.rgb,vec3<f32>(0.3,0.5,0.2));
        let fibre=0.22+0.72*sqrt(min(1.1,luma*(255./highlights[tile-3u])));
        color=select(hair,brow,v.mat==13u)*fibre;
    }
    case 6u: {
        let clock=camera.settings.x+17.*id.x;
        let glance=vec2<f32>(smoothstep(0.20,0.65,sin(clock*1.41)),smoothstep(0.35,0.76,sin(clock*0.92)));
        let gaze=(glance-0.5)*vec2<f32>(0.28,0.13);
        let iris=length(v.uv-gaze);
        let rim=1.-smoothstep(0.43,0.49,iris);
        let stroma=mix(vec3<f32>(0.26,0.16,0.095),vec3<f32>(0.29,0.34,0.27),id.z);
        color=mix(vec3<f32>(0.79,0.76,0.72),mix(stroma,stroma*0.44,smoothstep(0.35,0.46,iris)),rim);
        color=mix(vec3<f32>(0.027,0.024,0.021),color,smoothstep(0.16,0.22,iris));
        color*=1.-0.42*smoothstep(0.02,0.55,v.uv.y);
        specular=0.95; gloss=95.;
    }
    case 7u: {color=hair;}
    default: {}
    }
    let sun=select(normalize(vec3<f32>(-0.65,0.85,0.5)),camera.sun.xyz,camera.game.x>0.);
    var sun_glint=0.;
    var lamp_glint=0.;
    if specular>0. {
        sun_glint=specular*pow(max(dot(n,normalize(eye_dir+sun)),0.),gloss)*max(dot(n,sun),0.);
        let lamp=v.direction_sun.xyz;
        lamp_glint=specular*pow(max(dot(n,normalize(eye_dir+lamp)),0.),gloss)*max(dot(n,lamp),0.);
    }
    if camera.game.x>0. {
        let direct=camera.sun_color.rgb*camera.game.w*v.direction_sun.w*max(dot(n,camera.sun.xyz),0.);
        // Wrapped diffuse retains opposing lamps when their first moments cancel.
        let local=v.lamps*max(0.,0.25+0.75*dot(n,v.direction_sun.xyz));
        let irradiance=camera.game.z*(0.5+0.25*n.y)+direct+local;
        return vec4<f32>(pow(max(color,vec3<f32>(0.)),vec3<f32>(2.2))*irradiance/3.14159+
            (sun_glint*camera.sun_color.rgb*camera.game.w*v.direction_sun.w+lamp_glint*v.lamps)/3.14159,1.);
    }
    let light=0.68+0.33*max(dot(n,sun),0.)+0.07*n.y;
    return vec4<f32>(clamp(color*light+sun_glint,vec3<f32>(0.),vec3<f32>(1.)),1.);
}

@fragment fn fs_review(v:V)->@location(0) vec4<f32> {
    return human_color(v,normalize(cross(camera.right.xyz,camera.up.xyz)));
}

@fragment fn fs_match(v:V)->@location(0) vec4<f32> {
    let ray = v.world - view.cam.xyz;
    let radiance = human_color(v,normalize(-ray)).rgb;
    let d = length(ray);
    return vec4<f32>(apply_distance_fog(radiance, d, ray / max(d, 1e-6)), 1.0);
}
