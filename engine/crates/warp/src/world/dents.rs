use super::{show, Look, Shape, World};
use kern::crush::Dent;
use render::{cluster::Vertex, mesh_deform::DentModel};
use std::{collections::BTreeMap, ops::Range};

#[derive(Default)]
pub(super) struct Pool {
    paths: Vec<String>,
    models: BTreeMap<String, DentModel>,
    slots: Vec<Slot>,
    coarse: BTreeMap<u32, u32>,
}

struct Slot {
    vertices: Range<usize>,
    owner: Option<u32>,
    fields: Vec<Dent>,
    mask: u32,
    dirty: usize,
}

impl World {
    /// A deterministic three-centimetre LOD of the authored panels is shared
    /// by every car of this model. Detached doors leave holes in this surface.
    pub(crate) fn prepare_panel_query(
        &mut self,
        id: u32,
        profile: &str,
        body: u32,
        fields: &[Dent],
        mask: u32,
    ) -> Result<(), String> {
        let e = self.entities.get(&body).ok_or("unknown panel body")?;
        let Shape::Model(path) = &e.shape else {
            return Err("panel body must be a model".into());
        };
        let fleet = self.fleet.as_mut().ok_or("call vehicles.reset first")?;
        if !fleet.has_panel_surface(path) {
            let source = &self.models[path];
            let mesh = &self.scene.meshes[source.mesh];
            let mut ids = std::collections::HashMap::new();
            let mut vertices = Vec::new();
            let mut faces = Vec::new();
            let start = mesh.first_cluster as usize;
            for cluster in &self.scene.clusters[start..start + mesh.cluster_count as usize] {
                if cluster.own_error > 0.03 || cluster.parent_error <= 0.03 {
                    continue;
                }
                let first = cluster.first_tri as usize;
                for &word in &self.scene.tris[first..first + cluster.tri_count as usize] {
                    let global = [word & 255, (word >> 8) & 255, (word >> 16) & 255].map(|i| {
                        self.scene.cluster_verts[cluster.first_vert as usize + i as usize] as usize
                    });
                    let part = source
                        .vertex_parts
                        .get(global[0] - source.rest.start)
                        .copied()
                        .unwrap_or(0);
                    let face = global.map(|i| {
                        *ids.entry(i).or_insert_with(|| {
                            let next = vertices.len() as u32;
                            vertices.push(self.scene.vertices[i].pos.into());
                            next
                        })
                    });
                    faces.push((face, part));
                }
            }
            fleet.set_panel_surface(path.clone(), vertices, faces);
        }
        fleet.cut_query(id, profile, path, fields, mask)
    }

    pub fn warm_dents(&mut self, paths: Vec<String>, count: usize) -> Result<(), String> {
        if self.dents.paths == paths && self.dents.slots.len() == count {
            return Ok(());
        }
        if !self.dents.slots.is_empty() {
            return Err("one dent pool per scene; declare all models together".into());
        }
        if !(1..=8).contains(&count) || !(1..=16).contains(&paths.len()) {
            return Err("dent pool needs 1..16 model paths and 1..8 slots".into());
        }
        for path in &paths {
            self.mesh(&Shape::Model(path.clone()), &Look::default())?;
        }
        let max = paths
            .iter()
            .map(|p| self.models[p].rest.len())
            .max()
            .unwrap_or(0);
        if max == 0
            || max * count > 1_000_000
            || paths.iter().any(|p| self.models[p].rest.is_empty())
        {
            return Err("invalid or oversized dent model".into());
        }
        let start = self.scene.vertices.len();
        self.scene.vertices.resize(
            start + max * count,
            Vertex {
                pos: [0.; 3],
                normal: [0., 1., 0.],
                uv: [0.; 2],
            },
        );
        let ranges: Vec<_> = (0..count)
            .map(|i| start + i * max..start + (i + 1) * max)
            .collect();
        let old_extent = (
            self.scene.meshes.len(),
            self.scene.clusters.len(),
            self.scene.cluster_verts.len(),
        );
        let mut models = BTreeMap::new();
        for path in &paths {
            let m = &self.models[path];
            match DentModel::reserve(
                &mut self.scene,
                m.mesh,
                m.rest.clone(),
                &ranges,
                &m.vertex_parts,
            ) {
                Ok(model) => {
                    models.insert(path.clone(), model);
                }
                Err(error) => {
                    self.scene.vertices.truncate(start);
                    self.scene.meshes.truncate(old_extent.0);
                    self.scene.clusters.truncate(old_extent.1);
                    self.scene.cluster_verts.truncate(old_extent.2);
                    return Err(error);
                }
            }
        }
        self.dents = Pool {
            paths,
            models,
            coarse: BTreeMap::new(),
            slots: ranges
                .into_iter()
                .map(|vertices| Slot {
                    vertices,
                    owner: None,
                    fields: Vec::new(),
                    mask: 0,
                    dirty: 0,
                })
                .collect(),
        };
        self.rebuild = true;
        Ok(())
    }

    pub fn dent(&mut self, id: u32, fields: Option<Vec<Dent>>, mask: u32) -> Result<bool, String> {
        let entity = self.entities.get_mut(&id).ok_or("unknown dent body")?;
        let Shape::Model(path) = &entity.shape else {
            return Err("world.dent needs an imported model".into());
        };
        let source = self.models.get(path).ok_or("unknown model")?;
        let previous = self.dents.slots.iter().position(|s| s.owner == Some(id));
        if let Some(fields) = fields {
            let variant = self
                .dents
                .models
                .get(path)
                .ok_or("model was not reserved by world.warm_dents")?;
            let Some(i) =
                previous.or_else(|| self.dents.slots.iter().position(|s| s.owner.is_none()))
            else {
                return Ok(false);
            };
            let slot = &mut self.dents.slots[i];
            self.dents.coarse.remove(&id);
            slot.owner = Some(id);
            slot.mask = mask;
            slot.fields = fields;
            variant.write(
                &mut self.scene,
                slot.vertices.start,
                &slot.fields,
                &source.vertex_parts,
                mask,
            );
            slot.dirty = variant.rest.len();
            entity.mesh = variant.meshes[i];
        } else {
            if let Some(i) = previous {
                self.dents.slots[i].owner = None;
                self.dents.slots[i].fields.clear();
            }
            if mask == 0 {
                self.dents.coarse.remove(&id);
            } else {
                self.dents.coarse.insert(id, mask);
            }
            entity.mesh = self
                .dents
                .models
                .get(path)
                .and_then(|m| m.coarse.get(&(mask & m.removable)))
                .copied()
                .unwrap_or(source.mesh);
        }
        show(&mut self.slots, &mut self.scene, entity);
        Ok(true)
    }

    pub(super) fn release_dent(&mut self, id: u32) {
        self.dents.coarse.remove(&id);
        if let Some(s) = self.dents.slots.iter_mut().find(|s| s.owner == Some(id)) {
            s.owner = None;
            s.fields.clear();
            s.dirty = 0;
        }
    }

    pub(super) fn restore_dents(&self, next: &mut World) -> Result<(), String> {
        if self.dents.slots.is_empty() {
            return Ok(());
        }
        next.warm_dents(self.dents.paths.clone(), self.dents.slots.len())?;
        for (&id, &mask) in &self.dents.coarse {
            next.dent(id, None, mask)?;
        }
        for s in &self.dents.slots {
            if let Some(id) = s.owner {
                next.dent(id, Some(s.fields.clone()), s.mask)?;
            }
        }
        Ok(())
    }

    pub(super) fn upload_dents(
        &mut self,
        gpu: &render::Gpu,
        r: &render::Renderer,
    ) -> Result<(), String> {
        for s in &mut self.dents.slots {
            if s.dirty != 0 {
                r.update_vertices(
                    gpu,
                    s.vertices.start,
                    &self.scene.vertices[s.vertices.start..s.vertices.start + s.dirty],
                )?;
                s.dirty = 0;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Spawn;
    use glam::Vec3;
    use std::path::Path;

    #[test]
    fn dent_slots_isolate_identical_paint_and_survive_rebuilds() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
        let mut w = World::new(&game, 16);
        let path = "assets/cars/sedan.gltf".to_string();
        let wheel = "assets/cars/wheel-left.gltf".to_string();
        w.warm_dents(vec![path.clone(), wheel.clone()], 1).unwrap();
        let car = Shape::Model(path.clone());
        let a = w
            .spawn(Spawn {
                shape: car.clone(),
                ..Default::default()
            })
            .unwrap();
        let b = w
            .spawn(Spawn {
                shape: car,
                ..Default::default()
            })
            .unwrap();
        let stock = w.entities[&b].mesh;
        let source = &w.models[&path];
        let i = source.vertex_parts.iter().position(|id| *id == 2).unwrap();
        let original = w.scene.vertices[source.rest.start + i];
        let strike = Dent::new(original.pos.into(), Vec3::X, 0.8, 0.35).unwrap();
        assert!(w.dent(a, Some(vec![strike]), 0).unwrap());
        assert!(!w.dent(b, Some(vec![strike]), 0).unwrap());
        let data = &w.scene.vertices;
        let bent = data[w.dents.slots[0].vertices.start + i];
        assert!(Vec3::from(original.pos).distance(bent.pos.into()) > 0.25);
        assert_eq!(data[w.models[&path].rest.start + i].pos, original.pos);
        assert_eq!(w.entities[&b].mesh, stock);
        assert_ne!(w.entities[&a].mesh, stock);
        w.fresh_scene().unwrap();
        assert_eq!(
            w.scene.vertices[w.dents.slots[0].vertices.start + i].pos,
            bent.pos
        );
        assert!(w.dent(a, Some(vec![strike]), 1 << 2).unwrap());
        assert_eq!(
            w.scene.vertices[w.dents.slots[0].vertices.start + i].pos,
            [0.; 3]
        );
        assert!(w.dent(a, None, 1 << 2).unwrap());
        assert_eq!(w.entities[&a].mesh, w.dents.models[&path].coarse[&(1 << 2)]);
        assert!(w.dent(b, Some(vec![strike]), 0).unwrap());
        w.update(b, |e| e.shape = Shape::Cube).unwrap();
        let plain = w
            .spawn(Spawn {
                shape: Shape::Model(wheel),
                ..Default::default()
            })
            .unwrap();
        assert!(w.dent(plain, Some(vec![strike]), 0).unwrap());
    }
}
