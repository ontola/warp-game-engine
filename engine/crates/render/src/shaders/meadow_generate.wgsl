// Composed after meadow_common.wgsl; exactly one invocation per world-cell
// candidate. A fixed root pool plus three compact index lists bounds memory.
struct View {
    view_proj:mat4x4<f32>,inv_view_proj:mat4x4<f32>,cam:vec4<f32>,
    planes:array<vec4<f32>,6>,params:vec4<f32>,size:vec4<f32>,extra:vec4<f32>,
    grass:vec4<f32>,fog_cull:vec4<f32>,
};
// Same20-byte record for indexed and reference draws. The three trailing
// words remain zero: first_index/base_vertex/first_instance (or the first two
// are first_vertex/first_instance for draw_indirect's16-byte prefix).
struct MeadowDrawArgs {vertex_count:u32,instance_count:atomic<u32>,first_index:u32,base_vertex:i32,first_instance:u32};
@group(0) @binding(0) var<uniform> meadow:MeadowSettings;
@group(0) @binding(1) var<storage,read> meadow_ground:array<vec4<f32>>;
@group(0) @binding(2) var<storage,read_write> meadow_pool:array<u32>;
@group(0) @binding(3) var<storage,read_write> meadow_args:array<MeadowDrawArgs,3>;
@group(0) @binding(4) var<storage,read> meadow_recipe:array<vec4<f32>>;
@group(1) @binding(0) var<uniform> view:View;
// Current-frame opaque depth, MIN-reduced in reversed-Z. Zero sky/holes must
// remain zero at every ancestor. This is not last-frame occlusion history.
@group(1) @binding(1) var meadow_hiz:texture_2d<f32>;

// A small, permanent offset breaks the grid alignment without allowing roots
// to exchange cells or bunch arbitrarily. At 0.45 m spacing this is at most
// 1.8 cm per axis (2.55 cm radially); adjacent centres remain >=0.414 m apart.
// Both source and aggregate dispatch this same function for the same cell.
const MEADOW_GROWTH_BROAD_METRES:f32=9.0;
const MEADOW_GROWTH_LOCAL_METRES:f32=3.0;
const MEADOW_GROWTH_BROAD_WEIGHT:f32=0.75;
const MEADOW_GROWTH_LOCAL_WEIGHT:f32=0.20;
const MEADOW_GROWTH_ROOT_WEIGHT:f32=0.05;
fn meadow_growth_factor(xz:vec2<f32>,cell:vec2<i32>,seed:u32)->f32 {
    // Most stature follows a smooth world field, rather than jumping by up to
    // 60 percent between adjoining patches. The remaining per-root difference
    // is at most 3 percent. Neither camera, distance nor wind changes stature.
    let broad=meadow_noise(xz/MEADOW_GROWTH_BROAD_METRES+vec2<f32>(-26.3,17.1),seed);
    let local=meadow_noise(xz/MEADOW_GROWTH_LOCAL_METRES+vec2<f32>(5.7,-12.9),seed);
    let vigor=broad*MEADOW_GROWTH_BROAD_WEIGHT+local*MEADOW_GROWTH_LOCAL_WEIGHT
        +meadow_random(cell,9u,seed)*MEADOW_GROWTH_ROOT_WEIGHT;
    return clamp(0.7+0.6*vigor,0.7,1.3);
}

struct MeadowGround {height:f32,road:f32,up:f32,valid:bool,slope:vec2<f32>,road_gradient:vec2<f32>,road_plane_safe:bool};
fn meadow_ground_at(xz:vec2<f32>)->MeadowGround {
    var result:MeadowGround;
    if meadow.grid.x<2u || meadow.grid.y<2u || meadow.field.z<=0.0 {return result;}
    let q=(xz-meadow.field.xy)/meadow.field.z;
    if any(q<vec2<f32>(0.0)) || any(q>vec2<f32>(meadow.grid.xy-vec2<u32>(1u))) {return result;}
    let cell=min(vec2<u32>(floor(q)),meadow.grid.xy-vec2<u32>(2u));let f=q-vec2<f32>(cell);
    let a=cell.y*meadow.grid.x+cell.x;let d=a+meadow.grid.x+1u;
    if d>=arrayLength(&meadow_ground) {return result;}
    var aa=meadow_ground[a];var bb=meadow_ground[a+1u];var cc=meadow_ground[a+meadow.grid.x];
    var weights=vec3<f32>(1.0-f.x-f.y,f.x,f.y);
    var slope=vec2<f32>(bb.x-aa.x,cc.x-aa.x)/meadow.field.z;
    var road_gradient=vec2<f32>(bb.y-aa.y,cc.y-aa.y)/meadow.field.z;
    if f.x+f.y>1.0 {
        aa=meadow_ground[d];bb=meadow_ground[a+meadow.grid.x];cc=meadow_ground[a+1u];
        weights=vec3<f32>(f.x+f.y-1.0,1.0-f.x,1.0-f.y);
        slope=vec2<f32>(aa.x-bb.x,aa.x-cc.x)/meadow.field.z;
        road_gradient=vec2<f32>(aa.y-bb.y,aa.y-cc.y)/meadow.field.z;
    }
    if aa.z<0.5 || bb.z<0.5 || cc.z<0.5 {return result;}
    // Invalid/nonfinite source samples are not a supported ground surface.
    if !all(abs(vec3<f32>(aa.x,bb.x,cc.x))<vec3<f32>(1e20)) ||
       !all(abs(vec3<f32>(aa.y,bb.y,cc.y))<vec3<f32>(1e20)) {return result;}
    result.height=dot(vec3<f32>(aa.x,bb.x,cc.x),weights);
    result.road=dot(vec3<f32>(aa.y,bb.y,cc.y),weights);
    result.road_gradient=road_gradient;
    result.road_plane_safe=meadow_road_plane_safe(f,meadow.field.z,meadow.support.x);
    result.up=inverseSqrt(1.0+dot(slope,slope));result.slope=slope;result.valid=true;return result;
}
fn meadow_write4(offset:u32,value:vec4<f32>) {
    let words=bitcast<vec4<u32>>(value);
    for(var k=0u;k<4u;k++){meadow_pool[offset+k]=words[k];}
}
fn meadow_in_frustum(root:vec3<f32>,radius:f32)->bool {
    for(var plane=0u;plane<6u;plane++) {
        if dot(view.planes[plane].xyz,root)+view.planes[plane].w < -radius {return false;}
    }
    return true;
}
fn meadow_occluded(root:vec3<f32>,radius:f32)->bool {
    if meadow.distances.w<0.5 {return false;}
    let size=textureDimensions(meadow_hiz,0);
    if any(size==vec2<u32>(0u)) {return false;}
    var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var nearest=0.0;
    // The root-centred AABB encloses the entire source plus its maximum wind
    // envelope. Positive W makes projected extrema occur at its eight corners.
    // A near-plane crossing or any invalid projection is conservatively kept.
    for(var corner=0u;corner<8u;corner++) {
        let sign3=vec3<f32>(select(-1.0,1.0,(corner&1u)!=0u),
            select(-1.0,1.0,(corner&2u)!=0u),select(-1.0,1.0,(corner&4u)!=0u));
        let clip=view.view_proj*vec4<f32>(root+sign3*radius,1.0);
        if !all(abs(clip)<vec4<f32>(1e30)) || clip.w<=1e-5 ||
            clip.z<0.0 || clip.z>=clip.w-1e-6 {return false;}
        let ndc=clip.xyz/clip.w;
        let uv=ndc.xy*vec2<f32>(0.5,-0.5)+0.5;
        lo=min(lo,uv);hi=max(hi,uv);nearest=max(nearest,ndc.z);
    }
    if any(hi<vec2<f32>(0.0)) || any(lo>vec2<f32>(1.0)) {return false;}
    // Include a full base pixel and roundoff allowance beyond both edges.
    // Work in base pixel indices: uv*mipDimensions is NOT equivalent for odd
    // sizes. Reduction folds an odd final source row/column into the last
    // destination texel; integer shifts plus clamp preserve that footprint.
    let last=vec2<f32>(size-vec2<u32>(1u));
    let first_pixel=vec2<u32>(clamp(floor(lo*vec2<f32>(size)-1.001),vec2<f32>(0.0),last));
    let last_pixel=vec2<u32>(clamp(ceil(hi*vec2<f32>(size)+1.001),vec2<f32>(0.0),last));
    let levels=textureNumLevels(meadow_hiz);
    for(var level=0u;level<levels;level++) {
        let dimensions=textureDimensions(meadow_hiz,i32(level));
        let end=dimensions-vec2<u32>(1u);
        let first=min(first_pixel>>vec2<u32>(level),end);
        let last=min(last_pixel>>vec2<u32>(level),end);
        if any(last-first>vec2<u32>(1u)) {continue;}
        // All covered mip texels are tested, including zero-depth openings.
        // A single centre/corner depth tap cannot certify full occlusion.
        let a=textureLoad(meadow_hiz,vec2<i32>(first),i32(level)).r;
        let b=textureLoad(meadow_hiz,vec2<i32>(i32(last.x),i32(first.y)),i32(level)).r;
        let c=textureLoad(meadow_hiz,vec2<i32>(i32(first.x),i32(last.y)),i32(level)).r;
        let d=textureLoad(meadow_hiz,vec2<i32>(last),i32(level)).r;
        let depth=min(min(a,b),min(c,d));
        return depth>nearest+2e-5 && depth<=1.0;
    }
    return false;
}
// Pure root producer shared by visible source/atlas and world-page cooks.
// A page bypasses only camera selection, never habitat/road/seed or poses.
struct MeadowCandidate {plant:MeadowPlant,radius:f32,lod:u32,valid:bool};
// The ordinary renderer retains every eligible root. The optional certified
// group pipeline replaces this pure hook, after the SAME habitat/frustum/Hi-Z
// tests and before wind evaluation or any compact-list/pool writes.
fn meadow_retain_root(cell:vec2<i32>,root:vec3<f32>)->bool {return true;}
// Recipe header 0.z is a world-space species patch scale, not a draw range.
// Pure spatial identity is shared by all views, source LODs and contact. Keep
// the zero mode's existing hash exactly; no per-cell colour speckle in mixed.
fn meadow_source_variant(cell:vec2<i32>,xz:vec2<f32>,seed:u32,count:u32,patch_metres:f32)->u32 {
    let variants=max(1u,min(count,3u));
    if patch_metres<=0.0 {return meadow_hash(cell,67u,seed)%variants;}
    let field=meadow_noise(xz/patch_metres+vec2<f32>(23.4,-9.7),seed^0x7a31d5b9u);
    let variant=select(select(0u,1u,field>=0.38),2u,field>=0.62);
    return min(variant,variants-1u);
}
fn meadow_generate_cell(cell:vec2<i32>,page:bool)->MeadowCandidate {
    var result:MeadowCandidate;
    let seed=meadow.grid.z;
    if !meadow_patch_retained(cell,seed,meadow.edits.w) {return result;}
    // Identity remains the integer world cell. The rounded shared recipe is
    // translated once; all support, road, wind and coverage queries below use
    // this actual root, independent of the camera's pool slot or draw range.
    let xz=(vec2<f32>(cell)+meadow_root_fraction(cell,seed))*meadow.field.w;
    let horizontal=distance(xz,view.cam.xz);
    if !page && (horizontal>=meadow.range.x || (meadow.support.z>1.5 && horizontal<meadow.shape.w)) {return result;}
    let scale=meadow_growth_factor(xz,cell,seed)*meadow.shape.x;
    if scale<=0.0 {return result;}
    let variant=meadow_source_variant(cell,xz,seed,meadow.grid.w,meadow_recipe[0].z);
    var max_angle=0.0;
    for(var branch=0u;branch<4u;branch++) {
        let pivot=meadow_recipe[9u+variant*4u+branch];
        max_angle=max(max_angle,abs(pivot.w));
    }
    let rest_radius=meadow.support.x+meadow.support.y*scale;
    // Each blade bends around its own fixed anchor. Growth and wind never
    // move the lattice; displacement <= root-relative radius * angle.
    // Membership/road clearance is based on the MAX supported strength and
    // angle multiplier (WindState grass_shear<=.6), so wind cannot pop plants.
    // The full root-relative radius already encloses every rotated vertex of
    // the windswept recipe. Its angle varies with height but preserves length.
    let wind_margin=select(meadow.support.y*scale*max_angle*1.5,0.0,meadow.wind_history.w>0.5);
    let clearance=rest_radius+wind_margin;
    let ground=meadow_ground_at(xz);
    if !ground.valid || ground.up<=0.7 {return result;}
    let trim_road=ground.road_plane_safe && dot(ground.road_gradient,ground.road_gradient)>1e-8
        && all(abs(ground.road_gradient)<vec2<f32>(65504.0));
    var packed_road_gradient=0u;var usable_road=0.0;
    if trim_road {
        packed_road_gradient=pack2x16float(ground.road_gradient);
        let decoded=unpack2x16float(packed_road_gradient);
        // The blade envelope excludes the fixed patch anchor radius. Cover
        // every supported wind pose, and bound half-float error over all
        // anchors instead of trusting rounded gradients near the boundary.
        let blade_reach=meadow.support.y*scale*select(1.0+max_angle*1.5,1.0,meadow.wind_history.w>0.5);
        let quantization_guard=length(ground.road_gradient-decoded)*meadow.support.x;
        usable_road=ground.road-0.9-blade_reach*max(1.0,length(ground.road_gradient))-quantization_guard;
        if usable_road+length(decoded)*meadow.support.x<0.0 {return result;}
        // Interior patches cannot fail even the largest static edge jitter.
        // Avoid a per-vertex hash across millions of fully eligible blades.
        if usable_road-length(decoded)*meadow.support.x>=0.12 {packed_road_gradient=0u;}
    } else if ground.road<0.9+clearance {return result;}
    let directions=array<vec2<f32>,4>(vec2<f32>(1.0,0.0),vec2<f32>(-1.0,0.0),vec2<f32>(0.0,1.0),vec2<f32>(0.0,-1.0));
    for(var k=0u;k<4u;k++) {
        let edge=meadow_ground_at(xz+directions[k]*clearance);
        if !edge.valid || (!trim_road && edge.road<0.9) {return result;}
    }
    let coverage=meadow_habitat_coverage(xz,ground.road,ground.up,seed);
    // Suitable cells retain their shared source. Gradual patch-edge density
    // is authored into actual blade roots and recooked into the far atlas;
    // deleting a whole cell here would expose square holes in both paths.
    // Road/slope eligibility remains conservative; no scalar opacity proxy
    // is substituted for the union coverage of individually thinned blades.
    if coverage<=0.0 {return result;}
    // Exact native-triangle support plus terrain-conforming source geometry
    // needs only a small soil inset. A 2.5 cm inset buried the basal canopy.
    let root=vec3<f32>(xz.x,ground.height-0.005,xz.y);
    let ground_scale=meadow_ground_scale(ground.slope);
    let radius=(rest_radius+wind_margin)*ground_scale;
    if !page && !meadow_in_frustum(root,radius) {return result;}
    if !page && view.fog_cull.y>0.5 && distance(root,view.cam.xyz)>view.fog_cull.x+radius {return result;}
    if !page && meadow_occluded(root,radius) {return result;}
    // LOD keeps every authored blade. The static per-blade road habitat test
    // below may reject edge members consistently across every source LOD.
    // Select source geometry by its conservative projected error, not fixed
    // distance rings. The nearest point of the wind-expanded sphere bounds
    // projection, and the same physical plant is used in every view.
    let near_distance=max(distance(root,view.cam.xyz)-radius,max(view.params.y,1e-4));
    var lod=0u;
    for(var candidate=1u;candidate<3u;candidate++) {
        let error=bitcast<f32>(meadow.lods[candidate].z)*scale*ground_scale;
        let pixels=select(error*view.cam.w/near_distance,error*view.cam.w,view.params.z>0.5);
        if pixels<=meadow.distances.z {lod=candidate;}
    }
    if meadow.wind_history.w>0.5 {
        // Quick_Grass uses six segments near and one beyond 15m, with 1.5m
        // nominal blades. Our .6m recipe scales that to6m. The source vertex
        // path morphs to the actual coarse triangle over3..6m before this cut.
        // This is an authored appearance LOD, not a projected-error promise.
        let d=distance(root.xyz,view.cam.xyz)/max(scale,0.01);
        lod=select(0u,2u,d>=6.0);
    }
    if meadow.support.z>1.5 {lod=0u;}
    if !page && meadow.support.z>1.5 && !meadow_retain_root(cell,root) {return result;}
    result.lod=lod;result.radius=radius;
    result.plant.root_scale=vec4<f32>(root,scale);
    // Full-angle yaw removes repeated axis-aligned patch edges. Existing
    // anchor/blade radial bounds cover every angle, including road trimming.
    let yaw=meadow_random(cell,3u,seed)*MEADOW_TAU;
    let pigment=meadow_material_tint(xz,cell,seed);
    result.plant.yaw=vec4<f32>(cos(yaw),sin(yaw),ground.slope);
    // World-cell identity has already seeded placement/wind. These formerly
    // unused render words carry the edge plane in both serialized layouts.
    result.plant.road_variant_tint=vec4<u32>(packed_road_gradient,bitcast<u32>(usable_road),variant,meadow_pack_tint(pigment));
    if meadow_plant_words(meadow.wind_history.w)>12u {for(var branch=0u;branch<4u;branch++) {
        let pivot=meadow_recipe[9u+variant*4u+branch];
        let amplitude=pivot.w*meadow.range.y*meadow.range.z;
        let detail=MEADOW_TAU*meadow_random(cell,83u+branch,seed);
        let current=meadow_branch_angle(xz,meadow.previous.yz,meadow.range.w,detail,amplitude);
        let previous=meadow_branch_angle(xz,meadow.previous.yz,meadow.previous.x,detail,amplitude);
        result.plant.branches[branch]=vec4<f32>(sin(current),cos(current),sin(previous),cos(previous));
    }}
    result.valid=true;return result;
}

fn meadow_store_candidate(candidate:MeadowCandidate,slot:u32,capacity:u32) {
    if !candidate.valid {return;}
    let lod=candidate.lod;
    let draw_slot=atomicAdd(&meadow_args[lod].instance_count,1u);
    if draw_slot>=capacity {return;}
    let stride=meadow_plant_words(meadow.wind_history.w);
    meadow_pool[capacity*stride+lod*capacity+draw_slot]=slot;
    let offset=slot*stride;let plant=candidate.plant;
    meadow_write4(offset,plant.root_scale);meadow_write4(offset+4u,plant.yaw);
    for(var k=0u;k<4u;k++){meadow_pool[offset+8u+k]=plant.road_variant_tint[k];}
    if stride>12u {for(var branch=0u;branch<4u;branch++){meadow_write4(offset+12u+branch*4u,plant.branches[branch]);}}
}
@compute @workgroup_size(8,8)
fn generate_meadow(@builtin(global_invocation_id) id:vec3<u32>) {
    let dimension=u32(max(meadow.dispatch.z,0));let capacity=u32(max(meadow.dispatch.w,0));
    if id.x>=dimension || id.y>=dimension || capacity==0u || meadow.field.w<=0.0 {return;}
    let slot=id.y*dimension+id.x;
    if slot>=capacity || arrayLength(&meadow_pool)<meadow_pool_words(capacity,meadow.wind_history.w) || arrayLength(&meadow_recipe)<21u {return;}
    let base=vec2<i32>(floor(view.cam.xz/meadow.field.w))-vec2<i32>(i32(dimension/2u));
    meadow_store_candidate(meadow_generate_cell(base+vec2<i32>(id.xy),false),slot,capacity);
}
@compute @workgroup_size(8,8)
fn generate_meadow_page(@builtin(global_invocation_id) id:vec3<u32>) {
    let dimension=u32(max(meadow.dispatch.z,0));let capacity=u32(max(meadow.dispatch.w,0));
    if id.x>=dimension || id.y>=dimension || capacity==0u || meadow.field.w<=0.0 {return;}
    let slot=id.y*dimension+id.x;
    if slot>=capacity || arrayLength(&meadow_pool)<meadow_pool_words(capacity,meadow.wind_history.w) || arrayLength(&meadow_recipe)<21u {return;}
    meadow_store_candidate(meadow_generate_cell(meadow.dispatch.xy+vec2<i32>(id.xy),true),slot,capacity);
}
