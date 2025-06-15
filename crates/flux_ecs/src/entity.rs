use crate::archetype::ArchetypeId;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entity {
    index: u32,
}

pub(crate) struct EntityManager {
    next_index: u32,
}

impl EntityManager {
    pub fn new() -> Self {
        Self { next_index: 0 }
    }

    pub fn spawn(&mut self) -> Entity {
        let entity = Entity {
            index: self.next_index,
        };
        self.next_index += 1;
        entity
    }
}

pub struct EntityLocation {
    pub archetype_id: ArchetypeId,
    pub row: usize,
}
