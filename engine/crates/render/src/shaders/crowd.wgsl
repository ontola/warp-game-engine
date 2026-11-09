fn write(index: u32, x: vec3<f32>, y: vec3<f32>, z: vec3<f32>, t: vec3<f32>, first: u32, count: u32, radius: f32, scale: f32) {
    var inst = instances[index];
    inst.r0 = vec4<f32>(x.x, y.x, z.x, t.x);
    inst.r1 = vec4<f32>(x.y, y.y, z.y, t.y);
    inst.r2 = vec4<f32>(x.z, y.z, z.z, t.z);
    inst.sphere = vec4<f32>(t + y * 0.5, radius);
    inst.first_cluster = first;
    inst.cluster_count = select(count, 0u, u.counts.z != 0u);
    inst.scale = scale;
    instances[index] = inst;
}

@compute @workgroup_size(64)
fn animate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= u.counts.x { return; }
    let s = seats[i];
    let body_index = u.counts.y + i * 2u;
    if alive[i] == 0u || reactions[i].dead!=0u {
        var a = instances[body_index]; a.cluster_count = 0u; a.sphere = vec4<f32>(0.0); instances[body_index] = a;
        var b = instances[body_index + 1u]; b.cluster_count = 0u; b.sphere = vec4<f32>(0.0); instances[body_index + 1u] = b;
        return;
    }
    let pose=body_pose(s,reactions[i],u.mood.x,u.mood.yz);
    let up=pose.up;
    let panic=u.mood.z;
    let scale=s.params.x;
    // Local frame: yaw about Y, scaled by height, narrowed by the shape; figure +Z faces the arena.
    var yaw = s.pos_yaw.w;
    if u.counts.z == 0u { yaw -= blast_turn_at(reactions[i], u.mood.x); }
    let c = cos(yaw);
    let sn = sin(yaw);
    let ex = vec3<f32>(c, 0.0, -sn) * scale * s.shape.x;
    let ey = vec3<f32>(0.0, 1.0, 0.0) * scale;
    let ez = vec3<f32>(sn, 0.0, c) * scale * s.shape.x;
    let origin = pose.origin;
    // Body: crouch squashes Y, shear leans X with height.
    let by = (ey + ex * pose.shear) * pose.crouch;
    write(body_index, ex, by, ez, origin, s.meshes.x, s.meshes.y, scale * 0.9, scale);
    // Arms: raised with the cheer, spread outward (they sit at x = +-0.21).
    let arm = 1.0 - panic;
    if u.mood.w > 0.0 {
        let angle = -2.4 * arm * up;
        let ay = by * cos(angle) + ez * sin(angle);
        let az = ez * cos(angle) - by * sin(angle);
        let pivot = u.mood.w;
        write(body_index + 1u, ex, ay, az, origin + (by - ay) * pivot,
              s.meshes.z, s.meshes.w, scale * 0.9, scale);
        return;
    }
    let spread = 1.0 + arm * up * 0.12 / 0.21;
    let arms_origin = origin + by * 0.0 + ey * (arm * up * 0.34);
    write(body_index + 1u, ex * spread, by, ez, arms_origin, s.meshes.z, s.meshes.w, scale * 0.9, scale * spread);
}
