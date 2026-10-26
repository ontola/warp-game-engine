// Fold software-rasterized winners into the hardware visibility and depth
// targets. A fullscreen triangle writes each software pixel's id and depth;
// the pipeline's reversed-Z Greater test keeps whichever is nearer.

@group(0) @binding(0) var<storage, read> vis64: array<u32>; // lo, hi u32 pairs
@group(0) @binding(1) var<uniform> merge_size: vec4<u32>;   // width, height

struct MergeOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn merge_vs(@builtin(vertex_index) i: u32) -> MergeOut {
    var o: MergeOut;
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

struct Fragment {
    @location(0) id: u32,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn merge_fs(in: MergeOut) -> Fragment {
    let p = vec2<u32>(in.pos.xy);
    let i = (p.y * merge_size.x + p.x) * 2u;
    let hi = vis64[i + 1u];
    if hi == 0u { discard; }
    var f: Fragment;
    f.id = vis64[i];
    f.depth = bitcast<f32>(hi);
    return f;
}
