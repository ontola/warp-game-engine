//! Preallocate private writable vertices for a small set of deformed mesh
//! instances. All slots and paints reuse the source triangle topology.
use crate::{
    cluster::Vertex,
    scene::{MeshRef, Scene},
};
use std::ops::Range;

pub struct DentModel {
    pub rest: Range<usize>,
    pub meshes: Vec<usize>,
    pub coarse: std::collections::BTreeMap<u32, usize>,
    pub removable: u32,
}

impl DentModel {
    pub fn reserve(
        scene: &mut Scene,
        mesh: usize,
        rest: Range<usize>,
        slots: &[Range<usize>],
        parts: &[u8],
    ) -> Result<Self, String> {
        let source = &scene.meshes[mesh];
        let (first, count) = (source.first_cluster as usize, source.cluster_count as usize);
        let mut sphere = source.sphere;
        sphere[3] += kern::crush::DENT_ENVELOPE;
        let clusters = &scene.clusters[first..first + count];
        let mut cv_lo = usize::MAX;
        let mut cv_hi = 0usize;
        for c in clusters {
            cv_lo = cv_lo.min(c.first_vert as usize);
            let tris = &scene.tris[c.first_tri as usize..(c.first_tri + c.tri_count) as usize];
            let max = tris
                .iter()
                .flat_map(|t| [t & 255, (t >> 8) & 255, (t >> 16) & 255])
                .max()
                .unwrap_or(0);
            cv_hi = cv_hi.max(c.first_vert as usize + max as usize + 1);
        }
        if cv_hi <= cv_lo
            || rest.end > scene.vertices.len()
            || slots.iter().any(|r| r.len() < rest.len())
        {
            return Err("invalid mesh vertex reservation".into());
        }
        if scene.cluster_verts[cv_lo..cv_hi]
            .iter()
            .any(|i| !rest.contains(&(*i as usize)))
        {
            return Err("mesh indices outside rest vertex range".into());
        }
        let mut meshes = Vec::with_capacity(slots.len());
        for slot in slots {
            let vertex_map = scene.cluster_verts.len();
            scene.cluster_verts.extend_from_within(cv_lo..cv_hi);
            for i in &mut scene.cluster_verts[vertex_map..] {
                *i = (slot.start + (*i as usize - rest.start)) as u32;
            }
            let start = scene.clusters.len();
            scene.clusters.extend_from_within(first..first + count);
            for c in &mut scene.clusters[start..] {
                c.first_vert = (vertex_map + c.first_vert as usize - cv_lo) as u32;
                // Every LOD samples the same field. Keep displaced surfaces in
                // the shadow/colour frusta and compensate its local stretch.
                c.cull[3] += kern::crush::DENT_ENVELOPE;
                c.own[3] += kern::crush::DENT_ENVELOPE;
                c.parent[3] += kern::crush::DENT_ENVELOPE;
                c.own_error *= 1.8;
                if c.parent_error < 1e30 {
                    c.parent_error *= 1.8;
                }
            }
            meshes.push(scene.meshes.len());
            scene.meshes.push(MeshRef {
                first_cluster: start as u32,
                cluster_count: count as u32,
                sphere,
            });
        }
        // Root LOD masks use only compact cluster headers. Far and unselected
        // cars keep missing panels in a single draw owner, with shared indices.
        let mut roots = Vec::new();
        let mut removable = 0u32;
        for c in &scene.clusters[first..first + count] {
            if c.parent_error <= 1e30 || c.tri_count == 0 {
                continue;
            }
            let k = scene.tris[c.first_tri as usize] & 255;
            let i = scene.cluster_verts[(c.first_vert + k) as usize] as usize;
            let part = parts.get(i - rest.start).copied().unwrap_or(0);
            if part >= 32 {
                return Err("part ID must fit the 32-bit mask".into());
            }
            let bit = if part == 0 { 0 } else { 1u32 << part };
            removable |= bit;
            let mut c = *c;
            c.own_error = 0.;
            roots.push((bit, c));
        }
        if removable.count_ones() > 6 {
            return Err("up to six detachable mesh groups per model".into());
        }
        let mut coarse = std::collections::BTreeMap::new();
        let mut mask = removable;
        while mask > 0 {
            let start = scene.clusters.len();
            scene.clusters.extend(
                roots
                    .iter()
                    .filter(|(bit, _)| bit & mask == 0)
                    .map(|(_, c)| *c),
            );
            coarse.insert(mask, scene.meshes.len());
            scene.meshes.push(MeshRef {
                first_cluster: start as u32,
                cluster_count: (scene.clusters.len() - start) as u32,
                sphere,
            });
            mask = (mask - 1) & removable;
        }
        Ok(Self {
            rest,
            meshes,
            coarse,
            removable,
        })
    }

    pub fn write(
        &self,
        scene: &mut Scene,
        dest: usize,
        fields: &[kern::crush::Dent],
        parts: &[u8],
        mask: u32,
    ) {
        let (source, target) = scene.vertices.split_at_mut(dest);
        for (i, (v, out)) in source[self.rest.clone()]
            .iter()
            .zip(target.iter_mut())
            .enumerate()
        {
            let part = parts.get(i).copied().unwrap_or(0);
            let (pos, normal) = if part != 0
                && 1u32
                    .checked_shl(part.into())
                    .is_some_and(|bit| mask & bit != 0)
            {
                // Each part has its own triangle vertices, including borders.
                // Collapse removed triangles to a shared, zero-area point.
                (glam::Vec3::ZERO, glam::Vec3::Y)
            } else {
                kern::crush::dent_vertex(v.pos.into(), v.normal.into(), fields)
            };
            *out = Vertex {
                pos: pos.to_array(),
                normal: normal.to_array(),
                uv: v.uv,
            };
        }
    }
}
