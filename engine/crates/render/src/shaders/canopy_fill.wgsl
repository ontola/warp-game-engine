// Rest-pose crown mass. ALL levels use additive extinction; this intentionally
// replaces the old max-union proxy. Wind is not a structural tree revision.
struct Instance {
    r0:vec4<f32>, r1:vec4<f32>, r2:vec4<f32>, sphere:vec4<f32>,
    first_cluster:u32, cluster_count:u32, scale:f32, flags:u32,
};
struct Parameters {
    origins:array<vec4<f32>,3>, dims:vec4<u32>, mass:vec4<f32>, state:vec4<u32>,
};
@group(0) @binding(4) var<storage,read> instances:array<Instance>;
@group(1) @binding(0) var<uniform> params:Parameters;
@group(1) @binding(1) var<storage,read_write> density:array<atomic<u32>>;
// Instance IDs followed by shared 8-word local leaf bounds. ID zero keeps
// compatibility for foliage without a supported source leaf material.
@group(1) @binding(3) var<storage,read> source_crowns:array<u32>;

// Bounded quadrature over cell/crown overlap. At coarse pitches a crown's
// centre can lie anywhere inside a cell: sampling only its centre loses it.
fn cell_mass(cell:vec3<i32>,pitch:f32,centre:vec3<f32>,radii:vec3<f32>,fine:bool,support:vec2<f32>,compact:bool)->f32 {
    let lower=vec3<f32>(cell)*pitch;
    if fine && (!compact || (lower.y>=support.x && lower.y+pitch<=support.y)) {
        let q=(lower+vec3<f32>(pitch*0.5)-centre)/radii;
        return max(1.0-dot(q,q),0.0)*pitch*pitch*pitch;
    }
    var lo=max(lower,centre-radii);var hi=min(lower+vec3<f32>(pitch),centre+radii);
    if compact {lo.y=max(lo.y,support.x);hi.y=min(hi.y,support.y);}
    if any(hi<=lo) {return 0.0;}
    let midpoint=(lo+hi)*0.5;let half_size=(hi-lo)*0.5;
    let nodes=array<f32,3>(-0.7745966692,0.0,0.7745966692);
    let weights=array<f32,3>(0.5555555556,0.8888888889,0.5555555556);
    var sum=0.0;
    for(var z=0u;z<3u;z++) {for(var y=0u;y<3u;y++) {for(var x=0u;x<3u;x++) {
        let p=midpoint+half_size*vec3<f32>(nodes[x],nodes[y],nodes[z]);
        let q=(p-centre)/radii;
        sum+=max(1.0-dot(q,q),0.0)*weights[x]*weights[y]*weights[z];
    }}}
    return sum*half_size.x*half_size.y*half_size.z;
}

// Monotone encoding lets zero-cleared atomicMax accumulate both upper bounds
// and negated lower bounds, including below-datum trees. Empty cells have no mass.
fn ordered_float(v:f32)->u32 {
    let bits=bitcast<u32>(v);
    return select(bits^0x80000000u,~bits,(bits&0x80000000u)!=0u);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid:vec3<u32>) {
    let index=gid.x+gid.y*65535u*64u;
    if index>=params.state.y || index>=arrayLength(&instances) {return;}
    let inst=instances[index];if (inst.flags&1u)==0u {return;}
    let r=inst.sphere.w;
    var centre=inst.sphere.xyz+vec3<f32>(0.0,r*0.25,0.0);
    var radii=vec3<f32>(r*0.38,r*0.62,r*0.38);
    var source_centre=centre;var source_radii=radii;
    var source_id=0u;
    if index<params.state.z {source_id=source_crowns[index];}
    if source_id!=0u {
        let at=params.state.z+source_id*8u;
        let c=vec3<f32>(bitcast<f32>(source_crowns[at]),bitcast<f32>(source_crowns[at+1u]),bitcast<f32>(source_crowns[at+2u]));
        let h=vec3<f32>(bitcast<f32>(source_crowns[at+4u]),bitcast<f32>(source_crowns[at+5u]),bitcast<f32>(source_crowns[at+6u]));
        source_centre=vec3<f32>(dot(inst.r0.xyz,c)+inst.r0.w,dot(inst.r1.xyz,c)+inst.r1.w,dot(inst.r2.xyz,c)+inst.r2.w);
        source_radii=vec3<f32>(dot(abs(inst.r0.xyz),h),dot(abs(inst.r1.xyz),h),dot(abs(inst.r2.xyz),h));
    }
    if (params.state.w&1u)!=0u {centre=source_centre;radii=source_radii;}
    let mass=(8.0*3.14159265359/15.0)*radii.x*radii.y*radii.z;
    let compact=(params.state.w&2u)!=0u;
    let support=vec2<f32>(source_centre.y-source_radii.y,source_centre.y+source_radii.y);
    // A badly placed legacy proxy must not deposit invisible mass with no
    // reconstruction support. Keep XY and its original mass, move only its
    // integration Y envelope to the actual source as a bounded fallback.
    if compact && (centre.y+radii.y<=support.x || centre.y-radii.y>=support.y) {
        centre.y=source_centre.y;radii.y=source_radii.y;
    }
    let dims=vec3<i32>(params.dims.xyz);
    let level_count=params.dims.x*params.dims.y*params.dims.z;
    for(var level=0u;level<3u;level++) {
        if (params.state.x&(1u<<level))==0u {continue;}
        let pitch=params.origins[level].w;
        let origin=params.origins[level].xyz;
        if any(centre+radii<origin) || any(centre-radii>origin+vec3<f32>(dims)*pitch) {continue;}
        // Absolute grid indices, so normalization is identical across every
        // snapped resident window. Normalize before clipping resident cells.
        let lo=vec3<i32>(floor((centre-radii)/pitch));
        let hi=vec3<i32>(floor((centre+radii)/pitch));
        let resident=vec3<i32>(round(origin/pitch));
        let fine=pitch<=0.5*min(radii.x,min(radii.y,radii.z));
        var integral=0.0;
        for(var z=lo.z;z<=hi.z;z++) {for(var y=lo.y;y<=hi.y;y++) {for(var x=lo.x;x<=hi.x;x++) {
            integral+=cell_mass(vec3<i32>(x,y,z),pitch,centre,radii,fine,support,compact);
        }}}
        if integral<=0.0 {continue;}
        let factor=mass/integral;
        let write_lo=max(lo,resident);let write_hi=min(hi,resident+dims-vec3<i32>(1));
        for(var z=write_lo.z;z<=write_hi.z;z++) {for(var y=write_lo.y;y<=write_hi.y;y++) {for(var x=write_lo.x;x<=write_hi.x;x++) {
            let cell=vec3<i32>(x,y,z);
            let value=u32(round(cell_mass(cell,pitch,centre,radii,fine,support,compact)*factor*params.mass.x));
            if value==0u {continue;}
            let q=vec3<u32>(cell-resident);
            let offset=level*level_count*select(1u,3u,compact)+(q.z*params.dims.y+q.y)*params.dims.x+q.x;
            atomicAdd(&density[offset],value);
            if compact {
                atomicMax(&density[offset+level_count],ordered_float(-support.x));
                atomicMax(&density[offset+2u*level_count],ordered_float(support.y));
            }
        }}}
    }
}
