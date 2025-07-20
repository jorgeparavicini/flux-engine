use crate::system::{IntoSystem, System};
use crate::world::World;

#[derive(Default, PartialEq, Clone, Debug)]
pub enum CommandFlushTechnique {
    AfterEach,
    #[default]
    AfterAll,
}

#[derive(Default)]
pub struct Systems {
    pub(crate) systems: Vec<Box<dyn System>>,
    command_flush_technique: CommandFlushTechnique,
}

impl Systems {
    pub fn new(command_flush_technique: CommandFlushTechnique) -> Self {
        Self {
            systems: Vec::new(),
            command_flush_technique,
        }
    }

    pub fn add_system<M>(&mut self, system: impl IntoSystem<M>) {
        self.systems.push(Box::new(IntoSystem::into_system(system)));
    }

    pub fn run(&mut self, world: &mut World) {
        for system in &mut self.systems {
            system.run(world);
            
            if self.command_flush_technique == CommandFlushTechnique::AfterEach {
                world.flush_commands()
            }
        }
        
        if self.command_flush_technique == CommandFlushTechnique::AfterAll {
            world.flush_commands()
        }
    }
}
