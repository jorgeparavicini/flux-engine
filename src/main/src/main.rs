use flux_ecs::schedule::ScheduleLabel::Initialization;
use flux_ecs::world::World;
use flux_renderer::RendererPlugin;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let mut world = World::new();
    world.add_plugin(RendererPlugin);
    world.run_system(&Initialization);
    sleep(Duration::from_secs(1));
}
