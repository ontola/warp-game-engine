//! CPU-only source/LOD diagnostic. Never initializes a GPU or game window.
use glam::Vec3;
use render::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex, read_kmesh};
use serde_json::json;

fn metrics(vertices: &[Vertex], triangles: &[[u32; 3]]) -> serde_json::Value {
    let mut area = 0.0f64; let mut reversed = 0.0f64; let mut flat_uv = 0.0f64;
    let mut reversed_count = 0; let mut flat_count = 0; let mut low_count = 0;
    for tri in triangles {
        let p = tri.map(|i| Vec3::from(vertices[i as usize].pos));
        if (p[0].y + p[1].y + p[2].y) / 3.0 > 5.5 { continue; }
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let weight = face.length() as f64 * 0.5;
        if weight < 1e-12 { continue; }
        let normal = tri.map(|i| Vec3::from(vertices[i as usize].normal));
        let mean = (normal[0] + normal[1] + normal[2]).normalize_or_zero();
        // Original retained KMSH wood has clockwise winding and outward
        // normals, so a positive dot is the exceptional/opposite orientation.
        if face.normalize().dot(mean) > 0.1 { reversed += weight; reversed_count += 1; }
        let uv = tri.map(|i| glam::Vec2::from(vertices[i as usize].uv));
        if (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs() < 1e-8 { flat_uv += weight; flat_count += 1; }
        area += weight; low_count += 1;
    }
    json!({"low_tris":low_count,"low_area":area,"opposed_to_source_winding_count":reversed_count,
        "opposed_to_source_winding_area_fraction":reversed/area.max(1e-12),
        "collapsed_uv_count":flat_count,"collapsed_uv_area_fraction":flat_uv/area.max(1e-12)})
}

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut rows = Vec::new();
    for asset in ["a", "b", "c"] {
        let parts = read_kmesh(&std::fs::read(root.join(format!("fir_{asset}_near.kmesh"))).unwrap()).unwrap();
        for (name, vertices, indices) in parts.iter().filter(|(n,_,_)| n.contains("trunk") || n.contains("bark")) {
            let source: Vec<[u32; 3]> = indices.chunks_exact(3).map(|v| [v[0],v[1],v[2]]).collect();
            rows.push(json!({"asset":asset,"part":name,"selection":"source","metrics":metrics(vertices,&source)}));
          for policy in [SimplificationPolicy::Geometry,SimplificationPolicy::TexturedGeometry] {
            let mesh = ClusterMesh::build_with_policy(&[Part { vertices, indices, material:0 }], |_|policy);
            for cut in [0.0f32,0.001,0.003,0.01,0.015625,0.09375] {
                let mut triangles = Vec::new();
                for c in mesh.clusters.iter().filter(|c|c.own_error<=cut && c.parent_error>cut) {
                    for packed in &mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count) as usize] {
                        triangles.push(std::array::from_fn(|k|mesh.cluster_verts[c.first_vert as usize+((packed>>(8*k))&255) as usize]));
                    }
                }
                rows.push(json!({"asset":asset,"part":name,"selection":format!("{policy:?}"),"error_cut_m":cut,
                    "total_tris":triangles.len(),"metrics":metrics(&mesh.vertices,&triangles)}));
            }
          }
        }
    }
    println!("{}",serde_json::to_string_pretty(&rows).unwrap());
}
