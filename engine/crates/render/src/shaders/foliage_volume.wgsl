// Shared fractional foliage optics. No resource bindings, owner transforms,
// cameras, lights, scheduling or terrain assumptions live in this module.
// Providers load sparse cells/levels and call these value-only helpers; a
// ground-field page and a tree asset therefore use the same reconstruction.
struct VolumeLevel { origin_cell: vec4<f32>, dims_offset: vec4<u32> };
struct VolumeCell {
    albedo_rough: vec4<f32>,
    extinction: array<vec4<f32>,4>,
    // Actual conditional mean normals and normalized lobe area weights.
    normals: array<vec4<f32>,6>,
};
// This specializes only private interpolation state. Uploaded cells retain
// all six records and their 176-byte ABI in every consumer.
const VOLUME_NORMAL_COUNT: u32 = 6u;
struct VolumeDirection { indices:vec3<u32>,weights:vec3<f32> };
struct VolumeSample {
    sigma:f32,albedo:vec3<f32>,roughness:f32,leaf_fraction:f32,normals:array<vec4<f32>,VOLUME_NORMAL_COUNT>,
    // Nonempty sparse-cell fetches, including zero interpolation weights.
    cell_samples:u32,
};

// Piecewise-linear interpolation over the same thirteen antipodal directions
// as foliage_voxel::Voxel::projected_area_along. Input direction is normalized
// by the provider; its inverse-transform length scales sigma per world metre.
fn volume_direction(d:vec3<f32>)->VolumeDirection {
    let magnitude=abs(d);
    var axes=array<u32,3>(0u,1u,2u);
    if magnitude[axes[0]]<magnitude[axes[1]] {let swap=axes[0];axes[0]=axes[1];axes[1]=swap;}
    if magnitude[axes[1]]<magnitude[axes[2]] {let swap=axes[1];axes[1]=axes[2];axes[2]=swap;}
    if magnitude[axes[0]]<magnitude[axes[1]] {let swap=axes[0];axes[0]=axes[1];axes[1]=swap;}
    let a=axes[0];let b=axes[1];let c=axes[2];
    var pair=2u;
    if min(a,b)==0u {pair=select(1u,0u,max(a,b)==1u);}
    let diagonal=3u+pair*2u+select(0u,1u,d[a]*d[b]<0.0);
    let corner=9u+select(0u,2u,d.x*d.y<0.0)+select(0u,1u,d.x*d.z<0.0);
    let weight=vec3<f32>(magnitude[a]-magnitude[b],(magnitude[b]-magnitude[c])*1.41421356,magnitude[c]*1.73205081);
    return VolumeDirection(vec3<u32>(a,diagonal,corner),weight/max(dot(weight,vec3<f32>(1.0)),1e-8));
}
fn volume_sigma(cell:VolumeCell,d:VolumeDirection)->f32 {
    var value=0.0;
    for(var k=0u;k<3u;k++) {let i=d.indices[k];value+=cell.extinction[i/4u][i%4u]*d.weights[k];}
    return max(value,0.0);
}

// Accumulate one occupied trilinear corner. Keep the existing operation order
// and do not normalize moments until all corners, then all mips, are combined.
fn volume_accumulate_cell(previous:VolumeSample,cell:VolumeCell,sigma:f32)->VolumeSample {
    var result=previous;
    result.cell_samples++;
    result.sigma+=sigma;
    result.albedo+=cell.albedo_rough.rgb*sigma;
    result.roughness+=cell.albedo_rough.a*sigma;
    result.leaf_fraction+=cell.extinction[3].y*sigma;
    for(var k=0u;k<VOLUME_NORMAL_COUNT;k++) {
        let lobe=cell.normals[k];
        result.normals[k]+=vec4<f32>(lobe.xyz*lobe.w,lobe.w)*sigma;
    }
    return result;
}
fn volume_normalize_sample(sample:VolumeSample)->VolumeSample {
    var result=sample;
    if result.sigma>0.0 {
        result.albedo/=result.sigma; result.roughness/=result.sigma;
        result.leaf_fraction/=result.sigma;
        for(var k=0u;k<VOLUME_NORMAL_COUNT;k++) {result.normals[k]/=result.sigma;}
    }
    return result;
}

// Filter extinction, not alpha. Conditional materials/normals are weighted by
// extinction; an empty mip contributes no black albedo or artificial normal.
fn volume_blend_mips(fine:VolumeSample,coarse:VolumeSample,blend:f32)->VolumeSample {
    var a=fine;let b=coarse;
    a.cell_samples+=b.cell_samples;
    let wa=a.sigma*(1.0-blend);let wb=b.sigma*blend;
    let sigma=wa+wb;
    if sigma>0.0 {
        a.albedo=(a.albedo*wa+b.albedo*wb)/sigma;
        a.roughness=(a.roughness*wa+b.roughness*wb)/sigma;
        a.leaf_fraction=(a.leaf_fraction*wa+b.leaf_fraction*wb)/sigma;
        for(var k=0u;k<VOLUME_NORMAL_COUNT;k++) {a.normals[k]=(a.normals[k]*wa+b.normals[k]*wb)/sigma;}
    }
    a.sigma=sigma;
    return a;
}

struct VolumeTransport { transmittance:f32,opacity_weight:f32 };
// A homogeneous segment's exact Beer integral. The caller supplies the actual
// clipped length and summed extinction of ALL overlapping participating cells.
// Empty segments are identity; no omitted interval becomes opaque or glowing.
fn volume_transport_segment(transmittance:f32,sigma:f32,distance:f32)->VolumeTransport {
    if sigma<=0.0 || distance<=0.0 {return VolumeTransport(transmittance,0.0);}
    let attenuation=exp(-sigma*distance);
    return VolumeTransport(transmittance*attenuation,transmittance*(1.0-attenuation));
}

// Source Meadow's thin-leaf direct response, generalized only by material
// leaf_fraction (bark=0). Incident radiance and visibility remain the caller's
// responsibility. Reflection+transmission share the same coloured energy.
const VOLUME_LEAF_TRANSMISSION: f32 = 0.25;
fn volume_thin_leaf_direct(base:vec3<f32>,n:vec3<f32>,v:vec3<f32>,l:vec3<f32>,rough:f32,leaf_fraction:f32)->vec3<f32> {
    let f0=vec3<f32>(0.04);
    let specular_weight=clamp(env_brdf(f0,rough,max(dot(n,v),1e-4)),vec3<f32>(0.0),vec3<f32>(1.0));
    let budget=clamp(base,vec3<f32>(0.0),vec3<f32>(1.0))*(1.0-specular_weight)/PI;
    let transmission=VOLUME_LEAF_TRANSMISSION*clamp(leaf_fraction,0.0,1.0);
    let ndl=dot(n,l);
    return budget*((1.0-transmission)*max(ndl,0.0)+transmission*max(-ndl,0.0))
        +ggx(n,v,l,rough,f0);
}
