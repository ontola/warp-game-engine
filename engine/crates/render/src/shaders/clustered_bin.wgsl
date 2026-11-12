// Bin clustered lights into the world grid (clustered_lights.rs): one
// invocation per cell tests every light's sphere against the cell's box and
// keeps the first 62 in light order. Cell record: 32 words, word 0 is the
// count, then u16 light indices two per word. `arena` is the scene tables
// buffer; every offset is in its vec4s.
struct LightGrid { origin: vec4<f32>, inv_cell: vec4<f32>, dims: vec4<u32>, offsets: vec4<u32> };
@group(0) @binding(0) var<uniform> grid: LightGrid;
@group(0) @binding(1) var<storage, read_write> arena: array<vec4<u32>>;

const MAX_PER_CELL: u32 = 62u;
var<workgroup> chunk: array<vec4<f32>, 64>;

@compute @workgroup_size(64)
fn bin(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let cells = grid.offsets.z;
    let cell = gid.x;
    let dims = grid.dims.xyz;
    let coord = vec3<u32>(cell % dims.x, (cell / dims.x) % dims.y, cell / (dims.x * dims.y));
    let size = 1.0 / grid.inv_cell.xyz;
    let lo = grid.origin.xyz + vec3<f32>(coord) * size;
    let hi = lo + size;
    let base = grid.offsets.y + cell * 8u;
    let lights = grid.dims.w;
    var count = 0u;
    var packed = 0u;
    for (var first = 0u; first < lights; first += 64u) {
        workgroupBarrier();
        if first + li < lights { chunk[li] = bitcast<vec4<f32>>(arena[grid.offsets.w + (first + li) * 5u]); }
        workgroupBarrier();
        let n = min(64u, lights - first);
        for (var j = 0u; j < n; j++) {
            let s = chunk[j];
            let d = max(lo - s.xyz, vec3<f32>(0.0)) + max(s.xyz - hi, vec3<f32>(0.0));
            if cell < cells && count < MAX_PER_CELL && dot(d, d) < s.w * s.w {
                let index = first + j;
                if (count & 1u) == 0u { packed = index; } else {
                    packed = packed | (index << 16u);
                    let word = 1u + count / 2u;
                    arena[base + word / 4u][word % 4u] = packed;
                }
                count += 1u;
            }
        }
    }
    if cell >= cells { return; }
    if (count & 1u) == 1u {
        let word = 1u + count / 2u;
        arena[base + word / 4u][word % 4u] = packed;
    }
    arena[base].x = count;
}
