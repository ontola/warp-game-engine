const VIRTUAL_MEMBER: u32 = 0x80000000u;
fn member_slot(id: u32) -> u32 { return select(id, (id >> 12u) & 0x7ffffu, (id & VIRTUAL_MEMBER) != 0u); }
fn compose_member(owner: Instance, t: Instance) -> Instance {
    var m = t;
    m.r0 = owner.r0.x * t.r0 + owner.r0.y * t.r1 + owner.r0.z * t.r2 + vec4<f32>(0.0, 0.0, 0.0, owner.r0.w);
    m.r1 = owner.r1.x * t.r0 + owner.r1.y * t.r1 + owner.r1.z * t.r2 + vec4<f32>(0.0, 0.0, 0.0, owner.r1.w);
    m.r2 = owner.r2.x * t.r0 + owner.r2.y * t.r1 + owner.r2.z * t.r2 + vec4<f32>(0.0, 0.0, 0.0, owner.r2.w);
    let c = vec4<f32>(t.sphere.xyz, 1.0);
    m.sphere = vec4<f32>(dot(owner.r0, c), dot(owner.r1, c), dot(owner.r2, c), -t.sphere.w * owner.scale);
    m.scale = t.scale * owner.scale;
    // The owner's history epoch (bit 7) marks a woken pose for motion.
    m.flags = (t.flags & ~0x80u) | (owner.flags & 0x80u);
    return m;
}
// Shared address ABI. Keep these functions identical for cull, raster,
// material resolve and both frames of motion reprojection.
fn member_owner(id:u32)->u32 { return (id>>12u)&0x7ffffu; }
fn member_template(owner:Instance,id:u32)->u32 { return (owner.first_cluster>>12u)+(id&0xfffu); }
fn member_id(owner:u32,member:u32)->u32 { return VIRTUAL_MEMBER|(owner<<12u)|member; }
