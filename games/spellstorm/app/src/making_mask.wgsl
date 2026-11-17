fn game_mask(world: vec3<f32>, instance: u32, material: u32) -> bool {
    let b = mk_record(instance);
    if b == 0u { return false; }
    return mk_depth(world, b) < 0.0;
}
