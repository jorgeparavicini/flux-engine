use flux_ecs::schedule::ScheduleLabel::{Destroy, Initialization};
use flux_ecs::world::World;
use flux_renderer::RendererPlugin;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    pretty_env_logger::init();

    let mut world = World::new();
    world.add_plugin(RendererPlugin);
    world.run_system(&Initialization);
    sleep(Duration::from_secs(1));
    world.run_system(&Destroy);
}
