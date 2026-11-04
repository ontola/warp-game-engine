// Shared 64 KiB, two-hash visibility Bloom filter. False positives merely
// render extra current geometry first; no false negative can remove geometry
// because phase two always handles the complementary set.
fn tree_history_hash(value:u32)->u32 {
 var x=value;x=(x^(x>>16u))*0x7feb352du;
 x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
fn tree_history_keys(member:u32,cluster:u32)->vec2<u32> {
 let seed=member^(cluster*0x9e3779b9u);
 return vec2<u32>(tree_history_hash(seed),tree_history_hash(seed^0x85ebca6bu))&vec2<u32>(0x3ffffu);
}
