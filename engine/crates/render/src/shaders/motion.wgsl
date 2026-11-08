struct MotionUniform {
    inverse_current: mat4x4<f32>, current: mat4x4<f32>, previous: mat4x4<f32>,
    camera: vec4<f32>, size: vec4<f32>, flags: vec4<u32>,
    grass: vec4<f32>,
    // Skin table start (vec4s into previous_wind), cluster-vertex and
    // triangle region starts (words into topology), skinned actor count.
    skin: vec4<u32>,
};
struct Instance {
    r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, sphere: vec4<f32>,
    first_cluster: u32, cluster_count: u32, scale: f32, flags: u32,
};
@group(0) @binding(0) var<uniform> motion: MotionUniform;
@group(0) @binding(1) var<storage,read> current_instances: array<Instance>;
@group(0) @binding(2) var<storage,read> previous_instances: array<Instance>;

// Address/deformation composition is shared in virtual_member.wgsl.
fn current_at(id: u32) -> Instance {
    if (id & VIRTUAL_MEMBER) == 0u { return current_instances[id]; }
    let owner = current_instances[member_owner(id)];
    return compose_member(owner, current_instances[member_template(owner,id)]);
}
fn previous_at(id: u32) -> Instance {
    if (id & VIRTUAL_MEMBER) == 0u { return previous_instances[id]; }
    let owner = previous_instances[member_owner(id)];
    return compose_member(owner, previous_instances[member_template(owner,id)]);
}
@group(0) @binding(3) var<storage,read> current_wind: array<vec4<f32>>;
// Previous wind per slot, then the skin table (count, then per actor:
// owner, first vertex, vertex count, vec4 offset of its previous vertices),
// then last frame's skinned vertices (two vec4s each, Vertex layout).
@group(0) @binding(4) var<storage,read> previous_wind: array<vec4<u32>>;
@group(0) @binding(5) var<storage,read> visible: array<vec2<u32>>;
@group(0) @binding(6) var visibility: texture_2d<u32>;
@group(0) @binding(7) var opaque_depth: texture_depth_2d;
@group(0) @binding(8) var grass_metadata: texture_2d<u32>;
@group(0) @binding(9) var glass_metadata: texture_2d<u32>;
@group(0) @binding(10) var far_color: texture_2d<f32>;
@group(0) @binding(11) var far_distance: texture_2d<f32>;
@group(0) @binding(12) var voxel_color: texture_2d<f32>;
@group(0) @binding(13) var voxel_reprojection: texture_2d<f32>;
@group(0) @binding(14) var meadow_color: texture_2d<f32>;
@group(0) @binding(15) var meadow_reprojection: texture_2d<f32>;
struct Cluster {
    cull: vec4<f32>, parent: vec4<f32>, own: vec4<f32>,
    parent_error: f32, own_error: f32,
    first_tri: u32, tri_count: u32, first_vert: u32, material: u32, depth: u32, pad: u32,
};
@group(0) @binding(16) var<storage,read> topology: array<u32>;
@group(0) @binding(17) var<storage,read> clusters: array<Cluster>;
// Last frame's surface ids (location 3 below) of this view.
@group(0) @binding(18) var previous_ids: texture_2d<u32>;
// This view's voxel splat records (voxel_forest.wgsl volume_splats): a
// voxel pixel's splat id leads to its tree and that tree's owner instance.
@group(0) @binding(19) var<storage,read> voxel_splats: array<vec4<u32>>;
fn voxel_owner(splat:u32)->u32 {
    let count=arrayLength(&voxel_splats);
    if splat==0u || count<12u {return 0xffffffffu;}
    let at=12u+voxel_splats[11].x*5u+splat-1u;
    if at>=count {return 0xffffffffu;}
    let tree=12u+voxel_splats[at].x*5u+4u;
    if tree>=count {return 0xffffffffu;}
    return voxel_splats[tree].x;
}
// Which surface a pixel shows: its owner instance and whether as mesh or
// voxels, hashed. A pixel whose reprojected 2x2 footprint showed none of
// it last frame (a representation swap, a recycled slot, disocclusion)
// rejects its history there, instead of resetting the whole view.
struct MotionOut { @location(0) vector: vec2<f32>, @location(1) depth: f32, @location(2) bias: f32, @location(3) id: u32 };
fn surface_hash(x:u32)->u32 {
    var h=x*747796405u+2891336453u;
    h=((h>>((h>>28u)+4u))^h)*277803737u;
    return (h>>22u)^h;
}
@vertex fn motion_vs(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    return vec4<f32>(array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0))[index],0.0,1.0);
}
fn finite4(v:vec4<f32>)->bool { return all(abs(v)<vec4<f32>(1e30)); }
fn transform(inst:Instance,p:vec3<f32>)->vec3<f32> {
    let v=vec4<f32>(p,1.0);return vec3<f32>(dot(inst.r0,v),dot(inst.r1,v),dot(inst.r2,v));
}
fn rest_root(inst:Instance)->vec3<f32> {return vec3<f32>(inst.r0.w,inst.r1.w,inst.r2.w);}
fn deform(p:vec3<f32>,root_y:f32,shear:vec2<f32>)->vec3<f32> {return p+vec3<f32>(shear.x,0.0,shear.y)*(p.y-root_y);}

// Source primitives have stable owner IDs even when several share one branch
// pose. PART_WIND writes branch * attachment-rest into each live instance;
// previous_instances contains those same rows from the last rendered frame.
// A branch pivot moving in world space is motion, not recycled-slot identity.
// Only LIVE_GRASS's camera-resident pool uses its root as the slot identity.
// A skinned surface point: its barycentric position on the current skinned
// triangle (object space), taken on the same triangle of last frame's
// skinned vertices. Zero (reject history) when the triangle is unknown.
fn previous_skinned_local(owner:u32,local:vec3<f32>,cluster:u32,tri:u32)->vec4<f32> {
    let table=motion.skin.x;
    var actor=vec4<u32>(0xffffffffu);
    for (var i=0u;i<motion.skin.w;i++) {
        let a=previous_wind[table+1u+i];
        if a.x==owner {actor=a;break;}
    }
    if actor.x!=owner || cluster>=arrayLength(&clusters) {return vec4<f32>(0.0);}
    let cl=clusters[cluster];
    if tri>=cl.tri_count {return vec4<f32>(0.0);}
    let packed=topology[motion.skin.z+cl.first_tri+tri];
    var now:array<vec3<f32>,3>;var old:array<vec3<f32>,3>;
    for (var k=0u;k<3u;k++) {
        let v=topology[motion.skin.y+cl.first_vert+((packed>>(8u*k))&0xffu)];
        if v<actor.y || v-actor.y>=actor.z {return vec4<f32>(0.0);}
        now[k]=vec3<f32>(bitcast<f32>(topology[v*8u]),bitcast<f32>(topology[v*8u+1u]),bitcast<f32>(topology[v*8u+2u]));
        old[k]=bitcast<vec4<f32>>(previous_wind[actor.w+(v-actor.y)*2u]).xyz;
    }
    let e1=now[1]-now[0];let e2=now[2]-now[0];let d=local-now[0];
    let d00=dot(e1,e1);let d01=dot(e1,e2);let d11=dot(e2,e2);
    let den=d00*d11-d01*d01;
    if !(abs(den)>1e-24) {return vec4<f32>(0.0);}
    let b1=(d11*dot(d,e1)-d01*dot(d,e2))/den;
    let b2=(d00*dot(d,e2)-d01*dot(d,e1))/den;
    return vec4<f32>(old[0]*(1.0-b1-b2)+old[1]*b1+old[2]*b2,1.0);
}

// Rasterized voxels (voxel_splat_raster.wgsl) are drawn in their owner
// instance's current pose without wind shear: a branch voxel sways with
// its frame, a whole-tree proxy stays put.
fn previous_voxel_point(owner:u32,world:vec3<f32>)->vec4<f32> {
    if owner>=arrayLength(&current_instances) || owner>=arrayLength(&previous_instances) {return vec4<f32>(0.0);}
    let now=current_instances[owner];let old=previous_instances[owner];
    if ((now.flags^old.flags)&~(7u<<24u))!=0u {return vec4<f32>(0.0);}
    let rest=world-rest_root(now);
    let co0=cross(now.r1.xyz,now.r2.xyz);let co1=cross(now.r2.xyz,now.r0.xyz);let co2=cross(now.r0.xyz,now.r1.xyz);
    let determinant=dot(now.r0.xyz,co0);
    if !(abs(determinant)>1e-12 && abs(determinant)<1e20) {return vec4<f32>(0.0);}
    return vec4<f32>(transform(old,(co0*rest.x+co1*rest.y+co2*rest.z)/determinant),1.0);
}

fn previous_source_point(owner:u32,world:vec3<f32>,cluster:u32,tri:u32)->vec4<f32> {
    let slot=member_slot(owner);
    if slot>=arrayLength(&current_instances) || slot>=arrayLength(&previous_instances)
        || slot>=arrayLength(&current_wind) || slot>=arrayLength(&previous_wind) {return vec4<f32>(0.0);}
    let now=current_at(owner);let old=previous_at(owner);
    if old.cluster_count==0u || now.first_cluster!=old.first_cluster || now.cluster_count!=old.cluster_count
        // Category provenance cannot change geometry or motion history.
        || ((now.flags^old.flags)&~(7u<<24u))!=0u
        || ((now.flags&8u)!=0u && any(rest_root(now)!=rest_root(old))) {return vec4<f32>(0.0);}
    // Match raster's actual affine wind buffer (zero for PART_WIND). The full
    // cofactor inverse is required: a nonuniform root followed by a rotated
    // child/primitive attachment can produce a nonorthogonal world basis.
    let rest=deform(world,now.r1.w,-current_wind[slot].xy)-rest_root(now);
    let co0=cross(now.r1.xyz,now.r2.xyz);let co1=cross(now.r2.xyz,now.r0.xyz);let co2=cross(now.r0.xyz,now.r1.xyz);
    let determinant=dot(now.r0.xyz,co0);
    if !(abs(determinant)>1e-12 && abs(determinant)<1e20) {return vec4<f32>(0.0);}
    let local=(co0*rest.x+co1*rest.y+co2*rest.z)/determinant;
    // Skinned vertices change object-space shape: follow the triangle.
    if (now.flags&64u)!=0u {
        if cluster==0xffffffffu {return vec4<f32>(0.0);}
        let before=previous_skinned_local(owner,local,cluster,tri);
        if before.w==0.0 {return before;}
        return vec4<f32>(transform(old,before.xyz),1.0);
    }
    return vec4<f32>(deform(transform(old,local),old.r1.w,bitcast<vec4<f32>>(previous_wind[slot]).xy),1.0);
}

@fragment fn motion_fs(@builtin(position) position:vec4<f32>)->MotionOut {
    let pixel=vec2<i32>(position.xy);
    let ndc=vec2<f32>(position.x*motion.size.z*2.0-1.0,1.0-position.y*motion.size.w*2.0);
    var out:MotionOut;out.vector=vec2<f32>(0.0);out.depth=textureLoad(opaque_depth,pixel,0);out.bias=0.0;
    var owner=0xffffffffu;
    // The opaque surface's cluster and triangle, while it owns the motion.
    var cluster=0xffffffffu;var tri=0u;
    // A rasterized voxel (bit 31 of the visibility id, shade.wgsl
    // VOXEL_SURFACE); the low bits are its splat (voxel_owner).
    var voxel=false;
    if out.depth>0.0 {
        let id=textureLoad(visibility,pixel,0).r;
        voxel=(id&0x80000000u)!=0u;
        let slot=id>>7u;
        if voxel {owner=voxel_owner(id&0x7fffffffu);}
        else {
            if slot>=arrayLength(&visible) {out.bias=1.0;return out;}
            owner=visible[slot].x;cluster=visible[slot].y;tri=id&127u;
        }
    }
    var grass=vec2<u32>(0u);var glass=vec2<u32>(0u);
    if motion.flags.y!=0u {grass=textureLoad(grass_metadata,pixel,0).rg;}
    if motion.flags.z!=0u {glass=textureLoad(glass_metadata,pixel,0).rg;}
    let grass_alpha=f32(grass.x&255u)/255.0;
    let glass_alpha=f32(glass.x&255u)/255.0;
    var meadow_alpha=0.0;
    if motion.grass.w>0.5 {meadow_alpha=clamp(textureLoad(meadow_color,pixel,0).a,0.0,1.0);}
    let underlying_transmission=(1.0-meadow_alpha)*(1.0-glass_alpha);
    let opaque=out.depth;
    // Production order: voxel, old grass, meadow, glass. Later layers attenuate
    // earlier contributions; a faint front layer must not take motion ownership.
    out.bias=max(4.0*grass_alpha*(1.0-grass_alpha)*(1.0-meadow_alpha),4.0*glass_alpha*(1.0-glass_alpha));
    if grass_alpha*underlying_transmission>=0.5 {
        let d=bitcast<f32>(grass.y);
        if d>=opaque && d<=1.0 {owner=(grass.x>>8u)-1u;cluster=0xffffffffu;voxel=false;out.depth=d;}
    }
    if glass_alpha>=0.5 {
        let d=bitcast<f32>(glass.y);
        if d>=opaque && d<=1.0 {owner=(glass.x>>8u)-1u;cluster=0xffffffffu;voxel=false;out.depth=d;}
    }
    // Statistical distant grass has no tracked individual surface. Use its
    // optical centroid only when dominant and bias history by its coverage;
    // never pretend that an opaque ground vector describes swaying needles.
    var layer_current=vec4<f32>(0.0);var layer_previous=vec4<f32>(0.0);
    var grass_coverage=grass_alpha;
    if motion.flags.w!=0u {
        let q=clamp(pixel/2,vec2<i32>(0),vec2<i32>(textureDimensions(far_color))-1);
        let far=textureLoad(far_color,q,0);let horizontal_distance=textureLoad(far_distance,q,0).r;
        let near_weight=select(grass_distance_opacity(horizontal_distance,motion.grass.xy),grass_alpha,grass_alpha>0.0);
        let contribution=far.a*(1.0-near_weight)*underlying_transmission;
        grass_coverage=clamp(grass_alpha+far.a*(1.0-near_weight),0.0,1.0);
        if contribution>0.0 && horizontal_distance>0.0 {
            out.bias=max(out.bias,contribution);
            if contribution>=0.5 {
                // The colour/distance were traced at the half-resolution ray
                // centre and store horizontal distance, not full ray length.
                let centre=min(vec2<f32>(q*2)+1.0,motion.size.xy-0.5);
                let half_ndc=centre*motion.size.zw*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
                let direction4=motion.inverse_current*vec4<f32>(half_ndc,0.0,1.0);
                let direction=normalize(direction4.xyz);
                let point=motion.camera.xyz+direction*(horizontal_distance/max(length(direction.xz),1e-8));
                let clip=motion.current*vec4<f32>(point,1.0);
                if clip.w>0.0 && clip.z/clip.w>=out.depth {
                    layer_current=vec4<f32>(point,1.0);layer_previous=layer_current;owner=0xffffffffu;out.depth=clip.z/clip.w;
                }
            }
        }
    }
    // The voxel layer precedes the grass blend and glass in production. Its
    // integrator weights all overlapping branch motions by the same extinction
    // as color. One centroid cannot encode multiple moving/background layers,
    // so partial coverage biases history even when the background stays owner.
    if motion.grass.z>0.5 {
        let alpha=textureLoad(voxel_color,pixel,0).a;
        let contribution=clamp(alpha,0.0,1.0)*(1.0-grass_coverage)*underlying_transmission;
        out.bias=max(out.bias,4.0*contribution*(1.0-contribution));
        if contribution>0.0 {
            let reprojection=textureLoad(voxel_reprojection,pixel,0);
            let direction4=motion.inverse_current*vec4<f32>(ndc,0.0,1.0);
            let ray=direction4.xyz-motion.camera.xyz*direction4.w;
            if !finite4(reprojection) || !(reprojection.w>0.0) || !finite4(direction4) || dot(ray,ray)<1e-20 {
                out.bias=1.0;
            } else if contribution>=0.5 {
                let point=motion.camera.xyz+normalize(ray)*reprojection.w;
                let clip=motion.current*vec4<f32>(point,1.0);
                if finite4(clip) && clip.w>0.0 && clip.z/clip.w>=opaque && clip.z/clip.w<=1.0 {
                    layer_current=vec4<f32>(point,1.0);
                    layer_previous=vec4<f32>(point+reprojection.xyz,1.0);
                    owner=0xffffffffu;out.depth=clip.z/clip.w;
                } else {out.bias=1.0;}
            }
        }
    }
    // Meadow supplies the previous position of the very same procedural blade
    // point, evaluated with the previous rendered wind clock. Camera-resident
    // list indices never act as owner IDs and cannot inherit a recycled slot.
    let meadow_contribution=meadow_alpha*(1.0-glass_alpha);
    out.bias=max(out.bias,4.0*meadow_contribution*(1.0-meadow_contribution));
    if meadow_contribution>0.0 {
        let reprojection=textureLoad(meadow_reprojection,pixel,0);
        let direction4=motion.inverse_current*vec4<f32>(ndc,0.0,1.0);
        let ray=direction4.xyz-motion.camera.xyz*direction4.w;
        if !finite4(reprojection) || !(reprojection.w>0.0) || !finite4(direction4) || dot(ray,ray)<1e-20 {
            out.bias=1.0;
        } else if meadow_contribution>=0.5 {
            let point=motion.camera.xyz+normalize(ray)*reprojection.w;
            let clip=motion.current*vec4<f32>(point,1.0);
            if finite4(clip) && clip.w>0.0 && clip.z/clip.w>=opaque && clip.z/clip.w<=1.0 {
                layer_current=vec4<f32>(point,1.0);
                layer_previous=vec4<f32>(point+reprojection.xyz,1.0);
                owner=0xffffffffu;out.depth=clip.z/clip.w;
            } else {out.bias=1.0;}
        }
    }
    if layer_current.w!=0.0 {out.id=0xfffffffdu;}
    else if out.depth<=0.0 {out.id=0xfffffffeu;}
    else {out.id=surface_hash(owner)^select(0u,0x9e3779b9u,voxel);}
    // Reset rejects correspondence, not the current frame's selected depth.
    if motion.flags.x!=0u {out.bias=1.0;return out;}
    var current=motion.inverse_current*vec4<f32>(ndc,out.depth,1.0);
    var previous=current;
    if layer_current.w!=0.0 {current=layer_current;previous=layer_previous;}
    else if out.depth<=0.0 {current.w=0.0;previous.w=0.0;}
    else {
        if !finite4(current) || abs(current.w)<1e-10 {out.bias=1.0;return out;}
        current=vec4<f32>(current.xyz/current.w,1.0);
        if voxel {
            previous=previous_voxel_point(owner,current.xyz);
            if previous.w==0.0 {out.bias=1.0;return out;}
        } else {
            previous=previous_source_point(owner,current.xyz,cluster,tri);
            if previous.w==0.0 {out.bias=1.0;return out;}
        }
    }
    let a=motion.current*current;let b=motion.previous*previous;
    if !finite4(a) || !finite4(b) || a.w<=1e-7 || b.w<=1e-7 {out.bias=1.0;return out;}
    let prev_ndc=b.xy/b.w;
    let vector=(prev_ndc-a.xy/a.w)*vec2<f32>(0.5,-0.5)*motion.size.xy;
    if !all(abs(vector)<vec2<f32>(32752.0)) {out.bias=1.0;return out;}
    out.vector=vector;
    if any(abs(prev_ndc)>vec2<f32>(1.0)) {out.bias=1.0;}
    else if out.bias<1.0 {
        let base=vec2<i32>(floor(position.xy+vector-0.5));
        let top=vec2<i32>(textureDimensions(previous_ids))-1;
        var seen=false;
        for (var k=0;k<4;k++) {
            let q=clamp(base+vec2<i32>(k&1,k>>1u),vec2<i32>(0),top);
            seen=seen || textureLoad(previous_ids,q,0).r==out.id;
        }
        if !seen {out.bias=1.0;}
    }
    return out;
}
