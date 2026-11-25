//! What a building's glTF says about itself survives: textured materials,
//! lamps and saved views.
use architecture::House;
use glam::Vec2;
use render::Scene;
use std::path::Path;

const BUFFER: &str = "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAQAAAAAAAAAAAAAAAAAAAQEAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/AAABAAIAAAA=";
const RED_PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGM4oSH3HwAE2AIONevgXwAAAABJRU5ErkJggg==";

/// One wall triangle (4 x 3 m), a lamp above it and a camera in front.
fn house(textured: bool) -> String {
    let material = if textured { r#"{"name":"Red brick","pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}"# }
        else { r#"{"name":"Red brick","pbrMetallicRoughness":{"baseColorFactor":[0.8,0.2,0.1,1]}}"# };
    format!(r#"{{"asset":{{"version":"2.0"}},
        "extensionsUsed":["KHR_lights_punctual"],
        "extensions":{{"KHR_lights_punctual":{{"lights":[{{"type":"point","intensity":60}}]}}}},
        "buffers":[{{"byteLength":68,"uri":"{BUFFER}"}}],
        "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":24}},{{"buffer":0,"byteOffset":60,"byteLength":6}}],
        "accessors":[
            {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[4,3,0]}},
            {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}},
            {{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}}],
        "images":[{{"uri":"{RED_PIXEL}"}}],
        "textures":[{{"source":0}}],
        "materials":[{material}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
        "cameras":[{{"type":"perspective","perspective":{{"yfov":0.8,"znear":0.1}}}}],
        "scenes":[{{"nodes":[0,1,2]}}],
        "nodes":[
            {{"name":"Wall","mesh":0,"extras":{{"ifcClass":"IfcWall","reviewState":"confirmed"}}}},
            {{"name":"Lamp","translation":[2,2.5,1],"extensions":{{"KHR_lights_punctual":{{"light":0}}}}}},
            {{"name":"Front view","camera":0,"translation":[2,1.6,6]}}]}}"#)
}

fn load(textured: bool, at: Option<Vec2>) -> (Scene, House) {
    let mut scene = Scene::default();
    let house = House::from_glb(&mut scene, house(textured).as_bytes(), Path::new("wall.gltf"), at, 0.0).unwrap();
    (scene, house)
}

#[test]
fn a_textured_material_is_kept_and_a_plain_one_becomes_a_warp_surface() {
    let (plain, _) = load(false, None);
    let (textured, _) = load(true, None);
    // The plain wall gets a Warp surface material on top of the file's own
    // and the proposal ghost; the textured one keeps the file's.
    assert_eq!(textured.materials.len() + 1, plain.materials.len());
    let layers: Vec<i32> = textured.materials.iter().map(|m| m.layer).collect();
    assert!(layers.iter().any(|l| *l >= 0), "the file's texture stays: {layers:?}");
}

#[test]
fn lamps_and_views_are_placed_like_the_house() {
    let (_, here) = load(false, None);
    assert_eq!(here.lights.len(), 1);
    assert_eq!(here.cameras[0].name, "Front view");
    assert!((here.lights[0].position.x - 2.0).abs() < 1e-4, "{:?}", here.lights[0].position);
    // Placed with its middle (x = 2, the middle of the 4 m wall) at x = 10.
    let (_, moved) = load(false, Some(Vec2::new(10.0, 0.0)));
    assert!((moved.lights[0].position.x - 10.0).abs() < 1e-4);
    assert!((moved.cameras[0].camera.pos.x - 10.0).abs() < 1e-4);
    assert_eq!(moved.clustered_lights().len(), 1);
}
