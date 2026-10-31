// Source plasma, nozzle glow, flare and phosphor wake, in the per-view sprite
// pass. Single axis-facing plumes avoid the intersecting opaque-looking fins.
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
        let hot = vec3<f32>(1.6,2.0,2.6);
        let color = mix(vec3<f32>(.6,.8,1.0),hot,core*.6)+hot*core*diamonds*.5;
        return vec4<f32>(color,edge*tip*flow*(.10+core*.58));
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
        return vec4<f32>(mix(vec3<f32>(1.0),vec3<f32>(2.0,2.5,3.0),core),(1.0-smoothstep(.25,1.0,abs(x)))*.65);
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
    return vec4<f32>(0.0);
}
