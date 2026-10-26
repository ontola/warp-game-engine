//! Explicit capture-only geometry accounting; never a synchronous gameplay probe.
use render::{geometry_diagnostics::Config, scene::GeometryCategory, Gpu, Renderer, Scene};
use serde_json::{json, Value};

/// Diagnostic-only multiset fingerprint: GPU append order may vary, while
/// duplicate or missing (owner, cluster) submissions must remain observable.
fn selection_digest(pairs:&[[u32;2]])->String {
    let mut sorted=pairs.to_vec();sorted.sort_unstable();
    let mut hash=0xcbf29ce484222325u64;
    for byte in (pairs.len() as u64).to_le_bytes().into_iter().chain(
        sorted.iter().flat_map(|pair|pair.iter().flat_map(|word|word.to_le_bytes()))) {
        hash^=u64::from(byte);hash=hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn group_work(g:&render::cull_groups::Diagnostics)->Value {
    json!({"enabled":g.enabled,"total_parent_groups":g.total_parent_groups,
        "total_leaf_groups":g.total_leaf_groups,"owner_slots":g.owner_slots,
        "shared_bytes":g.shared_bytes,"owner_scratch_bytes_per_view":g.owner_scratch_bytes_per_view,
        "parent_groups_tested":g.parent_groups_tested,"parent_groups_rejected":g.parent_groups_rejected,
        "leaf_groups_tested":g.leaf_groups_tested,"leaf_groups_rejected":g.leaf_groups_rejected,
        "owner_tests":g.owner_tests,"compacted_owners":g.compacted_owners,
        "resident_clusters_visited":g.resident_clusters_visited,"overflow_fallbacks":g.overflow_fallbacks,
        "traversal_mismatch_fallbacks":g.traversal_mismatch_fallbacks})
}

pub fn config(c: &Config) -> Value {
    json!({"enabled":c.enabled,"mask":c.excluded_mask,"categories":c.excluded_names(),
        "scope":"color_geometry_only","retained":["shadows","GI","physics","effects"],
        "dedicated_passes":{"near_grass_present":c.near_grass_present,"far_grass_present":c.far_grass_present,
            "grass_enabled":c.grass_enabled(),"transparency_present":c.transparency_present},
        "interpretation":"Baseline minus exclusion is a marginal, non-additive cost; removing occluders changes visible work behind them."})
}

pub fn capture(renderer: &Renderer, gpu: &Gpu, scene: &Scene, view_count: usize, frame_id: u64) -> Result<Value,String> {
    let c=renderer.geometry_diagnostic_config()?;
    let mut result=config(&c);
    let mut occlusion_views=Vec::new();let mut occlusion_sequence=None;
    for view in 0..view_count {
        let d=renderer.debug_tree_occlusion(gpu,view,occlusion_sequence)?;
        if d.frame_id!=frame_id {return Err(format!("tree occlusion snapshot frame {} does not match capture {frame_id}",d.frame_id));}
        occlusion_sequence=Some(d.render_sequence);
        let groups=d.instance_groups.as_ref().map(|g|json!({
            "enabled":g.enabled,"total_parent_groups":g.total_parent_groups,
            "total_leaf_groups":g.total_leaf_groups,"owner_slots":g.owner_slots,
            "shared_bytes":g.shared_bytes,"owner_scratch_bytes_per_view":g.owner_scratch_bytes_per_view,
            "parent_groups_tested":g.parent_groups_tested,"parent_groups_rejected":g.parent_groups_rejected,
            "leaf_groups_tested":g.leaf_groups_tested,"leaf_groups_rejected":g.leaf_groups_rejected,
            "owner_tests":g.owner_tests,"compacted_owners":g.compacted_owners,
            "resident_clusters_visited":g.resident_clusters_visited,"overflow_fallbacks":g.overflow_fallbacks,
            "traversal_mismatch_fallbacks":g.traversal_mismatch_fallbacks,
            "scope":"Actual GPU phase work summed over this view's camera phases; not unique world owners. Shared bounds refit once for all views. Shadow selection stays independent. Read back only on this excluded diagnostic capture."}));
        let traversal=d.cluster_traversal.as_ref().map(|t|json!({
            "enabled":t.enabled,"shared_bytes":t.shared_bytes,"metadata_nodes":t.metadata_nodes,
            "indexed_meshes":t.indexed_meshes,"counter_bytes_per_view":t.counter_bytes_per_view,
            "accelerated_owners":t.accelerated_owners,"nodes_tested":t.nodes_tested,"nodes_rejected":t.nodes_rejected,
            "clusters_tested":t.clusters_tested,"clusters_skipped":t.clusters_skipped,
            "flat_owners":t.flat_owners,"fallback_owners":t.fallback_owners,"resident_clusters":t.resident_clusters,
            "scope":"Actual cluster-BVH work across this camera's phases. Tested+skipped equals resident clusters of accelerated owners; flat owners remain separate. Shared bytes counted once."}));
        occlusion_views.push(json!({"enabled":d.enabled,"rejection_enabled":d.rejection_enabled,"disabled_reason":d.disabled_reason,"frame_id":d.frame_id,"render_sequence":d.render_sequence,
            "view_index":d.view_index,"split_m":d.split_m,"phase1_clusters":d.phase1_clusters,
            "phase2_clusters":d.phase2_clusters,"instances_tested":d.instances_tested,"instances_rejected":d.instances_rejected,
            "clusters_tested":d.clusters_tested,"clusters_rejected":d.clusters_rejected,
            "instance_groups":groups,"cluster_traversal":traversal}));
    }
    result["tree_occlusion"]=json!({"enabled":occlusion_views.iter().any(|v|v["enabled"]==true),
        "views":occlusion_views,"scope":"Actual constructed per-view current-frame tree occlusion. Phase counts are disjoint submitted cluster lists. Rejected instances avoid cluster traversal; rejected clusters are among tested surviving owners. Counters are not shaded pixels, and shadow casters remain independent."});
    let mut shadow_views=Vec::new();
    for s in renderer.debug_sun_culling(gpu,occlusion_sequence)? {
        if s.frame_id!=frame_id {return Err("solar diagnostic frame does not match capture".into());}
        shadow_views.push(json!({"frame_id":s.frame_id,"render_sequence":s.render_sequence,
            "pass":s.name,"view_index":s.view_index,"instance_groups":group_work(&s.groups),
            "selected_pair_count":s.pairs.len(),"selected_pair_digest":selection_digest(&s.pairs),
            "selected_triangles":s.selected_triangles,"submitted_vertices":s.submitted_vertices}));
    }
    result["shadow_visibility"]=json!({"passes":shadow_views,
        "scope":"Only solar passes submitted this frame; reused maps are not fresh work. Exact sorted owner/cluster fingerprints; light visibility independent of camera. Capture-only blocking readback, excluded from timing."});
    if !shadow_views.is_empty() {
        let d=renderer.receiver_shadow_demand_diagnostics(gpu)?;
        if d.enabled && Some(u64::from(d.requested_generation))!=occlusion_sequence.map(|s|u64::from((s as u32).wrapping_add(1).max(1))) {
            return Err("stale receiver shadow demand capture".into());
        }
        result["shadow_receiver_demand"]=json!({"enabled":d.enabled,
            "requested_generation":d.requested_generation,"published_generation":d.published_generation,
            "fallback_reasons":d.fallback_reasons,"shared_bytes":d.shared_bytes,"mask_bytes":d.mask_bytes,
            "receiver_owner_tests":d.receiver_owner_tests,"receiver_bounds":d.receiver_bounds,"extra_bounds":d.extra_bounds,
            "coarse_receiver_owners":d.coarse_receiver_owners,"refined_receiver_owners":d.refined_receiver_owners,
            "receiver_cluster_tests":d.receiver_cluster_tests,"receiver_cluster_bounds":d.receiver_cluster_bounds,
            "maps":d.maps.iter().map(|m|json!({"map_index":m.map_index,"size":m.size,"tile_size":m.tile_size,
                "requested_tiles":m.requested_tiles,"total_tiles":m.total_tiles,"receiver_bounds":m.receiver_bounds,
                "caster_clusters_tested":m.caster_clusters_tested,"caster_clusters_rejected":m.caster_clusters_rejected,
                "caster_clusters_full_demand":m.caster_clusters_full_demand})).collect::<Vec<_>>(),
            "scope":"Current-frame conservative receivers; partial demand can omit caster clusters only outside demanded light-space tiles. Explicit capture readback excluded from timing."});
    }
    let mut lamp_views=Vec::new();
    let lamp_depth=renderer.debug_headlight_depth_hashes(gpu,occlusion_sequence)?;
    for s in renderer.debug_headlight_culling(gpu,occlusion_sequence)? {
        if s.frame_id!=frame_id {return Err("headlight diagnostic frame does not match capture".into());}
        let depth=lamp_depth.get(s.view_index.ok_or("headlight index missing")?).ok_or("headlight depth missing")?;
        lamp_views.push(json!({"frame_id":s.frame_id,"render_sequence":s.render_sequence,
            "pass":s.name,"light_index":s.view_index,"instance_groups":group_work(&s.groups),
            "selected_pair_count":s.pairs.len(),"selected_pair_digest":selection_digest(&s.pairs),
            "selected_triangles":s.selected_triangles,"submitted_vertices":s.submitted_vertices,
            "shadow_depth_sha256":depth}));
    }
    result["headlight_visibility"]=json!({"passes":lamp_views,
        "scope":"Independent perspective shadow frusta, actual current-pose casters. No camera fog/Hi-Z rejection. Sorted owner/cluster fingerprints preserve multiplicity; capture readback excluded from timing."});
    if !c.enabled {return Ok(result);}
    result["source_counts"]=json!(scene.geometry_category_counts().into_iter().enumerate().map(|(i,row)|json!({
        "category":GeometryCategory::ALL[i].name(),"instances":row.instances,"active_instances":row.active_instances,
        "referenced_clusters":row.referenced_clusters})).collect::<Vec<_>>());
    result["source_scope"]="Allocated CPU owners and all available LOD references; not GPU-selected or unique geometry.".into();
    let mut views=Vec::new();let mut sequence=None;
    for view in 0..view_count {
        let selected=renderer.debug_visible_clusters(gpu,view,sequence)?;
        if selected.frame_id!=frame_id {return Err(format!("category snapshot frame {} does not match capture {frame_id}",selected.frame_id));}
        sequence=Some(selected.render_sequence);
        let counts=selected.categories.ok_or("enabled category diagnostic has no selected counts")?;
        let pixels=renderer.debug_geometry_pixels(gpu,view,sequence)?;
        if pixels.frame_id!=frame_id || pixels.input_size!=selected.input_size {return Err("category pixel snapshot does not match selected geometry".into());}
        views.push(json!({"view_index":view,"frame_id":selected.frame_id,"render_sequence":selected.render_sequence,
            "viewport":selected.viewport,"input_size":selected.input_size,"output_size":selected.output_size,
            "selected_pair_count":selected.pairs.len(),"selected_pair_digest":selection_digest(&selected.pairs),
            "selected_pair_digest_scope":"Sorted (owner u32, cluster u32) multiset, little-endian count u64 then pair words; FNV1a64. Capture-only, append-order independent, duplicates retained. Fingerprint equality is not a proof of pixel equality.",
            "categories":counts.iter().enumerate().map(|(i,row)|json!({"category":row.category.name(),"instances":row.instances,
                "clusters":row.clusters,"triangles":row.triangles,"submitted_vertices":row.submitted_vertices,
                "visibility_pixels":pixels.pixels[i]})).collect::<Vec<_>>(),"uncovered_pixels":pixels.uncovered}));
    }
    result["views"]=json!(views);
    result["view_scope"]="Shared visibility raster only; input-resolution depth-winning pixels before dedicated grass, transparency, effects, DLSS and HUD. Selected clusters include hidden/overdrawn geometry. Synchronous capture frame excluded from timing.".into();
    Ok(result)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn selection_fingerprint_ignores_append_order_but_keeps_missing_and_duplicate_work() {
        let a=[[5,10],[7,9],[5,11]];
        assert_eq!(selection_digest(&a),selection_digest(&[a[2],a[0],a[1]]));
        assert_ne!(selection_digest(&a),selection_digest(&a[..2]));
        assert_ne!(selection_digest(&a),selection_digest(&[a[0],a[0],a[2]]));
        assert_ne!(selection_digest(&a),selection_digest(&[[5,10],[7,10],[5,11]]));
    }
}
