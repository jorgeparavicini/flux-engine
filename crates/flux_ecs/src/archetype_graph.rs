use crate::component::ComponentId;
use std::collections::HashMap;
use crate::archetype::ArchetypeId;

type ArchetypeSignature = Box<[ComponentId]>;

#[derive(Default)]
pub struct ArchetypeGraph {
    by_signature: HashMap<ArchetypeSignature, ArchetypeId>,
    signatures: Vec<ArchetypeSignature>,
    add_component_edges: HashMap<(ArchetypeId, ComponentId), ArchetypeId>,
    remove_component_edges: HashMap<(ArchetypeId, ComponentId), ArchetypeId>,
}

impl ArchetypeGraph {
    pub fn get_or_create_archetype(&mut self, components: &mut [ComponentId]) -> ArchetypeId {
        components.sort();
        let signature: ArchetypeSignature = components.into();

        if let Some(id) = self.by_signature.get(&signature) {
            return *id;
        }

        let new_id = ArchetypeId(self.signatures.len());
        self.by_signature.insert(signature.clone(), new_id);
        self.signatures.push(signature.clone());

        for (i, component_id) in signature.iter().enumerate() {
            let mut smaller_components = Vec::with_capacity(components.len() - 1);
            smaller_components.extend_from_slice(&signature[..i]);
            smaller_components.extend_from_slice(&signature[i + 1..]);

            let smaller_archetype_id = self.get_or_create_archetype(&mut smaller_components);

            self.add_component_edges
                .insert((smaller_archetype_id, *component_id), new_id);
            self.remove_component_edges
                .insert((new_id, *component_id), smaller_archetype_id);
        }

        new_id
    }

    pub fn get_add_edge(&self, start: ArchetypeId, component: ComponentId) -> Option<ArchetypeId> {
        self.add_component_edges.get(&(start, component)).copied()
    }
    
    pub fn get_signature(&self, id: ArchetypeId) -> Option<&ArchetypeSignature> {
        self.signatures.get(id.0)
    }
}
