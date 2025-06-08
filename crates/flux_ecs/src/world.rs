use crate::archetype::ArchetypeManager;
use crate::component::ComponentRegistry;
use crate::entity::{Entity, EntityManager};
use crate::resource::{Resource, Resources};
use crate::system::systems::Systems;
use crate::system::IntoSystem;

pub struct World {
    entity_manager: EntityManager,
    archetype_manager: ArchetypeManager,
    component_registry: ComponentRegistry,
    pub resources: Resources,
    pub systems: Systems,
}

impl World {
    pub fn new() -> Self {
        Self {
            entity_manager: EntityManager::new(),
            archetype_manager: ArchetypeManager::new(),
            component_registry: ComponentRegistry::new(),
            resources: Resources::new(),
            systems: Systems::new(),
        }
    }

    pub fn spawn(&mut self) -> Entity {
        self.entity_manager.spawn()
    }

    pub fn get_resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    pub fn get_resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.resources.get_mut::<T>()
    }

    pub fn add_system<M>(&mut self, system: impl IntoSystem<M>) {
        self.systems.add_system(system);
    }

    pub fn run_systems(&mut self) {
        // TODO: This is a temporary solution to avoid borrowing issues.
        let mut systems = std::mem::take(&mut self.systems);

        systems.run(self);

        self.systems = systems;
    }
}
