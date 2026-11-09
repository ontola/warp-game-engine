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
struct V { @builtin(position) pos:vec4<f32>, @location(0) n:vec3<f32>, @location(1) local:vec3<f32>,
    @location(2) color:vec3<f32>, @location(3) @interpolate(flat) mat:u32,
    @location(4) identity:vec4<f32>, @location(5) disc:vec2<f32> }
// The scalp blend drives silhouette and pigment from the same hairline.
fn scalp(p:vec3<f32>, id:vec4<f32>) -> f32 {
    let front=smoothstep(0.009,0.042,p.z);
    let temple=0.007*smoothstep(0.023,0.050,abs(p.x))*front;
    let line=0.907+0.047*front+temple+0.004*sin(p.x*62.+id.x*4.)+0.015*id.w;
    return smoothstep(line-0.001,line+0.004,p.y);
}
@vertex fn vs(@location(0) rest:vec3<f32>,@location(1) mat:u32,@location(2) normal:vec3<f32>,
    @location(3) arm:f32,@location(4) center:vec3<f32>,@location(5) bin:u32,@builtin(instance_index) idx:u32)->V {
    let a=actors[draw_ids[bin*u32(camera.settings.w)+idx]];
    var p=rest; var n=normal; var anchor=center;
    let phase=camera.settings.x*(2.+a.size_phase.w)+a.size_phase.z*6.283;
    if bin!=0u {
        let motion=sin(phase);
        let activity=smoothstep(0.18,0.54,a.size_phase.z);
        let wave=mix(-0.24,-1.85-0.75*motion,activity);
        let angle=select(0.13*sin(phase*0.67),wave,arm>0.);
        if arm!=0. {
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
        if mat==5u {
            let growth=scalp(rest,a.identity)*(0.002+0.014*a.identity.z);
            let waves=0.72+0.28*cos(rest.x*210.+rest.z*120.);
            p+=growth*(n*waves+vec3<f32>(0.25,0.49,0.));
        }
        let sway=0.013*sin(phase*0.65);
        p.x+=p.y*sway; anchor.x+=anchor.y*sway;
        if mat==5u || mat==6u || mat==7u || (mat==4u && rest.y>0.84) {
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
    var o:V; o.pos=camera.vp*vec4<f32>(world,1); o.n=rot*(n/scale); o.local=rest;
    o.color=a.color.rgb; o.mat=mat; o.identity=a.identity; o.disc=disc; return o;
}
@fragment fn fs(v:V)->@location(0) vec4<f32>{
    if dot(v.disc,v.disc)>1. {discard;}
    let id=v.identity; let p=v.local; let n=normalize(v.n);
    let skin=mix(vec3<f32>(0.28,0.125,0.076),vec3<f32>(0.85,0.55,0.409),id.y);
    let hair=mix(vec3<f32>(0.046,0.030,0.025),vec3<f32>(0.39,0.28,0.16),id.w*id.w);
    var color=v.color;
    switch v.mat {
    case 1u: {let stripe=step(0.56,id.z)*step(0.54,p.y)*step(p.y,0.68)*step(0.,sin(p.y*276.)); color*=1.-0.24*stripe;}
    case 2u: {color=vec3<f32>(0.16,0.185,0.21)*(0.65+0.5*id.z);}
    case 3u: {color=vec3<f32>(0.105,0.083,0.073);}
    case 4u: {color=skin;}
    case 5u: {
        color=skin;
        let front=smoothstep(0.012,0.043,p.z);
        let lip=exp(-pow(abs(p.x)/0.021,2.)-pow(abs(p.y-0.886)/0.0035,2.))*front;
        color=mix(color,color*vec3<f32>(0.89,0.62,0.66),lip);
        let brow=exp(-pow(abs(abs(p.x)-0.019)/0.016,4.)-pow(abs(p.y-0.936+0.08*abs(p.x))/0.0015,2.))*front;
        color=mix(color,hair,brow*0.85);

        let forehead=smoothstep(0.932,0.947,p.y)*(1.-smoothstep(0.954,0.963,p.y))*front;
        let age=smoothstep(0.48,0.90,id.x);
        let crease=pow(0.5+0.5*cos(p.y*1730.+4.*cos(p.x*69.)+2.*sin(p.x*117.+p.y*61.)),8.);
        color*=1.-0.04*crease*forehead*age*(1.-smoothstep(0.019,0.047,abs(p.x)));
        let crop=scalp(p,id);
        color=mix(color,hair*(0.94+0.06*sin(p.x*1160.+16.*sin(p.z*48.)+p.y*380.)),crop);
    }
    case 6u: {
        let eye=length(vec2<f32>(abs(p.x)-0.01846,p.y-0.92455));
        color=mix(vec3<f32>(0.78,0.74,0.68),vec3<f32>(0.17,0.12+0.11*id.z,0.09),1.-smoothstep(0.0030,0.0036,eye));
        color=mix(vec3<f32>(0.025),color,smoothstep(0.0011,0.0016,eye));
        let glint=length(vec2<f32>(abs(p.x)-0.0176,p.y-0.9257));
        color=mix(color,vec3<f32>(0.92),1.-smoothstep(0.00045,0.00095,glint));
    }
    case 7u: {color=hair;}
    default: {}
    }
    let sun=normalize(vec3<f32>(-0.65,0.85,0.5)); let light=0.68+0.33*max(dot(n,sun),0.)+0.07*n.y;
    return vec4<f32>(clamp(color*light,vec3<f32>(0.),vec3<f32>(1.)),1.);
}
