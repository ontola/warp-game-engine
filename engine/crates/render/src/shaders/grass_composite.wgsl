@group(0) @binding(0) var nearest_layer: texture_2d<f32>;
@group(0) @binding(1) var far_layer: texture_2d<f32>;
@group(0) @binding(2) var far_root_distance: texture_2d<f32>;
struct GrassBlend { values: vec4<f32> };
@group(0) @binding(3) var<uniform> grass_blend: GrassBlend;
@vertex
fn vs(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[vertex],0.0,1.0);
}
@fragment
fn fs(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let pixel=vec2<i32>(position.xy);
    let near = textureLoad(nearest_layer,pixel,0);
    let far_size = textureDimensions(far_layer);
    // Nearest half-resolution reconstruction retains the conservative 2x2
    // opaque-depth bound. Bilinear interpolation could leak behind cars.
    let far_pixel = min(vec2<u32>(position.xy) / 2u, far_size - 1u);
    let distant = textureLoad(far_layer,vec2<i32>(far_pixel),0);
    if distant.a <= 0.0 { return near; }
    let root_distance = textureLoad(far_root_distance,vec2<i32>(far_pixel),0).r;
    var weight = grass_distance_opacity(root_distance,grass_blend.values.x>0.5);
    // Covered near pixels already carry their exact plant-root fade. Holes
    // need the geometric fade too: using (1-near.a) there would fill the
    // photograph's silhouette even at full near detail. Far root distance
    // is a density-weighted representative, not the same individual plant.
    if near.a>0.0 {weight=near.a;}
    // Blend representations first, then composite once over opaque colour.
    return near + distant*(1.0-weight);
}
