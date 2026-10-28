// The reference fades its six-segment shape towards a one-segment shape
// over half the LOD distance. Here the target is the actual ANIMATED coarse
// triangle, so position and affine shading normals coincide at the mesh cut.
fn meadow_lod_factor(root:vec3<f32>,scale:f32,camera:vec3<f32>)->f32 {
    return smoothstep(3.0,6.0,distance(root,camera)/max(scale,0.01));
}
fn meadow_coarse_weights(vertex:u32,vertices_per_blade:u32,t:f32)->vec3<f32> {
    let local=vertex%vertices_per_blade;
    let corner=local%6u;
    let right=select(0.0,1.0,corner==1u || corner==4u || corner==5u);
    return vec3<f32>((1.0-t)*(1.0-right),(1.0-t)*right,t);
}
struct MeadowMorphPose {world:vec3<f32>,previous_world:vec3<f32>,normal:vec3<f32>};
fn meadow_morph_pose(fine:MeadowDeformed,left:MeadowDeformed,right:MeadowDeformed,tip:MeadowDeformed,
    weights:vec3<f32>,factors:vec2<f32>)->MeadowMorphPose {
    var out:MeadowMorphPose;
    let coarse=left.world*weights.x+right.world*weights.y+tip.world*weights.z;
    let previous=left.previous_world*weights.x+right.previous_world*weights.y+tip.previous_world*weights.z;
    out.world=mix(fine.world,coarse,factors.x);
    out.previous_world=mix(fine.previous_world,previous,factors.y);
    // Normalization belongs AFTER fragment interpolation. Normalizing at
    // every fine vertex would change the field when the tessellation changes.
    out.normal=mix(fine.normal,left.normal*weights.x+right.normal*weights.y+tip.normal*weights.z,factors.x);
    return out;
}
