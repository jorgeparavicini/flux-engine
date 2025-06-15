use crate::archetype::{Archetype, ArchetypeId};
use crate::archetype_graph::ArchetypeGraph;
use crate::component::{ComponentBundle, ComponentId, ComponentRegistry};
use crate::entity::{Entity, EntityLocation};

#[derive(Default)]
pub struct Archetypes {
    graph: ArchetypeGraph,
    storage: Vec<Archetype>,
}

impl Archetypes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create_for_bundle<B: ComponentBundle>(
        &mut self,
        registry: &mut ComponentRegistry,
    ) -> ArchetypeId {
        let mut component_ids = B::register_components(registry);

        let archetype_id = self.graph.get_or_create_archetype(&mut component_ids);

        if archetype_id.0 >= self.storage.len() {
            self.storage.resize_with(archetype_id.0 + 1, || {
                Archetype::new(ArchetypeId(usize::MAX))
            });
            self.storage[archetype_id.0] = Archetype::new(archetype_id);
        }

        archetype_id
    }

    pub fn get_mut(&mut self, id: ArchetypeId) -> Option<&mut Archetype> {
        self.storage.get_mut(id.0)
    }

    pub fn get_add_component_destination(
        &mut self,
        start_id: ArchetypeId,
        component_id: ComponentId,
    ) -> ArchetypeId {
        if let Some(id) = self.graph.get_add_edge(start_id, component_id) {
            return id;
        }

        let mut new_signature = self
            .graph
            .get_signature(start_id)
            .expect("Archetype signature not found")
            .to_vec();

        new_signature.push(component_id);

        self.graph.get_or_create_archetype(&mut new_signature)
    }

    pub fn move_entity(
        &mut self,
        entity: Entity,
        location: EntityLocation,
        target_archetype_id: ArchetypeId,
    ) -> (EntityLocation, Option<Entity>) {
        let (source_slice, target_slice) = self.storage.split_at_mut(std::cmp::max(
            location.archetype_id.0,
            target_archetype_id.0,
        ));

        let (source_archetype, target_archetype) =
            if location.archetype_id.0 < target_archetype_id.0 {
                (
                    &mut source_slice[location.archetype_id.0],
                    &mut target_slice[0],
                )
            } else {
                (
                    &mut target_slice[0],
                    &mut source_slice[target_archetype_id.0],
                )
            };

        let new_row;
        unsafe {
            new_row = target_archetype.add_moved_entity(entity, source_archetype, location.row);
        }

        let (_removed_entity, moved_entity_in_source) = source_archetype.remove(location.row);

        let new_location = EntityLocation {
            archetype_id: target_archetype_id,
            row: new_row,
        };

        (new_location, moved_entity_in_source)
    }

    pub fn iter(&self) -> ArchetypeIter<'_> {
        ArchetypeIter::new(&self.storage)
    }

    pub fn get(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.storage.get(id.0)
    }
}

pub struct ArchetypeIter<'a> {
    archetypes: &'a [Archetype],
    index: usize,
}

impl<'a> ArchetypeIter<'a> {
    fn new(archetypes: &'a [Archetype]) -> Self {
        Self {
            archetypes,
            index: 0,
        }
    }
}

impl<'a> Iterator for ArchetypeIter<'a> {
    type Item = &'a Archetype;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.archetypes.len() {
            let archetype = &self.archetypes[self.index];
            self.index += 1;
            Some(archetype)
        } else {
            None
        }
    }
}
