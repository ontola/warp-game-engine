// Match scene::LOD_PROXY_LAYER_BIAS. A marked, untextured material retains
// its original surface kind. The per-instance hit-flash byte instead holds
// the proxy's coverage, shared by every face of the shell at this pixel.
const LOD_PROXY_LAYER_BIAS: i32 = -65536;
fn lod_proxy(layer: i32) -> bool { return layer < LOD_PROXY_LAYER_BIAS; }
fn source_layer(layer: i32) -> i32 {
    return select(layer, layer - LOD_PROXY_LAYER_BIAS, lod_proxy(layer));
}
fn instance_byte(flags: u32) -> u32 { return (flags >> 16u) & 255u; }

// Nested 16x16 ordered coverage. The mask has no frame or triangle seed:
// rays through successive faces remove the same pixels and a parked camera
// does not sparkle. Full coverage retains every pixel, including rank 255.
fn bayer_digit(x: u32, y: u32, bit: u32) -> u32 {
    return ((((x ^ y) >> bit) & 1u) << 1u) | ((y >> bit) & 1u);
}
fn lod_proxy_discards(pixel: vec2<f32>, coverage: u32) -> bool {
    if coverage == 255u { return false; }
    let p = vec2<u32>(pixel);
    let rank = bayer_digit(p.x,p.y,0u)*64u + bayer_digit(p.x,p.y,1u)*16u
        + bayer_digit(p.x,p.y,2u)*4u + bayer_digit(p.x,p.y,3u);
    return rank >= coverage;
}
