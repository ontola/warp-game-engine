// Small world-aligned cache, independent of source frustum/Hi-Z/LOD selection.
// Three rows of controls: block/dimension/radius/reach; source area; strengths/fade.
@group(2) @binding(0) var<storage,read_write> meadow_contact_output:array<vec4<f32>>;
@group(2) @binding(1) var<uniform> meadow_contact_controls:array<vec4<f32>,3>;

@compute @workgroup_size(8,8)
fn generate_meadow_contact(@builtin(global_invocation_id) id:vec3<u32>) {
    let config=meadow_contact_controls[0];let dimension=u32(config.y);
    if id.x>=dimension || id.y>=dimension {return;}
    let stride=4u+dimension*dimension;let block=u32(config.x)*stride;
    if block+stride>arrayLength(&meadow_contact_output) {return;}
    let origin=vec2<i32>(floor(view.cam.xz/MEADOW_ROOT_CELL))-vec2<i32>(i32(dimension/2u));
    if all(id.xy==vec2<u32>(0u)) {
        meadow_contact_output[block]=vec4<f32>(bitcast<vec2<f32>>(origin),f32(dimension),1.0);
        meadow_contact_output[block+1u]=vec4<f32>(config.z,meadow_contact_controls[2].xy,config.w);
        meadow_contact_output[block+2u]=vec4<f32>(meadow_contact_controls[2].zw,meadow.shape.w,meadow.range.x);
        meadow_contact_output[block+3u]=vec4<f32>(view.cam.xyz,0.0);
    }
    let output=block+4u+id.y*dimension+id.x;
    meadow_contact_output[output]=vec4<f32>(0.0);
    let cell=origin+vec2<i32>(id.xy);
    let candidate=meadow_generate_cell(cell,true);
    if !candidate.valid {return;}
    let plant=candidate.plant;
    let distance_to_view=distance(plant.root_scale.xz,view.cam.xz);
    if distance_to_view>=meadow.range.x {return;}
    var fraction=1.0;var centre=vec3<f32>(0.0);
    if plant.road_variant_tint.x!=0u {
        // Every coarse triangle represents one authored blade. All source
        // LODs apply this same anchor decision, never a frame-random alpha.
        let mesh=bitcast<vec4<u32>>(meadow_recipe[plant.road_variant_tint.z*3u+2u]);
        var accepted=0.0;
        for(var vertex=0u;vertex<mesh.y;vertex+=3u) {
            let address=MEADOW_RECIPE_VERTICES+(mesh.x+vertex)*4u;
            if address+3u>=arrayLength(&meadow_recipe) {continue;}
            let anchor=meadow_recipe[address+3u];
            let offset=meadow_yaw(anchor.xyz,plant.yaw.xy);
            if meadow_road_anchor_allowed(plant.road_variant_tint.x,bitcast<f32>(plant.road_variant_tint.y),
                offset.xz,plant.root_scale.xz+offset.xz,meadow.grid.z) {
                accepted+=1.0;centre+=offset;
            }
        }
        if accepted<=0.0 {return;}
        fraction=accepted/max(f32(mesh.y/3u),1.0);centre/=accepted;
    }
    let area=meadow_contact_controls[1][min(plant.road_variant_tint.z,2u)]*plant.root_scale.w*plant.root_scale.w;
    let mass=(1.0-exp(-area))*fraction;
    let fade=1.0-smoothstep(meadow.shape.w,meadow.range.x,distance_to_view);
    // The fixed base remains stable under wind. Centroid terrain shear is the
    // same as the source's; receiver-side height gating prevents raised decks
    // from borrowing this ground contact.
    let world=plant.root_scale.xyz+meadow_ground_shear(centre,plant.yaw.zw);
    meadow_contact_output[output]=vec4<f32>(world,mass*fade);
}
