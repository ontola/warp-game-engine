// Separate binding scope: the output must not remain bound as writable storage
// while evaluate_poses consumes it as indirect arguments.
struct State { wind:vec4<f32>, counts:vec4<u32> };
@group(0) @binding(0) var<storage,read> queues:array<u32>;
@group(0) @binding(1) var<storage,read_write> dispatches:array<u32>;
@group(0) @binding(2) var<uniform> state:State;

@compute @workgroup_size(64)
fn prepare(@builtin(global_invocation_id) id:vec3<u32>) {
    let depth=id.x;
    if depth>=64u {return;}
    // Match the consumer's bounded queue count, including an empty frame after
    // a previously populated frame. Never retain stale dispatch arguments.
    let count=min(queues[depth],state.counts.x);
    dispatches[depth*3u]=(count+63u)/64u;
    dispatches[depth*3u+1u]=1u;
    dispatches[depth*3u+2u]=1u;
}
