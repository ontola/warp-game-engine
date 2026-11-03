// Source plasma, nozzle glow, flare and phosphor wake, in the per-view sprite
// pass. Single axis-facing plumes avoid the intersecting opaque-looking fins.
fn vapor_hash(cell: vec2<i32>) -> f32 {
    let bits=bitcast<vec2<u32>>(cell);
    var h=bits.x*1597334677u ^ bits.y*3812015801u;
    h=(h^(h>>16u))*2246822519u;h=(h^(h>>13u))*3266489917u;
    return f32((h^(h>>16u))>>8u)/16777215.;
}
fn game_sprite(s: GameSprite) -> vec4<f32> {
    let x = s.uv.x * 2.0 - 1.0;
    let t = s.uv.y;
    if s.effect == 15u { // local 3D exhaust volume: a full, tapering jet stream
        var axis=vec3<f32>(s.params.xy,1.-abs(s.params.x)-abs(s.params.y));
        if axis.z<0. {axis=vec3<f32>((1.-abs(axis.yx))*select(vec2<f32>(-1.),vec2<f32>(1.),axis.xy>=vec2<f32>(0.)),axis.z);}
        axis=normalize(axis);
        let full_length=s.params.z;let time=s.params.w;
        let bound=s.size.x*.5/1.12;
        let radius=sqrt(max(.01,bound*bound-full_length*full_length*.25));
        let root=s.center-axis*full_length*.5;
        // Independent rapid changes, smoothly joined at 42 Hz. The root stays
        // fixed; downstream gas bends and pulses instead of being a rigid cone.
        let beat=time*42.;let tick=i32(floor(beat));let f=fract(beat);let blend=f*f*(3.-2.*f);
        let jitter=mix(vec2<f32>(vapor_hash(vec2<i32>(tick,71)),vapor_hash(vec2<i32>(tick,97))),
            vec2<f32>(vapor_hash(vec2<i32>(tick+1,71)),vapor_hash(vec2<i32>(tick+1,97))),blend)*2.-1.;
        let plume_length=full_length*(.92+.08*jitter.x);
        let helper=select(vec3<f32>(0.,1.,0.),vec3<f32>(1.,0.,0.),abs(axis.y)>.9);
        let side=normalize(cross(axis,helper));let up=cross(side,axis);
        let ray=normalize(s.world-u.camera_near.xyz);
        let oc=u.camera_near.xyz-s.center;
        let b=dot(oc,ray);let disc=b*b-dot(oc,oc)+bound*bound;
        if disc<=0. {return vec4<f32>(0.);}
        let entry=max(.05,-b-sqrt(disc));
        let clip=u.depth_vp*vec4<f32>(s.world,1.);
        if clip.w<=0. {return vec4<f32>(0.);}
        let uv=clip.xy/clip.w*vec2<f32>(.5,-.5)+.5;
        let forward=clip.w/max(.001,distance(s.world,u.camera_near.xyz));
        let end=min(-b+sqrt(disc),depth_metres(uv)/max(.01,forward));
        if end<=entry {return vec4<f32>(0.);}
        let step=(end-entry)/16.;var optical=0.;var energy=0.;
        for(var i=0u;i<16u;i++) {
            let local=u.camera_near.xyz+ray*(entry+(f32(i)+.5)*step)-root;
            let along=dot(local,axis);let z=along/plume_length;
            if z<0. || z>1. {continue;}
            let bend=(side*(jitter.x+sin(z*17.-time*65.)*.5)+up*(jitter.y+cos(z*13.-time*79.)*.5))*radius*z*(1.-z)*1.2;
            let radial=local-axis*along-bend;
            let width=radius*(1.-z)*(.88+.12*sin(z*28.-time*90.+jitter.x*3.));
            let r=length(radial)/max(.01,width);
            let flow=.85+.15*sin(along*8.-time*80.+dot(radial,vec3<f32>(3.,5.,7.)));
            let density=exp(-r*r*2.)*(1.-smoothstep(.65,1.,r))*smoothstep(0.,.035,z)*flow;
            let diamond=pow(.5+.5*cos(z*32.-time*4.),5.)*exp(-r*r*9.);
            optical+=density*step;
            energy+=density*step*(.28+exp(-z*9.)*2.2+diamond*.3*exp(-z*3.));
        }
        return vec4<f32>(vec3<f32>(energy/max(.001,optical)),1.-exp(-optical*1.6));
    }
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
        let strength=mix(s.params.z,s.params.w,t);
        return vec4<f32>(vec3<f32>(glow*.85+core*4.0)+bleed*.32,pow(max(0.0,1.0-age),2.2)*near*(1.0-smoothstep(.7,1.0,abs(x)))*strength);
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
        // Overlapping smooth billows, not thresholded lattice noise: the latter
        // made conspicuous angular channels when the cone filled the view.
        let key=i32(seed*4096.);var cloud=0.;
        for(var k=0;k<6;k++) {
            let centre=(vec2<f32>(vapor_hash(vec2<i32>(key,k*3)),vapor_hash(vec2<i32>(key,k*3+1)))*2.-1.)*.5;
            let radius=.22+.23*vapor_hash(vec2<i32>(key,k*3+2));
            let d=q-centre;
            cloud+=exp(-dot(d,d)/(radius*radius))*.55;
        }
        let density=(1.-exp(-cloud))*(1.-smoothstep(.55,1.,length(q)));
        let clip=u.depth_vp*vec4<f32>(s.world,1.);
        let uv=clip.xy/clip.w*vec2<f32>(.5,-.5)+.5;
        let soft=clamp((depth_metres(uv)-clip.w)/1.2,0.,1.);
        let near=smoothstep(3.,10.,distance(u.camera_near.xyz,s.world));
        return vec4<f32>(vec3<f32>(.85+.15*cloud),density*near*soft);
    }
    if s.effect == 16u { // expanding railgun induction ring
        let r=length(s.uv*2.-1.);
        return vec4<f32>(vec3<f32>(1.),exp(-pow((r-.72)*24.,2.))*.7);
    }
    if s.effect == 17u { // transparent blue shoulder around the white rail core
        return vec4<f32>(vec3<f32>(1.),exp(-x*x*6.)*(1.-smoothstep(.6,1.,abs(x)))*.3);
    }
    return vec4<f32>(0.0);
}
