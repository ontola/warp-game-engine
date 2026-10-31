struct ExhaustUniform { output_vp:mat4x4<f32>,depth_vp:mat4x4<f32>,camera_near:vec4<f32>,right:vec4<f32>,up:vec4<f32>,size:vec4<f32>,ambient:vec4<f32> };
struct ExhaustSprite {position_radius:vec4<f32>,axis_length:vec4<f32>,colour_alpha:vec4<f32>,params:vec4<f32>};
@group(0) @binding(0) var<uniform> exhaust:ExhaustUniform;
@group(0) @binding(1) var<storage,read> exhaust_sprites:array<ExhaustSprite>;
@group(0) @binding(2) var<storage,read> exhaust_order:array<u32>;
@group(0) @binding(3) var exhaust_background:texture_2d<f32>;
@group(0) @binding(4) var exhaust_depth:texture_depth_2d;
@group(0) @binding(5) var exhaust_sampler:sampler;
struct ExhaustVertex {@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) world:vec3<f32>,@location(2) @interpolate(flat) id:u32};
@vertex fn exhaust_vs(@builtin(vertex_index) v:u32,@builtin(instance_index) i:u32)->ExhaustVertex {
    let q=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(-1.0,1.0),vec2<f32>(-1.0,1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0))[v];
    let id=exhaust_order[i];let s=exhaust_sprites[id];var p=s.position_radius.xyz;
    if s.params.x>3.5 {return ExhaustVertex(vec4<f32>(q,0.0,1.0),q,p,id);}
    if s.params.x>2.5 {p+=exhaust.right.xyz*q.x*s.position_radius.w+exhaust.up.xyz*q.y*s.axis_length.w;}
    else if s.params.x<0.5 {p+=(exhaust.right.xyz*q.x+exhaust.up.xyz*q.y)*s.position_radius.w;}
    else {let axis=normalize(s.axis_length.xyz);let cross_side=cross(axis,exhaust.camera_near.xyz-p);var side=exhaust.right.xyz;if dot(cross_side,cross_side)>1e-8{side=normalize(cross_side);}let along=(q.y+1.0)*0.5;p+=axis*along*s.axis_length.w+side*q.x*s.position_radius.w;}
    return ExhaustVertex(exhaust.output_vp*vec4<f32>(p,1.0),q,p,id);
}
fn exhaust_depth_metres(uv:vec2<f32>)->f32 {
    let dims=vec2<i32>(textureDimensions(exhaust_depth));let px=clamp(vec2<i32>(uv*vec2<f32>(dims)),vec2<i32>(0),dims-1);
    return exhaust.camera_near.w/max(textureLoad(exhaust_depth,px,0),1e-9);
}
@fragment fn exhaust_fs(in:ExhaustVertex)->@location(0) vec4<f32> {
    let s=exhaust_sprites[in.id];
    if s.params.x>3.5 {
        let uv=in.position.xy*exhaust.size.zw;let p=(uv-.5)*2.;let r=length(p);
        let strength=s.colour_alpha.a;let envelope=smoothstep(.12,1.1,r)*strength;
        let pulse=sin(r*16.-s.params.z*11.)*.35;
        let bend=p*(1.+pulse)*envelope*s.params.w*exhaust.size.zw;
        let clamp_uv=clamp(uv+bend,exhaust.size.zw*.5,1.-exhaust.size.zw*.5);
        let split=p*envelope*2.5*exhaust.size.zw;
        var colour=textureSampleLevel(exhaust_background,exhaust_sampler,clamp_uv,0.).rgb;
        colour.r=textureSampleLevel(exhaust_background,exhaust_sampler,clamp(clamp_uv+split,exhaust.size.zw*.5,1.-exhaust.size.zw*.5),0.).r;
        colour.b=textureSampleLevel(exhaust_background,exhaust_sampler,clamp(clamp_uv-split,exhaust.size.zw*.5,1.-exhaust.size.zw*.5),0.).b;
        return vec4<f32>(colour*strength,strength);
    }
    let depth_clip=exhaust.depth_vp*vec4<f32>(in.world,1.0);
    let depth_uv=depth_clip.xy/depth_clip.w*vec2<f32>(0.5,-0.5)+0.5;
    let gap=exhaust_depth_metres(depth_uv)-depth_clip.w;
    if gap<=0.0 {discard;}
    if s.params.x>2.5 {
        let local=in.uv;let radius=length(local);let envelope=1.-smoothstep(.32,1.,radius);
        let flow=sin(radius*17.-s.params.z*7.+local.x*2.);
        let waves=vec2<f32>(sin(local.y*8.+s.params.z*3.1),cos(local.x*9.-s.params.z*2.7));
        let fade=smoothstep(9.,17.,depth_clip.w)*(1.-smoothstep(400.,600.,depth_clip.w));
        let bend=(local*(.55+flow*.45)+waves*.16)*envelope*s.colour_alpha.a*s.params.w*exhaust.size.zw*30./max(12.,depth_clip.w);
        let at=clamp(in.position.xy*exhaust.size.zw+bend,exhaust.size.zw*.5,1.-exhaust.size.zw*.5);
        let behind=smoothstep(0.,1.2,min(gap,exhaust_depth_metres(depth_uv+bend)-depth_clip.w));
        let alpha=envelope*s.colour_alpha.a*fade*.9*behind;
        return vec4<f32>(textureSampleLevel(exhaust_background,exhaust_sampler,at,0.).rgb*alpha,alpha);
    }
    let soften=clamp(gap/max(0.05,s.position_radius.w),0.0,1.0);
    let seed=s.params.y;let time=s.params.z;let uv=in.uv;
    let swirl=sin(uv.x*5.7+uv.y*4.3+seed+time*3.2)*sin(uv.y*6.1-uv.x*3.2-seed*1.7-time*2.1);
    var alpha=0.0;var color=vec3<f32>(0.0);
    if s.params.x<0.5 {
        let shape=max(0.0,1.0-dot(uv,uv));alpha=shape*shape*(0.76+0.24*swirl)*s.colour_alpha.a*soften;
        color=s.colour_alpha.rgb*exhaust.ambient.rgb;
    } else {
        let along=(uv.y+1.0)*0.5;let taper=select(1.0-0.72*along,0.4+0.6*along,s.params.x<1.5);
        let cross_section=max(0.0,1.0-uv.x*uv.x/(taper*taper));
        let shape=cross_section*cross_section*sin(along*3.14159265);
        alpha=shape*s.colour_alpha.a*soften;
        if s.params.x<1.5 {
            let distortion=vec2<f32>(sin(uv.y*11.0+seed-time*6.0),cos(uv.x*8.0+seed*2.1+time*4.0))*(0.4+0.6*swirl)*s.params.w*exhaust.size.zw;
            var warped=in.position.xy*exhaust.size.zw+distortion;
            // Never pull a nearer car/guardrail into gas behind it.
            if exhaust_depth_metres(depth_uv+distortion)<depth_clip.w {warped=in.position.xy*exhaust.size.zw;}
            color=textureSampleLevel(exhaust_background,exhaust_sampler,clamp(warped,exhaust.size.zw*0.5,1.0-exhaust.size.zw*0.5),0.0).rgb;
        } else {
            alpha*=0.8+0.2*swirl;
            let core=pow(max(0.0,1.0-abs(uv.x)/taper),4.0)*(1.0-along);
            color=mix(s.colour_alpha.rgb,vec3<f32>(14000.0,12000.0,7000.0),core*0.7);
        }
    }
    if alpha<0.0001 {discard;}
    return vec4<f32>(color*alpha,alpha);
}
