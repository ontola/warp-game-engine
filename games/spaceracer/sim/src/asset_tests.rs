//! Asset parity gate: no stripped engines, missing control pivots or corrupt topology.
#[test]
fn all_original_ship_variants_keep_controls_effects_and_mesh_topology() {
    let data: serde_json::Value = serde_json::from_str(&crate::oracle("ships")).unwrap();
    let models = data["models"].as_array().unwrap();
    assert_eq!(models.len(), 5);
    fn names(node: &serde_json::Value, out: &mut Vec<String>) {
        out.push(node["name"].as_str().unwrap().into());
        for child in node["children"].as_array().unwrap() {
            names(child, out);
        }
    }
    for model in models {
        let mut found = vec![];
        names(model, &mut found);
        for expected in [
            "WingControlL",
            "WingControlR",
            "RudderL",
            "RudderR",
            "WeaponSocket",
            "ExhaustL",
            "ExhaustR",
            "EngineHeat-1",
            "EngineHeat1",
            "BoostArcs-1",
            "BoostArcs1",
        ] {
            assert!(
                found.iter().any(|s| s == expected),
                "{} missing {expected}",
                model["name"]
            );
        }
    }
    let mut triangles = 0;
    for mesh in data["meshes"].as_array().unwrap() {
        for s in mesh["surfaces"].as_array().unwrap() {
            let p = s["positions"].as_array().unwrap();
            assert!(!p.is_empty());
            for point in p {
                assert!(
                    point
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|x| x.as_f64().unwrap().is_finite())
                );
            }
            for key in ["normals", "uv", "colors"] {
                let a = s[key].as_array().unwrap();
                assert!(a.is_empty() || a.len() == p.len(), "invalid {key}");
            }
            let idx = s["indices"].as_array().unwrap();
            let count = if idx.is_empty() { p.len() } else { idx.len() };
            assert_eq!(count % 3, 0);
            triangles += count / 3;
            for index in idx {
                assert!((index.as_u64().unwrap() as usize) < p.len());
            }
        }
    }
    assert!(
        triangles > 10000,
        "source model detail unexpectedly missing"
    );
}
