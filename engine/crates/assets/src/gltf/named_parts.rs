use super::{policy, Document, ModelPart};
use glam::Vec3;
use render::cluster::{ClusterMesh, Part};
use render::scene::Scene;
use std::collections::HashSet;

/// A named glTF mesh node can be drawn on its own around its local centre.
/// The intact mesh uses the same part ID on every level of its cluster tree.
pub(super) fn build(scene: &mut Scene, doc: &Document) -> Result<(Vec<u8>, Vec<(String, ModelPart)>), String> {
    let tags: Vec<_> = doc.nodes.iter().map(|node| node.extras.as_deref()
        .map(serde_json::from_str::<serde_json::Value>).transpose()).collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if !tags.iter().any(|tag| tag.as_ref().is_some_and(|v| v.get("warpPart").is_some())) {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut ids = Vec::new();
    let mut parts = Vec::new();
    let mut names = HashSet::new();
    let mut used = HashSet::new();
    for (node, tag) in doc.nodes.iter().zip(tags) {
        let value = tag.as_ref().and_then(|tag| tag.get("warpPart"));
        let count = node.primitives.iter().filter(|p| !p.detached_only && !p.indices.is_empty()).map(|p| p.vertices.len()).sum::<usize>();
        let Some(value) = value else {
            ids.resize(ids.len() + count, 0);
            continue;
        };
        let id = value.as_u64().filter(|id| *id < 32).ok_or("warpPart must be an integer between 0 and 31")? as u8;
        let name = node.name.as_deref().filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
            .ok_or("warpPart needs a mesh node name containing letters, digits, _ or -")?;
        if !names.insert(name) || !used.insert(id) { return Err(format!("duplicate warpPart name or id: {name} ({id})")); }
        if count == 0 { return Err(format!("warpPart has no assembled surface: {name}")); }
        ids.resize(ids.len() + count, id);
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in node.primitives.iter().flat_map(|p| &p.vertices) {
            min = min.min(Vec3::from(v.pos));
            max = max.max(Vec3::from(v.pos));
        }
        let center = (min + max) * 0.5;
        let centered: Vec<_> = node.primitives.iter().map(|prim| prim.vertices.iter().map(|v| {
            let mut v = *v;
            v.pos = (Vec3::from(v.pos) - center).to_array();
            v
        }).collect::<Vec<_>>()).collect();
        let primitives: Vec<_> = node.primitives.iter().zip(&centered).map(|(p, vertices)| Part {
            vertices, indices: &p.indices, material: p.material,
        }).collect();
        let cluster = ClusterMesh::build_with_policy(&primitives, |p| policy(scene, doc, p));
        let mesh = scene.add_mesh(&cluster);
        parts.push((name.to_owned(), ModelPart { mesh, min: min-center, max: max-center, hinge: center }));
    }
    Ok((ids, parts))
}
