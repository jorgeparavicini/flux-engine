use flux_ecs::commands::Commands;
use flux_ecs::component::Component;
use flux_ecs::query::Query;
use flux_ecs::resource::{Res, Resource};
use flux_ecs::schedule::ScheduleLabel;
use flux_ecs::world::World;
use flux_renderer::RendererPlugin;

fn main() {
    let mut world = World::new();
    world.add_plugin(RendererPlugin);
}

struct Time {
    seconds: f32,
}

impl Resource for Time {}

fn test_system(r: Res<Time>) {
    println!("Seconds elapsed: {}", r.seconds);
}

fn empty_system() {}

#[derive(Debug)]
struct TestComponent {}

impl Component for TestComponent {}

fn query_system(q: Query<&TestComponent>) {
    for item in q {
        println!("Querying TestComponent: {item:?}");
    }
}

fn command_system(mut commands: Commands) {
    commands.insert_resource(Time { seconds: 0.0 });
}

fn test() {
    let mut world = World::new();
    world.insert_resource(Time { seconds: 42.0 });
    world.spawn((TestComponent {},));

    //Add a system that prints the entity index
    world.add_system(ScheduleLabel::Main, test_system);
    world.add_system(ScheduleLabel::Main, empty_system);
    world.add_system(ScheduleLabel::Main, query_system);
    world.add_system(ScheduleLabel::Main, command_system);

    // Run all systems
    for _ in 0..10 {
        world.get_resource_mut::<Time>().unwrap().seconds += 1.0;
        //world.run_systems();
    }
}
