use crate::system::systems::Systems;
use std::collections::HashMap;
use crate::system::IntoSystem;

#[derive(Debug, Hash, Eq, PartialEq)]
pub enum Schedule {
    Initialization,
    Main,
}

pub struct Schedules {
    schedule_map: HashMap<Schedule, Systems>,
}

impl Schedules {
    pub fn new() -> Self {
        Self {
            schedule_map: HashMap::from([
                (Schedule::Initialization, Systems::new()),
                (Schedule::Main, Systems::new()),
            ]),
        }
    }

    pub fn add<M>(&mut self, schedule: Schedule, system: impl IntoSystem<M>) {
        let schedules = self.schedule_map
            .entry(schedule)
            .or_default();
        
        schedules.add_system(system);
    }

    pub fn get_schedule(&self, schedule: &Schedule) -> Option<&Systems> {
        self.schedule_map.get(schedule)
    }

    pub fn run_schedule(&mut self, schedule: &Schedule, world: &mut crate::world::World) {
        if let Some(systems) = self.schedule_map.get_mut(schedule) {
            systems.run(world);
        }
    }
}
