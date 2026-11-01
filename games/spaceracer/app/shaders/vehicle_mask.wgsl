fn game_mask(world:vec3<f32>, instance:u32, material:u32)->bool {
    let b=sr_vehicle(instance);
    // Detached scorched debris remains visible while the new hull is assembled.
    if b==0u || materials[material].layer == -25 {return false;}
    let progress=bitcast<f32>(aux[b+3u]);
    return progress<1.0 && sr_hull(world,b).y>mix(-1.0,2.6,min(1.0,progress*1.4));
}
