// Source plasma, nozzle glow, flare and phosphor wake, in the per-view sprite
// pass. Single axis-facing plumes avoid the intersecting opaque-looking fins.
fn vapor_noise(p: vec2<f32>) -> f32 {
    let cell=floor(p);let f=fract(p);let u=f*f*(3.-2.*f);
    let a=fract(sin(dot(cell,vec2<f32>(127.1,311.7)))*43758.5453);
    let b=fract(sin(dot(cell+vec2<f32>(1.,0.),vec2<f32>(127.1,311.7)))*43758.5453);
    let c=fract(sin(dot(cell+vec2<f32>(0.,1.),vec2<f32>(127.1,311.7)))*43758.5453);
    let d=fract(sin(dot(cell+vec2<f32>(1.,1.),vec2<f32>(127.1,311.7)))*43758.5453);
    return mix(mix(a,b,u.x),mix(c,d,u.x),u.y);
}
fn game_sprite(s: GameSprite) -> vec4<f32> {
    let x = s.uv.x * 2.0 - 1.0;
    let t = s.uv.y;
    if s.effect == 1u { // plasma: parameters time, boost, reverse, unused
        let boost = s.params.y;
        let time = s.params.x;
        let bend = sin(t*19.0-time*31.0)*t*t*.13*boost;
        let width = mix(.60, 1.0, smoothstep(0.0,.12,t)) * (1.0-smoothstep(.22,1.04,t));
        let across = abs(x+bend) / max(width,.015);
        let core = exp(-across*across*16.0);
        let edge = 1.0-smoothstep(.38,1.0,across);
        let tip = 1.0-smoothstep(.50,1.0,t);
        let flow = .83+.17*sin(t*43.0-time*65.0);
        let diamonds = pow(.5+.5*cos(t*30.0-time*8.0),5.0);
        // Radiance is blue; AgX rolls the intense nozzle end toward white.
        // Most energy lives in the first metre, not the length of the plume.
        let root=exp(-t*12.0);
        let falloff=exp(-t*5.5);
        let forward=vec3<f32>(.35+root*6.0+core*diamonds*root);
        let reverse=vec3<f32>(1.6,2.0,2.6)*(1.0+core*diamonds*.5);
        let color=select(forward,reverse,s.params.z>.5);
        let energy=select(falloff,1.0,s.params.z>.5);
        return vec4<f32>(color,edge*tip*flow*(.10+core*.58)*energy);
    }
    if s.effect == 2u { // compact bright nozzle and broad low-energy glow
        let p = s.uv*2.0-1.0;
        let r2 = dot(p,p);
        let halo = exp(-r2*7.0)*(1.0-smoothstep(.65,1.0,sqrt(r2)));
        let core = exp(-r2*70.0);
        return vec4<f32>(mix(vec3<f32>(1.0),vec3<f32>(2.0,2.4,3.0),core),halo*.08+core*.9);
    }
    if s.effect == 3u { // subtle anamorphic flare, visible only if source is visible
        let source = s.params.xyz;
        let clip = u.depth_vp*vec4<f32>(source,1.0);
        if clip.w <= 0.0 { return vec4<f32>(0.0); }
        let uv = clip.xy/clip.w*vec2<f32>(.5,-.5)+.5;
        let visible = smoothstep(-.15,.15,depth_metres(uv)-clip.w);
        let y = (t*2.0-1.0)*.3;
        let streak = exp(-abs(x)*4.0)*exp(-y*y*11000.0);
        let glow = exp(-(x*x+y*y)*180.0);
        return vec4<f32>(1.0,1.0,1.0,(streak*.32+glow*.14)*(1.0-smoothstep(.72,1.0,abs(x)))*visible);
    }
    if s.effect == 4u { // short, soft exhaust wisps, never square confetti
        let bend = sin(t*9.0+s.params.x)*.15*sin(t*3.14159);
        let edge = exp(-(x+bend)*(x+bend)*5.0)*(1.0-smoothstep(.6,1.0,abs(x)));
        let ends = smoothstep(0.0,.22,t)*(1.0-smoothstep(.62,1.0,t));
        let near = smoothstep(3.0,9.0,distance(u.camera_near.xyz,s.world));
        return vec4<f32>(1.0,1.0,1.0,edge*ends*near*.16);
    }
    if s.effect == 5u { // thin one-frame electrical discharge
        let core = exp(-x*x*32.0);
        let near=smoothstep(1.5,4.,distance(u.camera_near.xyz,s.world));
        return vec4<f32>(mix(vec3<f32>(1.0),vec3<f32>(2.0,2.5,3.0),core),exp(-x*x*8.)*(1.-smoothstep(.65,1.,abs(x)))*near);
    }
    if s.effect == 6u { // analog colour shoulders around the boost wake
        let age = mix(s.params.x,s.params.y,t);
        let core = exp(-x*x*130.0);
        let glow = exp(-x*x*12.0);
        let bleed = vec3<f32>(1.0,.08,.16)*exp(-pow(x-.26,2.0)*50.0)
                  + vec3<f32>(.03,.65,1.0)*exp(-pow(x+.26,2.0)*50.0);
        let near = smoothstep(1.0,5.0,distance(u.camera_near.xyz,s.world));
        return vec4<f32>(vec3<f32>(glow*.85+core*4.0)+bleed*.32,pow(max(0.0,1.0-age),2.2)*near*(1.0-smoothstep(.7,1.0,abs(x))));
    }
    if s.effect == 7u { // continuous low-opacity exhaust ribbon; fades when flown through
        let age=mix(s.params.x,s.params.y,t);
        let world=s.world*.18;
        let turbulence=.65+.18*sin(world.x+world.y*1.7+s.params.z*.5)+.12*sin(world.z*1.3-world.y);
        let across=exp(-x*x*5.)*(1.-smoothstep(.65,1.,abs(x)));
        let near=smoothstep(4.,15.,distance(u.camera_near.xyz,s.world));
        return vec4<f32>(vec3<f32>(.75+.25*turbulence),across*pow(1.-age,1.7)*near*turbulence);
    }
    if s.effect == 8u {
        let crossfade=pow(max(0.,1.-abs(x)),2.8);let ends=smoothstep(0.,.1,t)*pow(1.-t,1.6);
        let pulse=pow(.5+.5*sin(t*15.-s.params.x*11.+s.params.y*2.3),5.);
        return vec4<f32>(vec3<f32>(1.),ends*crossfade*(.1+pulse*.55));
    }
    if s.effect == 9u {
        let q=s.uv*2.-1.;let core=exp(-dot(q,q)*15.);let halo=exp(-dot(q,q)*4.);
        return vec4<f32>(mix(vec3<f32>(1.,.65,.25),vec3<f32>(2.),core),halo*.6+core*.4);
    }
    if s.effect == 10u {
        let q=s.uv*2.-1.;let r=length(q);
        let cloud=.8+.12*sin(q.x*11.+s.params.x)+.08*sin(q.y*15.-q.x*7.+s.params.x*2.);
        return vec4<f32>(vec3<f32>(1.),exp(-r*r*3.)*(1.-smoothstep(.65,1.,r))*cloud);
    }
    if s.effect == 11u { // analytic, unbroken tracer / hot impact streak
        let edge=1.-smoothstep(.55,1.,abs(x));
        return vec4<f32>(mix(vec3<f32>(1.),vec3<f32>(1.5,1.45,1.3),1.-abs(x)),edge);
    }
    if s.effect == 13u { // screen-sized optical bloom on distant engines
        let clip=u.depth_vp*vec4<f32>(s.params.xyz,1.);
        if clip.w<=0. {return vec4<f32>(0.);}
        let uv=clip.xy/clip.w*vec2<f32>(.5,-.5)+.5;
        let visible=smoothstep(-.12,.12,depth_metres(uv)-clip.w);
        let q=s.uv*2.-1.;let r2=dot(q,q);
        let halo=exp(-r2*9.)*(1.-smoothstep(.65,1.,sqrt(r2)));
        let core=exp(-r2*65.);
        let far=1.-smoothstep(650.,1200.,clip.w);
        return vec4<f32>(mix(vec3<f32>(.65,.85,1.),vec3<f32>(2.5),core),
            (halo*.08+core*.9)*visible*far);
    }
    if s.effect == 12u { // inexpensive road-contact pool, no opaque disk
        let q=s.uv*2.-1.;let r2=dot(q,q);
        return vec4<f32>(vec3<f32>(1.),exp(-r2*5.)*(1.-smoothstep(.6,1.,sqrt(r2)))*.45);
    }
    if s.effect == 14u { // hollow sonic condensation: turbulent, broken smoke patches
        let q=s.uv*2.-1.;let seed=s.params.x;
        let flow=vec2<f32>(sin(q.y*6.+seed),cos(q.x*7.-seed))*.16;
        let p=q+flow;
        let cloud=.65*vapor_noise(p*3.+vec2<f32>(seed))+.35*vapor_noise(p*7.-vec2<f32>(seed));
        let density=smoothstep(.2,.7,cloud)*exp(-dot(p,p)*2.)*(1.-smoothstep(.55,1.,length(q)));
        let clip=u.depth_vp*vec4<f32>(s.world,1.);
        let uv=clip.xy/clip.w*vec2<f32>(.5,-.5)+.5;
        let soft=clamp((depth_metres(uv)-clip.w)/1.2,0.,1.);
        let near=smoothstep(3.,10.,distance(u.camera_near.xyz,s.world));
        return vec4<f32>(vec3<f32>(.85+.15*cloud),density*near*soft);
    }
    return vec4<f32>(0.0);
}
