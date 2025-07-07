use crate::archetypes::Archetypes;
use crate::commands::{Command, CommandQueue};
use crate::component::{ComponentBundle, ComponentRegistry};
use crate::entity::{Entity, EntityManager};
use crate::module::Module;
use crate::plugin::Plugin;
use crate::resource::{Resource, Resources};
use crate::schedule::{ScheduleLabel, Schedules};
use crate::system::IntoSystem;

pub struct World {
    entity_manager: EntityManager,
    archetypes: Archetypes,
    pub(crate) component_registry: ComponentRegistry,
    resources: Resources,
    schedules: Schedules,
    command_queue: CommandQueue,
}

impl World {
    pub fn new() -> Self {
        Self {
            entity_manager: EntityManager::new(),
            archetypes: Archetypes::new(),
            component_registry: ComponentRegistry::default(),
            resources: Resources::new(),
            schedules: Schedules::new(),
            command_queue: CommandQueue::new(),
        }
    }

    pub fn spawn<C: ComponentBundle>(&mut self, bundle: C) -> Entity {
        let entity = self.entity_manager.spawn();

        let mut component_ids = C::register_components(&mut self.component_registry);

        let archetype_id = self
            .archetypes
            .get_or_create_for_bundle::<C>(&mut self.component_registry);

        let archetype = self
            .archetypes
            .get_mut(archetype_id)
            .expect("Archetype was not found for the given bundle");

        let pointers = unsafe { bundle.get_component_painters() };

        let component_data_to_add: Vec<_> = component_ids.into_iter().zip(pointers).collect();

        let row =
            unsafe { archetype.add(entity, &component_data_to_add, &self.component_registry) };

        // TODO: Update entity location

        entity
    }

    pub fn archetypes(&self) -> &Archetypes {
        &self.archetypes
    }

    pub fn get_resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    pub fn get_resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.resources.get_mut::<T>()
    }

    pub fn insert_resource<T: Resource>(&mut self, resource: T) {
        self.resources.insert(resource);
    }

    pub fn add_system<M>(&mut self, schedule: ScheduleLabel, system: impl IntoSystem<M>) {
        self.schedules.add(schedule, system);
    }

    pub fn register_module<T: Module>(&mut self) {
        T::register(self);
    }

    pub fn add_command(&mut self, command: Box<dyn Command>) {
        self.command_queue.push(command);
    }

    pub fn flush_commands(&mut self) {
        while !self.command_queue.is_empty() {
            let commands = std::mem::take(&mut self.command_queue.commands);
            
            for command in commands {
                command.execute(self);
            }
        }
    }

    pub fn add_plugin(&mut self, plugin: impl Plugin) {
        plugin.init(self);
    }
}
