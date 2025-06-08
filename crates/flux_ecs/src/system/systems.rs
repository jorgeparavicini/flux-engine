use crate::system::{IntoSystem, System};
use crate::world::World;

#[derive(Default)]
pub struct Systems {
    pub(crate) systems: Vec<Box<dyn System>>,
}

impl Systems {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    pub fn add_system<M>(&mut self, system: impl IntoSystem<M>) {
        self.systems.push(Box::new(IntoSystem::into_system(system)));
    }

    pub fn run(&mut self, world: &mut World) {
        for system in &mut self.systems {
            system.run(world);
        }
    }
}
