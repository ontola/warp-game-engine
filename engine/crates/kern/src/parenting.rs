//! Model sockets and allocation-free traversal of parent relationships.
use std::{collections::HashMap, ops::Mul};

/// Walk from a node to its root, including both. Validate imported links before traversal.
pub fn ancestors<K: Copy>(node: K, parent: impl Fn(K) -> Option<K>) -> impl Iterator<Item = K> {
    std::iter::successors(Some(node), move |&id| parent(id))
}

/// Fixed topology shared by every pose of an imported model. Socket names map to
/// node indices, so a light can follow an animated gem even if it has no mesh.
#[derive(Clone, Debug, Default)]
pub struct ModelHierarchy {
    parents: Vec<Option<usize>>,
    sockets: HashMap<String, usize>,
}

impl ModelHierarchy {
    /// Validate parent links and named sockets once when loading a model.
    pub fn new(
        parents: Vec<Option<usize>>,
        sockets: HashMap<String, usize>,
    ) -> Result<Self, String> {
        validate(&parents)?;
        for (name, &id) in &sockets {
            if id >= parents.len() {
                return Err(format!("socket {name} points to missing node {id}"));
            }
        }
        Ok(Self { parents, sockets })
    }

    /// Resolve a socket name to a stable index for this model.
    pub fn node(&self, socket: &str) -> Option<usize> {
        self.sockets.get(socket).copied()
    }

    /// Walk from a valid model index through its root (both included).
    pub fn ancestors(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        ancestors(node, |id| self.parents[id])
    }

    /// Compose local transforms up to and including the root. The caller supplies
    /// the rest or animated local at each index. Apply the actor's world pose last.
    pub fn model_transform<T: Mul<Output = T>>(
        &self,
        node: usize,
        local: impl Fn(usize) -> T,
    ) -> T {
        self.ancestors(node)
            .skip(1)
            .fold(local(node), |child, id| local(id) * child)
    }
}

/// Check an indexed model forest once on import. Indices may be in any order.
fn validate(parents: &[Option<usize>]) -> Result<(), String> {
    let mut visited = vec![0; parents.len()];
    for start in 0..parents.len() {
        let mut next = Some(start);
        while let Some(id) = next {
            let mark = visited
                .get_mut(id)
                .ok_or_else(|| format!("missing parent {id}"))?;
            if *mark == start + 1 {
                return Err(format!("parent cycle at node {id}"));
            }
            if *mark != 0 {
                break;
            }
            *mark = start + 1;
            next = parents[id];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    #[test]
    fn animated_socket_in_an_unordered_forest() {
        let tree = ModelHierarchy::new(
            vec![Some(2), None, Some(1), None],
            [("gem".into(), 0)].into(),
        )
        .unwrap();
        let locals = [
            Mat4::from_translation(Vec3::Y),
            Mat4::from_rotation_z(0.8),
            Mat4::from_scale(Vec3::new(0.2, 3.0, 1.0)),
            Mat4::IDENTITY,
        ];
        assert_eq!(
            tree.model_transform(tree.node("gem").unwrap(), |i| locals[i]),
            locals[1] * (locals[2] * locals[0])
        );
        assert_eq!(tree.node("missing"), None);
        assert!(validate(&[Some(1), Some(0)]).is_err());
        assert!(validate(&[Some(2)]).is_err());
        assert!(ModelHierarchy::new(vec![], [("missing".into(), 1)].into()).is_err());
    }
}
