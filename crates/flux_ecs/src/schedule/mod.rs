use crate::system::systems::Systems;
use crate::system::IntoSystem;
use crate::world::World;
use std::collections::HashMap;

#[derive(Debug, Hash, Eq, PartialEq)]
pub enum ScheduleLabel {
    Initialization,
    Main,
}

#[derive(Default)]
pub struct Schedule {
    pub systems: Systems,
}

pub struct Schedules {
    schedule_map: HashMap<ScheduleLabel, Schedule>,
}

impl Schedules {
    pub fn new() -> Self {
        Self {
            schedule_map: HashMap::from([
                (ScheduleLabel::Initialization, Schedule::default()),
                (ScheduleLabel::Main, Schedule::default()),
            ]),
        }
    }

    pub fn add<M>(&mut self, schedule: ScheduleLabel, system: impl IntoSystem<M>) {
        let schedules = self.schedule_map
            .entry(schedule)
            .or_default();

        schedules.systems.add_system(system);
    }

    pub fn get_schedule(&self, schedule: &ScheduleLabel) -> Option<&Schedule> {
        self.schedule_map.get(schedule)
    }

    pub fn run_schedule(&mut self, schedule: &ScheduleLabel, world: &mut World) {
        if let Some(schedule) = self.schedule_map.get_mut(schedule) {
            schedule.systems.run(world);
        }
    }
}
