use crate::instance::create_instance;
use crate::surface::create_surface;
use flux_ecs::plugin::Plugin;
use flux_ecs::schedule::ScheduleLabel;
use flux_ecs::world::World;

mod instance;
mod surface;

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn init(&self, world: &mut World) {
        world.add_system(ScheduleLabel::Initialization, create_instance);
        world.add_system(ScheduleLabel::Initialization, create_surface);
    }
}
