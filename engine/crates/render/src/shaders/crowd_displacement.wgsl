const FLEE_OUT_END: f32 = 1.2;
const FLEE_RETURN_DUR: f32 = 4.5;

// One persistent movement and death record per spectator, shared by all cameras.
struct Reaction {
    start: f32, hold_end: f32, initial: f32, goal: f32,
    dead: u32, bullet_lo: u32, bullet_hi: u32, pad: u32,
}
fn displacement(r:Reaction,time:f32)->f32 {
    let outward=smoothstep(0.,FLEE_OUT_END,time-r.start);
    let home=smoothstep(0.,FLEE_RETURN_DUR,time-r.hold_end);
    return mix(r.initial,r.goal,outward)*(1.-home);
}
