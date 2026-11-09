const FLEE_OUT_END: f32 = 1.2;
const FLEE_RETURN_DUR: f32 = 4.5;

// One persistent movement and death record per spectator, shared by all cameras.
struct Reaction {
    start: f32, hold_end: f32, initial: f32, goal: f32,
    dead: u32, bullet_lo: u32, bullet_hi: u32,
    turn_start: f32, turn_from: f32, turn_goal: f32,
}
// Signed angles follow the rig's local XZ rotation (+Z towards -X).
fn blast_turn_at(r:Reaction,time:f32)->f32 {
    let age=time-r.turn_start;
    if age>=2.5 || (r.turn_goal==0. && r.turn_from==0.) {return 0.;}
    let reach=0.23+0.15*abs(r.turn_goal-r.turn_from);
    return mix(r.turn_from,r.turn_goal,smoothstep(0.,reach,age))
        *(1.-smoothstep(reach+0.48,reach+1.35,age));
}
fn turn_from_blast(old:Reaction,time:f32,away:vec2<f32>,yaw:f32,seed:f32,proximity:f32)->Reaction {
    var r=old;
    let forward=vec2<f32>(sin(yaw),cos(yaw));
    let front=dot(away,forward);
    let side=dot(away,vec2<f32>(forward.y,-forward.x));
    let current=blast_turn_at(r,time);
    let limit=mix(1.82,2.35,seed);
    var target_angle=0.;
    if length(away)<0.001 || (front<0. && abs(side)< -0.10*front) {
        // Head-on blast: keep the shoulder already leading on a second blast.
        let preferred=select(-1.,1.,seed>=0.5);
        target_angle=select(preferred,sign(current),abs(current)>0.1)*limit;
    } else {
        target_angle=clamp(atan2(-side,front),-limit,limit);
    }
    r.turn_from=current;
    r.turn_start=time;
    r.turn_goal=target_angle*smoothstep(0.,0.45,proximity);
    return r;
}
fn displacement(r:Reaction,time:f32)->f32 {
    let outward=smoothstep(0.,FLEE_OUT_END,time-r.start);
    let home=smoothstep(0.,FLEE_RETURN_DUR,time-r.hold_end);
    return mix(r.initial,r.goal,outward)*(1.-home);
}
