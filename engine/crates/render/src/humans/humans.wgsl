struct Camera {
    vp: mat4x4<f32>, right: vec4<f32>, up: vec4<f32>,
    settings: vec4<f32>, // time, focal length in pixels, far mode, actor count
}
struct Actor { pos_yaw:vec4<f32>, size_phase:vec4<f32>, color:vec4<f32>, identity:vec4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage,read> actors:array<Actor>;
@group(0) @binding(2) var<storage,read_write> visible:array<u32>;
@group(0) @binding(3) var<storage,read_write> draws:array<atomic<u32>>;

@compute @workgroup_size(64)
fn select_lod(@builtin(global_invocation_id) gid:vec3<u32>) {
    let i=gid.x; let count=u32(camera.settings.w); if i>=count{return;}
    let a=actors[i]; let h=a.size_phase.y;
    let p=camera.vp*vec4<f32>(a.pos_yaw.xyz+vec3<f32>(0,h*0.5,0),1);
    var bin=0u;
    if a.color.w>0.5 {
        if p.w < -h || abs(p.x)>max(p.w,0.)+h*4. || abs(p.y)>max(p.w,0.)+h*4. { return; }
        let pixels=h*camera.settings.y/max(p.w,0.01);
        bin=1u;
        if pixels>32. {bin=2u;}
        if pixels>170. {bin=4u;}
    }
    let slot=atomicAdd(&draws[bin*5u+1u],1u);
    visible[bin*count+slot]=i;
}
@group(0) @binding(4) var<storage,read> draw_ids:array<u32>;
@group(1) @binding(0) var atlas:texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler:sampler;
struct V { @builtin(position) pos:vec4<f32>, @location(0) n:vec3<f32>, @location(1) local:vec3<f32>,
    @location(2) color:vec3<f32>, @location(3) @interpolate(flat) mat:u32,
    @location(4) identity:vec4<f32>, @location(5) disc:vec2<f32>, @location(6) uv:vec2<f32> }
fn rotate(angle:f32) -> mat2x2<f32> {
    return mat2x2<f32>(cos(angle),sin(angle),-sin(angle),cos(angle));
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
    let a=actors[draw_ids[bin*u32(camera.settings.w)+idx]];
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
        let activity=smoothstep(0.18,0.54,a.size_phase.z);
        let wave=mix(-0.24,-1.85-0.75*motion,activity);
        let angle=select(0.13*sin(phase*0.67),wave,arm>0.);
        if anatomical {
            let weights=a.identity*2.-1.;
            p+=0.055*(nose.xyz*weights.x+mouth.xyz*weights.z+brow.xyz*weights.y+adult.xyz*weights.w);
            let side=sign(rest.x);
            let shoulder=vec3<f32>(side*0.1008,0.805,-0.012);
            let elbow=vec3<f32>(side*0.1878,0.6994,-0.0132);
            let lower=rotate(-0.28-0.48*activity);
            p=vec3<f32>(p.x,mix(p.yz,elbow.yz+lower*(p.yz-elbow.yz),bones.y));
            n=vec3<f32>(n.x,mix(n.yz,lower*n.yz,bones.y));
            let relax=rotate(-0.61*side);
            let rest_arm=vec3<f32>(shoulder.xy+relax*(p.xy-shoulder.xy),p.z);
            let swing=rotate(select(0.11*sin(phase*0.67),wave,side>0.));
            p=mix(p,vec3<f32>(rest_arm.x,shoulder.yz+swing*(rest_arm.yz-shoulder.yz)),bones.x);
            let arm_normal=vec3<f32>(relax*n.xy,n.z);
            n=mix(n,vec3<f32>(arm_normal.x,swing*arm_normal.yz),bones.x);
        } else if arm!=0. {
            let pivot=vec3<f32>(arm*0.118,0.770,0.);
            let rot=mat2x2<f32>(cos(angle),sin(angle),-sin(angle),cos(angle));
            if center.y<0.65 {
                let elbow=vec3<f32>(arm*0.13,0.646,0.008);
                let bend=-0.30-0.55*activity;
                let forearm=mat2x2<f32>(cos(bend),sin(bend),-sin(bend),cos(bend));
                var e=p-elbow; p=elbow+vec3<f32>(e.x,forearm*e.yz);
                e=anchor-elbow; anchor=elbow+vec3<f32>(e.x,forearm*e.yz);
                n=vec3<f32>(n.x,forearm*n.yz);
            }
            var d=p-pivot; p=pivot+vec3<f32>(d.x,rot*d.yz);
            d=anchor-pivot; anchor=pivot+vec3<f32>(d.x,rot*d.yz);
            n=vec3<f32>(n.x,rot*n.yz);
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
        let sway=0.013*sin(phase*0.65);
        p.x+=p.y*sway; anchor.x+=anchor.y*sway;
        if (anatomical && rest.y>0.83) || mat==7u || (mat==4u && rest.y>0.84) {
            let jaw=1.+(a.identity.w-0.5)*0.28*(1.-smoothstep(0.87,0.92,p.y));
            let cheek=(0.87+0.22*a.identity.x)*jaw;
            p.x*=cheek; n.x/=cheek;
            let depth=0.92+0.18*a.identity.z;
            p.z*=depth; n.z/=depth;
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
    o.uv=center.xy; o.pos=camera.vp*vec4<f32>(world,1); o.n=rot*(n/scale); o.local=rest;
    o.color=a.color.rgb; o.mat=mat; o.identity=a.identity; o.disc=disc; return o;
}
@fragment fn fs(v:V)->@location(0) vec4<f32>{
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
    let sun=normalize(vec3<f32>(-0.65,0.85,0.5)); let light=0.68+0.33*max(dot(n,sun),0.)+0.07*n.y;
    return vec4<f32>(clamp(color*light,vec3<f32>(0.),vec3<f32>(1.)),1.);
}
